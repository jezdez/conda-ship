use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use rattler_conda_types::{MatchSpec, PackageName, PackageRecord, ParseMatchSpecOptions, Platform};
use rattler_lock::{CondaPackageData, LockFile, LockFileBuilder, PlatformData};
use sha2::{Digest, Sha256};

#[path = "../runtime_data.rs"]
mod runtime_data;
#[path = "../tls.rs"]
mod tls;

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
#[serde(deny_unknown_fields)]
struct ProntoConfig {
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default, rename = "source-environment")]
    source_environment: Option<String>,
    #[serde(default, rename = "docs-url")]
    docs_url: Option<String>,
    #[serde(default)]
    scheme: Option<runtime_data::InstallScheme>,
    #[serde(default, rename = "install-name")]
    install_name: Option<String>,
}

#[derive(Clone, Default)]
struct RuntimeStampConfig {
    channels: Vec<String>,
    packages: Vec<String>,
    exclude: Vec<String>,
    docs_url: Option<String>,
    scheme: Option<runtime_data::InstallScheme>,
    install_name: Option<String>,
}

#[derive(Parser)]
#[command(name = "pronto", about = "Build ready-to-run conda runtimes")]
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

    /// Build and stage a ready-to-run runtime
    Build {
        /// Artifact layout to produce
        #[arg(long, value_enum, default_value_t = BundleLayout::Online)]
        layout: BundleLayout,

        /// Command name to stage for the generated runtime
        #[arg(long)]
        command: String,

        /// Optional target label appended to staged artifact names
        #[arg(long)]
        target_label: Option<String>,

        /// Conda platform to bundle/describe (default: current)
        #[arg(long)]
        platform: Option<String>,

        /// Rust target triple to pass to cargo build
        #[arg(long)]
        target: Option<String>,

        /// Prebuilt generic runtime template binary to stamp
        #[arg(long)]
        template: Option<PathBuf>,

        /// Documentation URL stamped into the generated runtime
        #[arg(long)]
        docs_url: Option<String>,

        /// Install scheme stamped into the generated runtime
        #[arg(long, value_enum)]
        scheme: Option<runtime_data::InstallScheme>,

        /// Install name used inside the install scheme
        #[arg(long)]
        install_name: Option<String>,

        /// Output directory for staged artifacts
        #[arg(long, default_value = "dist")]
        out_dir: PathBuf,

        /// Project root (default: auto-detect from current directory)
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// Build and run a staged runtime for local smoke testing
    Run {
        /// Artifact layout to produce before running
        #[arg(long, value_enum, default_value_t = BundleLayout::Online)]
        layout: BundleLayout,

        /// Command name to stage for the generated runtime
        #[arg(long)]
        command: String,

        /// Conda platform to bundle/describe (default: current)
        #[arg(long)]
        platform: Option<String>,

        /// Output directory for staged artifacts
        #[arg(long, default_value = "dist")]
        out_dir: PathBuf,

        /// Prebuilt generic runtime template binary to stamp
        #[arg(long)]
        template: Option<PathBuf>,

        /// Documentation URL stamped into the generated runtime
        #[arg(long)]
        docs_url: Option<String>,

        /// Install scheme stamped into the generated runtime
        #[arg(long, value_enum)]
        scheme: Option<runtime_data::InstallScheme>,

        /// Install name used inside the install scheme
        #[arg(long)]
        install_name: Option<String>,

        /// Project root (default: auto-detect from current directory)
        #[arg(long)]
        root: Option<PathBuf>,

        /// Arguments passed to the staged runtime
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

    /// Patch runtime environment packages, channels, or excludes in the project manifest
    Configure {
        /// Conda package matchspec to include; repeat for multiple packages
        #[arg(long = "package")]
        packages: Vec<String>,

        /// Conda channel name or URL to use; repeat for multiple channels
        #[arg(long = "channel")]
        channels: Vec<String>,

        /// Package name to exclude at runtime; repeat for multiple packages
        #[arg(long = "exclude")]
        exclude: Vec<String>,

        /// Project root (default: auto-detect from current directory)
        #[arg(long)]
        root: Option<PathBuf>,
    },
}

const PRONTO_STATE_DIR: &str = "target/pronto";
const RUNTIME_LOCK_FILE: &str = "runtime.lock";
const BUNDLE_ARCHIVE_FILE: &str = "bundle.tar.zst";
const REQUIRED_RUNTIME_PACKAGES: &[&str] = &["conda", "conda-spawn", "conda-rattler-solver"];

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum BundleLayout {
    /// Binary contains lock/metadata; packages download during bootstrap.
    Online,
    /// Binary is paired with a compressed package bundle.
    External,
    /// Binary contains the compressed package bundle.
    Embedded,
}

impl BundleLayout {
    fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
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
        .expect("could not find project root containing conda.toml, pixi.toml, or pyproject.toml")
}

fn find_project_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|p| has_supported_manifest(p))
        .map(Path::to_path_buf)
}

