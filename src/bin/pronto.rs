use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use clap::{Parser, Subcommand, ValueEnum};
use rattler_conda_types::{PackageName, Platform};
use rattler_lock::{CondaPackageData, LockFile, LockFileBuilder};
use sha2::{Digest, Sha256};

#[path = "../runtime_data.rs"]
mod runtime_data;

#[derive(Clone, Default, serde::Deserialize)]
struct ProjectManifest {
    #[serde(default)]
    tool: ToolSection,
}

#[derive(Clone, Default, serde::Deserialize)]
struct ToolSection {
    #[serde(default)]
    pronto: ProntoConfig,
}

#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
struct ProntoConfig {
    #[serde(default)]
    channels: Vec<String>,
    #[serde(default)]
    packages: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    environment: Option<String>,
    #[serde(default, rename = "docs-url")]
    docs_url: Option<String>,
}

#[derive(Parser)]
#[command(name = "pronto", about = "Build ready-to-run conda bootstrap binaries")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Derive the runtime lock from the configured lockfile environment and filters
    Lock {
        /// Only verify that the runtime lock can be derived; do not write it
        #[arg(long)]
        check: bool,

        /// Project root (default: auto-detect from current directory)
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// Download packages from the derived runtime lock and bundle them
    Bundle {
        /// Target platform (default: current)
        #[arg(long)]
        platform: Option<String>,

        /// Project root (default: auto-detect from current directory)
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// Build and stage a ready-to-run artifact
    Build {
        /// Artifact layout to produce
        #[arg(long, value_enum, default_value_t = BundleLayout::None)]
        layout: BundleLayout,

        /// Distribution binary name to stage
        #[arg(long)]
        name: String,

        /// Optional target label appended to staged artifact names
        #[arg(long)]
        target_label: Option<String>,

        /// Conda platform to bundle/describe (default: current)
        #[arg(long)]
        platform: Option<String>,

        /// Rust target triple to pass to cargo build
        #[arg(long)]
        target: Option<String>,

        /// Output directory for staged artifacts
        #[arg(long, default_value = "dist")]
        out_dir: PathBuf,

        /// Project root (default: auto-detect from current directory)
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// Build and run a staged artifact for local smoke testing
    Run {
        /// Artifact layout to produce before running
        #[arg(long, value_enum, default_value_t = BundleLayout::None)]
        layout: BundleLayout,

        /// Distribution binary name to stage
        #[arg(long)]
        name: String,

        /// Conda platform to bundle/describe (default: current)
        #[arg(long)]
        platform: Option<String>,

        /// Output directory for staged artifacts
        #[arg(long, default_value = "dist")]
        out_dir: PathBuf,

        /// Project root (default: auto-detect from current directory)
        #[arg(long)]
        root: Option<PathBuf>,

        /// Arguments passed to the staged runtime binary
        #[arg(last = true)]
        args: Vec<OsString>,
    },

    /// Inspect the derived runtime lock
    Inspect {
        /// Conda platform to inspect (default: current)
        #[arg(long)]
        platform: Option<String>,

        /// Emit JSON
        #[arg(long)]
        json: bool,

        /// Project root (default: auto-detect from current directory)
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// Override runtime packages, channels, or excludes in the project manifest
    Configure {
        /// Comma-separated conda package specs (replaces [feature.runtime.dependencies])
        #[arg(long)]
        packages: Option<String>,

        /// Comma-separated conda channels (replaces `[workspace].channels`)
        #[arg(long)]
        channels: Option<String>,

        /// Comma-separated packages to exclude at runtime (replaces [tool.pronto].exclude)
        #[arg(long)]
        exclude: Option<String>,

        /// Project root (default: auto-detect from current directory)
        #[arg(long)]
        root: Option<PathBuf>,
    },
}

const PRONTO_STATE_DIR: &str = "target/pronto";
const RUNTIME_LOCK_FILE: &str = "runtime.lock";
const BUNDLE_ARCHIVE_FILE: &str = "bundle.tar.zst";

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum BundleLayout {
    /// Binary contains lock/metadata; packages download during bootstrap.
    None,
    /// Binary is paired with a compressed package bundle.
    External,
    /// Binary contains the compressed package bundle.
    Embedded,
}

impl BundleLayout {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::External => "external",
            Self::Embedded => "embedded",
        }
    }

    fn needs_bundle(self) -> bool {
        matches!(self, Self::External | Self::Embedded)
    }
}

fn project_root(override_root: Option<&Path>) -> PathBuf {
    if let Some(root) = override_root {
        return root.to_path_buf();
    }

    let current_dir = std::env::current_dir().expect("failed to read current directory");
    find_project_root(&current_dir)
        .or_else(|| find_project_root(&PathBuf::from(env!("CARGO_MANIFEST_DIR"))))
        .expect("could not find project root containing conda.toml or pixi.toml")
}

fn find_project_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|p| p.join("conda.toml").exists() || p.join("pixi.toml").exists())
        .map(Path::to_path_buf)
}

