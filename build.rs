//! Build script that solves conda dependencies at compile time and embeds a
//! rattler-lock v6 lockfile into the binary.
//!
//! Reads the `[tool.cx]` section from `pixi.toml` and uses a
//! content-hash to cache the lockfile: if the config hasn't changed since
//! the last build, the solve is skipped entirely.

use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

#[path = "src/exclude.rs"]
mod exclude;

use rattler::{default_cache_dir, package_cache::PackageCache};
use rattler_conda_types::{
    Channel, ChannelConfig, GenericVirtualPackage, MatchSpec, ParseMatchSpecOptions, Platform,
    RepoDataRecord,
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
    let checked_in_lock = manifest_dir.join("cx.lock");
    let lock_path = out_dir.join("cx.lock");
    let hash_path = out_dir.join("cx.lock.hash");
    let payload_path = out_dir.join("payload.tar.zst");

    println!("cargo:rerun-if-changed=cx.lock");

    let config_contents = std::fs::read_to_string(&config_path).expect("failed to read pixi.toml");
    let mut config: PixiToml = toml::from_str(&config_contents).expect("failed to parse pixi.toml");

    println!("cargo:rerun-if-env-changed=CX_PACKAGES");
    println!("cargo:rerun-if-env-changed=CX_CHANNELS");
    println!("cargo:rerun-if-env-changed=CX_EXCLUDE");
    println!("cargo:rerun-if-env-changed=CX_PLATFORM");
    println!("cargo:rerun-if-env-changed=CX_EMBED_PAYLOAD");
    println!("cargo:rerun-if-env-changed=CX_INSTALL_METHOD");

    let embed_payload = env::var("CX_EMBED_PAYLOAD").ok().is_some_and(|v| v == "1");

    let env_packages = env::var("CX_PACKAGES").ok().filter(|v| !v.is_empty());
    let env_channels = env::var("CX_CHANNELS").ok().filter(|v| !v.is_empty());
    let env_exclude = env::var("CX_EXCLUDE").ok().filter(|v| !v.is_empty());
    let env_platform = env::var("CX_PLATFORM").ok().filter(|v| !v.is_empty());
    let has_env_overrides = env_packages.is_some()
        || env_channels.is_some()
        || env_exclude.is_some()
        || env_platform.is_some();

    if let Some(ref val) = env_packages {
        config.tool.cx.packages = val
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        eprintln!("cx: CX_PACKAGES override: {:?}", config.tool.cx.packages);
    }
    if let Some(ref val) = env_channels {
        config.tool.cx.channels = val
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        eprintln!("cx: CX_CHANNELS override: {:?}", config.tool.cx.channels);
    }
    if let Some(ref val) = env_exclude {
        config.tool.cx.exclude = val
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        eprintln!("cx: CX_EXCLUDE override: {:?}", config.tool.cx.exclude);
    }

    let target_platform = if let Some(ref val) = env_platform {
        let p = val
            .parse::<Platform>()
            .unwrap_or_else(|_| panic!("cx: invalid CX_PLATFORM value: {val}"));
        eprintln!("cx: CX_PLATFORM override: {p}");
        p
    } else {
        Platform::current()
    };

    let input_hash = {
        let mut hasher = Sha256::new();
        hasher.update(config_contents.as_bytes());
        hasher.update(target_platform.as_str().as_bytes());
        if let Some(ref v) = env_packages {
            hasher.update(v.as_bytes());
        }
        if let Some(ref v) = env_channels {
            hasher.update(v.as_bytes());
        }
        if let Some(ref v) = env_exclude {
            hasher.update(v.as_bytes());
        }
        format!("{:x}", hasher.finalize())
    };

    // Fast path: use a checked-in cx.lock from the repo root if it exists
    // and the config hash matches. This avoids the network solve entirely.
    // Skipped when env var overrides are active (different package set).
    // Also skipped when CX_EMBED_PAYLOAD=1 (need the solve results to download packages).
    if !has_env_overrides && !embed_payload && checked_in_lock.exists() {
        let checked_in_hash_path = manifest_dir.join("cx.lock.hash");
        if checked_in_hash_path.exists() {
            let stored_hash = std::fs::read_to_string(&checked_in_hash_path).unwrap_or_default();
            if stored_hash.trim() == input_hash {
                eprintln!("cx: using checked-in cx.lock, skipping solve");
                std::fs::copy(&checked_in_lock, &lock_path).expect("failed to copy cx.lock");
                std::fs::write(&hash_path, &input_hash).expect("failed to write hash");
                ensure_payload_file(&payload_path, false);
                return;
            }
        }
    }

    // Second fast path: OUT_DIR cached lockfile from a previous build.
    // Also skipped when env var overrides are active or payload embedding is requested.
    if !has_env_overrides && !embed_payload && lock_path.exists() && hash_path.exists() {
        let stored_hash = std::fs::read_to_string(&hash_path).unwrap_or_default();
        if stored_hash.trim() == input_hash {
            eprintln!("cx: lockfile is fresh, skipping solve");
            ensure_payload_file(&payload_path, false);
            return;
        }
    }

    eprintln!("cx: solving packages at compile time...");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to create tokio runtime");

    let (lock_content, solved_records) = runtime
        .block_on(solve_and_lock(&config.tool.cx, target_platform))
        .expect("cx: failed to solve");

    std::fs::write(&lock_path, &lock_content).expect("failed to write cx.lock");
    std::fs::write(&hash_path, &input_hash).expect("failed to write hash file");

    // Write to the repo root so the lockfile can be checked in — but only
    // when no env var overrides are active (those produce a one-off lockfile).
    if !has_env_overrides {
        let repo_lock = manifest_dir.join("cx.lock");
        let repo_hash = manifest_dir.join("cx.lock.hash");
        std::fs::write(&repo_lock, &lock_content).expect("failed to write repo cx.lock");
        std::fs::write(&repo_hash, &input_hash).expect("failed to write repo hash");
        eprintln!(
            "cx: lockfile written to {} and {}",
            lock_path.display(),
            repo_lock.display()
        );
    } else {
        eprintln!("cx: lockfile written to {}", lock_path.display());
    }

    if embed_payload {
        runtime
            .block_on(download_and_bundle_payload(&solved_records, &payload_path))
            .expect("cx: failed to download/bundle payload");
    } else {
        ensure_payload_file(&payload_path, false);
    }
}

