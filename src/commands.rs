//! Subcommand implementations and prefix helpers.

use std::{env, path::Path};

use miette::{Context, IntoDiagnostic};
use rattler_conda_types::PrefixRecord;

use crate::cli::Verbosity;
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

pub(crate) fn require_managed_prefix(prefix: &Path, action: &str) -> miette::Result<()> {
    read_managed_metadata(prefix, action).map(|_| ())
}

fn read_managed_metadata(prefix: &Path, action: &str) -> miette::Result<PrefixMetadata> {
    let metadata_path = crate::config::metadata_path(prefix);
    if !metadata_path.is_file() {
        return Err(miette::miette!(
            "refusing to {action} unmanaged install path: {}\n  Expected runtime metadata file: {}",
            prefix.display(),
            metadata_path.display()
        ));
    }

    let meta = read_metadata(prefix).map_err(|err| {
        miette::miette!(
            "refusing to {action} unmanaged install path: {}\n  Invalid runtime metadata file: {}\n  {err}",
            prefix.display(),
            metadata_path.display()
        )
    })?;
    crate::config::validate_metadata_identity(&meta).map_err(|err| {
        miette::miette!(
            "refusing to {action} install path owned by a different runtime: {}\n  Invalid runtime metadata file: {}\n  {err}",
            prefix.display(),
            metadata_path.display()
        )
    })?;
    Ok(meta)
}

pub(crate) async fn ensure_bootstrapped(prefix: &Path) -> miette::Result<()> {
    if is_bootstrapped(prefix) {
        require_managed_prefix(prefix, "use")?;
    } else {
        eprintln!(
            "{} No conda installation found. Bootstrapping now...",
            console::style(">>").cyan().bold()
        );
        bootstrap(prefix, false, None, false, Verbosity::Quiet).await?;
    }
    Ok(())
}

fn reject_dangerous_prefix(prefix: &Path) -> miette::Result<()> {
    let home = dirs::home_dir();
    if std::fs::symlink_metadata(prefix)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(miette::miette!(
            "refusing to remove symbolic-link install path: {}",
            prefix.display()
        ));
    }
    let canon = prefix
        .canonicalize()
        .unwrap_or_else(|_| prefix.to_path_buf());

    let dangerous = canon.parent().is_none()
        || canon == Path::new("/")
        || canon == Path::new("")
        || home.as_deref() == Some(&canon)
        || canon == std::env::current_dir().unwrap_or_default();

    if dangerous {
        return Err(miette::miette!(
            "refusing to remove dangerous path: {}",
            prefix.display()
        ));
    }
    Ok(())
}

pub(crate) fn validate_bootstrap_flags(bundle: &Option<std::path::PathBuf>) -> miette::Result<()> {
    if let Some(path) = bundle
        && !path.is_dir()
    {
        return Err(miette::miette!(
            "--bundle path is not a directory: {}",
            path.display()
        ));
    }
    Ok(())
}

