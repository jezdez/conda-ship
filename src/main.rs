//! cx — lightweight single-binary conda bootstrapper powered by rattler.

use std::{env, path::Path};

use clap::Parser;
use miette::{Context, IntoDiagnostic};
use rattler_conda_types::PrefixRecord;

mod cli;
mod config;
mod exec;
mod install;

use cli::{Cli, Command, LockSource};
use config::{
    EMBEDDED_LOCK, embedded_config, read_metadata, write_condarc, write_frozen, write_metadata,
};

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
        // Commands handled by clap (cx's own subcommands, flags, bare invocation)
        Some("bootstrap") | Some("status") | Some("shell") | Some("help") | Some("--help")
        | Some("-h") | Some("--version") | Some("-V") | None => {
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
                    return cmd_bootstrap(&prefix, force, channel, package, &excludes, lock_source)
                        .await;
                }
                Some(Command::Status { prefix }) => {
                    let prefix = prefix.map(Ok).unwrap_or_else(default_prefix)?;
                    return cmd_status(&prefix);
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
        // Everything else passes through to conda
        Some(_) => {
            let prefix = default_prefix()?;
            ensure_bootstrapped(&prefix).await?;
            let conda_args: Vec<&str> = raw_args[1..].iter().map(|s| s.as_str()).collect();
            if exec::needs_output_filtering(&conda_args) {
                return exec::run_conda_filtered(&prefix, &conda_args);
            }
            return exec::replace_process_with_conda(&prefix, &conda_args);
        }
    }
    Ok(())
}

// ─── Prefix helpers ──────────────────────────────────────────────────────────

fn default_prefix() -> miette::Result<std::path::PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| miette::miette!("could not determine home directory"))?;
    Ok(home.join(".cx"))
}

fn is_bootstrapped(prefix: &Path) -> bool {
    prefix.join("conda-meta").is_dir()
}

async fn ensure_bootstrapped(prefix: &Path) -> miette::Result<()> {
    if !is_bootstrapped(prefix) {
        eprintln!(
            "{} No conda installation found. Bootstrapping now...",
            console::style(">>").cyan().bold()
        );
        let cfg = embedded_config();
        cmd_bootstrap(
            prefix,
            false,
            None,
            None,
            &cfg.exclude,
            LockSource::Embedded,
        )
        .await?;
    }
    Ok(())
}

fn conda_executable(prefix: &Path) -> std::path::PathBuf {
    if cfg!(windows) {
        prefix.join("Scripts").join("conda.exe")
    } else {
        prefix.join("bin").join("conda")
    }
}

// ─── Commands ────────────────────────────────────────────────────────────────

