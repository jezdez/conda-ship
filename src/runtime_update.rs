//! Opt-in updates for stamped runtime executables.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use fs4::fs_std::FileExt;
use miette::{Context, IntoDiagnostic};
use rattler_cache::package_cache::{CacheKey, PackageCache};
use rattler_cache::validation::ValidationMode;
use rattler_conda_types::package::{IndexJson, PackageFile, PathType, PathsJson};
use rattler_conda_types::{
    Channel, ChannelConfig, PackageName, Platform, RepoData, RepoDataRecord, VersionWithSource,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::runtime_data::{RuntimeDataHeader, RuntimeUpdateConfig, UpdateOwnership};
use crate::{config, executable_update, hash, http, policy, runtime_data};

pub(crate) const INTERNAL_UPDATE_ENV: &str = "CONDA_SHIP_INTERNAL_UPDATE";
pub(crate) const INTERNAL_OFFLINE_ENV: &str = "CONDA_SHIP_INTERNAL_UPDATE_OFFLINE";
pub(crate) const INTERNAL_CANDIDATE_ENV: &str = "CONDA_SHIP_INTERNAL_UPDATE_CANDIDATE";
pub(crate) const INTERNAL_OWNERSHIP_ENV: &str = "CONDA_SHIP_INTERNAL_UPDATE_OWNERSHIP";
pub(crate) const INTERNAL_INSTALLATION_ENV: &str = "CONDA_SHIP_INTERNAL_UPDATE_INSTALLATION";
pub(crate) const INTERNAL_EXECUTABLE_ENV: &str = "CONDA_SHIP_INTERNAL_UPDATE_EXECUTABLE";
pub(crate) const INTERNAL_INSTRUCTION_ENV: &str = "CONDA_SHIP_INTERNAL_UPDATE_INSTRUCTION";
pub(crate) const CHECK_ACTION: &str = "v1/check";
pub(crate) const STAGE_ACTION: &str = "v1/stage";
pub(crate) const APPLY_ACTION: &str = "v1/apply";
pub(crate) const RECORD_INSTALLATION_ACTION: &str = "v1/record-installation";
#[cfg(windows)]
pub(crate) const WINDOWS_REPLACE_ACTION: &str = "v1/windows-replace";

#[derive(Clone, Debug)]
struct SelectedPackage {
    record: RepoDataRecord,
    filename: String,
    sha256: String,
    size: u64,
}

#[derive(Debug, Serialize)]
struct CheckResponse {
    available: bool,
    current_version: String,
    current_build_number: u64,
    version: Option<String>,
    build_number: Option<u64>,
    package: Option<String>,
    sha256: Option<String>,
    ownership: UpdateOwnership,
    installation: Option<String>,
    instruction: Option<String>,
}

pub(crate) async fn run_internal_helper(action: &str, prefix: &Path) -> miette::Result<()> {
    let header = &runtime_data::current().header;
    let update = header
        .update
        .as_ref()
        .ok_or_else(|| miette::miette!("this runtime has no executable update configuration"))?;
    if matches!(action, CHECK_ACTION | STAGE_ACTION | APPLY_ACTION)
        && !executable_update::update_lock_is_held(prefix, &header.metadata_file)?
    {
        return Err(miette::miette!(
            "runtime update coordination lock is not held by the caller"
        ));
    }
    if action == CHECK_ACTION {
        let outcome = executable_update::discard_unapproved(prefix, &header.metadata_file)?;
        if outcome != executable_update::ExecutableUpdateOutcome::None {
            return Err(miette::miette!(
                "runtime executable update recovery completed with {outcome:?}, retry the command"
            ));
        }
        initialize_current(prefix)?;
    }
    if action == STAGE_ACTION {
        match executable_update::discard_unapproved(prefix, &header.metadata_file)? {
            executable_update::ExecutableUpdateOutcome::None
            | executable_update::ExecutableUpdateOutcome::DiscardedStaged => {}
            #[cfg(windows)]
            executable_update::ExecutableUpdateOutcome::ReplacementPending => {
                return Err(miette::miette!(
                    "runtime executable update is waiting for running processes to exit"
                ));
            }
            outcome => {
                return Err(miette::miette!(
                    "runtime executable update recovery completed with {outcome:?}, retry the command"
                ));
            }
        }
        initialize_current(prefix)?;
    }
    let metadata = config::read_metadata_for(prefix, &header.metadata_file)?;
    config::validate_metadata_ready_for(
        &metadata,
        &header.runtime_name,
        &header.install_name,
        &header.metadata_file,
    )?;
    let offline = environment_flag(INTERNAL_OFFLINE_ENV);

    match action {
        #[cfg(windows)]
        WINDOWS_REPLACE_ACTION => {
            executable_update::run_windows_worker(prefix, &header.metadata_file)?;
        }
        CHECK_ACTION => {
            let recorded = metadata.update.as_ref().ok_or_else(|| {
                miette::miette!("runtime metadata does not configure executable updates")
            })?;
            let candidate = resolve_candidate(prefix, header, update, offline).await?;
            let response = CheckResponse {
                available: candidate.is_some(),
                current_version: header.runtime_version.clone(),
                current_build_number: update.build_number,
                version: candidate
                    .as_ref()
                    .map(|candidate| candidate.record.package_record.version.to_string()),
                build_number: candidate
                    .as_ref()
                    .map(|candidate| candidate.record.package_record.build_number),
                package: candidate.as_ref().map(|_| update.package.clone()),
                sha256: candidate.as_ref().map(|candidate| candidate.sha256.clone()),
                ownership: recorded.ownership,
                installation: recorded.installation.clone(),
                instruction: recorded.instruction.clone(),
            };
            serde_json::to_writer(std::io::stdout().lock(), &response)
                .into_diagnostic()
                .context("failed to write runtime update check")?;
            std::io::stdout()
                .lock()
                .write_all(b"\n")
                .into_diagnostic()?;
        }
        STAGE_ACTION => {
            if !update.supports_direct_update() {
                return Err(miette::miette!(
                    "this runtime executable does not support direct updates"
                ));
            }
            let recorded = metadata.update.as_ref().ok_or_else(|| {
                miette::miette!("runtime metadata does not configure executable updates")
            })?;
            if recorded.ownership != UpdateOwnership::Direct {
                let instruction = recorded
                    .instruction
                    .as_deref()
                    .map(|instruction| format!(": {instruction}"))
                    .unwrap_or_default();
                return Err(miette::miette!(
                    "this runtime executable is managed externally{instruction}"
                ));
            }
            let selected_sha256 = selected_candidate_digest()?;
            let candidate = require_selected_candidate(
                resolve_candidate(prefix, header, update, offline).await?,
                &selected_sha256,
            )?;
            let extracted = fetch_package(prefix, &candidate, offline).await?;
            let payload = validate_extracted_candidate(&extracted, header, update, &candidate)?;
            executable_update::stage_candidate(
                prefix,
                &header.metadata_file,
                &payload,
                &candidate.record.package_record.version.to_string(),
                candidate.record.package_record.build_number,
            )?;
            write_response(&serde_json::json!({"staged": true}))?;
        }
        APPLY_ACTION => {
            if executable_update::mark_pending_ready(prefix, &header.metadata_file)?
                == executable_update::ExecutableUpdateOutcome::None
            {
                return Err(miette::miette!(
                    "no staged runtime executable update is available to apply"
                ));
            }
            let outcome = executable_update::apply_pending(prefix, &header.metadata_file)?;
            match outcome {
                executable_update::ExecutableUpdateOutcome::Applied
                | executable_update::ExecutableUpdateOutcome::CleanupPending => {
                    write_response(&serde_json::json!({"applied": true}))?;
                }
                #[cfg(windows)]
                executable_update::ExecutableUpdateOutcome::ReplacementPending => {
                    write_response(&serde_json::json!({
                        "applied": false,
                        "replacement_pending": true,
                    }))?;
                }
                executable_update::ExecutableUpdateOutcome::RestoredPrevious => {
                    return Err(miette::miette!(
                        "the selected runtime executable could not be installed and the previous executable was restored"
                    ));
                }
                other => {
                    return Err(miette::miette!(
                        "unexpected runtime executable update outcome: {other:?}"
                    ));
                }
            }
        }
        RECORD_INSTALLATION_ACTION => {
            let ownership = requested_ownership()?;
            let installation = required_environment(INTERNAL_INSTALLATION_ENV)?;
            let executable = std::env::var_os(INTERNAL_EXECUTABLE_ENV)
                .map(PathBuf::from)
                .map_or_else(policy::invocation_path, Ok)?;
            let instruction = optional_environment(INTERNAL_INSTRUCTION_ENV)?;
            executable_update::record_installation(
                prefix,
                &header.metadata_file,
                &executable,
                ownership,
                &installation,
                instruction.as_deref(),
            )?;
            let recorded = config::read_metadata_for(prefix, &header.metadata_file)?
                .update
                .ok_or_else(|| {
                    miette::miette!("runtime metadata does not configure executable updates")
                })?;
            write_response(&serde_json::json!({
                "recorded": true,
                "ownership": recorded.ownership,
                "installation": recorded.installation,
                "executable": recorded.executable,
                "instruction": recorded.instruction,
            }))?;
        }
        other => {
            return Err(miette::miette!(
                "unknown internal runtime update action: {other}"
            ));
        }
    }
    Ok(())
}

fn required_environment(name: &str) -> miette::Result<String> {
    let value = std::env::var(name)
        .into_diagnostic()
        .with_context(|| format!("{name} is required"))?;
    if value.is_empty() {
        return Err(miette::miette!("{name} must not be empty"));
    }
    Ok(value)
}

fn optional_environment(name: &str) -> miette::Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) if value.is_empty() => Err(miette::miette!("{name} must not be empty")),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error)
            .into_diagnostic()
            .with_context(|| format!("{name} is invalid")),
    }
}

