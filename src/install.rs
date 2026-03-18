//! Package installation — lockfile fast-path and live-solve fallback.

use std::{
    borrow::Cow,
    collections::HashMap,
    env,
    future::IntoFuture,
    path::Path,
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

fn multi_progress() -> MultiProgress {
    GLOBAL_MP.clone()
}

/// Install packages from a pre-solved lockfile (fast path, no solve needed).
pub async fn from_lockfile(
    prefix: &Path,
    lock_content: &str,
    excludes: &[String],
) -> miette::Result<()> {
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

    let required_packages = apply_excludes(records, excludes);

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

fn wrap_spinner<T, F: FnOnce() -> T>(msg: impl Into<Cow<'static, str>>, func: F) -> T {
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
}
