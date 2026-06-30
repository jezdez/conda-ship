//! Experimental APIs for managing multiple locked conda runtimes.
//!
//! `conda-fleet` is an optional API layer for tools that want to install and
//! inspect several conda-ship-managed prefixes. It does not solve
//! environments, choose catalogs, create shims, or mutate a user's global
//! `PATH`.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

use miette::{Context, IntoDiagnostic};
use rattler_lock::LockFile;

use crate::{config, constructor_metadata, exec, hash, install, policy};

const FLEET_COMMAND_NAME: &str = "conda-fleet";

/// Configuration for a [`Fleet`] instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FleetConfig {
    /// Directory that contains one managed runtime prefix per direct child.
    pub install_root: PathBuf,
}

impl FleetConfig {
    /// Create a fleet configuration with the given install root.
    pub fn new(install_root: impl Into<PathBuf>) -> Self {
        Self {
            install_root: install_root.into(),
        }
    }
}

/// Manager for multiple conda-ship-managed runtime prefixes.
#[derive(Clone, Debug)]
pub struct Fleet {
    config: FleetConfig,
}

impl Fleet {
    /// Create a new fleet manager.
    pub fn new(config: FleetConfig) -> Self {
        Self { config }
    }

    /// Install a locked runtime into `install_root/<id>`.
    ///
    /// The spec must already contain a resolved lockfile. Fleet does not solve
    /// environments or look up packages in a catalog.
    pub async fn install(
        &self,
        spec: RuntimeSpec,
        options: InstallOptions,
    ) -> miette::Result<InstalledRuntime> {
        spec.validate()?;
        validate_bundle_options(options.bundle_dir.as_deref())?;
        let lock_sha256 = lock_sha256(&spec.lock_content);
        let channels = channels_from_lock_content(&spec.lock_content)?;

        let prefix = self.prefix_for(&spec.id)?;
        std::fs::create_dir_all(&self.config.install_root)
            .into_diagnostic()
            .with_context(|| {
                format!(
                    "failed to create fleet install root {}",
                    policy::path_for_display(&self.config.install_root)
                )
            })?;

        if prefix.exists() {
            if is_bootstrapped(&prefix) {
                if !options.force {
                    return self.read_installed_runtime(&prefix, &spec.id, "use");
                }
                self.read_installed_runtime(&prefix, &spec.id, "replace")?;
            } else if !is_empty_dir(&prefix)? {
                return Err(miette::miette!(
                    "refusing to install into unmanaged non-empty prefix: {}",
                    policy::path_for_display(&prefix)
                ));
            }
        }

        if options.force && prefix.exists() {
            reject_dangerous_prefix(&prefix)?;
            if !is_empty_dir(&prefix)? {
                self.read_installed_runtime(&prefix, &spec.id, "remove")?;
            }
            remove_install_path(&prefix)?;
        }

        if let Some(ref bundle_dir) = options.bundle_dir {
            install::from_lockfile_with_bundle_and_specs(
                &prefix,
                &spec.lock_content,
                &spec.requested_specs,
                bundle_dir,
                options.offline,
            )
            .await?;
        } else if options.offline {
            install::from_lockfile_offline_with_specs(
                &prefix,
                &spec.lock_content,
                &spec.requested_specs,
            )
            .await?;
        } else {
            install::from_lockfile_with_specs(&prefix, &spec.lock_content, &spec.requested_specs)
                .await?;
        }

        constructor_metadata::write_prefix_metadata_with_command(
            &prefix,
            &spec.lock_content,
            &spec.requested_specs,
            FLEET_COMMAND_NAME,
        )?;
        config::write_condarc(&prefix, &channels)?;
        config::write_frozen_with_message(&prefix, &fleet_frozen_message(&spec.id))?;
        config::write_metadata_for_identity(
            &prefix,
            config::PrefixMetadataIdentity {
                display_name: &spec.id,
                install_name: &spec.id,
                metadata_file: &metadata_file_for(&spec.id),
                version: &spec.version,
                delegate_executable: Some(&spec.delegate_executable),
                lock_sha256: Some(&lock_sha256),
            },
            &channels,
            &spec.requested_specs,
        )?;

        if options.compile_python_bytecode {
            compile_python_bytecode(&prefix);
        }

        self.read_installed_runtime(&prefix, &spec.id, "inspect")
    }