fn requested_ownership() -> miette::Result<UpdateOwnership> {
    match required_environment(INTERNAL_OWNERSHIP_ENV)?.as_str() {
        "direct" => Ok(UpdateOwnership::Direct),
        "external" => Ok(UpdateOwnership::External),
        ownership => Err(miette::miette!(
            "invalid runtime installation ownership: {ownership}"
        )),
    }
}

fn write_response(response: &impl Serialize) -> miette::Result<()> {
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, response)
        .into_diagnostic()
        .context("failed to write runtime update response")?;
    output.write_all(b"\n").into_diagnostic()
}

fn selected_candidate_digest() -> miette::Result<String> {
    let digest = std::env::var(INTERNAL_CANDIDATE_ENV)
        .into_diagnostic()
        .context("runtime update candidate was not selected")?;
    validate_sha256_digest(&digest)?;
    Ok(digest)
}

fn require_selected_candidate(
    candidate: Option<SelectedPackage>,
    selected_sha256: &str,
) -> miette::Result<SelectedPackage> {
    let candidate = candidate
        .ok_or_else(|| miette::miette!("the selected runtime update is no longer available"))?;
    if candidate.sha256 != selected_sha256 {
        return Err(miette::miette!(
            "the selected runtime update changed after confirmation"
        ));
    }
    Ok(candidate)
}

fn validate_sha256_digest(value: &str) -> miette::Result<()> {
    if value.len() != 64
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
        || value.bytes().any(|byte| byte.is_ascii_uppercase())
    {
        return Err(miette::miette!(
            "invalid selected runtime update SHA-256 digest"
        ));
    }
    Ok(())
}

pub(crate) fn recover_pending(prefix: &Path) -> miette::Result<bool> {
    let header = &runtime_data::current().header;
    if header.update.is_none() {
        return Ok(false);
    }
    match executable_update::recover_pending(prefix, &header.metadata_file)? {
        executable_update::ExecutableUpdateOutcome::None => Ok(false),
        #[cfg(windows)]
        executable_update::ExecutableUpdateOutcome::ReplacementPending => Err(miette::miette!(
            "runtime executable update is waiting for running processes to exit, retry the command"
        )),
        _ => Ok(true),
    }
}

pub(crate) fn initialize_current(prefix: &Path) -> miette::Result<()> {
    let header = &runtime_data::current().header;
    let Some(update) = header.update.as_ref() else {
        return Ok(());
    };
    executable_update::ensure_update_lock(prefix, &header.metadata_file)?;
    let metadata = config::read_metadata_for(prefix, &header.metadata_file)?;
    let Some(previous) = metadata.update.as_ref() else {
        return reconcile_current(prefix);
    };
    if previous.pending.is_some() {
        return Ok(());
    }
    if previous.artifact_name != header.artifact_name
        || previous.channel != update.channel
        || previous.package != update.package
        || previous.build_number != update.build_number
        || metadata.version != header.runtime_version
    {
        return reconcile_current(prefix);
    }
    Ok(())
}

