//! Internal ownership state for an in-progress bootstrap.

use std::io::Write;
use std::path::{Path, PathBuf};

use miette::{Context, IntoDiagnostic};

use crate::policy;

const BOOTSTRAP_STATE_SCHEMA_VERSION: u8 = 1;
const BOOTSTRAP_STATE_FILE: &str = ".conda-ship-bootstrap.json";
const BOOTSTRAP_STATE_TEMP_FILE: &str = ".conda-ship-bootstrap.json.tmp";

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum BootstrapPhase {
    Installing,
    Ready,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BootstrapState {
    schema_version: u8,
    state: BootstrapPhase,
    display_name: String,
    install_name: String,
    metadata_file: String,
}

impl BootstrapState {
    fn current(state: BootstrapPhase) -> Self {
        Self {
            schema_version: BOOTSTRAP_STATE_SCHEMA_VERSION,
            state,
            display_name: policy::display_name().to_string(),
            install_name: policy::install_name().to_string(),
            metadata_file: policy::metadata_file().to_string(),
        }
    }

    pub(crate) fn phase(&self) -> BootstrapPhase {
        self.state
    }
}

pub(crate) fn path(prefix: &Path) -> PathBuf {
    prefix.join(BOOTSTRAP_STATE_FILE)
}

fn temporary_path(prefix: &Path) -> PathBuf {
    prefix.join(BOOTSTRAP_STATE_TEMP_FILE)
}

fn remove_regular_file_if_present(path: &Path) -> miette::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(miette::miette!(
                "bootstrap state is not a regular file: {}",
                policy::path_for_display(path)
            ))
        }
        Ok(_) => std::fs::remove_file(path)
            .into_diagnostic()
            .with_context(|| {
                format!(
                    "failed to remove bootstrap state at {}",
                    policy::path_for_display(path)
                )
            }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).into_diagnostic(),
    }
}

fn read_state_file(path: &Path) -> miette::Result<Option<String>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).into_diagnostic().with_context(|| {
                format!(
                    "failed to inspect bootstrap state at {}",
                    policy::path_for_display(path)
                )
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(miette::miette!(
            "bootstrap state is not a regular file: {}",
            policy::path_for_display(path)
        ));
    }
    std::fs::read_to_string(path)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to read bootstrap state at {}",
                policy::path_for_display(path)
            )
        })
        .map(Some)
}

pub(crate) fn read(prefix: &Path) -> miette::Result<Option<BootstrapState>> {
    let path = path(prefix);
    let (path, data) = if let Some(data) = read_state_file(&path)? {
        (path, data)
    } else {
        let temporary_path = temporary_path(prefix);
        let Some(data) = read_state_file(&temporary_path)? else {
            return Ok(None);
        };
        (temporary_path, data)
    };
    let state = serde_json::from_str(&data)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to parse bootstrap state at {}",
                policy::path_for_display(&path)
            )
        })?;
    Ok(Some(state))
}

pub(crate) fn write_installing(prefix: &Path) -> miette::Result<()> {
    std::fs::create_dir_all(prefix)
        .into_diagnostic()
        .with_context(|| format!("failed to create {}", policy::path_for_display(prefix)))?;

    let path = path(prefix);
    let temporary_path = temporary_path(prefix);
    remove_regular_file_if_present(&temporary_path)?;
    let mut temporary = std::fs::File::create(&temporary_path)
        .into_diagnostic()
        .context("failed to create temporary bootstrap state")?;
    serde_json::to_writer_pretty(
        &mut temporary,
        &BootstrapState::current(BootstrapPhase::Installing),
    )
    .into_diagnostic()
    .context("failed to render bootstrap state")?;
    temporary
        .write_all(b"\n")
        .into_diagnostic()
        .context("failed to write bootstrap state")?;
    temporary
        .sync_all()
        .into_diagnostic()
        .context("failed to sync bootstrap state")?;
    drop(temporary);

    remove_regular_file_if_present(&path)?;
    std::fs::rename(&temporary_path, &path)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to persist bootstrap state at {}",
                policy::path_for_display(&path)
            )
        })?;
    Ok(())
}

pub(crate) fn remove(prefix: &Path) -> miette::Result<()> {
    for path in [path(prefix), temporary_path(prefix)] {
        remove_regular_file_if_present(&path)?;
    }
    Ok(())
}

pub(crate) fn validate_identity(state: &BootstrapState) -> miette::Result<()> {
    if state.schema_version != BOOTSTRAP_STATE_SCHEMA_VERSION {
        return Err(miette::miette!(
            "unsupported bootstrap state schema version: {}",
            state.schema_version
        ));
    }
    if state.display_name != policy::display_name() {
        return Err(miette::miette!(
            "bootstrap state belongs to {}, not {}",
            state.display_name,
            policy::display_name()
        ));
    }
    if state.install_name != policy::install_name() {
        return Err(miette::miette!(
            "bootstrap state install name is {}, expected {}",
            state.install_name,
            policy::install_name()
        ));
    }
    if state.metadata_file != policy::metadata_file() {
        return Err(miette::miette!(
            "bootstrap state metadata file is {}, expected {}",
            state.metadata_file,
            policy::metadata_file()
        ));
    }
    if state.state != BootstrapPhase::Installing {
        return Err(miette::miette!(
            "bootstrap ownership marker has invalid state: {:?}",
            state.state
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_bootstrap_installing_state_roundtrip() {
        let tmp = TempDir::new().unwrap();

        write_installing(tmp.path()).unwrap();
        let installing = read(tmp.path()).unwrap().unwrap();
        validate_identity(&installing).unwrap();
        assert_eq!(installing.phase(), BootstrapPhase::Installing);
    }

    #[test]
    fn test_bootstrap_state_rejects_another_runtime() {
        let mut state = BootstrapState::current(BootstrapPhase::Installing);
        state.install_name = "another-runtime".to_string();

        let error = validate_identity(&state).unwrap_err().to_string();

        assert!(error.contains("another-runtime"));
        assert!(error.contains("expected"));
    }

    #[test]
    fn test_temporary_installing_state_is_recoverable() {
        let tmp = TempDir::new().unwrap();
        let state = BootstrapState::current(BootstrapPhase::Installing);
        std::fs::write(
            temporary_path(tmp.path()),
            serde_json::to_vec_pretty(&state).unwrap(),
        )
        .unwrap();

        let recovered = read(tmp.path()).unwrap().unwrap();

        validate_identity(&recovered).unwrap();
        assert_eq!(recovered.phase(), BootstrapPhase::Installing);
        assert!(!path(tmp.path()).exists());
    }

    #[test]
    fn test_ready_marker_is_rejected() {
        let state = BootstrapState::current(BootstrapPhase::Ready);

        let error = validate_identity(&state).unwrap_err().to_string();

        assert!(error.contains("invalid state"));
    }

    #[test]
    fn test_remove_cleans_final_and_temporary_markers() {
        let tmp = TempDir::new().unwrap();
        write_installing(tmp.path()).unwrap();
        std::fs::write(temporary_path(tmp.path()), b"stale").unwrap();

        remove(tmp.path()).unwrap();

        assert!(!path(tmp.path()).exists());
        assert!(!temporary_path(tmp.path()).exists());
    }
}