    /// List runtimes with valid fleet metadata under the install root.
    ///
    /// Directories without valid metadata are ignored. Fleet does not maintain
    /// a separate registry database.
    pub fn list(&self) -> miette::Result<Vec<InstalledRuntime>> {
        if !self.config.install_root.is_dir() {
            return Ok(Vec::new());
        }

        let mut runtimes = Vec::new();
        let entries = std::fs::read_dir(&self.config.install_root)
            .into_diagnostic()
            .with_context(|| {
                format!(
                    "failed to read fleet install root {}",
                    policy::path_for_display(&self.config.install_root)
                )
            })?;

        for entry in entries {
            let entry = entry.into_diagnostic()?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(id) = entry.file_name().to_str().map(ToString::to_string) else {
                continue;
            };
            if validate_runtime_id(&id).is_err() {
                continue;
            }
            if let Ok(Some(runtime)) = self.status(&id) {
                runtimes.push(runtime);
            }
        }

        runtimes.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(runtimes)
    }

    /// Return status for a managed runtime by id.
    pub fn status(&self, id: &str) -> miette::Result<Option<InstalledRuntime>> {
        validate_runtime_id(id)?;
        let prefix = self.prefix_for(id)?;
        let metadata_file = metadata_file_for(id);
        let metadata_path = config::metadata_path_for(&prefix, &metadata_file);
        if !metadata_path.is_file() {
            return Ok(None);
        }
        self.read_installed_runtime(&prefix, id, "inspect")
            .map(Some)
    }

    /// Remove a managed runtime prefix by id.
    ///
    /// Fleet refuses to remove unmanaged non-empty prefixes.
    pub fn remove(&self, id: &str) -> miette::Result<()> {
        validate_runtime_id(id)?;
        let prefix = self.prefix_for(id)?;
        if !prefix.exists() {
            return Ok(());
        }

        reject_dangerous_prefix(&prefix)?;
        if is_bootstrapped(&prefix) {
            self.read_installed_runtime(&prefix, id, "remove")?;
        } else if !is_empty_dir(&prefix)? {
            return Err(miette::miette!(
                "refusing to remove unmanaged non-empty prefix: {}",
                policy::path_for_display(&prefix)
            ));
        }

        remove_install_path(&prefix)
    }

    fn prefix_for(&self, id: &str) -> miette::Result<PathBuf> {
        validate_runtime_id(id)?;
        Ok(self.config.install_root.join(id))
    }

    fn read_installed_runtime(
        &self,
        prefix: &Path,
        id: &str,
        action: &str,
    ) -> miette::Result<InstalledRuntime> {
        let metadata_file = metadata_file_for(id);
        let metadata_path = config::metadata_path_for(prefix, &metadata_file);
        if !metadata_path.is_file() {
            return Err(miette::miette!(
                "refusing to {action} unmanaged fleet prefix: {}\n  Expected runtime metadata file: {}",
                policy::path_for_display(prefix),
                policy::path_for_display(&metadata_path)
            ));
        }

        let meta = config::read_metadata_for(prefix, &metadata_file).map_err(|err| {
            miette::miette!(
                "refusing to {action} unmanaged fleet prefix: {}\n  Invalid runtime metadata file: {}\n  {err}",
                policy::path_for_display(prefix),
                policy::path_for_display(&metadata_path)
            )
        })?;
        config::validate_metadata_identity_for(&meta, id, id, &metadata_file).map_err(|err| {
            miette::miette!(
                "refusing to {action} fleet prefix owned by a different runtime: {}\n  Invalid runtime metadata file: {}\n  {err}",
                policy::path_for_display(prefix),
                policy::path_for_display(&metadata_path)
            )
        })?;

        Ok(InstalledRuntime::from_metadata(prefix.to_path_buf(), meta))
    }
}

