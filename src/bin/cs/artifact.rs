use std::env;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};

use miette::{Context, IntoDiagnostic};
use rattler_conda_types::Platform;
use rattler_lock::{CondaPackageData, LockFile};
use sha2::{Digest, Sha256};

use super::bundle::gen_bundle_from_lock;
use super::diagnostic::{DiagnosticKind, ship_error};
use super::project::{
    DerivedRuntimeLock, derive_runtime_lock, package_record, project_root,
    write_generated_runtime_lock,
};
use super::{
    BUNDLE_ARCHIVE_FILE, BundleLayout, REQUIRED_RUNTIME_PACKAGES, RUNTIME_LOCK_FILE,
    RUNTIME_TEMPLATE_ENV, RuntimeStampConfig, RuntimeVersionSource, SHIP_STATE_DIR, ShipConfig,
    runtime_data,
};

#[derive(Debug)]
pub(crate) struct BuildOutput {
    pub(crate) binary: PathBuf,
    pub(crate) bundle: Option<PathBuf>,
    pub(crate) info: PathBuf,
    pub(crate) checksums: PathBuf,
    pub(crate) lock: PathBuf,
    pub(crate) package_list: PathBuf,
}

#[derive(Debug)]
struct PlannedArtifactPaths {
    stem: String,
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
struct InspectReport {
    project: InspectProject,
    runtime_input: InspectRuntimeInput,
    validation: InspectValidation,
    exclusions: InspectExclusions,
    selected_platform: String,
    selected_package_count: usize,
    platform_summary: Vec<PlatformSummary>,
    packages: Vec<PackageInfo>,
}

#[derive(serde::Serialize)]
struct InspectProject {
    root: String,
    manifest: String,
    manifest_kind: String,
    lockfile: String,
    source_environment: String,
}

#[derive(serde::Serialize)]
struct InspectRuntimeInput {
    channels: Vec<String>,
    packages: Vec<String>,
    package_count: usize,
    platform_count: usize,
}

#[derive(serde::Serialize)]
struct InspectValidation {
    source_lockfile: String,
    runtime_lock_derivation: String,
    required_packages: Vec<String>,
}

#[derive(serde::Serialize)]
struct InspectExclusions {
    configured: Vec<String>,
    removed: Vec<String>,
    removed_count: usize,
}

#[derive(serde::Serialize)]
struct PlatformSummary {
    platform: String,
    packages: usize,
}

#[derive(Clone, serde::Serialize)]
pub(crate) struct PackageInfo {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) build: String,
    pub(crate) url: String,
    pub(crate) sha256: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn dry_run_build_artifact(
    layout: Option<BundleLayout>,
    runtime: Option<String>,
    delegate: Option<String>,
    runtime_version: Option<String>,
    target_label: Option<String>,
    platform_str: Option<String>,
    target: Option<String>,
    template: Option<PathBuf>,
    docs_url: Option<String>,
    install_scheme: Option<runtime_data::InstallScheme>,
    install_name: Option<String>,
    install_method: Option<String>,
    out_dir: PathBuf,
    root_override: Option<PathBuf>,
) -> miette::Result<()> {
    if let Some(label) = target_label.as_deref() {
        validate_target_label(label)?;
    }
    if let Some(target) = target.as_deref() {
        validate_target_triple(target)?;
    }
    let root = project_root(root_override.as_deref())?;
    let platform = parse_platform(platform_str)?;

    let mut derived = derive_runtime_lock(&root)?;
    let runtime = resolve_runtime_name(runtime, &derived.input.config)?;
    let delegate = resolve_delegate(delegate, &derived.input.config)?;
    let layout = resolve_bundle_layout(layout, &derived.input.config);
    validate_runtime_name(&runtime)?;
    validate_delegate(&delegate)?;
    derived.runtime_config.delegate = Some(delegate.clone());
    apply_runtime_metadata_overrides(
        &mut derived.runtime_config,
        runtime_version,
        docs_url,
        install_method,
    )?;
    apply_install_location_overrides(&mut derived.runtime_config, install_scheme, install_name)?;
    validate_install_location_config(&derived.runtime_config, &runtime)?;

    let runtime_lock_path = generated_runtime_lock_path(&root);
    let packages = packages_for_platform(&derived.lock_file, &runtime_lock_path, platform)?;
    if layout.needs_bundle() {
        validate_bundle_package_hashes(&packages)?;
    }

    let template_source = source_binary_plan(template.as_deref(), target.as_deref())?;
    let out_dir = resolve_out_dir(&root, &out_dir);
    let paths = planned_artifact_paths(
        &out_dir,
        &runtime,
        layout,
        target_label.as_deref(),
        target.as_deref(),
    );
    print_build_dry_run(
        &root,
        &derived,
        layout,
        &runtime,
        platform,
        target.as_deref(),
        &template_source,
        &out_dir,
        &paths,
        packages.len(),
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_artifact(
    layout: Option<BundleLayout>,
    runtime: Option<String>,
    delegate: Option<String>,
    runtime_version: Option<String>,
    target_label: Option<String>,
    platform_str: Option<String>,
    target: Option<String>,
    template: Option<PathBuf>,
    docs_url: Option<String>,
    install_scheme: Option<runtime_data::InstallScheme>,
    install_name: Option<String>,
    install_method: Option<String>,
    out_dir: PathBuf,
    root_override: Option<PathBuf>,
) -> miette::Result<BuildOutput> {
    if let Some(label) = target_label.as_deref() {
        validate_target_label(label)?;
    }
    if let Some(target) = target.as_deref() {
        validate_target_triple(target)?;
    }
    let root = project_root(root_override.as_deref())?;
    let platform = parse_platform(platform_str)?;

    let mut derived = derive_runtime_lock(&root)?;
    let runtime = resolve_runtime_name(runtime, &derived.input.config)?;
    let delegate = resolve_delegate(delegate, &derived.input.config)?;
    let layout = resolve_bundle_layout(layout, &derived.input.config);
    validate_runtime_name(&runtime)?;
    validate_delegate(&delegate)?;
    derived.runtime_config.delegate = Some(delegate);
    apply_runtime_metadata_overrides(
        &mut derived.runtime_config,
        runtime_version,
        docs_url,
        install_method,
    )?;
    apply_install_location_overrides(&mut derived.runtime_config, install_scheme, install_name)?;
    validate_install_location_config(&derived.runtime_config, &runtime)?;
    let runtime_lock_path = generated_runtime_lock_path(&root);
    write_generated_runtime_lock(&runtime_lock_path, &derived.content)?;

    let generated_bundle = if layout.needs_bundle() {
        Some(gen_bundle_from_lock(
            &derived.lock_file,
            &runtime_lock_path,
            platform,
            &generated_bundle_path(&root),
        )?)
    } else {
        None
    };

    let source_binary = source_binary(template.as_deref(), target.as_deref())?;

    stage_artifacts(
        &root,
        &source_binary,
        layout,
        &runtime,
        target_label.as_deref(),
        platform,
        target.as_deref(),
        &out_dir,
        &derived,
        generated_bundle.as_deref(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_artifact(
    layout: Option<BundleLayout>,
    runtime: Option<String>,
    delegate: Option<String>,
    runtime_version: Option<String>,
    platform: Option<String>,
    out_dir: PathBuf,
    template: Option<PathBuf>,
    docs_url: Option<String>,
    install_scheme: Option<runtime_data::InstallScheme>,
    install_name: Option<String>,
    install_method: Option<String>,
    root_override: Option<PathBuf>,
    args: Vec<OsString>,
) -> miette::Result<()> {
    let root = project_root(root_override.as_deref())?;
    let derived = derive_runtime_lock(&root)?;
    let runtime = resolve_runtime_name(runtime, &derived.input.config)?;
    let layout = resolve_bundle_layout(layout, &derived.input.config);
    let bundle_env_var = runtime_env_var(&runtime, "BUNDLE");
    let bundle_dir = root.join(SHIP_STATE_DIR).join("bundle");
    let output = build_artifact(
        Some(layout),
        Some(runtime),
        delegate,
        runtime_version,
        None,
        platform,
        None,
        template,
        docs_url,
        install_scheme,
        install_name,
        install_method,
        out_dir,
        Some(root.clone()),
    )?;

    let mut command = std::process::Command::new(&output.binary);
    command.args(args);
    if layout == BundleLayout::External {
        command.env(bundle_env_var, bundle_dir);
    }

    let status = command
        .status()
        .into_diagnostic()
        .with_context(|| format!("failed to run {}", output.binary.display()))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

pub(crate) fn resolve_runtime_name(
    runtime: Option<String>,
    config: &ShipConfig,
) -> miette::Result<String> {
    runtime.or_else(|| config.runtime.clone()).ok_or_else(|| {
        ship_error(
            DiagnosticKind::MissingRuntime,
            "runtime is required; pass --runtime or set [tool.conda-ship].runtime",
            Some(
                "Pass --runtime to this command or set [tool.conda-ship].runtime in the source manifest."
                    .to_string(),
            ),
        )
    })
}

pub(crate) fn resolve_delegate(
    delegate: Option<String>,
    config: &ShipConfig,
) -> miette::Result<String> {
    delegate.or_else(|| config.delegate.clone()).ok_or_else(|| {
        ship_error(
            DiagnosticKind::MissingDelegate,
            "delegate is required; pass --delegate or set [tool.conda-ship].delegate",
            Some(
                "Pass --delegate to this command or set [tool.conda-ship].delegate in the source manifest."
                    .to_string(),
            ),
        )
    })
}

pub(crate) fn resolve_bundle_layout(
    layout: Option<BundleLayout>,
    config: &ShipConfig,
) -> BundleLayout {
    layout.or(config.layout).unwrap_or(BundleLayout::Online)
}

pub(crate) fn inspect_artifact(
    platform_str: Option<String>,
    json: bool,
    root_override: Option<PathBuf>,
) -> miette::Result<()> {
    let root = project_root(root_override.as_deref())?;
    let derived = derive_runtime_lock(&root)?;
    let platform = parse_platform(platform_str)?;
    let runtime_lock_path = generated_runtime_lock_path(&root);

    let summaries = platform_summaries(&derived.lock_file, &runtime_lock_path)?;
    let packages = packages_for_platform(&derived.lock_file, &runtime_lock_path, platform)?;
    let package_infos = package_infos(&packages)?;

    if json {
        let inspect = inspect_report(&root, &derived, platform, summaries, package_infos);
        println!(
            "{}",
            serde_json::to_string_pretty(&inspect)
                .into_diagnostic()
                .context("failed to render inspect JSON")?
        );
        return Ok(());
    }

    print_inspect_report(&root, &derived, platform, &summaries, &package_infos);
    Ok(())
}

fn inspect_report(
    root: &Path,
    derived: &DerivedRuntimeLock,
    platform: Platform,
    summaries: Vec<PlatformSummary>,
    package_infos: Vec<PackageInfo>,
) -> InspectReport {
    InspectReport {
        project: InspectProject {
            root: root.display().to_string(),
            manifest: display_path(root, &derived.input.manifest_path),
            manifest_kind: derived.input.manifest_kind.manifest_label().to_string(),
            lockfile: display_path(root, &derived.input.lock_path),
            source_environment: derived.source_environment.clone(),
        },
        runtime_input: InspectRuntimeInput {
            channels: derived.runtime_config.channels.clone(),
            packages: derived.runtime_config.packages.clone(),
            package_count: derived.total_packages,
            platform_count: derived.platforms.len(),
        },
        validation: InspectValidation {
            source_lockfile: "ok".to_string(),
            runtime_lock_derivation: "ok".to_string(),
            required_packages: REQUIRED_RUNTIME_PACKAGES
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
        },
        exclusions: InspectExclusions {
            configured: derived.runtime_config.exclude.clone(),
            removed: derived.removed_excludes.clone(),
            removed_count: derived.total_excluded,
        },
        selected_platform: platform.to_string(),
        selected_package_count: package_infos.len(),
        platform_summary: summaries,
        packages: package_infos,
    }
}

fn print_inspect_report(
    root: &Path,
    derived: &DerivedRuntimeLock,
    platform: Platform,
    summaries: &[PlatformSummary],
    package_infos: &[PackageInfo],
) {
    println!("Project");
    print_project_summary(root, derived);
    println!();
    println!("Runtime input");
    println!("  source environment: {}", derived.source_environment);
    println!("  platforms: {}", platform_list(&derived.platforms));
    println!(
        "  channels: {}",
        string_list(&derived.runtime_config.channels)
    );
    println!("  packages: {}", derived.total_packages);
    println!();
    println!("Validation");
    println!("  source lockfile: ok");
    println!("  runtime lock derivation: ok");
    println!(
        "  required packages: {}",
        REQUIRED_RUNTIME_PACKAGES.join(", ")
    );
    println!();
    println!("Exclusions");
    println!(
        "  configured: {}",
        string_list(&derived.runtime_config.exclude)
    );
    println!("  removed: {}", string_list(&derived.removed_excludes));
    println!("  removed package entries: {}", derived.total_excluded);
    println!();
    println!("Platforms");
    for summary in summaries {
        println!("  {}: {} packages", summary.platform, summary.packages);
    }
    println!();
    println!("Selected platform: {platform}");
    println!("Packages: {}", package_infos.len());
    for package in package_infos {
        println!("  {} {} {}", package.name, package.version, package.build);
    }
}

#[allow(clippy::too_many_arguments)]
fn print_build_dry_run(
    root: &Path,
    derived: &DerivedRuntimeLock,
    layout: BundleLayout,
    runtime: &str,
    platform: Platform,
    target: Option<&str>,
    template_source: &RuntimeTemplateSource,
    out_dir: &Path,
    paths: &PlannedArtifactPaths,
    selected_package_count: usize,
) -> miette::Result<()> {
    let install_scheme = derived.runtime_config.install_scheme.unwrap_or_default();
    let install_name = derived
        .runtime_config
        .install_name
        .as_deref()
        .unwrap_or(runtime);
    let delegate =
        derived.runtime_config.delegate.as_deref().ok_or_else(|| {
            miette::miette!("delegate has not been resolved before dry-run output")
        })?;
    let docs_url = derived
        .runtime_config
        .docs_url
        .clone()
        .unwrap_or_else(default_docs_url);
    let runtime_version = derived
        .runtime_config
        .runtime_version
        .as_deref()
        .ok_or_else(|| missing_runtime_version(derived.runtime_config.project_dynamic_version))?;

    println!("Build dry run");
    println!("Project");
    print_project_summary(root, derived);
    println!();
    println!("Runtime");
    println!("  runtime: {runtime}");
    println!("  version: {runtime_version}");
    println!("  delegate: {delegate}");
    println!("  layout: {}", layout.as_str());
    println!("  platform: {platform}");
    println!("  target: {}", target.unwrap_or("current"));
    println!("  install scheme: {}", install_scheme_name(install_scheme));
    println!("  install name: {install_name}");
    if let Some(method) = derived.runtime_config.install_method.as_deref() {
        println!("  install method: {method}");
    }
    println!("  docs URL: {docs_url}");
    println!("  packages: {selected_package_count}");
    println!();
    println!("Template");
    println!("  source: {}", template_source.label());
    println!("  path: {}", display_path(root, template_source.path()));
    println!();
    println!("Artifacts");
    println!("  output directory: {}", display_path(root, out_dir));
    println!("  binary: {}", display_path(root, &paths.binary));
    if let Some(bundle) = &paths.bundle {
        println!("  bundle: {}", display_path(root, bundle));
    }
    println!("  info: {}", display_path(root, &paths.info));
    println!("  runtime lock: {}", display_path(root, &paths.lock));
    println!("  packages: {}", display_path(root, &paths.package_list));
    println!("  checksums: {}", display_path(root, &paths.checksums));
    println!();
    println!("No files written.");
    Ok(())
}

fn print_project_summary(root: &Path, derived: &DerivedRuntimeLock) {
    println!("  root: {}", root.display());
    println!(
        "  manifest: {}",
        display_path(root, &derived.input.manifest_path)
    );
    println!(
        "  manifest kind: {}",
        derived.input.manifest_kind.manifest_label()
    );
    println!(
        "  source lockfile: {}",
        display_path(root, &derived.input.lock_path)
    );
}

pub(crate) fn validate_bundle_package_hashes(packages: &[&CondaPackageData]) -> miette::Result<()> {
    let mut missing = Vec::new();
    for pkg in packages {
        let record = package_record(pkg)?;
        if record.sha256.is_none() {
            missing.push(record.name.as_normalized().to_string());
        }
    }
    missing.sort();
    missing.dedup();
    if !missing.is_empty() {
        return Err(ship_error(
            DiagnosticKind::MissingSha256,
            format!(
                "cannot bundle packages without SHA256 hashes in the source lockfile: {}",
                missing.join(", ")
            ),
            Some(
                "Refresh the source lockfile with package hash metadata before building an external or embedded layout."
                    .to_string(),
            ),
        ));
    }
    Ok(())
}

fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn string_list(values: &[String]) -> String {
    if values.is_empty() {
        "(none)".to_string()
    } else {
        values.join(", ")
    }
}

fn platform_list(values: &[Platform]) -> String {
    if values.is_empty() {
        "(none)".to_string()
    } else {
        let mut platforms: Vec<_> = values.iter().map(ToString::to_string).collect();
        platforms.sort();
        platforms.join(", ")
    }
}

fn install_scheme_name(scheme: runtime_data::InstallScheme) -> &'static str {
    match scheme {
        runtime_data::InstallScheme::CondaHome => "conda-home",
        runtime_data::InstallScheme::UserData => "user-data",
    }
}

fn default_docs_url() -> String {
    "https://jezdez.github.io/conda-ship/".to_string()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn stage_artifacts(
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
) -> miette::Result<BuildOutput> {
    let out_dir = resolve_out_dir(root, out_dir);
    std::fs::create_dir_all(&out_dir)
        .into_diagnostic()
        .with_context(|| format!("failed to create {}", out_dir.display()))?;

    let paths = planned_artifact_paths(&out_dir, name, layout, target_label, target);
    std::fs::copy(source_binary, &paths.binary)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to copy {} to {}",
                source_binary.display(),
                paths.binary.display()
            )
        })?;
    stamp_runtime_data(&paths.binary, layout, name, derived, generated_bundle)?;

    let bundle = if layout == BundleLayout::External {
        let source_bundle = generated_bundle
            .ok_or_else(|| miette::miette!("external builds require a generated bundle"))?;
        let staged_bundle = paths
            .bundle
            .as_ref()
            .ok_or_else(|| miette::miette!("external builds have a planned bundle path"))?;
        std::fs::copy(source_bundle, staged_bundle)
            .into_diagnostic()
            .with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    source_bundle.display(),
                    staged_bundle.display()
                )
            })?;
        Some(staged_bundle.clone())
    } else {
        None
    };

    let metadata = write_artifact_metadata(
        root,
        &out_dir,
        &paths.stem,
        layout,
        platform,
        &paths.binary,
        bundle.as_deref(),
        derived,
    )?;

    eprintln!("staged {}", paths.binary.display());
    if let Some(bundle) = &bundle {
        eprintln!("staged {}", bundle.display());
    }
    eprintln!("wrote {}", metadata.info.display());
    eprintln!("wrote {}", metadata.checksums.display());

    Ok(BuildOutput {
        binary: paths.binary,
        bundle,
        info: metadata.info,
        checksums: metadata.checksums,
        lock: metadata.lock,
        package_list: metadata.package_list,
    })
}

fn planned_artifact_paths(
    out_dir: &Path,
    name: &str,
    layout: BundleLayout,
    target_label: Option<&str>,
    target: Option<&str>,
) -> PlannedArtifactPaths {
    let stem = artifact_stem(name, layout, target_label);
    let binary = out_dir.join(binary_filename(&stem, target));
    let bundle =
        (layout == BundleLayout::External).then(|| out_dir.join(format!("{stem}.bundle.tar.zst")));
    PlannedArtifactPaths {
        info: out_dir.join(format!("{stem}.info.json")),
        checksums: out_dir.join(format!("{stem}.sha256")),
        lock: out_dir.join(format!("{stem}.runtime.lock")),
        package_list: out_dir.join(format!("{stem}.packages.txt")),
        stem,
        binary,
        bundle,
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
) -> miette::Result<ArtifactMetadataPaths> {
    let runtime_lock_path = generated_runtime_lock_path(root);
    let packages = packages_for_platform(&derived.lock_file, &runtime_lock_path, platform)?;
    let package_infos = package_infos(&packages)?;

    let lock = out_dir.join(format!("{stem}.runtime.lock"));
    std::fs::write(&lock, &derived.content)
        .into_diagnostic()
        .with_context(|| format!("failed to write {}", lock.display()))?;

    let package_list = out_dir.join(format!("{stem}.packages.txt"));
    let package_list_content = render_package_list(&package_infos);
    std::fs::write(&package_list, package_list_content)
        .into_diagnostic()
        .with_context(|| format!("failed to write {}", package_list.display()))?;

    let mut checksums = vec![
        checksum_for_path(binary)?,
        checksum_for_path(&lock)?,
        checksum_for_path(&package_list)?,
    ];
    if let Some(bundle) = bundle {
        checksums.push(checksum_for_path(bundle)?);
    }

    let info = out_dir.join(format!("{stem}.info.json"));
    let info_doc = ArtifactInfo {
        schema_version: 1,
        name: stem.to_string(),
        layout: layout.as_str().to_string(),
        platform: platform.to_string(),
        binary: file_name(binary)?,
        bundle: bundle.map(file_name).transpose()?,
        lock: file_name(&lock)?,
        package_list: file_name(&package_list)?,
        package_count: package_infos.len(),
        checksums,
    };
    let info_json = serde_json::to_string_pretty(&info_doc)
        .into_diagnostic()
        .context("failed to render info JSON")?;
    std::fs::write(&info, format!("{info_json}\n"))
        .into_diagnostic()
        .with_context(|| format!("failed to write {}", info.display()))?;

    let mut final_checksums = info_doc.checksums;
    final_checksums.push(checksum_for_path(&info)?);
    final_checksums.sort_by(|a, b| a.path.cmp(&b.path));

    let checksums = out_dir.join(format!("{stem}.sha256"));
    write_checksums(&checksums, &final_checksums)?;

    Ok(ArtifactMetadataPaths {
        info,
        checksums,
        lock,
        package_list,
    })
}

pub(crate) fn generated_runtime_lock_path(root: &Path) -> PathBuf {
    root.join(SHIP_STATE_DIR).join(RUNTIME_LOCK_FILE)
}

pub(crate) fn generated_bundle_path(root: &Path) -> PathBuf {
    root.join(SHIP_STATE_DIR).join(BUNDLE_ARCHIVE_FILE)
}

pub(crate) fn validate_package_archive_name(name: &str) -> Result<(), &'static str> {
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

pub(crate) fn parse_platform(platform_str: Option<String>) -> miette::Result<Platform> {
    if let Some(ref platform) = platform_str {
        platform.parse::<Platform>().map_err(|_| {
            ship_error(
                DiagnosticKind::InvalidPlatform,
                format!("invalid platform: {platform}"),
                Some("Use a conda platform such as linux-64, osx-arm64, or win-64.".to_string()),
            )
        })
    } else {
        Ok(Platform::current())
    }
}

pub(crate) fn validate_runtime_name(runtime: &str) -> miette::Result<()> {
    validate_artifact_component("runtime name", runtime)
}

pub(crate) fn validate_delegate(delegate: &str) -> miette::Result<()> {
    validate_artifact_component("delegate", delegate)
}

pub(crate) fn validate_target_label(label: &str) -> miette::Result<()> {
    validate_artifact_component("target label", label)
}

pub(crate) fn validate_target_triple(target: &str) -> miette::Result<()> {
    validate_artifact_component("target triple", target)
}

fn apply_install_location_overrides(
    config: &mut RuntimeStampConfig,
    install_scheme: Option<runtime_data::InstallScheme>,
    install_name: Option<String>,
) -> miette::Result<()> {
    if let Some(install_scheme) = install_scheme {
        config.install_scheme = Some(install_scheme);
    }

    if let Some(install_name) = install_name {
        validate_install_name(&install_name)?;
        config.install_name = Some(install_name);
    }
    Ok(())
}

pub(crate) fn apply_runtime_metadata_overrides(
    config: &mut RuntimeStampConfig,
    runtime_version: Option<String>,
    docs_url: Option<String>,
    install_method: Option<String>,
) -> miette::Result<()> {
    if let Some(runtime_version) = runtime_version {
        validate_runtime_version(&runtime_version)?;
        config.runtime_version = Some(runtime_version);
        config.runtime_version_source = None;
    } else if config.runtime_version.is_none() {
        match config.runtime_version_source.take() {
            Some(RuntimeVersionSource::ProjectMetadata) => {
                return Err(project_metadata_runtime_version_requires_adapter());
            }
            None => return Err(missing_runtime_version(config.project_dynamic_version)),
        }
    }
    if let Some(runtime_version) = config.runtime_version.as_deref() {
        validate_runtime_version(runtime_version)?;
    }
    if let Some(docs_url) = docs_url {
        validate_docs_url(&docs_url)?;
        config.docs_url = Some(docs_url);
    }
    if let Some(docs_url) = config.docs_url.as_deref() {
        validate_docs_url(docs_url)?;
    }
    if let Some(install_method) = install_method {
        validate_install_method(&install_method)?;
        config.install_method = Some(install_method);
    }
    validate_install_method_config(config)
}

fn project_metadata_runtime_version_requires_adapter() -> miette::Report {
    ship_error(
        DiagnosticKind::MissingRuntimeVersion,
        "runtime version from project metadata must be resolved before invoking cs",
        Some(
            "Use `conda ship build` from the Python package so the adapter can call the PEP 517 metadata hook, or pass --runtime-version explicitly."
                .to_string(),
        ),
    )
}

fn missing_runtime_version(project_dynamic_version: bool) -> miette::Report {
    if project_dynamic_version {
        ship_error(
            DiagnosticKind::MissingRuntimeVersion,
            "runtime version is required; [project] declares a dynamic version but project metadata resolution is not enabled",
            Some(
                "Pass --runtime-version, set [tool.conda-ship].runtime-version, or set runtime-version = { from = \"project-metadata\" }."
                    .to_string(),
            ),
        )
    } else {
        ship_error(
            DiagnosticKind::MissingRuntimeVersion,
            "runtime version is required",
            Some(
                "Pass --runtime-version, set [tool.conda-ship].runtime-version, or define [project].version in pyproject.toml."
                    .to_string(),
            ),
        )
    }
}

fn validate_install_location_config(
    config: &RuntimeStampConfig,
    runtime: &str,
) -> miette::Result<()> {
    validate_install_name(config.install_name.as_deref().unwrap_or(runtime))
}

fn validate_install_method_config(config: &RuntimeStampConfig) -> miette::Result<()> {
    if let Some(method) = config.install_method.as_deref() {
        validate_artifact_component("install method", method)?;
    }
    Ok(())
}

pub(crate) fn validate_runtime_version(runtime_version: &str) -> miette::Result<()> {
    if runtime_version.is_empty() {
        return Err(ship_error(
            DiagnosticKind::InvalidArtifactName,
            "runtime version must not be empty",
            Some("Use a non-empty runtime version such as 1.0.0.".to_string()),
        ));
    }
    if !runtime_version
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        return Err(ship_error(
            DiagnosticKind::InvalidArtifactName,
            "runtime version must start with an ASCII letter or digit",
            Some("Use a package-style version such as 1.0.0 or 1.0.0+local.".to_string()),
        ));
    }
    if !runtime_version
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+' | '!'))
    {
        return Err(ship_error(
            DiagnosticKind::InvalidArtifactName,
            "runtime version may only contain ASCII letters, digits, dots, dashes, underscores, plus signs, and exclamation marks",
            Some("Remove whitespace, path separators, and shell metacharacters from the runtime version.".to_string()),
        ));
    }
    Ok(())
}

pub(crate) fn validate_docs_url(docs_url: &str) -> miette::Result<()> {
    if docs_url.is_empty() {
        return Err(ship_error(
            DiagnosticKind::InvalidArtifactName,
            "docs URL must not be empty",
            Some("Use a full documentation URL such as https://example.com/demo/.".to_string()),
        ));
    }
    if !(docs_url.starts_with("https://") || docs_url.starts_with("http://")) {
        return Err(ship_error(
            DiagnosticKind::InvalidArtifactName,
            "docs URL must start with https:// or http://",
            Some("Use a full documentation URL such as https://example.com/demo/.".to_string()),
        ));
    }
    if docs_url
        .chars()
        .any(|c| c.is_ascii_control() || c.is_whitespace())
    {
        return Err(ship_error(
            DiagnosticKind::InvalidArtifactName,
            "docs URL must not contain whitespace or control characters",
            Some(
                "Remove spaces, newlines, and terminal control characters from the docs URL."
                    .to_string(),
            ),
        ));
    }
    Ok(())
}

pub(crate) fn validate_install_method(install_method: &str) -> miette::Result<()> {
    validate_artifact_component("install method", install_method)
}

pub(crate) fn validate_install_name(install_name: &str) -> miette::Result<()> {
    validate_artifact_component("install name", install_name)
}

fn validate_artifact_component(kind: &str, value: &str) -> miette::Result<()> {
    if value.is_empty() {
        return Err(ship_error(
            DiagnosticKind::InvalidArtifactName,
            format!("{kind} must not be empty"),
            Some(format!("Use a non-empty {kind} value.")),
        ));
    }
    if value == "." || value == ".." {
        return Err(ship_error(
            DiagnosticKind::InvalidArtifactName,
            format!("{kind} must not be . or .."),
            Some(format!("Use a filename-safe {kind} such as demo-runtime.")),
        ));
    }
    if !value
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        return Err(ship_error(
            DiagnosticKind::InvalidArtifactName,
            format!("{kind} must start with an ASCII letter or digit"),
            Some(format!("Use a filename-safe {kind} such as demo-runtime.")),
        ));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(ship_error(
            DiagnosticKind::InvalidArtifactName,
            format!("{kind} may only contain ASCII letters, digits, dots, dashes, and underscores"),
            Some(format!(
                "Remove path separators, spaces, and shell metacharacters from the {kind}."
            )),
        ));
    }
    Ok(())
}

