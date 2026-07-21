//! Automatic bootstrap and prefix helpers.

use std::path::{Path, PathBuf};

use miette::IntoDiagnostic;

use crate::bootstrap_lock::BootstrapLock;
use crate::bootstrap_state::{self, BootstrapPhase};
use crate::config::{
    PrefixMetadata, embedded_config, embedded_lock, read_metadata, write_condarc, write_frozen,
    write_metadata,
};
use crate::{constructor_metadata, exec, install, policy};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrefixDisposition {
    Ready,
    Bootstrap { reinstall: bool },
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
    crate::config::validate_metadata_ready(&meta).map_err(|err| {
        miette::miette!(
            "refusing to {action} install path owned by a different runtime: {}\n  Invalid runtime metadata file: {}\n  {err}",
            policy::path_for_display(prefix),
            policy::path_for_display(&metadata_path)
        )
    })?;
    Ok(meta)
}

pub(crate) async fn ensure_bootstrapped(prefix: &Path) -> miette::Result<()> {
    let _lock = BootstrapLock::acquire(prefix)?;
    let reinstall = match prefix_disposition(prefix)? {
        PrefixDisposition::Ready => return Ok(()),
        PrefixDisposition::Bootstrap { reinstall } => reinstall,
    };

    if reinstall {
        eprintln!(
            "{} Previous bootstrap was interrupted. Retrying...",
            console::style(">>").cyan().bold()
        );
    } else {
        eprintln!(
            "{} No runtime installation found. Bootstrapping now...",
            console::style(">>").cyan().bold()
        );
    }
    bootstrap(
        prefix,
        configured_bundle()?,
        configured_offline(),
        reinstall,
    )
    .await
}

fn prefix_disposition(prefix: &Path) -> miette::Result<PrefixDisposition> {
    validate_prefix_path(prefix)?;

    if let Some(state) = bootstrap_state::read(prefix).map_err(|error| {
        miette::miette!(
            "refusing to use install path with invalid bootstrap state: {}\n  {error}",
            policy::path_for_display(prefix)
        )
    })? {
        bootstrap_state::validate_identity(&state).map_err(|error| {
            miette::miette!(
                "refusing to use install path owned by a different runtime: {}\n  {error}",
                policy::path_for_display(prefix)
            )
        })?;
        if state.phase() != BootstrapPhase::Installing {
            return Err(miette::miette!(
                "refusing to use install path with invalid bootstrap state: {}",
                policy::path_for_display(prefix)
            ));
        }

        let metadata_path = crate::config::metadata_path(prefix);
        match std::fs::symlink_metadata(&metadata_path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(miette::miette!(
                    "refusing to recover install path with invalid runtime metadata: {}",
                    policy::path_for_display(&metadata_path)
                ));
            }
            Ok(_) => {
                if let Ok(meta) = read_metadata(prefix) {
                    crate::config::validate_metadata_identity(&meta).map_err(|error| {
                        miette::miette!(
                            "refusing to recover install path owned by a different runtime: {}\n  {error}",
                            policy::path_for_display(prefix)
                        )
                    })?;
                    if meta.bootstrap_state == BootstrapPhase::Ready
                        && exec::validate_delegate(prefix, policy::delegate_executable()).is_ok()
                    {
                        bootstrap_state::remove(prefix)?;
                        return Ok(PrefixDisposition::Ready);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).into_diagnostic(),
        }

        return Ok(PrefixDisposition::Bootstrap { reinstall: true });
    }

    if !prefix.exists() || is_empty_dir(prefix)? {
        return Ok(PrefixDisposition::Bootstrap { reinstall: false });
    }

    validate_ready_prefix(prefix).map_err(|error| {
        miette::miette!(
            "refusing to bootstrap into existing non-empty path: {}\n  {error}",
            policy::path_for_display(prefix)
        )
    })?;
    Ok(PrefixDisposition::Ready)
}

fn validate_prefix_path(prefix: &Path) -> miette::Result<()> {
    match std::fs::symlink_metadata(prefix) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(miette::miette!(
                "refusing to use install path that is not a directory: {}",
                policy::path_for_display(prefix)
            ))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).into_diagnostic(),
    }
}

