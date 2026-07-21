//! Generic single-binary conda bootstrap runtime powered by rattler.

use std::env;

use miette::IntoDiagnostic;

mod bootstrap_lock;
mod bootstrap_state;
mod commands;
mod config;
mod constructor_metadata;
mod exec;
mod hash;
mod http;
mod install;
mod policy;
mod runtime_data;
mod tls;

use commands::ensure_bootstrapped;

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

    let prefix = policy::install_path()?;
    ensure_bootstrapped(&prefix).await?;

    let args: Vec<_> = env::args_os().skip(1).collect();
    exec::replace_process_with_delegate(&prefix, policy::delegate_executable(), &args)
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