pub(crate) fn artifact_stem(
    name: &str,
    layout: BundleLayout,
    target_label: Option<&str>,
) -> String {
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

pub(crate) fn binary_filename(stem: &str, target: Option<&str>) -> String {
    if target_is_windows(target) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

#[derive(Debug)]
pub(crate) enum RuntimeTemplateSource {
    Explicit(PathBuf),
    Environment(PathBuf),
    Installed(PathBuf),
}

impl RuntimeTemplateSource {
    fn path(&self) -> &Path {
        match self {
            Self::Explicit(path) | Self::Environment(path) | Self::Installed(path) => path,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Explicit(_) => "explicit --template",
            Self::Environment(_) => "CONDA_SHIP_TEMPLATE",
            Self::Installed(_) => "installed template",
        }
    }
}

pub(crate) fn source_binary_plan(
    template: Option<&Path>,
    target: Option<&str>,
) -> miette::Result<RuntimeTemplateSource> {
    if let Some(path) = template {
        return Ok(RuntimeTemplateSource::Explicit(resolve_runtime_template(
            path,
        )?));
    }

    if let Some(path) = runtime_template_from_env()? {
        return Ok(RuntimeTemplateSource::Environment(path));
    }

    if target.is_none()
        && let Some(path) = find_installed_runtime_template()
    {
        return Ok(RuntimeTemplateSource::Installed(path));
    }

    if target.is_some() {
        return Err(ship_error(
            DiagnosticKind::CrossBuildRequiresTemplate,
            "cross-builds require --template with a prebuilt runtime template for the requested target",
            Some(
                "Pass --template PATH for the target runtime template you want to stamp."
                    .to_string(),
            ),
        ));
    }

    Err(ship_error(
        DiagnosticKind::MissingRuntimeTemplate,
        "runtime template not found; install conda-ship with its runtime template or pass --template",
        Some(
            "Install a packaged conda-ship build that includes cs-template, set CONDA_SHIP_TEMPLATE, or pass --template PATH."
                .to_string(),
        ),
    ))
}

pub(crate) fn source_binary(
    template: Option<&Path>,
    target: Option<&str>,
) -> miette::Result<PathBuf> {
    let plan = source_binary_plan(template, target)?;
    Ok(plan.path().to_path_buf())
}

pub(crate) fn runtime_template_filename() -> &'static str {
    if cfg!(windows) {
        "cs-template.exe"
    } else {
        "cs-template"
    }
}

pub(crate) fn runtime_template_from_env() -> miette::Result<Option<PathBuf>> {
    match env::var_os(RUNTIME_TEMPLATE_ENV) {
        Some(value) if !value.is_empty() => Ok(Some(resolve_runtime_template(Path::new(&value))?)),
        _ => Ok(None),
    }
}

fn find_installed_runtime_template() -> Option<PathBuf> {
    if let Ok(exe) = env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let candidate = dir.join(runtime_template_filename());
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn resolve_runtime_template(path: &Path) -> miette::Result<PathBuf> {
    let template = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .into_diagnostic()
            .context("failed to read current directory")?
            .join(path)
    };
    if !template.is_file() {
        return Err(ship_error(
            DiagnosticKind::MissingRuntimeTemplate,
            format!("runtime template not found at {}", template.display()),
            Some(
                "Check the --template path or set CONDA_SHIP_TEMPLATE to a generic cs-template binary."
                    .to_string(),
            ),
        ));
    }
    if runtime_data::read_from_path(&template)
        .into_diagnostic()
        .with_context(|| format!("failed to inspect runtime template {}", template.display()))?
        .is_some_and(|data| data.stamped)
    {
        return Err(ship_error(
            DiagnosticKind::RuntimeTemplateStamped,
            format!("runtime template is already stamped: {}", template.display()),
            Some(
                "Use the generic cs-template binary as the template, not a runtime already produced by cs build."
                    .to_string(),
            ),
        ));
    }
    Ok(template)
}

fn stamp_runtime_data(
    binary: &Path,
    layout: BundleLayout,
    name: &str,
    derived: &DerivedRuntimeLock,
    generated_bundle: Option<&Path>,
) -> miette::Result<()> {
    let runtime_name = if layout == BundleLayout::Embedded {
        format!("{name}z")
    } else {
        name.to_string()
    };
    let delegate = derived
        .runtime_config
        .delegate
        .clone()
        .ok_or_else(|| miette::miette!("delegate has not been resolved before stamping"))?;
    let header = runtime_data::RuntimeDataHeader {
        schema_version: 1,
        runtime_name,
        runtime_version: derived
            .runtime_config
            .runtime_version
            .clone()
            .ok_or_else(|| {
                missing_runtime_version(derived.runtime_config.project_dynamic_version)
            })?,
        embedded_runtime_name: format!("{name}z"),
        delegate,
        display_name: name.to_string(),
        install_scheme: derived.runtime_config.install_scheme.unwrap_or_default(),
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
            .unwrap_or_else(default_docs_url),
        install_method: derived.runtime_config.install_method.clone(),
        runtime_config: runtime_data::RuntimeConfig {
            channels: derived.runtime_config.channels.clone(),
            packages: derived.runtime_config.packages.clone(),
        },
        runtime_lock: derived.content.clone(),
    };

    let embedded_bundle = if layout == BundleLayout::Embedded {
        Some(
            generated_bundle
                .ok_or_else(|| miette::miette!("embedded builds require a generated bundle"))?,
        )
    } else {
        None
    };
    runtime_data::append_to_binary(binary, &header, embedded_bundle)
        .into_diagnostic()
        .with_context(|| format!("failed to stamp runtime data onto {}", binary.display()))?;
    Ok(())
}

pub(crate) fn runtime_env_var(name: &str, suffix: &str) -> String {
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

fn platform_summaries(
    lock_file: &LockFile,
    runtime_lock_path: &Path,
) -> miette::Result<Vec<PlatformSummary>> {
    let env = lock_file.default_environment().ok_or_else(|| {
        miette::miette!("no default environment in {}", runtime_lock_path.display())
    })?;

    let mut summaries: Vec<_> = env
        .conda_packages_by_platform()
        .map(|(platform, packages)| PlatformSummary {
            platform: platform.name().to_string(),
            packages: packages.count(),
        })
        .collect();
    summaries.sort_by(|a, b| a.platform.cmp(&b.platform));
    Ok(summaries)
}

fn packages_for_platform<'a>(
    lock_file: &'a LockFile,
    runtime_lock_path: &Path,
    platform: Platform,
) -> miette::Result<Vec<&'a CondaPackageData>> {
    let env = lock_file.default_environment().ok_or_else(|| {
        miette::miette!("no default environment in {}", runtime_lock_path.display())
    })?;

    let packages: Vec<_> = env
        .conda_packages_by_platform()
        .filter(|(p, _)| p.subdir() == platform)
        .flat_map(|(_, pkgs)| pkgs)
        .collect();

    if packages.is_empty() {
        return Err(miette::miette!(
            "no packages for platform {platform} in {}",
            runtime_lock_path.display()
        ));
    }

    Ok(packages)
}

