//! cx — lightweight single-binary conda bootstrapper powered by rattler.

use std::env;

use clap::Parser;
use miette::IntoDiagnostic;

mod cli;
mod commands;
mod config;
mod exclude;
mod exec;
mod install;

use cli::{Cli, Command, LockSource};
use commands::{
    bootstrap, default_prefix, ensure_bootstrapped, is_bootstrapped, print_disabled_init,
    print_disabled_shell_command, status, uninstall,
};
use config::embedded_config;

fn main() -> miette::Result<()> {
    let num_cores = std::thread::available_parallelism()
        .map_or(2, std::num::NonZero::get)
        .max(2);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(num_cores / 2)
        .max_blocking_threads(num_cores)
        .enable_all()
        .build()
        .into_diagnostic()?;

    runtime.block_on(async_main())
}

async fn async_main() -> miette::Result<()> {
    init_tracing()?;

    let raw_args: Vec<String> = env::args().collect();
    let first_arg = raw_args.get(1).map(|s| s.as_str());

    match first_arg {
        Some("activate") | Some("deactivate") => {
            print_disabled_shell_command(first_arg.unwrap());
        }
        Some("init") => {
            print_disabled_init();
        }
        Some("bootstrap") | Some("status") | Some("shell") | Some("uninstall") | Some("help")
        | Some("--help") | Some("-h") | Some("--version") | Some("-V") | None => {
            let cli = Cli::parse();
            match cli.command {
                Some(Command::Bootstrap {
                    force,
                    prefix,
                    channel,
                    package,
                    exclude,
                    no_exclude,
                    no_lock,
                    lockfile,
                }) => {
                    let prefix = prefix.map(Ok).unwrap_or_else(default_prefix)?;
                    let excludes = if no_exclude {
                        vec![]
                    } else {
                        exclude.unwrap_or_else(|| embedded_config().exclude)
                    };
                    let lock_source = if no_lock {
                        LockSource::None
                    } else if let Some(path) = lockfile {
                        LockSource::File(path)
                    } else {
                        LockSource::Embedded
                    };
                    return bootstrap(&prefix, force, channel, package, &excludes, lock_source)
                        .await;
                }
                Some(Command::Status { prefix }) => {
                    let prefix = prefix.map(Ok).unwrap_or_else(default_prefix)?;
                    return status(&prefix);
                }
                Some(Command::Uninstall { prefix, yes }) => {
                    let prefix = prefix.map(Ok).unwrap_or_else(default_prefix)?;
                    return uninstall(&prefix, yes);
                }
                Some(Command::Shell { env }) => {
                    let prefix = default_prefix()?;
                    ensure_bootstrapped(&prefix).await?;
                    let mut conda_args = vec!["spawn"];
                    if let Some(ref name) = env {
                        conda_args.push(name);
                    }
                    let extra: Vec<&str> = raw_args[2..]
                        .iter()
                        .skip(env.is_some() as usize)
                        .map(|s| s.as_str())
                        .collect();
                    conda_args.extend(extra);
                    return exec::replace_process_with_conda(&prefix, &conda_args);
                }
                Some(Command::Help) => {
                    Cli::parse_from(["cx", "--help"]);
                }
                None => {
                    let prefix = default_prefix()?;
                    if !is_bootstrapped(&prefix) {
                        eprintln!(
                            "{} No conda installation found. Run `cx bootstrap` first.",
                            console::style("!").yellow().bold()
                        );
                        std::process::exit(1);
                    }
                    return exec::replace_process_with_conda(&prefix, &["--help"]);
                }
            }
        }
        Some(_) => {
            let prefix = default_prefix()?;
            ensure_bootstrapped(&prefix).await?;
            let conda_args: Vec<&str> = raw_args[1..].iter().map(|s| s.as_str()).collect();
            if exec::should_filter_conda_output(&conda_args) {
                return exec::run_conda_filtered(&prefix, &conda_args);
            }
            return exec::replace_process_with_conda(&prefix, &conda_args);
        }
    }
    Ok(())
}

fn init_tracing() -> miette::Result<()> {
    use tracing_subscriber::{EnvFilter, filter::LevelFilter, util::SubscriberInitExt};

    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::WARN.into())
        .from_env()
        .into_diagnostic()?;

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .without_time()
        .finish()
        .try_init()
        .into_diagnostic()?;

    Ok(())
}
