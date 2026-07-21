//! Cross-process serialization for automatic runtime bootstrap.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use fs4::fs_std::FileExt;
use miette::{Context, IntoDiagnostic};

use crate::policy;

pub(crate) struct BootstrapLock {
    _file: File,
}

impl BootstrapLock {
    pub(crate) fn acquire(prefix: &Path) -> miette::Result<Self> {
        let path = path(prefix)?;
        let parent = path
            .parent()
            .ok_or_else(|| miette::miette!("bootstrap lock has no parent directory"))?;
        std::fs::create_dir_all(parent)
            .into_diagnostic()
            .with_context(|| {
                format!(
                    "failed to create bootstrap lock directory at {}",
                    policy::path_for_display(parent)
                )
            })?;

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .into_diagnostic()
            .with_context(|| {
                format!(
                    "failed to open bootstrap lock at {}",
                    policy::path_for_display(&path)
                )
            })?;
        file.lock_exclusive().into_diagnostic().with_context(|| {
            format!(
                "failed to acquire bootstrap lock at {}",
                policy::path_for_display(&path)
            )
        })?;
        Ok(Self { _file: file })
    }
}

pub(crate) fn path(prefix: &Path) -> miette::Result<PathBuf> {
    let parent = prefix.parent().ok_or_else(|| {
        miette::miette!(
            "install path has no parent directory: {}",
            policy::path_for_display(prefix)
        )
    })?;
    let name = prefix.file_name().ok_or_else(|| {
        miette::miette!(
            "install path has no final component: {}",
            policy::path_for_display(prefix)
        )
    })?;
    Ok(parent.join(format!(".{}.conda-ship.lock", name.to_string_lossy())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_lock_is_adjacent_to_prefix() {
        let tmp = TempDir::new().unwrap();
        let prefix = tmp.path().join("runtime");

        let lock = path(&prefix).unwrap();

        assert_eq!(lock.parent(), prefix.parent());
        assert!(!lock.starts_with(&prefix));
    }
}
