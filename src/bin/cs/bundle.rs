use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use miette::{Context, IntoDiagnostic};
use rattler_conda_types::Platform;
use rattler_lock::{CondaPackageData, LockFile};
use sha2::Digest;

use super::artifact::{validate_bundle_package_hashes, validate_package_archive_name};
use super::tls;

pub(crate) fn gen_bundle_from_lock(
    lock_file: &LockFile,
    runtime_lock_path: &Path,
    platform: Platform,
    bundle_path: &Path,
) -> miette::Result<PathBuf> {
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
    validate_bundle_package_hashes(&packages)?;

    eprintln!("downloading {} packages for {platform}...", packages.len());

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .into_diagnostic()
        .context("failed to create tokio runtime")?;

    rt.block_on(download_and_bundle(&packages, bundle_path))
        .map_err(|err| miette::miette!("failed to download bundle: {err}"))?;
    Ok(bundle_path.to_path_buf())
}

async fn download_and_bundle(
    packages: &[&CondaPackageData],
    bundle_path: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use futures::stream::{self, StreamExt};

    tls::install_default_provider();
    let client = reqwest::Client::builder()
        .user_agent(crate::http::USER_AGENT)
        .no_gzip()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .build()?;

    let bundle_parent = bundle_path
        .parent()
        .ok_or_else(|| format!("bundle path has no parent: {}", bundle_path.display()))?;
    let bundle_dir = bundle_parent.join("bundle");
    std::fs::create_dir_all(bundle_parent)?;
    if let Ok(metadata) = std::fs::symlink_metadata(&bundle_dir) {
        if metadata.file_type().is_symlink() || metadata.is_file() {
            std::fs::remove_file(&bundle_dir)?;
        } else {
            std::fs::remove_dir_all(&bundle_dir)?;
        }
    }
    std::fs::create_dir_all(&bundle_dir)?;

    let start = std::time::Instant::now();

    let download_tasks = packages.iter().map(|pkg| {
        let client = client.clone();
        let bundle_dir = bundle_dir.clone();
        async move {
            let url = pkg
                .location()
                .as_url()
                .ok_or_else(|| format!("package location is not a URL: {:?}", pkg.location()))?;
            let archive_name = url
                .path_segments()
                .and_then(|mut s| s.next_back())
                .ok_or_else(|| format!("package URL has no archive name: {url}"))?;
            validate_package_archive_name(archive_name)
                .map_err(|e| format!("invalid package archive name from {url}: {e}"))?;

            let dest = bundle_dir.join(archive_name);
            let expected = pkg
                .record()
                .ok_or_else(|| {
                    format!("{archive_name} is missing its package record in the runtime lock")
                })?
                .sha256
                .as_ref()
                .ok_or_else(|| format!("{archive_name} has no SHA256 in the runtime lock"))?;

            if dest.exists() {
                let (actual, _) = crate::hash::sha256_file(&dest)?;
                if actual.as_slice() == expected.as_slice() {
                    return Ok::<(), Box<dyn std::error::Error + Send + Sync>>(());
                }
                eprintln!("SHA256 mismatch for {archive_name}, re-downloading");
                std::fs::remove_file(&dest)?;
            }

            let mut response = client
                .get(url.clone())
                .send()
                .await
                .map_err(|e| format!("failed to fetch {archive_name}: {e}"))?;

            let status = response.status();
            if !status.is_success() {
                return Err(format!("HTTP {status} fetching {archive_name}").into());
            }

            let tmp_dest = dest.with_file_name(format!(".{archive_name}.download"));
            let mut out = std::fs::File::create(&tmp_dest)?;
            let mut hasher = sha2::Sha256::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|e| format!("failed to read {archive_name}: {e}"))?
            {
                hasher.update(&chunk);
                out.write_all(&chunk)?;
            }
            out.flush()?;
            drop(out);

            let actual = crate::hash::digest_to_array(hasher.finalize());
            if actual.as_slice() != expected.as_slice() {
                let _ = std::fs::remove_file(&tmp_dest);
                return Err(format!("SHA256 mismatch for {archive_name}").into());
            }

            std::fs::rename(&tmp_dest, &dest)?;
            Ok(())
        }
    });

    let results: Vec<_> = stream::iter(download_tasks)
        .buffer_unordered(8)
        .collect()
        .await;

    for result in results {
        result?;
    }

    eprintln!(
        "downloaded {} packages in {:.1}s, bundling...",
        packages.len(),
        start.elapsed().as_secs_f64()
    );

    let bundle_start = std::time::Instant::now();
    let out_file = std::fs::File::create(bundle_path)?;
    let zstd_encoder = zstd::Encoder::new(out_file, 1)?;
    let mut tar_builder = tar::Builder::new(zstd_encoder);

    for entry in std::fs::read_dir(&bundle_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file()
            && let Some(name) = path.file_name()
        {
            tar_builder.append_path_with_name(&path, name)?;
        }
    }

    let zstd_encoder = tar_builder.into_inner()?;
    zstd_encoder.finish()?;

    let bundle_size = std::fs::metadata(bundle_path)?.len();
    eprintln!(
        "bundle.tar.zst = {:.1} MB ({} packages, bundled in {:.1}s)",
        bundle_size as f64 / 1_048_576.0,
        packages.len(),
        bundle_start.elapsed().as_secs_f64()
    );

    Ok(())
}
