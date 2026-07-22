//! Experimental APIs for managing multiple locked conda runtimes.
//!
//! Fleet is an optional Rust API for tools that want to install and
//! inspect several conda-ship-managed prefixes. It does not solve
//! environments, choose catalogs, create shims, or mutate a user's global
//! `PATH`.

use std::path::{Path, PathBuf};

use miette::{Context, IntoDiagnostic};
use rattler_lock::LockFile;

use crate::bootstrap_lock::BootstrapLock;
use crate::bootstrap_state::{self, BootstrapIdentity};
use crate::commands::{self, ManagedPrefixIdentity, PrefixDisposition};
use crate::config::PrefixMetadataIdentity;
use crate::constructor_metadata::InstallerMetadata;
use crate::{config, constructor_metadata, exec, hash, install, policy};

/// Manager for multiple conda-ship-managed runtime prefixes.
#[derive(Clone, Debug)]
pub struct Fleet {
    install_root: PathBuf,
}

impl Fleet {
    /// Create a new fleet manager.
    pub fn new(install_root: impl Into<PathBuf>) -> Self {
        Self {
            install_root: install_root.into(),
        }
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
        let metadata_file = metadata_file_for(&spec.id);
        let identity = managed_identity(&spec.id, &metadata_file);

        let prefix = self.prefix_for(&spec.id)?;
        std::fs::create_dir_all(&self.install_root)
            .into_diagnostic()
            .with_context(|| {
                format!(
                    "failed to create fleet install root {}",
                    policy::path_for_display(&self.install_root)
                )
            })?;

        let _lock = BootstrapLock::acquire(&prefix)?;
        let disposition = commands::prefix_disposition_for(&prefix, identity, !options.force)?;
        let reinstall = match disposition {
            PrefixDisposition::Ready if !options.force => {
                return self.read_installed_runtime(&prefix, &spec.id, "use");
            }
            PrefixDisposition::Ready => true,
            PrefixDisposition::Bootstrap { reinstall } => reinstall,
        };

        if reinstall {
            eprintln!("   Reinstalling every package from the selected lock");
        }

        validate_managed_policy_outputs(&prefix)?;
        bootstrap_state::write_installing_for(
            &prefix,
            BootstrapIdentity {
                display_name: &spec.id,
                install_name: &spec.id,
                metadata_file: &metadata_file,
            },
        )?;
        config::invalidate_metadata_for(&prefix, &metadata_file)?;
        reset_managed_policy_outputs(&prefix)?;

        if let Some(ref bundle_dir) = options.bundle_dir {
            install::from_lockfile_with_bundle_and_specs(
                &prefix,
                &spec.lock_content,
                &spec.requested_specs,
                bundle_dir,
                options.offline,
                reinstall,
            )
            .await?;
        } else if options.offline {
            install::from_lockfile_offline_with_specs(
                &prefix,
                &spec.lock_content,
                &spec.requested_specs,
                reinstall,
            )
            .await?;
        } else {
            install::from_lockfile_with_specs(
                &prefix,
                &spec.lock_content,
                &spec.requested_specs,
                reinstall,
            )
            .await?;
        }

        reset_managed_policy_outputs(&prefix)?;
        let installer = spec
            .installer
            .as_deref()
            .map(|installer_type| InstallerMetadata {
                name: &spec.id,
                version: &spec.version,
                installer_type,
            });
        constructor_metadata::write_prefix_metadata_for(
            &prefix,
            &spec.lock_content,
            &spec.requested_specs,
            &spec.delegate_executable,
            installer,
        )?;
        write_configured_policy(&prefix, &spec)?;
        install::compile_python_bytecode(&prefix);
        exec::validate_delegate(&prefix, &spec.delegate_executable)?;
        config::write_metadata_for_identity(
            &prefix,
            PrefixMetadataIdentity {
                display_name: &spec.id,
                install_name: &spec.id,
                metadata_file: &metadata_file,
                version: &spec.version,
                delegate_executable: Some(&spec.delegate_executable),
                lock_sha256: Some(&lock_sha256),
                update: None,
            },
            &channels,
            &spec.requested_specs,
        )?;
        bootstrap_state::remove(&prefix)?;

        self.read_installed_runtime(&prefix, &spec.id, "inspect")
    }

