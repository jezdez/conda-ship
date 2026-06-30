#![cfg(feature = "fleet")]

use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use rattler_conda_types::Platform;
use serde_json::Value;
use tempfile::TempDir;

fn empty_lock() -> String {
    let platform = Platform::current();
    format!(
        r#"
---
version: 6
environments:
  default:
    channels:
      - url: https://conda.anaconda.org/conda-forge
    packages:
      {platform}: []
packages: []
"#
    )
}

fn write_spec(tmp: &TempDir, id: &str, delegate: &str) -> PathBuf {
    let spec_path = tmp.path().join(format!("{id}.json"));
    let spec = serde_json::json!({
        "id": id,
        "version": "1.0.0",
        "delegate_executable": delegate,
        "lock_content": empty_lock(),
        "channels": ["conda-forge"],
        "requested_specs": [],
    });
    std::fs::write(&spec_path, serde_json::to_string_pretty(&spec).unwrap()).unwrap();
    spec_path
}

fn install_runtime(install_root: &Path, spec_path: &Path) {
    cargo_bin_cmd!("nan")
        .args([
            "--install-root",
            install_root.to_str().unwrap(),
            "install",
            "--spec",
            spec_path.to_str().unwrap(),
        ])
        .assert()
        .success();
}

fn nan_json(args: &[&str]) -> Value {
    let output = cargo_bin_cmd!("nan")
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).unwrap()
}

fn executable_path(prefix: &Path, name: &str) -> PathBuf {
    if cfg!(windows) {
        prefix.join(format!("{name}.exe"))
    } else {
        prefix.join("bin").join(name)
    }
}

#[test]
fn test_nan_list_and_status_emit_json() {
    let tmp = TempDir::new().unwrap();
    let install_root = tmp.path().join("fleet");
    let spec_path = write_spec(&tmp, "tool", "runner");
    install_runtime(&install_root, &spec_path);

    let list = nan_json(&["--install-root", install_root.to_str().unwrap(), "list"]);
    assert_eq!(list[0]["id"], "tool");
    assert_eq!(list[0]["delegate_executable"], "runner");

    let status = nan_json(&[
        "--install-root",
        install_root.to_str().unwrap(),
        "status",
        "tool",
    ]);
    assert_eq!(status["id"], "tool");
    assert_eq!(
        status["prefix"].as_str().unwrap(),
        install_root.join("tool").to_string_lossy()
    );

    let missing = nan_json(&[
        "--install-root",
        install_root.to_str().unwrap(),
        "status",
        "missing",
    ]);
    assert!(missing.is_null());
}

#[test]
fn test_nan_shim_plan_emits_json() {
    let tmp = TempDir::new().unwrap();
    let install_root = tmp.path().join("fleet");
    let spec_path = write_spec(&tmp, "tool", "runner");
    install_runtime(&install_root, &spec_path);

    let executable = executable_path(&install_root.join("tool"), "runner");
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    std::fs::write(&executable, "stub").unwrap();

    let plan = nan_json(&[
        "--install-root",
        install_root.to_str().unwrap(),
        "shim-plan",
        "tool",
        "runner",
        "--shim-name",
        "nan",
    ]);
    assert_eq!(plan["shim_name"], "nan");
    assert_eq!(plan["target_command"], "runner");
    assert_eq!(
        plan["target_executable"].as_str().unwrap(),
        executable.to_string_lossy()
    );
    assert!(
        plan["wrapper_contents"]
            .as_str()
            .unwrap()
            .contains("conda-fleet-shim")
    );
}
