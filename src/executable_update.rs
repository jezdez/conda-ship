//! Crash-recoverable replacement of a stamped runtime executable.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::time::{Duration, Instant};
#[cfg(windows)]
use std::{ffi::OsString, os::windows::ffi::OsStrExt};

use fs4::fs_std::FileExt as _;
use miette::{Context, IntoDiagnostic};
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::CloseHandle,
    System::Threading::{
        CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, PROCESS_INFORMATION,
        STARTUPINFOW,
    },
};

use crate::bootstrap_lock::BootstrapLock;
use crate::config::{
    self, ExecutableUpdateMetadata, PendingExecutablePhase, PendingExecutableUpdate, PrefixMetadata,
};
use crate::{hash, policy, runtime_data};
use runtime_data::UpdateOwnership;

#[cfg(windows)]
const WINDOWS_WORKER_RETRY_INTERVAL: Duration = Duration::from_millis(50);
#[cfg(windows)]
const WINDOWS_WORKER_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ExecutableUpdateOutcome {
    None,
    Staged,
    Ready,
    Applied,
    #[cfg(windows)]
    ReplacementPending,
    CleanupPending,
    DiscardedStaged,
    RestoredPrevious,
    Reconciled,
}

/// Copy a verified update payload next to the installed executable and record it.
///
/// The source is the finalized executable extracted from the selected conda
/// package. Package resolution and archive verification happen before this
/// function is called.
pub(crate) fn stage_candidate(
    prefix: &Path,
    metadata_file: &str,
    source: &Path,
    version: &str,
    build_number: u64,
) -> miette::Result<ExecutableUpdateOutcome> {
    let _lock = BootstrapLock::acquire(prefix)?;
    let mut meta = read_owned_metadata(prefix, metadata_file)?;
    let update = direct_update(&meta)?.clone();
    if update.pending.is_some() {
        return Err(miette::miette!(
            "a runtime executable update is already pending"
        ));
    }
    require_regular_file(source, "update candidate")?;
    require_regular_file(&update.executable, "installed runtime executable")?;
    verify_stamped_file(
        &meta,
        &update,
        &update.executable,
        &meta.version,
        &update.sha256,
    )?;

    let new_sha256 = sha256_hex(source)?;
    verify_stamped_file(&meta, &update, source, version, &new_sha256)?;
    if new_sha256 == update.sha256 {
        return Err(miette::miette!(
            "selected runtime executable is identical to the installed executable"
        ));
    }

    let candidate = candidate_path(&update.executable, &new_sha256)?;
    let backup = backup_path(&update.executable, &update.sha256)?;
    if regular_file_present(&backup, "previous executable backup")? {
        return Err(miette::miette!(
            "refusing to stage an update while a previous executable backup exists at {}",
            policy::path_for_display(&backup)
        ));
    }

    let created = if regular_file_present(&candidate, "staged runtime executable")? {
        match verify_stamped_file(&meta, &update, &candidate, version, &new_sha256) {
            Ok(()) => false,
            Err(_) => {
                std::fs::remove_file(&candidate)
                    .into_diagnostic()
                    .context("failed to remove an incomplete runtime executable candidate")?;
                stage_file(source, &candidate, &update.executable)?
            }
        }
    } else {
        stage_file(source, &candidate, &update.executable)?
    };
    let staged_result = verify_stamped_file(&meta, &update, &candidate, version, &new_sha256);
    if let Err(error) = staged_result {
        if created {
            let _ = std::fs::remove_file(&candidate);
        }
        return Err(error);
    }

    meta.update
        .as_mut()
        .expect("validated update metadata should be present")
        .pending = Some(PendingExecutableUpdate {
        phase: PendingExecutablePhase::Staged,
        version: version.to_string(),
        build_number,
        executable_sha256: new_sha256,
    });
    if let Err(error) = config::persist_metadata_for(prefix, metadata_file, &meta) {
        if created {
            let _ = std::fs::remove_file(candidate);
        }
        return Err(error);
    }
    Ok(ExecutableUpdateOutcome::Staged)
}

/// Mark a staged candidate as approved by a successful inner conda command.
pub(crate) fn mark_pending_ready(
    prefix: &Path,
    metadata_file: &str,
) -> miette::Result<ExecutableUpdateOutcome> {
    let _lock = BootstrapLock::acquire(prefix)?;
    let mut meta = read_owned_metadata(prefix, metadata_file)?;
    let update = direct_update(&meta)?.clone();
    let Some(pending) = update.pending.as_ref() else {
        return Ok(ExecutableUpdateOutcome::None);
    };
    validate_pending_paths(&update, pending)?;
    match pending.phase {
        PendingExecutablePhase::Staged => {
            let files = classify_files(&meta, &update, pending)?;
            if files != FileState::staged() {
                return invalid_recovery_state(pending.phase, files);
            }
            meta.update
                .as_mut()
                .and_then(|update| update.pending.as_mut())
                .expect("validated pending update should be present")
                .phase = PendingExecutablePhase::Ready;
            config::persist_metadata_for(prefix, metadata_file, &meta)?;
            Ok(ExecutableUpdateOutcome::Ready)
        }
        PendingExecutablePhase::Ready => Ok(ExecutableUpdateOutcome::Ready),
        phase => Err(miette::miette!(
            "cannot approve a runtime executable update in phase {phase:?}"
        )),
    }
}

/// Apply an approved candidate to the recorded stable executable path.
pub(crate) fn apply_pending(
    prefix: &Path,
    metadata_file: &str,
) -> miette::Result<ExecutableUpdateOutcome> {
    let _lock = BootstrapLock::acquire(prefix)?;
    let mut meta = read_owned_metadata(prefix, metadata_file)?;
    let phase = direct_update(&meta)?
        .pending
        .as_ref()
        .map(|pending| pending.phase);
    match phase {
        None => Ok(ExecutableUpdateOutcome::None),
        Some(PendingExecutablePhase::Ready | PendingExecutablePhase::Replacing) => {
            #[cfg(not(windows))]
            {
                apply_or_recover_locked(prefix, metadata_file, &mut meta)
            }
            #[cfg(windows)]
            {
                recover_or_schedule_windows_locked(prefix, metadata_file, &mut meta)
            }
        }
        Some(PendingExecutablePhase::Cleanup) => {
            finish_cleanup_locked(prefix, metadata_file, &mut meta)
        }
        Some(PendingExecutablePhase::Staged) => Err(miette::miette!(
            "runtime executable update has not been approved"
        )),
    }
}

/// Recover an interrupted replacement or retry deferred cleanup.
pub(crate) fn recover_pending(
    prefix: &Path,
    metadata_file: &str,
) -> miette::Result<ExecutableUpdateOutcome> {
    recover_pending_with(prefix, metadata_file, false)
}

pub(crate) fn discard_unapproved(
    prefix: &Path,
    metadata_file: &str,
) -> miette::Result<ExecutableUpdateOutcome> {
    recover_pending_with(prefix, metadata_file, true)
}

fn recover_pending_with(
    prefix: &Path,
    metadata_file: &str,
    discard_staged: bool,
) -> miette::Result<ExecutableUpdateOutcome> {
    let _lock = BootstrapLock::acquire(prefix)?;
    let mut meta = config::read_metadata_for(prefix, metadata_file)?;
    validate_current_metadata(&meta, metadata_file)?;
    let Some(phase) = meta
        .update
        .as_ref()
        .and_then(|update| update.pending.as_ref())
        .map(|pending| pending.phase)
    else {
        return Ok(ExecutableUpdateOutcome::None);
    };
    match phase {
        PendingExecutablePhase::Staged => {
            let update = direct_update(&meta)?;
            let pending = update
                .pending
                .as_ref()
                .expect("pending phase came from this update");
            validate_pending_paths(update, pending)?;
            let files = classify_files(&meta, update, pending)?;
            let candidate = candidate_path(&update.executable, &pending.executable_sha256)?;
            match files {
                state if state == FileState::staged() => {
                    if !discard_staged && update_lock_is_held(prefix, metadata_file)? {
                        return Ok(ExecutableUpdateOutcome::Staged);
                    }
                    std::fs::remove_file(&candidate)
                        .into_diagnostic()
                        .with_context(|| {
                            format!(
                                "failed to discard unapproved runtime executable at {}",
                                policy::path_for_display(&candidate)
                            )
                        })?;
                }
                FileState {
                    target: FileVersion::Old,
                    candidate: FileVersion::Missing,
                    backup: FileVersion::Missing,
                } => {}
                state => return invalid_recovery_state(phase, state),
            }
            clear_pending(&mut meta);
            config::persist_metadata_for(prefix, metadata_file, &meta)?;
            Ok(ExecutableUpdateOutcome::DiscardedStaged)
        }
        PendingExecutablePhase::Ready | PendingExecutablePhase::Replacing => {
            #[cfg(not(windows))]
            {
                apply_or_recover_locked(prefix, metadata_file, &mut meta)
            }
            #[cfg(windows)]
            {
                recover_or_schedule_windows_locked(prefix, metadata_file, &mut meta)
            }
        }
        PendingExecutablePhase::Cleanup => finish_cleanup_locked(prefix, metadata_file, &mut meta),
    }
}