struct DerivedRuntimeLock {
    lock_file: LockFile,
    content: String,
    runtime_config: ProntoConfig,
    platforms: Vec<Platform>,
    total_packages: usize,
    total_excluded: usize,
}

struct ProjectInput {
    lock_path: PathBuf,
    config: ProntoConfig,
}

fn write_runtime_lock(check: bool, root_override: Option<PathBuf>) {
    let root = project_root(root_override.as_deref());
    let derived = derive_runtime_lock(&root);

    if check {
        eprintln!(
            "runtime lock can be derived: {} packages across {} platforms (excluded {})",
            derived.total_packages,
            derived.platforms.len(),
            derived.total_excluded
        );
        return;
    }

    let runtime_lock_path = generated_runtime_lock_path(&root);
    write_generated_runtime_lock(&runtime_lock_path, &derived.content);
    eprintln!(
        "wrote {}: {} packages across {} platforms (excluded {})",
        runtime_lock_path.display(),
        derived.total_packages,
        derived.platforms.len(),
        derived.total_excluded
    );
}

fn derive_runtime_lock(root: &Path) -> DerivedRuntimeLock {
    let input = discover_project_input(root);
    let lock_content = std::fs::read_to_string(&input.lock_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", input.lock_path.display()));

    let lock_file = parse_lock(&lock_content, &input.lock_path);

    let runtime_env = if let Some(environment) = input.config.environment.as_deref() {
        lock_file.environment(environment).unwrap_or_else(|| {
            panic!(
                "environment {environment:?} not found in {}",
                input.lock_path.display()
            )
        })
    } else if let Some(environment) = lock_file.environment("runtime") {
        environment
    } else {
        lock_file.default_environment().unwrap_or_else(|| {
            panic!(
                "no runtime or default environment found in {}",
                input.lock_path.display()
            )
        })
    };

    let mut builder = LockFileBuilder::new();
    let platforms: Vec<Platform> = runtime_env.platforms().collect();
    let mut runtime_config = input.config.clone();

    if !runtime_env.channels().is_empty() {
        builder.set_channels("default", runtime_env.channels().iter().cloned());
    }
    if runtime_config.channels.is_empty() {
        runtime_config.channels = runtime_env
            .channels()
            .iter()
            .map(|channel| channel.url.clone())
            .collect();
    }

    let mut total_packages = 0usize;
    let mut total_excluded = 0usize;
    let mut resolved_package_names = HashSet::new();

    for (platform, packages) in runtime_env.conda_packages_by_platform() {
        let pkgs: Vec<_> = packages.cloned().collect();

        let filtered = if input.config.exclude.is_empty() {
            pkgs
        } else {
            let (kept, removed) = filter_excluded(&pkgs, &input.config.exclude);
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
            resolved_package_names.insert(pkg.record().name.as_normalized().to_string());
            builder.add_conda_package("default", platform, pkg);
        }
    }
    if runtime_config.packages.is_empty() {
        runtime_config.packages = resolved_package_names.into_iter().collect();
        runtime_config.packages.sort();
    }

    let new_lock = builder.finish();
    let new_content = new_lock
        .render_to_string()
        .expect("failed to render runtime lock");

    DerivedRuntimeLock {
        lock_file: new_lock,
        content: new_content,
        runtime_config,
        platforms,
        total_packages,
        total_excluded,
    }
}

fn discover_project_input(root: &Path) -> ProjectInput {
    let manifest_path = discover_manifest_path(root);

    let lock_path = if manifest_path.file_name().and_then(|n| n.to_str()) == Some("conda.toml") {
        root.join("conda.lock")
    } else {
        root.join("pixi.lock")
    };
    if !lock_path.exists() {
        let lock_command = if lock_path.file_name().and_then(|n| n.to_str()) == Some("conda.lock") {
            "conda workspace lock"
        } else {
            "pixi lock"
        };
        panic!(
            "lockfile not found at {}; run `{lock_command}` first",
            lock_path.display()
        );
    }

    let manifest = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", manifest_path.display()));
    let config: ProjectManifest = toml::from_str(&manifest)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", manifest_path.display()));

    ProjectInput {
        lock_path,
        config: config.tool.pronto,
    }
}

fn discover_manifest_path(root: &Path) -> PathBuf {
    if root.join("conda.toml").exists() {
        root.join("conda.toml")
    } else if root.join("pixi.toml").exists() {
        root.join("pixi.toml")
    } else {
        panic!(
            "could not find conda.toml or pixi.toml in {}",
            root.display()
        );
    }
}

fn write_generated_runtime_lock(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("failed to create {}: {e}", parent.display()));
    }
    std::fs::write(path, content)
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", path.display()));
}

