use std::path::Path;
use std::str::FromStr;

use clap::Parser;
use rattler_conda_types::{PackageName, PackageRecord, Platform, VersionWithSource};
use rattler_lock::{CondaPackageData, LockFileBuilder, PlatformData};
use rstest::rstest;
use tempfile::TempDir;

use super::artifact::{
    PackageInfo, artifact_stem, binary_filename, render_package_list, resolve_bundle_layout,
    resolve_delegate, resolve_runtime_name, runtime_env_var, runtime_template_filename,
    runtime_template_from_env, source_binary, source_binary_plan, stage_artifacts,
    validate_delegate, validate_install_method, validate_install_name,
    validate_package_archive_name, validate_runtime_name, validate_target_label,
    validate_target_triple,
};
use super::project::{
    DerivedRuntimeLock, ManifestKind, ProjectInput, discover_manifest_path, discover_project_input,
    filter_excluded, find_project_root, is_supported_pyproject_manifest, manifest_kind,
    validate_required_runtime_packages,
};
use super::{
    BundleLayout, Cli, Command, RUNTIME_TEMPLATE_ENV, RuntimeStampConfig, ShipConfig, runtime_data,
};

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
fn test_find_project_root_finds_conda_pyproject() {
    let tmp = TempDir::new().unwrap();
    let nested = tmp.path().join("src").join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(
        tmp.path().join("pyproject.toml"),
        r#"
[tool.conda.workspace]
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
        discover_manifest_path(tmp.path()).unwrap(),
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

[tool.conda-ship]
runtime = "demo"
delegate = "conda"
layout = "external"
source-environment = "ship"
"#,
    )
    .unwrap();
    std::fs::write(tmp.path().join("pixi.lock"), "").unwrap();

    let input = discover_project_input(tmp.path()).unwrap();

    assert_eq!(input.lock_path, tmp.path().join("pixi.lock"));
    assert_eq!(input.config.runtime.as_deref(), Some("demo"));
    assert_eq!(input.config.delegate.as_deref(), Some("conda"));
    assert_eq!(input.config.layout, Some(BundleLayout::External));
    assert_eq!(input.config.source_environment.as_deref(), Some("ship"));
}

#[test]
fn test_discover_project_input_uses_conda_lock_for_conda_pyproject() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("pyproject.toml"),
        r#"
[tool.conda.workspace]
name = "demo"
channels = ["conda-forge"]

[tool.conda-ship]
runtime = "demo"
delegate = "conda"
layout = "embedded"
source-environment = "ship"
"#,
    )
    .unwrap();
    std::fs::write(tmp.path().join("conda.lock"), "").unwrap();

    let input = discover_project_input(tmp.path()).unwrap();

    assert_eq!(input.lock_path, tmp.path().join("conda.lock"));
    assert_eq!(input.config.runtime.as_deref(), Some("demo"));
    assert_eq!(input.config.delegate.as_deref(), Some("conda"));
    assert_eq!(input.config.layout, Some(BundleLayout::Embedded));
    assert_eq!(input.config.source_environment.as_deref(), Some("ship"));
}

#[test]
fn test_conda_pyproject_wins_over_pixi_pyproject() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("pyproject.toml"),
        r#"
[tool.conda.workspace]
name = "demo"

[tool.pixi.workspace]
name = "demo-pixi"
"#,
    )
    .unwrap();

    assert_eq!(
        manifest_kind(&tmp.path().join("pyproject.toml")).unwrap(),
        ManifestKind::CondaPyproject
    );
}

#[test]
fn test_pyproject_requires_conda_or_pixi_config() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("pyproject.toml"),
        r#"
[tool.conda-ship]
source-environment = "ship"
"#,
    )
    .unwrap();

    assert!(!is_supported_pyproject_manifest(
        &tmp.path().join("pyproject.toml")
    ));
}

#[test]
fn test_find_installed_runtime_template_uses_env_override() {
    let tmp = TempDir::new().unwrap();
    let template = tmp.path().join(runtime_template_filename());
    std::fs::write(&template, b"runtime template").unwrap();

    temp_env::with_var(RUNTIME_TEMPLATE_ENV, Some(template.as_os_str()), || {
        assert_eq!(runtime_template_from_env().unwrap(), Some(template.clone()));
    });
}

