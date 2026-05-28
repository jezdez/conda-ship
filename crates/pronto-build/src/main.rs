use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use clap::{Parser, Subcommand};
use rattler_conda_types::{PackageName, Platform};
use rattler_lock::{CondaPackageData, LockFile, LockFileBuilder};
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
    #[serde(default)]
    exclude: Vec<String>,
}

#[derive(Parser)]
#[command(name = "pronto-build", about = "Build helper for Pronto artifacts")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Extract cx.lock from pixi.lock's cx-env environment and apply exclude filters
    Prepare {
        /// Only verify cx.lock is up-to-date; exit 1 if stale
        #[arg(long)]
        check: bool,

        /// Project root (default: auto-detect from Cargo workspace)
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// Download packages from cx.lock and bundle them for embedded builds
    #[command(alias = "payload")]
    Bundle {
        /// Target platform (default: current)
        #[arg(long)]
        platform: Option<String>,

        /// Project root (default: auto-detect from Cargo workspace)
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// Override cx-env packages/channels/exclude in pixi.toml for custom builds
    Configure {
        /// Comma-separated conda package specs (replaces [feature.cx-env.dependencies])
        #[arg(long)]
        packages: Option<String>,

        /// Comma-separated conda channels (replaces `[workspace].channels`)
        #[arg(long)]
        channels: Option<String>,

        /// Comma-separated packages to exclude at runtime (replaces [tool.cx].exclude)
        #[arg(long)]
        exclude: Option<String>,

        /// Project root (default: auto-detect from Cargo workspace)
        #[arg(long)]
        root: Option<PathBuf>,
    },
}

fn project_root(override_root: Option<&Path>) -> PathBuf {
    if let Some(root) = override_root {
        return root.to_path_buf();
    }
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .ancestors()
        .find(|p| p.join("pixi.toml").exists())
        .expect("could not find project root containing pixi.toml")
        .to_path_buf()
}

fn prepare(check: bool, root_override: Option<PathBuf>) {
    let root = project_root(root_override.as_deref());
    let pixi_lock_path = root.join("pixi.lock");
    let cx_lock_path = root.join("cx.lock");
    let cx_hash_path = root.join("cx.lock.hash");
    let pixi_toml_path = root.join("pixi.toml");

    let pixi_toml = std::fs::read_to_string(&pixi_toml_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", pixi_toml_path.display()));
    let pixi_lock_content = std::fs::read_to_string(&pixi_lock_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", pixi_lock_path.display()));

    let input_hash = {
        let mut hasher = Sha256::new();
        hasher.update(pixi_toml.as_bytes());
        hasher.update(pixi_lock_content.as_bytes());
        let digest = hasher.finalize();
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };

    if check {
        if !cx_lock_path.exists() {
            eprintln!(
                "cx.lock does not exist; run `cargo run -p pronto-build -- prepare` to create it"
            );
            std::process::exit(1);
        }
        if !cx_hash_path.exists() {
            eprintln!(
                "cx.lock.hash does not exist; run `cargo run -p pronto-build -- prepare` to create it"
            );
            std::process::exit(1);
        }
        let stored_hash = std::fs::read_to_string(&cx_hash_path).unwrap_or_default();
        if stored_hash.trim() != input_hash {
            eprintln!(
                "cx.lock is stale (hash mismatch); run `cargo run -p pronto-build -- prepare` to update"
            );
            std::process::exit(1);
        }
        eprintln!("cx.lock is up-to-date");
        return;
    }

    let config: PixiToml =
        toml::from_str(&pixi_toml).expect("failed to parse [tool.cx] from pixi.toml");
    let excludes = &config.tool.cx.exclude;

    let pixi_lock = parse_pixi_lock(&pixi_lock_content, &pixi_lock_path);

    let cx_env = pixi_lock.environment("cx-env").unwrap_or_else(|| {
        panic!(
            "cx-env environment not found in {}",
            pixi_lock_path.display()
        )
    });

    let mut builder = LockFileBuilder::new();

    if !cx_env.channels().is_empty() {
        builder.set_channels("default", cx_env.channels().iter().cloned());
    }

    let mut total_packages = 0usize;
    let mut total_excluded = 0usize;

    for (platform, packages) in cx_env.conda_packages_by_platform() {
        let pkgs: Vec<_> = packages.cloned().collect();

        let filtered = if excludes.is_empty() {
            pkgs
        } else {
            let (kept, removed) = filter_excluded(&pkgs, excludes);
            if !removed.is_empty() {
                eprintln!(
                    "  {platform}: excluded {} packages ({})",
                    removed.len(),
                    removed.join(", ")
                );
            }
            total_excluded += removed.len();
            kept
        };

        total_packages += filtered.len();
        for pkg in filtered {
            builder.add_conda_package("default", platform, pkg);
        }
    }

    let new_lock = builder.finish();
    let new_content = new_lock
        .render_to_string()
        .expect("failed to render cx.lock");

    std::fs::write(&cx_lock_path, &new_content)
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", cx_lock_path.display()));
    std::fs::write(&cx_hash_path, &input_hash)
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", cx_hash_path.display()));

    let platforms: Vec<Platform> = cx_env.platforms().collect();
    eprintln!(
        "wrote cx.lock: {} packages across {} platforms (excluded {})",
        total_packages,
        platforms.len(),
        total_excluded
    );
}

fn parse_pixi_lock(pixi_lock_content: &str, pixi_lock_path: &Path) -> LockFile {
    let normalized_lock;
    let lock_content = if pixi_lock_content.starts_with("version: 7\n") {
        // Pixi lock v7 is backwards-compatible with rattler_lock's v6 parser for
        // the conda package data pronto-build consumes.
        normalized_lock = pixi_lock_content.replacen("version: 7\n", "version: 6\n", 1);
        normalized_lock.as_str()
    } else {
        pixi_lock_content
    };

    LockFile::from_str(lock_content)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", pixi_lock_path.display()))
}

/// Remove explicitly excluded packages and any transitive dependencies that
/// are not required by any remaining package.
fn filter_excluded(
    packages: &[CondaPackageData],
    excludes: &[String],
) -> (Vec<CondaPackageData>, Vec<String>) {
    let exclude_set: HashSet<&str> = excludes.iter().map(|s| s.as_str()).collect();

    let pkg_names: Vec<String> = packages
        .iter()
        .map(|p| p.record().name.as_normalized().to_string())
        .collect();
    let name_to_idx: HashMap<&str, usize> = pkg_names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();

    let n = packages.len();
    let mut reverse_deps: Vec<HashSet<usize>> = vec![HashSet::new(); n];
    for (i, pkg) in packages.iter().enumerate() {
        for dep_str in &pkg.record().depends {
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
        for dep_str in &packages[pkg_idx].record().depends {
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

    let mut removed_names: Vec<String> = removed.iter().map(|&i| pkg_names[i].clone()).collect();
    removed_names.sort();

    let filtered: Vec<CondaPackageData> = packages
        .iter()
        .enumerate()
        .filter(|(i, _)| !removed.contains(i))
        .map(|(_, p)| p.clone())
        .collect();

    (filtered, removed_names)
}

fn gen_bundle(platform_str: Option<String>, root_override: Option<PathBuf>) {
    let root = project_root(root_override.as_deref());
    let cx_lock_path = root.join("cx.lock");
    let payload_path = root.join("payload.tar.zst");

    let platform = if let Some(ref s) = platform_str {
        s.parse::<Platform>()
            .unwrap_or_else(|_| panic!("invalid platform: {s}"))
    } else {
        Platform::current()
    };

    let lock_file = LockFile::from_path(&cx_lock_path)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", cx_lock_path.display()));

    let env = lock_file
        .default_environment()
        .unwrap_or_else(|| panic!("no default environment in {}", cx_lock_path.display()));

    let packages: Vec<_> = env
        .conda_packages_by_platform()
        .filter(|(p, _)| *p == platform)
        .flat_map(|(_, pkgs)| pkgs)
        .collect();

    if packages.is_empty() {
        panic!(
            "no packages for platform {platform} in {}",
            cx_lock_path.display()
        );
    }

    eprintln!("downloading {} packages for {platform}...", packages.len());

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to create tokio runtime");

    rt.block_on(download_and_bundle(&packages, &payload_path))
        .expect("failed to download/bundle payload");
}

async fn download_and_bundle(
    packages: &[&rattler_lock::CondaPackageData],
    payload_path: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use futures::stream::{self, StreamExt};

    let client = reqwest::Client::builder().no_gzip().build()?;

    let payload_dir = payload_path
        .parent()
        .expect("payload path has parent")
        .join("payload");
    std::fs::create_dir_all(&payload_dir)?;

    let start = std::time::Instant::now();

    let download_tasks = packages.iter().map(|pkg| {
        let client = client.clone();
        let payload_dir = payload_dir.clone();
        async move {
            let url = pkg.location().as_url().expect("package has URL");
            let archive_name = url
                .path_segments()
                .and_then(|mut s| s.next_back())
                .unwrap_or("unknown");

            let dest = payload_dir.join(archive_name);

            if dest.exists() {
                if let Some(ref expected) = pkg.record().sha256 {
                    let data = std::fs::read(&dest)?;
                    let actual = Sha256::digest(&data);
                    if actual.as_slice() == expected.as_slice() {
                        return Ok::<(), Box<dyn std::error::Error + Send + Sync>>(());
                    }
                    eprintln!("SHA256 mismatch for {archive_name}, re-downloading");
                    std::fs::remove_file(&dest)?;
                } else {
                    return Ok(());
                }
            }

            let response = client
                .get(url.clone())
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

            if let Some(ref expected) = pkg.record().sha256 {
                let actual = Sha256::digest(&bytes);
                if actual.as_slice() != expected.as_slice() {
                    return Err(format!("SHA256 mismatch for {archive_name}").into());
                }
            }

            std::fs::write(&dest, &bytes)?;
            Ok(())
        }
    });

    let results: Vec<_> = stream::iter(download_tasks)
        .buffer_unordered(8)
        .collect()
        .await;

    for result in results {
        result?;
    }

    eprintln!(
        "downloaded {} packages in {:.1}s, bundling...",
        packages.len(),
        start.elapsed().as_secs_f64()
    );

    let bundle_start = std::time::Instant::now();
    let out_file = std::fs::File::create(payload_path)?;
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
        "payload.tar.zst = {:.1} MB ({} packages, bundled in {:.1}s)",
        payload_size as f64 / 1_048_576.0,
        packages.len(),
        bundle_start.elapsed().as_secs_f64()
    );

    Ok(())
}

fn configure(
    packages: Option<String>,
    channels: Option<String>,
    exclude: Option<String>,
    root_override: Option<PathBuf>,
) {
    let root = project_root(root_override.as_deref());
    let pixi_toml_path = root.join("pixi.toml");
    let content = std::fs::read_to_string(&pixi_toml_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", pixi_toml_path.display()));

    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", pixi_toml_path.display()));

    if let Some(ref pkgs) = packages {
        let specs: Vec<&str> = pkgs
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        let mut deps = toml_edit::Table::new();
        for spec in &specs {
            let (name, version) = match spec.split_once(' ') {
                Some((n, v)) => (n.trim(), v.trim().to_string()),
                None => (spec.trim(), "*".to_string()),
            };
            deps[name] = toml_edit::value(version);
        }
        doc["feature"]["cx-env"]["dependencies"] = toml_edit::Item::Table(deps);
        eprintln!("configured {} custom packages", specs.len());

        let mut tool_packages = toml_edit::Array::new();
        for spec in &specs {
            tool_packages.push(spec.to_string());
        }
        doc["tool"]["cx"]["packages"] = toml_edit::value(tool_packages);
    }

    if let Some(ref ch) = channels {
        let channel_list: Vec<&str> = ch
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        let mut arr = toml_edit::Array::new();
        for c in &channel_list {
            arr.push(c.to_string());
        }
        doc["workspace"]["channels"] = toml_edit::value(arr);

        let mut tool_channels = toml_edit::Array::new();
        for c in &channel_list {
            tool_channels.push(c.to_string());
        }
        doc["tool"]["cx"]["channels"] = toml_edit::value(tool_channels);
        eprintln!("configured channels: {}", channel_list.join(", "));
    }

    if let Some(ref ex) = exclude {
        let excludes: Vec<&str> = ex
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        let mut arr = toml_edit::Array::new();
        for e in &excludes {
            arr.push(e.to_string());
        }
        doc["tool"]["cx"]["exclude"] = toml_edit::value(arr);
        eprintln!("configured excludes: {}", excludes.join(", "));
    }

    std::fs::write(&pixi_toml_path, doc.to_string())
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", pixi_toml_path.display()));
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Prepare { check, root } => prepare(check, root),
        Command::Bundle { platform, root } => gen_bundle(platform, root),
        Command::Configure {
            packages,
            channels,
            exclude,
            root,
        } => configure(packages, channels, exclude, root),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rattler_conda_types::{PackageName, PackageRecord, VersionWithSource};
    use rattler_lock::CondaPackageData;
    use std::str::FromStr;

    fn make_pkg(name: &str, depends: &[&str]) -> CondaPackageData {
        let mut record = PackageRecord::new(
            PackageName::new_unchecked(name),
            VersionWithSource::from_str("1.0").unwrap(),
            "0".to_string(),
        );
        record.depends = depends.iter().map(|d| d.to_string()).collect();
        CondaPackageData::from(rattler_conda_types::RepoDataRecord {
            package_record: record,
            identifier: rattler_conda_types::package::DistArchiveIdentifier::from(
                format!("{name}-1.0-0.conda")
                    .parse::<rattler_conda_types::package::CondaArchiveIdentifier>()
                    .unwrap(),
            ),
            url: format!("https://example.com/{name}-1.0-0.conda")
                .parse()
                .unwrap(),
            channel: Some("test".to_string()),
        })
    }

    #[test]
    fn test_empty_excludes_returns_all() {
        let packages = vec![make_pkg("a", &[]), make_pkg("b", &["a"])];
        let (filtered, removed) = filter_excluded(&packages, &[]);
        assert!(removed.is_empty());
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_exclude_single_leaf() {
        let packages = vec![make_pkg("a", &[]), make_pkg("b", &[])];
        let excludes = vec!["b".to_string()];
        let (filtered, removed) = filter_excluded(&packages, &excludes);
        assert_eq!(removed, vec!["b"]);
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_exclude_with_transitive_deps() {
        let packages = vec![
            make_pkg("a", &["b"]),
            make_pkg("b", &["c"]),
            make_pkg("c", &[]),
        ];
        let excludes = vec!["a".to_string()];
        let (filtered, removed) = filter_excluded(&packages, &excludes);
        assert_eq!(removed, vec!["a", "b", "c"]);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_shared_dep_not_removed() {
        let packages = vec![
            make_pkg("a", &["c"]),
            make_pkg("b", &["c"]),
            make_pkg("c", &[]),
        ];
        let excludes = vec!["a".to_string()];
        let (filtered, removed) = filter_excluded(&packages, &excludes);
        assert_eq!(removed, vec!["a"]);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_exclude_nonexistent_package() {
        let packages = vec![make_pkg("a", &[]), make_pkg("b", &[])];
        let excludes = vec!["nonexistent".to_string()];
        let (filtered, removed) = filter_excluded(&packages, &excludes);
        assert!(removed.is_empty());
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_diamond_dependency() {
        let packages = vec![
            make_pkg("a", &["c"]),
            make_pkg("b", &["c"]),
            make_pkg("c", &[]),
            make_pkg("d", &["a"]),
        ];
        let excludes = vec!["d".to_string()];
        let (filtered, removed) = filter_excluded(&packages, &excludes);
        assert_eq!(removed, vec!["a", "d"]);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_multiple_simultaneous_excludes() {
        let packages = vec![
            make_pkg("a", &["shared"]),
            make_pkg("b", &["only-b"]),
            make_pkg("shared", &[]),
            make_pkg("only-b", &[]),
            make_pkg("keep", &[]),
        ];
        let excludes = vec!["a".to_string(), "b".to_string()];
        let (filtered, removed) = filter_excluded(&packages, &excludes);
        assert_eq!(removed, vec!["a", "b", "only-b", "shared"]);
        assert_eq!(filtered.len(), 1);
    }
}
