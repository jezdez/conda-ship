//! Subcommand implementations and prefix helpers.

use std::process::Stdio;
use std::{env, path::Path};

use miette::{Context, IntoDiagnostic};
use rattler_conda_types::PrefixRecord;

use crate::cli::{LockSource, Verbosity};
use crate::config::{
    PrefixMetadata, embedded_config, embedded_lock, read_metadata, write_condarc, write_frozen,
    write_metadata,
};
use crate::{exec, install, policy};

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

    read_metadata(prefix).map_err(|err| {
        miette::miette!(
            "refusing to {action} unmanaged install path: {}\n  Invalid runtime metadata file: {}\n  {err}",
            prefix.display(),
            metadata_path.display()
        )
    })
}

pub(crate) async fn ensure_bootstrapped(prefix: &Path) -> miette::Result<()> {
    if is_bootstrapped(prefix) {
        require_managed_prefix(prefix, "use")?;
    } else {
        eprintln!(
            "{} No conda installation found. Bootstrapping now...",
            console::style(">>").cyan().bold()
        );
        bootstrap(
            prefix,
            false,
            None,
            None,
            LockSource::Embedded,
            None,
            false,
            Verbosity::Quiet,
        )
        .await?;
    }
    Ok(())
}

fn reject_dangerous_prefix(prefix: &Path) -> miette::Result<()> {
    let home = dirs::home_dir();
    let canon = prefix
        .canonicalize()
        .unwrap_or_else(|_| prefix.to_path_buf());

    let dangerous = canon == Path::new("/")
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

pub(crate) fn validate_bootstrap_flags(
    offline: bool,
    no_lock: bool,
    lockfile: &Option<std::path::PathBuf>,
    bundle: &Option<std::path::PathBuf>,
    channels: &Option<Vec<String>>,
    packages: &Option<Vec<String>>,
) -> miette::Result<()> {
    if no_lock && lockfile.is_some() {
        return Err(miette::miette!(
            "--no-lock and --lockfile are incompatible (choose one package source)"
        ));
    }
    if offline && no_lock {
        return Err(miette::miette!(
            "--offline and --no-lock are incompatible (offline mode requires a lockfile)"
        ));
    }
    if no_lock && bundle.is_some() {
        return Err(miette::miette!(
            "--bundle and --no-lock are incompatible (bundles require a lockfile)"
        ));
    }
    if !no_lock && (channels.is_some() || packages.is_some()) {
        return Err(miette::miette!(
            "--channel and --package only affect live solves; pass --no-lock or update the project lockfile"
        ));
    }
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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn bootstrap(
    prefix: &Path,
    force: bool,
    channels: Option<Vec<String>>,
    extra_packages: Option<Vec<String>>,
    lock_source: LockSource,
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
        std::fs::remove_dir_all(prefix).into_diagnostic()?;
    }

    let cfg = embedded_config();
    let channels = channels.unwrap_or_else(|| cfg.channels.clone());
    let mut specs = cfg.packages.clone();
    if let Some(extra) = extra_packages {
        specs.extend(extra);
    }

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

    let lock_content = match &lock_source {
        LockSource::Embedded => {
            if let Some(lock) = embedded_lock() {
                if verbosity != Verbosity::Quiet {
                    eprintln!("   Using embedded lockfile");
                }
                Some(lock.to_string())
            } else {
                None
            }
        }
        LockSource::File(path) => {
            let content = std::fs::read_to_string(path)
                .into_diagnostic()
                .context("failed to read lockfile")?;
            if verbosity != Verbosity::Quiet {
                eprintln!("   Using lockfile: {}", path.display());
            }
            Some(content)
        }
        LockSource::None => {
            if verbosity != Verbosity::Quiet {
                eprintln!("   Live solve (lockfile disabled)");
            }
            None
        }
    };

    if let Some(ref bundle_dir) = bundle {
        let content = lock_content.ok_or_else(|| {
            miette::miette!("--bundle requires a lockfile (embedded or --lockfile)")
        })?;
        if verbosity != Verbosity::Quiet {
            eprintln!("   Bundle:   {}", bundle_dir.display());
        }
        install::from_lockfile_with_bundle(prefix, &content, bundle_dir, offline).await?;
    } else if let Some(embedded_dir) = install::extract_embedded_bundle()? {
        let content =
            lock_content.ok_or_else(|| miette::miette!("embedded bundle requires a lockfile"))?;
        if verbosity != Verbosity::Quiet {
            eprintln!("   Bundle:   embedded");
        }
        let result =
            install::from_lockfile_with_bundle(prefix, &content, &embedded_dir, true).await;
        let _ = std::fs::remove_dir_all(&embedded_dir);
        result?;
    } else if offline {
        let content = lock_content.ok_or_else(|| {
            miette::miette!("--offline requires a lockfile (embedded or --lockfile)")
        })?;
        install::from_lockfile_offline(prefix, &content).await?;
    } else {
        match &lock_content {
            Some(content) => install::from_lockfile(prefix, content).await?,
            None => install::from_solve(prefix, &channels, &specs).await?,
        };
    }

    write_condarc(prefix, &channels)?;
    write_frozen(prefix)?;
    write_metadata(prefix, &channels, &specs)?;

    compile_python_bytecode(prefix);

    if verbosity != Verbosity::Quiet {
        eprintln!(
            "\n{} conda bootstrapped successfully!",
            console::style("✔").green().bold()
        );
        eprintln!("   Install path: {}", prefix.display());
        eprintln!("   Run `{} status` for details.", policy::command_name());
        eprintln!(
            "   Use `{} <conda-args>` to run conda commands.",
            policy::command_name()
        );
    }

    Ok(())
}

fn compile_python_bytecode(prefix: &Path) {
    let python = prefix.join("bin").join("python");
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
    println!("{} {}", binary_name, env!("CARGO_PKG_VERSION"));
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

    let conda_bin = exec::conda_binary(prefix);
    println!(
        "  conda:     {}",
        if conda_bin.exists() {
            conda_bin.display().to_string()
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

    if !named_envs.is_empty() {
        let conda = crate::exec::conda_binary(prefix);
        for env_name in &named_envs {
            if verbosity != Verbosity::Quiet {
                eprintln!(
                    "{} Removing environment: {}",
                    console::style(">>").cyan().bold(),
                    env_name
                );
            }
            let output = std::process::Command::new(&conda)
                .args(["remove", "--all", "-n", env_name, "-y", "--json"])
                .env("CONDA_ROOT_PREFIX", prefix)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output();
            match output {
                Ok(out) if out.status.success() => {
                    if verbosity == Verbosity::Verbose
                        && let Ok(json) = serde_json::from_slice::<serde_json::Value>(&out.stdout)
                        && let Some(actions) = json
                            .get("actions")
                            .and_then(|a| a.get("UNLINK"))
                            .and_then(|u| u.as_array())
                    {
                        for pkg in actions {
                            if let Some(name) = pkg.get("name").and_then(|n| n.as_str()) {
                                let version =
                                    pkg.get("version").and_then(|v| v.as_str()).unwrap_or("?");
                                eprintln!("     - {name} {version}");
                            }
                        }
                    }
                }
                Ok(out) => {
                    eprintln!(
                        "{} conda exited with {} for {} (will force-remove it)",
                        console::style("!").yellow().bold(),
                        out.status,
                        env_name
                    );
                }
                Err(e) => {
                    eprintln!(
                        "{} Failed to run conda for {}: {} (will force-remove it)",
                        console::style("!").yellow().bold(),
                        env_name,
                        e
                    );
                }
            }
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
    std::fs::remove_dir_all(prefix)
        .into_diagnostic()
        .context("failed to remove install path")?;

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
            policy::command_name()
        );
        eprintln!("{hint}");
    }

    remove_shell_path_entries(prefix);

    if verbosity != Verbosity::Quiet {
        eprintln!(
            "\n{} {} has been uninstalled.",
            console::style("✔").green().bold(),
            policy::display_name()
        );
    }

    Ok(())
}

fn remove_shell_path_entries(prefix: &Path) {
    let condabin_path = format!("export PATH=\"{}/condabin:$PATH\"", prefix.display());
    let install_path = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let install_path = install_path
        .as_ref()
        .map(|d| format!("export PATH=\"{}:$PATH\"", d.display()));

    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return,
    };

    let profiles = vec![
        home.join(".bashrc"),
        home.join(".zshrc"),
        home.join(".config/fish/config.fish"),
    ];

    clean_path_entries_from_profiles(&profiles, &condabin_path, install_path.as_deref());
}

pub(crate) fn clean_path_entries_from_profiles(
    profiles: &[std::path::PathBuf],
    condabin_path: &str,
    install_path: Option<&str>,
) {
    for profile in profiles {
        if !profile.exists() {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(profile) else {
            continue;
        };

        let mut changed = false;
        let filtered: Vec<&str> = contents
            .lines()
            .filter(|line| {
                let dominated =
                    line.trim() == condabin_path || install_path.is_some_and(|p| line.trim() == p);
                if dominated {
                    changed = true;
                }
                !dominated
            })
            .collect();

        if changed {
            let new_contents = filtered.join("\n");
            if std::fs::write(profile, &new_contents).is_ok() {
                eprintln!(
                    "{} Cleaned PATH entry from {}",
                    console::style(">>").cyan().bold(),
                    profile.display()
                );
            }
        }
    }
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
        console::style(format!("{} shell myenv", policy::command_name())).green()
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
    eprintln!("  profile modifications. Just add condabin to your PATH:");
    eprintln!();
    eprintln!(
        "    {}",
        console::style(format!(
            "export PATH=\"{}/condabin:$PATH\"",
            policy::install_path_for_posix_shell()
        ))
        .green()
    );
    eprintln!();
    eprintln!("  Then activate environments with:");
    eprintln!();
    eprintln!(
        "    {}",
        console::style(format!("{} shell myenv", policy::command_name())).green()
    );
    eprintln!();
    eprintln!("  Learn more: https://github.com/conda-incubator/conda-spawn");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
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
    fn test_clean_path_entries_removes_condabin() {
        let tmp = TempDir::new().unwrap();
        let condabin_line = "export PATH=\"/home/user/.demo/condabin:$PATH\"";
        let bashrc = tmp.path().join(".bashrc");
        std::fs::write(
            &bashrc,
            format!("# my bashrc\n{condabin_line}\nalias ll='ls -la'\n"),
        )
        .unwrap();

        clean_path_entries_from_profiles(std::slice::from_ref(&bashrc), condabin_line, None);

        let result = std::fs::read_to_string(&bashrc).unwrap();
        assert!(
            !result.contains("condabin"),
            "condabin line should be removed"
        );
        assert!(
            result.contains("alias ll"),
            "other lines should be preserved"
        );
    }

    #[test]
    fn test_clean_path_entries_removes_install_path() {
        let tmp = TempDir::new().unwrap();
        let install_line = "export PATH=\"/usr/local/bin:$PATH\"";
        let zshrc = tmp.path().join(".zshrc");
        std::fs::write(
            &zshrc,
            format!("# zshrc\n{install_line}\nexport EDITOR=vim\n"),
        )
        .unwrap();

        clean_path_entries_from_profiles(
            std::slice::from_ref(&zshrc),
            "export PATH=\"/unused/condabin:$PATH\"",
            Some(install_line),
        );

        let result = std::fs::read_to_string(&zshrc).unwrap();
        assert!(
            !result.contains("/usr/local/bin"),
            "install path line should be removed"
        );
        assert!(
            result.contains("EDITOR=vim"),
            "other lines should be preserved"
        );
    }

    #[test]
    fn test_clean_path_entries_skips_missing_profiles() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("nonexistent");
        clean_path_entries_from_profiles(&[missing], "whatever", None);
    }

    #[test]
    fn test_clean_path_entries_no_change_when_no_match() {
        let tmp = TempDir::new().unwrap();
        let bashrc = tmp.path().join(".bashrc");
        let original = "# my bashrc\nalias ll='ls -la'\n";
        std::fs::write(&bashrc, original).unwrap();

        clean_path_entries_from_profiles(
            std::slice::from_ref(&bashrc),
            "export PATH=\"/not/present:$PATH\"",
            None,
        );

        let result = std::fs::read_to_string(&bashrc).unwrap();
        assert_eq!(result, original, "file should be unchanged when no match");
    }

    #[rstest]
    #[case::offline_no_lock(true, true, false, false, false, "incompatible")]
    #[case::bundle_missing_dir(false, false, true, false, false, "not a directory")]
    #[case::no_lock_with_lockfile(false, true, false, true, false, "choose one package source")]
    #[case::locked_with_package(false, false, false, false, true, "only affect live solves")]
    fn test_validate_bootstrap_flags(
        #[case] offline: bool,
        #[case] no_lock: bool,
        #[case] bad_bundle_path: bool,
        #[case] has_lockfile: bool,
        #[case] has_package: bool,
        #[case] expected_err_contains: &str,
    ) {
        let bundle = if bad_bundle_path {
            Some(std::path::PathBuf::from("/nonexistent/bundle/path"))
        } else {
            None
        };
        let lockfile = has_lockfile.then(|| std::path::PathBuf::from("runtime.lock"));
        let packages = has_package.then(|| vec!["numpy".to_string()]);
        let result =
            validate_bootstrap_flags(offline, no_lock, &lockfile, &bundle, &None, &packages);
        assert!(result.is_err(), "should fail validation");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains(expected_err_contains),
            "error should contain '{expected_err_contains}', got: {err}"
        );
    }

    #[test]
    fn test_validate_bootstrap_flags_valid_bundle() {
        let tmp = TempDir::new().unwrap();
        let bundle = Some(tmp.path().to_path_buf());
        let result = validate_bootstrap_flags(false, false, &None, &bundle, &None, &None);
        assert!(result.is_ok(), "valid bundle path should pass validation");
    }

    #[test]
    fn test_validate_bootstrap_flags_no_flags() {
        let result = validate_bootstrap_flags(false, false, &None, &None, &None, &None);
        assert!(result.is_ok(), "no flags should pass validation");
    }
}