/// Refresh the installed outer identity after an external package manager
/// replaces the stable executable.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconcile_current_executable(
    prefix: &Path,
    metadata_file: &str,
    executable: &Path,
    ownership: UpdateOwnership,
    artifact_name: &str,
    channel: &str,
    package: &str,
    build_number: u64,
    instruction: Option<&str>,
    runtime_version: &str,
    runtime_digest: &str,
) -> miette::Result<ExecutableUpdateOutcome> {
    let _lock = BootstrapLock::acquire(prefix)?;
    let mut meta = config::read_metadata_for(prefix, metadata_file)?;
    validate_current_metadata(&meta, metadata_file)?;
    validate_sha256(runtime_digest, "runtime executable")?;
    let actual_digest = sha256_hex(executable)?;
    if actual_digest != runtime_digest {
        return Err(miette::miette!(
            "runtime executable digest does not match the recorded digest"
        ));
    }
    if let Some(previous) = meta.update.as_ref()
        && let Some(pending) = previous.pending.as_ref()
    {
        let unchanged = previous.executable == executable
            && previous.ownership == ownership
            && previous.artifact_name == artifact_name
            && previous.channel == channel
            && previous.package == package
            && previous.build_number == build_number
            && previous.instruction.as_deref() == instruction
            && previous.sha256 == runtime_digest
            && meta.version == runtime_version;
        if !matches!(
            pending.phase,
            PendingExecutablePhase::Staged | PendingExecutablePhase::Cleanup
        ) || !unchanged
        {
            return Err(miette::miette!(
                "cannot reconcile a runtime executable while replacement is pending"
            ));
        }
        verify_stamped_file(&meta, previous, executable, runtime_version, runtime_digest)?;
        return Ok(ExecutableUpdateOutcome::None);
    }
    if meta.update.as_ref().is_some_and(|previous| {
        let initialized = !previous.executable.as_os_str().is_empty()
            || !previous.artifact_name.is_empty()
            || !previous.sha256.is_empty();
        initialized
            && previous.ownership == UpdateOwnership::Direct
            && (ownership != UpdateOwnership::Direct
                || previous.executable != executable
                || previous.artifact_name != artifact_name
                || previous.channel != channel
                || previous.package != package
                || previous.build_number != build_number
                || previous.instruction.as_deref() != instruction
                || previous.sha256 != runtime_digest
                || meta.version != runtime_version)
    }) {
        return Err(miette::miette!(
            "directly managed runtime executable changed outside its coordinated update"
        ));
    }
    let replacement = ExecutableUpdateMetadata {
        executable: executable.to_path_buf(),
        ownership,
        artifact_name: artifact_name.to_string(),
        channel: channel.to_string(),
        package: package.to_string(),
        build_number,
        instruction: instruction.map(str::to_string),
        sha256: runtime_digest.to_string(),
        pending: None,
    };
    verify_stamped_file(
        &meta,
        &replacement,
        executable,
        runtime_version,
        runtime_digest,
    )?;
    if meta.update.as_ref() == Some(&replacement) && meta.version == runtime_version {
        return Ok(ExecutableUpdateOutcome::None);
    }
    meta.version = runtime_version.to_string();
    meta.update = Some(replacement);
    config::persist_metadata_for(prefix, metadata_file, &meta)?;
    Ok(ExecutableUpdateOutcome::Reconciled)
}

fn read_owned_metadata(prefix: &Path, metadata_file: &str) -> miette::Result<PrefixMetadata> {
    let meta = config::read_metadata_for(prefix, metadata_file)?;
    validate_current_metadata(&meta, metadata_file)?;
    if meta.update.is_none() {
        return Err(miette::miette!(
            "runtime metadata does not configure executable updates"
        ));
    }
    Ok(meta)
}

fn validate_current_metadata(meta: &PrefixMetadata, metadata_file: &str) -> miette::Result<()> {
    if runtime_data::current().stamped {
        config::validate_metadata_ready_for(
            meta,
            policy::display_name(),
            policy::install_name(),
            metadata_file,
        )
    } else {
        config::validate_metadata_ready_for(
            meta,
            &meta.display_name,
            &meta.install_name,
            metadata_file,
        )
    }
}

fn direct_update(meta: &PrefixMetadata) -> miette::Result<&ExecutableUpdateMetadata> {
    let update = meta
        .update
        .as_ref()
        .ok_or_else(|| miette::miette!("runtime executable updates are not configured"))?;
    if update.ownership != UpdateOwnership::Direct {
        return Err(miette::miette!(
            "runtime executable is managed by an external package manager"
        ));
    }
    if !update.executable.is_absolute() {
        return Err(miette::miette!(
            "recorded runtime executable path is not absolute"
        ));
    }
    if update.artifact_name.is_empty() || update.channel.is_empty() || update.package.is_empty() {
        return Err(miette::miette!(
            "runtime executable update metadata is incomplete"
        ));
    }
    validate_sha256(&update.sha256, "installed runtime executable")?;
    Ok(update)
}

#[cfg(not(windows))]
fn apply_or_recover_locked(
    prefix: &Path,
    metadata_file: &str,
    meta: &mut PrefixMetadata,
) -> miette::Result<ExecutableUpdateOutcome> {
    let update = direct_update(meta)?.clone();
    let pending = update
        .pending
        .as_ref()
        .ok_or_else(|| miette::miette!("no runtime executable update is pending"))?
        .clone();
    validate_pending_paths(&update, &pending)?;
    let files = classify_files(meta, &update, &pending)?;

    match (pending.phase, files) {
        (PendingExecutablePhase::Ready, state) if state == FileState::staged() => {
            set_phase(meta, PendingExecutablePhase::Replacing);
            config::persist_metadata_for(prefix, metadata_file, meta)?;
            replace_candidate(prefix, metadata_file, meta)
        }
        (PendingExecutablePhase::Replacing, state) if state == FileState::staged() => {
            replace_candidate(prefix, metadata_file, meta)
        }
        (
            PendingExecutablePhase::Replacing,
            FileState {
                target: FileVersion::New,
                candidate: FileVersion::Missing,
                backup: FileVersion::Missing,
            },
        ) => {
            commit_new_version(meta);
            clear_pending(meta);
            config::persist_metadata_for(prefix, metadata_file, meta)?;
            Ok(ExecutableUpdateOutcome::Applied)
        }
        (
            PendingExecutablePhase::Ready | PendingExecutablePhase::Replacing,
            FileState {
                target: FileVersion::Old,
                candidate: FileVersion::Missing,
                backup: FileVersion::Missing,
            },
        ) => {
            clear_pending(meta);
            config::persist_metadata_for(prefix, metadata_file, meta)?;
            Ok(ExecutableUpdateOutcome::RestoredPrevious)
        }
        (phase, state) => invalid_recovery_state(phase, state),
    }
}

#[cfg(not(windows))]
fn replace_candidate(
    prefix: &Path,
    metadata_file: &str,
    meta: &mut PrefixMetadata,
) -> miette::Result<ExecutableUpdateOutcome> {
    let update = direct_update(meta)?.clone();
    let pending = update
        .pending
        .as_ref()
        .expect("replacement requires pending metadata")
        .clone();

    let candidate = candidate_path(&update.executable, &pending.executable_sha256)?;
    match std::fs::rename(&candidate, &update.executable) {
        Ok(()) => {
            commit_new_version(meta);
            clear_pending(meta);
            config::persist_metadata_for(prefix, metadata_file, meta)?;
            Ok(ExecutableUpdateOutcome::Applied)
        }
        Err(atomic_error) => Err(atomic_error).into_diagnostic().with_context(|| {
            format!(
                "failed to replace runtime executable at {}",
                policy::path_for_display(&update.executable)
            )
        }),
    }
}

