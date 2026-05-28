//! Integration tests verifying the embedded lockfile has been pre-filtered
//! by `pronto-build prepare` (exclude filter applied at build time).

use std::str::FromStr;

use rattler_conda_types::{Platform, RepoDataRecord};
use rattler_lock::LockFile;

const EMBEDDED_LOCK: &str = include_str!(concat!(env!("OUT_DIR"), "/cx.lock"));

fn records_from_embedded_lock() -> Vec<RepoDataRecord> {
    let lock_file = LockFile::from_str(EMBEDDED_LOCK).expect("failed to parse embedded lockfile");
    let env = lock_file
        .default_environment()
        .expect("no default environment");
    let platform = Platform::current();
    env.conda_repodata_records(platform)
        .expect("failed to extract records")
        .expect("no records for current platform")
}

fn sorted_names(records: &[RepoDataRecord]) -> Vec<String> {
    let mut names: Vec<String> = records
        .iter()
        .map(|r| r.package_record.name.as_normalized().to_string())
        .collect();
    names.sort();
    names
}

#[test]
fn test_embedded_lockfile_package_composition() {
    let records = records_from_embedded_lock();
    let names = sorted_names(&records);

    let excluded = ["conda-libmamba-solver", "libmamba", "libsolv"];
    for pkg in &excluded {
        assert!(
            !names.contains(&pkg.to_string()),
            "embedded lockfile should not contain {pkg} (pre-filtered by pronto-build prepare)"
        );
    }

    let required = [
        "conda",
        "conda-rattler-solver",
        "conda-spawn",
        "conda-pypi",
        "conda-self",
    ];
    for pkg in &required {
        assert!(
            names.contains(&pkg.to_string()),
            "embedded lockfile should contain {pkg}"
        );
    }
    assert!(
        names.iter().any(|n| n.starts_with("python")),
        "embedded lockfile should contain python"
    );
}
