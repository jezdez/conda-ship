//! Automatic bootstrap and prefix helpers.

use std::path::{Path, PathBuf};

use miette::IntoDiagnostic;

use crate::config::{
    PrefixMetadata, embedded_config, embedded_lock, read_metadata, write_condarc, write_frozen,
    write_metadata,
};
use crate::{constructor_metadata, exec, install, policy};

pub(crate) fn is_bootstrapped(prefix: &Path) -> bool {
    prefix.join("conda-meta").is_dir()
}

fn is_empty_dir(prefix: &Path) -> miette::Result<bool> {
    if !prefix.is_dir() {
        return Ok(false);
    }
    Ok(std::fs::read_dir(prefix)
        .into_diagnostic()?
        .next()
        .is_none())
}

fn require_managed_prefix(prefix: &Path, action: &str) -> miette::Result<()> {
    read_managed_metadata(prefix, action).map(|_| ())
}

fn read_managed_metadata(prefix: &Path, action: &str) -> miette::Result<PrefixMetadata> {
    let metadata_path = crate::config::metadata_path(prefix);
    if !metadata_path.is_file() {
        return Err(miette::miette!(
            "refusing to {action} unmanaged install path: {}\n  Expected runtime metadata file: {}",
            policy::path_for_display(prefix),
            policy::path_for_display(&metadata_path)
        ));
    }

    let meta = read_metadata(prefix).map_err(|err| {
        miette::miette!(
            "refusing to {action} unmanaged install path: {}\n  Invalid runtime metadata file: {}\n  {err}",
            policy::path_for_display(prefix),
            policy::path_for_display(&metadata_path)
        )
    })?;
    crate::config::validate_metadata_identity(&meta).map_err(|err| {
        miette::miette!(
            "refusing to {action} install path owned by a different runtime: {}\n  Invalid runtime metadata file: {}\n  {err}",
            policy::path_for_display(prefix),
            policy::path_for_display(&metadata_path)
        )
    })?;
    Ok(meta)
}

pub(crate) async fn ensure_bootstrapped(prefix: &Path) -> miette::Result<()> {
    if is_bootstrapped(prefix) {
        require_managed_prefix(prefix, "use")?;
        return Ok(());
    }

    eprintln!(
        "{} No runtime installation found. Bootstrapping now...",
        console::style(">>").cyan().bold()
    );
    bootstrap(prefix, configured_bundle()?, configured_offline()).await
}

fn configured_bundle() -> miette::Result<Option<PathBuf>> {
    let Some(value) = std::env::var_os(policy::bundle_env_var()).filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    if !path.is_dir() {
        return Err(miette::miette!(
            "configured bundle path is not a directory: {}",
            policy::path_for_display(&path)
        ));
    }
    Ok(Some(path))
}

fn configured_offline() -> bool {
    std::env::var_os(policy::offline_env_var())
        .filter(|value| !value.is_empty())
        .is_some_and(|value| {
            let value = value.to_string_lossy();
            value != "0" && !value.eq_ignore_ascii_case("false")
        })
}

