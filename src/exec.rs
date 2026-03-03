//! Replace the current process with the installed conda binary,
//! or run it as a subprocess with output filtering.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Stdio;

use miette::IntoDiagnostic;

fn conda_binary(prefix: &Path) -> std::path::PathBuf {
    if cfg!(windows) {
        prefix.join("Scripts").join("conda.exe")
    } else {
        prefix.join("bin").join("conda")
    }
}

fn build_command(prefix: &Path, args: &[&str]) -> miette::Result<std::process::Command> {
    let conda_bin = conda_binary(prefix);
    if !conda_bin.exists() {
        return Err(miette::miette!(
            "conda binary not found at {}",
            conda_bin.display()
        ));
    }
    let mut cmd = std::process::Command::new(conda_bin);
    cmd.args(args);
    cmd.env("CONDA_ROOT_PREFIX", prefix);
    Ok(cmd)
}

/// Replace the current process with the conda binary, passing along arguments.
/// On Unix this uses the exec syscall; on Windows it spawns and exits.
pub fn replace_process_with_conda(prefix: &Path, args: &[&str]) -> miette::Result<()> {
    hand_off(build_command(prefix, args)?)
}

/// Run conda as a subprocess, filtering activation hints from stdout and
/// replacing them with cx-appropriate guidance. Used for commands like
/// `create` and `env create` that print "conda activate" instructions.
pub fn run_conda_filtered(prefix: &Path, args: &[&str]) -> miette::Result<()> {
    let mut child = build_command(prefix, args)?
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .into_diagnostic()?;

    let stdout = child.stdout.take().unwrap();
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
            println!("#     $ cx shell {name}");
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

fn extract_env_name(args: &[&str]) -> Option<String> {
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
fn hand_off(mut cmd: std::process::Command) -> miette::Result<()> {
    use std::os::unix::process::CommandExt;
    let err = cmd.exec();
    Err(miette::miette!("failed to launch conda: {}", err))
}

#[cfg(not(unix))]
fn hand_off(mut cmd: std::process::Command) -> miette::Result<()> {
    let status = cmd.status().into_diagnostic()?;
    std::process::exit(status.code().unwrap_or(1));
}
