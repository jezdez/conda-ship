//! Post-solve package exclusion filter.
//!
//! Shared between the compile-time build script (`build.rs`) and the runtime
//! binary (`install.rs`).

use std::collections::{HashMap, HashSet};

use rattler_conda_types::{PackageName, RepoDataRecord};

/// Remove explicitly excluded packages and any of their dependencies that are
/// not required by any remaining package.
///
/// Walks the reverse-dependency graph: starting from the excluded set, it
/// transitively removes dependencies whose *every* dependent has already been
/// removed.
pub fn filter_excluded_packages(
    packages: Vec<RepoDataRecord>,
    excludes: &[String],
) -> (Vec<RepoDataRecord>, Vec<String>) {
    let exclude_set: HashSet<&str> = excludes.iter().map(|s| s.as_str()).collect();

    let name_of = |r: &RepoDataRecord| r.package_record.name.as_normalized().to_string();
    let pkg_names: Vec<String> = packages.iter().map(name_of).collect();
    let name_to_idx: HashMap<&str, usize> = pkg_names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();

    let n = packages.len();
    let mut reverse_deps: Vec<HashSet<usize>> = vec![HashSet::new(); n];
    for (i, rec) in packages.iter().enumerate() {
        for dep_str in &rec.package_record.depends {
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
        for dep_str in &packages[pkg_idx].package_record.depends {
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

    let removed_names: Vec<String> = removed
        .iter()
        .map(|&i| pkg_names[i].clone())
        .collect::<Vec<_>>();

    let filtered: Vec<RepoDataRecord> = packages
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !removed.contains(i))
        .map(|(_, r)| r)
        .collect();

    let mut sorted_names = removed_names;
    sorted_names.sort();
    (filtered, sorted_names)
}

/// Extract sorted package names from a slice of records.
#[allow(dead_code)] // used in integration tests; appears dead in build.rs context
pub fn sorted_names(records: &[RepoDataRecord]) -> Vec<String> {
    let mut names: Vec<String> = records
        .iter()
        .map(|r| r.package_record.name.as_normalized().to_string())
        .collect();
    names.sort();
    names
}

#[cfg(test)]
pub(crate) mod tests {
    use std::str::FromStr;

    use super::*;
    use rattler_conda_types::{
        PackageRecord, VersionWithSource,
        package::{CondaArchiveIdentifier, DistArchiveIdentifier},
    };
    use reqwest::Url;

    pub(crate) fn make_record(name: &str, depends: &[&str]) -> RepoDataRecord {
        let mut record = PackageRecord::new(
            PackageName::new_unchecked(name),
            VersionWithSource::from_str("1.0").unwrap(),
            "0".to_string(),
        );
        record.depends = depends.iter().map(|d| d.to_string()).collect();

        let archive_name = format!("{name}-1.0-0.conda");
        RepoDataRecord {
            package_record: record,
            identifier: DistArchiveIdentifier::from(
                archive_name.parse::<CondaArchiveIdentifier>().unwrap(),
            ),
            url: Url::parse(&format!("https://example.com/{name}-1.0-0.conda")).unwrap(),
            channel: Some("test".to_string()),
        }
    }

    #[allow(dead_code)] // used by install::tests
    pub(crate) fn make_test_records() -> Vec<RepoDataRecord> {
        vec![
            make_record("a", &["c"]),
            make_record("b", &["c"]),
            make_record("c", &[]),
        ]
    }

    #[test]
    fn test_empty_excludes_returns_all() {
        let packages = vec![make_record("a", &[]), make_record("b", &["a"])];
        let (filtered, removed) = filter_excluded_packages(packages, &[]);
        assert!(removed.is_empty(), "no packages should be removed");
        assert_eq!(sorted_names(&filtered), vec!["a", "b"]);
    }

    #[test]
    fn test_exclude_single_leaf() {
        let packages = vec![make_record("a", &[]), make_record("b", &[])];
        let excludes = vec!["b".to_string()];
        let (filtered, removed) = filter_excluded_packages(packages, &excludes);
        assert_eq!(removed, vec!["b"]);
        assert_eq!(sorted_names(&filtered), vec!["a"]);
    }

    #[test]
    fn test_exclude_with_transitive_deps() {
        let packages = vec![
            make_record("a", &["b"]),
            make_record("b", &["c"]),
            make_record("c", &[]),
        ];
        let excludes = vec!["a".to_string()];
        let (filtered, removed) = filter_excluded_packages(packages, &excludes);
        assert_eq!(removed, vec!["a", "b", "c"]);
        assert!(filtered.is_empty(), "all packages should be removed");
    }

    #[test]
    fn test_shared_dep_not_removed() {
        let packages = vec![
            make_record("a", &["c"]),
            make_record("b", &["c"]),
            make_record("c", &[]),
        ];
        let excludes = vec!["a".to_string()];
        let (filtered, removed) = filter_excluded_packages(packages, &excludes);
        assert_eq!(removed, vec!["a"]);
        assert_eq!(sorted_names(&filtered), vec!["b", "c"]);
    }

    #[test]
    fn test_exclude_nonexistent_package() {
        let packages = vec![make_record("a", &[]), make_record("b", &[])];
        let excludes = vec!["nonexistent".to_string()];
        let (filtered, removed) = filter_excluded_packages(packages, &excludes);
        assert!(removed.is_empty(), "nonexistent package should be no-op");
        assert_eq!(sorted_names(&filtered), vec!["a", "b"]);
    }

    #[test]
    fn test_diamond_dependency() {
        let packages = vec![
            make_record("a", &["c"]),
            make_record("b", &["c"]),
            make_record("c", &[]),
            make_record("d", &["a"]),
        ];
        let excludes = vec!["d".to_string()];
        let (filtered, removed) = filter_excluded_packages(packages, &excludes);
        assert_eq!(removed, vec!["a", "d"]);
        assert_eq!(sorted_names(&filtered), vec!["b", "c"]);
    }

    #[test]
    fn test_multiple_simultaneous_excludes() {
        let packages = vec![
            make_record("a", &["shared"]),
            make_record("b", &["only-b"]),
            make_record("shared", &[]),
            make_record("only-b", &[]),
            make_record("keep", &[]),
        ];
        let excludes = vec!["a".to_string(), "b".to_string()];
        let (filtered, removed) = filter_excluded_packages(packages, &excludes);
        assert_eq!(removed, vec!["a", "b", "only-b", "shared"]);
        assert_eq!(sorted_names(&filtered), vec!["keep"]);
    }
}