#[cfg(windows)]
fn recover_or_schedule_windows_locked(
    prefix: &Path,
    metadata_file: &str,
    meta: &mut PrefixMetadata,
) -> miette::Result<ExecutableUpdateOutcome> {
    let update = direct_update(meta)?.clone();
    let pending = update
        .pending
        .as_ref()
        .ok_or_else(|| miette::miette!("no runtime executable update is pending"))?
        .clone();
    validate_pending_paths(&update, &pending)?;
    let files = classify_files(meta, &update, &pending)?;
    let backup = pending_backup_path(&update)?;

    match (pending.phase, files) {
        (
            PendingExecutablePhase::Ready | PendingExecutablePhase::Replacing,
            FileState {
                target: FileVersion::Old,
                candidate: FileVersion::New,
                backup: FileVersion::Missing | FileVersion::Old,
            },
        ) => prepare_and_spawn_windows_worker_locked(prefix, metadata_file, meta),
        (
            PendingExecutablePhase::Ready | PendingExecutablePhase::Replacing,
            FileState {
                target: FileVersion::Old,
                candidate: FileVersion::Missing,
                backup: FileVersion::Missing | FileVersion::Old,
            },
        ) => abandon_missing_candidate(prefix, metadata_file, meta, files.backup),
        (
            PendingExecutablePhase::Replacing,
            FileState {
                target: FileVersion::Missing,
                candidate: FileVersion::New,
                backup: FileVersion::Old,
            },
        ) => {
            std::fs::rename(&backup, &update.executable)
                .into_diagnostic()
                .context("failed to restore the stable runtime executable path")?;
            prepare_and_spawn_windows_worker_locked(prefix, metadata_file, meta)
        }
        (
            PendingExecutablePhase::Replacing,
            FileState {
                target: FileVersion::New,
                candidate: FileVersion::Missing,
                backup: FileVersion::Old,
            },
        ) => {
            set_phase(meta, PendingExecutablePhase::Cleanup);
            config::persist_metadata_for(prefix, metadata_file, meta)?;
            finish_cleanup_locked(prefix, metadata_file, meta)
        }
        (
            PendingExecutablePhase::Replacing,
            FileState {
                target: FileVersion::New,
                candidate: FileVersion::Missing,
                backup: FileVersion::Missing,
            },
        ) => {
            commit_new_version(meta);
            clear_pending(meta);
            config::persist_metadata_for(prefix, metadata_file, meta)?;
            Ok(ExecutableUpdateOutcome::Applied)
        }
        (
            PendingExecutablePhase::Replacing,
            FileState {
                target: FileVersion::Missing,
                candidate: FileVersion::Missing,
                backup: FileVersion::Old,
            },
        ) => {
            std::fs::rename(&backup, &update.executable)
                .into_diagnostic()
                .context("failed to restore the previous runtime executable")?;
            clear_pending(meta);
            config::persist_metadata_for(prefix, metadata_file, meta)?;
            Ok(ExecutableUpdateOutcome::RestoredPrevious)
        }
        (phase, state) => invalid_recovery_state(phase, state),
    }
}

#[cfg(windows)]
fn prepare_and_spawn_windows_worker_locked(
    prefix: &Path,
    metadata_file: &str,
    meta: &mut PrefixMetadata,
) -> miette::Result<ExecutableUpdateOutcome> {
    let worker = prepare_windows_worker_locked(prefix, metadata_file, meta)?;
    spawn_windows_worker(prefix, &worker)?;
    Ok(ExecutableUpdateOutcome::ReplacementPending)
}

#[cfg(windows)]
fn prepare_windows_worker_locked(
    prefix: &Path,
    metadata_file: &str,
    meta: &mut PrefixMetadata,
) -> miette::Result<PathBuf> {
    let update = direct_update(meta)?.clone();
    stage_windows_worker_backup(meta, &update)?;
    set_phase(meta, PendingExecutablePhase::Replacing);
    config::persist_metadata_for(prefix, metadata_file, meta)?;
    pending_backup_path(&update)
}

#[cfg(windows)]
fn stage_windows_worker_backup(
    meta: &PrefixMetadata,
    update: &ExecutableUpdateMetadata,
) -> miette::Result<()> {
    let backup = pending_backup_path(update)?;
    let temporary = backup.with_extension("tmp.exe");
    if regular_file_present(&backup, "runtime update worker")? {
        verify_stamped_file(meta, update, &backup, &meta.version, &update.sha256)?;
        remove_regular_file_if_present(&temporary, "temporary runtime update worker")?;
        return Ok(());
    }

    if regular_file_present(&temporary, "temporary runtime update worker")?
        && verify_stamped_file(meta, update, &temporary, &meta.version, &update.sha256).is_err()
    {
        std::fs::remove_file(&temporary)
            .into_diagnostic()
            .context("failed to remove an incomplete runtime update worker")?;
    }
    stage_file(&update.executable, &temporary, &update.executable)?;
    verify_stamped_file(meta, update, &temporary, &meta.version, &update.sha256)?;
    std::fs::rename(&temporary, &backup)
        .into_diagnostic()
        .context("failed to commit the runtime update worker")?;
    verify_stamped_file(meta, update, &backup, &meta.version, &update.sha256)
}

#[cfg(windows)]
fn remove_regular_file_if_present(path: &Path, description: &str) -> miette::Result<()> {
    if !regular_file_present(path, description)? {
        return Ok(());
    }
    std::fs::remove_file(path)
        .into_diagnostic()
        .with_context(|| format!("failed to remove {description} at {}", path.display()))
}

#[cfg(windows)]
fn spawn_windows_worker(prefix: &Path, worker: &Path) -> miette::Result<()> {
    require_regular_file(worker, "runtime update worker")?;
    let environment = windows_process_environment(
        &[
            (
                crate::runtime_update::INTERNAL_UPDATE_ENV,
                OsString::from(crate::runtime_update::WINDOWS_REPLACE_ACTION),
            ),
            (policy::PREFIX_ENV_VAR, prefix.as_os_str().to_os_string()),
        ],
        &[crate::runtime_update::INTERNAL_CANDIDATE_ENV],
    )?;
    let mut command_line = windows_application_command_line(worker)?;
    spawn_windows_process(worker, &mut command_line, &environment).with_context(|| {
        format!(
            "failed to start runtime update worker at {}",
            policy::path_for_display(worker)
        )
    })
}