#[test]
fn test_source_binary_prefers_explicit_template() {
    let tmp = TempDir::new().unwrap();
    let template = tmp.path().join("custom-template");
    std::fs::write(&template, b"runtime template").unwrap();

    assert_eq!(source_binary(Some(&template), None).unwrap(), template);
}

#[rstest]
#[case::cross_build(Some("x86_64-unknown-linux-gnu"), "cross-builds require --template")]
#[case::installed_template(None, "runtime template not found")]
fn test_source_binary_plan_reports_missing_template(
    #[case] target: Option<&str>,
    #[case] expected: &str,
) {
    temp_env::with_var(RUNTIME_TEMPLATE_ENV, None::<&str>, || {
        let err = source_binary_plan(None, target).unwrap_err().to_string();
        assert!(err.contains(expected));
    });
}

#[test]
fn test_empty_excludes_returns_all() {
    let packages = vec![make_pkg("a", &[]), make_pkg("b", &["a"])];
    let (filtered, removed) = filter_excluded(&packages, &[]).unwrap();
    assert!(removed.is_empty());
    assert_eq!(filtered.len(), 2);
}

#[test]
fn test_exclude_single_leaf() {
    let packages = vec![make_pkg("a", &[]), make_pkg("b", &[])];
    let excludes = vec!["b".to_string()];
    let (filtered, removed) = filter_excluded(&packages, &excludes).unwrap();
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
    let (filtered, removed) = filter_excluded(&packages, &excludes).unwrap();
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
    let (filtered, removed) = filter_excluded(&packages, &excludes).unwrap();
    assert_eq!(removed, vec!["a"]);
    assert_eq!(filtered.len(), 2);
}

#[test]
fn test_exclude_nonexistent_package() {
    let packages = vec![make_pkg("a", &[]), make_pkg("b", &[])];
    let excludes = vec!["nonexistent".to_string()];
    let (filtered, removed) = filter_excluded(&packages, &excludes).unwrap();
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
    let (filtered, removed) = filter_excluded(&packages, &excludes).unwrap();
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
    let (filtered, removed) = filter_excluded(&packages, &excludes).unwrap();
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

    validate_required_runtime_packages("linux-64", &packages).unwrap();
}

#[test]
fn test_validate_required_runtime_packages_rejects_missing_runtime_package() {
    let packages = vec![
        make_pkg("conda", &[]),
        make_pkg("conda-rattler-solver", &[]),
    ];

    let err = validate_required_runtime_packages("linux-64", &packages)
        .unwrap_err()
        .to_string();
    assert!(err.contains("missing required package(s): conda-spawn"));
}

#[rstest]
#[case::embedded_with_label(BundleLayout::Embedded, Some("linux-64"), "demoz-linux-64")]
#[case::external_with_label(BundleLayout::External, Some("linux-64"), "demo-linux-64")]
#[case::online_without_label(BundleLayout::Online, None, "demo")]
fn test_artifact_stem(
    #[case] layout: BundleLayout,
    #[case] target_label: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(artifact_stem("demo", layout, target_label), expected);
}

