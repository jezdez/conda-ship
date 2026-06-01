use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

#[path = "../runtime_data.rs"]
mod runtime_data;
#[path = "../tls.rs"]
mod tls;

#[path = "cs/artifact.rs"]
mod artifact;
#[path = "cs/bundle.rs"]
mod bundle;
#[path = "cs/diagnostic.rs"]
mod diagnostic;
#[path = "cs/project.rs"]
mod project;
#[cfg(test)]
#[path = "cs/tests.rs"]
mod tests;

use artifact::{build_artifact, dry_run_build_artifact, inspect_artifact, run_artifact};

#[derive(Clone, Default, serde::Deserialize)]
struct ProjectManifest {
    #[serde(default)]
    tool: ToolSection,
}

#[derive(Clone, Default, serde::Deserialize)]
struct ToolSection {
    #[serde(default, rename = "conda-ship")]
    conda_ship: ShipConfig,
}

#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ShipConfig {
    #[serde(default)]
    runtime: Option<String>,
    #[serde(default)]
    delegate: Option<String>,
    #[serde(default)]
    layout: Option<BundleLayout>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default, rename = "source-environment")]
    source_environment: Option<String>,
    #[serde(default, rename = "docs-url")]
    docs_url: Option<String>,
    #[serde(default, rename = "install-scheme")]
    install_scheme: Option<runtime_data::InstallScheme>,
    #[serde(default, rename = "install-name")]
    install_name: Option<String>,
    #[serde(default, rename = "install-method")]
    install_method: Option<String>,
}

#[derive(Clone, Default)]
struct RuntimeStampConfig {
    channels: Vec<String>,
    packages: Vec<String>,
    exclude: Vec<String>,
    delegate: Option<String>,
    docs_url: Option<String>,
    install_scheme: Option<runtime_data::InstallScheme>,
    install_name: Option<String>,
    install_method: Option<String>,
}

#[derive(Parser)]
#[command(name = "cs", about = "Build ready-to-run conda runtimes")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build and stage a ready-to-run runtime
    Build {
        /// Artifact layout to produce
        #[arg(long, value_enum)]
        layout: Option<BundleLayout>,

        /// Runtime name to stage for the generated runtime
        #[arg(long)]
        runtime: Option<String>,

        /// Delegate executable inside the managed prefix
        #[arg(long)]
        delegate: Option<String>,

        /// Optional target label appended to staged artifact names
        #[arg(long)]
        target_label: Option<String>,

        /// Conda platform to bundle/describe (default: current)
        #[arg(long)]
        platform: Option<String>,

        /// Target triple used for artifact naming and template selection
        #[arg(long)]
        target: Option<String>,

        /// Prebuilt generic runtime template binary to stamp
        #[arg(long)]
        template: Option<PathBuf>,

        /// Documentation URL stamped into the generated runtime
        #[arg(long)]
        docs_url: Option<String>,

        /// Install scheme stamped into the generated runtime
        #[arg(long = "install-scheme", value_enum)]
        install_scheme: Option<runtime_data::InstallScheme>,

        /// Install name used inside the install scheme
        #[arg(long)]
        install_name: Option<String>,

        /// Package-manager or installer method that provided the runtime binary
        #[arg(long)]
        install_method: Option<String>,

        /// Output directory for staged artifacts
        #[arg(long, default_value = "dist")]
        out_dir: PathBuf,

        /// Preview the build without downloading, stamping, or writing files
        #[arg(long)]
        dry_run: bool,

        /// Project root (default: auto-detect from current directory)
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// Build and run a staged runtime for local smoke testing
    Run {
        /// Artifact layout to produce before running
        #[arg(long, value_enum)]
        layout: Option<BundleLayout>,

        /// Runtime name to stage for the generated runtime
        #[arg(long)]
        runtime: Option<String>,

        /// Delegate executable inside the managed prefix
        #[arg(long)]
        delegate: Option<String>,

        /// Conda platform to bundle/describe (default: current)
        #[arg(long)]
        platform: Option<String>,

        /// Output directory for staged artifacts
        #[arg(long, default_value = "dist")]
        out_dir: PathBuf,

        /// Prebuilt generic runtime template binary to stamp
        #[arg(long)]
        template: Option<PathBuf>,

        /// Documentation URL stamped into the generated runtime
        #[arg(long)]
        docs_url: Option<String>,

        /// Install scheme stamped into the generated runtime
        #[arg(long = "install-scheme", value_enum)]
        install_scheme: Option<runtime_data::InstallScheme>,

        /// Install name used inside the install scheme
        #[arg(long)]
        install_name: Option<String>,

        /// Package-manager or installer method that provided the runtime binary
        #[arg(long)]
        install_method: Option<String>,

        /// Project root (default: auto-detect from current directory)
        #[arg(long)]
        root: Option<PathBuf>,

        /// Arguments passed to the staged runtime
        #[arg(last = true)]
        args: Vec<OsString>,
    },

    /// Inspect project input and derived runtime packages without writing files
    Inspect {
        /// Conda platform to inspect (default: current)
        #[arg(long)]
        platform: Option<String>,

        /// Emit JSON
        #[arg(long)]
        json: bool,

        /// Project root (default: auto-detect from current directory)
        #[arg(long)]
        root: Option<PathBuf>,
    },
}