#[cfg(windows)]
fn windows_process_environment(
    set: &[(&str, OsString)],
    remove: &[&str],
) -> miette::Result<Vec<u16>> {
    let mut variables: Vec<_> = std::env::vars_os()
        .filter(|(key, _)| {
            !remove
                .iter()
                .chain(set.iter().map(|(name, _)| name))
                .any(|name| key.to_string_lossy().eq_ignore_ascii_case(name))
        })
        .collect();
    variables.extend(
        set.iter()
            .map(|(name, value)| (OsString::from(name), value.clone())),
    );
    variables.sort_by_cached_key(|(key, _)| key.to_string_lossy().to_uppercase());

    let mut block = Vec::new();
    for (key, value) in &variables {
        let key: Vec<_> = key.encode_wide().collect();
        let value: Vec<_> = value.encode_wide().collect();
        if key.is_empty() || key.iter().chain(&value).any(|code_unit| *code_unit == 0) {
            return Err(miette::miette!(
                "runtime update worker environment contains an invalid variable"
            ));
        }
        block.extend(key);
        block.push('=' as u16);
        block.extend(value);
        block.push(0);
    }
    if variables.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

#[cfg(windows)]
fn windows_application_command_line(application: &Path) -> miette::Result<Vec<u16>> {
    let application: Vec<_> = application.as_os_str().encode_wide().collect();
    if application.iter().any(|code_unit| *code_unit == 0) {
        return Err(miette::miette!(
            "runtime update worker path contains an invalid NUL character"
        ));
    }
    let mut command_line = Vec::with_capacity(application.len() + 3);
    command_line.push('"' as u16);
    command_line.extend(application);
    command_line.push('"' as u16);
    command_line.push(0);
    Ok(command_line)
}

#[cfg(windows)]
fn spawn_windows_process(
    application: &Path,
    command_line: &mut [u16],
    environment: &[u16],
) -> miette::Result<()> {
    let mut application: Vec<_> = application.as_os_str().encode_wide().collect();
    if application.iter().any(|code_unit| *code_unit == 0) {
        return Err(miette::miette!(
            "runtime update worker path contains an invalid NUL character"
        ));
    }
    application.push(0);

    let mut startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    // No handles may cross this boundary. In particular, inheriting the
    // helper's captured output pipes would keep its caller blocked until the
    // worker exits while the worker waits for that caller's executable to
    // close.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            std::ptr::null(),
            &raw mut startup,
            &raw mut process,
        )
    };
    if created == 0 {
        return Err(std::io::Error::last_os_error()).into_diagnostic();
    }
    unsafe {
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn run_windows_worker(prefix: &Path, metadata_file: &str) -> miette::Result<()> {
    let deadline = Instant::now() + WINDOWS_WORKER_TIMEOUT;
    loop {
        match run_windows_worker_once(prefix, metadata_file)? {
            WindowsWorkerAttempt::Complete => return Ok(()),
            WindowsWorkerAttempt::Retry(_) if Instant::now() < deadline => {
                std::thread::sleep(WINDOWS_WORKER_RETRY_INTERVAL);
            }
            WindowsWorkerAttempt::Retry(error) => {
                return Err(error).into_diagnostic().with_context(
                    || "runtime update worker timed out waiting for the stable executable to close",
                );
            }
        }
    }
}

#[cfg(windows)]
enum WindowsWorkerAttempt {
    Complete,
    Retry(std::io::Error),
}

#[cfg(windows)]
fn run_windows_worker_once(
    prefix: &Path,
    metadata_file: &str,
) -> miette::Result<WindowsWorkerAttempt> {
    let _lock = BootstrapLock::acquire(prefix)?;
    let mut meta = read_owned_metadata(prefix, metadata_file)?;
    let update = direct_update(&meta)?.clone();
    let Some(pending) = update.pending.as_ref().cloned() else {
        return Ok(WindowsWorkerAttempt::Complete);
    };
    validate_pending_paths(&update, &pending)?;
    let candidate = pending_candidate_path(&update, &pending)?;
    let backup = pending_backup_path(&update)?;
    validate_windows_worker_path(&backup)?;
    let files = classify_files(&meta, &update, &pending)?;

    match (pending.phase, files) {
        (
            PendingExecutablePhase::Replacing,
            FileState {
                target: FileVersion::Old,
                candidate: FileVersion::New,
                backup: FileVersion::Old,
            },
        ) => {
            if let Err(error) = std::fs::rename(&candidate, &update.executable) {
                return Ok(WindowsWorkerAttempt::Retry(error));
            }
            if classify_target(&meta, &update, &pending)? != FileVersion::New {
                return Err(miette::miette!(
                    "runtime update worker did not install the selected executable"
                ));
            }
            set_phase(&mut meta, PendingExecutablePhase::Cleanup);
            config::persist_metadata_for(prefix, metadata_file, &meta)?;
            Ok(WindowsWorkerAttempt::Complete)
        }
        (
            PendingExecutablePhase::Replacing | PendingExecutablePhase::Cleanup,
            FileState {
                target: FileVersion::New,
                candidate: FileVersion::Missing,
                backup: FileVersion::Old,
            },
        ) => {
            set_phase(&mut meta, PendingExecutablePhase::Cleanup);
            config::persist_metadata_for(prefix, metadata_file, &meta)?;
            Ok(WindowsWorkerAttempt::Complete)
        }
        (phase, state) => invalid_recovery_state(phase, state),
    }
}

#[cfg(windows)]
fn validate_windows_worker_path(expected: &Path) -> miette::Result<()> {
    let current = std::env::current_exe()
        .into_diagnostic()
        .context("failed to locate the runtime update worker")?
        .canonicalize()
        .into_diagnostic()
        .context("failed to resolve the runtime update worker")?;
    let expected = expected
        .canonicalize()
        .into_diagnostic()
        .context("failed to resolve the recorded runtime update worker")?;
    if current != expected {
        return Err(miette::miette!(
            "runtime replacement can only run from the recorded update worker"
        ));
    }
    Ok(())
}

fn finish_cleanup_locked(
    prefix: &Path,
    metadata_file: &str,
    meta: &mut PrefixMetadata,
) -> miette::Result<ExecutableUpdateOutcome> {
    let update = direct_update(meta)?.clone();
    let pending = update
        .pending
        .as_ref()
        .ok_or_else(|| miette::miette!("no runtime executable cleanup is pending"))?
        .clone();
    validate_pending_paths(&update, &pending)?;
    let files = classify_files(meta, &update, &pending)?;
    match files {
        FileState {
            target: FileVersion::New,
            candidate: FileVersion::Missing,
            backup: FileVersion::Old,
        } => {
            let backup = pending_backup_path(&update)?;
            match std::fs::remove_file(&backup) {
                Ok(()) => {
                    commit_new_version(meta);
                    clear_pending(meta);
                    config::persist_metadata_for(prefix, metadata_file, meta)?;
                    Ok(ExecutableUpdateOutcome::Applied)
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    commit_new_version(meta);
                    clear_pending(meta);
                    config::persist_metadata_for(prefix, metadata_file, meta)?;
                    Ok(ExecutableUpdateOutcome::Applied)
                }
                Err(_) => Ok(ExecutableUpdateOutcome::CleanupPending),
            }
        }
        FileState {
            target: FileVersion::New,
            candidate: FileVersion::Missing,
            backup: FileVersion::Missing,
        } => {
            commit_new_version(meta);
            clear_pending(meta);
            config::persist_metadata_for(prefix, metadata_file, meta)?;
            Ok(ExecutableUpdateOutcome::Applied)
        }
        state => invalid_recovery_state(pending.phase, state),
    }
}

#[cfg(windows)]
fn abandon_missing_candidate(
    prefix: &Path,
    metadata_file: &str,
    meta: &mut PrefixMetadata,
    backup: FileVersion,
) -> miette::Result<ExecutableUpdateOutcome> {
    if backup == FileVersion::Old {
        let update = direct_update(meta)?;
        let backup_path = pending_backup_path(update)?;
        match std::fs::remove_file(&backup_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Ok(ExecutableUpdateOutcome::CleanupPending),
        }
    }
    clear_pending(meta);
    config::persist_metadata_for(prefix, metadata_file, meta)?;
    Ok(ExecutableUpdateOutcome::RestoredPrevious)
}

fn commit_new_version(meta: &mut PrefixMetadata) {
    let update = meta
        .update
        .as_mut()
        .expect("pending executable update requires update metadata");
    let pending = update
        .pending
        .as_ref()
        .expect("pending executable update should be present");
    let version = pending.version.clone();
    update.build_number = pending.build_number;
    update.sha256 = pending.executable_sha256.clone();
    meta.version = version;
}

fn set_phase(meta: &mut PrefixMetadata, phase: PendingExecutablePhase) {
    meta.update
        .as_mut()
        .and_then(|update| update.pending.as_mut())
        .expect("pending executable update should be present")
        .phase = phase;
}

fn clear_pending(meta: &mut PrefixMetadata) {
    meta.update
        .as_mut()
        .expect("pending executable update requires update metadata")
        .pending = None;
}

fn stage_file(source: &Path, candidate: &Path, target: &Path) -> miette::Result<bool> {
    if source == candidate {
        return Err(miette::miette!(
            "update candidate source is already the staging path"
        ));
    }
    if regular_file_present(candidate, "staged runtime executable")? {
        return Ok(false);
    }
    let temporary = candidate.with_extension("new.tmp");
    if regular_file_present(&temporary, "incomplete runtime executable candidate")? {
        std::fs::remove_file(&temporary)
            .into_diagnostic()
            .context("failed to remove an incomplete runtime executable candidate")?;
    }
    let permissions = std::fs::metadata(target)
        .into_diagnostic()
        .context("failed to inspect installed runtime executable permissions")?
        .permissions();
    let mut input = File::open(source)
        .into_diagnostic()
        .context("failed to open runtime executable update candidate")?;
    let result = (|| -> miette::Result<()> {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .into_diagnostic()
            .context("failed to create adjacent runtime executable candidate")?;
        std::io::copy(&mut input, &mut output)
            .into_diagnostic()
            .context("failed to stage runtime executable candidate")?;
        output
            .flush()
            .into_diagnostic()
            .context("failed to flush runtime executable candidate")?;
        std::fs::set_permissions(&temporary, permissions)
            .into_diagnostic()
            .context("failed to preserve runtime executable permissions")?;
        output
            .sync_all()
            .into_diagnostic()
            .context("failed to sync runtime executable candidate")?;
        drop(output);
        std::fs::rename(&temporary, candidate)
            .into_diagnostic()
            .context("failed to commit runtime executable candidate")?;
        sync_parent(candidate)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map(|()| true)
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> miette::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| miette::miette!("runtime executable candidate has no parent directory"))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .into_diagnostic()
        .context("failed to sync the runtime executable directory")
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> miette::Result<()> {
    Ok(())
}

fn verify_stamped_file(
    meta: &PrefixMetadata,
    update: &ExecutableUpdateMetadata,
    path: &Path,
    version: &str,
    expected_sha256: &str,
) -> miette::Result<()> {
    validate_sha256(expected_sha256, "runtime executable")?;
    let actual_sha256 = sha256_hex(path)?;
    if actual_sha256 != expected_sha256 {
        return Err(miette::miette!(
            "runtime executable digest mismatch at {}",
            policy::path_for_display(path)
        ));
    }
    let data = runtime_data::read_from_path(path)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to read stamped runtime data from {}",
                policy::path_for_display(path)
            )
        })?
        .ok_or_else(|| {
            miette::miette!(
                "runtime executable is not stamped: {}",
                policy::path_for_display(path)
            )
        })?;
    let header = &data.header;
    if header.artifact_name != update.artifact_name
        || header.runtime_name != meta.display_name
        || header.install_name != meta.install_name
        || header.metadata_file != meta.metadata_file
        || header.runtime_version != version
        || header.update.as_ref().is_none_or(|header_update| {
            header_update.channel != update.channel
                || header_update.package != update.package
                || header_update.ownership != update.ownership
        })
        || meta
            .delegate_executable
            .as_deref()
            .is_some_and(|delegate| header.delegate_executable != delegate)
    {
        return Err(miette::miette!(
            "runtime executable stamp does not match the managed runtime at {}",
            policy::path_for_display(path)
        ));
    }
    if let Some(bundle) = data.bundle {
        bundle.verify().into_diagnostic().with_context(|| {
            format!(
                "failed to verify embedded bundle in {}",
                policy::path_for_display(path)
            )
        })?;
    }
    Ok(())
}

fn require_regular_file(path: &Path, description: &str) -> miette::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(miette::miette!(
                "{description} is not a regular file: {}",
                policy::path_for_display(path)
            ))
        }
        Ok(_) => Ok(()),
        Err(error) => Err(error).into_diagnostic().with_context(|| {
            format!(
                "failed to inspect {description} at {}",
                policy::path_for_display(path)
            )
        }),
    }
}

fn regular_file_present(path: &Path, description: &str) -> miette::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(miette::miette!(
                "{description} is not a regular file: {}",
                policy::path_for_display(path)
            ))
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).into_diagnostic().with_context(|| {
            format!(
                "failed to inspect {description} at {}",
                policy::path_for_display(path)
            )
        }),
    }
}

fn sha256_hex(path: &Path) -> miette::Result<String> {
    let (digest, _) = hash::sha256_file(path)
        .into_diagnostic()
        .with_context(|| format!("failed to hash {}", policy::path_for_display(path)))?;
    Ok(hash::hex(&digest))
}