#[test]
fn test_stage_artifacts_external_outputs_bundle_and_metadata() {
    let tmp = TempDir::new().unwrap();
    let source_binary = tmp.path().join("cs-template");
    let source_bundle = tmp.path().join("bundle.tar.zst");
    std::fs::write(&source_binary, b"runtime template").unwrap();
    std::fs::write(&source_bundle, b"bundle archive").unwrap();

    let platform = Platform::Linux64;
    let platform_name = platform.to_string();
    let platform_data = PlatformData {
        name: rattler_lock::PlatformName::try_from(platform_name.clone()).unwrap(),
        subdir: platform,
        virtual_packages: Vec::new(),
    };
    let mut builder = LockFileBuilder::new()
        .with_platforms(vec![platform_data])
        .unwrap();
    builder
        .add_conda_package("default", platform_name.as_str(), make_pkg("conda", &[]))
        .unwrap();
    let lock_file = builder.finish();
    let content = lock_file.render_to_string().unwrap();
    let derived = DerivedRuntimeLock {
        input: ProjectInput {
            manifest_path: tmp.path().join("conda.toml"),
            manifest_kind: ManifestKind::CondaToml,
            lock_path: tmp.path().join("conda.lock"),
            config: ShipConfig::default(),
        },
        lock_file,
        content,
        source_environment: "ship".to_string(),
        runtime_config: RuntimeStampConfig {
            channels: vec!["conda-forge".to_string()],
            packages: vec!["conda".to_string()],
            delegate: Some("conda".to_string()),
            install_method: Some("homebrew".to_string()),
            ..RuntimeStampConfig::default()
        },
        platforms: vec![platform],
        total_packages: 1,
        total_excluded: 0,
        removed_excludes: Vec::new(),
    };

    let output = stage_artifacts(
        tmp.path(),
        &source_binary,
        BundleLayout::External,
        "demo",
        Some("linux-64"),
        platform,
        None,
        Path::new("dist"),
        &derived,
        Some(&source_bundle),
    )
    .unwrap();

    assert!(output.binary.is_file());
    let stamped = runtime_data::read_from_path(&output.binary)
        .unwrap()
        .expect("staged binary should be stamped");
    assert_eq!(stamped.header.install_method.as_deref(), Some("homebrew"));
    assert_eq!(
        stamped.header.runtime_config.packages,
        vec!["conda".to_string()]
    );
    let bundle = output
        .bundle
        .expect("external layout should stage a bundle");
    assert_eq!(
        bundle.file_name().and_then(|name| name.to_str()),
        Some("demo-linux-64.bundle.tar.zst")
    );
    assert!(bundle.is_file());

    let info: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&output.info).unwrap()).unwrap();
    assert_eq!(info["layout"], "external");
    assert_eq!(info["bundle"], "demo-linux-64.bundle.tar.zst");

    let checksums = std::fs::read_to_string(&output.checksums).unwrap();
    assert!(checksums.contains("demo-linux-64.bundle.tar.zst"));
}

#[rstest]
#[case::runtime_name("runtime name", validate_runtime_name, "conda-ship_1.0")]
#[case::delegate("delegate", validate_delegate, "python3.12")]
#[case::install_name("install name", validate_install_name, "conda-express_1.0")]
fn test_artifact_component_allows_filename_safe_values(
    #[case] _kind: &str,
    #[case] validate: fn(&str) -> miette::Result<()>,
    #[case] value: &str,
) {
    validate(value).unwrap();
}

#[rstest]
#[case::runtime_name_dot(validate_runtime_name, ".", "runtime name must not be . or ..")]
#[case::runtime_name_leading_dash(
    validate_runtime_name,
    "-demo",
    "runtime name must start with an ASCII letter or digit"
)]
#[case::runtime_name_path(
    validate_runtime_name,
    "demo/tool",
    "runtime name may only contain ASCII letters, digits, dots, dashes, and underscores"
)]
#[case::runtime_name_newline(
    validate_runtime_name,
    "demo\ntool",
    "runtime name may only contain ASCII letters, digits, dots, dashes, and underscores"
)]
#[case::target_label_path(
    validate_target_label,
    "linux/64",
    "target label may only contain ASCII letters, digits, dots, dashes, and underscores"
)]
#[case::target_triple_path(
    validate_target_triple,
    "custom/target.json",
    "target triple may only contain ASCII letters, digits, dots, dashes, and underscores"
)]
#[case::install_name_dot(validate_install_name, ".", "install name must not be . or ..")]
#[case::install_name_path(
    validate_install_name,
    "conda/express",
    "install name may only contain ASCII letters, digits, dots, dashes, and underscores"
)]
#[case::install_name_newline(
    validate_install_name,
    "express\n",
    "install name may only contain ASCII letters, digits, dots, dashes, and underscores"
)]
#[case::install_method_path(
    validate_install_method,
    "home/brew",
    "install method may only contain ASCII letters, digits, dots, dashes, and underscores"
)]
fn test_artifact_component_rejects_unsafe_values(
    #[case] validate: fn(&str) -> miette::Result<()>,
    #[case] value: &str,
    #[case] expected: &str,
) {
    let err = validate(value).unwrap_err().to_string();
    assert!(err.contains(expected));
}

#[test]
fn test_build_accepts_install_scheme_with_install_name() {
    let cli = Cli::try_parse_from([
        "cs",
        "build",
        "--runtime",
        "cx",
        "--delegate",
        "conda",
        "--install-scheme",
        "user-data",
        "--install-name",
        "express",
        "--install-method",
        "homebrew",
    ])
    .unwrap();

    let Command::Build {
        runtime,
        delegate,
        install_scheme,
        install_name,
        install_method,
        ..
    } = cli.command
    else {
        panic!("expected build command");
    };

    assert_eq!(runtime.as_deref(), Some("cx"));
    assert_eq!(delegate.as_deref(), Some("conda"));
    assert_eq!(install_scheme, Some(runtime_data::InstallScheme::UserData));
    assert_eq!(install_name.as_deref(), Some("express"));
    assert_eq!(install_method.as_deref(), Some("homebrew"));
}