/// Fully resolved runtime input accepted by [`Fleet::install`].
///
/// `RuntimeSpec` is the programmatic boundary between a downstream
/// orchestrator and fleet. It is not a user-facing catalog format: callers are
/// expected to derive it from their own catalog, policy layer, downloaded
/// descriptor, or conda-ship-generated runtime metadata.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSpec {
    /// Stable runtime id and on-disk directory name under the install root.
    pub id: String,
    /// Runtime version recorded in conda-ship prefix metadata.
    pub version: String,
    /// Default executable that callers usually expose for this runtime.
    pub delegate_executable: String,
    /// Resolved rattler-lock content for the runtime environment.
    pub lock_content: String,
    /// Requested specs recorded in `conda-meta/history` and prefix metadata.
    #[serde(default)]
    pub requested_specs: Vec<String>,
}

impl RuntimeSpec {
    /// Validate runtime identity and command names before installation.
    pub fn validate(&self) -> miette::Result<()> {
        validate_runtime_id(&self.id)?;
        validate_command_name(&self.delegate_executable)?;
        if self.version.trim().is_empty() {
            return Err(miette::miette!("runtime version must not be empty"));
        }
        if self.lock_content.trim().is_empty() {
            return Err(miette::miette!("runtime lock_content must not be empty"));
        }
        Ok(())
    }

    /// Return the SHA256 digest of this runtime's lock content.
    pub fn lock_sha256(&self) -> String {
        lock_sha256(&self.lock_content)
    }
}

/// Options for installing a runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallOptions {
    /// Replace an existing managed runtime with the same id.
    pub force: bool,
    /// Install without network access. Packages must already be in the shared
    /// rattler cache or in `bundle_dir`.
    pub offline: bool,
    /// Optional directory containing `.conda` or `.tar.bz2` package archives.
    pub bundle_dir: Option<PathBuf>,
    /// Compile Python bytecode after installation when a Python executable is
    /// present in the runtime.
    pub compile_python_bytecode: bool,
}

impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            force: false,
            offline: false,
            bundle_dir: None,
            compile_python_bytecode: true,
        }
    }
}

/// Runtime prefix discovered or installed by [`Fleet`].
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct InstalledRuntime {
    /// Runtime id and directory name under the fleet install root.
    pub id: String,
    /// Version recorded in conda-ship prefix metadata.
    pub version: String,
    /// Managed conda prefix path.
    pub prefix: PathBuf,
    /// Default executable that callers usually expose for this runtime.
    pub delegate_executable: String,
    /// Channels recorded in the runtime `.condarc`.
    pub channels: Vec<String>,
    /// SHA256 digest of the lock content used for installation, when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_sha256: Option<String>,
    /// Requested specs recorded at install time.
    pub requested_specs: Vec<String>,
}

impl InstalledRuntime {
    fn from_metadata(prefix: PathBuf, meta: config::PrefixMetadata) -> Self {
        Self {
            id: meta.install_name,
            version: meta.version,
            prefix,
            delegate_executable: meta
                .delegate_executable
                .unwrap_or_else(|| "conda".to_string()),
            channels: meta.channels,
            lock_sha256: meta.lock_sha256,
            requested_specs: meta.packages,
        }
    }

    /// Build a command description for an executable inside this runtime.
    pub fn command(&self, command_name: &str) -> miette::Result<RuntimeCommand> {
        validate_command_name(command_name)?;
        let executable = self.executable_path(command_name);
        if !executable.exists() {
            return Err(miette::miette!(
                "{command_name} executable not found at {}",
                policy::path_for_display(&executable)
            ));
        }
        Ok(RuntimeCommand {
            executable,
            env: self.activation_env(command_name),
            path_entries: self.path_entries(),
        })
    }

    /// Return the expected executable path inside this runtime.
    pub fn executable_path(&self, command_name: &str) -> PathBuf {
        exec::executable_in_prefix(&self.prefix, command_name)
    }

