//! Replace the current process with the installed delegate executable.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::policy;

pub(crate) fn executable_in_prefix(prefix: &Path, executable: &str) -> PathBuf {
    let executable = executable_filename(executable);
    if cfg!(windows) {
        for dir in delegate_path_dirs(prefix) {
            let candidate = dir.join(&executable);
            if candidate.exists() {
                return candidate;
            }
        }
        if executable.eq_ignore_ascii_case("conda.exe") {
            prefix.join("Scripts").join(executable)
        } else {
            prefix.join(executable)
        }
    } else {
        prefix.join("bin").join(executable)
    }
}

fn executable_filename(executable: &str) -> String {
    if cfg!(windows) && !executable.to_ascii_lowercase().ends_with(".exe") {
        format!("{executable}.exe")
    } else {
        executable.to_string()
    }
}

fn validate_executable_name(executable: &str) -> miette::Result<()> {
    if executable.is_empty()
        || executable == "."
        || executable == ".."
        || executable.contains('/')
        || executable.contains('\\')
        || executable.chars().any(char::is_control)
    {
        return Err(miette::miette!(
            "invalid delegate executable name: {executable:?}"
        ));
    }
    Ok(())
}

pub(crate) fn validate_delegate(prefix: &Path, delegate: &str) -> miette::Result<()> {
    validate_executable_name(delegate)?;
    let delegate_bin = executable_in_prefix(prefix, delegate);
    if !delegate_bin.is_file() {
        return Err(miette::miette!(
            "{delegate} executable not found at {}",
            policy::path_for_display(&delegate_bin)
        ));
    }
    Ok(())
}

fn build_delegate_command(
    prefix: &Path,
    delegate: &str,
    args: &[OsString],
) -> miette::Result<Command> {
    validate_delegate(prefix, delegate)?;
    let delegate_bin = executable_in_prefix(prefix, delegate);
    let mut command = Command::new(delegate_bin);
    command.args(args);
    apply_delegate_environment(&mut command, prefix)?;
    Ok(command)
}

pub(crate) fn apply_delegate_environment(
    command: &mut Command,
    prefix: &Path,
) -> miette::Result<()> {
    command.env("PATH", delegate_path_env(prefix)?);
    Ok(())
}

fn delegate_path_env(prefix: &Path) -> miette::Result<OsString> {
    let mut paths = delegate_path_dirs(prefix);
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths)
        .map_err(|err| miette::miette!("failed to construct delegate PATH: {err}"))
}

fn delegate_path_dirs(prefix: &Path) -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![
            prefix.to_path_buf(),
            prefix.join("Library").join("mingw-w64").join("bin"),
            prefix.join("Library").join("usr").join("bin"),
            prefix.join("Library").join("bin"),
            prefix.join("Scripts"),
            prefix.join("bin"),
            prefix.join("condabin"),
        ]
    } else {
        vec![prefix.join("bin"), prefix.join("condabin")]
    }
}

/// Replace the current process with the configured delegate executable.
///
/// Unix uses `exec`. Windows spawns the delegate and exits with the same code.
pub fn replace_process_with_delegate(
    prefix: &Path,
    delegate: &str,
    args: &[OsString],
) -> miette::Result<()> {
    hand_off(build_delegate_command(prefix, delegate, args)?, delegate)
}

#[cfg(unix)]
fn hand_off(mut command: Command, delegate: &str) -> miette::Result<()> {
    use std::os::unix::process::CommandExt;
    let error = command.exec();
    Err(miette::miette!("failed to launch {delegate}: {error}"))
}

#[cfg(not(unix))]
fn hand_off(mut command: Command, _delegate: &str) -> miette::Result<()> {
    use miette::IntoDiagnostic;

    let status = command.status().into_diagnostic()?;
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    #[cfg(not(windows))]
    fn test_executable_in_prefix_conda_unix() {
        assert_eq!(
            executable_in_prefix(Path::new("/opt/conda"), "conda"),
            Path::new("/opt/conda/bin/conda")
        );
    }

    #[test]
    #[cfg(windows)]
    fn test_executable_in_prefix_conda_windows() {
        assert_eq!(
            executable_in_prefix(Path::new("C:\\conda"), "conda"),
            Path::new("C:\\conda\\Scripts\\conda.exe")
        );
    }

    #[test]
    fn test_build_command_missing_binary() {
        let tmp = TempDir::new().unwrap();
        let result = build_delegate_command(tmp.path(), "conda", &["info".into()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_command_preserves_arguments_and_environment() {
        let tmp = TempDir::new().unwrap();
        let bin_dir = if cfg!(windows) {
            tmp.path().join("Scripts")
        } else {
            tmp.path().join("bin")
        };
        std::fs::create_dir_all(&bin_dir).unwrap();

        let delegate_path = if cfg!(windows) {
            bin_dir.join("conda.exe")
        } else {
            bin_dir.join("conda")
        };
        std::fs::write(&delegate_path, "#!/bin/sh\n").unwrap();

        let args = vec![OsString::from("info"), OsString::from("--json")];
        let command = build_delegate_command(tmp.path(), "conda", &args).unwrap();
        let actual_args: Vec<OsString> = command
            .get_args()
            .map(std::ffi::OsStr::to_os_string)
            .collect();
        assert_eq!(actual_args, args);

        let envs: Vec<_> = command.get_envs().collect();
        assert!(envs.iter().all(|(name, _)| {
            !matches!(
                name.to_str(),
                Some("CONDA_ROOT_PREFIX" | "CONDA_PREFIX" | "CONDA_DEFAULT_ENV" | "CONDA_SHLVL")
            )
        }));
        let path_env = envs
            .iter()
            .find(|(name, _)| *name == "PATH")
            .and_then(|(_, value)| *value)
            .expect("PATH should be set");
        let path_dirs: Vec<_> = std::env::split_paths(path_env).collect();
        assert!(path_dirs.contains(&bin_dir));
    }

    #[test]
    fn test_executable_in_prefix_uses_delegate_name() {
        let expected = if cfg!(windows) {
            Path::new("/opt/conda/python.exe")
        } else {
            Path::new("/opt/conda/bin/python")
        };
        assert_eq!(
            executable_in_prefix(Path::new("/opt/conda"), "python"),
            expected
        );
    }

    #[test]
    fn test_delegate_command_rejects_path_like_name() {
        let tmp = TempDir::new().unwrap();
        let result = build_delegate_command(tmp.path(), "../python", &["--version".into()]);
        assert!(result.is_err());
    }
}