async fn bootstrap(prefix: &Path, bundle: Option<PathBuf>, offline: bool) -> miette::Result<()> {
    if prefix.exists() {
        if is_bootstrapped(prefix) {
            require_managed_prefix(prefix, "use")?;
            return Ok(());
        }
        if !is_empty_dir(prefix)? {
            return Err(miette::miette!(
                "refusing to bootstrap into existing non-empty path: {}",
                policy::path_for_display(prefix)
            ));
        }
    }

    let cfg = embedded_config();
    let channels = cfg.channels.clone();
    let specs = cfg.packages.clone();

    eprintln!("   Install path: {}", policy::path_for_display(prefix));
    eprintln!("   Channels: {}", channels.join(", "));
    eprintln!("   Packages: {}", specs.join(", "));
    if offline {
        eprintln!("   Mode:     offline");
    }

    let lock_content = embedded_lock().map(ToOwned::to_owned);
    if lock_content.is_some() {
        eprintln!("   Using stamped lockfile");
    }

    if let Some(ref bundle_dir) = bundle {
        let content = lock_content
            .as_deref()
            .ok_or_else(|| miette::miette!("configured bundle requires a stamped runtime lock"))?;
        eprintln!("   Bundle:   {}", policy::path_for_display(bundle_dir));
        install::from_lockfile_with_bundle(prefix, content, bundle_dir, offline).await?;
    } else if let Some(embedded_dir) = install::extract_embedded_bundle()? {
        let content = lock_content
            .as_deref()
            .ok_or_else(|| miette::miette!("embedded bundle requires a stamped runtime lock"))?;
        eprintln!("   Bundle:   embedded");
        let result = install::from_lockfile_with_bundle(prefix, content, &embedded_dir, true).await;
        let _ = std::fs::remove_dir_all(&embedded_dir);
        result?;
    } else if offline {
        let content = lock_content
            .as_deref()
            .ok_or_else(|| miette::miette!("offline bootstrap requires a stamped runtime lock"))?;
        install::from_lockfile_offline(prefix, content).await?;
    } else {
        let content = lock_content.as_deref().ok_or_else(|| {
            miette::miette!("runtime has no stamped lockfile; rebuild it with `cs build`")
        })?;
        install::from_lockfile(prefix, content).await?;
    }

    if let Some(content) = lock_content.as_deref() {
        constructor_metadata::write_prefix_metadata(prefix, content, &specs)?;
    }
    write_configured_policy(prefix, cfg)?;
    write_metadata(prefix, &channels, &specs)?;

    compile_python_bytecode(prefix);

    eprintln!(
        "{} Runtime bootstrapped successfully.",
        console::style("✔").green().bold()
    );
    Ok(())
}

fn write_configured_policy(
    prefix: &Path,
    config: &crate::config::RuntimeConfig,
) -> miette::Result<()> {
    if let Some(contents) = config.condarc.as_deref() {
        write_condarc(prefix, contents)?;
    }
    if config.freeze_base {
        write_frozen(prefix)?;
    }
    Ok(())
}

fn compile_python_bytecode(prefix: &Path) {
    let python = exec::executable_in_prefix(prefix, "python");
    if !python.exists() {
        return;
    }

    let lib_dir = prefix.join("lib");
    let result = install::wrap_spinner("compiling Python bytecode", move || {
        std::process::Command::new(&python)
            .args(["-m", "compileall", "-q", "-j", "0"])
            .arg(&lib_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
    });

    match result {
        Ok(status) if status.success() => {}
        _ => {
            eprintln!(
                "   {} bytecode compilation finished with errors (non-fatal)",
                console::style("!").yellow(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_is_bootstrapped_true() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("conda-meta")).unwrap();
        assert!(is_bootstrapped(tmp.path()));
    }

    #[test]
    fn test_is_bootstrapped_false() {
        let tmp = TempDir::new().unwrap();
        assert!(!is_bootstrapped(tmp.path()));
    }

    #[test]
    fn test_default_policy_leaves_prefix_files_untouched() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("conda-meta")).unwrap();
        std::fs::write(tmp.path().join(".condarc"), "package: condarc\n").unwrap();
        std::fs::write(
            tmp.path().join("conda-meta").join("frozen"),
            "package marker",
        )
        .unwrap();

        write_configured_policy(tmp.path(), &crate::config::RuntimeConfig::default()).unwrap();

        assert_eq!(
            std::fs::read_to_string(tmp.path().join(".condarc")).unwrap(),
            "package: condarc\n"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("conda-meta").join("frozen")).unwrap(),
            "package marker"
        );
    }

    #[test]
    fn test_configured_policy_writes_condarc_and_frozen_marker() {
        let tmp = TempDir::new().unwrap();
        let config = crate::config::RuntimeConfig {
            condarc: Some("solver: rattler\n".to_string()),
            freeze_base: true,
            ..crate::config::RuntimeConfig::default()
        };

        write_configured_policy(tmp.path(), &config).unwrap();

        assert_eq!(
            std::fs::read_to_string(tmp.path().join(".condarc")).unwrap(),
            "solver: rattler\n"
        );
        assert!(tmp.path().join("conda-meta").join("frozen").is_file());
    }
}