fn validate_ready_prefix(prefix: &Path) -> miette::Result<()> {
    read_managed_metadata(prefix, "use")?;
    exec::validate_delegate(prefix, policy::delegate_executable())
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

async fn bootstrap(
    prefix: &Path,
    bundle: Option<PathBuf>,
    offline: bool,
    reinstall: bool,
) -> miette::Result<()> {
    if !reinstall && prefix.exists() && !is_empty_dir(prefix)? {
        return Err(miette::miette!(
            "refusing to bootstrap into existing non-empty path: {}",
            policy::path_for_display(prefix)
        ));
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
    let content = lock_content.as_deref().ok_or_else(|| {
        if bundle.is_some() {
            miette::miette!("configured bundle requires a stamped runtime lock")
        } else if crate::config::embedded_bundle().is_some() {
            miette::miette!("embedded bundle requires a stamped runtime lock")
        } else if offline {
            miette::miette!("offline bootstrap requires a stamped runtime lock")
        } else {
            miette::miette!("runtime has no stamped lockfile; rebuild it with `cs build`")
        }
    })?;
    bootstrap_state::write_installing(prefix)?;
    crate::config::invalidate_metadata(prefix)?;

    if let Some(ref bundle_dir) = bundle {
        eprintln!("   Bundle:   {}", policy::path_for_display(bundle_dir));
        install::from_lockfile_with_bundle(prefix, content, bundle_dir, offline, reinstall).await?;
    } else if let Some(embedded_dir) = install::extract_embedded_bundle()? {
        eprintln!("   Bundle:   embedded");
        let result =
            install::from_lockfile_with_bundle(prefix, content, &embedded_dir, true, reinstall)
                .await;
        let _ = std::fs::remove_dir_all(&embedded_dir);
        result?;
    } else if offline {
        install::from_lockfile_offline(prefix, content, reinstall).await?;
    } else {
        install::from_lockfile(prefix, content, reinstall).await?;
    }

    if let Some(content) = lock_content.as_deref() {
        constructor_metadata::write_prefix_metadata(prefix, content, &specs)?;
    }
    write_configured_policy(prefix, cfg)?;
    compile_python_bytecode(prefix);
    exec::validate_delegate(prefix, policy::delegate_executable())?;
    write_metadata(prefix, &channels, &specs)?;
    bootstrap_state::remove(prefix)?;

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
    use std::process::Command;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    #[test]
    fn test_absent_prefix_needs_initial_bootstrap() {
        let tmp = TempDir::new().unwrap();
        let prefix = tmp.path().join("runtime");

        assert_eq!(
            prefix_disposition(&prefix).unwrap(),
            PrefixDisposition::Bootstrap { reinstall: false }
        );
    }

    #[test]
    fn test_unknown_nonempty_prefix_is_refused() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("conda-meta")).unwrap();

        let error = prefix_disposition(tmp.path()).unwrap_err().to_string();

        assert!(error.contains("existing non-empty path"));
    }

    #[test]
    #[cfg(unix)]
    fn test_symlink_prefix_is_refused() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("target");
        let prefix = tmp.path().join("runtime");
        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, &prefix).unwrap();

        let error = prefix_disposition(&prefix).unwrap_err().to_string();

        assert!(error.contains("not a directory"));
    }

    #[test]
    fn test_foreign_bootstrap_marker_is_refused() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            bootstrap_state::path(tmp.path()),
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": 1,
                "state": "installing",
                "display_name": policy::display_name(),
                "install_name": "another-runtime",
                "metadata_file": policy::metadata_file(),
            }))
            .unwrap(),
        )
        .unwrap();

        let error = prefix_disposition(tmp.path()).unwrap_err().to_string();

        assert!(error.contains("different runtime"));
        assert!(error.contains("another-runtime"));
    }

    #[test]
    fn test_malformed_bootstrap_marker_is_refused() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(bootstrap_state::path(tmp.path()), b"not json").unwrap();

        let error = prefix_disposition(tmp.path()).unwrap_err().to_string();

        assert!(error.contains("invalid bootstrap state"));
    }

    fn create_ready_prefix(prefix: &Path) {
        let delegate = exec::executable_in_prefix(prefix, policy::delegate_executable());
        std::fs::create_dir_all(delegate.parent().unwrap()).unwrap();
        std::fs::write(delegate, b"delegate").unwrap();
        write_metadata(prefix, &[], &[]).unwrap();
    }

    #[test]
    fn test_pre_state_marker_metadata_is_accepted_as_ready() {
        let tmp = TempDir::new().unwrap();
        create_ready_prefix(tmp.path());

        assert_eq!(
            prefix_disposition(tmp.path()).unwrap(),
            PrefixDisposition::Ready
        );
        assert!(!bootstrap_state::path(tmp.path()).exists());
    }

    #[test]
    fn test_ready_metadata_without_delegate_is_refused() {
        let tmp = TempDir::new().unwrap();
        write_metadata(tmp.path(), &[], &[]).unwrap();

        let error = prefix_disposition(tmp.path()).unwrap_err().to_string();

        assert!(error.contains("existing non-empty path"));
        assert!(error.contains("executable not found"));
    }

    #[test]
    fn test_owned_incomplete_prefix_forces_reinstall() {
        let tmp = TempDir::new().unwrap();
        bootstrap_state::write_installing(tmp.path()).unwrap();
        std::fs::create_dir_all(tmp.path().join("conda-meta")).unwrap();

        assert_eq!(
            prefix_disposition(tmp.path()).unwrap(),
            PrefixDisposition::Bootstrap { reinstall: true }
        );
    }

    #[test]
    fn test_ready_commit_cleans_stale_installing_marker() {
        let tmp = TempDir::new().unwrap();
        create_ready_prefix(tmp.path());
        bootstrap_state::write_installing(tmp.path()).unwrap();

        assert_eq!(
            prefix_disposition(tmp.path()).unwrap(),
            PrefixDisposition::Ready
        );
        assert!(!bootstrap_state::path(tmp.path()).exists());
    }

    #[test]
    fn bootstrap_lock_child_reclassifies_after_lock_release() {
        let Some(prefix) = std::env::var_os("CONDA_SHIP_LOCK_TEST_PREFIX") else {
            return;
        };
        let signal = std::env::var_os("CONDA_SHIP_LOCK_TEST_SIGNAL").unwrap();
        std::fs::write(signal, b"waiting").unwrap();

        let prefix = PathBuf::from(prefix);
        let _lock = BootstrapLock::acquire(&prefix).unwrap();

        assert_eq!(
            prefix_disposition(&prefix).unwrap(),
            PrefixDisposition::Ready
        );
    }

    #[test]
    fn test_waiting_process_reclassifies_after_lock_release() {
        let tmp = TempDir::new().unwrap();
        let prefix = tmp.path().join("runtime");
        let first = BootstrapLock::acquire(&prefix).unwrap();
        let signal = tmp.path().join("child-waiting");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "commands::tests::bootstrap_lock_child_reclassifies_after_lock_release",
                "--nocapture",
            ])
            .env("CONDA_SHIP_LOCK_TEST_PREFIX", &prefix)
            .env("CONDA_SHIP_LOCK_TEST_SIGNAL", &signal)
            .spawn()
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        while !signal.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(signal.exists(), "child did not reach the lock");
        std::thread::sleep(Duration::from_millis(50));
        assert!(child.try_wait().unwrap().is_none(), "child did not block");

        create_ready_prefix(&prefix);
        drop(first);

        assert!(child.wait().unwrap().success());
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
