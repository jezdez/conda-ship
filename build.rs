//! Build script that solves conda dependencies at compile time and embeds a
//! rattler-lock v6 lockfile into the binary.
//!
//! Reads the `[tool.cx]` section from `pixi.toml` and uses a
//! content-hash to cache the lockfile: if the config hasn't changed since
//! the last build, the solve is skipped entirely.

use std::{
    collections::{HashMap, HashSet},
    env,
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

use rattler::{default_cache_dir, package_cache::PackageCache};
use rattler_conda_types::{
    Channel, ChannelConfig, GenericVirtualPackage, MatchSpec, PackageName, ParseMatchSpecOptions,
    Platform, RepoDataRecord,
};
use rattler_lock::{CondaPackageData, LockFileBuilder};
use rattler_networking::AuthenticationMiddleware;
use rattler_repodata_gateway::{Gateway, RepoData, SourceConfig};
use rattler_solve::{SolverImpl, SolverTask, resolvo};
use sha2::{Digest, Sha256};

#[derive(serde::Deserialize)]
struct PixiToml {
    tool: ToolSection,
}

#[derive(serde::Deserialize)]
struct ToolSection {
    cx: CxConfig,
}

#[derive(serde::Deserialize)]
struct CxConfig {
    channels: Vec<String>,
    packages: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
}

fn main() {
    println!("cargo:rerun-if-changed=pixi.toml");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let config_path = manifest_dir.join("pixi.toml");
    let lock_path = out_dir.join("cx.lock");
    let hash_path = out_dir.join("cx.lock.hash");

    let config_contents = std::fs::read_to_string(&config_path).expect("failed to read pixi.toml");
    let config: PixiToml = toml::from_str(&config_contents).expect("failed to parse pixi.toml");

    let input_hash = {
        let mut hasher = Sha256::new();
        hasher.update(config_contents.as_bytes());
        format!("{:x}", hasher.finalize())
    };

    if lock_path.exists() && hash_path.exists() {
        let stored_hash = std::fs::read_to_string(&hash_path).unwrap_or_default();
        if stored_hash.trim() == input_hash {
            eprintln!("cx: lockfile is fresh, skipping solve");
            return;
        }
    }

    eprintln!("cx: solving packages at compile time...");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to create tokio runtime");

    let lock_content = runtime
        .block_on(solve_and_lock(&config.tool.cx))
        .expect("cx: failed to solve");

    std::fs::write(&lock_path, &lock_content).expect("failed to write cx.lock");
    std::fs::write(&hash_path, &input_hash).expect("failed to write hash file");
    eprintln!("cx: lockfile written to {}", lock_path.display());
}

/// Fetch repodata, solve, filter exclusions, and produce a lockfile string.
async fn solve_and_lock(config: &CxConfig) -> Result<String, Box<dyn std::error::Error>> {
    let channel_config = ChannelConfig::default_with_root_dir(env::current_dir()?);
    let platform = Platform::current();

    let match_specs: Vec<MatchSpec> = config
        .packages
        .iter()
        .map(|s| MatchSpec::from_str(s, ParseMatchSpecOptions::default()))
        .collect::<Result<Vec<_>, _>>()?;

    let cache_dir = default_cache_dir().map_err(|e| format!("cache dir: {e}"))?;
    rattler_cache::ensure_cache_dir(&cache_dir).map_err(|e| format!("create cache dir: {e}"))?;

    let parsed_channels: Vec<Channel> = config
        .channels
        .iter()
        .map(|c| Channel::from_str(c, &channel_config))
        .collect::<Result<Vec<_>, _>>()?;

    let raw_client = reqwest::Client::builder().no_gzip().build()?;
    let client = reqwest_middleware::ClientBuilder::new(raw_client.clone())
        .with_arc(Arc::new(AuthenticationMiddleware::from_env_and_defaults()?))
        .with(rattler_networking::OciMiddleware::new(raw_client))
        .build();

    let gateway = Gateway::builder()
        .with_cache_dir(cache_dir.join(rattler_cache::REPODATA_CACHE_DIR))
        .with_package_cache(PackageCache::new(
            cache_dir.join(rattler_cache::PACKAGE_CACHE_DIR),
        ))
        .with_client(client)
        .with_channel_config(rattler_repodata_gateway::ChannelConfig {
            default: SourceConfig {
                sharded_enabled: true,
                ..SourceConfig::default()
            },
            per_channel: HashMap::new(),
        })
        .finish();

    let start = Instant::now();
    let repo_data = gateway
        .query(
            parsed_channels.clone(),
            [platform, Platform::NoArch],
            match_specs.clone(),
        )
        .recursive(true)
        .await?;

    let total_records: usize = repo_data.iter().map(RepoData::len).sum();
    eprintln!(
        "cx: loaded {} records in {:.1}s",
        total_records,
        start.elapsed().as_secs_f64()
    );

    let virtual_packages = rattler_virtual_packages::VirtualPackage::detect(
        &rattler_virtual_packages::VirtualPackageOverrides::default(),
    )?
    .iter()
    .map(|vpkg| GenericVirtualPackage::from(vpkg.clone()))
    .collect::<Vec<_>>();

    let solver_task = SolverTask {
        virtual_packages,
        specs: match_specs,
        ..SolverTask::from_iter(&repo_data)
    };

    eprintln!("cx: solving...");
    let solved = resolvo::Solver.solve(solver_task)?;
    eprintln!("cx: solved {} packages", solved.records.len());

    let required_packages = if config.exclude.is_empty() {
        solved.records
    } else {
        let (filtered, removed) = filter_excluded_packages(solved.records, &config.exclude);
        eprintln!(
            "cx: excluded {} packages ({})",
            removed.len(),
            removed.join(", ")
        );
        filtered
    };

    eprintln!(
        "cx: writing lockfile with {} packages",
        required_packages.len()
    );

    let channel_urls: Vec<String> = parsed_channels
        .iter()
        .map(|c| c.base_url.to_string())
        .collect();

    let mut builder = LockFileBuilder::new();
    builder.set_channels(
        "default",
        channel_urls.into_iter().map(rattler_lock::Channel::from),
    );

    for record in &required_packages {
        let conda_data = CondaPackageData::from(record.clone());
        builder.add_conda_package("default", platform, conda_data);
    }

    let lock_file = builder.finish();
    Ok(lock_file.render_to_string()?)
}

/// Remove explicitly excluded packages and any of their dependencies that are
/// not required by any remaining package.
fn filter_excluded_packages(
    packages: Vec<RepoDataRecord>,
    excludes: &[String],
) -> (Vec<RepoDataRecord>, Vec<String>) {
    let exclude_set: HashSet<&str> = excludes.iter().map(|s| s.as_str()).collect();

    let name_of = |r: &RepoDataRecord| r.package_record.name.as_normalized().to_string();
    let pkg_names: Vec<String> = packages.iter().map(name_of).collect();
    let name_to_idx: HashMap<&str, usize> = pkg_names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();

    let n = packages.len();
    let mut reverse_deps: Vec<HashSet<usize>> = vec![HashSet::new(); n];
    for (i, rec) in packages.iter().enumerate() {
        for dep_str in &rec.package_record.depends {
            let dep_name = PackageName::from_matchspec_str_unchecked(dep_str);
            if let Some(&dep_idx) = name_to_idx.get(dep_name.as_normalized()) {
                reverse_deps[dep_idx].insert(i);
            }
        }
    }

    let mut removed: HashSet<usize> = HashSet::new();
    let mut queue: Vec<usize> = Vec::new();
    for (i, name) in pkg_names.iter().enumerate() {
        if exclude_set.contains(name.as_str()) {
            removed.insert(i);
            queue.push(i);
        }
    }

    while let Some(pkg_idx) = queue.pop() {
        for dep_str in &packages[pkg_idx].package_record.depends {
            let dep_name = PackageName::from_matchspec_str_unchecked(dep_str);
            if let Some(&dep_idx) = name_to_idx.get(dep_name.as_normalized()) {
                if removed.contains(&dep_idx) {
                    continue;
                }
                let all_dependents_removed = reverse_deps[dep_idx]
                    .iter()
                    .all(|rdep| removed.contains(rdep));
                if all_dependents_removed {
                    removed.insert(dep_idx);
                    queue.push(dep_idx);
                }
            }
        }
    }

    let removed_names: Vec<String> = removed.iter().map(|&i| pkg_names[i].clone()).collect();
    let filtered: Vec<RepoDataRecord> = packages
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !removed.contains(i))
        .map(|(_, r)| r)
        .collect();

    let mut sorted_names = removed_names;
    sorted_names.sort();
    (filtered, sorted_names)
}