#[test]
fn test_build_accepts_manifest_runtime_without_cli_runtime() {
    let cli = Cli::try_parse_from(["cs", "build"]).unwrap();

    let Command::Build {
        runtime, layout, ..
    } = cli.command
    else {
        panic!("expected build command");
    };

    assert_eq!(runtime, None);
    assert_eq!(layout, None);
}

#[test]
fn test_resolve_runtime_name_uses_manifest_config() {
    let config = ShipConfig {
        runtime: Some("demo".to_string()),
        ..ShipConfig::default()
    };

    assert_eq!(resolve_runtime_name(None, &config).unwrap(), "demo");
    assert_eq!(
        resolve_runtime_name(Some("override".to_string()), &config).unwrap(),
        "override"
    );
}

#[test]
fn test_resolve_delegate_uses_manifest_config() {
    let config = ShipConfig {
        delegate: Some("python".to_string()),
        ..ShipConfig::default()
    };

    assert_eq!(resolve_delegate(None, &config).unwrap(), "python");
    assert_eq!(
        resolve_delegate(Some("conda".to_string()), &config).unwrap(),
        "conda"
    );
}

#[test]
fn test_resolve_bundle_layout_uses_manifest_config() {
    let config = ShipConfig {
        layout: Some(BundleLayout::Embedded),
        ..ShipConfig::default()
    };

    assert_eq!(resolve_bundle_layout(None, &config), BundleLayout::Embedded);
    assert_eq!(
        resolve_bundle_layout(Some(BundleLayout::External), &config),
        BundleLayout::External
    );
    assert_eq!(
        resolve_bundle_layout(None, &ShipConfig::default()),
        BundleLayout::Online
    );
}

#[test]
fn test_build_accepts_dry_run() {
    let cli = Cli::try_parse_from(["cs", "build", "--runtime", "demo", "--dry-run"]).unwrap();

    let Command::Build { dry_run, .. } = cli.command else {
        panic!("expected build command");
    };

    assert!(dry_run);
}

#[test]
fn test_lock_subcommand_is_not_accepted() {
    let result = Cli::try_parse_from(["cs", "lock"]);

    assert!(result.is_err(), "cs lock should not be a public command");
}

#[test]
fn test_build_rejects_path_option() {
    let result = Cli::try_parse_from(["cs", "build", "--runtime", "demo", "--path", "/tmp/demo"]);

    assert!(result.is_err(), "build-time --path should not be accepted");
}

#[test]
fn test_run_rejects_path_option_before_runtime_args() {
    let result = Cli::try_parse_from([
        "cs",
        "run",
        "--runtime",
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
fn test_run_accepts_install_method() {
    let cli = Cli::try_parse_from(["cs", "run", "--install-method", "conda-forge", "--"]).unwrap();

    let Command::Run { install_method, .. } = cli.command else {
        panic!("expected run command");
    };

    assert_eq!(install_method.as_deref(), Some("conda-forge"));
}

#[rstest]
#[case::windows_target(Some("x86_64-pc-windows-msvc"), "demo.exe")]
#[case::unix_target(Some("x86_64-unknown-linux-gnu"), "demo")]
fn test_binary_filename(#[case] target: Option<&str>, #[case] expected: &str) {
    assert_eq!(binary_filename("demo", target), expected);
}

#[test]
fn test_binary_filename_for_current_target() {
    let expected = if cfg!(windows) { "demo.exe" } else { "demo" };

    assert_eq!(binary_filename("demo", None), expected);
}

#[rstest]
#[case::conda("python-3.12-h123_0.conda")]
#[case::tar_bz2("python-3.12-h123_0.tar.bz2")]
fn test_package_archive_name_accepts_conda_archives(#[case] name: &str) {
    assert!(validate_package_archive_name(name).is_ok());
}

#[rstest]
#[case::parent_dir("../python-3.12-h123_0.conda")]
#[case::nested("nested/python-3.12-h123_0.conda")]
#[case::wrong_suffix("python-3.12-h123_0.zip")]
fn test_package_archive_name_rejects_invalid_archives(#[case] name: &str) {
    assert!(validate_package_archive_name(name).is_err());
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
