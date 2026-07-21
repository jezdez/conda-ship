//! Integration tests for direct stamped-runtime delegation.
#![cfg(feature = "runtime-template")]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use rstest::rstest;
use tempfile::TempDir;

fn runtime() -> assert_cmd::Command {
    let mut cmd = cargo_bin_cmd!("cs-template");
    cmd.env("CONDA_SHIP_ALLOW_UNSTAMPED_TEMPLATE", "1");
    cmd
}

fn runtime_at(prefix: &Path) -> assert_cmd::Command {
    let mut cmd = runtime();
    cmd.env("CS_TEMPLATE_PREFIX", prefix);
    cmd
}

fn write_demo_runtime_metadata(prefix: &Path) {
    std::fs::write(
        prefix.join(".demo.json"),
        r#"{"schema_version":1,"display_name":"demo","install_name":"demo","metadata_file":".demo.json","version":"test","channels":[],"packages":[]}"#,
    )
    .unwrap();
}

fn build_stamped_runtime(tmp: &TempDir, delegate: &str) -> PathBuf {
    let root = env!("CARGO_MANIFEST_DIR");
    let template = assert_cmd::cargo::cargo_bin!("cs-template");
    let out_dir = tmp.path().join("dist");

    cargo_bin_cmd!("cs")
        .env("CONDA_SHIP_TEMPLATE", template)
        .args([
            "build",
            "--root",
            root,
            "--runtime-name",
            "demo",
            "--delegate-executable",
            delegate,
            "--runtime-version",
            "9.8.7",
            "--out-dir",
            out_dir.to_str().unwrap(),
        ])
        .assert()
        .success();

    out_dir.join(if cfg!(windows) { "demo.exe" } else { "demo" })
}

fn install_cs_delegate(prefix: &Path) {
    let destination = if cfg!(windows) {
        prefix.join("cs.exe")
    } else {
        prefix.join("bin").join("cs")
    };
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    std::fs::copy(assert_cmd::cargo::cargo_bin!("cs"), &destination).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn test_runtime_template_refuses_to_run_without_stamp() {
    cargo_bin_cmd!("cs-template")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "runtime template, not a runnable conda runtime",
        ));
}

#[rstest]
#[case(&["--help"])]
#[case(&["--version"])]
#[case(&["bootstrap"])]
#[case(&["status"])]
#[case(&["shell"])]
#[case(&["uninstall"])]
fn test_runtime_does_not_parse_delegate_arguments(#[case] args: &[&str]) {
    let tmp = TempDir::new().unwrap();
    runtime_at(tmp.path())
        .args(args)
        .assert()
        .failure()
        .stderr(predicate::str::contains("runtime has no stamped lockfile"));
}

#[test]
fn test_runtime_refuses_an_unmanaged_existing_prefix() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir(tmp.path().join("conda-meta")).unwrap();

    runtime_at(tmp.path())
        .arg("--help")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unmanaged install path"));
}

#[test]
fn test_runtime_bundle_env_var_rejects_missing_directory() {
    let tmp = TempDir::new().unwrap();
    runtime_at(tmp.path())
        .env("CS_TEMPLATE_BUNDLE", tmp.path().join("missing"))
        .arg("info")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "configured bundle path is not a directory",
        ));
}

#[rstest]
#[case::set_to_1("1", true)]
#[case::set_to_true("true", true)]
#[case::set_to_0("0", false)]
#[case::set_to_false("false", false)]
#[case::empty("", false)]
fn test_runtime_offline_env_var_parsing(#[case] value: &str, #[case] offline: bool) {
    let tmp = TempDir::new().unwrap();
    let expected = if offline {
        "offline bootstrap requires a stamped runtime lock"
    } else {
        "runtime has no stamped lockfile"
    };

    runtime_at(tmp.path())
        .env("CS_TEMPLATE_OFFLINE", value)
        .arg("info")
        .assert()
        .failure()
        .stderr(predicate::str::contains(expected));
}

#[test]
fn test_stamped_runtime_delegates_help_to_configured_executable() {
    let tmp = TempDir::new().unwrap();
    let binary = build_stamped_runtime(&tmp, "cs");
    let prefix = tmp.path().join("prefix");
    std::fs::create_dir_all(prefix.join("conda-meta")).unwrap();
    write_demo_runtime_metadata(&prefix);
    install_cs_delegate(&prefix);

    assert_cmd::Command::new(binary)
        .env("DEMO_PREFIX", &prefix)
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Build ready-to-run conda runtimes",
        ));
}

#[rstest]
#[case("status")]
#[case("shell")]
#[case("uninstall")]
fn test_stamped_runtime_has_no_private_management_subcommands(#[case] arg: &str) {
    let tmp = TempDir::new().unwrap();
    let binary = build_stamped_runtime(&tmp, "cs");
    let prefix = tmp.path().join("prefix");
    std::fs::create_dir_all(prefix.join("conda-meta")).unwrap();
    write_demo_runtime_metadata(&prefix);
    install_cs_delegate(&prefix);

    assert_cmd::Command::new(binary)
        .env("DEMO_PREFIX", &prefix)
        .arg(arg)
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
}

#[cfg_attr(not(feature = "online_tests"), ignore)]
#[test]
fn test_first_delegate_command_auto_bootstraps_stamped_runtime() {
    let tmp = TempDir::new().unwrap();
    let binary = build_stamped_runtime(&tmp, "conda");
    let prefix = tmp.path().join("prefix");

    assert_cmd::Command::new(binary)
        .env("DEMO_PREFIX", &prefix)
        .args([OsString::from("info"), OsString::from("--json")])
        .timeout(std::time::Duration::from_secs(300))
        .assert()
        .success();

    assert!(prefix.join("conda-meta").is_dir());
    assert!(prefix.join(".demo.json").is_file());
}
