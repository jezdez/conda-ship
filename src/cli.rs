//! Command-line interface definitions.

use std::path::PathBuf;

use clap::Parser;

const AFTER_HELP: &str = "\x1b[1;4mQuick start:\x1b[0m

  cx bootstrap                           Install conda into ~/.cx
  cx create -n myenv python=3.12 numpy   Create an environment
  cx shell myenv                         Activate (spawns a subshell)
  exit                                   Leave the environment

\x1b[1;4mPass-through:\x1b[0m

  Any command not listed above is passed through to conda:
  cx install, cx remove, cx list, cx env, cx info, cx config, ...

\x1b[4mDocs:\x1b[0m https://jezdez.github.io/conda-express/";

#[derive(Debug, Parser)]
#[clap(
    name = "cx",
    about = "Lightweight single-binary conda bootstrapper powered by rattler",
    long_about = "cx (conda-express) is a lightweight, single-binary bootstrapper for conda.\n\n\
        It installs a minimal conda environment from an embedded lockfile in seconds,\n\
        uses conda-rattler-solver instead of libmamba, and conda-spawn for activation.",
    version,
    after_help = AFTER_HELP,
    allow_external_subcommands = true,
)]
pub struct Cli {
    #[clap(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Bootstrap a fresh conda installation into the prefix
    Bootstrap {
        /// Force re-bootstrap even if the prefix already exists
        #[clap(long)]
        force: bool,

        /// Target prefix directory (default: ~/.cx)
        #[clap(long)]
        prefix: Option<PathBuf>,

        /// Channels to use (default: conda-forge)
        #[clap(short, long)]
        channel: Option<Vec<String>>,

        /// Additional packages to install
        #[clap(short, long)]
        package: Option<Vec<String>>,

        /// Packages to exclude from installation (along with their exclusive
        /// dependencies). Defaults to conda-libmamba-solver.
        #[clap(short, long)]
        exclude: Option<Vec<String>>,

        /// Disable default exclusions (install everything including conda-libmamba-solver)
        #[clap(long)]
        no_exclude: bool,

        /// Ignore the embedded lockfile and perform a live solve instead
        #[clap(long)]
        no_lock: bool,

        /// Use an external lockfile instead of the embedded one
        #[clap(long)]
        lockfile: Option<PathBuf>,
    },

    /// Print cx status (prefix, channels, packages, excludes)
    Status {
        /// Target prefix directory (default: ~/.cx)
        #[clap(long)]
        prefix: Option<PathBuf>,
    },

    /// Activate an environment via subshell (alias for conda spawn)
    Shell {
        /// Name of the environment to activate
        env: Option<String>,
    },

    /// Show this help message
    Help,
}

/// Where to source the lockfile for bootstrap.
pub enum LockSource {
    /// Use the lockfile embedded at compile time.
    Embedded,
    /// Use an external lockfile from disk.
    File(PathBuf),
    /// Skip the lockfile entirely and do a live solve.
    None,
}
