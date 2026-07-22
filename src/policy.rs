//! Runtime distribution policy.
//!
//! The install/runtime code is generic. Values in this module come from the
//! runtime data block stamped onto each generated distribution artifact.

use std::env;
use std::path::{Path, PathBuf};

use crate::runtime_data;

pub(crate) fn command_name() -> &'static str {
    &runtime_data::current().header.artifact_name
}

pub(crate) fn runtime_name() -> &'static str {
    &runtime_data::current().header.runtime_name
}

pub(crate) fn runtime_version() -> &'static str {
    &runtime_data::current().header.runtime_version
}

pub(crate) fn embedded_artifact_name() -> &'static str {
    &runtime_data::current().header.embedded_artifact_name
}

pub(crate) fn delegate_executable() -> &'static str {
    &runtime_data::current().header.delegate_executable
}

pub(crate) fn installer() -> Option<&'static str> {
    runtime_data::current().header.installer.as_deref()
}

pub(crate) fn display_name() -> &'static str {
    runtime_name()
}

pub(crate) fn install_scheme() -> runtime_data::InstallScheme {
    runtime_data::current().header.install_scheme
}

pub(crate) fn install_name() -> &'static str {
    &runtime_data::current().header.install_name
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

pub(crate) fn prefix_env_var() -> String {
    runtime_data::runtime_env_var(runtime_name(), "PREFIX")
}

pub(crate) const PREFIX_ENV_VAR: &str = "CONDA_SHIP_PREFIX";

pub(crate) fn default_install_path() -> miette::Result<PathBuf> {
    install_path_for_scheme(install_scheme(), install_name())
}

pub(crate) fn install_path() -> miette::Result<PathBuf> {
    let configured = env::var_os(PREFIX_ENV_VAR)
        .filter(|path| !path.is_empty())
        .or_else(|| {
            (runtime_name() != "conda")
                .then(prefix_env_var)
                .and_then(env::var_os)
                .filter(|path| !path.is_empty())
        });
    match configured {
        Some(path) if !path.is_empty() => expand_install_path(path),
        _ => default_install_path(),
    }
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

pub(crate) fn expand_install_path(path: impl AsRef<Path>) -> miette::Result<PathBuf> {
    expand_path_str(&path.as_ref().to_string_lossy())
}

pub(crate) fn path_for_display(path: &Path) -> String {
    normalize_path_display(&path.display().to_string(), std::path::MAIN_SEPARATOR)
}

/// Return the stable path used to invoke this runtime without resolving links.
pub(crate) fn invocation_path() -> miette::Result<PathBuf> {
    let invoked = env::args_os()
        .next()
        .ok_or_else(|| miette::miette!("could not determine runtime invocation path"))?;
    let invoked = PathBuf::from(invoked);
    if invoked.is_absolute() {
        return Ok(invoked);
    }
    if invoked.components().count() > 1 {
        return Ok(env::current_dir()
            .map_err(|error| miette::miette!("could not determine current directory: {error}"))?
            .join(invoked));
    }
    if let Some(path) = env::var_os("PATH") {
        for directory in env::split_paths(&path) {
            let candidate = directory.join(&invoked);
            if candidate.is_file() {
                return Ok(if candidate.is_absolute() {
                    candidate
                } else {
                    env::current_dir()
                        .map_err(|error| {
                            miette::miette!("could not determine current directory: {error}")
                        })?
                        .join(candidate)
                });
            }
        }
    }
    env::current_exe()
        .map_err(|error| miette::miette!("could not determine runtime executable: {error}"))
}

fn normalize_path_display(path: &str, separator: char) -> String {
    if separator == '\\' {
        path.replace('/', "\\")
    } else {
        path.to_string()
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

pub(crate) fn frozen_message() -> String {
    format!(
        "This base environment is managed by {display}.\n\
Create a new environment instead: conda create -n myenv\n\
To override: pass --override-frozen",
        display = display_name(),
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

    #[test]
    fn test_normalize_path_display_uses_windows_separators() {
        assert_eq!(
            normalize_path_display(
                r"D:\a\_temp/distro-bootstrap-smoke-online\conda-meta/history",
                '\\'
            ),
            r"D:\a\_temp\distro-bootstrap-smoke-online\conda-meta\history"
        );
    }

    #[test]
    fn test_normalize_path_display_preserves_unix_paths() {
        assert_eq!(
            normalize_path_display(r"/tmp/prefix\literal/conda-meta/history", '/'),
            r"/tmp/prefix\literal/conda-meta/history"
        );
    }
}