    /// List runtimes with valid fleet metadata under the install root.
    ///
    /// Directories without valid metadata are ignored. Fleet does not maintain
    /// a separate registry database.
    pub fn list(&self) -> miette::Result<Vec<InstalledRuntime>> {
        if !self.install_root.is_dir() {
            return Ok(Vec::new());
        }

        let mut runtimes = Vec::new();
        let entries = std::fs::read_dir(&self.install_root)
            .into_diagnostic()
            .with_context(|| {
                format!(
                    "failed to read fleet install root {}",
                    policy::path_for_display(&self.install_root)
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
            if let Ok(Some(runtime)) = self.get(&id) {
                runtimes.push(runtime);
            }
        }

        runtimes.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(runtimes)
    }

    /// Return a managed runtime by id.
    pub fn get(&self, id: &str) -> miette::Result<Option<InstalledRuntime>> {
        validate_runtime_id(id)?;
        let prefix = self.prefix_for(id)?;
        let metadata_file = metadata_file_for(id);
        let _lock = BootstrapLock::acquire(&prefix)?;
        commands::validate_prefix_path(&prefix)?;
        let metadata_path = config::metadata_path_for(&prefix, &metadata_file);
        match std::fs::symlink_metadata(&metadata_path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).into_diagnostic(),
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
        let metadata_file = metadata_file_for(id);
        match std::fs::symlink_metadata(&prefix) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error).into_diagnostic(),
        }

        let _lock = BootstrapLock::acquire(&prefix)?;
        reject_dangerous_prefix(&prefix)?;
        if let Some(state) = bootstrap_state::read(&prefix)? {
            bootstrap_state::validate_identity_for(
                &state,
                BootstrapIdentity {
                    display_name: id,
                    install_name: id,
                    metadata_file: &metadata_file,
                },
            )?;
        } else if !commands::is_empty_dir(&prefix)? {
            commands::read_managed_metadata_for(
                &prefix,
                managed_identity(id, &metadata_file),
                "remove",
            )?;
        }

        remove_install_path(&prefix)
    }

    fn prefix_for(&self, id: &str) -> miette::Result<PathBuf> {
        validate_runtime_id(id)?;
        Ok(self.install_root.join(id))
    }

    fn read_installed_runtime(
        &self,
        prefix: &Path,
        id: &str,
        action: &str,
    ) -> miette::Result<InstalledRuntime> {
        let metadata_file = metadata_file_for(id);
        let meta = commands::read_managed_metadata_for(
            prefix,
            managed_identity(id, &metadata_file),
            action,
        )?;

        let delegate = meta.delegate_executable.as_deref().ok_or_else(|| {
            miette::miette!(
                "refusing to {action} fleet prefix without an explicit delegate: {}",
                policy::path_for_display(prefix)
            )
        })?;
        exec::validate_delegate(prefix, delegate)?;

        InstalledRuntime::from_metadata(prefix.to_path_buf(), meta)
    }
}

/// Resolved runtime input accepted by [`Fleet::install`].
///
/// `RuntimeSpec` is not a user-facing catalog format. Callers construct it from
/// their catalog or conda-ship stamped runtime data.
#[derive(Clone, Debug, Eq, PartialEq)]
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
    pub requested_specs: Vec<String>,
    /// Exact `.condarc` text supplied by the caller, when configured.
    pub condarc: Option<String>,
    /// Whether to write the CEP 22 frozen marker for the base prefix.
    pub freeze_base: bool,
    /// Optional Constructor-compatible installer provenance type.
    pub installer: Option<String>,
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
        install::lockfile_records_for_current_platform(&self.lock_content)?;
        install::parse_specs(&self.requested_specs)?;
        if let Some(contents) = self.condarc.as_deref() {
            validate_condarc(contents)?;
        }
        if self
            .installer
            .as_deref()
            .is_some_and(|installer| installer.trim().is_empty())
        {
            return Err(miette::miette!("installer type must not be empty"));
        }
        Ok(())
    }

    /// Return the SHA256 digest of this runtime's lock content.
    pub fn lock_sha256(&self) -> String {
        lock_sha256(&self.lock_content)
    }
}

