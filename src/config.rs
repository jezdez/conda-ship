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

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PendingExecutablePhase {
    Staged,
    Ready,
    Replacing,
    Cleanup,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PendingExecutableUpdate {
    pub phase: PendingExecutablePhase,
    pub version: String,
    #[serde(rename = "build-number")]
    pub build_number: u64,
    pub executable_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ExecutableUpdateMetadata {
    #[serde(default)]
    pub executable: PathBuf,
    #[serde(default)]
    pub ownership: runtime_data::UpdateOwnership,
    #[serde(default)]
    pub artifact_name: String,
    pub channel: String,
    pub package: String,
    #[serde(default, rename = "build-number")]
    pub build_number: u64,
    #[serde(default)]
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingExecutableUpdate>,
}

impl ExecutableUpdateMetadata {
    fn from_config(update: &runtime_data::RuntimeUpdateConfig) -> Self {
        Self {
            executable: PathBuf::new(),
            ownership: update.ownership,
            artifact_name: String::new(),
            channel: update.channel.clone(),
            package: update.package.clone(),
            build_number: update.build_number,
            sha256: String::new(),
            instruction: update.instruction.clone(),
            pending: None,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PrefixMetadata {
    pub schema_version: u8,
    pub display_name: String,
    pub install_name: String,
    pub metadata_file: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegate_executable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_sha256: Option<String>,
    pub channels: Vec<String>,
    pub packages: Vec<String>,
    #[serde(default = "ready_bootstrap_phase")]
    pub(crate) bootstrap_state: BootstrapPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) update: Option<ExecutableUpdateMetadata>,
}

fn ready_bootstrap_phase() -> BootstrapPhase {
    BootstrapPhase::Ready
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PrefixMetadataIdentity<'a> {
    pub display_name: &'a str,
    pub install_name: &'a str,
    pub metadata_file: &'a str,
    pub version: &'a str,
    pub delegate_executable: Option<&'a str>,
    pub lock_sha256: Option<&'a str>,
    pub update: Option<&'a runtime_data::RuntimeUpdateConfig>,
}

#[cfg(test)]
pub(crate) fn metadata_path(prefix: &Path) -> PathBuf {
    metadata_path_for(prefix, policy::metadata_file())
}

pub(crate) fn metadata_path_for(prefix: &Path, metadata_file: &str) -> PathBuf {
    prefix.join(metadata_file)
}

fn temporary_metadata_path_for(prefix: &Path, metadata_file: &str) -> PathBuf {
    metadata_path_for(prefix, metadata_file).with_extension("tmp")
}

fn backup_metadata_path_for(prefix: &Path, metadata_file: &str) -> PathBuf {
    metadata_path_for(prefix, metadata_file).with_extension("bak")
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
    invalidate_metadata_for(prefix, policy::metadata_file())
}

pub(crate) fn invalidate_metadata_for(prefix: &Path, metadata_file: &str) -> miette::Result<()> {
    remove_regular_file_if_present(&metadata_path_for(prefix, metadata_file))?;
    remove_regular_file_if_present(&temporary_metadata_path_for(prefix, metadata_file))?;
    remove_regular_file_if_present(&backup_metadata_path_for(prefix, metadata_file))
}

pub fn write_metadata(
    prefix: &Path,
    channels: &[String],
    packages: &[String],
) -> miette::Result<()> {
    write_metadata_for_identity(
        prefix,
        PrefixMetadataIdentity {
            display_name: policy::display_name(),
            install_name: policy::install_name(),
            metadata_file: policy::metadata_file(),
            version: policy::runtime_version(),
            delegate_executable: Some(policy::delegate_executable()),
            lock_sha256: None,
            update: runtime_data::current().header.update.as_ref(),
        },
        channels,
        packages,
    )
}

pub(crate) fn write_metadata_for_identity(
    prefix: &Path,
    identity: PrefixMetadataIdentity<'_>,
    channels: &[String],
    packages: &[String],
) -> miette::Result<()> {
    let meta = PrefixMetadata {
        schema_version: PREFIX_METADATA_SCHEMA_VERSION,
        display_name: identity.display_name.to_string(),
        install_name: identity.install_name.to_string(),
        metadata_file: identity.metadata_file.to_string(),
        version: identity.version.to_string(),
        delegate_executable: identity.delegate_executable.map(ToString::to_string),
        lock_sha256: identity.lock_sha256.map(ToString::to_string),
        channels: channels.to_vec(),
        packages: packages.to_vec(),
        bootstrap_state: BootstrapPhase::Ready,
        update: identity.update.map(ExecutableUpdateMetadata::from_config),
    };
    persist_metadata_for(prefix, identity.metadata_file, &meta)
}

pub(crate) fn persist_metadata_for(
    prefix: &Path,
    metadata_file: &str,
    meta: &PrefixMetadata,
) -> miette::Result<()> {
    if meta.metadata_file != metadata_file {
        return Err(miette::miette!(
            "runtime metadata file is {}, expected {metadata_file}",
            meta.metadata_file
        ));
    }
    let path = metadata_path_for(prefix, metadata_file);
    let temporary_path = temporary_metadata_path_for(prefix, metadata_file);
    let backup_path = backup_metadata_path_for(prefix, metadata_file);
    remove_regular_file_if_present(&temporary_path)?;
    let mut temporary = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
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

    if let Err(replace_error) = std::fs::rename(&temporary_path, &path) {
        if !path.exists() {
            return Err(replace_error).into_diagnostic().with_context(|| {
                format!(
                    "failed to replace runtime metadata at {}",
                    policy::path_for_display(&path)
                )
            });
        }
        remove_regular_file_if_present(&backup_path)?;
        std::fs::rename(&path, &backup_path)
            .into_diagnostic()
            .with_context(|| {
                format!(
                    "failed to preserve runtime metadata at {}",
                    policy::path_for_display(&backup_path)
                )
            })?;
        if let Err(error) = std::fs::rename(&temporary_path, &path) {
            let rollback = std::fs::rename(&backup_path, &path);
            return match rollback {
                Ok(()) => Err(error).into_diagnostic().with_context(|| {
                    format!(
                        "failed to replace runtime metadata at {}",
                        policy::path_for_display(&path)
                    )
                }),
                Err(rollback_error) => Err(miette::miette!(
                    "failed to replace runtime metadata at {}: {error}. The previous metadata could not be restored: {rollback_error}",
                    policy::path_for_display(&path)
                )),
            };
        }
    }
    remove_regular_file_if_present(&backup_path)?;
    Ok(())
}

#[cfg(test)]
pub fn read_metadata(prefix: &Path) -> miette::Result<PrefixMetadata> {
    read_metadata_from_path(&metadata_path(prefix))
}

pub(crate) fn read_metadata_for(
    prefix: &Path,
    metadata_file: &str,
) -> miette::Result<PrefixMetadata> {
    let path = metadata_path_for(prefix, metadata_file);
    if let Some(meta) = read_regular_metadata_if_present(&path)? {
        return Ok(meta);
    }
    let temporary_path = temporary_metadata_path_for(prefix, metadata_file);
    let backup_path = backup_metadata_path_for(prefix, metadata_file);
    if let Some(meta) = read_regular_metadata_if_present(&temporary_path)? {
        let meta = restore_metadata_path(&temporary_path, &path, meta)?;
        remove_regular_file_if_present(&backup_path)?;
        return Ok(meta);
    }
    if let Some(meta) = read_regular_metadata_if_present(&backup_path)? {
        return restore_metadata_path(&backup_path, &path, meta);
    }
    read_metadata_from_path(&path)
}

fn restore_metadata_path(
    recovery_path: &Path,
    final_path: &Path,
    meta: PrefixMetadata,
) -> miette::Result<PrefixMetadata> {
    match std::fs::rename(recovery_path, final_path) {
        Ok(()) => Ok(meta),
        Err(error) => {
            if let Some(current) = read_regular_metadata_if_present(final_path)? {
                return Ok(current);
            }
            Err(error).into_diagnostic().with_context(|| {
                format!(
                    "failed to restore runtime metadata at {}",
                    policy::path_for_display(final_path)
                )
            })
        }
    }
}

fn read_regular_metadata_if_present(path: &Path) -> miette::Result<Option<PrefixMetadata>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(miette::miette!(
                "runtime metadata is not a regular file: {}",
                policy::path_for_display(path)
            ))
        }
        Ok(_) => read_metadata_from_path(path).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).into_diagnostic(),
    }
}

