//! Package installation — lockfile fast-path and live-solve fallback.

use std::{
    borrow::Cow,
    collections::HashMap,
    env,
    future::IntoFuture,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use miette::{Context, IntoDiagnostic};
use rattler::{
    default_cache_dir,
    install::{IndicatifReporter, Installer},
    package_cache::PackageCache,
};
use rattler_conda_types::{
    Channel, ChannelConfig, GenericVirtualPackage, MatchSpec, ParseMatchSpecOptions, Platform,
    PrefixRecord, RepoDataRecord,
};
use rattler_lock::LockFile;
use rattler_networking::AuthenticationMiddleware;
use rattler_repodata_gateway::{Gateway, RepoData, SourceConfig};
use rattler_solve::{SolverImpl, SolverTask, resolvo};

use crate::config;
use crate::exclude::filter_excluded_packages;

static GLOBAL_MP: std::sync::LazyLock<MultiProgress> = std::sync::LazyLock::new(|| {
    let mp = MultiProgress::new();
    mp.set_draw_target(ProgressDrawTarget::stderr_with_hz(20));
    mp
});

pub(crate) fn multi_progress() -> MultiProgress {
    GLOBAL_MP.clone()
}

/// Parse a lockfile and return the platform and filtered records.
fn lockfile_records(
    lock_content: &str,
    excludes: &[String],
) -> miette::Result<(Platform, Vec<RepoDataRecord>)> {
    let lock_file = LockFile::from_str(lock_content)
        .into_diagnostic()
        .context("failed to parse lockfile")?;

    let env = lock_file
        .default_environment()
        .ok_or_else(|| miette::miette!("lockfile has no default environment"))?;

    let platform = Platform::current();
    let records = env
        .conda_repodata_records(platform)
        .into_diagnostic()
        .context("failed to extract records from lockfile")?
        .ok_or_else(|| miette::miette!("lockfile has no records for platform {}", platform))?;

    eprintln!(
        "   Lockfile contains {} packages for {}",
        records.len(),
        platform
    );

    Ok((platform, apply_excludes(records, excludes)))
}

/// Install packages from a pre-solved lockfile (fast path, no solve needed).
pub async fn from_lockfile(
    prefix: &Path,
    lock_content: &str,
    excludes: &[String],
) -> miette::Result<()> {
    let (platform, required_packages) = lockfile_records(lock_content, excludes)?;

    let cfg = config::embedded_config();
    let match_specs = parse_specs(&cfg.packages)?;
    let installed = PrefixRecord::collect_from_prefix::<PrefixRecord>(prefix).into_diagnostic()?;
    let client = make_download_client()?;

    run_installer(
        prefix,
        platform,
        &installed,
        &match_specs,
        client,
        required_packages,
    )
    .await
}

/// Install packages from a lockfile using a local payload directory.
///
/// Pre-populates the rattler package cache from the payload directory, then
/// runs the normal install path. When `offline` is true, no download client
/// is configured — all packages must be present in the payload or cache.
pub async fn from_lockfile_with_payload(
    prefix: &Path,
    lock_content: &str,
    excludes: &[String],
    payload_dir: &Path,
    offline: bool,
) -> miette::Result<()> {
    let (platform, required_packages) = lockfile_records(lock_content, excludes)?;

    let payload_index = index_payload_dir(payload_dir)?;
    let (matched, missing) = match_records_to_payload(&required_packages, &payload_index);

    if offline && !missing.is_empty() {
        return Err(miette::miette!(
            "offline mode: {} package(s) not found in payload: {}",
            missing.len(),
            missing.join(", ")
        ));
    }

    eprintln!(
        "   Payload: {}/{} packages found locally",
        matched.len(),
        required_packages.len()
    );

    let cache_dir = default_cache_dir()
        .map_err(|e| miette::miette!("could not determine cache directory: {}", e))?;
    rattler_cache::ensure_cache_dir(&cache_dir)
        .map_err(|e| miette::miette!("could not create cache directory: {}", e))?;

    let package_cache = PackageCache::new(cache_dir.join(rattler_cache::PACKAGE_CACHE_DIR));

    let start = Instant::now();
    for path in &matched {
        package_cache
            .get_or_fetch_from_path(path, None)
            .await
            .into_diagnostic()
            .context(format!(
                "failed to cache package from payload: {}",
                path.display()
            ))?;
    }
    eprintln!(
        "   Cached {} packages from payload in {:.1}s",
        matched.len(),
        start.elapsed().as_secs_f64()
    );

    let cfg = config::embedded_config();
    let match_specs = parse_specs(&cfg.packages)?;
    let installed = PrefixRecord::collect_from_prefix::<PrefixRecord>(prefix).into_diagnostic()?;

    let mut installer = Installer::new()
        .with_package_cache(package_cache)
        .with_target_platform(platform)
        .with_installed_packages(installed.to_vec())
        .with_execute_link_scripts(true)
        .with_requested_specs(match_specs)
        .with_reporter(
            IndicatifReporter::builder()
                .with_multi_progress(multi_progress())
                .finish(),
        );

    if !offline {
        installer = installer.with_download_client(make_download_client()?);
    }

    let start = Instant::now();
    let result = installer
        .install(prefix, required_packages)
        .await
        .into_diagnostic()
        .context("failed to install packages")?;

    if result.transaction.operations.is_empty() {
        eprintln!("   {} Already up to date", console::style("✔").green());
    } else {
        eprintln!(
            "   Installed {} packages in {:.1}s",
            result.transaction.operations.len(),
            start.elapsed().as_secs_f64()
        );
    }
    Ok(())
}

/// Install packages from a lockfile in offline mode (cache only, no payload).
pub async fn from_lockfile_offline(
    prefix: &Path,
    lock_content: &str,
    excludes: &[String],
) -> miette::Result<()> {
    let (platform, required_packages) = lockfile_records(lock_content, excludes)?;

    let cache_dir = default_cache_dir()
        .map_err(|e| miette::miette!("could not determine cache directory: {}", e))?;
    let package_cache = PackageCache::new(cache_dir.join(rattler_cache::PACKAGE_CACHE_DIR));

    let cfg = config::embedded_config();
    let match_specs = parse_specs(&cfg.packages)?;
    let installed = PrefixRecord::collect_from_prefix::<PrefixRecord>(prefix).into_diagnostic()?;

    let start = Instant::now();
    let result = Installer::new()
        .with_package_cache(package_cache)
        .with_target_platform(platform)
        .with_installed_packages(installed.to_vec())
        .with_execute_link_scripts(true)
        .with_requested_specs(match_specs)
        .with_reporter(
            IndicatifReporter::builder()
                .with_multi_progress(multi_progress())
                .finish(),
        )
        .install(prefix, required_packages)
        .await
        .into_diagnostic()
        .context("failed to install packages (offline mode — are all packages cached?)")?;

    if result.transaction.operations.is_empty() {
        eprintln!("   {} Already up to date", console::style("✔").green());
    } else {
        eprintln!(
            "   Installed {} packages in {:.1}s",
            result.transaction.operations.len(),
            start.elapsed().as_secs_f64()
        );
    }
    Ok(())
}

/// Extract the embedded payload (if any) to a temporary directory.
///
/// Returns `Some(path)` when the binary was built with `CX_EMBED_PAYLOAD=1`
/// and contains a non-empty `payload.tar.zst`. Returns `None` for standard
/// `cx` builds where the payload is empty.
pub fn extract_embedded_payload() -> miette::Result<Option<PathBuf>> {
    let payload = config::EMBEDDED_PAYLOAD;
    if payload.is_empty() {
        return Ok(None);
    }

    let tmp_dir = std::env::temp_dir().join(format!("cxz-payload-{}", std::process::id()));
    std::fs::create_dir_all(&tmp_dir)
        .into_diagnostic()
        .context("failed to create temp dir for embedded payload")?;

    let decoder = zstd::Decoder::new(payload)
        .into_diagnostic()
        .context("failed to decompress embedded payload")?;
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(&tmp_dir)
        .into_diagnostic()
        .context("failed to unpack embedded payload")?;

    eprintln!(
        "   Extracted embedded payload ({:.1} MB) to {}",
        payload.len() as f64 / 1_048_576.0,
        tmp_dir.display()
    );

    Ok(Some(tmp_dir))
}

/// Scan a directory for `.conda` and `.tar.bz2` package archives.
///
/// Returns a map from filename to full path.
pub(crate) fn index_payload_dir(dir: &Path) -> miette::Result<HashMap<String, PathBuf>> {
    let mut index = HashMap::new();
    let entries = std::fs::read_dir(dir).into_diagnostic().context(format!(
        "failed to read payload directory: {}",
        dir.display()
    ))?;

    for entry in entries {
        let entry = entry.into_diagnostic()?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name.ends_with(".conda") || name.ends_with(".tar.bz2") {
            index.insert(name, path);
        }
    }
    Ok(index)
}

/// Match lockfile records to files in a payload index.
///
/// Returns `(matched_paths, missing_names)` where `missing_names` lists
/// packages not found in the payload.
pub(crate) fn match_records_to_payload(
    records: &[RepoDataRecord],
    payload_index: &HashMap<String, PathBuf>,
) -> (Vec<PathBuf>, Vec<String>) {
    let mut matched = Vec::new();
    let mut missing = Vec::new();

    for record in records {
        let filename = record
            .url
            .path_segments()
            .and_then(|mut s| s.next_back())
            .unwrap_or_default()
            .to_string();

        if let Some(path) = payload_index.get(&filename) {
            matched.push(path.clone());
        } else {
            missing.push(filename);
        }
    }
    (matched, missing)
}

/// Fetch repodata, solve, and install packages into the prefix.
pub async fn from_solve(
    prefix: &Path,
    channels: &[String],
    specs: &[String],
    excludes: &[String],
) -> miette::Result<()> {
    let channel_config =
        ChannelConfig::default_with_root_dir(env::current_dir().into_diagnostic()?);
    let platform = Platform::current();
    let match_specs = parse_specs(specs)?;

    let cache_dir = default_cache_dir()
        .map_err(|e| miette::miette!("could not determine cache directory: {}", e))?;
    rattler_cache::ensure_cache_dir(&cache_dir)
        .map_err(|e| miette::miette!("could not create cache directory: {}", e))?;

    let parsed_channels: Vec<Channel> = channels
        .iter()
        .map(|c| Channel::from_str(c, &channel_config))
        .collect::<Result<Vec<_>, _>>()
        .into_diagnostic()?;

    let installed = PrefixRecord::collect_from_prefix::<PrefixRecord>(prefix).into_diagnostic()?;
    let client = make_download_client()?;

    let gateway = Gateway::builder()
        .with_cache_dir(cache_dir.join(rattler_cache::REPODATA_CACHE_DIR))
        .with_package_cache(PackageCache::new(
            cache_dir.join(rattler_cache::PACKAGE_CACHE_DIR),
        ))
        .with_client(client.clone())
        .with_channel_config(rattler_repodata_gateway::ChannelConfig {
            default: SourceConfig {
                sharded_enabled: true,
                ..SourceConfig::default()
            },
            per_channel: HashMap::new(),
        })
        .finish();

    let start = Instant::now();
    let repo_data = wrap_async_spinner(
        "fetching repodata",
        gateway
            .query(
                parsed_channels,
                [platform, Platform::NoArch],
                match_specs.clone(),
            )
            .recursive(true),
    )
    .await
    .into_diagnostic()
    .context("failed to load repodata")?;

    let total_records: usize = repo_data.iter().map(RepoData::len).sum();
    eprintln!(
        "   Loaded {} records in {:.1}s",
        total_records,
        start.elapsed().as_secs_f64()
    );

    let virtual_packages = rattler_virtual_packages::VirtualPackage::detect(
        &rattler_virtual_packages::VirtualPackageOverrides::default(),
    )
    .map(|vpkgs| {
        vpkgs
            .iter()
            .map(|vpkg| GenericVirtualPackage::from(vpkg.clone()))
            .collect::<Vec<_>>()
    })
    .into_diagnostic()?;

    let locked_packages = installed
        .iter()
        .map(|r| r.repodata_record.clone())
        .collect();

    let solver_task = SolverTask {
        locked_packages,
        virtual_packages,
        specs: match_specs.clone(),
        ..SolverTask::from_iter(&repo_data)
    };

    let solved = wrap_spinner("solving environment", move || {
        resolvo::Solver.solve(solver_task)
    })
    .into_diagnostic()
    .context("failed to solve environment")?
    .records;

    let required_packages = apply_excludes(solved, excludes);

    run_installer(
        prefix,
        platform,
        &installed,
        &match_specs,
        client,
        required_packages,
    )
    .await
}

pub(crate) fn parse_specs(specs: &[String]) -> miette::Result<Vec<MatchSpec>> {
    specs
        .iter()
        .map(|s| MatchSpec::from_str(s, ParseMatchSpecOptions::default()))
        .collect::<Result<Vec<_>, _>>()
        .into_diagnostic()
        .context("failed to parse package specs")
}

fn make_download_client() -> miette::Result<reqwest_middleware::ClientWithMiddleware> {
    let raw = reqwest::Client::builder()
        .no_gzip()
        .build()
        .expect("failed to create HTTP client");

    Ok(reqwest_middleware::ClientBuilder::new(raw.clone())
        .with_arc(Arc::new(
            AuthenticationMiddleware::from_env_and_defaults().into_diagnostic()?,
        ))
        .with(rattler_networking::OciMiddleware::new(raw))
        .build())
}

async fn run_installer(
    prefix: &Path,
    platform: Platform,
    installed: &[PrefixRecord],
    specs: &[MatchSpec],
    client: reqwest_middleware::ClientWithMiddleware,
    packages: Vec<RepoDataRecord>,
) -> miette::Result<()> {
    let start = Instant::now();
    let result = Installer::new()
        .with_download_client(client)
        .with_target_platform(platform)
        .with_installed_packages(installed.to_vec())
        .with_execute_link_scripts(true)
        .with_requested_specs(specs.to_vec())
        .with_reporter(
            IndicatifReporter::builder()
                .with_multi_progress(multi_progress())
                .finish(),
        )
        .install(prefix, packages)
        .await
        .into_diagnostic()
        .context("failed to install packages")?;

    if result.transaction.operations.is_empty() {
        eprintln!("   {} Already up to date", console::style("✔").green());
    } else {
        eprintln!(
            "   Installed {} packages in {:.1}s",
            result.transaction.operations.len(),
            start.elapsed().as_secs_f64()
        );
    }
    Ok(())
}

pub(crate) fn apply_excludes(
    packages: Vec<RepoDataRecord>,
    excludes: &[String],
) -> Vec<RepoDataRecord> {
    if excludes.is_empty() {
        return packages;
    }
    let (filtered, removed) = filter_excluded_packages(packages, excludes);
    if !removed.is_empty() {
        eprintln!(
            "   Excluded {} packages ({})",
            removed.len(),
            removed.join(", ")
        );
    }
    filtered
}

pub(crate) fn wrap_spinner<T, F: FnOnce() -> T>(msg: impl Into<Cow<'static, str>>, func: F) -> T {
    let pb = multi_progress().add(ProgressBar::new_spinner());
    pb.enable_steady_tick(Duration::from_millis(100));
    pb.set_style(ProgressStyle::with_template("   {spinner:.green} {msg}").unwrap());
    pb.set_message(msg);
    let result = func();
    pb.finish_and_clear();
    result
}

async fn wrap_async_spinner<T, F: IntoFuture<Output = T>>(
    msg: impl Into<Cow<'static, str>>,
    fut: F,
) -> T {
    let pb = multi_progress().add(ProgressBar::new_spinner());
    pb.enable_steady_tick(Duration::from_millis(100));
    pb.set_style(ProgressStyle::with_template("   {spinner:.green} {msg}").unwrap());
    pb.set_message(msg);
    let result = fut.into_future().await;
    pb.finish_and_clear();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exclude::sorted_names;
    use rstest::rstest;

    #[test]
    fn test_parse_specs_valid() {
        let specs = vec![
            "python >=3.12".to_string(),
            "conda >=25.1".to_string(),
            "numpy".to_string(),
        ];
        let result = parse_specs(&specs);
        assert!(result.is_ok(), "valid specs should parse successfully");
        assert_eq!(result.unwrap().len(), 3);
    }

    #[test]
    fn test_parse_specs_empty() {
        let result = parse_specs(&[]);
        assert!(result.is_ok(), "empty specs should parse successfully");
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_parse_specs_invalid() {
        let specs = vec![">=>=not_a_package!!!".to_string()];
        let result = parse_specs(&specs);
        assert!(result.is_err(), "malformed spec should fail to parse");
    }

    #[test]
    fn test_apply_excludes_empty_excludes() {
        let records = crate::exclude::tests::make_test_records();
        let original_count = records.len();
        let filtered = apply_excludes(records, &[]);
        assert_eq!(
            filtered.len(),
            original_count,
            "empty excludes should return all packages"
        );
    }

    #[test]
    fn test_apply_excludes_with_match() {
        let records = crate::exclude::tests::make_test_records();
        let excludes = vec!["a".to_string()];
        let filtered = apply_excludes(records, &excludes);
        let names = sorted_names(&filtered);
        assert!(
            !names.contains(&"a".to_string()),
            "excluded package should be removed"
        );
    }

    #[rstest]
    #[case::empty(vec![], 0)]
    #[case::conda_only(vec!["foo-1.0-h1.conda"], 1)]
    #[case::tar_bz2(vec!["bar-2.0-h2.tar.bz2"], 1)]
    #[case::mixed_with_junk(vec!["a-1-h1.conda", "b-2-h2.tar.bz2", "readme.txt"], 2)]
    fn test_index_payload_dir(#[case] files: Vec<&str>, #[case] expected_count: usize) {
        let tmp = tempfile::TempDir::new().unwrap();
        for name in &files {
            std::fs::write(tmp.path().join(name), b"").unwrap();
        }
        let index = index_payload_dir(tmp.path()).unwrap();
        assert_eq!(index.len(), expected_count);
        for (name, path) in &index {
            assert!(name.ends_with(".conda") || name.ends_with(".tar.bz2"));
            assert!(path.exists());
        }
    }

    fn make_record_with_url(filename: &str) -> RepoDataRecord {
        use rattler_conda_types::{
            PackageName, VersionWithSource,
            package::{CondaArchiveIdentifier, DistArchiveIdentifier},
        };
        use std::str::FromStr;

        let record = rattler_conda_types::PackageRecord::new(
            PackageName::new_unchecked("dummy"),
            VersionWithSource::from_str("1.0").unwrap(),
            "0".to_string(),
        );
        RepoDataRecord {
            package_record: record,
            identifier: DistArchiveIdentifier::from(
                filename.parse::<CondaArchiveIdentifier>().unwrap(),
            ),
            url: format!("https://conda.anaconda.org/conda-forge/linux-64/{filename}")
                .parse()
                .unwrap(),
            channel: Some("conda-forge".to_string()),
        }
    }

    #[rstest]
    #[case::all_found(
        vec!["a-1-h1.conda", "b-2-h2.conda"],
        vec!["a-1-h1.conda", "b-2-h2.conda"],
        0
    )]
    #[case::partial(
        vec!["a-1-h1.conda"],
        vec!["a-1-h1.conda", "b-2-h2.conda"],
        1
    )]
    #[case::none_found(vec![], vec!["a-1-h1.conda"], 1)]
    fn test_match_records_to_payload(
        #[case] payload_files: Vec<&str>,
        #[case] record_filenames: Vec<&str>,
        #[case] expected_missing: usize,
    ) {
        let mut payload_index = HashMap::new();
        for name in &payload_files {
            payload_index.insert(name.to_string(), PathBuf::from(format!("/payload/{name}")));
        }
        let records: Vec<RepoDataRecord> = record_filenames
            .iter()
            .map(|f| make_record_with_url(f))
            .collect();
        let (matched, missing) = match_records_to_payload(&records, &payload_index);
        assert_eq!(
            matched.len(),
            record_filenames.len() - expected_missing,
            "matched count"
        );
        assert_eq!(missing.len(), expected_missing, "missing count");
    }
}