fn package_infos(packages: &[&CondaPackageData]) -> miette::Result<Vec<PackageInfo>> {
    let mut infos = Vec::with_capacity(packages.len());
    for pkg in packages {
        let record = package_record(pkg)?;
        infos.push(PackageInfo {
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
        });
    }
    infos.sort_by(|a, b| a.name.cmp(&b.name).then(a.version.cmp(&b.version)));
    Ok(infos)
}

pub(crate) fn render_package_list(packages: &[PackageInfo]) -> String {
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

fn checksum_for_path(path: &Path) -> miette::Result<ArtifactChecksum> {
    let (digest, bytes) = sha256_file_for_artifact(path)?;
    Ok(ArtifactChecksum {
        path: file_name(path)?,
        sha256: hex_bytes(&digest),
        bytes,
    })
}

fn sha256_file_for_artifact(path: &Path) -> miette::Result<([u8; 32], u64)> {
    let mut file = std::fs::File::open(path)
        .into_diagnostic()
        .with_context(|| format!("failed to read {} for checksum", path.display()))?;
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .into_diagnostic()
            .with_context(|| format!("failed to read {} for checksum", path.display()))?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut out = [0_u8; 32];
    out.copy_from_slice(&digest);
    Ok((out, bytes))
}

fn write_checksums(path: &Path, checksums: &[ArtifactChecksum]) -> miette::Result<()> {
    let mut content = String::new();
    for checksum in checksums {
        content.push_str(&format!("{}  {}\n", checksum.sha256, checksum.path));
    }
    std::fs::write(path, content)
        .into_diagnostic()
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn file_name(path: &Path) -> miette::Result<String> {
    Ok(path
        .file_name()
        .ok_or_else(|| miette::miette!("path has no file name: {}", path.display()))?
        .to_string_lossy()
        .to_string())
}