fn validate_sha256(value: &str, description: &str) -> miette::Result<()> {
    if value.len() != 64
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
        || value.bytes().any(|byte| byte.is_ascii_uppercase())
    {
        return Err(miette::miette!(
            "invalid {description} SHA-256 digest: {value:?}"
        ));
    }
    Ok(())
}

fn update_lock_path(prefix: &Path, metadata_file: &str) -> miette::Result<PathBuf> {
    let stem = metadata_file
        .strip_suffix(".json")
        .filter(|stem| stem.starts_with('.') && stem.len() > 1)
        .ok_or_else(|| miette::miette!("invalid runtime metadata filename: {metadata_file}"))?;
    Ok(prefix.join(format!("{stem}.update.lock")))
}

pub(crate) fn ensure_update_lock(prefix: &Path, metadata_file: &str) -> miette::Result<()> {
    let path = update_lock_path(prefix, metadata_file)?;
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(miette::miette!(
                "runtime update coordination lock is not a regular file: {}",
                policy::path_for_display(&path)
            ));
        }
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).into_diagnostic(),
    }
    match OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            file.write_all(&[0])
                .into_diagnostic()
                .context("failed to initialize the runtime update coordination lock")?;
            file.sync_all()
                .into_diagnostic()
                .context("failed to sync the runtime update coordination lock")
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            ensure_update_lock(prefix, metadata_file)
        }
        Err(error) => Err(error).into_diagnostic().with_context(|| {
            format!(
                "failed to create runtime update coordination lock at {}",
                policy::path_for_display(&path)
            )
        }),
    }
}

pub(crate) fn update_lock_is_held(prefix: &Path, metadata_file: &str) -> miette::Result<bool> {
    let path = update_lock_path(prefix, metadata_file)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to open runtime update coordination lock at {}",
                policy::path_for_display(&path)
            )
        })?;
    file.try_lock_exclusive()
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to inspect runtime update coordination lock at {}",
                policy::path_for_display(&path)
            )
        })
        .map(|acquired| !acquired)
}

fn candidate_path(target: &Path, digest: &str) -> miette::Result<PathBuf> {
    adjacent_update_path(target, digest, "new")
}

fn backup_path(target: &Path, digest: &str) -> miette::Result<PathBuf> {
    #[cfg(windows)]
    let suffix = "old.exe";
    #[cfg(not(windows))]
    let suffix = "old";
    adjacent_update_path(target, digest, suffix)
}

fn adjacent_update_path(target: &Path, digest: &str, suffix: &str) -> miette::Result<PathBuf> {
    validate_sha256(digest, "runtime executable")?;
    let parent = target
        .parent()
        .ok_or_else(|| miette::miette!("runtime executable has no parent directory"))?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| miette::miette!("runtime executable name is not valid UTF-8"))?;
    Ok(parent.join(format!(".{name}.conda-ship-{}.{suffix}", &digest[..16])))
}

fn validate_pending_paths(
    update: &ExecutableUpdateMetadata,
    pending: &PendingExecutableUpdate,
) -> miette::Result<()> {
    validate_sha256(&update.sha256, "previous runtime executable")?;
    validate_sha256(&pending.executable_sha256, "new runtime executable")?;
    let _ = candidate_path(&update.executable, &pending.executable_sha256)?;
    let _ = backup_path(&update.executable, &update.sha256)?;
    Ok(())
}

fn pending_candidate_path(
    update: &ExecutableUpdateMetadata,
    pending: &PendingExecutableUpdate,
) -> miette::Result<PathBuf> {
    candidate_path(&update.executable, &pending.executable_sha256)
}