struct DerivedRuntimeLock {
    lock_file: LockFile,
    content: String,
    runtime_config: RuntimeStampConfig,
    platforms: Vec<Platform>,
    total_packages: usize,
    total_excluded: usize,
}

struct ProjectInput {
    lock_path: PathBuf,
    config: ProntoConfig,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ManifestKind {
    CondaToml,
    PixiToml,
    PixiPyproject,
}

impl ManifestKind {
    fn lockfile_name(self) -> &'static str {
        match self {
            Self::CondaToml => "conda.lock",
            Self::PixiToml | Self::PixiPyproject => "pixi.lock",
        }
    }

    fn lock_command(self) -> &'static str {
        match self {
            Self::CondaToml => "conda workspace lock",
            Self::PixiToml | Self::PixiPyproject => "pixi lock",
        }
    }
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

    let runtime_env = if let Some(source_environment) = input.config.source_environment.as_deref() {
        lock_file
            .environment(source_environment)
            .unwrap_or_else(|| {
                panic!(
                    "source environment {source_environment:?} not found in {}",
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

    let platform_data: Vec<_> = runtime_env
        .platforms()
        .map(|platform| PlatformData {
            name: platform.name().clone(),
            subdir: platform.subdir(),
            virtual_packages: platform.virtual_packages().to_vec(),
        })
        .collect();
    let platforms: Vec<Platform> = platform_data
        .iter()
        .map(|platform| platform.subdir)
        .collect();
    let mut builder = LockFileBuilder::new()
        .with_platforms(platform_data)
        .expect("failed to initialize runtime lock platforms");
    if !runtime_env.channels().is_empty() {
        builder.set_channels("default", runtime_env.channels().iter().cloned());
    }
    let runtime_channels = runtime_env
        .channels()
        .iter()
        .map(|channel| channel.url.clone())
        .collect();

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
                    "  {}: excluded {} packages ({})",
                    platform.name(),
                    removed.len(),
                    removed.join(", ")
                );
            }
            total_excluded += removed.len();
            kept
        };
        validate_required_runtime_packages(platform.name().as_str(), &filtered);

        total_packages += filtered.len();
        for pkg in filtered {
            resolved_package_names.insert(package_record(&pkg).name.as_normalized().to_string());
            builder
                .add_conda_package("default", platform.name().as_str(), pkg)
                .expect("failed to add package to runtime lock");
        }
    }
    let mut runtime_packages: Vec<_> = resolved_package_names.into_iter().collect();
    runtime_packages.sort();

    let new_lock = builder.finish();
    let new_content = new_lock
        .render_to_string()
        .expect("failed to render runtime lock");

    DerivedRuntimeLock {
        lock_file: new_lock,
        content: new_content,
        runtime_config: RuntimeStampConfig {
            channels: runtime_channels,
            packages: runtime_packages,
            exclude: input.config.exclude,
            docs_url: input.config.docs_url,
            scheme: input.config.scheme,
            install_name: input.config.install_name,
        },
        platforms,
        total_packages,
        total_excluded,
    }
}

fn validate_required_runtime_packages(platform: &str, packages: &[CondaPackageData]) {
    let package_names: HashSet<String> = packages
        .iter()
        .map(|pkg| package_record(pkg).name.as_normalized().to_string())
        .collect();
    let missing: Vec<_> = REQUIRED_RUNTIME_PACKAGES
        .iter()
        .copied()
        .filter(|name| !package_names.contains(*name))
        .collect();

    assert!(
        missing.is_empty(),
        "runtime environment for {platform} is missing required package(s): {}\n  Add them to the selected source environment or choose another source-environment.",
        missing.join(", ")
    );
}

