//! Replace the current process with the installed conda binary.

use std::path::Path;

fn conda_binary(prefix: &Path) -> std::path::PathBuf {
    if cfg!(windows) {
        prefix.join("Scripts").join("conda.exe")
    } else {
        prefix.join("bin").join("conda")
    }
}

/// Replace the current process with the conda binary, passing along arguments.
/// On Unix this uses the exec syscall; on Windows it spawns and exits.
pub fn replace_process_with_conda(prefix: &Path, args: &[&str]) -> miette::Result<()> {
    let conda_bin = conda_binary(prefix);
    if !conda_bin.exists() {
        return Err(miette::miette!(
            "conda binary not found at {}",
            conda_bin.display()
        ));
    }

    let mut cmd = std::process::Command::new(&conda_bin);
    cmd.args(args);
    cmd.env("CONDA_ROOT_PREFIX", prefix);

    hand_off(cmd)
}

#[cfg(unix)]
fn hand_off(mut cmd: std::process::Command) -> miette::Result<()> {
    use std::os::unix::process::CommandExt;
    // Replaces the current process image with conda via the exec syscall
    let err = cmd.exec();
    Err(miette::miette!("failed to launch conda: {}", err))
}

#[cfg(not(unix))]
fn hand_off(mut cmd: std::process::Command) -> miette::Result<()> {
    use miette::IntoDiagnostic;
    let status = cmd.status().into_diagnostic()?;
    std::process::exit(status.code().unwrap_or(1));
}