    /// Return activation-like environment variables for running a command.
    ///
    /// `PATH` is intentionally not included. Callers should prepend
    /// [`InstalledRuntime::path_entries`] to the existing process `PATH`.
    pub fn activation_env(&self, command_name: &str) -> BTreeMap<String, OsString> {
        let mut env = BTreeMap::new();
        env.insert("CONDA_ROOT_PREFIX".to_string(), self.prefix.clone().into());
        env.insert("CONDA_PREFIX".to_string(), self.prefix.clone().into());
        env.insert("CONDA_DEFAULT_ENV".to_string(), OsString::from("base"));
        env.insert("CONDA_SHLVL".to_string(), OsString::from("1"));
        env.insert(
            "CONDA_COMPLETION_COMMAND_NAME".to_string(),
            OsString::from(command_name),
        );
        env
    }

    /// Return prefix-local directories that should be prepended to `PATH`.
    pub fn path_entries(&self) -> Vec<PathBuf> {
        exec::prefix_path_entries(&self.prefix)
    }

    /// Build a data-only plan for exposing a command through a shim.
    ///
    /// Fleet does not write the shim. Callers should review the destination and
    /// wrapper contents, refuse overwrites by default, and add their own
    /// ownership metadata when writing files.
    pub fn shim_plan(&self, command_name: &str, options: ShimOptions) -> miette::Result<ShimPlan> {
        validate_command_name(command_name)?;
        validate_shim_name(&options.shim_name)?;

        let command = self.command(command_name)?;
        let env = os_env_to_strings(&command.env);
        let destination = options
            .shim_dir
            .as_ref()
            .map(|dir| dir.join(&options.shim_name))
            .unwrap_or_else(|| PathBuf::from(&options.shim_name));
        let wrapper_contents =
            render_wrapper_script(self, &options.shim_name, command_name, &command)?;

        Ok(ShimPlan {
            shim_name: options.shim_name,
            target_command: command_name.to_string(),
            destination,
            target_executable: command.executable,
            env,
            path_entries: command.path_entries,
            wrapper_contents,
        })
    }
}

/// Command data needed to run an executable inside an installed runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeCommand {
    /// Absolute executable path inside the runtime prefix.
    pub executable: PathBuf,
    /// Environment variables to set on the child process.
    pub env: BTreeMap<String, OsString>,
    /// Prefix-local directories that should be prepended to `PATH`.
    pub path_entries: Vec<PathBuf>,
}

/// Options for building a [`ShimPlan`].
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShimOptions {
    /// Name of the shim file the caller intends to expose.
    pub shim_name: String,
    /// Optional directory where the caller plans to write the shim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shim_dir: Option<PathBuf>,
}

/// Data-only plan for exposing a runtime command.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ShimPlan {
    /// Name of the shim file.
    pub shim_name: String,
    /// Runtime command the shim targets.
    pub target_command: String,
    /// Planned shim path. This may be relative when no shim directory was
    /// provided.
    pub destination: PathBuf,
    /// Absolute executable path inside the runtime prefix.
    pub target_executable: PathBuf,
    /// Environment variables represented as UTF-8 strings for JSON output.
    pub env: BTreeMap<String, String>,
    /// Prefix-local directories that should be prepended to `PATH`.
    pub path_entries: Vec<PathBuf>,
    /// Suggested wrapper script contents.
    pub wrapper_contents: String,
}

fn metadata_file_for(id: &str) -> String {
    format!(".{id}.json")
}

fn lock_sha256(lock_content: &str) -> String {
    let (digest, _) = hash::sha256_reader(lock_content.as_bytes())
        .expect("hashing an in-memory lockfile cannot fail");
    hash::hex(&digest)
}

fn channels_from_lock_content(lock_content: &str) -> miette::Result<Vec<String>> {
    let lock_file = LockFile::from_str_with_base_directory(lock_content, None)
        .into_diagnostic()
        .context("failed to parse lockfile")?;
    let env = lock_file
        .default_environment()
        .ok_or_else(|| miette::miette!("lockfile has no default environment"))?;
    Ok(env
        .channels()
        .iter()
        .map(|channel| channel.url.clone())
        .collect())
}