pub(crate) fn reconcile_current(prefix: &Path) -> miette::Result<()> {
    let header = &runtime_data::current().header;
    let Some(update) = header.update.as_ref() else {
        return Ok(());
    };
    let metadata = config::read_metadata_for(prefix, &header.metadata_file)?;
    let (executable, ownership, installation, instruction) = match metadata.update.as_ref() {
        Some(previous)
            if previous.installation.is_some() && !previous.executable.as_os_str().is_empty() =>
        {
            (
                previous.executable.clone(),
                previous.ownership,
                previous.installation.as_deref(),
                previous.instruction.as_deref(),
            )
        }
        previous => {
            let ownership = previous.map_or(update.ownership, |previous| previous.ownership);
            (
                initial_executable_path(ownership)?,
                ownership,
                None,
                previous
                    .and_then(|previous| previous.instruction.as_deref())
                    .or(update.instruction.as_deref()),
            )
        }
    };
    verify_current_executable(&executable)?;
    let (digest, _) = hash::sha256_file(&executable)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to hash runtime executable at {}",
                executable.display()
            )
        })?;
    executable_update::reconcile_current_executable(
        prefix,
        &header.metadata_file,
        &executable,
        ownership,
        installation,
        &header.artifact_name,
        &update.channel,
        &update.package,
        update.build_number,
        instruction,
        &header.runtime_version,
        &hash::hex(&digest),
    )
    .map(|_| ())
}

fn verify_current_executable(recorded: &Path) -> miette::Result<()> {
    let current = std::env::current_exe()
        .into_diagnostic()
        .context("failed to determine current runtime executable")?;
    let current = std::fs::canonicalize(&current)
        .into_diagnostic()
        .context("failed to resolve current runtime executable")?;
    let recorded_resolved = std::fs::canonicalize(recorded)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to resolve recorded runtime executable at {}",
                policy::path_for_display(recorded)
            )
        })?;
    if current != recorded_resolved {
        return Err(miette::miette!(
            "current runtime executable does not match the recorded stable path at {}",
            policy::path_for_display(recorded)
        ));
    }
    Ok(())
}

fn initial_executable_path(ownership: UpdateOwnership) -> miette::Result<PathBuf> {
    let invoked = policy::invocation_path()?;
    if ownership != UpdateOwnership::Direct {
        return Ok(invoked);
    }
    let metadata = std::fs::symlink_metadata(&invoked)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to inspect runtime executable at {}",
                policy::path_for_display(&invoked)
            )
        })?;
    if !metadata.file_type().is_symlink() {
        return Ok(invoked);
    }
    std::fs::canonicalize(&invoked)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to resolve runtime executable at {}",
                policy::path_for_display(&invoked)
            )
        })
}

async fn resolve_candidate(
    prefix: &Path,
    header: &RuntimeDataHeader,
    update: &RuntimeUpdateConfig,
    offline: bool,
) -> miette::Result<Option<SelectedPackage>> {
    validate_update_config(update)?;
    let platform = Platform::current();
    if !header.platform.is_empty() && header.platform != platform.to_string() {
        return Err(miette::miette!(
            "runtime was built for {}, but is running on {}",
            header.platform,
            platform
        ));
    }
    let channel_config = ChannelConfig::default_with_root_dir(
        std::env::current_dir()
            .into_diagnostic()
            .context("failed to determine current directory")?,
    );
    let channel = Channel::from_str(&update.channel, &channel_config)
        .into_diagnostic()
        .context("failed to parse runtime update channel")?;
    let repodata = load_repodata(prefix, &channel, platform, offline).await?;
    let package_name = PackageName::from_str(&update.package)
        .into_diagnostic()
        .context("failed to parse runtime update package name")?;
    let current_version = VersionWithSource::from_str(&header.runtime_version)
        .into_diagnostic()
        .context("failed to parse current runtime version")?;
    let current = (&current_version, update.build_number);

    let mut candidates = Vec::new();
    for record in repodata.into_repo_data_records(&channel) {
        if record.package_record.name != package_name
            || !record.identifier.to_string().ends_with(".conda")
            || record.package_record.subdir != platform.to_string()
        {
            continue;
        }
        let candidate = (
            &record.package_record.version,
            record.package_record.build_number,
        );
        if candidate <= current {
            continue;
        }
        let size = record
            .package_record
            .size
            .ok_or_else(|| miette::miette!("{} has no size in repodata", record.identifier))?;
        let digest =
            record.package_record.sha256.as_ref().ok_or_else(|| {
                miette::miette!("{} has no SHA-256 in repodata", record.identifier)
            })?;
        let sha256 = hash::hex(digest.as_slice());
        let filename = record.identifier.to_string();
        validate_package_filename(&filename)?;
        candidates.push(SelectedPackage {
            record,
            filename,
            sha256,
            size,
        });
    }
    candidates.sort_by(|left, right| {
        (
            &left.record.package_record.version,
            left.record.package_record.build_number,
        )
            .cmp(&(
                &right.record.package_record.version,
                right.record.package_record.build_number,
            ))
    });
    Ok(candidates.pop())
}

async fn load_repodata(
    prefix: &Path,
    channel: &Channel,
    platform: Platform,
    offline: bool,
) -> miette::Result<RepoData> {
    let url = channel
        .base_url
        .url()
        .join(&format!("{platform}/repodata.json"))
        .into_diagnostic()
        .context("failed to construct runtime update repodata URL")?;
    let cache = repodata_cache_path(prefix, url.as_str())?;
    let bytes = if url.scheme() == "file" {
        let path = url
            .to_file_path()
            .map_err(|()| miette::miette!("invalid file channel URL: {url}"))?;
        std::fs::read(&path).into_diagnostic().with_context(|| {
            format!(
                "failed to read runtime update repodata at {}",
                path.display()
            )
        })?
    } else if offline {
        std::fs::read(&cache).into_diagnostic().with_context(|| {
            format!(
                "offline runtime update requires cached repodata at {}",
                cache.display()
            )
        })?
    } else {
        let response = http::runtime_update_client()?
            .get(url.clone())
            .send()
            .await
            .into_diagnostic()
            .with_context(|| format!("failed to download runtime update repodata from {url}"))?
            .error_for_status()
            .into_diagnostic()
            .with_context(|| format!("runtime update channel returned an error for {url}"))?;
        validate_response_url(&url, response.url())?;
        response
            .bytes()
            .await
            .into_diagnostic()
            .context("failed to read runtime update repodata response")?
            .to_vec()
    };
    let repodata: RepoData = serde_json::from_slice(&bytes)
        .into_diagnostic()
        .with_context(|| format!("failed to parse runtime update repodata from {url}"))?;
    if url.scheme() != "file" && !offline {
        write_cache_file(&cache, &bytes)?;
    }
    Ok(repodata)
}