pub(crate) async fn bootstrap(
    prefix: &Path,
    force: bool,
    bundle: Option<std::path::PathBuf>,
    offline: bool,
    verbosity: Verbosity,
) -> miette::Result<()> {
    if prefix.exists() {
        if is_bootstrapped(prefix) {
            if !force {
                require_managed_prefix(prefix, "use")?;
                eprintln!(
                    "{} conda is already installed at {}",
                    console::style("✔").green(),
                    prefix.display()
                );
                eprintln!("  Use --force to re-bootstrap.");
                return Ok(());
            }
        } else if !is_empty_dir(prefix)? {
            return Err(miette::miette!(
                "refusing to bootstrap into existing non-empty path: {}",
                prefix.display()
            ));
        }
    }

    if force && prefix.exists() {
        reject_dangerous_prefix(prefix)?;
        if !is_empty_dir(prefix)? {
            require_managed_prefix(prefix, "remove")?;
        }
        eprintln!(
            "{} Removing existing install path at {}",
            console::style(">>").cyan().bold(),
            prefix.display()
        );
        remove_install_path(prefix)?;
    }

    let cfg = embedded_config();
    let channels = cfg.channels.clone();
    let specs = cfg.packages.clone();

    if verbosity != Verbosity::Quiet {
        eprintln!(
            "{} Bootstrapping conda into {}",
            console::style(">>").cyan().bold(),
            prefix.display()
        );
        eprintln!("   Channels: {}", channels.join(", "));
        eprintln!("   Packages: {}", specs.join(", "));
        if offline {
            eprintln!("   Mode:     offline");
        }
    }

    let lock_content = if let Some(lock) = embedded_lock() {
        if verbosity != Verbosity::Quiet {
            eprintln!("   Using stamped lockfile");
        }
        Some(lock.to_string())
    } else {
        None
    };

    if let Some(ref bundle_dir) = bundle {
        let content = lock_content
            .as_deref()
            .ok_or_else(|| miette::miette!("--bundle requires a stamped runtime lock"))?;
        if verbosity != Verbosity::Quiet {
            eprintln!("   Bundle:   {}", bundle_dir.display());
        }
        install::from_lockfile_with_bundle(prefix, content, bundle_dir, offline).await?;
    } else if let Some(embedded_dir) = install::extract_embedded_bundle()? {
        let content = lock_content
            .as_deref()
            .ok_or_else(|| miette::miette!("embedded bundle requires a stamped runtime lock"))?;
        if verbosity != Verbosity::Quiet {
            eprintln!("   Bundle:   embedded");
        }
        let result = install::from_lockfile_with_bundle(prefix, content, &embedded_dir, true).await;
        let _ = std::fs::remove_dir_all(&embedded_dir);
        result?;
    } else if offline {
        let content = lock_content
            .as_deref()
            .ok_or_else(|| miette::miette!("--offline requires a stamped runtime lock"))?;
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
    write_condarc(prefix, &channels)?;
    write_frozen(prefix)?;
    write_metadata(prefix, &channels, &specs)?;

    compile_python_bytecode(prefix);

    if verbosity != Verbosity::Quiet {
        eprintln!(
            "\n{} runtime bootstrapped successfully!",
            console::style("✔").green().bold()
        );
        eprintln!("   Install path: {}", prefix.display());
        eprintln!("   Run `{} status` for details.", policy::runtime_name());
        eprintln!(
            "   Use `{} <{}-args>` to run delegate commands.",
            policy::runtime_name(),
            policy::delegate()
        );
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
        Ok(s) if s.success() => {}
        _ => {
            eprintln!(
                "   {} bytecode compilation finished with errors (non-fatal)",
                console::style("!").yellow(),
            );
        }
    }
}

pub(crate) fn status(prefix: &Path) -> miette::Result<()> {
    if !is_bootstrapped(prefix) {
        eprintln!("No conda installation found at {}", prefix.display());
        return Ok(());
    }
    let meta = read_managed_metadata(prefix, "inspect")?;

    let bundle_len = crate::config::embedded_bundle_len();
    let binary_name = policy::status_binary_name(bundle_len.is_some());
    println!("{} {}", binary_name, policy::runtime_version());
    println!("  path:      {}", prefix.display());
    println!("  channels:  {}", meta.channels.join(", "));
    println!("  packages:  {}", meta.packages.join(", "));
    if let Some(bundle_len) = bundle_len {
        println!(
            "  bundle:    embedded ({:.1} MB)",
            bundle_len as f64 / 1_048_576.0
        );
    }

    let installed = PrefixRecord::collect_from_prefix::<PrefixRecord>(prefix).into_diagnostic()?;
    println!("  installed: {} packages", installed.len());

    let delegate = policy::delegate();
    let delegate_bin = exec::executable_in_prefix(prefix, delegate);
    println!(
        "  delegate:  {} ({})",
        delegate,
        if delegate_bin.exists() {
            delegate_bin.display().to_string()
        } else {
            "(not found)".to_string()
        }
    );

    Ok(())
}

pub(crate) fn uninstall(prefix: &Path, yes: bool, verbosity: Verbosity) -> miette::Result<()> {
    if !is_bootstrapped(prefix) {
        eprintln!(
            "{} No conda installation found at {}",
            console::style("!").yellow().bold(),
            prefix.display()
        );
        eprintln!("  Nothing to uninstall.");
        return Ok(());
    }
    require_managed_prefix(prefix, "uninstall")?;

    let envs_dir = prefix.join("envs");
    let named_envs: Vec<String> = if envs_dir.is_dir() {
        std::fs::read_dir(&envs_dir)
            .into_diagnostic()?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                if entry.path().join("conda-meta").is_dir() {
                    Some(entry.file_name().to_string_lossy().into_owned())
                } else {
                    None
                }
            })
            .collect()
    } else {
        vec![]
    };

    let runtime_binary = env::current_exe().ok();

    eprintln!(
        "{} This will permanently remove:",
        console::style("!").yellow().bold()
    );
    eprintln!("   Install path: {}", prefix.display());
    if !named_envs.is_empty() {
        eprintln!(
            "   Named environments ({}): {}",
            named_envs.len(),
            named_envs.join(", ")
        );
    }
    if !yes {
        eprint!("\n   Continue? [y/N] ");
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .into_diagnostic()
            .context("failed to read from stdin")?;
        if !matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
            eprintln!("  Aborted.");
            return Ok(());
        }
    }

    reject_dangerous_prefix(prefix)?;

    if verbosity != Verbosity::Quiet {
        eprintln!(
            "\n{} Removing install path at {}",
            console::style(">>").cyan().bold(),
            prefix.display()
        );
    }
    remove_install_path(prefix)?;

    if let Some(ref bin) = runtime_binary {
        let hint = match crate::config::install_method() {
            Some("homebrew") => format!("   brew uninstall {}", policy::display_name()),
            Some("cargo") => format!("   cargo uninstall {}", policy::display_name()),
            Some(method) => format!("   Installed via: {method}"),
            None => format!("   {}", bin.display()),
        };
        eprintln!(
            "\n{} To complete removal, delete the {} binary:",
            console::style("i").blue().bold(),
            policy::runtime_name()
        );
        eprintln!("{hint}");
    }
    if verbosity != Verbosity::Quiet {
        eprintln!(
            "\n{} {} has been uninstalled.",
            console::style("✔").green().bold(),
            policy::display_name()
        );
    }

    Ok(())
}

