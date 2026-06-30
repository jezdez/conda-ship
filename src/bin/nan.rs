use std::{
    ffi::OsString,
    path::PathBuf,
    process::{Command as ProcessCommand, Stdio},
};

use clap::{Parser, Subcommand};
use conda_ship::fleet::{
    Fleet, FleetConfig, InstallOptions, InstalledRuntime, RemoveOptions, RuntimeCommand,
    RuntimeSpec, ShimMode, ShimOptions,
};
use miette::{Context, IntoDiagnostic};

#[derive(Parser)]
#[command(
    name = "nan",
    about = "Example CLI for the experimental conda-fleet API",
    long_about = "nan is an example binary for the experimental conda-fleet API. It is not a product CLI."
)]
struct Cli {
    /// Fleet install root. Required to avoid accidental writes to user-global locations.
    #[arg(long, global = true)]
    install_root: Option<PathBuf>,

    #[command(subcommand)]
    command: NanCommand,
}

#[derive(Subcommand)]
enum NanCommand {
    /// Install a runtime from a JSON RuntimeSpec.
    Install {
        /// JSON file matching conda_ship::fleet::RuntimeSpec.
        #[arg(long)]
        spec: PathBuf,

        /// Optional local package bundle directory.
        #[arg(long)]
        bundle: Option<PathBuf>,

        /// Install without network access.
        #[arg(long)]
        offline: bool,

        /// Replace an existing managed runtime with the same id.
        #[arg(long)]
        force: bool,
    },

    /// List installed runtimes as JSON.
    List,

    /// Show one installed runtime as JSON, or null when missing.
    Status {
        /// Runtime id.
        id: String,
    },

    /// Remove an installed runtime.
    Remove {
        /// Runtime id.
        id: String,

        /// Signal an explicit destructive action.
        #[arg(long)]
        force: bool,
    },

    /// Run a command inside an installed runtime.
    Run {
        /// Runtime id.
        id: String,

        /// Command name inside the runtime prefix.
        command: String,

        /// Arguments passed to the runtime command after `--`.
        #[arg(last = true)]
        args: Vec<OsString>,
    },

    /// Print a data-only shim plan as JSON.
    ShimPlan {
        /// Runtime id.
        id: String,

        /// Command name inside the runtime prefix.
        command: String,

        /// Name of the shim file the caller would write.
        #[arg(long)]
        shim_name: String,
    },
}

#[tokio::main]
async fn main() -> miette::Result<()> {
    let cli = Cli::parse();
    let install_root = cli
        .install_root
        .ok_or_else(|| miette::miette!("--install-root PATH is required"))?;
    let fleet = Fleet::new(FleetConfig::new(install_root));

    match cli.command {
        NanCommand::Install {
            spec,
            bundle,
            offline,
            force,
        } => {
            let spec = read_spec(&spec)?;
            let runtime = fleet
                .install(
                    spec,
                    InstallOptions {
                        force,
                        offline,
                        bundle_dir: bundle,
                        ..InstallOptions::default()
                    },
                )
                .await?;
            print_json(&runtime)?;
        }
        NanCommand::List => {
            print_json(&fleet.list()?)?;
        }
        NanCommand::Status { id } => {
            print_json(&fleet.status(&id)?)?;
        }
        NanCommand::Remove { id, force } => {
            fleet.remove(&id, RemoveOptions { force })?;
        }
        NanCommand::Run { id, command, args } => {
            let runtime = require_runtime(&fleet, &id)?;
            let runtime_command = runtime.command(&command)?;
            run_command(runtime_command, args)?;
        }
        NanCommand::ShimPlan {
            id,
            command,
            shim_name,
        } => {
            let runtime = require_runtime(&fleet, &id)?;
            let plan = runtime.shim_plan(
                &command,
                ShimOptions {
                    shim_name,
                    target_command: command.clone(),
                    mode: ShimMode::WrapperScript,
                    shim_dir: None,
                },
            )?;
            print_json(&plan)?;
        }
    }

    Ok(())
}

fn read_spec(path: &PathBuf) -> miette::Result<RuntimeSpec> {
    let data = std::fs::read_to_string(path)
        .into_diagnostic()
        .with_context(|| format!("failed to read runtime spec at {}", path.to_string_lossy()))?;
    serde_json::from_str(&data)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to parse runtime spec JSON at {}",
                path.to_string_lossy()
            )
        })
}

fn require_runtime(fleet: &Fleet, id: &str) -> miette::Result<InstalledRuntime> {
    fleet
        .status(id)?
        .ok_or_else(|| miette::miette!("runtime {id} is not installed"))
}

fn run_command(command: RuntimeCommand, args: Vec<OsString>) -> miette::Result<()> {
    let mut child = ProcessCommand::new(&command.executable);
    child
        .args(args)
        .envs(command.env)
        .env("PATH", path_with_existing(&command.path_entries)?)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let status = child
        .status()
        .into_diagnostic()
        .with_context(|| format!("failed to run {}", command.executable.to_string_lossy()))?;
    std::process::exit(status.code().unwrap_or(1));
}

fn path_with_existing(entries: &[PathBuf]) -> miette::Result<OsString> {
    let mut paths = entries.to_vec();
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths).map_err(|err| miette::miette!("failed to construct PATH: {err}"))
}

fn print_json<T: serde::Serialize>(value: &T) -> miette::Result<()> {
    println!("{}", serde_json::to_string_pretty(value).into_diagnostic()?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_cli_shape_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn test_run_requires_args_after_separator() {
        let parsed = Cli::try_parse_from([
            "nan",
            "--install-root",
            "/tmp/fleet",
            "run",
            "tool",
            "python",
            "--",
            "--version",
        ])
        .unwrap();

        let NanCommand::Run { args, .. } = parsed.command else {
            panic!("expected run command");
        };
        assert_eq!(args, vec![OsString::from("--version")]);
    }

    #[test]
    fn test_install_root_is_global() {
        let parsed = Cli::try_parse_from(["nan", "list", "--install-root", "/tmp/fleet"]).unwrap();

        assert_eq!(parsed.install_root, Some(PathBuf::from("/tmp/fleet")));
    }
}