fn discover_project_input(root: &Path) -> ProjectInput {
    let manifest_path = discover_manifest_path(root);
    let kind = manifest_kind(&manifest_path);

    let lock_path = root.join(kind.lockfile_name());
    if !lock_path.exists() {
        panic!(
            "lockfile not found at {}; run `{}` first",
            lock_path.display(),
            kind.lock_command()
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
    } else if is_supported_pyproject_manifest(&root.join("pyproject.toml")) {
        root.join("pyproject.toml")
    } else {
        panic!(
            "could not find conda.toml, pixi.toml, or Pixi pyproject.toml in {}",
            root.display()
        );
    }
}

fn has_supported_manifest(root: &Path) -> bool {
    root.join("conda.toml").exists()
        || root.join("pixi.toml").exists()
        || is_supported_pyproject_manifest(&root.join("pyproject.toml"))
}

fn manifest_kind(manifest_path: &Path) -> ManifestKind {
    match manifest_path.file_name().and_then(|n| n.to_str()) {
        Some("conda.toml") => ManifestKind::CondaToml,
        Some("pixi.toml") => ManifestKind::PixiToml,
        Some("pyproject.toml") => ManifestKind::PixiPyproject,
        _ => panic!("unsupported manifest path: {}", manifest_path.display()),
    }
}

fn is_supported_pyproject_manifest(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }

    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = toml::from_str::<toml::Value>(&content) else {
        return false;
    };

    has_toml_table(&value, &["tool", "pixi"])
}

fn has_toml_table(value: &toml::Value, path: &[&str]) -> bool {
    let Some((head, tail)) = path.split_first() else {
        return value.is_table();
    };
    value
        .get(*head)
        .is_some_and(|nested| has_toml_table(nested, tail))
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
    LockFile::from_str_with_base_directory(lock_content, lock_path.parent())
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", lock_path.display()))
}

