//! Runtime distribution policy.
//!
//! The install/runtime code is generic. Values in this module come from the
//! runtime data block stamped onto each generated distribution artifact.

use std::env;
use std::path::{Path, PathBuf};

use crate::runtime_data;

pub(crate) fn runtime_name() -> &'static str {
    &runtime_data::current().header.runtime_name
}

pub(crate) fn runtime_version() -> &'static str {
    &runtime_data::current().header.runtime_version
}

pub(crate) fn embedded_runtime_name() -> &'static str {
    &runtime_data::current().header.embedded_runtime_name
}

pub(crate) fn delegate() -> &'static str {
    &runtime_data::current().header.delegate
}

pub(crate) fn display_name() -> &'static str {
    &runtime_data::current().header.display_name
}

pub(crate) fn install_scheme() -> runtime_data::InstallScheme {
    runtime_data::current().header.install_scheme
}

pub(crate) fn install_name() -> &'static str {
    &runtime_data::current().header.install_name
}

pub(crate) fn install_path_for_display() -> String {
    install_scheme_path_for_display(install_scheme(), install_name())
}

pub(crate) fn metadata_file() -> &'static str {
    &runtime_data::current().header.metadata_file
}

pub(crate) fn bundle_env_var() -> &'static str {
    &runtime_data::current().header.bundle_env_var
}

pub(crate) fn offline_env_var() -> &'static str {
    &runtime_data::current().header.offline_env_var
}

pub(crate) fn docs_url() -> &'static str {
    &runtime_data::current().header.docs_url
}

pub(crate) fn default_install_path() -> miette::Result<PathBuf> {
    install_path_for_scheme(install_scheme(), install_name())
}

pub(crate) fn install_path_for_scheme(
    scheme: runtime_data::InstallScheme,
    install_name: &str,
) -> miette::Result<PathBuf> {
    match scheme {
        runtime_data::InstallScheme::CondaHome => {
            let home = dirs::home_dir()
                .ok_or_else(|| miette::miette!("could not determine home directory"))?;
            Ok(home.join(".conda").join(install_name))
        }
        runtime_data::InstallScheme::UserData => {
            let data_dir = dirs::data_local_dir()
                .ok_or_else(|| miette::miette!("could not determine user data directory"))?;
            Ok(data_dir.join("conda").join(install_name))
        }
    }
}

pub(crate) fn install_scheme_path_for_display(
    scheme: runtime_data::InstallScheme,
    install_name: &str,
) -> String {
    match scheme {
        runtime_data::InstallScheme::CondaHome => format!("~/.conda/{install_name}"),
        runtime_data::InstallScheme::UserData => {
            format!("{}/conda/{install_name}", user_data_dir_for_display())
        }
    }
}

pub(crate) fn expand_install_path(path: impl AsRef<Path>) -> miette::Result<PathBuf> {
    expand_path_str(&path.as_ref().to_string_lossy())
}

pub(crate) fn install_path_for_posix_shell() -> String {
    match install_scheme() {
        runtime_data::InstallScheme::CondaHome => format!("$HOME/.conda/{}", install_name()),
        runtime_data::InstallScheme::UserData => {
            format!(
                "{}/conda/{}",
                user_data_dir_for_posix_shell(),
                install_name()
            )
        }
    }
}

fn user_data_dir_for_display() -> &'static str {
    if cfg!(target_os = "windows") {
        "%LOCALAPPDATA%"
    } else if cfg!(target_os = "macos") {
        "~/Library/Application Support"
    } else {
        "${XDG_DATA_HOME:-~/.local/share}"
    }
}

fn user_data_dir_for_posix_shell() -> &'static str {
    if cfg!(target_os = "macos") {
        "$HOME/Library/Application Support"
    } else {
        "${XDG_DATA_HOME:-$HOME/.local/share}"
    }
}

fn expand_path_str(path: &str) -> miette::Result<PathBuf> {
    let expanded_user = expand_user(path)?;
    let expanded_vars = expand_env_vars(&expanded_user);
    let path = PathBuf::from(expanded_vars);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(env::current_dir()
            .map_err(|err| miette::miette!("could not determine current directory: {err}"))?
            .join(path))
    }
}

