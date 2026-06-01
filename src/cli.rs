//! Command-line interface definitions.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{CommandFactory, FromArgMatches, Parser};

use crate::policy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    Normal,
    Verbose,
    Quiet,
}

#[derive(Debug, Parser)]
#[clap(
    disable_help_subcommand = true,
    disable_version_flag = true,
    allow_external_subcommands = true
)]
pub struct Cli {
    /// Show the runtime version
    #[clap(long, short = 'V', global = true)]
    pub version: bool,

    /// Increase output detail
    #[clap(long, short, global = true)]
    pub verbose: bool,

    /// Suppress non-essential output
    #[clap(long, short, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Runtime install path to use instead of the stamped default
    #[clap(long, global = true, value_name = "PATH")]
    pub path: Option<PathBuf>,

    #[clap(subcommand)]
    pub command: Option<Command>,
}

impl Cli {
    pub fn parse_runtime() -> Self {
        Self::parse_runtime_from(std::env::args_os())
    }

    pub fn parse_runtime_from<I, T>(args: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let matches = Self::runtime_command().get_matches_from(args);
        Self::from_arg_matches(&matches).unwrap_or_else(|err| err.exit())
    }

    pub fn verbosity(&self) -> Verbosity {
        if self.verbose {
            Verbosity::Verbose
        } else if self.quiet {
            Verbosity::Quiet
        } else {
            Verbosity::Normal
        }
    }

    fn runtime_command() -> clap::Command {
        Self::command()
            .name(policy::runtime_name())
            .about("Single-binary conda runtime")
            .long_about(
                "This runtime installs a conda environment from a stamped lockfile.\n\n\
                After bootstrap, commands are passed through to the configured delegate executable.",
            )
            .after_help(runtime_after_help())
    }
}