/// Options for installing a runtime.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InstallOptions {
    /// Reinstall every locked package in an existing managed runtime.
    ///
    /// Named environments and unrelated files under the prefix are preserved.
    pub force: bool,
    /// Install without network access. Packages must already be in the shared
    /// rattler cache or in `bundle_dir`.
    pub offline: bool,
    /// Optional directory containing `.conda` or `.tar.bz2` package archives.
    pub bundle_dir: Option<PathBuf>,
}

/// Runtime prefix discovered or installed by [`Fleet`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstalledRuntime {
    /// Runtime id and directory name under the fleet install root.
    pub id: String,
    /// Version recorded in conda-ship prefix metadata.
    pub version: String,
    /// Managed conda prefix path.
    pub prefix: PathBuf,
    /// Default executable that callers usually expose for this runtime.
    pub delegate_executable: String,
    /// Channels recorded as lockfile provenance.
    pub channels: Vec<String>,
    /// SHA256 digest of the lock content used for installation, when recorded.
    pub lock_sha256: Option<String>,
    /// Requested specs recorded at install time.
    pub requested_specs: Vec<String>,
}

impl InstalledRuntime {
    fn from_metadata(prefix: PathBuf, meta: config::PrefixMetadata) -> miette::Result<Self> {
        let delegate_executable = meta.delegate_executable.ok_or_else(|| {
            miette::miette!("fleet runtime metadata has no explicit delegate executable")
        })?;
        Ok(Self {
            id: meta.install_name,
            version: meta.version,
            prefix,
            delegate_executable,
            channels: meta.channels,
            lock_sha256: meta.lock_sha256,
            requested_specs: meta.packages,
        })
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
            path_entries: self.path_entries(),
        })
    }

    /// Return the expected executable path inside this runtime.
    pub fn executable_path(&self, command_name: &str) -> PathBuf {
        exec::executable_in_prefix(&self.prefix, command_name)
    }

    /// Return prefix-local directories that should be prepended to `PATH`.
    pub fn path_entries(&self) -> Vec<PathBuf> {
        exec::prefix_path_entries(&self.prefix)
    }

    /// Build a plan for exposing a command through a shim.
    ///
    /// Fleet does not write the shim. Callers should review the destination and
    /// command data, refuse overwrites by default, and add their own ownership
    /// metadata when writing files.
    pub fn shim_plan(
        &self,
        command_name: &str,
        shim_name: &str,
        shim_dir: Option<&Path>,
    ) -> miette::Result<ShimPlan> {
        validate_command_name(command_name)?;
        validate_shim_name(shim_name)?;

        let command = self.command(command_name)?;
        let destination = shim_dir
            .map(|dir| dir.join(shim_name))
            .unwrap_or_else(|| PathBuf::from(shim_name));

        Ok(ShimPlan {
            shim_name: shim_name.to_string(),
            target_command: command_name.to_string(),
            destination,
            command,
        })
    }
}

/// Command data needed to run an executable inside an installed runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeCommand {
    /// Absolute executable path inside the runtime prefix.
    pub executable: PathBuf,
    /// Prefix-local directories that should be prepended to `PATH`.
    pub path_entries: Vec<PathBuf>,
}

/// Data-only plan for exposing a runtime command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShimPlan {
    /// Name of the shim file.
    pub shim_name: String,
    /// Runtime command the shim targets.
    pub target_command: String,
    /// Planned shim path. This may be relative when no shim directory was
    /// provided.
    pub destination: PathBuf,
    /// Runtime command data the caller can use when writing a wrapper.
    pub command: RuntimeCommand,
}

fn metadata_file_for(id: &str) -> String {
    format!(".{id}.json")
}

fn managed_identity<'a>(id: &'a str, metadata_file: &'a str) -> ManagedPrefixIdentity<'a> {
    ManagedPrefixIdentity {
        display_name: id,
        install_name: id,
        metadata_file,
        expected_delegate: None,
    }
}

fn write_configured_policy(prefix: &Path, spec: &RuntimeSpec) -> miette::Result<()> {
    if let Some(contents) = spec.condarc.as_deref() {
        config::write_condarc(prefix, contents)?;
    }
    if spec.freeze_base {
        config::write_frozen_with_message(prefix, &fleet_frozen_message(&spec.id))?;
    }
    Ok(())
}

fn reset_managed_policy_outputs(prefix: &Path) -> miette::Result<()> {
    for path in managed_policy_output_paths(prefix) {
        remove_managed_policy_output(&path)?;
    }
    Ok(())
}