fn parse_lock(lock_content: &str, lock_path: &Path) -> LockFile {
    let normalized_lock;
    let lock_content = if lock_content.starts_with("version: 7\n") {
        // Pixi lock v7 is backwards-compatible with rattler_lock's v6 parser
        // for the conda package data `pronto lock` consumes.
        normalized_lock = lock_content.replacen("version: 7\n", "version: 6\n", 1);
        normalized_lock.as_str()
    } else if lock_content.starts_with("version: 1\n") {
        // conda-workspaces writes the rattler-lock schema with a conda-facing
        // on-disk version. The conda package records are compatible here.
        normalized_lock = lock_content.replacen("version: 1\n", "version: 6\n", 1);
        normalized_lock.as_str()
    } else {
        lock_content
    };

    LockFile::from_str(lock_content)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", lock_path.display()))
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
    let derived = derive_runtime_lock(&root);
    let runtime_lock_path = generated_runtime_lock_path(&root);
    write_generated_runtime_lock(&runtime_lock_path, &derived.content);
    let bundle_path = generated_bundle_path(&root);

    let platform = parse_platform(platform_str);
    gen_bundle_from_lock(
        &derived.lock_file,
        &runtime_lock_path,
        platform,
        &bundle_path,
    );
}

fn gen_bundle_from_lock(
    lock_file: &LockFile,
    runtime_lock_path: &Path,
    platform: Platform,
    bundle_path: &Path,
) -> PathBuf {
    let env = lock_file
        .default_environment()
        .unwrap_or_else(|| panic!("no default environment in {}", runtime_lock_path.display()));

    let packages: Vec<_> = env
        .conda_packages_by_platform()
        .filter(|(p, _)| *p == platform)
        .flat_map(|(_, pkgs)| pkgs)
        .collect();

    if packages.is_empty() {
        panic!(
            "no packages for platform {platform} in {}",
            runtime_lock_path.display()
        );
    }

    eprintln!("downloading {} packages for {platform}...", packages.len());

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to create tokio runtime");

    rt.block_on(download_and_bundle(&packages, bundle_path))
        .expect("failed to download bundle");
    bundle_path.to_path_buf()
}