fn package_record(package: &CondaPackageData) -> &PackageRecord {
    package
        .record()
        .expect("conda package in lockfile has no package record")
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
        .map(|p| package_record(p).name.as_normalized().to_string())
        .collect();
    let name_to_idx: HashMap<&str, usize> = pkg_names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();

    let n = packages.len();
    let mut reverse_deps: Vec<HashSet<usize>> = vec![HashSet::new(); n];
    for (i, pkg) in packages.iter().enumerate() {
        for dep_str in &package_record(pkg).depends {
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
        for dep_str in &package_record(&packages[pkg_idx]).depends {
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
        .filter(|(p, _)| p.subdir() == platform)
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

    tls::install_default_provider();
    let client = reqwest::Client::builder().no_gzip().build()?;

    let bundle_dir = bundle_path
        .parent()
        .expect("bundle path has parent")
        .join("bundle");
    std::fs::create_dir_all(bundle_path.parent().expect("bundle path has parent"))?;
    if let Ok(metadata) = std::fs::symlink_metadata(&bundle_dir) {
        if metadata.file_type().is_symlink() || metadata.is_file() {
            std::fs::remove_file(&bundle_dir)?;
        } else {
            std::fs::remove_dir_all(&bundle_dir)?;
        }
    }
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
                .ok_or_else(|| format!("package URL has no archive name: {url}"))?;
            validate_package_archive_name(archive_name)
                .map_err(|e| format!("invalid package archive name from {url}: {e}"))?;

            let dest = bundle_dir.join(archive_name);
            let expected = pkg
                .record()
                .expect("conda package in runtime lock has no package record")
                .sha256
                .as_ref()
                .ok_or_else(|| format!("{archive_name} has no SHA256 in the runtime lock"))?;

            if dest.exists() {
                let data = std::fs::read(&dest)?;
                let actual = Sha256::digest(&data);
                if actual.as_slice() == expected.as_slice() {
                    return Ok::<(), Box<dyn std::error::Error + Send + Sync>>(());
                }
                eprintln!("SHA256 mismatch for {archive_name}, re-downloading");
                std::fs::remove_file(&dest)?;
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

            let actual = Sha256::digest(&bytes);
            if actual.as_slice() != expected.as_slice() {
                return Err(format!("SHA256 mismatch for {archive_name}").into());
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

#[allow(clippy::too_many_arguments)]
fn build_artifact(
    layout: BundleLayout,
    command: String,
    target_label: Option<String>,
    platform_str: Option<String>,
    target: Option<String>,
    template: Option<PathBuf>,
    docs_url: Option<String>,
    scheme: Option<runtime_data::InstallScheme>,
    install_name: Option<String>,
    out_dir: PathBuf,
    root_override: Option<PathBuf>,
) -> BuildOutput {
    validate_command_name(&command);
    if let Some(label) = target_label.as_deref() {
        validate_target_label(label);
    }
    if let Some(target) = target.as_deref() {
        validate_target_triple(target);
    }
    let root = project_root(root_override.as_deref());
    let platform = parse_platform(platform_str);

    let mut derived = derive_runtime_lock(&root);
    if let Some(docs_url) = docs_url {
        derived.runtime_config.docs_url = Some(docs_url);
    }
    apply_install_location_overrides(&mut derived.runtime_config, scheme, install_name);
    validate_install_location_config(&derived.runtime_config, &command);
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

    let source_binary = match template {
        Some(path) => resolve_runtime_template(&path),
        None => {
            run_cargo_build(&root, target.as_deref());
            source_binary_path(&root, target.as_deref())
        }
    };

    stage_artifacts(
        &root,
        &source_binary,
        layout,
        &command,
        target_label.as_deref(),
        platform,
        target.as_deref(),
        &out_dir,
        &derived,
        generated_bundle.as_deref(),
    )
}

#[allow(clippy::too_many_arguments)]
fn run_artifact(
    layout: BundleLayout,
    command: String,
    platform: Option<String>,
    out_dir: PathBuf,
    template: Option<PathBuf>,
    docs_url: Option<String>,
    scheme: Option<runtime_data::InstallScheme>,
    install_name: Option<String>,
    root_override: Option<PathBuf>,
    args: Vec<OsString>,
) {
    let root = project_root(root_override.as_deref());
    let bundle_env_var = runtime_env_var(&command, "BUNDLE");
    let bundle_dir = root.join(PRONTO_STATE_DIR).join("bundle");
    let output = build_artifact(
        layout,
        command,
        None,
        platform,
        None,
        template,
        docs_url,
        scheme,
        install_name,
        out_dir,
        Some(root.clone()),
    );

    let mut command = std::process::Command::new(&output.binary);
    command.args(args);
    if layout == BundleLayout::External {
        command.env(bundle_env_var, bundle_dir);
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
    source_binary: &Path,
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
    let binary = out_dir.join(binary_name);
    std::fs::copy(source_binary, &binary).unwrap_or_else(|e| {
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

fn validate_package_archive_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("archive name must not be empty");
    }
    if name == "." || name == ".." {
        return Err("archive name must not be . or ..");
    }
    if name.contains('/') || name.contains('\\') || name.chars().any(char::is_control) {
        return Err("archive name must be a plain filename");
    }
    if !(name.ends_with(".conda") || name.ends_with(".tar.bz2")) {
        return Err("archive name must end with .conda or .tar.bz2");
    }
    Ok(())
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

fn validate_command_name(command: &str) {
    validate_artifact_component("command name", command);
}

fn validate_target_label(label: &str) {
    validate_artifact_component("target label", label);
}

fn validate_target_triple(target: &str) {
    validate_artifact_component("target triple", target);
}

fn apply_install_location_overrides(
    config: &mut RuntimeStampConfig,
    scheme: Option<runtime_data::InstallScheme>,
    install_name: Option<String>,
) {
    if let Some(scheme) = scheme {
        config.scheme = Some(scheme);
    }

    if let Some(install_name) = install_name {
        validate_install_name(&install_name);
        config.install_name = Some(install_name);
    }
}

fn validate_install_location_config(config: &RuntimeStampConfig, command: &str) {
    validate_install_name(config.install_name.as_deref().unwrap_or(command));
}

fn validate_install_name(install_name: &str) {
    validate_artifact_component("install name", install_name);
}

fn validate_artifact_component(kind: &str, value: &str) {
    assert!(!value.is_empty(), "{kind} must not be empty");
    assert!(value != "." && value != "..", "{kind} must not be . or ..");
    assert!(
        value
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric()),
        "{kind} must start with an ASCII letter or digit"
    );
    assert!(
        value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')),
        "{kind} may only contain ASCII letters, digits, dots, dashes, and underscores"
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

fn resolve_runtime_template(path: &Path) -> PathBuf {
    let template = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .expect("failed to read current directory")
            .join(path)
    };
    assert!(
        template.is_file(),
        "runtime template not found at {}",
        template.display()
    );
    template
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
        install_scheme: derived.runtime_config.scheme.unwrap_or_default(),
        install_name: derived
            .runtime_config
            .install_name
            .clone()
            .unwrap_or_else(|| name.to_string()),
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
            platform: platform.name().to_string(),
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
        .filter(|(p, _)| p.subdir() == platform)
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
            let record = package_record(pkg);
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
    packages: Vec<String>,
    channels: Vec<String>,
    exclude: Vec<String>,
    root_override: Option<PathBuf>,
) {
    let root = project_root(root_override.as_deref());
    let manifest_path = discover_manifest_path(&root);
    let kind = manifest_kind(&manifest_path);
    let content = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", manifest_path.display()));

    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", manifest_path.display()));

    if !packages.is_empty() {
        let mut deps = toml_edit::Table::new();
        for spec in &packages {
            let (name, version) = pixi_dependency_from_matchspec(spec);
            deps[&name] = toml_edit::value(version);
        }
        match kind {
            ManifestKind::PixiPyproject => {
                doc["tool"]["pixi"]["feature"]["runtime"]["dependencies"] =
                    toml_edit::Item::Table(deps);
            }
            ManifestKind::CondaToml | ManifestKind::PixiToml => {
                doc["feature"]["runtime"]["dependencies"] = toml_edit::Item::Table(deps);
            }
        }
        eprintln!("configured {} custom packages", packages.len());
    }

    if !channels.is_empty() {
        let mut arr = toml_edit::Array::new();
        for c in &channels {
            arr.push(c.to_string());
        }
        match kind {
            ManifestKind::PixiPyproject => {
                doc["tool"]["pixi"]["workspace"]["channels"] = toml_edit::value(arr);
            }
            ManifestKind::CondaToml | ManifestKind::PixiToml => {
                doc["workspace"]["channels"] = toml_edit::value(arr);
            }
        }

        eprintln!("configured channels: {}", channels.join(", "));
    }

    if !exclude.is_empty() {
        let mut arr = toml_edit::Array::new();
        for e in &exclude {
            arr.push(e.to_string());
        }
        doc["tool"]["pronto"]["exclude"] = toml_edit::value(arr);
        eprintln!("configured excludes: {}", exclude.join(", "));
    }

    std::fs::write(&manifest_path, doc.to_string())
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", manifest_path.display()));
}

fn pixi_dependency_from_matchspec(spec: &str) -> (String, String) {
    let parsed = MatchSpec::from_str(spec, ParseMatchSpecOptions::default())
        .unwrap_or_else(|e| panic!("failed to parse package matchspec {spec:?}: {e}"));
    assert!(
        parsed.build.is_none()
            && parsed.build_number.is_none()
            && parsed.file_name.is_none()
            && parsed.extras.is_none()
            && parsed.flags.is_none()
            && parsed.channel.is_none()
            && parsed.subdir.is_none()
            && parsed.namespace.is_none()
            && parsed.md5.is_none()
            && parsed.sha256.is_none()
            && parsed.url.is_none()
            && parsed.license.is_none(),
        "package matchspec {spec:?} cannot be represented in Pixi dependency syntax; use a project manifest and lockfile instead"
    );
    let version = parsed
        .version
        .map(|version| version.to_string())
        .unwrap_or_else(|| "*".to_string());
    (parsed.name.to_string(), version)
}

fn main() {
    tls::install_default_provider();

    let cli = Cli::parse();
    match cli.command {
        Command::Lock { check, root } => write_runtime_lock(check, root),
        Command::Bundle { platform, root } => gen_bundle(platform, root),
        Command::Build {
            layout,
            command,
            target_label,
            platform,
            target,
            template,
            docs_url,
            scheme,
            install_name,
            out_dir,
            root,
        } => {
            let output = build_artifact(
                layout,
                command,
                target_label,
                platform,
                target,
                template,
                docs_url,
                scheme,
                install_name,
                out_dir,
                root,
            );
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
            command,
            platform,
            out_dir,
            template,
            docs_url,
            scheme,
            install_name,
            root,
            args,
        } => run_artifact(
            layout,
            command,
            platform,
            out_dir,
            template,
            docs_url,
            scheme,
            install_name,
            root,
            args,
        ),
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
    use tempfile::TempDir;

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
    fn test_find_project_root_finds_pixi_pyproject() {
        let tmp = TempDir::new().unwrap();
        let nested = tmp.path().join("src").join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            tmp.path().join("pyproject.toml"),
            r#"
[tool.pixi.workspace]
name = "demo"
channels = ["conda-forge"]
"#,
        )
        .unwrap();

        assert_eq!(find_project_root(&nested), Some(tmp.path().to_path_buf()));
    }

    #[test]
    fn test_discover_manifest_prefers_conda_toml() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("conda.toml"), "").unwrap();
        std::fs::write(tmp.path().join("pixi.toml"), "").unwrap();
        std::fs::write(tmp.path().join("pyproject.toml"), "[tool.pixi.workspace]\n").unwrap();

        assert_eq!(
            discover_manifest_path(tmp.path()),
            tmp.path().join("conda.toml")
        );
    }

    #[test]
    fn test_discover_project_input_uses_pixi_lock_for_pyproject() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("pyproject.toml"),
            r#"
[tool.pixi.workspace]
name = "demo"
channels = ["conda-forge"]

[tool.pronto]
source-environment = "runtime"
"#,
        )
        .unwrap();
        std::fs::write(tmp.path().join("pixi.lock"), "").unwrap();

        let input = discover_project_input(tmp.path());

        assert_eq!(input.lock_path, tmp.path().join("pixi.lock"));
        assert_eq!(input.config.source_environment.as_deref(), Some("runtime"));
    }

    #[test]
    fn test_pyproject_requires_pixi_config() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("pyproject.toml"),
            r#"
[tool.pronto]
source-environment = "runtime"
"#,
        )
        .unwrap();

        assert!(!is_supported_pyproject_manifest(
            &tmp.path().join("pyproject.toml")
        ));
    }

    #[test]
    fn test_configure_updates_pixi_pyproject_sections() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("pyproject.toml"),
            r#"
[tool.pixi.workspace]
name = "demo"
channels = ["conda-forge"]

[tool.pronto]
source-environment = "runtime"
"#,
        )
        .unwrap();

        configure(
            vec!["python >=3.12,<3.15".to_string()],
            vec![
                "conda-forge".to_string(),
                "https://repo.example.test/conda".to_string(),
            ],
            vec!["conda-libmamba-solver".to_string()],
            Some(tmp.path().to_path_buf()),
        );

        let content = std::fs::read_to_string(tmp.path().join("pyproject.toml")).unwrap();
        let doc: toml::Value = toml::from_str(&content).unwrap();

        assert_eq!(
            doc["tool"]["pixi"]["feature"]["runtime"]["dependencies"]["python"]
                .as_str()
                .unwrap(),
            ">=3.12,<3.15"
        );
        assert_eq!(
            doc["tool"]["pixi"]["workspace"]["channels"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["conda-forge", "https://repo.example.test/conda"]
        );
        assert_eq!(
            doc["tool"]["pronto"]["exclude"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["conda-libmamba-solver"]
        );
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
    fn test_validate_required_runtime_packages_accepts_runtime_contract() {
        let packages = vec![
            make_pkg("conda", &[]),
            make_pkg("conda-spawn", &[]),
            make_pkg("conda-rattler-solver", &[]),
        ];

        validate_required_runtime_packages("linux-64", &packages);
    }

    #[test]
    #[should_panic(expected = "missing required package(s): conda-spawn")]
    fn test_validate_required_runtime_packages_rejects_missing_runtime_package() {
        let packages = vec![
            make_pkg("conda", &[]),
            make_pkg("conda-rattler-solver", &[]),
        ];

        validate_required_runtime_packages("linux-64", &packages);
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
    fn test_command_name_allows_filename_safe_components() {
        validate_command_name("conda-pronto_1.0");
    }

    #[test]
    #[should_panic(expected = "command name must not be . or ..")]
    fn test_command_name_rejects_dot_component() {
        validate_command_name(".");
    }

    #[test]
    #[should_panic(expected = "command name must start with an ASCII letter or digit")]
    fn test_command_name_rejects_leading_dash() {
        validate_command_name("-demo");
    }

    #[test]
    #[should_panic(
        expected = "command name may only contain ASCII letters, digits, dots, dashes, and underscores"
    )]
    fn test_command_name_rejects_path_separator() {
        validate_command_name("demo/tool");
    }

    #[test]
    #[should_panic(
        expected = "command name may only contain ASCII letters, digits, dots, dashes, and underscores"
    )]
    fn test_command_name_rejects_newline() {
        validate_command_name("demo\ntool");
    }

    #[test]
    #[should_panic(
        expected = "target label may only contain ASCII letters, digits, dots, dashes, and underscores"
    )]
    fn test_target_label_rejects_path_separator() {
        validate_target_label("linux/64");
    }

    #[test]
    #[should_panic(
        expected = "target triple may only contain ASCII letters, digits, dots, dashes, and underscores"
    )]
    fn test_target_triple_rejects_path_like_value() {
        validate_target_triple("custom/target.json");
    }

    #[test]
    fn test_install_name_allows_filename_safe_components() {
        validate_install_name("conda-express_1.0");
    }

    #[test]
    fn test_build_accepts_scheme_with_install_name() {
        let cli = Cli::try_parse_from([
            "pronto",
            "build",
            "--command",
            "cx",
            "--scheme",
            "data",
            "--install-name",
            "express",
        ])
        .unwrap();

        let Command::Build {
            command,
            scheme,
            install_name,
            ..
        } = cli.command
        else {
            panic!("expected build command");
        };

        assert_eq!(command, "cx");
        assert_eq!(scheme, Some(runtime_data::InstallScheme::Data));
        assert_eq!(install_name.as_deref(), Some("express"));
    }

    #[test]
    fn test_build_rejects_path_option() {
        let result = Cli::try_parse_from([
            "pronto",
            "build",
            "--command",
            "demo",
            "--path",
            "/tmp/demo",
        ]);

        assert!(result.is_err(), "build-time --path should not be accepted");
    }

    #[test]
    fn test_run_rejects_path_option_before_runtime_args() {
        let result = Cli::try_parse_from([
            "pronto",
            "run",
            "--command",
            "demo",
            "--path",
            "/tmp/demo",
            "--",
            "status",
        ]);

        assert!(
            result.is_err(),
            "run-time --path must be passed after `--` to the staged runtime"
        );
    }

    #[test]
    #[should_panic(expected = "install name must not be . or ..")]
    fn test_install_name_rejects_dot_component() {
        validate_install_name(".");
    }

    #[test]
    #[should_panic(
        expected = "install name may only contain ASCII letters, digits, dots, dashes, and underscores"
    )]
    fn test_install_name_rejects_path_separator() {
        validate_install_name("conda/express");
    }

    #[test]
    #[should_panic(
        expected = "install name may only contain ASCII letters, digits, dots, dashes, and underscores"
    )]
    fn test_install_name_rejects_newline() {
        validate_install_name("express\n");
    }

    #[test]
    fn test_binary_filename_uses_windows_extension_for_target() {
        assert_eq!(
            binary_filename("demo", Some("x86_64-pc-windows-msvc")),
            "demo.exe"
        );
    }

    #[test]
    fn test_package_archive_name_accepts_conda_archives() {
        assert!(validate_package_archive_name("python-3.12-h123_0.conda").is_ok());
        assert!(validate_package_archive_name("python-3.12-h123_0.tar.bz2").is_ok());
    }

    #[test]
    fn test_package_archive_name_rejects_path_components() {
        assert!(validate_package_archive_name("../python-3.12-h123_0.conda").is_err());
        assert!(validate_package_archive_name("nested/python-3.12-h123_0.conda").is_err());
    }

    #[test]
    fn test_package_archive_name_rejects_non_package_suffix() {
        assert!(validate_package_archive_name("python-3.12-h123_0.zip").is_err());
    }

    #[test]
    fn test_runtime_env_var_sanitizes_artifact_name() {
        assert_eq!(runtime_env_var("demo-tool", "BUNDLE"), "DEMO_TOOL_BUNDLE");
    }

    #[test]
    fn test_pixi_dependency_from_matchspec_preserves_comma_version() {
        assert_eq!(
            pixi_dependency_from_matchspec("python >=3.12,<3.15"),
            ("python".to_string(), ">=3.12,<3.15".to_string())
        );
    }

    #[test]
    fn test_pixi_dependency_from_matchspec_defaults_to_wildcard() {
        assert_eq!(
            pixi_dependency_from_matchspec("conda-spawn"),
            ("conda-spawn".to_string(), "*".to_string())
        );
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