fn validate_bundle_options(bundle: Option<&Path>) -> miette::Result<()> {
    if let Some(path) = bundle
        && !path.is_dir()
    {
        return Err(miette::miette!(
            "bundle path is not a directory: {}",
            policy::path_for_display(path)
        ));
    }
    Ok(())
}

fn validate_runtime_id(id: &str) -> miette::Result<()> {
    validate_safe_name(id, "runtime id")
}

fn validate_command_name(name: &str) -> miette::Result<()> {
    validate_safe_name(name, "command name")
}

fn validate_shim_name(name: &str) -> miette::Result<()> {
    validate_safe_name(name, "shim name")
}

fn validate_safe_name(value: &str, kind: &str) -> miette::Result<()> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(miette::miette!("{kind} must not be empty"));
    };
    if !first.is_ascii_alphanumeric() {
        return Err(miette::miette!(
            "{kind} must start with an ASCII letter or digit: {value:?}"
        ));
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')) {
        return Err(miette::miette!(
            "{kind} may only contain ASCII letters, digits, '.', '-', and '_': {value:?}"
        ));
    }
    if value == "." || value == ".." {
        return Err(miette::miette!("{kind} must not be {value:?}"));
    }
    Ok(())
}

fn is_bootstrapped(prefix: &Path) -> bool {
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

fn reject_dangerous_prefix(prefix: &Path) -> miette::Result<()> {
    let home = dirs::home_dir();
    if std::fs::symlink_metadata(prefix)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(miette::miette!(
            "refusing to remove symbolic-link install path: {}",
            policy::path_for_display(prefix)
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
            policy::path_for_display(prefix)
        ));
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
        .with_context(|| format!("failed to inspect {}", policy::path_for_display(path)))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }

    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)
            .into_diagnostic()
            .with_context(|| format!("failed to read {}", policy::path_for_display(path)))?
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
            .with_context(|| {
                format!(
                    "failed to clear read-only bit on {}",
                    policy::path_for_display(path)
                )
            })?;
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

fn fleet_frozen_message(id: &str) -> String {
    format!(
        "This base environment is managed by conda-fleet runtime {id}.\n\
Create a new environment instead: conda create -n myenv\n\
To reinstall: use the fleet caller that installed this runtime\n\
To override: pass --override-frozen-env"
    )
}

fn os_env_to_strings(env: &BTreeMap<String, OsString>) -> BTreeMap<String, String> {
    env.iter()
        .map(|(key, value)| (key.clone(), value.to_string_lossy().into_owned()))
        .collect()
}

fn render_wrapper_script(
    runtime: &InstalledRuntime,
    shim_name: &str,
    command_name: &str,
    command: &RuntimeCommand,
) -> miette::Result<String> {
    if cfg!(windows) {
        Ok(render_windows_wrapper(
            runtime,
            shim_name,
            command_name,
            command,
        ))
    } else {
        render_posix_wrapper(runtime, shim_name, command_name, command)
    }
}

fn render_posix_wrapper(
    runtime: &InstalledRuntime,
    shim_name: &str,
    command_name: &str,
    command: &RuntimeCommand,
) -> miette::Result<String> {
    let path = std::env::join_paths(&command.path_entries)
        .map_err(|err| miette::miette!("failed to construct shim PATH: {err}"))?
        .to_string_lossy()
        .into_owned();
    let mut script = String::new();
    script.push_str("#!/bin/sh\n");
    script.push_str("# conda-fleet-shim: v1\n");
    script.push_str(&format!("# conda-fleet-runtime: {}\n", runtime.id));
    script.push_str(&format!("# conda-fleet-shim-name: {shim_name}\n"));
    script.push_str(&format!("# conda-fleet-command: {command_name}\n"));
    for (key, value) in os_env_to_strings(&command.env) {
        script.push_str("export ");
        script.push_str(&key);
        script.push('=');
        script.push_str(&shell_quote(&value));
        script.push('\n');
    }
    script.push_str("export PATH=");
    script.push_str(&shell_quote(&path));
    script.push_str(":${PATH:-}\n");
    script.push_str("exec ");
    let executable = command.executable.to_string_lossy();
    script.push_str(&shell_quote(executable.as_ref()));
    script.push_str(" \"$@\"\n");
    Ok(script)
}

