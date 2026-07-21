//! Configuration and runtime metadata management.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use miette::{Context, IntoDiagnostic};

use crate::bootstrap_state::BootstrapPhase;
use crate::{policy, runtime_data};

pub use crate::runtime_data::RuntimeConfig;

static EMBEDDED_RUNTIME_CONFIG: LazyLock<RuntimeConfig> = LazyLock::new(|| {
    let stamped = &runtime_data::current().header.runtime_config;
    if !stamped.is_empty() {
        return stamped.clone();
    }

    RuntimeConfig::default()
});

/// Return the runtime package metadata embedded at build time.
pub fn embedded_config() -> &'static RuntimeConfig {
    &EMBEDDED_RUNTIME_CONFIG
}

/// Return the rattler-lock runtime lock stamped onto the current artifact.
pub fn embedded_lock() -> Option<&'static str> {
    let lock = runtime_data::current().header.runtime_lock.as_str();
    (!lock.is_empty()).then_some(lock)
}

/// Return the embedded compressed package bundle stamped onto the current
/// artifact, when present.
pub fn embedded_bundle() -> Option<&'static runtime_data::EmbeddedBundle> {
    runtime_data::current().bundle.as_ref()
}

// Prefix metadata.

const PREFIX_METADATA_SCHEMA_VERSION: u8 = 1;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct PrefixMetadata {
    pub schema_version: u8,
    pub display_name: String,
    pub install_name: String,
    pub metadata_file: String,
    pub version: String,
    pub channels: Vec<String>,
    pub packages: Vec<String>,
    #[serde(default = "ready_bootstrap_phase")]
    pub(crate) bootstrap_state: BootstrapPhase,
}

fn ready_bootstrap_phase() -> BootstrapPhase {
    BootstrapPhase::Ready
}

pub(crate) fn metadata_path(prefix: &Path) -> PathBuf {
    prefix.join(policy::metadata_file())
}

fn temporary_metadata_path(prefix: &Path) -> PathBuf {
    metadata_path(prefix).with_extension("tmp")
}

fn remove_regular_file_if_present(path: &Path) -> miette::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(miette::miette!(
                "runtime metadata is not a regular file: {}",
                policy::path_for_display(path)
            ))
        }
        Ok(_) => std::fs::remove_file(path)
            .into_diagnostic()
            .with_context(|| {
                format!(
                    "failed to remove runtime metadata at {}",
                    policy::path_for_display(path)
                )
            }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).into_diagnostic(),
    }
}

pub(crate) fn invalidate_metadata(prefix: &Path) -> miette::Result<()> {
    remove_regular_file_if_present(&metadata_path(prefix))?;
    remove_regular_file_if_present(&temporary_metadata_path(prefix))
}

pub fn write_metadata(
    prefix: &Path,
    channels: &[String],
    packages: &[String],
) -> miette::Result<()> {
    let meta = PrefixMetadata {
        schema_version: PREFIX_METADATA_SCHEMA_VERSION,
        display_name: policy::display_name().to_string(),
        install_name: policy::install_name().to_string(),
        metadata_file: policy::metadata_file().to_string(),
        version: policy::runtime_version().to_string(),
        channels: channels.to_vec(),
        packages: packages.to_vec(),
        bootstrap_state: BootstrapPhase::Ready,
    };
    let path = metadata_path(prefix);
    let temporary_path = temporary_metadata_path(prefix);
    remove_regular_file_if_present(&temporary_path)?;
    let mut temporary = std::fs::File::create(&temporary_path)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to create temporary runtime metadata at {}",
                policy::path_for_display(&temporary_path)
            )
        })?;
    serde_json::to_writer_pretty(&mut temporary, &meta)
        .into_diagnostic()
        .context("failed to render runtime metadata")?;
    temporary
        .write_all(b"\n")
        .into_diagnostic()
        .context("failed to write runtime metadata")?;
    temporary
        .sync_all()
        .into_diagnostic()
        .context("failed to sync runtime metadata")?;
    drop(temporary);

    remove_regular_file_if_present(&path)?;
    std::fs::rename(&temporary_path, &path)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to commit runtime metadata at {}",
                policy::path_for_display(&path)
            )
        })?;
    Ok(())
}

pub fn read_metadata(prefix: &Path) -> miette::Result<PrefixMetadata> {
    let path = metadata_path(prefix);
    let data = std::fs::read_to_string(&path)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to read runtime metadata at {}",
                policy::path_for_display(&path)
            )
        })?;
    serde_json::from_str(&data)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to parse runtime metadata at {}",
                policy::path_for_display(&path)
            )
        })
}

pub(crate) fn validate_metadata_identity(meta: &PrefixMetadata) -> miette::Result<()> {
    if meta.schema_version != PREFIX_METADATA_SCHEMA_VERSION {
        return Err(miette::miette!(
            "unsupported runtime metadata schema version: {}",
            meta.schema_version
        ));
    }
    if meta.display_name != policy::display_name() {
        return Err(miette::miette!(
            "runtime metadata belongs to {}, not {}",
            meta.display_name,
            policy::display_name()
        ));
    }
    if meta.install_name != policy::install_name() {
        return Err(miette::miette!(
            "runtime metadata install name is {}, expected {}",
            meta.install_name,
            policy::install_name()
        ));
    }
    if meta.metadata_file != policy::metadata_file() {
        return Err(miette::miette!(
            "runtime metadata file is {}, expected {}",
            meta.metadata_file,
            policy::metadata_file()
        ));
    }
    Ok(())
}

