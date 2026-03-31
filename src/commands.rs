//! Subcommand implementations and prefix helpers.

use std::{env, path::Path};

use miette::{Context, IntoDiagnostic};
use rattler_conda_types::PrefixRecord;

use crate::cli::LockSource;
use crate::config::{
    EMBEDDED_LOCK, embedded_config, read_metadata, write_condarc, write_frozen, write_metadata,
};
use crate::{exec, install};

pub(crate) fn default_prefix() -> miette::Result<std::path::PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| miette::miette!("could not determine home directory"))?;
    Ok(home.join(".cx"))
}

pub(crate) fn is_bootstrapped(prefix: &Path) -> bool {
    prefix.join("conda-meta").is_dir()
}

pub(crate) async fn ensure_bootstrapped(prefix: &Path) -> miette::Result<()> {
    if !is_bootstrapped(prefix) {
        eprintln!(
            "{} No conda installation found. Bootstrapping now...",
            console::style(">>").cyan().bold()
        );
        let cfg = embedded_config();
        bootstrap(
            prefix,
            false,
            None,
            None,
            &cfg.exclude,
            LockSource::Embedded,
            None,
            false,
        )
        .await?;
    }
    Ok(())
}

pub(crate) fn validate_bootstrap_flags(
    offline: bool,
    no_lock: bool,
    payload: &Option<std::path::PathBuf>,
) -> miette::Result<()> {
    if offline && no_lock {
        return Err(miette::miette!(
            "--offline and --no-lock are incompatible (offline mode requires a lockfile)"
        ));
    }
    if let Some(dir) = payload
        && !dir.is_dir()
    {
        return Err(miette::miette!(
            "--payload path is not a directory: {}",
            dir.display()
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
    excludes: &[String],
    lock_source: LockSource,
    payload: Option<std::path::PathBuf>,
    offline: bool,
) -> miette::Result<()> {
    if is_bootstrapped(prefix) && !force {
        eprintln!(
            "{} conda is already bootstrapped at {}",
            console::style("✔").green(),
            prefix.display()
        );
        eprintln!("  Use --force to re-bootstrap.");
        return Ok(());
    }

    if force && prefix.exists() {
        eprintln!(
            "{} Removing existing prefix at {}",
            console::style(">>").cyan().bold(),
            prefix.display()
        );
        std::fs::remove_dir_all(prefix).into_diagnostic()?;
    }

    let cfg = embedded_config();
    let channels = channels.unwrap_or(cfg.channels);
    let mut specs = cfg.packages;
    if let Some(extra) = extra_packages {
        specs.extend(extra);
    }

    eprintln!(
        "{} Bootstrapping conda into {}",
        console::style(">>").cyan().bold(),
        prefix.display()
    );
    eprintln!("   Channels: {}", channels.join(", "));
    eprintln!("   Packages: {}", specs.join(", "));
    if !excludes.is_empty() {
        eprintln!("   Exclude:  {}", excludes.join(", "));
    }
    if offline {
        eprintln!("   Mode:     offline");
    }

    let lock_content = match &lock_source {
        LockSource::Embedded => {
            if !EMBEDDED_LOCK.is_empty() {
                eprintln!("   Using embedded lockfile");
                Some(EMBEDDED_LOCK.to_string())
            } else {
                None
            }
        }
        LockSource::File(path) => {
            let content = std::fs::read_to_string(path)
                .into_diagnostic()
                .context("failed to read lockfile")?;
            eprintln!("   Using lockfile: {}", path.display());
            Some(content)
        }
        LockSource::None => {
            eprintln!("   Live solve (lockfile disabled)");
            None
        }
    };

    if let Some(ref payload_dir) = payload {
        let content = lock_content.ok_or_else(|| {
            miette::miette!("--payload requires a lockfile (embedded or --lockfile)")
        })?;
        eprintln!("   Payload:  {}", payload_dir.display());
        install::from_lockfile_with_payload(prefix, &content, excludes, payload_dir, offline)
            .await?;
    } else if let Some(embedded_dir) = install::extract_embedded_payload()? {
        let content =
            lock_content.ok_or_else(|| miette::miette!("embedded payload requires a lockfile"))?;
        eprintln!("   Payload:  embedded");
        let result =
            install::from_lockfile_with_payload(prefix, &content, excludes, &embedded_dir, true)
                .await;
        let _ = std::fs::remove_dir_all(&embedded_dir);
        result?;
    } else if offline {
        let content = lock_content.ok_or_else(|| {
            miette::miette!("--offline requires a lockfile (embedded or --lockfile)")
        })?;
        install::from_lockfile_offline(prefix, &content, excludes).await?;
    } else {
        match &lock_content {
            Some(content) => install::from_lockfile(prefix, content, excludes).await?,
            None => install::from_solve(prefix, &channels, &specs, excludes).await?,
        };
    }

    write_condarc(prefix)?;
    write_frozen(prefix)?;
    write_metadata(prefix, &channels, &specs, excludes)?;

    compile_python_bytecode(prefix);

    eprintln!(
        "\n{} conda bootstrapped successfully!",
        console::style("✔").green().bold()
    );
    eprintln!("   Prefix: {}", prefix.display());
    eprintln!("   Run `cx status` for details.");
    eprintln!("   Use `cx <conda-args>` to run conda commands.");

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

    let meta = read_metadata(prefix)?;

    let payload = crate::config::EMBEDDED_PAYLOAD;
    let binary_name = if payload.is_empty() { "cx" } else { "cxz" };
    println!("{} {}", binary_name, env!("CARGO_PKG_VERSION"));
    println!("  prefix:   {}", prefix.display());
    println!("  channels: {}", meta.channels.join(", "));
    println!("  packages: {}", meta.packages.join(", "));
    if !meta.excludes.is_empty() {
        println!("  excludes: {}", meta.excludes.join(", "));
    }
    if !payload.is_empty() {
        println!(
            "  payload:  embedded ({:.1} MB)",
            payload.len() as f64 / 1_048_576.0
        );
    }

    let installed = PrefixRecord::collect_from_prefix::<PrefixRecord>(prefix).into_diagnostic()?;
    println!("  installed: {} packages", installed.len());

    let conda_bin = exec::conda_binary(prefix);
    println!(
        "  conda:    {}",
        if conda_bin.exists() {
            conda_bin.display().to_string()
        } else {
            "(not found)".to_string()
        }
    );

    Ok(())
}

pub(crate) fn uninstall(prefix: &Path, yes: bool) -> miette::Result<()> {
    if !is_bootstrapped(prefix) {
        eprintln!(
            "{} No conda installation found at {}",
            console::style("!").yellow().bold(),
            prefix.display()
        );
        eprintln!("  Nothing to uninstall.");
        return Ok(());
    }

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

    let cx_binary = env::current_exe().ok();

    eprintln!(
        "{} This will permanently remove:",
        console::style("!").yellow().bold()
    );
    eprintln!("   Conda prefix: {}", prefix.display());
    if !named_envs.is_empty() {
        eprintln!(
            "   Named environments ({}): {}",
            named_envs.len(),
            named_envs.join(", ")
        );
    }
    if let Some(ref bin) = cx_binary {
        eprintln!("   cx binary: {}", bin.display());
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

    eprintln!(
        "\n{} Removing conda prefix at {}",
        console::style(">>").cyan().bold(),
        prefix.display()
    );
    std::fs::remove_dir_all(prefix)
        .into_diagnostic()
        .context("failed to remove conda prefix")?;

    if let Some(ref bin) = cx_binary
        && bin.exists()
    {
        eprintln!(
            "{} Removing cx binary at {}",
            console::style(">>").cyan().bold(),
            bin.display()
        );
        std::fs::remove_file(bin)
            .into_diagnostic()
            .context("failed to remove cx binary")?;
    }

    remove_shell_path_entries(prefix);

    eprintln!(
        "\n{} cx has been uninstalled.",
        console::style("✔").green().bold()
    );

    Ok(())
}

fn remove_shell_path_entries(prefix: &Path) {
    let condabin_path = format!("export PATH=\"{}/condabin:$PATH\"", prefix.display());
    let install_dir = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let install_path = install_dir
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

pub(crate) fn print_disabled_shell_command(command: &str) {
    eprintln!(
        "{} `conda {command}` is not available in cx.",
        console::style("!").yellow().bold()
    );
    eprintln!();
    eprintln!("  cx uses conda-spawn for environment activation.");
    eprintln!("  Instead of `conda activate myenv`, run:");
    eprintln!();
    eprintln!("    {}", console::style("cx shell myenv").green());
    eprintln!();
    eprintln!("  To leave the environment, exit the subshell (Ctrl+D or `exit`).");
    eprintln!();
    eprintln!("  Learn more: https://github.com/conda-incubator/conda-spawn");
    std::process::exit(1);
}

pub(crate) fn print_disabled_init() {
    eprintln!(
        "{} `conda init` is not needed with cx.",
        console::style("!").yellow().bold()
    );
    eprintln!();
    eprintln!("  cx uses conda-spawn, which does not require shell");
    eprintln!("  profile modifications. Just add condabin to your PATH:");
    eprintln!();
    eprintln!(
        "    {}",
        console::style("export PATH=\"$HOME/.cx/condabin:$PATH\"").green()
    );
    eprintln!();
    eprintln!("  Then activate environments with:");
    eprintln!();
    eprintln!("    {}", console::style("cx shell myenv").green());
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
    fn test_default_prefix_ends_with_cx() {
        let prefix = default_prefix().unwrap();
        assert_eq!(
            prefix.file_name().unwrap().to_str().unwrap(),
            ".cx",
            "default prefix should be ~/.cx"
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

        crate::config::write_condarc(prefix).unwrap();
        crate::config::write_frozen(prefix).unwrap();
        crate::config::write_metadata(
            prefix,
            &["conda-forge".to_string()],
            &["python >=3.12".to_string(), "conda >=25.1".to_string()],
            &["conda-libmamba-solver".to_string()],
        )
        .unwrap();

        let result = status(prefix);
        assert!(
            result.is_ok(),
            "status on bootstrapped prefix should succeed"
        );
    }

    #[test]
    fn test_uninstall_not_bootstrapped() {
        let tmp = TempDir::new().unwrap();
        let result = uninstall(tmp.path(), true);
        assert!(
            result.is_ok(),
            "uninstall on empty prefix should succeed with a no-op"
        );
    }

    #[test]
    fn test_clean_path_entries_removes_condabin() {
        let tmp = TempDir::new().unwrap();
        let condabin_line = "export PATH=\"/home/user/.cx/condabin:$PATH\"";
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
    fn test_clean_path_entries_removes_install_dir() {
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
            "install dir line should be removed"
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
    #[case::offline_no_lock(true, true, false, "incompatible")]
    #[case::payload_missing_dir(false, false, true, "not a directory")]
    fn test_validate_bootstrap_flags(
        #[case] offline: bool,
        #[case] no_lock: bool,
        #[case] bad_payload_path: bool,
        #[case] expected_err_contains: &str,
    ) {
        let payload = if bad_payload_path {
            Some(std::path::PathBuf::from("/nonexistent/payload/dir"))
        } else {
            None
        };
        let result = validate_bootstrap_flags(offline, no_lock, &payload);
        assert!(result.is_err(), "should fail validation");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains(expected_err_contains),
            "error should contain '{expected_err_contains}', got: {err}"
        );
    }

    #[test]
    fn test_validate_bootstrap_flags_valid_payload() {
        let tmp = TempDir::new().unwrap();
        let payload = Some(tmp.path().to_path_buf());
        let result = validate_bootstrap_flags(false, false, &payload);
        assert!(result.is_ok(), "valid payload dir should pass validation");
    }

    #[test]
    fn test_validate_bootstrap_flags_no_flags() {
        let result = validate_bootstrap_flags(false, false, &None);
        assert!(result.is_ok(), "no flags should pass validation");
    }
}