/// Write an empty payload file when CX_EMBED_PAYLOAD is not set, so
/// `include_bytes!` always has a valid target.
fn ensure_payload_file(path: &Path, force_empty: bool) {
    if force_empty || !path.exists() {
        std::fs::write(path, b"").expect("failed to write empty payload.tar.zst");
    }
}

/// Download all solved package archives and bundle them into a
/// zstd-compressed tar archive for embedding.
async fn download_and_bundle_payload(
    records: &[RepoDataRecord],
    payload_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let raw_client = reqwest::Client::builder().no_gzip().build()?;
    let client = reqwest_middleware::ClientBuilder::new(raw_client.clone())
        .with_arc(Arc::new(AuthenticationMiddleware::from_env_and_defaults()?))
        .with(rattler_networking::OciMiddleware::new(raw_client))
        .build();

    let start = Instant::now();
    let payload_dir = payload_path
        .parent()
        .expect("payload path has parent")
        .join("payload");
    std::fs::create_dir_all(&payload_dir)?;

    eprintln!(
        "cx: downloading {} packages for embedded payload...",
        records.len()
    );

    for record in records {
        let archive_name = record
            .url
            .path_segments()
            .and_then(|mut s| s.next_back())
            .unwrap_or("unknown");

        let dest = payload_dir.join(archive_name);

        if dest.exists() {
            if let Some(ref expected) = record.package_record.sha256 {
                let data = std::fs::read(&dest)
                    .map_err(|e| format!("failed to read {}: {e}", dest.display()))?;
                let actual = Sha256::digest(&data);
                if actual != *expected {
                    eprintln!("cx: SHA256 mismatch for {archive_name}, re-downloading");
                    std::fs::remove_file(&dest)
                        .map_err(|e| format!("failed to remove {}: {e}", dest.display()))?;
                } else {
                    continue;
                }
            } else {
                continue;
            }
        }

        let response = client
            .get(record.url.clone())
            .send()
            .await
            .map_err(|e| format!("failed to fetch {archive_name}: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            return Err(format!("HTTP {status} fetching {archive_name}").into());
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| format!("failed to read {archive_name}: {e}"))?;

        if let Some(ref expected) = record.package_record.sha256 {
            let actual = Sha256::digest(&bytes);
            if actual != *expected {
                return Err(format!("SHA256 mismatch for {archive_name}").into());
            }
        }

        std::fs::write(&dest, &bytes)
            .map_err(|e| format!("failed to write {}: {e}", dest.display()))?;
    }

    eprintln!(
        "cx: downloaded {} packages in {:.1}s, bundling...",
        records.len(),
        start.elapsed().as_secs_f64()
    );

    let bundle_start = Instant::now();
    let out_file = std::fs::File::create(payload_path)?;
    // Level 1: .conda archives are already zstd-compressed internally,
    // so the outer layer is just for bundling — minimal CPU, near-zero gain
    // from higher levels on pre-compressed data.
    let zstd_encoder = zstd::Encoder::new(out_file, 1)?;
    let mut tar_builder = tar::Builder::new(zstd_encoder);

    for entry in std::fs::read_dir(&payload_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let name = path.file_name().unwrap();
            tar_builder.append_path_with_name(&path, name)?;
        }
    }

    let zstd_encoder = tar_builder.into_inner()?;
    zstd_encoder.finish()?;

    let payload_size = std::fs::metadata(payload_path)?.len();
    eprintln!(
        "cx: payload.tar.zst = {:.1} MB ({} packages, bundled in {:.1}s)",
        payload_size as f64 / 1_048_576.0,
        records.len(),
        bundle_start.elapsed().as_secs_f64()
    );

    Ok(())
}

/// Fetch repodata, solve, filter exclusions, and produce a lockfile string
/// along with the solved records (for payload embedding).
async fn solve_and_lock(
    config: &CxConfig,
    platform: Platform,
) -> Result<(String, Vec<RepoDataRecord>), Box<dyn std::error::Error>> {
    let channel_config = ChannelConfig::default_with_root_dir(env::current_dir()?);

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

    let virtual_packages = if platform == Platform::current() {
        rattler_virtual_packages::VirtualPackage::detect(
            &rattler_virtual_packages::VirtualPackageOverrides::default(),
        )?
        .iter()
        .map(|vpkg| GenericVirtualPackage::from(vpkg.clone()))
        .collect::<Vec<_>>()
    } else {
        eprintln!("cx: cross-solving for {platform}, using no virtual packages");
        Vec::new()
    };

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
        let (filtered, removed) =
            exclude::filter_excluded_packages(solved.records, &config.exclude);
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
    Ok((lock_file.render_to_string()?, required_packages))
}