fn runtime_after_help() -> String {
    let name = policy::runtime_name();
    let delegate = policy::delegate();
    let quick_start = if delegate == "conda" {
        format!(
            "{name} bootstrap                           Install conda into {path}\n  \
            {name} create -n myenv python=3.12 numpy   Create an environment\n  \
            {name} shell myenv                         Activate (spawns a subshell)\n  \
            exit                                   Leave the environment",
            path = policy::install_path_for_display(),
        )
    } else {
        format!(
            "{name} bootstrap                           Install conda into {path}\n  \
            {name} --help                              Show {delegate} help after bootstrap",
            path = policy::install_path_for_display(),
        )
    };
    let pass_through_examples = if delegate == "conda" {
        format!(
            "Any command not listed above is passed through to conda:\n  \
            {name} install, {name} remove, {name} list, {name} env, {name} info, {name} config, ..."
        )
    } else {
        format!(
            "Any command not listed above is passed through to {delegate}:\n  \
            {name} <{delegate}-args> ..."
        )
    };
    format!(
        "\x1b[1;4mQuick start:\x1b[0m\n\n  \
        {quick_start}\n\n\
        \x1b[1;4mManagement:\x1b[0m\n\n  \
        {name} status                              Show installation details\n  \
        {name} uninstall                           Remove the install path and environments\n\n\
        \x1b[1;4mPass-through:\x1b[0m\n\n  \
        {pass_through_examples}\n\n\
        \x1b[4mDocs:\x1b[0m {docs_url}",
        docs_url = policy::docs_url(),
    )
}

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Bootstrap a fresh conda installation into the install path
    Bootstrap {
        /// Force re-bootstrap even if the install path already exists
        #[clap(long)]
        force: bool,

        /// Directory containing pre-downloaded .conda / .tar.bz2 package archives
        /// for offline installation
        #[clap(long)]
        bundle: Option<PathBuf>,

        /// Disable network access (install only from cache or --bundle)
        #[clap(long)]
        offline: bool,
    },

    /// Print runtime status (install path, channels, packages)
    Status,

    /// Activate an environment via subshell (alias for conda spawn)
    Shell {
        /// Name of the environment to activate
        env: Option<String>,

        /// Additional arguments passed to conda spawn
        #[clap(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },

    /// Uninstall this runtime: remove the install path and environments
    Uninstall {
        /// Skip confirmation prompt
        #[clap(long, short)]
        yes: bool,
    },

    /// Show this help message
    Help,

    #[clap(external_subcommand)]
    Passthrough(Vec<OsString>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use rstest::rstest;

    #[test]
    fn test_parse_bootstrap_defaults() {
        let cli = Cli::parse_from(["cs-template", "bootstrap"]);
        assert!(!cli.version);
        assert!(cli.path.is_none());
        assert_matches!(
            cli.command,
            Some(Command::Bootstrap {
                force: false,
                bundle: None,
                offline: false,
            })
        );
    }

    #[test]
    fn test_parse_global_path_for_bootstrap() {
        let cli = Cli::parse_from(["cs-template", "--path", "/tmp/test", "bootstrap"]);
        assert_eq!(cli.path.as_deref(), Some(std::path::Path::new("/tmp/test")));
        assert_matches!(cli.command, Some(Command::Bootstrap { force: false, .. }));
    }

    #[test]
    fn test_parse_status() {
        let cli = Cli::parse_from(["cs-template", "status"]);
        assert_matches!(cli.command, Some(Command::Status));
    }

    #[test]
    fn test_parse_status_with_global_path() {
        let cli = Cli::parse_from(["cs-template", "--path", "/opt/cs-template", "status"]);
        assert_eq!(
            cli.path.as_deref(),
            Some(std::path::Path::new("/opt/cs-template"))
        );
        assert_matches!(cli.command, Some(Command::Status));
    }

    #[test]
    fn test_parse_shell_with_env() {
        let cli = Cli::parse_from(["cs-template", "shell", "myenv"]);
        assert_matches!(
            cli.command,
            Some(Command::Shell { env: Some(ref e), ref args }) if e == "myenv" && args.is_empty()
        );
    }

    #[test]
    fn test_parse_shell_no_env() {
        let cli = Cli::parse_from(["cs-template", "shell"]);
        assert_matches!(
            cli.command,
            Some(Command::Shell {
                env: None,
                ref args
            }) if args.is_empty()
        );
    }

    #[test]
    fn test_parse_shell_extra_args() {
        let cli = Cli::parse_from(["cs-template", "shell", "myenv", "--", "python", "-q"]);
        assert_matches!(
            cli.command,
            Some(Command::Shell {
                env: Some(ref e),
                ref args
            }) if e == "myenv"
                && args == &vec![OsString::from("python"), OsString::from("-q")]
        );
    }

    #[test]
    fn test_parse_uninstall_yes() {
        let cli = Cli::parse_from(["cs-template", "uninstall", "--yes"]);
        assert_matches!(cli.command, Some(Command::Uninstall { yes: true }));
    }

    #[test]
    fn test_parse_uninstall_with_global_path() {
        let cli = Cli::parse_from([
            "cs-template",
            "--path",
            "/opt/cs-template",
            "uninstall",
            "-y",
        ]);
        assert_eq!(
            cli.path.as_deref(),
            Some(std::path::Path::new("/opt/cs-template"))
        );
        assert_matches!(cli.command, Some(Command::Uninstall { yes: true, .. }));
    }

    #[test]
    fn test_parse_global_path_for_passthrough() {
        let cli = Cli::parse_from([
            "cs-template",
            "--path",
            "/opt/cs-template",
            "install",
            "numpy",
        ]);
        assert_eq!(
            cli.path.as_deref(),
            Some(std::path::Path::new("/opt/cs-template"))
        );
        assert_matches!(
            cli.command,
            Some(Command::Passthrough(ref args))
                if args == &vec![OsString::from("install"), OsString::from("numpy")]
        );
    }

    #[test]
    fn test_parse_no_args() {
        let cli = Cli::parse_from(["cs-template"]);
        assert!(
            cli.command.is_none(),
            "bare `runtime` should have no command"
        );
    }

    #[test]
    fn test_parse_verbose_flag() {
        let cli = Cli::parse_from(["cs-template", "--verbose", "bootstrap"]);
        assert_eq!(cli.verbosity(), Verbosity::Verbose);
    }

    #[test]
    fn test_parse_quiet_flag() {
        let cli = Cli::parse_from(["cs-template", "--quiet", "bootstrap"]);
        assert_eq!(cli.verbosity(), Verbosity::Quiet);
    }

    #[test]
    fn test_parse_short_verbose_flag() {
        let cli = Cli::parse_from(["cs-template", "-v", "status"]);
        assert_eq!(cli.verbosity(), Verbosity::Verbose);
    }

    #[test]
    fn test_parse_short_quiet_flag() {
        let cli = Cli::parse_from(["cs-template", "-q", "status"]);
        assert_eq!(cli.verbosity(), Verbosity::Quiet);
    }

    #[test]
    fn test_parse_version_flag() {
        let cli = Cli::parse_from(["cs-template", "--version"]);
        assert!(cli.version);
    }

    #[test]
    fn test_parse_short_version_flag() {
        let cli = Cli::parse_from(["cs-template", "-V"]);
        assert!(cli.version);
    }

    #[test]
    fn test_parse_no_verbosity_flags() {
        let cli = Cli::parse_from(["cs-template", "bootstrap"]);
        assert_eq!(cli.verbosity(), Verbosity::Normal);
    }

    #[test]
    fn test_verbose_quiet_conflict() {
        let result = Cli::try_parse_from(["cs-template", "--verbose", "--quiet", "bootstrap"]);
        assert!(result.is_err(), "--verbose and --quiet should conflict");
    }

    #[rstest]
    #[case::bundle_only(&["cs-template", "bootstrap", "--bundle", "/tmp/pkgs"], Some("/tmp/pkgs"), false)]
    #[case::offline_only(&["cs-template", "bootstrap", "--offline"], None, true)]
    #[case::both(&["cs-template", "bootstrap", "--bundle", "/p", "--offline"], Some("/p"), true)]
    #[case::defaults(&["cs-template", "bootstrap"], None, false)]
    fn test_parse_bootstrap_offline_flags(
        #[case] args: &[&str],
        #[case] expected_bundle: Option<&str>,
        #[case] expected_offline: bool,
    ) {
        let cli = Cli::parse_from(args);
        match cli.command {
            Some(Command::Bootstrap {
                bundle, offline, ..
            }) => {
                assert_eq!(bundle.as_deref(), expected_bundle.map(std::path::Path::new));
                assert_eq!(offline, expected_offline);
            }
            other => panic!("expected Bootstrap, got {other:?}"),
        }
    }
}