async fn fetch_package(
    prefix: &Path,
    candidate: &SelectedPackage,
    offline: bool,
) -> miette::Result<PathBuf> {
    fetch_package_into(
        &update_cache_dir(prefix)?.join("packages"),
        candidate,
        offline,
    )
    .await
}

async fn fetch_package_into(
    cache_root: &Path,
    candidate: &SelectedPackage,
    offline: bool,
) -> miette::Result<PathBuf> {
    let expected_sha256 = candidate
        .record
        .package_record
        .sha256
        .ok_or_else(|| miette::miette!("{} has no SHA-256", candidate.filename))?;
    let cache_identity = Sha256::digest(format!("{}:{}", candidate.sha256, candidate.size));
    let cache = PackageCache::new_layered(
        [cache_root.join(hash::hex(cache_identity.as_slice()))],
        false,
        ValidationMode::Full,
    );
    let key =
        CacheKey::from(&candidate.record.package_record).with_url(candidate.record.url.clone());
    let url = candidate.record.url.clone();
    let filename = candidate.filename.clone();
    let expected_sha256_hex = candidate.sha256.clone();
    let expected_size = candidate.size;
    let client = if offline || url.scheme() == "file" {
        None
    } else {
        Some(http::runtime_update_client()?)
    };
    let cached = cache
        .get_or_fetch(
            key,
            move |destination| {
                let client = client.clone();
                let filename = filename.clone();
                let url = url.clone();
                let expected_sha256_hex = expected_sha256_hex.clone();
                async move {
                    let extracted = if url.scheme() == "file" {
                        let path = url.to_file_path().map_err(|()| {
                            std::io::Error::other(format!("invalid file package URL: {url}"))
                        })?;
                        rattler_package_streaming::tokio::fs::extract(&path, &destination)
                            .await
                            .map_err(std::io::Error::other)?
                    } else {
                        let client = client.ok_or_else(|| {
                            std::io::Error::other(format!(
                                "offline runtime update requires cached package {filename}"
                            ))
                        })?;
                        rattler_package_streaming::reqwest::tokio::extract(
                            client,
                            url,
                            &destination,
                            Some(expected_sha256),
                            None,
                        )
                        .await
                        .map_err(std::io::Error::other)?
                    };
                    if extracted.total_size != expected_size {
                        return Err(std::io::Error::other(format!(
                            "runtime update package size mismatch: expected {expected_size}, got {}",
                            extracted.total_size
                        )));
                    }
                    let actual_sha256 = hash::hex(extracted.sha256.as_slice());
                    if actual_sha256 != expected_sha256_hex {
                        return Err(std::io::Error::other(format!(
                            "runtime update package SHA-256 mismatch: expected {expected_sha256_hex}, got {actual_sha256}"
                        )));
                    }
                    Ok(())
                }
            },
            None,
        )
        .await
        .into_diagnostic()
        .with_context(|| format!("failed to cache runtime update package {}", candidate.filename))?;
    Ok(cached.path().to_path_buf())
}

fn validate_response_url(requested: &reqwest::Url, final_url: &reqwest::Url) -> miette::Result<()> {
    if requested.scheme() == "https" && final_url.scheme() != "https" {
        return Err(miette::miette!(
            "runtime update download refused an HTTPS downgrade to {final_url}"
        ));
    }
    Ok(())
}

fn validate_extracted_candidate(
    extracted: &Path,
    current: &RuntimeDataHeader,
    update: &RuntimeUpdateConfig,
    candidate: &SelectedPackage,
) -> miette::Result<PathBuf> {
    let index = IndexJson::from_package_directory(extracted)
        .into_diagnostic()
        .context("runtime update package has invalid index metadata")?;
    let expected_package = PackageName::from_str(&update.package)
        .into_diagnostic()
        .context("failed to parse runtime update package name")?;
    let expected_subdir = Platform::current().to_string();
    let record = &candidate.record.package_record;
    if index.name != expected_package
        || index.name != record.name
        || index.version != record.version
        || index.build != record.build
        || index.build_number != record.build_number
        || index.subdir.as_deref() != Some(expected_subdir.as_str())
        || record.subdir != expected_subdir
        || !index.depends.is_empty()
        || !record.depends.is_empty()
    {
        return Err(miette::miette!(
            "runtime update package identity does not match the installed runtime"
        ));
    }

    let paths = PathsJson::from_package_directory(extracted)
        .into_diagnostic()
        .context("runtime update package has invalid paths metadata")?;
    let [payload_entry] = paths.paths.as_slice() else {
        return Err(miette::miette!(
            "runtime update package must contain exactly one payload"
        ));
    };
    if paths.paths_version != 1
        || payload_entry.path_type != PathType::HardLink
        || payload_entry.prefix_placeholder.is_some()
        || payload_entry.relative_path.as_os_str().is_empty()
        || payload_entry
            .relative_path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(miette::miette!(
            "runtime update package has invalid payload metadata"
        ));
    }
    let payload_sha256 = payload_entry
        .sha256
        .as_ref()
        .ok_or_else(|| miette::miette!("runtime update payload has no SHA-256"))?;
    let payload_size = payload_entry
        .size_in_bytes
        .ok_or_else(|| miette::miette!("runtime update payload has no size"))?;
    let payload_sha256 = hash::hex(payload_sha256.as_slice());
    let payload = extracted.join(&payload_entry.relative_path);
    verify_file(
        &payload,
        &payload_sha256,
        payload_size,
        "runtime update executable",
    )?;
    let stamped = runtime_data::read_from_path(&payload)
        .into_diagnostic()
        .context("failed to inspect runtime update executable")?
        .ok_or_else(|| miette::miette!("runtime update executable is not stamped"))?;
    if stamped.header.runtime_name != current.runtime_name
        || stamped.header.artifact_name != current.artifact_name
        || stamped.header.runtime_version != index.version.to_string()
        || !matches!(
            stamped.header.artifact_layout.as_str(),
            "online" | "embedded"
        )
        || stamped.header.platform != expected_subdir
    {
        return Err(miette::miette!(
            "runtime update executable stamp does not match its package"
        ));
    }
    let stamped_update =
        stamped.header.update.as_ref().ok_or_else(|| {
            miette::miette!("runtime update executable has no update configuration")
        })?;
    validate_update_config(stamped_update)?;
    if stamped_update.channel != update.channel
        || stamped_update.package != update.package
        || !stamped_update.supports_direct_update()
        || stamped_update.build_number != index.build_number
    {
        return Err(miette::miette!(
            "runtime update executable configuration does not match its package"
        ));
    }
    Ok(payload)
}