fn expand_user(path: &str) -> miette::Result<String> {
    let Some(rest) = path
        .strip_prefix("~/")
        .or_else(|| path.strip_prefix("~\\"))
        .or_else(|| (path == "~").then_some(""))
    else {
        return Ok(path.to_string());
    };

    let home =
        dirs::home_dir().ok_or_else(|| miette::miette!("could not determine home directory"))?;
    Ok(if rest.is_empty() {
        home.to_string_lossy().into_owned()
    } else {
        home.join(rest).to_string_lossy().into_owned()
    })
}

fn expand_env_vars(path: &str) -> String {
    let chars: Vec<char> = path.chars().collect();
    let mut out = String::new();
    let mut index = 0;

    while index < chars.len() {
        match chars[index] {
            '$' if chars.get(index + 1) == Some(&'{') => {
                if let Some(end) = chars[index + 2..].iter().position(|c| *c == '}') {
                    let name: String = chars[index + 2..index + 2 + end].iter().collect();
                    match env::var(&name) {
                        Ok(value) => out.push_str(&value),
                        Err(_) => out.extend(chars[index..=index + 2 + end].iter().copied()),
                    }
                    index += end + 3;
                } else {
                    out.push(chars[index]);
                    index += 1;
                }
            }
            '$' => {
                let start = index + 1;
                let mut end = start;
                while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_')
                {
                    end += 1;
                }
                if end == start {
                    out.push(chars[index]);
                } else {
                    let name: String = chars[start..end].iter().collect();
                    match env::var(&name) {
                        Ok(value) => out.push_str(&value),
                        Err(_) => out.extend(chars[index..end].iter().copied()),
                    }
                }
                index = end;
            }
            '%' => {
                if let Some(end) = chars[index + 1..].iter().position(|c| *c == '%') {
                    let name: String = chars[index + 1..index + 1 + end].iter().collect();
                    match env::var(&name) {
                        Ok(value) if !name.is_empty() => out.push_str(&value),
                        _ => out.extend(chars[index..=index + 1 + end].iter().copied()),
                    }
                    index += end + 2;
                } else {
                    out.push(chars[index]);
                    index += 1;
                }
            }
            c => {
                out.push(c);
                index += 1;
            }
        }
    }

    out
}

pub(crate) fn status_binary_name(has_embedded_bundle: bool) -> &'static str {
    if has_embedded_bundle {
        embedded_runtime_name()
    } else {
        runtime_name()
    }
}

pub(crate) fn frozen_message() -> String {
    format!(
        "This base environment is managed by {display}.\n\
Create a new environment instead: conda create -n myenv\n\
To re-bootstrap: {command} bootstrap --force\n\
To override: pass --override-frozen-env",
        display = display_name(),
        command = runtime_name()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_env_vars_supports_posix_and_windows_forms() {
        temp_env::with_vars(
            [
                ("CONDA_SHIP_TEST_HOME", Some("/tmp/conda-ship-home")),
                ("CONDA_SHIP_TEST_WIN_HOME", Some("C:\\Users\\ship")),
            ],
            || {
                assert_eq!(
                    expand_env_vars("$CONDA_SHIP_TEST_HOME/demo"),
                    "/tmp/conda-ship-home/demo"
                );
                assert_eq!(
                    expand_env_vars("${CONDA_SHIP_TEST_HOME}/demo"),
                    "/tmp/conda-ship-home/demo"
                );
                assert_eq!(
                    expand_env_vars("%CONDA_SHIP_TEST_WIN_HOME%\\demo"),
                    "C:\\Users\\ship\\demo"
                );
            },
        );
    }

    #[test]
    fn test_expand_install_path_absolutizes_relative_paths() {
        let path = expand_install_path("relative-env").unwrap();
        assert!(path.is_absolute());
        assert!(path.ends_with("relative-env"));
    }

    #[test]
    fn test_conda_scheme_is_home_relative() {
        let path = install_path_for_scheme(runtime_data::InstallScheme::CondaHome, install_name())
            .unwrap();
        assert!(path.is_absolute());
        assert!(path.ends_with(PathBuf::from(".conda").join(install_name())));
        assert_ne!(
            path,
            env::current_dir()
                .unwrap()
                .join(format!(".conda/{}", install_name()))
        );
    }

    #[test]
    fn test_data_scheme_is_data_local_relative() {
        let path =
            install_path_for_scheme(runtime_data::InstallScheme::UserData, install_name()).unwrap();
        let expected_base = dirs::data_local_dir().unwrap();
        assert_eq!(path, expected_base.join("conda").join(install_name()));
    }
}
