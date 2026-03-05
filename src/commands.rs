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
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn bootstrap(
    prefix: &Path,
    force: bool,
    channels: Option<Vec<String>>,
    extra_packages: Option<Vec<String>>,
    excludes: &[String],
    lock_source: LockSource,
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

    match &lock_content {
        Some(content) => install::from_lockfile(prefix, content, excludes).await?,
        None => install::from_solve(prefix, &channels, &specs, excludes).await?,
    };

    write_condarc(prefix)?;
    write_frozen(prefix)?;
    write_metadata(prefix, &channels, &specs, excludes)?;

    eprintln!(
        "\n{} conda bootstrapped successfully!",
        console::style("✔").green().bold()
    );
    eprintln!("   Prefix: {}", prefix.display());
    eprintln!("   Run `cx status` for details.");
    eprintln!("   Use `cx <conda-args>` to run conda commands.");

    Ok(())
}

pub(crate) fn status(prefix: &Path) -> miette::Result<()> {
    if !is_bootstrapped(prefix) {
        eprintln!("No conda installation found at {}", prefix.display());
        return Ok(());
    }

    let meta = read_metadata(prefix)?;

    println!("cx {}", env!("CARGO_PKG_VERSION"));
    println!("  prefix:   {}", prefix.display());
    println!("  channels: {}", meta.channels.join(", "));
    println!("  packages: {}", meta.packages.join(", "));
    if !meta.excludes.is_empty() {
        println!("  excludes: {}", meta.excludes.join(", "));
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

    let profiles: Vec<std::path::PathBuf> = vec![
        home.join(".bashrc"),
        home.join(".zshrc"),
        home.join(".config/fish/config.fish"),
    ];

    for profile in &profiles {
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
                let dominated = line.trim() == condabin_path
                    || install_path.as_ref().is_some_and(|p| line.trim() == p);
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
}