#[cfg(windows)]
fn remove_install_path(prefix: &Path) -> miette::Result<()> {
    match std::fs::remove_dir_all(prefix) {
        Ok(()) => Ok(()),
        Err(_) => {
            clear_readonly_recursive(prefix)?;
            std::fs::remove_dir_all(prefix)
                .into_diagnostic()
                .context("failed to remove install path")
        }
    }
}

#[cfg(not(windows))]
fn remove_install_path(prefix: &Path) -> miette::Result<()> {
    std::fs::remove_dir_all(prefix)
        .into_diagnostic()
        .context("failed to remove install path")
}

#[cfg(windows)]
fn clear_readonly_recursive(path: &Path) -> miette::Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let metadata = std::fs::symlink_metadata(path)
        .into_diagnostic()
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }

    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)
            .into_diagnostic()
            .with_context(|| format!("failed to read {}", path.display()))?
        {
            let entry = entry.into_diagnostic()?;
            clear_readonly_recursive(&entry.path())?;
        }
    }

    let mut permissions = metadata.permissions();
    if permissions.readonly() {
        permissions.set_readonly(false);
        std::fs::set_permissions(path, permissions)
            .into_diagnostic()
            .with_context(|| format!("failed to clear read-only bit on {}", path.display()))?;
    }
    Ok(())
}

pub(crate) fn print_disabled_shell_command(command: &str) -> ! {
    eprintln!(
        "{} `conda {command}` is not available in {}.",
        console::style("!").yellow().bold(),
        policy::display_name()
    );
    eprintln!();
    eprintln!(
        "  {} uses conda-spawn for environment activation.",
        policy::display_name()
    );
    eprintln!("  Instead of `conda activate myenv`, run:");
    eprintln!();
    eprintln!(
        "    {}",
        console::style(format!("{} shell myenv", policy::runtime_name())).green()
    );
    eprintln!();
    eprintln!("  To leave the environment, exit the subshell (Ctrl+D or `exit`).");
    eprintln!();
    eprintln!("  Learn more: https://github.com/conda-incubator/conda-spawn");
    std::process::exit(1);
}