fn validate_managed_policy_outputs(prefix: &Path) -> miette::Result<()> {
    for path in managed_policy_output_paths(prefix) {
        validate_managed_policy_output(&path)?;
    }
    Ok(())
}

fn managed_policy_output_paths(prefix: &Path) -> [PathBuf; 3] {
    [
        prefix.join(".condarc"),
        prefix.join("conda-meta").join("frozen"),
        prefix.join(".installer.info"),
    ]
}

fn validate_managed_policy_output(path: &Path) -> miette::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(miette::miette!(
                "refusing to replace Fleet-managed output that is not a regular file: {}",
                policy::path_for_display(path)
            ))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).into_diagnostic(),
    }
}

fn remove_managed_policy_output(path: &Path) -> miette::Result<()> {
    validate_managed_policy_output(path)?;
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(miette::miette!(
                "refusing to replace Fleet-managed output that is not a regular file: {}",
                policy::path_for_display(path)
            ))
        }
        Ok(_) => std::fs::remove_file(path)
            .into_diagnostic()
            .with_context(|| {
                format!(
                    "failed to remove Fleet-managed output at {}",
                    policy::path_for_display(path)
                )
            }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).into_diagnostic(),
    }
}

fn validate_condarc(contents: &str) -> miette::Result<()> {
    let parsed: serde_yaml::Value = serde_yaml::from_str(contents)
        .into_diagnostic()
        .context("failed to parse RuntimeSpec condarc")?;
    if !parsed.is_mapping() {
        return Err(miette::miette!(
            "RuntimeSpec condarc must be a YAML mapping"
        ));
    }
    Ok(())
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

fn fleet_frozen_message(id: &str) -> String {
    format!(
        "This base environment is managed by Fleet runtime {id}.\n\
Create a new environment instead: conda create -n myenv\n\
To reinstall: use the fleet caller that installed this runtime\n\
To override: pass --override-frozen"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rattler_conda_types::Platform;
    use tempfile::TempDir;

    fn fleet(root: &Path) -> Fleet {
        Fleet::new(root)
    }

    fn empty_lock_for(platform: Platform) -> String {
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

    fn empty_lock() -> String {
        empty_lock_for(Platform::current())
    }

    fn spec(id: &str) -> RuntimeSpec {
        RuntimeSpec {
            id: id.to_string(),
            version: "1.0.0".to_string(),
            delegate_executable: "runner".to_string(),
            lock_content: empty_lock(),
            requested_specs: vec!["runner".to_string()],
            condarc: None,
            freeze_base: false,
            installer: None,
        }
    }

    fn write_delegate(prefix: &Path, delegate: &str) -> PathBuf {
        let executable = exec::executable_in_prefix(prefix, delegate);
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(&executable, "stub").unwrap();
        executable
    }

    fn mark_installing(prefix: &Path, id: &str) {
        let metadata_file = metadata_file_for(id);
        bootstrap_state::write_installing_for(
            prefix,
            BootstrapIdentity {
                display_name: id,
                install_name: id,
                metadata_file: &metadata_file,
            },
        )
        .unwrap();
    }

    fn write_managed_runtime(prefix: &Path, id: &str, delegate: &str) {
        std::fs::create_dir_all(prefix.join("conda-meta")).unwrap();
        write_delegate(prefix, delegate);
        let metadata_file = metadata_file_for(id);
        config::write_metadata_for_identity(
            prefix,
            config::PrefixMetadataIdentity {
                display_name: id,
                install_name: id,
                metadata_file: &metadata_file,
                version: "1.0.0",
                delegate_executable: Some(delegate),
                lock_sha256: Some("abc123"),
                update: None,
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

        runtime.delegate_executable = "runner".to_string();
        runtime.lock_content.clear();
        assert!(runtime.validate().is_err());

        runtime.lock_content = empty_lock();
        runtime.requested_specs = vec![">=>=not_a_package!!!".to_string()];
        assert!(runtime.validate().is_err());

        runtime.requested_specs = vec!["runner".to_string()];
        runtime.condarc = Some("- not-a-mapping\n".to_string());
        assert!(runtime.validate().is_err());

        runtime.condarc = None;
        runtime.installer = Some(" ".to_string());
        assert!(runtime.validate().is_err());

        let runtime = spec("tool");
        assert_eq!(runtime.lock_sha256(), lock_sha256(&runtime.lock_content));

        let other_platform = if Platform::current() == Platform::Linux64 {
            Platform::Win64
        } else {
            Platform::Linux64
        };
        let mut runtime = spec("tool");
        runtime.lock_content = empty_lock_for(other_platform);
        assert!(runtime.validate().is_err());
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
    async fn test_install_reinstall_get_and_remove_empty_locked_runtime() {
        let tmp = TempDir::new().unwrap();
        let install_root = tmp.path().join("fleet");
        let fleet = fleet(&install_root);
        let prefix = install_root.join("tool");
        write_delegate(&prefix, "runner");
        let sentinel = prefix.join("envs").join("named").join("keep.txt");
        std::fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
        std::fs::write(&sentinel, "keep").unwrap();
        mark_installing(&prefix, "tool");

        let installed = fleet
            .install(spec("tool"), InstallOptions::default())
            .await
            .unwrap();

        assert_eq!(installed.prefix, install_root.join("tool"));
        assert!(!installed.prefix.join(".condarc").exists());
        assert!(!installed.prefix.join("conda-meta").join("frozen").exists());
        assert!(!installed.prefix.join(".installer.info").exists());
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
        assert!(!bootstrap_state::path(&installed.prefix).exists());
        assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "keep");

        let mut configured = spec("tool");
        configured.condarc = Some("channels:\n  - conda-forge\n".to_string());
        configured.freeze_base = true;
        configured.installer = Some("homebrew".to_string());
        let configured = fleet
            .install(
                configured,
                InstallOptions {
                    force: true,
                    ..InstallOptions::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(configured.prefix.join(".condarc")).unwrap(),
            "channels:\n  - conda-forge\n"
        );
        assert!(
            configured
                .prefix
                .join("conda-meta")
                .join("frozen")
                .is_file()
        );
        let installer: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(configured.prefix.join(".installer.info")).unwrap(),
        )
        .unwrap();
        assert_eq!(installer["type"], "homebrew");
        assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "keep");

        let installed = fleet
            .install(
                spec("tool"),
                InstallOptions {
                    force: true,
                    ..InstallOptions::default()
                },
            )
            .await
            .unwrap();
        assert!(!installed.prefix.join(".condarc").exists());
        assert!(!installed.prefix.join("conda-meta").join("frozen").exists());
        assert!(!installed.prefix.join(".installer.info").exists());
        assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "keep");
        let history =
            std::fs::read_to_string(installed.prefix.join("conda-meta").join("history")).unwrap();
        assert!(history.contains("# cmd: runner [automatic bootstrap]"));
        assert!(!history.contains("fleet bootstrap"));
        let listed = fleet.list().unwrap();
        assert_eq!(listed, vec![installed.clone()]);
        assert_eq!(
            installed.channels,
            vec!["https://conda.anaconda.org/conda-forge".to_string()]
        );
        assert_eq!(installed.lock_sha256, Some(lock_sha256(&empty_lock())));

        let found = fleet.get("tool").unwrap().unwrap();
        assert_eq!(found, installed);

        fleet.remove("tool").unwrap();
        assert!(!install_root.join("tool").exists());
    }

    #[test]
    fn test_list_ignores_directories_without_valid_metadata() {
        let tmp = TempDir::new().unwrap();
        let install_root = tmp.path().join("fleet");
        std::fs::create_dir_all(install_root.join("ignored").join("conda-meta")).unwrap();
        write_managed_runtime(&install_root.join("tool"), "tool", "runner");

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
            .block_on(fleet.install(spec("tool"), InstallOptions::default()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("existing non-empty path"), "{err}");

        let err = fleet.remove("tool").unwrap_err().to_string();
        assert!(err.contains("unmanaged install path"), "{err}");
    }

    #[test]
    #[cfg(unix)]
    fn test_remove_refuses_metadata_symlink() {
        let tmp = TempDir::new().unwrap();
        let install_root = tmp.path().join("fleet");
        let prefix = install_root.join("tool");
        let external = tmp.path().join("external");
        write_managed_runtime(&external, "tool", "runner");
        std::fs::create_dir_all(&prefix).unwrap();
        write_delegate(&prefix, "runner");
        std::os::unix::fs::symlink(external.join(".tool.json"), prefix.join(".tool.json")).unwrap();

        let error = fleet(&install_root).remove("tool").unwrap_err().to_string();

        assert!(error.contains("not a regular file"), "{error}");
        assert!(prefix.exists());
        assert!(external.join(".tool.json").is_file());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn test_reinstall_refuses_policy_symlink_without_invalidating_ready_state() {
        let tmp = TempDir::new().unwrap();
        let install_root = tmp.path().join("fleet");
        let prefix = install_root.join("tool");
        let external = tmp.path().join("external-condarc");
        std::fs::write(&external, "owned elsewhere").unwrap();
        write_managed_runtime(&prefix, "tool", "runner");
        std::os::unix::fs::symlink(&external, prefix.join(".condarc")).unwrap();

        let error = fleet(&install_root)
            .install(
                spec("tool"),
                InstallOptions {
                    force: true,
                    ..InstallOptions::default()
                },
            )
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("Fleet-managed output"), "{error}");
        assert_eq!(
            std::fs::read_to_string(external).unwrap(),
            "owned elsewhere"
        );
        assert!(!bootstrap_state::path(&prefix).exists());
        assert!(prefix.join(".tool.json").is_file());
        assert!(!prefix.join("conda-meta").join("history").exists());
    }

    #[tokio::test]
    async fn test_invalid_specs_do_not_demote_ready_runtime() {
        let tmp = TempDir::new().unwrap();
        let install_root = tmp.path().join("fleet");
        let prefix = install_root.join("tool");
        write_managed_runtime(&prefix, "tool", "runner");
        std::fs::write(prefix.join(".condarc"), "channels: []\n").unwrap();
        let metadata_before = std::fs::read(prefix.join(".tool.json")).unwrap();

        let mut invalid = spec("tool");
        invalid.requested_specs = vec![">=>=not_a_package!!!".to_string()];
        let error = fleet(&install_root)
            .install(
                invalid,
                InstallOptions {
                    force: true,
                    ..InstallOptions::default()
                },
            )
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("failed to parse package specs"), "{error}");
        assert_eq!(
            std::fs::read(prefix.join(".tool.json")).unwrap(),
            metadata_before
        );
        assert_eq!(
            std::fs::read_to_string(prefix.join(".condarc")).unwrap(),
            "channels: []\n"
        );
        assert!(!bootstrap_state::path(&prefix).exists());
    }

    #[tokio::test]
    async fn test_force_restores_runtime_with_missing_recorded_delegate() {
        let tmp = TempDir::new().unwrap();
        let install_root = tmp.path().join("fleet");
        let prefix = install_root.join("tool");
        write_managed_runtime(&prefix, "tool", "missing-runner");
        std::fs::remove_file(exec::executable_in_prefix(&prefix, "missing-runner")).unwrap();
        write_delegate(&prefix, "replacement-runner");
        assert!(fleet(&install_root).get("tool").is_err());

        let mut replacement = spec("tool");
        replacement.delegate_executable = "replacement-runner".to_string();
        replacement.requested_specs = vec!["replacement-runner".to_string()];
        let installed = fleet(&install_root)
            .install(
                replacement,
                InstallOptions {
                    force: true,
                    ..InstallOptions::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(installed.delegate_executable, "replacement-runner");
        assert!(installed.executable_path("replacement-runner").is_file());
        assert!(!bootstrap_state::path(&prefix).exists());
        assert!(prefix.join(".tool.json").is_file());
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
            .get("tool")
            .unwrap()
            .unwrap();
        assert_eq!(runtime.executable_path("runner"), executable);
        assert_eq!(runtime.lock_sha256.as_deref(), Some("abc123"));

        let command = runtime.command("runner").unwrap();
        assert_eq!(command.executable, executable);
        assert!(command.path_entries.contains(&prefix.join("condabin")));

        let shim_dir = tmp.path().join("bin");
        let plan = runtime
            .shim_plan("runner", "runner-shim", Some(&shim_dir))
            .unwrap();
        assert_eq!(plan.destination, tmp.path().join("bin").join("runner-shim"));
        assert_eq!(plan.command.executable, executable);
        assert!(plan.command.path_entries.contains(&prefix.join("condabin")));
    }
}