fn validate_update_config(update: &RuntimeUpdateConfig) -> miette::Result<()> {
    let url = reqwest::Url::parse(&update.channel)
        .into_diagnostic()
        .context("runtime update channel must be an absolute URL")?;
    if !matches!(url.scheme(), "https" | "file") {
        return Err(miette::miette!(
            "runtime update channel must use https:// or file://"
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(miette::miette!(
            "runtime update channel must not contain credentials"
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(miette::miette!(
            "runtime update channel must not contain a query or fragment"
        ));
    }
    PackageName::from_str(&update.package)
        .into_diagnostic()
        .context("failed to parse runtime update package name")?;
    match update.ownership {
        UpdateOwnership::Direct if update.instruction.is_some() => Err(miette::miette!(
            "direct runtime updates must not configure an external update instruction"
        )),
        UpdateOwnership::External
            if update
                .instruction
                .as_deref()
                .is_some_and(|instruction| instruction.trim().is_empty()) =>
        {
            Err(miette::miette!(
                "external runtime update instructions must not be empty"
            ))
        }
        _ => Ok(()),
    }
}

fn update_cache_dir(prefix: &Path) -> miette::Result<PathBuf> {
    if let Some(cache) = dirs::cache_dir() {
        return Ok(cache.join("conda-ship").join("updates"));
    }
    let parent = prefix.parent().ok_or_else(|| {
        miette::miette!(
            "runtime prefix has no parent directory: {}",
            policy::path_for_display(prefix)
        )
    })?;
    Ok(parent.join(".conda-ship-update-cache"))
}

fn repodata_cache_path(prefix: &Path, source: &str) -> miette::Result<PathBuf> {
    let digest = Sha256::digest(source.as_bytes());
    Ok(update_cache_dir(prefix)?
        .join("repodata")
        .join(format!("{}.json", hash::hex(&digest))))
}

fn write_cache_file(path: &Path, bytes: &[u8]) -> miette::Result<()> {
    let _lock = lock_cache_file(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| miette::miette!("cache path has no parent: {}", path.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .into_diagnostic()
        .context("failed to create temporary runtime update cache file")?;
    temporary
        .write_all(bytes)
        .into_diagnostic()
        .context("failed to write runtime update cache")?;
    temporary
        .as_file()
        .sync_all()
        .into_diagnostic()
        .context("failed to sync runtime update cache")?;
    if path.exists() {
        std::fs::remove_file(path)
            .into_diagnostic()
            .with_context(|| {
                format!(
                    "failed to replace runtime update cache at {}",
                    path.display()
                )
            })?;
    }
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to commit runtime update cache at {}",
                path.display()
            )
        })
        .map(|_| ())
}

fn lock_cache_file(path: &Path) -> miette::Result<File> {
    let parent = path
        .parent()
        .ok_or_else(|| miette::miette!("cache path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to create runtime update cache at {}",
                parent.display()
            )
        })?;
    let lock_path = path.with_extension("lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to open runtime update cache lock at {}",
                lock_path.display()
            )
        })?;
    lock.lock_exclusive().into_diagnostic().with_context(|| {
        format!(
            "failed to acquire runtime update cache lock at {}",
            lock_path.display()
        )
    })?;
    Ok(lock)
}

fn verify_file(path: &Path, digest: &str, size: u64, label: &str) -> miette::Result<()> {
    let (actual_digest, actual_size) = hash::sha256_file(path)
        .into_diagnostic()
        .with_context(|| format!("failed to read {label} at {}", path.display()))?;
    if actual_size != size {
        return Err(miette::miette!(
            "{label} size mismatch: expected {size}, got {actual_size}"
        ));
    }
    let actual_digest = hash::hex(&actual_digest);
    if actual_digest != digest {
        return Err(miette::miette!(
            "{label} SHA-256 mismatch: expected {digest}, got {actual_digest}"
        ));
    }
    Ok(())
}

fn validate_package_filename(filename: &str) -> miette::Result<()> {
    if filename.is_empty()
        || !filename.ends_with(".conda")
        || filename.contains('/')
        || filename.contains('\\')
        || filename.chars().any(char::is_control)
    {
        return Err(miette::miette!(
            "invalid runtime update package filename: {filename:?}"
        ));
    }
    Ok(())
}

