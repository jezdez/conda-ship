//! Configuration, metadata, and `.condarc` management.

use std::path::{Path, PathBuf};

use miette::IntoDiagnostic;

/// The rattler-lock v6 lockfile embedded at compile time by `build.rs`.
pub const EMBEDDED_LOCK: &str = include_str!(concat!(env!("OUT_DIR"), "/cx.lock"));

/// The `pixi.toml` embedded at compile time (contains `[tool.cx]`).
const EMBEDDED_PIXI_TOML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/pixi.toml"));

// ─── [tool.cx] in pixi.toml ─────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct PixiToml {
    tool: ToolSection,
}

#[derive(serde::Deserialize)]
struct ToolSection {
    cx: CxConfig,
}

#[derive(serde::Deserialize)]
pub struct CxConfig {
    pub channels: Vec<String>,
    pub packages: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// Parse the `[tool.cx]` section from the embedded `pixi.toml`.
pub fn embedded_config() -> CxConfig {
    let pixi: PixiToml =
        toml::from_str(EMBEDDED_PIXI_TOML).expect("invalid [tool.cx] in pixi.toml");
    pixi.tool.cx
}

// ─── .cx.json (prefix metadata) ─────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
pub struct PrefixMetadata {
    pub version: String,
    pub channels: Vec<String>,
    pub packages: Vec<String>,
    #[serde(default)]
    pub excludes: Vec<String>,
}

fn metadata_path(prefix: &Path) -> PathBuf {
    prefix.join(".cx.json")
}

pub fn write_metadata(
    prefix: &Path,
    channels: &[String],
    packages: &[String],
    excludes: &[String],
) -> miette::Result<()> {
    let meta = PrefixMetadata {
        version: env!("CARGO_PKG_VERSION").to_string(),
        channels: channels.to_vec(),
        packages: packages.to_vec(),
        excludes: excludes.to_vec(),
    };
    let json = serde_json::to_string_pretty(&meta).into_diagnostic()?;
    std::fs::write(metadata_path(prefix), json).into_diagnostic()?;
    Ok(())
}

pub fn read_metadata(prefix: &Path) -> miette::Result<PrefixMetadata> {
    let path = metadata_path(prefix);
    if !path.exists() {
        let config = embedded_config();
        return Ok(PrefixMetadata {
            version: "unknown".to_string(),
            channels: config.channels,
            packages: config.packages,
            excludes: config.exclude,
        });
    }
    let data = std::fs::read_to_string(&path).into_diagnostic()?;
    serde_json::from_str(&data).into_diagnostic()
}

// ─── conda-meta/frozen (CEP 22) ──────────────────────────────────────────────

/// Write a CEP 22 frozen marker to protect the base prefix from accidental
/// modification. Users should create named environments for their work and
/// use `conda self update` (via conda-self) to update the base installation.
/// See: https://conda.org/learn/ceps/cep-0022/
pub fn write_frozen(prefix: &Path) -> miette::Result<()> {
    let frozen_path = prefix.join("conda-meta").join("frozen");
    let contents = serde_json::json!({
        "message": concat!(
            "This base environment is managed by cx (conda-express).\n",
            "Create a new environment instead: conda create -n myenv\n",
            "To re-bootstrap: cx bootstrap --force\n",
            "To override: pass --override-frozen-env"
        )
    });
    std::fs::create_dir_all(prefix.join("conda-meta")).into_diagnostic()?;
    std::fs::write(
        &frozen_path,
        serde_json::to_string_pretty(&contents).into_diagnostic()?,
    )
    .into_diagnostic()?;
    eprintln!("   Wrote {}", frozen_path.display());
    Ok(())
}

// ─── .condarc ────────────────────────────────────────────────────────────────

pub fn write_condarc(prefix: &Path) -> miette::Result<()> {
    let condarc_path = prefix.join(".condarc");
    let contents = "\
solver: rattler
auto_activate_base: false
notify_outdated_conda: false
show_channel_urls: true
default_channels:
  - conda-forge
";
    std::fs::create_dir_all(prefix).into_diagnostic()?;
    std::fs::write(&condarc_path, contents).into_diagnostic()?;
    eprintln!("   Wrote {}", condarc_path.display());
    Ok(())
}