async fn cmd_bootstrap(
    prefix: &Path,
    force: bool,
    channels: Option<Vec<String>>,
    extra_packages: Option<Vec<String>>,
    excludes: &[String],
    lock_source: LockSource,
) -> miette::Result<()> {
    if is_bootstrapped(prefix) && !force {
        eprintln!(
            "{} conda is already bootstrapped at {}",
            console::style("✔").green(),
            prefix.display()
        );
        eprintln!("  Use --force to re-bootstrap.");
        return Ok(());
    }

    if force && prefix.exists() {
        eprintln!(
            "{} Removing existing prefix at {}",
            console::style(">>").cyan().bold(),
            prefix.display()
        );
        std::fs::remove_dir_all(prefix).into_diagnostic()?;
    }

    let cfg = embedded_config();
    let channels = channels.unwrap_or(cfg.channels);
    let mut specs = cfg.packages;
    if let Some(extra) = extra_packages {
        specs.extend(extra);
    }

    eprintln!(
        "{} Bootstrapping conda into {}",
        console::style(">>").cyan().bold(),
        prefix.display()
    );
    eprintln!("   Channels: {}", channels.join(", "));
    eprintln!("   Packages: {}", specs.join(", "));
    if !excludes.is_empty() {
        eprintln!("   Exclude:  {}", excludes.join(", "));
    }

    let lock_content = match &lock_source {
        LockSource::Embedded => {
            if !EMBEDDED_LOCK.is_empty() {
                eprintln!("   Using embedded lockfile");
                Some(EMBEDDED_LOCK.to_string())
            } else {
                None
            }
        }
        LockSource::File(path) => {
            let content = std::fs::read_to_string(path)
                .into_diagnostic()
                .context("failed to read lockfile")?;
            eprintln!("   Using lockfile: {}", path.display());
            Some(content)
        }
        LockSource::None => {
            eprintln!("   Live solve (lockfile disabled)");
            None
        }
    };

    match &lock_content {
        Some(content) => install::from_lockfile(prefix, content, excludes).await?,
        None => install::from_solve(prefix, &channels, &specs, excludes).await?,
    };

    if !excludes.is_empty() {
        write_condarc(prefix)?;
    }

    write_frozen(prefix)?;
    write_metadata(prefix, &channels, &specs, excludes)?;

    eprintln!(
        "\n{} conda bootstrapped successfully!",
        console::style("✔").green().bold()
    );
    eprintln!("   Prefix: {}", prefix.display());
    eprintln!("   Run `cx status` for details.");
    eprintln!("   Use `cx <conda-args>` to run conda commands.");

    Ok(())
}

fn cmd_status(prefix: &Path) -> miette::Result<()> {
    if !is_bootstrapped(prefix) {
        eprintln!("No conda installation found at {}", prefix.display());
        return Ok(());
    }

    let meta = read_metadata(prefix)?;

    println!("cx {}", env!("CARGO_PKG_VERSION"));
    println!("  prefix:   {}", prefix.display());
    println!("  channels: {}", meta.channels.join(", "));
    println!("  packages: {}", meta.packages.join(", "));
    if !meta.excludes.is_empty() {
        println!("  excludes: {}", meta.excludes.join(", "));
    }

    let installed = PrefixRecord::collect_from_prefix::<PrefixRecord>(prefix).into_diagnostic()?;
    println!("  installed: {} packages", installed.len());

    let conda_bin = conda_executable(prefix);
    println!(
        "  conda:    {}",
        if conda_bin.exists() {
            conda_bin.display().to_string()
        } else {
            "(not found)".to_string()
        }
    );

    Ok(())
}

// ─── Disabled commands (shell integration replaced by conda-spawn) ───────────

fn print_disabled_shell_command(command: &str) {
    eprintln!(
        "{} `conda {command}` is not available in cx.",
        console::style("!").yellow().bold()
    );
    eprintln!();
    eprintln!("  cx uses conda-spawn for environment activation.");
    eprintln!("  Instead of `conda activate myenv`, run:");
    eprintln!();
    eprintln!("    {}", console::style("cx shell myenv").green());
    eprintln!();
    eprintln!("  To leave the environment, exit the subshell (Ctrl+D or `exit`).");
    eprintln!();
    eprintln!("  Learn more: https://github.com/conda-incubator/conda-spawn");
    std::process::exit(1);
}

fn print_disabled_init() {
    eprintln!(
        "{} `conda init` is not needed with cx.",
        console::style("!").yellow().bold()
    );
    eprintln!();
    eprintln!("  cx uses conda-spawn, which does not require shell");
    eprintln!("  profile modifications. Just add condabin to your PATH:");
    eprintln!();
    eprintln!(
        "    {}",
        console::style("export PATH=\"$HOME/.cx/condabin:$PATH\"").green()
    );
    eprintln!();
    eprintln!("  Then activate environments with:");
    eprintln!();
    eprintln!("    {}", console::style("cx shell myenv").green());
    eprintln!();
    eprintln!("  Learn more: https://github.com/conda-incubator/conda-spawn");
    std::process::exit(1);
}

// ─── Tracing ─────────────────────────────────────────────────────────────────

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