fn environment_flag(name: &str) -> bool {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .is_some_and(|value| {
            let value = value.to_string_lossy();
            value != "0" && !value.eq_ignore_ascii_case("false")
        })
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write as _};
    use std::net::TcpListener;

    use rattler_conda_types::compression_level::CompressionLevel;

    use super::*;

    const TEST_PACKAGE: &str = "demo-runtime";

    fn test_header(version: &str) -> RuntimeDataHeader {
        let mut header = RuntimeDataHeader::for_name("demo");
        header.runtime_version = version.to_string();
        header.platform = Platform::current().to_string();
        header
    }

    fn test_update(channel: impl Into<String>, build_number: u64) -> RuntimeUpdateConfig {
        RuntimeUpdateConfig {
            channel: channel.into(),
            package: TEST_PACKAGE.to_string(),
            build_number,
            ownership: UpdateOwnership::Direct,
            instruction: None,
        }
    }

    fn test_digest(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    fn test_record(
        version: &str,
        build_number: u64,
        sha256: Option<&str>,
        size: Option<u64>,
    ) -> (String, serde_json::Value) {
        let filename = format!("{TEST_PACKAGE}-{version}-{build_number}.conda");
        let mut record = serde_json::json!({
            "name": TEST_PACKAGE,
            "version": version,
            "build": build_number.to_string(),
            "build_number": build_number,
            "depends": [],
            "subdir": Platform::current().to_string(),
        });
        if let Some(sha256) = sha256 {
            record["sha256"] = serde_json::Value::String(sha256.to_string());
        }
        if let Some(size) = size {
            record["size"] = serde_json::Value::Number(size.into());
        }
        (filename, record)
    }

    fn test_repodata(records: Vec<(String, serde_json::Value)>) -> Vec<u8> {
        let packages = serde_json::Map::from_iter(records);
        serde_json::to_vec(&serde_json::json!({
            "info": {"subdir": Platform::current().to_string()},
            "packages": {},
            "packages.conda": packages,
            "removed": [],
            "repodata_version": 1,
        }))
        .unwrap()
    }

    fn write_file_channel(root: &Path, repodata: &[u8]) {
        let subdir = root.join(Platform::current().to_string());
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::write(subdir.join("repodata.json"), repodata).unwrap();
    }

    fn selected_package(
        version: &str,
        build_number: u64,
        sha256: &str,
        size: u64,
    ) -> SelectedPackage {
        selected_package_for_channel(
            "https://example.test/runtime",
            version,
            build_number,
            sha256,
            size,
        )
    }

    fn selected_package_for_channel(
        channel_url: &str,
        version: &str,
        build_number: u64,
        sha256: &str,
        size: u64,
    ) -> SelectedPackage {
        let channel_config = ChannelConfig::default_with_root_dir(std::env::current_dir().unwrap());
        let channel = Channel::from_str(channel_url, &channel_config).unwrap();
        let repodata: RepoData = serde_json::from_slice(&test_repodata(vec![test_record(
            version,
            build_number,
            Some(sha256),
            Some(size),
        )]))
        .unwrap();
        let record = repodata
            .into_repo_data_records(&channel)
            .into_iter()
            .next()
            .unwrap();
        SelectedPackage {
            filename: record.identifier.to_string(),
            record,
            sha256: sha256.to_string(),
            size,
        }
    }

    fn write_update_archive(
        root: &Path,
        current: &RuntimeDataHeader,
        update: &RuntimeUpdateConfig,
        version: &str,
        build_number: u64,
    ) -> (PathBuf, PathBuf, String, u64) {
        let candidate = selected_package(version, build_number, &test_digest(0), 0);
        let contents = root.join("contents");
        let payload = write_extracted_candidate(&contents, current, update, &candidate, None);
        let payload_relative = payload.strip_prefix(&contents).unwrap().to_path_buf();
        let archive = root.join(&candidate.filename);
        let paths = [
            contents.join("info/index.json"),
            contents.join("info/paths.json"),
            payload,
        ];
        rattler_package_streaming::write::write_conda_package(
            std::fs::File::create(&archive).unwrap(),
            &contents,
            &paths,
            CompressionLevel::Default,
            Some(1),
            archive.file_stem().unwrap().to_str().unwrap(),
            None,
            None,
        )
        .unwrap();
        let (digest, size) = hash::sha256_file(&archive).unwrap();
        (archive, payload_relative, hash::hex(&digest), size)
    }

    fn serve_package(bytes: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let started = std::time::Instant::now();
            let mut last_request = None;
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                            .unwrap();
                        let mut request = [0_u8; 8192];
                        let _ = stream.read(&mut request).unwrap();
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            bytes.len()
                        )
                        .unwrap();
                        stream.write_all(&bytes).unwrap();
                        last_request = Some(std::time::Instant::now());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if last_request.is_some_and(|last| {
                            last.elapsed() >= std::time::Duration::from_millis(250)
                        }) {
                            break;
                        }
                        assert!(
                            started.elapsed() < std::time::Duration::from_secs(10),
                            "package server received no request"
                        );
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(error) => panic!("package server failed: {error}"),
                }
            }
        });
        (format!("http://{address}/runtime"), server)
    }

    fn write_extracted_candidate(
        root: &Path,
        current: &RuntimeDataHeader,
        update: &RuntimeUpdateConfig,
        candidate: &SelectedPackage,
        next_update: Option<&RuntimeUpdateConfig>,
    ) -> PathBuf {
        let payload_name = if cfg!(windows) {
            "payload/demo.exe"
        } else {
            "payload/demo"
        };
        let payload = root.join(payload_name);
        std::fs::create_dir_all(payload.parent().unwrap()).unwrap();
        std::fs::create_dir_all(root.join("info")).unwrap();
        std::fs::write(&payload, b"new runtime").unwrap();

        let mut stamped = current.clone();
        stamped.runtime_version = candidate.record.package_record.version.to_string();
        let mut stamped_update = next_update.unwrap_or(update).clone();
        stamped_update.build_number = candidate.record.package_record.build_number;
        stamped.update = Some(stamped_update);
        runtime_data::append_to_binary(&payload, &stamped, None).unwrap();
        let (payload_digest, payload_size) = hash::sha256_file(&payload).unwrap();
        let record = &candidate.record.package_record;
        std::fs::write(
            root.join("info/index.json"),
            serde_json::to_vec(&serde_json::json!({
                "name": record.name.as_normalized(),
                "version": record.version.to_string(),
                "build": record.build.as_str(),
                "build_number": record.build_number,
                "depends": [],
                "subdir": record.subdir.as_str(),
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("info/paths.json"),
            serde_json::to_vec(&serde_json::json!({
                "paths": [{
                    "_path": payload_name,
                    "path_type": "hardlink",
                    "sha256": hash::hex(&payload_digest),
                    "size_in_bytes": payload_size,
                }],
                "paths_version": 1,
            }))
            .unwrap(),
        )
        .unwrap();
        payload
    }

    struct CacheFileGuard(PathBuf);

    impl Drop for CacheFileGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_file(self.0.with_extension("lock"));
        }
    }

    #[test]
    fn package_filenames_are_flat_conda_archives() {
        assert!(validate_package_filename("demo-2.0-0.conda").is_ok());
        assert!(validate_package_filename("../demo-2.0-0.conda").is_err());
        assert!(validate_package_filename("demo-2.0-0.tar.bz2").is_err());
    }

    #[test]
    fn update_downloads_reject_https_downgrades() {
        let requested = reqwest::Url::parse("https://packages.example.test/runtime").unwrap();
        let secure = reqwest::Url::parse("https://cdn.example.test/runtime").unwrap();
        let insecure = reqwest::Url::parse("http://cdn.example.test/runtime").unwrap();

        validate_response_url(&requested, &secure).unwrap();
        let error = validate_response_url(&requested, &insecure)
            .unwrap_err()
            .to_string();
        assert!(error.contains("HTTPS downgrade"), "{error}");
    }

    #[test]
    fn external_update_instruction_is_optional_but_cannot_be_empty() {
        let mut update = test_update("https://packages.example.test/runtime", 0);
        update.ownership = UpdateOwnership::External;

        validate_update_config(&update).unwrap();

        update.instruction = Some("   ".to_string());
        let error = validate_update_config(&update).unwrap_err().to_string();
        assert!(error.contains("must not be empty"), "{error}");

        update.instruction = Some("brew upgrade conda".to_string());
        validate_update_config(&update).unwrap();
    }

    #[test]
    fn direct_update_rejects_an_external_instruction() {
        let mut update = test_update("https://packages.example.test/runtime", 0);
        update.instruction = Some("brew upgrade conda".to_string());

        let error = validate_update_config(&update).unwrap_err().to_string();

        assert!(error.contains("must not configure"), "{error}");
    }

    #[test]
    fn stage_requires_the_candidate_selected_before_confirmation() {
        let selected = selected_package("2.0.0", 1, &test_digest(0x22), 42);
        let accepted = require_selected_candidate(Some(selected.clone()), &test_digest(0x22))
            .expect("matching candidate should be accepted");
        assert_eq!(accepted.sha256, test_digest(0x22));

        let changed = require_selected_candidate(Some(selected), &test_digest(0x33))
            .unwrap_err()
            .to_string();
        assert!(changed.contains("changed after confirmation"), "{changed}");

        let missing = require_selected_candidate(None, &test_digest(0x22))
            .unwrap_err()
            .to_string();
        assert!(missing.contains("no longer available"), "{missing}");
    }

    #[tokio::test]
    async fn file_channel_package_is_cached_as_a_validated_extraction() {
        let temp = tempfile::TempDir::new().unwrap();
        let current = test_header("1.0.0");
        let update = test_update("https://example.test/runtime", 0);
        let (archive, payload, digest, size) =
            write_update_archive(temp.path(), &current, &update, "2.0.0", 1);
        let channel = temp.path().join("channel");
        let subdir = channel.join(Platform::current().to_string());
        std::fs::create_dir_all(&subdir).unwrap();
        let package = subdir.join(archive.file_name().unwrap());
        std::fs::copy(&archive, &package).unwrap();
        let channel_url = reqwest::Url::from_directory_path(&channel)
            .unwrap()
            .to_string();
        let candidate = selected_package_for_channel(&channel_url, "2.0.0", 1, &digest, size);
        let cache = temp.path().join("cache");

        let extracted = fetch_package_into(&cache, &candidate, true).await.unwrap();
        assert!(extracted.join(&payload).is_file());

        std::fs::remove_file(package).unwrap();
        let cached = fetch_package_into(&cache, &candidate, true).await.unwrap();
        assert_eq!(cached, extracted);

        std::fs::write(cached.join(payload), b"tampered").unwrap();
        let error = fetch_package_into(&cache, &candidate, true)
            .await
            .unwrap_err();
        assert!(format!("{error:?}").contains("failed to cache runtime update package"));
    }

    #[tokio::test]
    async fn file_channel_selects_newest_candidate_while_offline() {
        let temp = tempfile::TempDir::new().unwrap();
        let channel = temp.path().join("channel");
        let first_digest = test_digest(0x11);
        let selected_digest = test_digest(0x22);
        write_file_channel(
            &channel,
            &test_repodata(vec![
                test_record("1.1.0", 0, Some(&first_digest), Some(11)),
                test_record("2.0.0", 0, Some(&first_digest), Some(22)),
                test_record("2.0.0", 1, Some(&selected_digest), Some(33)),
            ]),
        );
        let header = test_header("1.0.0");
        let channel_url = reqwest::Url::from_directory_path(&channel)
            .unwrap()
            .to_string();
        let update = test_update(channel_url, 0);

        let selected = resolve_candidate(temp.path(), &header, &update, true)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(selected.record.package_record.version.to_string(), "2.0.0");
        assert_eq!(selected.record.package_record.build_number, 1);
        assert_eq!(selected.filename, "demo-runtime-2.0.0-1.conda");
        assert_eq!(selected.sha256, selected_digest);
        assert_eq!(selected.size, 33);
    }

    #[tokio::test]
    async fn file_channel_accepts_a_standard_nonnumeric_build_string() {
        let temp = tempfile::TempDir::new().unwrap();
        let channel = temp.path().join("channel");
        let digest = test_digest(0x23);
        let (_, mut record) = test_record("2.0.0", 1, Some(&digest), Some(33));
        record["build"] = serde_json::Value::String("release_1".to_string());
        write_file_channel(
            &channel,
            &test_repodata(vec![(
                "demo-runtime-2.0.0-release_1.conda".to_string(),
                record,
            )]),
        );
        let header = test_header("1.0.0");
        let channel_url = reqwest::Url::from_directory_path(&channel)
            .unwrap()
            .to_string();
        let update = test_update(channel_url, 0);

        let selected = resolve_candidate(temp.path(), &header, &update, true)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(selected.record.package_record.build.as_str(), "release_1");
        assert_eq!(selected.record.package_record.build_number, 1);
    }

    #[tokio::test]
    async fn file_channel_returns_none_when_runtime_is_current() {
        let temp = tempfile::TempDir::new().unwrap();
        let channel = temp.path().join("channel");
        let digest = test_digest(0x33);
        write_file_channel(
            &channel,
            &test_repodata(vec![
                test_record("1.9.0", 9, Some(&digest), Some(11)),
                test_record("2.0.0", 4, Some(&digest), Some(22)),
            ]),
        );
        let header = test_header("2.0.0");
        let channel_url = reqwest::Url::from_directory_path(&channel)
            .unwrap()
            .to_string();
        let update = test_update(channel_url, 4);

        let selected = resolve_candidate(temp.path(), &header, &update, true)
            .await
            .unwrap();

        assert!(selected.is_none());
    }

    #[tokio::test]
    async fn candidate_requires_sha256_and_size_in_repodata() {
        let temp = tempfile::TempDir::new().unwrap();
        let channel = temp.path().join("channel");
        let header = test_header("1.0.0");
        let channel_url = reqwest::Url::from_directory_path(&channel)
            .unwrap()
            .to_string();
        let update = test_update(channel_url, 0);

        write_file_channel(
            &channel,
            &test_repodata(vec![test_record("2.0.0", 0, None, Some(12))]),
        );
        let missing_sha = resolve_candidate(temp.path(), &header, &update, true)
            .await
            .unwrap_err();
        assert!(
            missing_sha
                .to_string()
                .contains("has no SHA-256 in repodata")
        );

        let digest = test_digest(0x44);
        write_file_channel(
            &channel,
            &test_repodata(vec![test_record("2.0.0", 0, Some(&digest), None)]),
        );
        let missing_size = resolve_candidate(temp.path(), &header, &update, true)
            .await
            .unwrap_err();
        assert!(missing_size.to_string().contains("has no size in repodata"));
    }

    #[tokio::test]
    async fn remote_channel_uses_cached_repodata_while_offline() {
        let temp = tempfile::TempDir::new().unwrap();
        let unique = temp.path().file_name().unwrap().to_string_lossy();
        let channel_url = format!("https://example.invalid/conda-ship-test-{unique}");
        let header = test_header("1.0.0");
        let update = test_update(&channel_url, 0);
        let channel_config = ChannelConfig::default_with_root_dir(std::env::current_dir().unwrap());
        let channel = Channel::from_str(&channel_url, &channel_config).unwrap();
        let repodata_url = channel
            .base_url
            .url()
            .join(&format!("{}/repodata.json", Platform::current()))
            .unwrap();
        let cache = repodata_cache_path(temp.path(), repodata_url.as_str()).unwrap();
        let _guard = CacheFileGuard(cache.clone());
        let _ = std::fs::remove_file(&cache);

        let missing = resolve_candidate(temp.path(), &header, &update, true)
            .await
            .unwrap_err();
        assert!(
            missing
                .to_string()
                .contains("offline runtime update requires cached repodata")
        );

        let digest = test_digest(0x55);
        write_cache_file(
            &cache,
            &test_repodata(vec![test_record("2.0.0", 0, Some(&digest), Some(42))]),
        )
        .unwrap();

        let selected = resolve_candidate(temp.path(), &header, &update, true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(selected.filename, "demo-runtime-2.0.0-0.conda");
        assert_eq!(selected.sha256, digest);
        assert_eq!(selected.size, 42);
    }

    #[tokio::test]
    async fn remote_package_cache_supports_offline_hits_and_misses() {
        let temp = tempfile::TempDir::new().unwrap();
        let current = test_header("1.0.0");
        let update = test_update("https://example.test/runtime", 0);
        let (archive, _, digest, size) =
            write_update_archive(temp.path(), &current, &update, "2.0.0", 0);
        let (channel_url, server) = serve_package(std::fs::read(archive).unwrap());
        let candidate = selected_package_for_channel(&channel_url, "2.0.0", 0, &digest, size);
        let cache = temp.path().join("cache");

        let online = fetch_package_into(&cache, &candidate, false).await.unwrap();
        server.join().unwrap();
        let offline = fetch_package_into(&cache, &candidate, true).await.unwrap();

        assert_eq!(offline, online);

        let missing = selected_package("3.0.0", 0, &test_digest(0x77), 42);
        let error = fetch_package_into(&cache, &missing, true)
            .await
            .unwrap_err();
        let rendered = format!("{error:?}");
        assert!(
            rendered.contains("offline runtime update requires cached package")
                && rendered
                    .contains("failed to cache runtime update package demo-runtime-3.0.0-0.conda"),
            "{rendered}"
        );
    }

    #[tokio::test]
    async fn package_cache_rejects_archive_hash_and_size_mismatches() {
        let temp = tempfile::TempDir::new().unwrap();
        let current = test_header("1.0.0");
        let update = test_update("https://example.test/runtime", 0);
        let (archive, _, digest, size) =
            write_update_archive(temp.path(), &current, &update, "2.0.0", 0);
        let channel = temp.path().join("channel");
        let subdir = channel.join(Platform::current().to_string());
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::copy(&archive, subdir.join(archive.file_name().unwrap())).unwrap();
        let channel_url = reqwest::Url::from_directory_path(channel)
            .unwrap()
            .to_string();

        let wrong_hash =
            selected_package_for_channel(&channel_url, "2.0.0", 0, &test_digest(0x88), size);
        let hash_error = fetch_package_into(&temp.path().join("hash-cache"), &wrong_hash, true)
            .await
            .unwrap_err();
        assert!(format!("{hash_error:?}").contains("SHA-256 mismatch"));

        let wrong_size = selected_package_for_channel(&channel_url, "2.0.0", 0, &digest, size + 1);
        let size_error = fetch_package_into(&temp.path().join("size-cache"), &wrong_size, true)
            .await
            .unwrap_err();
        assert!(format!("{size_error:?}").contains("size mismatch"));

        let valid = selected_package_for_channel(&channel_url, "2.0.0", 0, &digest, size);
        fetch_package_into(&temp.path().join("valid-cache"), &valid, true)
            .await
            .unwrap();
    }

    #[test]
    fn extracted_candidate_matches_standard_metadata_and_stamp() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut current = test_header("1.0.0");
        current.artifact_layout = "online".to_string();
        let update = test_update("https://example.test/runtime", 0);
        let candidate = selected_package("2.0.0", 1, &test_digest(0x66), 42);
        let payload = write_extracted_candidate(temp.path(), &current, &update, &candidate, None);

        let validated =
            validate_extracted_candidate(temp.path(), &current, &update, &candidate).unwrap();

        assert_eq!(validated, payload);
    }

    #[test]
    fn extracted_candidate_cannot_rotate_its_next_update_source() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut current = test_header("1.0.0");
        current.artifact_layout = "online".to_string();
        let update = test_update("https://old.example.test/runtime", 0);
        let mut next_update = test_update("https://new.example.test/runtime", 0);
        next_update.package = "renamed-runtime".to_string();
        let candidate = selected_package("2.0.0", 1, &test_digest(0x67), 42);
        write_extracted_candidate(
            temp.path(),
            &current,
            &update,
            &candidate,
            Some(&next_update),
        );

        let error = validate_extracted_candidate(temp.path(), &current, &update, &candidate)
            .unwrap_err()
            .to_string();

        assert!(error.contains("configuration does not match"), "{error}");
    }

    #[test]
    fn extracted_candidate_rejects_payload_digest_mismatch() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut current = test_header("1.0.0");
        current.artifact_layout = "online".to_string();
        let update = test_update("https://example.test/runtime", 0);
        let candidate = selected_package("2.0.0", 1, &test_digest(0x77), 42);
        let payload = write_extracted_candidate(temp.path(), &current, &update, &candidate, None);
        std::fs::write(payload, b"tampered runtime").unwrap();

        let error = validate_extracted_candidate(temp.path(), &current, &update, &candidate)
            .unwrap_err()
            .to_string();

        assert!(error.contains("size mismatch") || error.contains("SHA-256 mismatch"));
    }
}
