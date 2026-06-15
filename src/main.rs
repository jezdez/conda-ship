//! Generic single-binary conda bootstrap runtime powered by rattler.

use std::env;

use miette::IntoDiagnostic;

mod cli;
mod commands;
mod config;
mod constructor_metadata;
mod exec;
mod http;
mod install;
mod policy;
mod runtime_data;
mod tls;

use cli::{Cli, Command};
use commands::{
    bootstrap, ensure_bootstrapped, is_bootstrapped, print_disabled_init,
    print_disabled_shell_command, require_managed_prefix, status, uninstall,
    validate_bootstrap_flags,
};

fn main() -> miette::Result<()> {
    tls::install_default_provider();

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

    ensure_stamped_runtime()?;

    let cli = Cli::parse_runtime();
    if cli.version {
        println!("{} {}", policy::runtime_name(), policy::runtime_version());
        return Ok(());
    }
    let verbosity = cli.verbosity();
    let path = cli.path.as_ref();

    match cli.command {
        Some(Command::Bootstrap {
            force,
            bundle,
            offline,
        }) => {
            let prefix = resolve_install_path(path)?;
            let bundle = bundle.or_else(|| {
                env::var(policy::bundle_env_var())
                    .ok()
                    .filter(|v| !v.is_empty())
                    .map(std::path::PathBuf::from)
            });
            let offline = offline
                || env::var(policy::offline_env_var())
                    .ok()
                    .filter(|v| !v.is_empty())
                    .is_some_and(|v| v != "0" && v.to_lowercase() != "false");

            validate_bootstrap_flags(&bundle)?;

            return bootstrap(&prefix, force, bundle, offline, verbosity).await;
        }
        Some(Command::Status) => {
            let prefix = resolve_install_path(path)?;
            return status(&prefix);
        }
        Some(Command::Uninstall { yes }) => {
            let prefix = resolve_install_path(path)?;
            return uninstall(&prefix, yes, verbosity);
        }
        Some(Command::Shell { env, args }) => {
            let prefix = resolve_install_path(path)?;
            ensure_bootstrapped(&prefix).await?;
            let mut conda_args = vec!["spawn".to_string()];
            if let Some(ref name) = env {
                conda_args.push(name.clone());
            }
            let extra: Vec<String> = args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            conda_args.extend(extra);
            let conda_arg_refs: Vec<&str> = conda_args.iter().map(String::as_str).collect();
            return exec::replace_process_with_conda(&prefix, &conda_arg_refs);
        }
        Some(Command::Help) => {
            Cli::parse_runtime_from([policy::runtime_name(), "--help"]);
        }
        Some(Command::Passthrough(args)) => {
            let prefix = resolve_install_path(path)?;
            let delegate_args: Vec<String> = args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            if policy::delegate() == "conda" {
                let first_arg = delegate_args.first().map(String::as_str);
                match first_arg {
                    Some(command @ ("activate" | "deactivate")) => {
                        print_disabled_shell_command(command);
                    }
                    Some("init") => {
                        print_disabled_init();
                    }
                    _ => {}
                }
            }
            ensure_bootstrapped(&prefix).await?;
            let delegate_arg_refs: Vec<&str> = delegate_args.iter().map(String::as_str).collect();
            if policy::delegate() == "conda" && exec::should_filter_conda_output(&delegate_arg_refs)
            {
                return exec::run_conda_filtered(&prefix, &delegate_arg_refs);
            }
            return exec::replace_process_with_delegate(
                &prefix,
                policy::delegate(),
                &delegate_arg_refs,
            );
        }
        None => {
            let prefix = resolve_install_path(path)?;
            if !is_bootstrapped(&prefix) {
                eprintln!(
                    "{} No conda installation found. Run `{} bootstrap` first.",
                    console::style("!").yellow().bold(),
                    policy::runtime_name()
                );
                std::process::exit(1);
            }
            require_managed_prefix(&prefix, "use")?;
            return exec::replace_process_with_delegate(&prefix, policy::delegate(), &["--help"]);
        }
    }
    Ok(())
}

fn resolve_install_path(path: Option<&std::path::PathBuf>) -> miette::Result<std::path::PathBuf> {
    if let Some(path) = path {
        policy::expand_install_path(path)
    } else {
        policy::default_install_path()
    }
}

fn ensure_stamped_runtime() -> miette::Result<()> {
    if runtime_data::current().stamped
        || env::var_os("CONDA_SHIP_ALLOW_UNSTAMPED_TEMPLATE").is_some()
    {
        return Ok(());
    }

    Err(miette::miette!(
        "{} is a runtime template, not a runnable conda runtime. Build a stamped runtime with `cs build`.",
        policy::display_name(),
    ))
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
