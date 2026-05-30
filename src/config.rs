//! Configuration, metadata, and `.condarc` management.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use miette::{Context, IntoDiagnostic};

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

pub fn embedded_bundle_len() -> Option<u64> {
    embedded_bundle().map(runtime_data::EmbeddedBundle::len)
}

pub(crate) fn install_method() -> Option<&'static str> {
    runtime_data::current().header.install_method.as_deref()
}

// Prefix metadata.

#[derive(serde::Serialize, serde::Deserialize)]
pub struct PrefixMetadata {
    pub version: String,
    pub channels: Vec<String>,
    pub packages: Vec<String>,
}

pub(crate) fn metadata_path(prefix: &Path) -> PathBuf {
    prefix.join(policy::metadata_file())
}

pub fn write_metadata(
    prefix: &Path,
    channels: &[String],
    packages: &[String],
) -> miette::Result<()> {
    let meta = PrefixMetadata {
        version: env!("CARGO_PKG_VERSION").to_string(),
        channels: channels.to_vec(),
        packages: packages.to_vec(),
    };
    let json = serde_json::to_string_pretty(&meta).into_diagnostic()?;
    std::fs::write(metadata_path(prefix), json).into_diagnostic()?;
    Ok(())
}

pub fn read_metadata(prefix: &Path) -> miette::Result<PrefixMetadata> {
    let path = metadata_path(prefix);
    let data = std::fs::read_to_string(&path)
        .into_diagnostic()
        .with_context(|| format!("failed to read runtime metadata at {}", path.display()))?;
    serde_json::from_str(&data)
        .into_diagnostic()
        .with_context(|| format!("failed to parse runtime metadata at {}", path.display()))
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
    eprintln!("   Wrote {}", frozen_path.display());
    Ok(())
}

// .condarc.

pub fn write_condarc(prefix: &Path, channels: &[String]) -> miette::Result<()> {
    let condarc_path = prefix.join(".condarc");
    let mut contents = "\
solver: rattler
auto_activate_base: false
notify_outdated_conda: false
show_channel_urls: true
"
    .to_string();

    if channels.is_empty() {
        contents.push_str("channels: []\n");
    } else {
        contents.push_str("channels:\n");
        for channel in channels {
            contents.push_str("  - ");
            contents.push_str(&serde_json::to_string(channel).into_diagnostic()?);
            contents.push('\n');
        }
    }

    std::fs::create_dir_all(prefix).into_diagnostic()?;
    std::fs::write(&condarc_path, contents).into_diagnostic()?;
    eprintln!("   Wrote {}", condarc_path.display());
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
        assert_eq!(meta.channels, channels);
        assert_eq!(meta.packages, packages);
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
    fn test_write_condarc_snapshot() {
        let tmp = TempDir::new().unwrap();
        write_condarc(
            tmp.path(),
            &[
                "conda-forge".to_string(),
                "https://repo.example.test/conda".to_string(),
            ],
        )
        .unwrap();

        let contents = std::fs::read_to_string(tmp.path().join(".condarc")).unwrap();
        insta::assert_snapshot!("condarc", contents);
    }

    #[test]
    fn test_write_condarc_idempotent() {
        let tmp = TempDir::new().unwrap();
        let channels = ["conda-forge".to_string()];

        write_condarc(tmp.path(), &channels).unwrap();
        let first = std::fs::read_to_string(tmp.path().join(".condarc")).unwrap();

        write_condarc(tmp.path(), &channels).unwrap();
        let second = std::fs::read_to_string(tmp.path().join(".condarc")).unwrap();

        assert_eq!(
            first, second,
            "writing condarc twice should produce identical content"
        );
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
