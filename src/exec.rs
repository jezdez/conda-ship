//! Replace the current process with the installed delegate executable,
//! or run conda as a subprocess with output filtering where needed.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use miette::IntoDiagnostic;

use crate::policy;

pub(crate) fn executable_in_prefix(prefix: &Path, executable: &str) -> std::path::PathBuf {
    let executable = executable_filename(executable);
    if cfg!(windows) {
        for dir in prefix_path_entries(prefix) {
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

fn build_delegate_command(prefix: &Path, delegate: &str, args: &[&str]) -> miette::Result<Command> {
    validate_executable_name(delegate)?;
    let delegate_bin = executable_in_prefix(prefix, delegate);
    if !delegate_bin.exists() {
        return Err(miette::miette!(
            "{delegate} executable not found at {}",
            policy::path_for_display(&delegate_bin)
        ));
    }
    let mut cmd = Command::new(delegate_bin);
    cmd.args(args);
    apply_delegate_environment(&mut cmd, prefix)?;
    Ok(cmd)
}

fn build_conda_command(prefix: &Path, args: &[&str]) -> miette::Result<Command> {
    build_delegate_command(prefix, "conda", args)
}

pub(crate) fn apply_delegate_environment(cmd: &mut Command, prefix: &Path) -> miette::Result<()> {
    cmd.env("CONDA_ROOT_PREFIX", prefix);
    cmd.env("CONDA_PREFIX", prefix);
    cmd.env("CONDA_DEFAULT_ENV", "base");
    cmd.env("CONDA_SHLVL", "1");
    cmd.env("CONDA_COMPLETION_COMMAND_NAME", policy::command_name());
    cmd.env("PATH", delegate_path_env(prefix)?);
    Ok(())
}

fn delegate_path_env(prefix: &Path) -> miette::Result<OsString> {
    let mut paths = prefix_path_entries(prefix);
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths)
        .map_err(|err| miette::miette!("failed to construct delegate PATH: {err}"))
}

pub(crate) fn prefix_path_entries(prefix: &Path) -> Vec<PathBuf> {
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

/// Replace the current process with the conda binary, passing along arguments.
/// On Unix this uses the exec syscall; on Windows it spawns and exits.
pub fn replace_process_with_conda(prefix: &Path, args: &[&str]) -> miette::Result<()> {
    replace_process_with_delegate(prefix, "conda", args)
}

/// Replace the current process with the configured delegate executable,
/// passing along arguments. On Unix this uses the exec syscall; on Windows it
/// spawns and exits.
pub fn replace_process_with_delegate(
    prefix: &Path,
    delegate: &str,
    args: &[&str],
) -> miette::Result<()> {
    hand_off(build_delegate_command(prefix, delegate, args)?, delegate)
}

/// Run conda as a subprocess, filtering activation hints from stdout and
/// replacing them with runtime-appropriate guidance. Used for commands like
/// `create` and `env create` that print "conda activate" instructions.
pub fn run_conda_filtered(prefix: &Path, args: &[&str]) -> miette::Result<()> {
    let mut child = build_conda_command(prefix, args)?
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .into_diagnostic()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| miette::miette!("failed to capture conda stdout"))?;
    let reader = BufReader::new(stdout);

    let mut in_activate_block = false;
    let mut env_name: Option<String> = None;

    for line in reader.lines() {
        let line = line.into_diagnostic()?;

        if line.contains("To activate this environment") {
            in_activate_block = true;
            continue;
        }

        if in_activate_block {
            if let Some(name) = line.strip_prefix("#     $ conda activate ") {
                env_name = Some(name.trim().trim_matches('"').to_string());
            }
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            in_activate_block = false;
        }

        println!("{}", line);
    }

    let status = child.wait().into_diagnostic()?;
    let code = status.code().unwrap_or(1);

    if code == 0 {
        let name = env_name.or_else(|| extract_env_name(args));
        if let Some(name) = name {
            println!("#");
            println!("# To activate this environment, use");
            println!("#");
            println!("#     $ {} shell {name}", policy::command_name());
            println!("#");
            println!("# To leave the environment, exit the subshell (Ctrl+D or `exit`).");
            println!("#");
        }
    }

    std::process::exit(code);
}

/// Returns true if this subcommand may print conda activation hints,
/// meaning it should be routed through `run_conda_filtered`.
pub fn needs_output_filtering(args: &[&str]) -> bool {
    match args.first().copied() {
        Some("create") => true,
        Some("env") => args.get(1).copied() == Some("create"),
        _ => false,
    }
}

/// True when `create` / `env create` should use piped stdout for activation-hint filtering.
///
/// Conda prompts for confirmation by writing to stdout without a trailing newline, then
/// reading stdin. If we pipe stdout and read it line-by-line, the parent blocks waiting for a
/// newline while conda blocks on stdin, so the prompt never reaches the terminal and input
/// looks swallowed.
pub fn should_filter_conda_output(args: &[&str]) -> bool {
    needs_output_filtering(args)
        && (!std::io::stdin().is_terminal() || conda_always_yes_in_args(args))
}

fn conda_always_yes_in_args(args: &[&str]) -> bool {
    args.iter().any(|&a| a == "-y" || a == "--yes")
}

pub(crate) fn extract_env_name(args: &[&str]) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match *arg {
            "-n" | "--name" => return iter.next().map(|s| s.to_string()),
            _ => {
                if let Some(name) = arg.strip_prefix("--name=") {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

#[cfg(unix)]
fn hand_off(mut cmd: std::process::Command, delegate: &str) -> miette::Result<()> {
    use std::os::unix::process::CommandExt;
    let err = cmd.exec();
    Err(miette::miette!("failed to launch {delegate}: {}", err))
}

#[cfg(not(unix))]
fn hand_off(mut cmd: std::process::Command, _delegate: &str) -> miette::Result<()> {
    let status = cmd.status().into_diagnostic()?;
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::path::Path;
    use tempfile::TempDir;

    #[rstest]
    #[case::create(&["create", "-n", "test"], true)]
    #[case::env_create(&["env", "create", "-n", "test"], true)]
    #[case::env_list(&["env", "list"], false)]
    #[case::install(&["install", "numpy"], false)]
    #[case::list(&["list"], false)]
    #[case::empty(&[], false)]
    fn test_needs_output_filtering(#[case] args: &[&str], #[case] expected: bool) {
        assert_eq!(needs_output_filtering(args), expected);
    }

    #[rstest]
    #[case::short_flag(&["create", "-n", "myenv"], Some("myenv"))]
    #[case::long_flag(&["create", "--name", "myenv"], Some("myenv"))]
    #[case::equals_syntax(&["create", "--name=myenv"], Some("myenv"))]
    #[case::prefix_flag(&["create", "-p", "/tmp"], None)]
    #[case::empty(&[], None)]
    fn test_extract_env_name(#[case] args: &[&str], #[case] expected: Option<&str>) {
        assert_eq!(extract_env_name(args), expected.map(String::from));
    }

    #[rstest]
    #[case::no_yes_flag(&["create", "-n", "x"], false)]
    #[case::short_y(&["create", "-y", "-n", "x"], true)]
    #[case::long_yes(&["env", "create", "--yes", "-n", "x"], true)]
    fn test_conda_always_yes_in_args(#[case] args: &[&str], #[case] expected: bool) {
        assert_eq!(super::conda_always_yes_in_args(args), expected);
    }

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
        let result = build_conda_command(tmp.path(), &["info"]);
        assert!(
            result.is_err(),
            "build_command should fail when conda binary is missing"
        );
    }

    #[test]
    fn test_build_command_with_binary() {
        let tmp = TempDir::new().unwrap();
        let bin_dir = if cfg!(windows) {
            tmp.path().join("Scripts")
        } else {
            tmp.path().join("bin")
        };
        std::fs::create_dir_all(&bin_dir).unwrap();

        let conda_path = if cfg!(windows) {
            bin_dir.join("conda.exe")
        } else {
            bin_dir.join("conda")
        };
        std::fs::write(&conda_path, "#!/bin/sh\n").unwrap();

        let result = build_conda_command(tmp.path(), &["info", "--json"]);
        assert!(result.is_ok(), "build_command should succeed with a binary");
        let cmd = result.unwrap();
        let program = cmd.get_program().to_str().unwrap().to_string();
        assert!(
            program.contains("conda"),
            "program should be the conda binary"
        );
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args.len(), 2, "should have 2 args");

        let envs: Vec<_> = cmd.get_envs().collect();
        let root_prefix = envs
            .iter()
            .find(|(k, _)| *k == "CONDA_ROOT_PREFIX")
            .expect("CONDA_ROOT_PREFIX should be set");
        assert_eq!(
            root_prefix.1.unwrap(),
            tmp.path().as_os_str(),
            "CONDA_ROOT_PREFIX should point to the prefix"
        );
        let conda_prefix = envs
            .iter()
            .find(|(k, _)| *k == "CONDA_PREFIX")
            .expect("CONDA_PREFIX should be set");
        assert_eq!(
            conda_prefix.1.unwrap(),
            tmp.path().as_os_str(),
            "CONDA_PREFIX should point to the prefix"
        );
        let default_env = envs
            .iter()
            .find(|(k, _)| *k == "CONDA_DEFAULT_ENV")
            .expect("CONDA_DEFAULT_ENV should be set");
        assert_eq!(default_env.1.unwrap(), "base");
        let completion_command_name = envs
            .iter()
            .find(|(k, _)| *k == "CONDA_COMPLETION_COMMAND_NAME")
            .expect("CONDA_COMPLETION_COMMAND_NAME should be set");
        assert_eq!(completion_command_name.1.unwrap(), policy::command_name());
        let path_env = envs
            .iter()
            .find(|(k, _)| *k == "PATH")
            .and_then(|(_, value)| *value)
            .expect("PATH should be set");
        let path_dirs: Vec<_> = std::env::split_paths(path_env).collect();
        assert!(
            path_dirs.contains(&bin_dir),
            "PATH should include the delegate binary directory"
        );
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
        let result = build_delegate_command(tmp.path(), "../python", &["--version"]);
        assert!(result.is_err());
    }
}