async fn download_and_bundle(
    packages: &[&rattler_lock::CondaPackageData],
    bundle_path: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use futures::stream::{self, StreamExt};

    let client = reqwest::Client::builder().no_gzip().build()?;

    let bundle_dir = bundle_path
        .parent()
        .expect("bundle path has parent")
        .join("bundle");
    std::fs::create_dir_all(bundle_path.parent().expect("bundle path has parent"))?;
    std::fs::create_dir_all(&bundle_dir)?;

    let start = std::time::Instant::now();

    let download_tasks = packages.iter().map(|pkg| {
        let client = client.clone();
        let bundle_dir = bundle_dir.clone();
        async move {
            let url = pkg.location().as_url().expect("package has URL");
            let archive_name = url
                .path_segments()
                .and_then(|mut s| s.next_back())
                .unwrap_or("unknown");

            let dest = bundle_dir.join(archive_name);

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
    let out_file = std::fs::File::create(bundle_path)?;
    let zstd_encoder = zstd::Encoder::new(out_file, 1)?;
    let mut tar_builder = tar::Builder::new(zstd_encoder);

    for entry in std::fs::read_dir(&bundle_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let name = path.file_name().unwrap();
            tar_builder.append_path_with_name(&path, name)?;
        }
    }

    let zstd_encoder = tar_builder.into_inner()?;
    zstd_encoder.finish()?;

    let bundle_size = std::fs::metadata(bundle_path)?.len();
    eprintln!(
        "bundle.tar.zst = {:.1} MB ({} packages, bundled in {:.1}s)",
        bundle_size as f64 / 1_048_576.0,
        packages.len(),
        bundle_start.elapsed().as_secs_f64()
    );

    Ok(())
}

#[derive(Debug)]
struct BuildOutput {
    binary: PathBuf,
    bundle: Option<PathBuf>,
    info: PathBuf,
    checksums: PathBuf,
    lock: PathBuf,
    package_list: PathBuf,
}

#[derive(serde::Serialize)]
struct ArtifactChecksum {
    path: String,
    sha256: String,
    bytes: u64,
}

#[derive(serde::Serialize)]
struct ArtifactInfo {
    schema_version: u8,
    name: String,
    layout: String,
    platform: String,
    binary: String,
    bundle: Option<String>,
    lock: String,
    package_list: String,
    package_count: usize,
    checksums: Vec<ArtifactChecksum>,
}

#[derive(serde::Serialize)]
struct LockInspect {
    lock: String,
    selected_platform: String,
    platforms: Vec<PlatformSummary>,
    packages: Vec<PackageInfo>,
}

#[derive(serde::Serialize)]
struct PlatformSummary {
    platform: String,
    packages: usize,
}

#[derive(serde::Serialize)]
struct PackageInfo {
    name: String,
    version: String,
    build: String,
    url: String,
    sha256: Option<String>,
}

fn build_artifact(
    layout: BundleLayout,
    name: String,
    target_label: Option<String>,
    platform_str: Option<String>,
    target: Option<String>,
    out_dir: PathBuf,
    root_override: Option<PathBuf>,
) -> BuildOutput {
    validate_artifact_name(&name);
    let root = project_root(root_override.as_deref());
    let platform = parse_platform(platform_str);

    let derived = derive_runtime_lock(&root);
    let runtime_lock_path = generated_runtime_lock_path(&root);
    write_generated_runtime_lock(&runtime_lock_path, &derived.content);

    let generated_bundle = layout.needs_bundle().then(|| {
        gen_bundle_from_lock(
            &derived.lock_file,
            &runtime_lock_path,
            platform,
            &generated_bundle_path(&root),
        )
    });

    run_cargo_build(&root, target.as_deref());
    stage_artifacts(
        &root,
        layout,
        &name,
        target_label.as_deref(),
        platform,
        target.as_deref(),
        &out_dir,
        &derived,
        generated_bundle.as_deref(),
    )
}

fn run_artifact(
    layout: BundleLayout,
    name: String,
    platform: Option<String>,
    out_dir: PathBuf,
    root_override: Option<PathBuf>,
    args: Vec<OsString>,
) {
    let root = project_root(root_override.as_deref());
    let bundle_env_var = runtime_env_var(&name, "BUNDLE");
    let output = build_artifact(
        layout,
        name,
        None,
        platform,
        None,
        out_dir,
        Some(root.clone()),
    );

    let mut command = std::process::Command::new(&output.binary);
    command.args(args);
    if layout == BundleLayout::External {
        command.env(bundle_env_var, root.join("bundle"));
    }

    let status = command
        .status()
        .unwrap_or_else(|e| panic!("failed to run {}: {e}", output.binary.display()));
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn inspect_artifact(platform_str: Option<String>, json: bool, root_override: Option<PathBuf>) {
    let root = project_root(root_override.as_deref());
    let derived = derive_runtime_lock(&root);
    let platform = parse_platform(platform_str);
    let runtime_lock_path = generated_runtime_lock_path(&root);

    let summaries = platform_summaries(&derived.lock_file, &runtime_lock_path);
    let packages = packages_for_platform(&derived.lock_file, &runtime_lock_path, platform);
    let package_infos = package_infos(&packages);

    if json {
        let inspect = LockInspect {
            lock: "derived runtime lock".to_string(),
            selected_platform: platform.to_string(),
            platforms: summaries,
            packages: package_infos,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&inspect).expect("failed to render inspect JSON")
        );
        return;
    }

    println!("derived runtime lock");
    for summary in summaries {
        println!("  {}: {} packages", summary.platform, summary.packages);
    }
    println!();
    println!("{platform}: {} packages", package_infos.len());
    for package in package_infos {
        println!("  {} {} {}", package.name, package.version, package.build);
    }
}

fn run_cargo_build(root: &Path, target: Option<&str>) {
    let mut command = std::process::Command::new("cargo");
    command
        .arg("build")
        .arg("--release")
        .arg("--bin")
        .arg("pronto-runtime")
        .arg("--features")
        .arg("runtime-template")
        .current_dir(root);
    if let Some(target) = target {
        command.arg("--target").arg(target);
    }

    let status = command.status().expect("failed to run cargo build");
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

#[allow(clippy::too_many_arguments)]
fn stage_artifacts(
    root: &Path,
    layout: BundleLayout,
    name: &str,
    target_label: Option<&str>,
    platform: Platform,
    target: Option<&str>,
    out_dir: &Path,
    derived: &DerivedRuntimeLock,
    generated_bundle: Option<&Path>,
) -> BuildOutput {
    let out_dir = resolve_out_dir(root, out_dir);
    std::fs::create_dir_all(&out_dir)
        .unwrap_or_else(|e| panic!("failed to create {}: {e}", out_dir.display()));

    let stem = artifact_stem(name, layout, target_label);
    let binary_name = binary_filename(&stem, target);
    let source_binary = source_binary_path(root, target);
    let binary = out_dir.join(binary_name);
    std::fs::copy(&source_binary, &binary).unwrap_or_else(|e| {
        panic!(
            "failed to copy {} to {}: {e}",
            source_binary.display(),
            binary.display()
        )
    });
    stamp_runtime_data(&binary, layout, name, derived, generated_bundle);

    let bundle = if layout == BundleLayout::External {
        let source_bundle = generated_bundle.expect("external builds require a generated bundle");
        let staged_bundle = out_dir.join(format!("{stem}.bundle.tar.zst"));
        std::fs::copy(source_bundle, &staged_bundle).unwrap_or_else(|e| {
            panic!(
                "failed to copy {} to {}: {e}",
                source_bundle.display(),
                staged_bundle.display()
            )
        });
        Some(staged_bundle)
    } else {
        None
    };

    let metadata = write_artifact_metadata(
        root,
        &out_dir,
        &stem,
        layout,
        platform,
        &binary,
        bundle.as_deref(),
        derived,
    );

    eprintln!("staged {}", binary.display());
    if let Some(bundle) = &bundle {
        eprintln!("staged {}", bundle.display());
    }
    eprintln!("wrote {}", metadata.info.display());
    eprintln!("wrote {}", metadata.checksums.display());

    BuildOutput {
        binary,
        bundle,
        info: metadata.info,
        checksums: metadata.checksums,
        lock: metadata.lock,
        package_list: metadata.package_list,
    }
}

#[derive(Debug)]
struct ArtifactMetadataPaths {
    info: PathBuf,
    checksums: PathBuf,
    lock: PathBuf,
    package_list: PathBuf,
}

#[allow(clippy::too_many_arguments)]
fn write_artifact_metadata(
    root: &Path,
    out_dir: &Path,
    stem: &str,
    layout: BundleLayout,
    platform: Platform,
    binary: &Path,
    bundle: Option<&Path>,
    derived: &DerivedRuntimeLock,
) -> ArtifactMetadataPaths {
    let runtime_lock_path = generated_runtime_lock_path(root);
    let packages = packages_for_platform(&derived.lock_file, &runtime_lock_path, platform);
    let package_infos = package_infos(&packages);

    let lock = out_dir.join(format!("{stem}.runtime.lock"));
    std::fs::write(&lock, &derived.content)
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", lock.display()));

    let package_list = out_dir.join(format!("{stem}.packages.txt"));
    let package_list_content = render_package_list(&package_infos);
    std::fs::write(&package_list, package_list_content)
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", package_list.display()));

    let mut checksums = vec![
        checksum_for_path(binary),
        checksum_for_path(&lock),
        checksum_for_path(&package_list),
    ];
    if let Some(bundle) = bundle {
        checksums.push(checksum_for_path(bundle));
    }

    let info = out_dir.join(format!("{stem}.info.json"));
    let info_doc = ArtifactInfo {
        schema_version: 1,
        name: stem.to_string(),
        layout: layout.as_str().to_string(),
        platform: platform.to_string(),
        binary: file_name(binary),
        bundle: bundle.map(file_name),
        lock: file_name(&lock),
        package_list: file_name(&package_list),
        package_count: package_infos.len(),
        checksums,
    };
    let info_json = serde_json::to_string_pretty(&info_doc).expect("failed to render info JSON");
    std::fs::write(&info, format!("{info_json}\n"))
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", info.display()));

    let mut final_checksums = info_doc.checksums;
    final_checksums.push(checksum_for_path(&info));
    final_checksums.sort_by(|a, b| a.path.cmp(&b.path));

    let checksums = out_dir.join(format!("{stem}.sha256"));
    write_checksums(&checksums, &final_checksums);

    ArtifactMetadataPaths {
        info,
        checksums,
        lock,
        package_list,
    }
}

fn generated_runtime_lock_path(root: &Path) -> PathBuf {
    root.join(PRONTO_STATE_DIR).join(RUNTIME_LOCK_FILE)
}

fn generated_bundle_path(root: &Path) -> PathBuf {
    root.join(PRONTO_STATE_DIR).join(BUNDLE_ARCHIVE_FILE)
}

fn parse_platform(platform_str: Option<String>) -> Platform {
    if let Some(ref platform) = platform_str {
        platform
            .parse::<Platform>()
            .unwrap_or_else(|_| panic!("invalid platform: {platform}"))
    } else {
        Platform::current()
    }
}

fn validate_artifact_name(name: &str) {
    assert!(!name.is_empty(), "artifact name must not be empty");
    assert!(
        !name.contains('/') && !name.contains('\\'),
        "artifact name must not contain path separators"
    );
    assert!(
        name.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')),
        "artifact name may only contain ASCII letters, digits, dots, dashes, and underscores"
    );
}

fn artifact_stem(name: &str, layout: BundleLayout, target_label: Option<&str>) -> String {
    let base = if layout == BundleLayout::Embedded {
        format!("{name}z")
    } else {
        name.to_string()
    };

    if let Some(label) = target_label {
        format!("{base}-{label}")
    } else {
        base
    }
}

fn binary_filename(stem: &str, target: Option<&str>) -> String {
    if target_is_windows(target) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

fn source_binary_path(root: &Path, target: Option<&str>) -> PathBuf {
    let mut path = root.join("target");
    if let Some(target) = target {
        path.push(target);
    }
    path.push("release");
    path.push(if target_is_windows(target) {
        "pronto-runtime.exe"
    } else {
        "pronto-runtime"
    });
    path
}

fn stamp_runtime_data(
    binary: &Path,
    layout: BundleLayout,
    name: &str,
    derived: &DerivedRuntimeLock,
    generated_bundle: Option<&Path>,
) {
    let command_name = if layout == BundleLayout::Embedded {
        format!("{name}z")
    } else {
        name.to_string()
    };
    let header = runtime_data::RuntimeDataHeader {
        schema_version: 1,
        command_name,
        embedded_command_name: format!("{name}z"),
        display_name: name.to_string(),
        default_prefix_dir: format!(".{name}"),
        metadata_file: format!(".{name}.json"),
        bundle_env_var: runtime_env_var(name, "BUNDLE"),
        offline_env_var: runtime_env_var(name, "OFFLINE"),
        docs_url: derived
            .runtime_config
            .docs_url
            .clone()
            .unwrap_or_else(|| "https://jezdez.github.io/conda-pronto/".to_string()),
        install_method: None,
        runtime_config: runtime_data::RuntimeConfig {
            channels: derived.runtime_config.channels.clone(),
            packages: derived.runtime_config.packages.clone(),
            exclude: derived.runtime_config.exclude.clone(),
        },
        runtime_lock: derived.content.clone(),
    };

    let embedded_bundle = (layout == BundleLayout::Embedded)
        .then(|| generated_bundle.expect("embedded builds require a generated bundle"));
    runtime_data::append_to_binary(binary, &header, embedded_bundle).unwrap_or_else(|e| {
        panic!(
            "failed to stamp runtime data onto {}: {e}",
            binary.display()
        )
    });
}

fn runtime_env_var(name: &str, suffix: &str) -> String {
    let prefix: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("{prefix}_{suffix}")
}

fn target_is_windows(target: Option<&str>) -> bool {
    target
        .map(|target| target.contains("windows"))
        .unwrap_or(cfg!(windows))
}

fn resolve_out_dir(root: &Path, out_dir: &Path) -> PathBuf {
    if out_dir.is_absolute() {
        out_dir.to_path_buf()
    } else {
        root.join(out_dir)
    }
}

fn platform_summaries(lock_file: &LockFile, runtime_lock_path: &Path) -> Vec<PlatformSummary> {
    let env = lock_file
        .default_environment()
        .unwrap_or_else(|| panic!("no default environment in {}", runtime_lock_path.display()));

    let mut summaries: Vec<_> = env
        .conda_packages_by_platform()
        .map(|(platform, packages)| PlatformSummary {
            platform: platform.to_string(),
            packages: packages.count(),
        })
        .collect();
    summaries.sort_by(|a, b| a.platform.cmp(&b.platform));
    summaries
}

fn packages_for_platform<'a>(
    lock_file: &'a LockFile,
    runtime_lock_path: &Path,
    platform: Platform,
) -> Vec<&'a CondaPackageData> {
    let env = lock_file
        .default_environment()
        .unwrap_or_else(|| panic!("no default environment in {}", runtime_lock_path.display()));

    let packages: Vec<_> = env
        .conda_packages_by_platform()
        .filter(|(p, _)| *p == platform)
        .flat_map(|(_, pkgs)| pkgs)
        .collect();

    if packages.is_empty() {
        panic!(
            "no packages for platform {platform} in {}",
            runtime_lock_path.display()
        );
    }

    packages
}

fn package_infos(packages: &[&CondaPackageData]) -> Vec<PackageInfo> {
    let mut infos: Vec<_> = packages
        .iter()
        .map(|pkg| {
            let record = pkg.record();
            PackageInfo {
                name: record.name.as_normalized().to_string(),
                version: record.version.to_string(),
                build: record.build.to_string(),
                url: pkg
                    .location()
                    .as_url()
                    .map(|url| url.to_string())
                    .unwrap_or_default(),
                sha256: record
                    .sha256
                    .as_ref()
                    .map(|hash| hex_bytes(hash.as_slice())),
            }
        })
        .collect();
    infos.sort_by(|a, b| a.name.cmp(&b.name).then(a.version.cmp(&b.version)));
    infos
}

fn render_package_list(packages: &[PackageInfo]) -> String {
    let mut content = String::from("name\tversion\tbuild\turl\tsha256\n");
    for package in packages {
        content.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            package.name,
            package.version,
            package.build,
            package.url,
            package.sha256.as_deref().unwrap_or("")
        ));
    }
    content
}