pub(crate) fn read_metadata_from_path(path: &Path) -> miette::Result<PrefixMetadata> {
    let data = std::fs::read_to_string(path)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to read runtime metadata at {}",
                policy::path_for_display(path)
            )
        })?;
    serde_json::from_str(&data)
        .into_diagnostic()
        .with_context(|| {
            format!(
                "failed to parse runtime metadata at {}",
                policy::path_for_display(path)
            )
        })
}

#[cfg(test)]
pub(crate) fn validate_metadata_identity(meta: &PrefixMetadata) -> miette::Result<()> {
    validate_metadata_identity_for(
        meta,
        policy::display_name(),
        policy::install_name(),
        policy::metadata_file(),
    )
}

pub(crate) fn validate_metadata_identity_for(
    meta: &PrefixMetadata,
    display_name: &str,
    install_name: &str,
    metadata_file: &str,
) -> miette::Result<()> {
    if meta.schema_version != PREFIX_METADATA_SCHEMA_VERSION {
        return Err(miette::miette!(
            "unsupported runtime metadata schema version: {}",
            meta.schema_version
        ));
    }
    if meta.display_name != display_name {
        return Err(miette::miette!(
            "runtime metadata belongs to {}, not {}",
            meta.display_name,
            display_name
        ));
    }
    if meta.install_name != install_name {
        return Err(miette::miette!(
            "runtime metadata install name is {}, expected {}",
            meta.install_name,
            install_name
        ));
    }
    if meta.metadata_file != metadata_file {
        return Err(miette::miette!(
            "runtime metadata file is {}, expected {}",
            meta.metadata_file,
            metadata_file
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn validate_metadata_ready(meta: &PrefixMetadata) -> miette::Result<()> {
    validate_metadata_identity(meta)?;
    validate_ready_phase(meta)
}

pub(crate) fn validate_metadata_ready_for(
    meta: &PrefixMetadata,
    display_name: &str,
    install_name: &str,
    metadata_file: &str,
) -> miette::Result<()> {
    validate_metadata_identity_for(meta, display_name, install_name, metadata_file)?;
    validate_ready_phase(meta)
}

fn validate_ready_phase(meta: &PrefixMetadata) -> miette::Result<()> {
    if meta.bootstrap_state != BootstrapPhase::Ready {
        return Err(miette::miette!(
            "runtime metadata does not mark bootstrap complete"
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
    write_frozen_with_message(prefix, &policy::frozen_message())
}

pub(crate) fn write_frozen_with_message(prefix: &Path, message: &str) -> miette::Result<()> {
    let frozen_path = prefix.join("conda-meta").join("frozen");
    let contents = serde_json::json!({ "message": message });
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
        assert!(meta.update.is_none());
    }

    #[test]
    fn test_update_metadata_starts_with_only_stamped_policy() {
        let tmp = TempDir::new().unwrap();
        let update = runtime_data::RuntimeUpdateConfig {
            channel: "https://conda.anaconda.org/jezdez".to_string(),
            package: "conda-runtime".to_string(),
            build_number: 4,
            ownership: runtime_data::UpdateOwnership::External,
            instruction: Some("brew update && brew upgrade conda".to_string()),
        };

        write_metadata_for_identity(
            tmp.path(),
            PrefixMetadataIdentity {
                display_name: "conda",
                install_name: "runtime",
                metadata_file: ".conda.json",
                version: "26.5.3",
                delegate_executable: Some("conda"),
                lock_sha256: None,
                update: Some(&update),
            },
            &[],
            &[],
        )
        .unwrap();

        let meta = read_metadata_from_path(&tmp.path().join(".conda.json")).unwrap();
        let recorded = meta.update.unwrap();
        assert_eq!(recorded.ownership, update.ownership);
        assert_eq!(recorded.channel, update.channel);
        assert_eq!(recorded.package, update.package);
        assert_eq!(recorded.build_number, update.build_number);
        assert_eq!(recorded.instruction, update.instruction);
        assert!(recorded.executable.as_os_str().is_empty());
        assert!(recorded.artifact_name.is_empty());
        assert!(recorded.sha256.is_empty());
        assert!(recorded.pending.is_none());
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
        assert!(parsed.update.is_none());
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