pub(crate) fn print_disabled_init() -> ! {
    eprintln!(
        "{} `conda init` is not needed with {}.",
        console::style("!").yellow().bold(),
        policy::display_name()
    );
    eprintln!();
    eprintln!(
        "  {} uses conda-spawn, which does not require shell",
        policy::display_name()
    );
    eprintln!("  profile modifications.");
    if cfg!(windows) {
        eprintln!();
        eprintln!(
            "  Add the managed prefix's condabin directory to PATH with your shell or installer if you need direct conda access."
        );
    } else {
        eprintln!("  If you need direct conda access, add condabin to your PATH:");
        eprintln!();
        eprintln!(
            "    {}",
            console::style(format!(
                "export PATH=\"{}/condabin:$PATH\"",
                policy::install_path_for_posix_shell()
            ))
            .green()
        );
    }
    eprintln!();
    eprintln!("  Then activate environments with:");
    eprintln!();
    eprintln!(
        "    {}",
        console::style(format!("{} shell myenv", policy::runtime_name())).green()
    );
    eprintln!();
    eprintln!("  Learn more: https://github.com/conda-incubator/conda-spawn");
    std::process::exit(1);
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
    fn test_default_install_path_uses_policy_default() {
        let prefix = policy::default_install_path().unwrap();
        assert_eq!(
            prefix.file_name().unwrap().to_str().unwrap(),
            policy::display_name(),
            "default install path should use policy default"
        );
        assert!(
            prefix.parent().is_some(),
            "prefix should be under a home directory"
        );
    }

    #[test]
    fn test_status_not_bootstrapped() {
        let tmp = TempDir::new().unwrap();
        let result = status(tmp.path());
        assert!(result.is_ok(), "status on empty prefix should succeed");
    }

    #[test]
    fn test_status_bootstrapped_prefix() {
        let tmp = TempDir::new().unwrap();
        let prefix = tmp.path();

        std::fs::create_dir(prefix.join("conda-meta")).unwrap();

        crate::config::write_condarc(prefix, &["conda-forge".to_string()]).unwrap();
        crate::config::write_frozen(prefix).unwrap();
        crate::config::write_metadata(
            prefix,
            &["conda-forge".to_string()],
            &["python >=3.12".to_string(), "conda >=25.1".to_string()],
        )
        .unwrap();

        let result = status(prefix);
        assert!(
            result.is_ok(),
            "status on bootstrapped prefix should succeed"
        );
    }

    #[test]
    fn test_status_refuses_invalid_runtime_metadata() {
        let tmp = TempDir::new().unwrap();
        let prefix = tmp.path();

        std::fs::create_dir(prefix.join("conda-meta")).unwrap();
        std::fs::write(crate::config::metadata_path(prefix), "not json").unwrap();

        let err = status(prefix).unwrap_err().to_string();
        assert!(
            err.contains("Invalid runtime metadata file"),
            "status should reject invalid runtime metadata, got: {err}"
        );
    }

    #[test]
    fn test_uninstall_not_bootstrapped() {
        let tmp = TempDir::new().unwrap();
        let result = uninstall(tmp.path(), true, Verbosity::Normal);
        assert!(
            result.is_ok(),
            "uninstall on empty prefix should succeed with a no-op"
        );
    }

    #[test]
    fn test_uninstall_removes_managed_prefix_without_conda_binary() {
        let tmp = TempDir::new().unwrap();
        let prefix = tmp.path().join("runtime");
        std::fs::create_dir_all(prefix.join("conda-meta")).unwrap();
        std::fs::create_dir_all(prefix.join("envs").join("demo").join("conda-meta")).unwrap();
        crate::config::write_metadata(
            &prefix,
            &["conda-forge".to_string()],
            &["conda".to_string()],
        )
        .unwrap();

        let result = uninstall(&prefix, true, Verbosity::Quiet);

        assert!(result.is_ok(), "uninstall should remove prefix directly");
        assert!(!prefix.exists(), "managed install path should be removed");
    }

    #[test]
    fn test_validate_bootstrap_flags_rejects_missing_bundle_dir() {
        let bundle = Some(std::path::PathBuf::from("/nonexistent/bundle/path"));
        let result = validate_bootstrap_flags(&bundle);
        assert!(result.is_err(), "should fail validation");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("not a directory"),
            "error should contain 'not a directory', got: {err}"
        );
    }

    #[test]
    fn test_validate_bootstrap_flags_valid_bundle() {
        let tmp = TempDir::new().unwrap();
        let bundle = Some(tmp.path().to_path_buf());
        let result = validate_bootstrap_flags(&bundle);
        assert!(result.is_ok(), "valid bundle path should pass validation");
    }

    #[test]
    fn test_validate_bootstrap_flags_no_flags() {
        let result = validate_bootstrap_flags(&None);
        assert!(result.is_ok(), "no flags should pass validation");
    }
}