impl Command {
    fn name(&self) -> &'static str {
        match self {
            Self::Build { .. } => "build",
            Self::Run { .. } => "run",
            Self::Inspect { .. } => "inspect",
        }
    }
}

const SHIP_STATE_DIR: &str = "target/conda-ship";
const RUNTIME_LOCK_FILE: &str = "runtime.lock";
const BUNDLE_ARCHIVE_FILE: &str = "bundle.tar.zst";
const RUNTIME_TEMPLATE_ENV: &str = "CONDA_SHIP_TEMPLATE";
const REQUIRED_RUNTIME_PACKAGES: &[&str] = &["conda", "conda-spawn", "conda-rattler-solver"];

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
enum BundleLayout {
    /// Binary contains lock/metadata; packages download during bootstrap.
    Online,
    /// Binary is paired with a compressed package bundle.
    External,
    /// Binary contains the compressed package bundle.
    Embedded,
}

impl BundleLayout {
    fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::External => "external",
            Self::Embedded => "embedded",
        }
    }

    fn needs_bundle(self) -> bool {
        matches!(self, Self::External | Self::Embedded)
    }
}

fn main() -> ExitCode {
    tls::install_default_provider();

    let cli = Cli::parse();
    let command = cli.command.name();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if diagnostic::structured_errors_requested() {
                diagnostic::print_structured_error(&error, Some(command), 1);
            } else {
                eprintln!("{error:?}");
            }
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> miette::Result<()> {
    match cli.command {
        Command::Build {
            layout,
            runtime,
            delegate,
            target_label,
            platform,
            target,
            template,
            docs_url,
            install_scheme,
            install_name,
            install_method,
            out_dir,
            dry_run,
            root,
        } => {
            if dry_run {
                dry_run_build_artifact(
                    layout,
                    runtime,
                    delegate,
                    target_label,
                    platform,
                    target,
                    template,
                    docs_url,
                    install_scheme,
                    install_name,
                    install_method,
                    out_dir,
                    root,
                )?;
                return Ok(());
            }
            let output = build_artifact(
                layout,
                runtime,
                delegate,
                target_label,
                platform,
                target,
                template,
                docs_url,
                install_scheme,
                install_name,
                install_method,
                out_dir,
                root,
            )?;
            eprintln!("metadata {}", output.info.display());
            eprintln!("checksums {}", output.checksums.display());
            eprintln!("lock {}", output.lock.display());
            eprintln!("packages {}", output.package_list.display());
            if let Some(bundle) = output.bundle {
                eprintln!("bundle {}", bundle.display());
            }
        }
        Command::Run {
            layout,
            runtime,
            delegate,
            platform,
            out_dir,
            template,
            docs_url,
            install_scheme,
            install_name,
            install_method,
            root,
            args,
        } => run_artifact(
            layout,
            runtime,
            delegate,
            platform,
            out_dir,
            template,
            docs_url,
            install_scheme,
            install_name,
            install_method,
            root,
            args,
        )?,
        Command::Inspect {
            platform,
            json,
            root,
        } => inspect_artifact(platform, json, root)?,
    }
    Ok(())
}
