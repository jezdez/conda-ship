//! Integration tests verifying the derived runtime lock has been pre-filtered.
#![cfg(feature = "runtime-template")]

use std::process::{Command, Stdio};

fn package_names_from_inspect() -> Vec<String> {
    let root = env!("CARGO_MANIFEST_DIR");
    let assert = assert_cmd::cargo::cargo_bin_cmd!("cs")
        .args(["inspect", "--json", "--root", root])
        .assert()
        .success();
    let output = assert.get_output();
    let inspect: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("failed to parse inspect JSON");

    let mut names: Vec<_> = inspect["packages"]
        .as_array()
        .expect("inspect packages should be an array")
        .iter()
        .map(|package| {
            package["name"]
                .as_str()
                .expect("inspect package should include a name")
                .to_string()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn test_derived_lockfile_package_composition() {
    let names = package_names_from_inspect();

    let excluded = ["conda-libmamba-solver", "libmamba", "libsolv"];
    for pkg in &excluded {
        assert!(
            !names.contains(&pkg.to_string()),
            "derived runtime lock should not contain {pkg}"
        );
    }

    let required = ["conda", "conda-rattler-solver", "conda-spawn"];
    for pkg in &required {
        assert!(
            names.contains(&pkg.to_string()),
            "derived runtime lock should contain {pkg}"
        );
    }
    assert!(
        names.iter().any(|n| n.starts_with("python")),
        "derived runtime lock should contain python"
    );
}

#[test]
fn test_inspect_json_treats_closed_stdout_as_success() {
    let root = env!("CARGO_MANIFEST_DIR");
    let mut child = Command::new(assert_cmd::cargo::cargo_bin("cs"))
        .args(["inspect", "--json", "--root", root])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn cs inspect");

    drop(child.stdout.take());
    let output = child
        .wait_with_output()
        .expect("failed to wait for cs inspect");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "closed stdout should not fail; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "closed stdout should not emit a Rust panic; stderr:\n{stderr}"
    );
}