fn checksum_for_path(path: &Path) -> ArtifactChecksum {
    let data = std::fs::read(path)
        .unwrap_or_else(|e| panic!("failed to read {} for checksum: {e}", path.display()));
    ArtifactChecksum {
        path: file_name(path),
        sha256: hex_bytes(Sha256::digest(&data).as_slice()),
        bytes: data.len() as u64,
    }
}

fn write_checksums(path: &Path, checksums: &[ArtifactChecksum]) {
    let mut content = String::new();
    for checksum in checksums {
        content.push_str(&format!("{}  {}\n", checksum.sha256, checksum.path));
    }
    std::fs::write(path, content)
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", path.display()));
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_else(|| panic!("path has no file name: {}", path.display()))
        .to_string_lossy()
        .to_string()
}

fn configure(
    packages: Option<String>,
    channels: Option<String>,
    exclude: Option<String>,
    root_override: Option<PathBuf>,
) {
    let root = project_root(root_override.as_deref());
    let manifest_path = discover_manifest_path(&root);
    let content = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", manifest_path.display()));

    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", manifest_path.display()));

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
        doc["feature"]["runtime"]["dependencies"] = toml_edit::Item::Table(deps);
        eprintln!("configured {} custom packages", specs.len());

        let mut tool_packages = toml_edit::Array::new();
        for spec in &specs {
            tool_packages.push(spec.to_string());
        }
        doc["tool"]["pronto"]["packages"] = toml_edit::value(tool_packages);
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
        doc["tool"]["pronto"]["channels"] = toml_edit::value(tool_channels);
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
        doc["tool"]["pronto"]["exclude"] = toml_edit::value(arr);
        eprintln!("configured excludes: {}", excludes.join(", "));
    }

    std::fs::write(&manifest_path, doc.to_string())
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", manifest_path.display()));
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Lock { check, root } => write_runtime_lock(check, root),
        Command::Bundle { platform, root } => gen_bundle(platform, root),
        Command::Build {
            layout,
            name,
            target_label,
            platform,
            target,
            out_dir,
            root,
        } => {
            let output =
                build_artifact(layout, name, target_label, platform, target, out_dir, root);
            eprintln!("metadata {}", output.info.display());
            eprintln!("checksums {}", output.checksums.display());
            eprintln!("lock {}", output.lock.display());
            eprintln!("packages {}", output.package_list.display());
            if let Some(bundle) = output.bundle {
                eprintln!("bundle {}", bundle.display());
            }
        }
        Command::Run {
            layout,
            name,
            platform,
            out_dir,
            root,
            args,
        } => run_artifact(layout, name, platform, out_dir, root, args),
        Command::Inspect {
            platform,
            json,
            root,
        } => inspect_artifact(platform, json, root),
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

    #[test]
    fn test_artifact_stem_embedded_adds_z_before_target_label() {
        assert_eq!(
            artifact_stem("demo", BundleLayout::Embedded, Some("linux-64")),
            "demoz-linux-64"
        );
    }

    #[test]
    fn test_artifact_stem_external_keeps_base_name() {
        assert_eq!(
            artifact_stem("demo", BundleLayout::External, Some("linux-64")),
            "demo-linux-64"
        );
    }

    #[test]
    fn test_binary_filename_uses_windows_extension_for_target() {
        assert_eq!(
            binary_filename("demo", Some("x86_64-pc-windows-msvc")),
            "demo.exe"
        );
    }

    #[test]
    fn test_runtime_env_var_sanitizes_artifact_name() {
        assert_eq!(runtime_env_var("demo-tool", "BUNDLE"), "DEMO_TOOL_BUNDLE");
    }

    #[test]
    fn test_render_package_list_is_tab_separated() {
        let packages = vec![PackageInfo {
            name: "python".to_string(),
            version: "3.12.0".to_string(),
            build: "h123_0".to_string(),
            url: "https://example.com/python.conda".to_string(),
            sha256: Some("abc123".to_string()),
        }];

        assert_eq!(
            render_package_list(&packages),
            "name\tversion\tbuild\turl\tsha256\npython\t3.12.0\th123_0\thttps://example.com/python.conda\tabc123\n"
        );
    }
}
