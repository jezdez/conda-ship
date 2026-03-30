//! Integration tests for the cx binary using assert_cmd.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use rstest::rstest;
use tempfile::TempDir;

fn cx() -> assert_cmd::Command {
    cargo_bin_cmd!("cx")
}

#[test]
fn test_cx_help() {
    let output = cx().arg("--help").output().unwrap();
    assert!(output.status.success(), "cx --help should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout).replace("cx.exe", "cx");
    insta::assert_snapshot!("cx_help", stdout);
}

#[test]
fn test_cx_version() {
    cx().arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[derive(Clone, Copy)]
enum MissingPrefixCmd {
    Status,
    Uninstall,
}

#[rstest]
#[case::status(MissingPrefixCmd::Status)]
#[case::uninstall(MissingPrefixCmd::Uninstall)]
fn test_cx_nonexistent_prefix_reports_missing(#[case] cmd: MissingPrefixCmd) {
    let tmp = TempDir::new().unwrap();
    let nonexistent = tmp.path().join("does-not-exist");
    let path = nonexistent.to_str().unwrap();

    let mut c = cx();
    match cmd {
        MissingPrefixCmd::Status => {
            c.args(["status", "--prefix", path]);
        }
        MissingPrefixCmd::Uninstall => {
            c.args(["uninstall", "--prefix", path, "--yes"]);
        }
    }
    c.assert()
        .success()
        .stderr(predicate::str::contains("No conda installation found"));
}

#[test]
fn test_cx_bootstrap_already_exists() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir(tmp.path().join("conda-meta")).unwrap();

    cx().args(["bootstrap", "--prefix", tmp.path().to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("already bootstrapped"));
}

#[cfg_attr(not(feature = "online_tests"), ignore)]
#[test]
fn test_cx_bootstrap_to_temp_prefix() {
    let tmp = TempDir::new().unwrap();
    let prefix = tmp.path().join("cx-test-bootstrap");

    cx().args(["bootstrap", "--prefix", prefix.to_str().unwrap()])
        .timeout(std::time::Duration::from_secs(120))
        .assert()
        .success()
        .stderr(predicate::str::contains("bootstrapped successfully"));

    assert!(
        prefix.join("conda-meta").is_dir(),
        "conda-meta should exist"
    );
    assert!(prefix.join(".cx.json").exists(), ".cx.json should exist");
    assert!(prefix.join(".condarc").exists(), ".condarc should exist");
    assert!(
        prefix.join("conda-meta/frozen").exists(),
        "frozen marker should exist"
    );
}

#[cfg_attr(not(feature = "online_tests"), ignore)]
#[test]
fn test_cx_status_after_bootstrap() {
    let tmp = TempDir::new().unwrap();
    let prefix = tmp.path().join("cx-test-status");

    cx().args(["bootstrap", "--prefix", prefix.to_str().unwrap()])
        .timeout(std::time::Duration::from_secs(120))
        .assert()
        .success();

    cx().args(["status", "--prefix", prefix.to_str().unwrap()])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("channels:")
                .and(predicate::str::contains("packages:"))
                .and(predicate::str::contains("installed:")),
        );
}

#[cfg_attr(not(feature = "online_tests"), ignore)]
#[test]
fn test_cx_uninstall_removes_prefix() {
    let tmp = TempDir::new().unwrap();
    let prefix = tmp.path().join("cx-test-uninstall");

    cx().args(["bootstrap", "--prefix", prefix.to_str().unwrap()])
        .timeout(std::time::Duration::from_secs(120))
        .assert()
        .success();

    assert!(prefix.exists(), "prefix should exist after bootstrap");

    cx().args(["uninstall", "--prefix", prefix.to_str().unwrap(), "--yes"])
        .assert()
        .success()
        .stderr(predicate::str::contains("uninstalled"));

    assert!(!prefix.exists(), "prefix should be removed after uninstall");
}

/// Uses `--prefix` so Windows CI does not depend on `dirs::home_dir()` (known-folder profile),
/// which ignores `HOME` / `USERPROFILE` for a synthetic layout.
#[test]
fn test_cx_uninstall_interactive_prompt_declined() {
    let tmp = TempDir::new().unwrap();
    let prefix = tmp.path().join("cx-uninstall-interactive");
    std::fs::create_dir_all(prefix.join("conda-meta")).unwrap();

    cx().args(["uninstall", "--prefix", prefix.to_str().unwrap()])
        .write_stdin("n\n")
        .assert()
        .success()
        .stderr(
            predicate::str::contains("Continue? [y/N]").and(predicate::str::contains("Aborted.")),
        );

    assert!(
        prefix.exists(),
        "prefix should remain after declining uninstall"
    );
}

#[cfg(unix)]
#[test]
fn test_cx_uninstall_default_prefix_respects_home() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join(".cx/conda-meta")).unwrap();

    cx().env("HOME", tmp.path().as_os_str())
        .arg("uninstall")
        .write_stdin("n\n")
        .assert()
        .success()
        .stderr(
            predicate::str::contains("Continue? [y/N]").and(predicate::str::contains("Aborted.")),
        );

    assert!(tmp.path().join(".cx").exists());
}