fn render_windows_wrapper(
    runtime: &InstalledRuntime,
    shim_name: &str,
    command_name: &str,
    command: &RuntimeCommand,
) -> String {
    let path = command
        .path_entries
        .iter()
        .map(|path| path.to_string_lossy())
        .collect::<Vec<_>>()
        .join(";");
    let mut script = String::new();
    script.push_str("@echo off\r\n");
    script.push_str("rem conda-fleet-shim: v1\r\n");
    script.push_str(&format!("rem conda-fleet-runtime: {}\r\n", runtime.id));
    script.push_str(&format!("rem conda-fleet-shim-name: {shim_name}\r\n"));
    script.push_str(&format!("rem conda-fleet-command: {command_name}\r\n"));
    for (key, value) in os_env_to_strings(&command.env) {
        script.push_str(&format!("set \"{key}={value}\"\r\n"));
    }
    script.push_str(&format!("set \"PATH={path};%PATH%\"\r\n"));
    script.push('"');
    script.push_str(&command.executable.to_string_lossy());
    script.push_str("\" %*\r\n");
    script
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rattler_conda_types::Platform;
    use tempfile::TempDir;

    fn fleet(root: &Path) -> Fleet {
        Fleet::new(FleetConfig::new(root))
    }

    fn empty_lock() -> String {
        let platform = Platform::current();
        format!(
            r#"
---
version: 6
environments:
  default:
    channels:
      - url: https://conda.anaconda.org/conda-forge
    packages:
      {platform}: []
packages: []
"#
        )
    }

    fn spec(id: &str) -> RuntimeSpec {
        RuntimeSpec {
            id: id.to_string(),
            version: "1.0.0".to_string(),
            delegate_executable: "conda".to_string(),
            lock_content: empty_lock(),
            requested_specs: vec!["conda".to_string()],
        }
    }

    fn write_managed_runtime(prefix: &Path, id: &str, delegate: &str) {
        std::fs::create_dir_all(prefix.join("conda-meta")).unwrap();
        config::write_metadata_for_identity(
            prefix,
            config::PrefixMetadataIdentity {
                display_name: id,
                install_name: id,
                metadata_file: &metadata_file_for(id),
                version: "1.0.0",
                delegate_executable: Some(delegate),
                lock_sha256: Some("abc123"),
            },
            &["conda-forge".to_string()],
            &["conda".to_string()],
        )
        .unwrap();
    }

    #[test]
    fn test_runtime_spec_validation_rejects_unsafe_ids() {
        let mut runtime = spec("tool");
        runtime.id = "../tool".to_string();
        assert!(runtime.validate().is_err());

        runtime.id = "tool".to_string();
        runtime.delegate_executable = "bin/tool".to_string();
        assert!(runtime.validate().is_err());

        runtime.delegate_executable = "conda".to_string();
        runtime.lock_content.clear();
        assert!(runtime.validate().is_err());

        let runtime = spec("tool");
        assert_eq!(runtime.lock_sha256(), lock_sha256(&runtime.lock_content));
    }

    #[test]
    fn test_channels_reads_default_environment_from_lock() {
        assert_eq!(
            channels_from_lock_content(&empty_lock()).unwrap(),
            vec!["https://conda.anaconda.org/conda-forge".to_string()]
        );
    }

    #[test]
    fn test_command_and_shim_name_validation() {
        assert!(validate_command_name("conda").is_ok());
        assert!(validate_command_name("python3.12").is_ok());
        assert!(validate_command_name("bin/conda").is_err());
        assert!(validate_shim_name("runner-shim").is_ok());
        assert!(validate_shim_name(".runner-shim").is_err());
    }

    #[tokio::test]
    async fn test_install_list_status_remove_empty_locked_runtime() {
        let tmp = TempDir::new().unwrap();
        let install_root = tmp.path().join("fleet");
        let fleet = fleet(&install_root);

        let installed = fleet
            .install(
                spec("tool"),
                InstallOptions {
                    compile_python_bytecode: false,
                    ..InstallOptions::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(installed.prefix, install_root.join("tool"));
        assert!(installed.prefix.join(".condarc").is_file());
        assert!(installed.prefix.join("conda-meta").join("frozen").is_file());
        assert!(
            installed
                .prefix
                .join("conda-meta")
                .join("history")
                .is_file()
        );
        assert!(
            installed
                .prefix
                .join("conda-meta")
                .join("initial-state.explicit.txt")
                .is_file()
        );
        assert!(installed.prefix.join(".tool.json").is_file());

        let listed = fleet.list().unwrap();
        assert_eq!(listed, vec![installed.clone()]);
        assert_eq!(
            installed.channels,
            vec!["https://conda.anaconda.org/conda-forge".to_string()]
        );
        assert_eq!(installed.lock_sha256, Some(lock_sha256(&empty_lock())));

        let status = fleet.status("tool").unwrap().unwrap();
        assert_eq!(status, installed);

        fleet.remove("tool").unwrap();
        assert!(!install_root.join("tool").exists());
    }

    #[test]
    fn test_list_ignores_directories_without_valid_metadata() {
        let tmp = TempDir::new().unwrap();
        let install_root = tmp.path().join("fleet");
        std::fs::create_dir_all(install_root.join("ignored").join("conda-meta")).unwrap();
        write_managed_runtime(&install_root.join("tool"), "tool", "conda");

        let listed = fleet(&install_root).list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "tool");
    }

    #[test]
    fn test_install_and_remove_reject_unmanaged_non_empty_prefixes() {
        let tmp = TempDir::new().unwrap();
        let install_root = tmp.path().join("fleet");
        let unmanaged = install_root.join("tool");
        std::fs::create_dir_all(&unmanaged).unwrap();
        std::fs::write(unmanaged.join("file.txt"), "owned elsewhere").unwrap();

        let fleet = fleet(&install_root);
        let err = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(fleet.install(
                spec("tool"),
                InstallOptions {
                    compile_python_bytecode: false,
                    ..InstallOptions::default()
                },
            ))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unmanaged non-empty prefix"), "{err}");

        let err = fleet.remove("tool").unwrap_err().to_string();
        assert!(err.contains("unmanaged non-empty prefix"), "{err}");
    }

    #[test]
    fn test_runtime_command_helpers_and_shim_plan() {
        let tmp = TempDir::new().unwrap();
        let prefix = tmp.path().join("fleet").join("tool");
        write_managed_runtime(&prefix, "tool", "runner");

        let executable = exec::executable_in_prefix(&prefix, "runner");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(&executable, "stub").unwrap();

        let runtime = fleet(&tmp.path().join("fleet"))
            .status("tool")
            .unwrap()
            .unwrap();
        assert_eq!(runtime.executable_path("runner"), executable);
        assert_eq!(runtime.lock_sha256.as_deref(), Some("abc123"));

        let command = runtime.command("runner").unwrap();
        assert_eq!(command.executable, executable);
        assert!(command.path_entries.contains(&prefix.join("condabin")));
        assert_eq!(
            command.env.get("CONDA_DEFAULT_ENV"),
            Some(&OsString::from("base"))
        );

        let activation = runtime.activation_env("runner");
        assert_eq!(activation.get("CONDA_PREFIX"), Some(&prefix.clone().into()));

        let plan = runtime
            .shim_plan(
                "runner",
                ShimOptions {
                    shim_name: "runner-shim".to_string(),
                    shim_dir: Some(tmp.path().join("bin")),
                },
            )
            .unwrap();
        assert_eq!(plan.destination, tmp.path().join("bin").join("runner-shim"));
        assert_eq!(plan.target_executable, executable);
        assert!(plan.wrapper_contents.contains("conda-fleet-shim"));
    }
}
