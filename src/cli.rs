//! Command-line interface definitions.

use std::path::PathBuf;

use clap::Parser;

const AFTER_HELP: &str = "\x1b[1;4mQuick start:\x1b[0m

  cx bootstrap                           Install conda into ~/.cx
  cx create -n myenv python=3.12 numpy   Create an environment
  cx shell myenv                         Activate (spawns a subshell)
  exit                                   Leave the environment

\x1b[1;4mManagement:\x1b[0m

  cx status                              Show installation details
  cx uninstall                           Remove cx, conda, and all environments

\x1b[1;4mPass-through:\x1b[0m

  Any command not listed above is passed through to conda:
  cx install, cx remove, cx list, cx env, cx info, cx config, ...

\x1b[4mDocs:\x1b[0m https://jezdez.github.io/conda-express/";

#[derive(Debug, Parser)]
#[clap(
    name = "cx",
    about = "Lightweight single-binary conda bootstrapper powered by rattler",
    disable_help_subcommand = true,
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

    /// Uninstall cx: remove the conda prefix, environments, and optionally the cx binary
    Uninstall {
        /// Target prefix directory (default: ~/.cx)
        #[clap(long)]
        prefix: Option<PathBuf>,

        /// Skip confirmation prompt
        #[clap(long, short)]
        yes: bool,
    },

    /// Show this help message
    Help,
}

pub enum LockSource {
    Embedded,
    File(PathBuf),
    None,
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;

    #[test]
    fn test_parse_bootstrap_defaults() {
        let cli = Cli::parse_from(["cx", "bootstrap"]);
        assert_matches!(
            cli.command,
            Some(Command::Bootstrap {
                force: false,
                prefix: None,
                channel: None,
                package: None,
                exclude: None,
                no_exclude: false,
                no_lock: false,
                lockfile: None,
            })
        );
    }

    #[test]
    fn test_parse_bootstrap_force_prefix() {
        let cli = Cli::parse_from(["cx", "bootstrap", "--force", "--prefix", "/tmp/test"]);
        assert_matches!(
            cli.command,
            Some(Command::Bootstrap { force: true, prefix: Some(ref p), .. })
                if p == std::path::Path::new("/tmp/test")
        );
    }

    #[test]
    fn test_parse_bootstrap_channels_packages() {
        let cli = Cli::parse_from(["cx", "bootstrap", "-c", "main", "-p", "numpy"]);
        assert_matches!(
            cli.command,
            Some(Command::Bootstrap { channel: Some(ref c), package: Some(ref p), .. })
                if c == &vec!["main".to_string()] && p == &vec!["numpy".to_string()]
        );
    }

    #[test]
    fn test_parse_bootstrap_multiple_channels_packages() {
        let cli = Cli::parse_from([
            "cx",
            "bootstrap",
            "-c",
            "conda-forge",
            "-c",
            "defaults",
            "-p",
            "numpy",
            "-p",
            "scipy",
        ]);
        assert_matches!(
            cli.command,
            Some(Command::Bootstrap { channel: Some(ref c), package: Some(ref p), .. })
                if c == &vec!["conda-forge".to_string(), "defaults".to_string()]
                    && p == &vec!["numpy".to_string(), "scipy".to_string()]
        );
    }

    #[test]
    fn test_parse_bootstrap_exclude() {
        let cli = Cli::parse_from(["cx", "bootstrap", "-e", "conda-libmamba-solver"]);
        assert_matches!(
            cli.command,
            Some(Command::Bootstrap { exclude: Some(ref e), no_exclude: false, .. })
                if e == &vec!["conda-libmamba-solver".to_string()]
        );
    }

    #[test]
    fn test_parse_bootstrap_no_exclude() {
        let cli = Cli::parse_from(["cx", "bootstrap", "--no-exclude"]);
        assert_matches!(
            cli.command,
            Some(Command::Bootstrap {
                exclude: None,
                no_exclude: true,
                ..
            })
        );
    }

    #[test]
    fn test_parse_bootstrap_no_lock() {
        let cli = Cli::parse_from(["cx", "bootstrap", "--no-lock"]);
        assert_matches!(
            cli.command,
            Some(Command::Bootstrap {
                no_lock: true,
                lockfile: None,
                ..
            })
        );
    }

    #[test]
    fn test_parse_bootstrap_lockfile() {
        let cli = Cli::parse_from(["cx", "bootstrap", "--lockfile", "/tmp/my.lock"]);
        assert_matches!(
            cli.command,
            Some(Command::Bootstrap { no_lock: false, lockfile: Some(ref p), .. })
                if p == std::path::Path::new("/tmp/my.lock")
        );
    }

    #[test]
    fn test_parse_status() {
        let cli = Cli::parse_from(["cx", "status"]);
        assert_matches!(cli.command, Some(Command::Status { prefix: None }));
    }

    #[test]
    fn test_parse_status_with_prefix() {
        let cli = Cli::parse_from(["cx", "status", "--prefix", "/opt/cx"]);
        assert_matches!(
            cli.command,
            Some(Command::Status { prefix: Some(ref p) })
                if p == std::path::Path::new("/opt/cx")
        );
    }

    #[test]
    fn test_parse_shell_with_env() {
        let cli = Cli::parse_from(["cx", "shell", "myenv"]);
        assert_matches!(
            cli.command,
            Some(Command::Shell { env: Some(ref e) }) if e == "myenv"
        );
    }

    #[test]
    fn test_parse_shell_no_env() {
        let cli = Cli::parse_from(["cx", "shell"]);
        assert_matches!(cli.command, Some(Command::Shell { env: None }));
    }

    #[test]
    fn test_parse_uninstall_yes() {
        let cli = Cli::parse_from(["cx", "uninstall", "--yes"]);
        assert_matches!(
            cli.command,
            Some(Command::Uninstall {
                yes: true,
                prefix: None
            })
        );
    }

    #[test]
    fn test_parse_uninstall_with_prefix() {
        let cli = Cli::parse_from(["cx", "uninstall", "--prefix", "/opt/cx", "-y"]);
        assert_matches!(
            cli.command,
            Some(Command::Uninstall { yes: true, prefix: Some(ref p) })
                if p == std::path::Path::new("/opt/cx")
        );
    }

    #[test]
    fn test_parse_no_args() {
        let cli = Cli::parse_from(["cx"]);
        assert!(cli.command.is_none(), "bare `cx` should have no command");
    }
}