fn pending_backup_path(update: &ExecutableUpdateMetadata) -> miette::Result<PathBuf> {
    backup_path(&update.executable, &update.sha256)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileVersion {
    Missing,
    Old,
    New,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileState {
    target: FileVersion,
    candidate: FileVersion,
    backup: FileVersion,
}

impl FileState {
    const fn staged() -> Self {
        Self {
            target: FileVersion::Old,
            candidate: FileVersion::New,
            backup: FileVersion::Missing,
        }
    }
}

fn classify_files(
    meta: &PrefixMetadata,
    update: &ExecutableUpdateMetadata,
    pending: &PendingExecutableUpdate,
) -> miette::Result<FileState> {
    let target = classify_target(meta, update, pending)?;
    let candidate_path = pending_candidate_path(update, pending)?;
    let candidate = classify_role_file(
        meta,
        update,
        &candidate_path,
        &pending.version,
        &pending.executable_sha256,
        FileVersion::New,
    )?;
    let backup_path = pending_backup_path(update)?;
    let backup = classify_role_file(
        meta,
        update,
        &backup_path,
        &meta.version,
        &update.sha256,
        FileVersion::Old,
    )?;
    Ok(FileState {
        target,
        candidate,
        backup,
    })
}

fn classify_target(
    meta: &PrefixMetadata,
    update: &ExecutableUpdateMetadata,
    pending: &PendingExecutableUpdate,
) -> miette::Result<FileVersion> {
    if !regular_file_present(&update.executable, "installed runtime executable")? {
        return Ok(FileVersion::Missing);
    }
    let digest = sha256_hex(&update.executable)?;
    if digest == update.sha256 {
        verify_stamped_file(
            meta,
            update,
            &update.executable,
            &meta.version,
            &update.sha256,
        )?;
        Ok(FileVersion::Old)
    } else if digest == pending.executable_sha256 {
        verify_stamped_file(
            meta,
            update,
            &update.executable,
            &pending.version,
            &pending.executable_sha256,
        )?;
        Ok(FileVersion::New)
    } else {
        Err(miette::miette!(
            "installed runtime executable matches neither recorded update digest"
        ))
    }
}

fn classify_role_file(
    meta: &PrefixMetadata,
    update: &ExecutableUpdateMetadata,
    path: &Path,
    version: &str,
    digest: &str,
    present: FileVersion,
) -> miette::Result<FileVersion> {
    if !regular_file_present(path, "runtime executable update file")? {
        return Ok(FileVersion::Missing);
    }
    verify_stamped_file(meta, update, path, version, digest)?;
    Ok(present)
}

fn invalid_recovery_state<T>(phase: PendingExecutablePhase, state: FileState) -> miette::Result<T> {
    Err(miette::miette!(
        "cannot recover runtime executable update in phase {phase:?}: target={:?}, candidate={:?}, backup={:?}",
        state.target,
        state.candidate,
        state.backup
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap_state::BootstrapPhase;
    use std::fs::{File, OpenOptions};
    use std::path::{Path, PathBuf};
    #[cfg(windows)]
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    const CHANNEL: &str = "https://example.test/channel";
    const PACKAGE: &str = "demo-runtime";
    const EXTERNAL_INSTRUCTION: &str = "upgrade with the external package manager";

    fn stamped_executable(path: &Path, version: &str) -> String {
        stamped_executable_with(
            path,
            version,
            0,
            UpdateOwnership::Direct,
            CHANNEL,
            PACKAGE,
            None,
        )
    }

    fn stamped_candidate(path: &Path, version: &str, build_number: u64) -> String {
        stamped_executable_with(
            path,
            version,
            build_number,
            UpdateOwnership::Direct,
            CHANNEL,
            PACKAGE,
            None,
        )
    }

    fn stamped_executable_with(
        path: &Path,
        version: &str,
        build_number: u64,
        ownership: UpdateOwnership,
        channel: &str,
        package: &str,
        instruction: Option<&str>,
    ) -> String {
        std::fs::write(path, format!("binary-{version}-{build_number}")).unwrap();
        let mut header = runtime_data::RuntimeDataHeader::for_name("demo");
        header.artifact_name = "demo".to_string();
        header.runtime_version = version.to_string();
        header.update = Some(runtime_data::RuntimeUpdateConfig {
            channel: channel.to_string(),
            package: package.to_string(),
            build_number,
            ownership,
            instruction: instruction.map(str::to_string),
        });
        runtime_data::append_to_binary(path, &header, None).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        sha256_hex(path).unwrap()
    }

    #[cfg(windows)]
    fn stamped_test_executable(path: &Path, version: &str, build_number: u64) -> String {
        std::fs::copy(std::env::current_exe().unwrap(), path).unwrap();
        let mut header = runtime_data::RuntimeDataHeader::for_name("demo");
        header.artifact_name = "demo".to_string();
        header.runtime_version = version.to_string();
        header.update = Some(runtime_data::RuntimeUpdateConfig {
            channel: CHANNEL.to_string(),
            package: PACKAGE.to_string(),
            build_number,
            ownership: UpdateOwnership::Direct,
            instruction: None,
        });
        runtime_data::append_to_binary(path, &header, None).unwrap();
        sha256_hex(path).unwrap()
    }

    fn metadata_for(
        executable: PathBuf,
        digest: String,
        ownership: UpdateOwnership,
        instruction: Option<&str>,
    ) -> PrefixMetadata {
        PrefixMetadata {
            schema_version: 1,
            display_name: "demo".to_string(),
            install_name: "demo".to_string(),
            metadata_file: ".demo.json".to_string(),
            version: "1.0".to_string(),
            delegate_executable: Some("conda".to_string()),
            lock_sha256: None,
            channels: vec![],
            packages: vec![],
            bootstrap_state: BootstrapPhase::Ready,
            update: Some(ExecutableUpdateMetadata {
                executable,
                ownership,
                artifact_name: "demo".to_string(),
                channel: CHANNEL.to_string(),
                package: PACKAGE.to_string(),
                build_number: 0,
                sha256: digest,
                instruction: instruction.map(str::to_string),
                pending: None,
            }),
        }
    }

    fn setup() -> (TempDir, PathBuf, PathBuf, String) {
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("prefix");
        let executable = tmp
            .path()
            .join(format!("demo{}", std::env::consts::EXE_SUFFIX));
        std::fs::create_dir(&prefix).unwrap();
        let digest = stamped_executable(&executable, "1.0");
        let meta = metadata_for(
            executable.clone(),
            digest.clone(),
            UpdateOwnership::Direct,
            None,
        );
        config::persist_metadata_for(&prefix, ".demo.json", &meta).unwrap();
        ensure_update_lock(&prefix, ".demo.json").unwrap();
        (tmp, prefix, executable, digest)
    }

    fn stage_update(
        tmp: &TempDir,
        prefix: &Path,
        version: &str,
        build_number: u64,
    ) -> (PathBuf, String) {
        let source = tmp.path().join(format!(
            "source-{version}-{build_number}{}",
            std::env::consts::EXE_SUFFIX
        ));
        let digest = stamped_candidate(&source, version, build_number);
        assert_eq!(
            stage_candidate(prefix, ".demo.json", &source, version, build_number).unwrap(),
            ExecutableUpdateOutcome::Staged
        );
        (source, digest)
    }

    fn pending_paths(prefix: &Path) -> (PathBuf, PathBuf) {
        let meta = config::read_metadata_for(prefix, ".demo.json").unwrap();
        let update = meta.update.as_ref().unwrap();
        let pending = update.pending.as_ref().unwrap();
        (
            pending_candidate_path(update, pending).unwrap(),
            pending_backup_path(update).unwrap(),
        )
    }

    fn persist_phase(prefix: &Path, phase: PendingExecutablePhase) {
        let mut meta = config::read_metadata_for(prefix, ".demo.json").unwrap();
        set_phase(&mut meta, phase);
        config::persist_metadata_for(prefix, ".demo.json", &meta).unwrap();
    }

    fn hold_update_lock(prefix: &Path) -> File {
        ensure_update_lock(prefix, ".demo.json").unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(update_lock_path(prefix, ".demo.json").unwrap())
            .unwrap();
        lock.lock_exclusive().unwrap();
        lock
    }

    fn assert_installed(
        prefix: &Path,
        executable: &Path,
        version: &str,
        build_number: u64,
        digest: &str,
    ) {
        assert_eq!(sha256_hex(executable).unwrap(), digest);
        let meta = config::read_metadata_for(prefix, ".demo.json").unwrap();
        assert_eq!(meta.version, version);
        let update = meta.update.unwrap();
        assert_eq!(update.build_number, build_number);
        assert_eq!(update.sha256, digest);
        assert!(update.pending.is_none());
    }

    #[test]
    #[cfg(not(windows))]
    fn staged_update_waits_for_approval_before_atomic_apply() {
        let (tmp, prefix, executable, old_digest) = setup();
        let (_source, new_digest) = stage_update(&tmp, &prefix, "2.0", 1);

        assert_eq!(sha256_hex(&executable).unwrap(), old_digest);
        let error = apply_pending(&prefix, ".demo.json")
            .unwrap_err()
            .to_string();
        assert!(error.contains("has not been approved"), "{error}");

        assert_eq!(
            mark_pending_ready(&prefix, ".demo.json").unwrap(),
            ExecutableUpdateOutcome::Ready
        );
        assert_eq!(sha256_hex(&executable).unwrap(), old_digest);
        assert_eq!(
            apply_pending(&prefix, ".demo.json").unwrap(),
            ExecutableUpdateOutcome::Applied
        );
        assert_installed(&prefix, &executable, "2.0", 1, &new_digest);
    }

    #[test]
    fn held_update_lock_preserves_a_staged_candidate_until_release() {
        let (tmp, prefix, executable, old_digest) = setup();
        stage_update(&tmp, &prefix, "2.0", 1);
        let (candidate, _) = pending_paths(&prefix);
        let coordinator_lock = hold_update_lock(&prefix);

        assert_eq!(
            recover_pending(&prefix, ".demo.json").unwrap(),
            ExecutableUpdateOutcome::Staged
        );
        assert!(candidate.is_file());
        assert_eq!(sha256_hex(&executable).unwrap(), old_digest);

        drop(coordinator_lock);
        assert_eq!(
            recover_pending(&prefix, ".demo.json").unwrap(),
            ExecutableUpdateOutcome::DiscardedStaged
        );
        assert!(!candidate.exists());
        assert!(
            config::read_metadata_for(&prefix, ".demo.json")
                .unwrap()
                .update
                .unwrap()
                .pending
                .is_none()
        );
    }

    #[test]
    fn forced_discard_replaces_a_stale_stage_while_the_new_lock_is_held() {
        let (tmp, prefix, _executable, _) = setup();
        stage_update(&tmp, &prefix, "2.0", 1);
        let (first_candidate, _) = pending_paths(&prefix);
        let _coordinator_lock = hold_update_lock(&prefix);

        assert_eq!(
            discard_unapproved(&prefix, ".demo.json").unwrap(),
            ExecutableUpdateOutcome::DiscardedStaged
        );
        assert!(!first_candidate.exists());

        stage_update(&tmp, &prefix, "3.0", 2);
        let meta = config::read_metadata_for(&prefix, ".demo.json").unwrap();
        let pending = meta.update.unwrap().pending.unwrap();
        assert_eq!(pending.version, "3.0");
        assert_eq!(pending.build_number, 2);
    }

    #[test]
    fn missing_staged_candidate_is_discarded_without_touching_the_old_runtime() {
        let (tmp, prefix, executable, old_digest) = setup();
        stage_update(&tmp, &prefix, "2.0", 1);
        let (candidate, _) = pending_paths(&prefix);
        std::fs::remove_file(candidate).unwrap();

        assert_eq!(
            discard_unapproved(&prefix, ".demo.json").unwrap(),
            ExecutableUpdateOutcome::DiscardedStaged
        );
        assert_installed(&prefix, &executable, "1.0", 0, &old_digest);
    }

    #[test]
    fn journal_json_contains_only_durable_recovery_state() {
        let (tmp, prefix, _executable, _) = setup();
        stage_update(&tmp, &prefix, "2.0", 1);

        let metadata_path = config::metadata_path_for(&prefix, ".demo.json");
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(metadata_path).unwrap()).unwrap();
        let update = value["update"].as_object().unwrap();
        let pending = update["pending"].as_object().unwrap();

        assert_eq!(pending["phase"], "staged");
        assert_eq!(pending["version"], "2.0");
        assert_eq!(pending["build-number"], 1);
        assert!(pending["executable_sha256"].as_str().is_some());

        for obsolete in [
            "transaction",
            "package_sha256",
            "provenance",
            "candidate",
            "backup",
            "old_version",
            "old_sha256",
            "new_sha256",
        ] {
            assert!(!pending.contains_key(obsolete), "unexpected {obsolete}");
        }
        assert!(!update.contains_key("version"));
        assert!(!update.contains_key("provenance"));
    }

    #[test]
    fn staging_repairs_invalid_deterministic_candidate_files() {
        let (tmp, prefix, _executable, _) = setup();
        let source = tmp
            .path()
            .join(format!("source-2.0-1{}", std::env::consts::EXE_SUFFIX));
        let digest = stamped_candidate(&source, "2.0", 1);
        let meta = config::read_metadata_for(&prefix, ".demo.json").unwrap();
        let update = meta.update.unwrap();
        let candidate = candidate_path(&update.executable, &digest).unwrap();
        let temporary = candidate.with_extension("new.tmp");
        std::fs::write(&candidate, b"incomplete candidate").unwrap();
        std::fs::write(&temporary, b"incomplete temporary candidate").unwrap();

        assert_eq!(
            stage_candidate(&prefix, ".demo.json", &source, "2.0", 1).unwrap(),
            ExecutableUpdateOutcome::Staged
        );
        assert_eq!(sha256_hex(&candidate).unwrap(), digest);
        assert!(!temporary.exists());
    }

    #[test]
    fn unknown_target_digest_fails_closed() {
        let (tmp, prefix, executable, _) = setup();
        stage_update(&tmp, &prefix, "2.0", 1);
        std::fs::write(&executable, b"not the recorded executable").unwrap();

        let error = recover_pending(&prefix, ".demo.json")
            .unwrap_err()
            .to_string();
        assert!(error.contains("neither recorded update digest"), "{error}");
    }

    #[test]
    fn matching_direct_reconciliation_preserves_an_unapproved_candidate() {
        let (tmp, prefix, executable, old_digest) = setup();
        stage_update(&tmp, &prefix, "2.0", 1);

        assert_eq!(
            reconcile_current_executable(
                &prefix,
                ".demo.json",
                &executable,
                UpdateOwnership::Direct,
                "demo",
                CHANNEL,
                PACKAGE,
                0,
                None,
                "1.0",
                &old_digest,
            )
            .unwrap(),
            ExecutableUpdateOutcome::None
        );
        let pending = config::read_metadata_for(&prefix, ".demo.json")
            .unwrap()
            .update
            .unwrap()
            .pending
            .unwrap();
        assert_eq!(pending.phase, PendingExecutablePhase::Staged);
        assert_eq!(pending.version, "2.0");
    }

    #[test]
    #[cfg(not(windows))]
    fn unix_recovery_applies_a_ready_candidate() {
        let (tmp, prefix, executable, _) = setup();
        let (_source, new_digest) = stage_update(&tmp, &prefix, "2.0", 1);
        mark_pending_ready(&prefix, ".demo.json").unwrap();

        assert_eq!(
            recover_pending(&prefix, ".demo.json").unwrap(),
            ExecutableUpdateOutcome::Applied
        );
        assert_installed(&prefix, &executable, "2.0", 1, &new_digest);
    }

    #[test]
    #[cfg(not(windows))]
    fn unix_recovery_retries_replacement_before_the_rename() {
        let (tmp, prefix, executable, _) = setup();
        let (_source, new_digest) = stage_update(&tmp, &prefix, "2.0", 1);
        mark_pending_ready(&prefix, ".demo.json").unwrap();
        persist_phase(&prefix, PendingExecutablePhase::Replacing);

        assert_eq!(
            recover_pending(&prefix, ".demo.json").unwrap(),
            ExecutableUpdateOutcome::Applied
        );
        assert_installed(&prefix, &executable, "2.0", 1, &new_digest);
    }

    #[test]
    #[cfg(not(windows))]
    fn unix_recovery_commits_metadata_after_the_atomic_rename() {
        let (tmp, prefix, executable, _) = setup();
        let (_source, new_digest) = stage_update(&tmp, &prefix, "2.0", 1);
        mark_pending_ready(&prefix, ".demo.json").unwrap();
        persist_phase(&prefix, PendingExecutablePhase::Replacing);
        let (candidate, _) = pending_paths(&prefix);
        std::fs::rename(candidate, &executable).unwrap();

        let before = config::read_metadata_for(&prefix, ".demo.json").unwrap();
        assert_eq!(before.version, "1.0");
        assert_eq!(before.update.as_ref().unwrap().build_number, 0);

        assert_eq!(
            recover_pending(&prefix, ".demo.json").unwrap(),
            ExecutableUpdateOutcome::Applied
        );
        assert_installed(&prefix, &executable, "2.0", 1, &new_digest);
    }

    fn assert_missing_candidate_preserves_old_runtime(phase: PendingExecutablePhase) {
        let (tmp, prefix, executable, old_digest) = setup();
        stage_update(&tmp, &prefix, "2.0", 1);
        mark_pending_ready(&prefix, ".demo.json").unwrap();
        persist_phase(&prefix, phase);
        let (candidate, _) = pending_paths(&prefix);
        std::fs::remove_file(candidate).unwrap();

        assert_eq!(
            recover_pending(&prefix, ".demo.json").unwrap(),
            ExecutableUpdateOutcome::RestoredPrevious
        );
        assert_installed(&prefix, &executable, "1.0", 0, &old_digest);
    }

    #[test]
    fn recovery_abandons_a_ready_update_when_its_candidate_is_missing() {
        assert_missing_candidate_preserves_old_runtime(PendingExecutablePhase::Ready);
    }

    #[test]
    fn recovery_abandons_a_replacing_update_when_its_candidate_is_missing() {
        assert_missing_candidate_preserves_old_runtime(PendingExecutablePhase::Replacing);
    }

    #[test]
    fn direct_reconciliation_accepts_the_recorded_identity() {
        let (_tmp, prefix, executable, digest) = setup();

        assert_eq!(
            reconcile_current_executable(
                &prefix,
                ".demo.json",
                &executable,
                UpdateOwnership::Direct,
                "demo",
                CHANNEL,
                PACKAGE,
                0,
                None,
                "1.0",
                &digest,
            )
            .unwrap(),
            ExecutableUpdateOutcome::None
        );
    }

    #[test]
    fn direct_reconciliation_rejects_update_source_rotation() {
        let (_tmp, prefix, executable, _) = setup();
        let digest = stamped_executable_with(
            &executable,
            "2.0",
            1,
            UpdateOwnership::Direct,
            "https://new.example.test/channel",
            "renamed-runtime",
            None,
        );

        let error = reconcile_current_executable(
            &prefix,
            ".demo.json",
            &executable,
            UpdateOwnership::Direct,
            "demo",
            "https://new.example.test/channel",
            "renamed-runtime",
            1,
            None,
            "2.0",
            &digest,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("outside its coordinated update"), "{error}");
    }

    #[test]
    fn direct_reconciliation_rejects_digest_changes_outside_an_update() {
        let (_tmp, prefix, executable, _) = setup();
        OpenOptions::new()
            .append(true)
            .open(&executable)
            .unwrap()
            .write_all(b"changed after bootstrap")
            .unwrap();
        let digest = sha256_hex(&executable).unwrap();

        let error = reconcile_current_executable(
            &prefix,
            ".demo.json",
            &executable,
            UpdateOwnership::Direct,
            "demo",
            CHANNEL,
            PACKAGE,
            0,
            None,
            "1.0",
            &digest,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("outside its coordinated update"), "{error}");
    }

    #[test]
    fn direct_reconciliation_rejects_a_different_executable_path() {
        let (tmp, prefix, executable, digest) = setup();
        let alternate = tmp
            .path()
            .join(format!("alternate{}", std::env::consts::EXE_SUFFIX));
        std::fs::copy(&executable, &alternate).unwrap();

        let error = reconcile_current_executable(
            &prefix,
            ".demo.json",
            &alternate,
            UpdateOwnership::Direct,
            "demo",
            CHANNEL,
            PACKAGE,
            0,
            None,
            "1.0",
            &digest,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("outside its coordinated update"), "{error}");
    }

    #[test]
    fn reconciliation_initializes_identity_from_pr1_metadata() {
        let (_tmp, prefix, executable, digest) = setup();
        let mut meta = config::read_metadata_for(&prefix, ".demo.json").unwrap();
        let update = meta.update.as_mut().unwrap();
        update.executable = PathBuf::new();
        update.artifact_name.clear();
        update.sha256.clear();
        config::persist_metadata_for(&prefix, ".demo.json", &meta).unwrap();

        assert_eq!(
            reconcile_current_executable(
                &prefix,
                ".demo.json",
                &executable,
                UpdateOwnership::Direct,
                "demo",
                CHANNEL,
                PACKAGE,
                0,
                None,
                "1.0",
                &digest,
            )
            .unwrap(),
            ExecutableUpdateOutcome::Reconciled
        );
        let meta = config::read_metadata_for(&prefix, ".demo.json").unwrap();
        let update = meta.update.unwrap();
        assert_eq!(update.executable, executable);
        assert_eq!(update.artifact_name, "demo");
        assert_eq!(update.sha256, digest);
    }

    #[test]
    fn external_reconciliation_accepts_manager_replacement_at_the_stable_path() {
        let (_tmp, prefix, executable, _) = setup();
        let mut meta = config::read_metadata_for(&prefix, ".demo.json").unwrap();
        let update = meta.update.as_mut().unwrap();
        update.ownership = UpdateOwnership::External;
        update.instruction = Some(EXTERNAL_INSTRUCTION.to_string());
        config::persist_metadata_for(&prefix, ".demo.json", &meta).unwrap();
        let digest = stamped_executable_with(
            &executable,
            "2.0",
            1,
            UpdateOwnership::External,
            CHANNEL,
            PACKAGE,
            Some(EXTERNAL_INSTRUCTION),
        );

        assert_eq!(
            reconcile_current_executable(
                &prefix,
                ".demo.json",
                &executable,
                UpdateOwnership::External,
                "demo",
                CHANNEL,
                PACKAGE,
                1,
                Some(EXTERNAL_INSTRUCTION),
                "2.0",
                &digest,
            )
            .unwrap(),
            ExecutableUpdateOutcome::Reconciled
        );
        let meta = config::read_metadata_for(&prefix, ".demo.json").unwrap();
        assert_eq!(meta.version, "2.0");
        let update = meta.update.unwrap();
        assert_eq!(update.executable, executable);
        assert_eq!(update.ownership, UpdateOwnership::External);
        assert_eq!(update.build_number, 1);
        assert_eq!(update.sha256, digest);
        assert!(update.pending.is_none());
    }

    #[test]
    #[cfg(windows)]
    fn windows_holds_test_executable() {
        let Some(ready) = std::env::var_os("CONDA_SHIP_UPDATE_HOLDER_READY") else {
            return;
        };
        let release = std::env::var_os("CONDA_SHIP_UPDATE_HOLDER_RELEASE").unwrap();
        std::fs::write(ready, b"ready").unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !Path::new(&release).exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(Path::new(&release).exists());
        if let Some(done) = std::env::var_os("CONDA_SHIP_UPDATE_HOLDER_DONE") {
            std::fs::write(done, b"done").unwrap();
        }
    }

    #[test]
    #[cfg(windows)]
    fn windows_worker_process_does_not_inherit_helper_handles() {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};

        let tmp = tempfile::tempdir().unwrap();
        let ready = tmp.path().join("ready");
        let release = tmp.path().join("release");
        let done = tmp.path().join("done");
        let sentinel_path = tmp.path().join("captured-output-handle");
        let sentinel = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(&sentinel_path)
            .unwrap();
        let inherited = unsafe {
            SetHandleInformation(
                sentinel.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT,
            )
        };
        assert_ne!(
            inherited, 0,
            "failed to mark the sentinel handle inheritable"
        );

        let application = std::env::current_exe().unwrap();
        let mut command_line = windows_application_command_line(&application).unwrap();
        command_line.pop();
        command_line.extend(
            " --exact executable_update::tests::windows_holds_test_executable --nocapture"
                .encode_utf16(),
        );
        command_line.push(0);
        let environment = windows_process_environment(
            &[
                (
                    "CONDA_SHIP_UPDATE_HOLDER_READY",
                    ready.as_os_str().to_os_string(),
                ),
                (
                    "CONDA_SHIP_UPDATE_HOLDER_RELEASE",
                    release.as_os_str().to_os_string(),
                ),
                (
                    "CONDA_SHIP_UPDATE_HOLDER_DONE",
                    done.as_os_str().to_os_string(),
                ),
            ],
            &[],
        )
        .unwrap();
        spawn_windows_process(&application, &mut command_line, &environment).unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(ready.exists(), "child process did not start");

        drop(sentinel);
        std::fs::remove_file(&sentinel_path)
            .expect("child process inherited an unrelated helper handle");
        std::fs::write(&release, b"release").unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        while !done.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(done.exists(), "child process did not exit");
    }

    #[test]
    #[cfg(windows)]
    fn windows_runs_update_worker() {
        let Some(prefix) = std::env::var_os("CONDA_SHIP_UPDATE_WORKER_PREFIX") else {
            return;
        };
        if let Some(ready) = std::env::var_os("CONDA_SHIP_UPDATE_WORKER_READY") {
            std::fs::write(ready, b"ready").unwrap();
        }
        run_windows_worker(Path::new(&prefix), ".demo.json").unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn windows_worker_keeps_the_stable_path_and_old_parent_until_cleanup() {
        use std::process::Command;

        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("prefix");
        let executable = tmp.path().join("demo.exe");
        let source = tmp.path().join("demo-new.exe");
        let holder_ready = tmp.path().join("holder-ready");
        let release = tmp.path().join("holder-release");
        let first_worker_ready = tmp.path().join("first-worker-ready");
        std::fs::create_dir(&prefix).unwrap();
        let old_digest = stamped_test_executable(&executable, "1.0", 0);
        let new_digest = stamped_test_executable(&source, "2.0", 1);
        let meta = metadata_for(
            executable.clone(),
            old_digest.clone(),
            UpdateOwnership::Direct,
            None,
        );
        config::persist_metadata_for(&prefix, ".demo.json", &meta).unwrap();
        ensure_update_lock(&prefix, ".demo.json").unwrap();

        let mut holder = Command::new(&executable)
            .args([
                "--exact",
                "executable_update::tests::windows_holds_test_executable",
                "--nocapture",
            ])
            .env("CONDA_SHIP_UPDATE_HOLDER_READY", &holder_ready)
            .env("CONDA_SHIP_UPDATE_HOLDER_RELEASE", &release)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !holder_ready.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(holder_ready.exists(), "holder process did not start");

        stage_candidate(&prefix, ".demo.json", &source, "2.0", 1).unwrap();
        mark_pending_ready(&prefix, ".demo.json").unwrap();
        let worker = {
            let _lock = BootstrapLock::acquire(&prefix).unwrap();
            let mut meta = read_owned_metadata(&prefix, ".demo.json").unwrap();
            prepare_windows_worker_locked(&prefix, ".demo.json", &mut meta).unwrap()
        };

        let mut first_worker = Command::new(&worker)
            .args([
                "--exact",
                "executable_update::tests::windows_runs_update_worker",
                "--nocapture",
            ])
            .env("CONDA_SHIP_UPDATE_WORKER_PREFIX", &prefix)
            .env("CONDA_SHIP_UPDATE_WORKER_READY", &first_worker_ready)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !first_worker_ready.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(first_worker_ready.exists(), "update worker did not start");
        std::thread::sleep(Duration::from_millis(100));
        assert!(first_worker.try_wait().unwrap().is_none());
        assert_eq!(sha256_hex(&executable).unwrap(), old_digest);

        first_worker.kill().unwrap();
        first_worker.wait().unwrap();
        assert_eq!(sha256_hex(&executable).unwrap(), old_digest);

        std::fs::write(&release, b"release").unwrap();
        assert!(holder.wait().unwrap().success());

        let second_worker = Command::new(&worker)
            .args([
                "--exact",
                "executable_update::tests::windows_runs_update_worker",
                "--nocapture",
            ])
            .env("CONDA_SHIP_UPDATE_WORKER_PREFIX", &prefix)
            .spawn()
            .unwrap()
            .wait_with_output()
            .unwrap();
        assert!(
            second_worker.status.success(),
            "{}",
            String::from_utf8_lossy(&second_worker.stderr)
        );
        assert_eq!(sha256_hex(&executable).unwrap(), new_digest);
        assert!(worker.is_file());

        let meta = config::read_metadata_for(&prefix, ".demo.json").unwrap();
        assert_eq!(meta.version, "1.0");
        let update = meta.update.unwrap();
        assert_eq!(update.build_number, 0);
        assert_eq!(update.sha256, old_digest);
        let pending = update.pending.unwrap();
        assert_eq!(pending.phase, PendingExecutablePhase::Cleanup);
        assert_eq!(pending.version, "2.0");
        assert_eq!(pending.build_number, 1);
        assert_eq!(pending.executable_sha256, new_digest);

        assert_eq!(
            recover_pending(&prefix, ".demo.json").unwrap(),
            ExecutableUpdateOutcome::Applied
        );
        assert!(!worker.exists());
        assert_installed(&prefix, &executable, "2.0", 1, &new_digest);
    }

    #[test]
    #[cfg(windows)]
    fn windows_recovery_commits_a_completed_rename() {
        let (tmp, prefix, executable, _) = setup();
        let source = tmp.path().join("source.exe");
        let new_digest = stamped_candidate(&source, "2.0", 1);
        stage_candidate(&prefix, ".demo.json", &source, "2.0", 1).unwrap();
        mark_pending_ready(&prefix, ".demo.json").unwrap();
        let worker = {
            let _lock = BootstrapLock::acquire(&prefix).unwrap();
            let mut meta = read_owned_metadata(&prefix, ".demo.json").unwrap();
            prepare_windows_worker_locked(&prefix, ".demo.json", &mut meta).unwrap()
        };
        let (candidate, _) = pending_paths(&prefix);
        std::fs::rename(candidate, &executable).unwrap();

        let meta = config::read_metadata_for(&prefix, ".demo.json").unwrap();
        assert_eq!(meta.version, "1.0");
        assert_eq!(meta.update.as_ref().unwrap().build_number, 0);
        assert_eq!(
            meta.update
                .as_ref()
                .unwrap()
                .pending
                .as_ref()
                .unwrap()
                .phase,
            PendingExecutablePhase::Replacing
        );

        assert_eq!(
            recover_pending(&prefix, ".demo.json").unwrap(),
            ExecutableUpdateOutcome::Applied
        );
        assert!(!worker.exists());
        assert_installed(&prefix, &executable, "2.0", 1, &new_digest);
    }

    #[test]
    #[cfg(windows)]
    fn windows_recovery_promotes_only_after_the_worker_was_removed() {
        let (tmp, prefix, executable, old_digest) = setup();
        let source = tmp.path().join("source.exe");
        let new_digest = stamped_candidate(&source, "2.0", 1);
        stage_candidate(&prefix, ".demo.json", &source, "2.0", 1).unwrap();
        mark_pending_ready(&prefix, ".demo.json").unwrap();
        let worker = {
            let _lock = BootstrapLock::acquire(&prefix).unwrap();
            let mut meta = read_owned_metadata(&prefix, ".demo.json").unwrap();
            prepare_windows_worker_locked(&prefix, ".demo.json", &mut meta).unwrap()
        };
        let (candidate, _) = pending_paths(&prefix);
        std::fs::rename(candidate, &executable).unwrap();
        persist_phase(&prefix, PendingExecutablePhase::Cleanup);

        let meta = config::read_metadata_for(&prefix, ".demo.json").unwrap();
        assert_eq!(meta.version, "1.0");
        assert_eq!(meta.update.as_ref().unwrap().build_number, 0);
        assert_eq!(meta.update.as_ref().unwrap().sha256, old_digest);
        assert!(worker.is_file());

        std::fs::remove_file(&worker).unwrap();
        assert_eq!(
            recover_pending(&prefix, ".demo.json").unwrap(),
            ExecutableUpdateOutcome::Applied
        );
        assert_installed(&prefix, &executable, "2.0", 1, &new_digest);
    }
}