pub(crate) fn validate_metadata_ready(meta: &PrefixMetadata) -> miette::Result<()> {
    validate_metadata_identity(meta)?;
    if meta.bootstrap_state != BootstrapPhase::Ready {
        return Err(miette::miette!(
            "runtime metadata is not a ready bootstrap commit"
        ));
    }
    Ok(())
}

// conda-meta/frozen (CEP 22).

/// Write a CEP 22 frozen marker to protect the base prefix from accidental
/// modification. Users should create named environments for their work and
/// let the distribution decide how base updates are performed.
/// See: <https://conda.org/learn/ceps/cep-0022/>
pub fn write_frozen(prefix: &Path) -> miette::Result<()> {
    let frozen_path = prefix.join("conda-meta").join("frozen");
    let contents = serde_json::json!({ "message": policy::frozen_message() });
    std::fs::create_dir_all(prefix.join("conda-meta")).into_diagnostic()?;
    std::fs::write(
        &frozen_path,
        serde_json::to_string_pretty(&contents).into_diagnostic()?,
    )
    .into_diagnostic()?;
    eprintln!("   Wrote {}", policy::path_for_display(&frozen_path));
    Ok(())
}

pub fn write_condarc(prefix: &Path, contents: &str) -> miette::Result<()> {
    let condarc_path = prefix.join(".condarc");
    std::fs::create_dir_all(prefix).into_diagnostic()?;
    std::fs::write(&condarc_path, contents).into_diagnostic()?;
    eprintln!("   Wrote {}", policy::path_for_display(&condarc_path));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_embedded_config_parses() {
        let config = embedded_config();
        assert!(
            config.channels.is_empty(),
            "unstamped templates should not carry channel defaults"
        );
        assert!(
            config.packages.is_empty(),
            "unstamped templates should not carry package defaults"
        );
    }

    #[test]
    fn test_embedded_config_snapshot() {
        let config = embedded_config();
        insta::assert_yaml_snapshot!(
            "embedded_config",
            serde_json::json!({
                "channels": config.channels,
                "packages": config.packages,
                "condarc": config.condarc,
                "freeze_base": config.freeze_base,
            })
        );
    }

    #[test]
    fn test_write_and_read_metadata_roundtrip() {
        let tmp = TempDir::new().unwrap();

        let channels = vec!["conda-forge".to_string()];
        let packages = vec!["python".to_string(), "conda".to_string()];

        write_metadata(tmp.path(), &channels, &packages).unwrap();

        let meta = read_metadata(tmp.path()).unwrap();
        assert_eq!(meta.schema_version, PREFIX_METADATA_SCHEMA_VERSION);
        assert_eq!(meta.display_name, policy::display_name());
        assert_eq!(meta.install_name, policy::install_name());
        assert_eq!(meta.metadata_file, policy::metadata_file());
        assert_eq!(meta.channels, channels);
        assert_eq!(meta.packages, packages);
        assert_eq!(meta.bootstrap_state, BootstrapPhase::Ready);
    }

    #[test]
    fn test_metadata_without_bootstrap_state_is_a_ready_commit() {
        let metadata = serde_json::json!({
            "schema_version": PREFIX_METADATA_SCHEMA_VERSION,
            "display_name": policy::display_name(),
            "install_name": policy::install_name(),
            "metadata_file": policy::metadata_file(),
            "version": policy::runtime_version(),
            "channels": [],
            "packages": [],
        });

        let parsed: PrefixMetadata = serde_json::from_value(metadata).unwrap();

        assert_eq!(parsed.bootstrap_state, BootstrapPhase::Ready);
        validate_metadata_ready(&parsed).unwrap();
    }

    #[test]
    fn test_write_metadata_includes_version() {
        let tmp = TempDir::new().unwrap();

        write_metadata(tmp.path(), &[], &[]).unwrap();

        let meta = read_metadata(tmp.path()).unwrap();
        assert_eq!(
            meta.version,
            env!("CARGO_PKG_VERSION"),
            "metadata version should match crate version"
        );
    }

    #[test]
    fn test_write_condarc_preserves_exact_contents() {
        let tmp = TempDir::new().unwrap();
        let expected = "# downstream policy\nchannels:\n  - conda-forge\n";

        write_condarc(tmp.path(), expected).unwrap();

        let contents = std::fs::read_to_string(tmp.path().join(".condarc")).unwrap();
        assert_eq!(contents, expected);
    }

    #[test]
    fn test_write_frozen_snapshot() {
        let tmp = TempDir::new().unwrap();
        write_frozen(tmp.path()).unwrap();

        let contents =
            std::fs::read_to_string(tmp.path().join("conda-meta").join("frozen")).unwrap();
        insta::assert_snapshot!("frozen", contents);
    }
}
