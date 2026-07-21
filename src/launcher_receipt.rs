//! Installer-owned receipts for stamped launcher executables.
//!
//! A receipt is an adjacent JSON sidecar written after an installer places a
//! launcher at its final path. It binds installer policy to that canonical path
//! and the launcher's SHA-256. The update planner validates the binding and
//! returns data that a downstream installer can use in its own verified
//! replacement flow.
//!
//! This module does not download artifacts, execute package-manager commands,
//! replace launchers, or modify managed prefixes.

use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Current launcher receipt schema version.
pub const LAUNCHER_RECEIPT_SCHEMA_VERSION: u32 = 1;

/// Suffix appended to the canonical launcher filename for its receipt.
pub const LAUNCHER_RECEIPT_SUFFIX: &str = ".conda-ship-receipt.json";

/// Maximum encoded size of a v1 launcher receipt.
pub const MAX_LAUNCHER_RECEIPT_BYTES: u64 = 64 * 1024;

/// Distribution identity recorded by the installer.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct DistributionIdentity {
    /// Downstream distribution name.
    pub name: String,
    /// Downstream distribution version.
    pub version: String,
}

/// Exact launcher identity recorded in a receipt.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct LauncherIdentity {
    /// Canonical absolute path of the installed launcher.
    pub path: String,
    /// Lowercase hexadecimal SHA-256 of the installed launcher.
    pub sha256: String,
}

/// Installer ownership policy for a launcher.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LauncherOwnership {
    /// The named installer may run its own verified replacement flow.
    Direct {
        /// Stable downstream installer identity.
        installer: String,
        /// HTTPS source used by the downstream updater to discover releases.
        release_source: String,
    },
    /// Another package manager owns launcher replacement.
    External {
        /// Stable external package-manager or installer identity.
        installer: String,
        /// Exact downstream command that may be displayed to the user.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        update_command: Option<String>,
    },
}

/// Versioned JSON receipt stored next to an installed launcher.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct LauncherReceipt {
    /// Receipt schema version.
    pub schema_version: u32,
    /// Downstream distribution identity.
    pub distribution: DistributionIdentity,
    /// Canonical path and digest of the exact installed launcher.
    pub launcher: LauncherIdentity,
    /// Installer policy that controls replacement.
    pub ownership: LauncherOwnership,
}

/// A validated direct-launcher update plan.
///
/// The downstream updater must still download and verify the replacement
/// artifact before invoking its installer-owned replacement flow. The expected
/// digest lets that flow recheck that the installed launcher has not changed
/// since the plan was created.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LauncherUpdatePlan {
    /// Canonical launcher path that the installer may replace.
    pub launcher_path: PathBuf,
    /// Adjacent receipt path that the installer must replace after the launcher.
    pub receipt_path: PathBuf,
    /// Digest that matched the launcher when the plan was created.
    pub expected_launcher_sha256: String,
    /// Distribution identity from the validated receipt.
    pub distribution: DistributionIdentity,
    /// Installer identity from the validated receipt.
    pub installer: String,
    /// HTTPS release source from the validated receipt.
    pub release_source: String,
}

/// Safe reasons why direct launcher replacement was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LauncherUpdateRefusalReason {
    /// The requested launcher could not be resolved to a regular file.
    LauncherUnavailable,
    /// No adjacent receipt exists.
    MissingReceipt,
    /// The adjacent receipt could not be read.
    UnreadableReceipt,
    /// The receipt is malformed, unsupported, or violates the v1 schema.
    InvalidReceipt,
    /// The receipt names a different canonical launcher path.
    LauncherPathMismatch,
    /// The launcher no longer matches the recorded digest.
    LauncherDigestMismatch,
    /// An external package manager owns launcher replacement.
    ExternallyManaged,
    /// A direct receipt belongs to another installer identity.
    InstallerMismatch,
    /// The validated launcher update plan changed after it was created.
    UpdatePlanChanged,
}

/// Structured refusal returned instead of an unsafe update plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LauncherUpdateRefusal {
    /// Machine-readable refusal reason.
    pub reason: LauncherUpdateRefusalReason,
    /// Human-readable explanation suitable for downstream error output.
    pub message: String,
    /// Validated receipt installer identity, when applicable.
    pub installer: Option<String>,
    /// Validated external update command for display only, when available.
    pub update_command: Option<String>,
}

/// Result of validating launcher ownership for an update.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LauncherUpdateDecision {
    /// The matching direct receipt allows downstream replacement planning.
    Allowed(LauncherUpdatePlan),
    /// Direct replacement is not allowed.
    Refused(LauncherUpdateRefusal),
}

/// Error produced while creating an installer-owned receipt.
#[derive(Debug)]
pub enum LauncherReceiptError {
    /// Installer input does not satisfy the v1 receipt schema.
    InvalidInput(String),
    /// A filesystem operation failed.
    Io {
        /// Operation that failed.
        operation: &'static str,
        /// Path involved in the failed operation.
        path: PathBuf,
        /// Underlying filesystem error.
        source: io::Error,
    },
    /// JSON serialization failed.
    Json(serde_json::Error),
}

impl fmt::Display for LauncherReceiptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(message) => formatter.write_str(message),
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} {}: {source}",
                path.display()
            ),
            Self::Json(source) => write!(formatter, "failed to render launcher receipt: {source}"),
        }
    }
}

impl Error for LauncherReceiptError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json(source) => Some(source),
            Self::InvalidInput(_) => None,
        }
    }
}

/// Return the adjacent receipt path for an existing launcher.
///
/// The launcher is canonicalized before the receipt filename is derived. A
/// launcher named `demo` uses `demo.conda-ship-receipt.json`, while
/// `demo.exe` uses `demo.exe.conda-ship-receipt.json`.
pub fn receipt_path_for_launcher(launcher: &Path) -> Result<PathBuf, LauncherReceiptError> {
    let canonical = canonical_launcher(launcher)?;
    receipt_path_from_canonical(&canonical)
}

/// Atomically write a v1 receipt for an existing installed launcher.
///
/// The function validates installer policy, canonicalizes and hashes the
/// launcher, writes a temporary file in the launcher's directory, syncs it,
/// and atomically replaces the adjacent receipt. It never changes the launcher.
pub fn write_launcher_receipt(
    launcher: &Path,
    distribution: DistributionIdentity,
    ownership: LauncherOwnership,
) -> Result<PathBuf, LauncherReceiptError> {
    validate_distribution(&distribution).map_err(LauncherReceiptError::InvalidInput)?;
    let ownership = normalize_ownership(ownership).map_err(LauncherReceiptError::InvalidInput)?;

    let canonical = canonical_launcher(launcher)?;
    let receipt_path = receipt_path_from_canonical(&canonical)?;
    let canonical_text = canonical.to_str().ok_or_else(|| {
        LauncherReceiptError::InvalidInput(
            "canonical launcher path must contain valid UTF-8".to_string(),
        )
    })?;
    let sha256 = sha256_file(&canonical).map_err(|source| LauncherReceiptError::Io {
        operation: "hash launcher at",
        path: canonical.clone(),
        source,
    })?;
    let receipt = LauncherReceipt {
        schema_version: LAUNCHER_RECEIPT_SCHEMA_VERSION,
        distribution,
        launcher: LauncherIdentity {
            path: canonical_text.to_string(),
            sha256,
        },
        ownership,
    };
    let mut rendered = serde_json::to_vec_pretty(&receipt).map_err(LauncherReceiptError::Json)?;
    rendered.push(b'\n');
    if rendered.len() as u64 > MAX_LAUNCHER_RECEIPT_BYTES {
        return Err(LauncherReceiptError::InvalidInput(format!(
            "rendered launcher receipt exceeds {MAX_LAUNCHER_RECEIPT_BYTES} bytes"
        )));
    }

    let parent = receipt_path.parent().ok_or_else(|| {
        LauncherReceiptError::InvalidInput(
            "canonical launcher path must have a parent directory".to_string(),
        )
    })?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".conda-ship-receipt-")
        .tempfile_in(parent)
        .map_err(|source| LauncherReceiptError::Io {
            operation: "create temporary launcher receipt in",
            path: parent.to_path_buf(),
            source,
        })?;

    #[cfg(unix)]
    temporary
        .as_file()
        .set_permissions(unix_receipt_permissions())
        .map_err(|source| LauncherReceiptError::Io {
            operation: "set permissions on temporary launcher receipt in",
            path: parent.to_path_buf(),
            source,
        })?;

    temporary
        .write_all(&rendered)
        .map_err(|source| LauncherReceiptError::Io {
            operation: "write temporary launcher receipt in",
            path: parent.to_path_buf(),
            source,
        })?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|source| LauncherReceiptError::Io {
            operation: "sync temporary launcher receipt in",
            path: parent.to_path_buf(),
            source,
        })?;
    temporary
        .persist(&receipt_path)
        .map_err(|error| LauncherReceiptError::Io {
            operation: "replace launcher receipt at",
            path: receipt_path.clone(),
            source: error.error,
        })?;
    sync_parent_directory(parent)?;

    Ok(receipt_path)
}

/// Validate an installed launcher for the expected installer identity.
///
/// Only a matching `direct` receipt yields [`LauncherUpdateDecision::Allowed`].
/// Every error and every external owner fails closed with a structured refusal.
pub fn plan_launcher_update(launcher: &Path, expected_installer: &str) -> LauncherUpdateDecision {
    if let Err(error) = validate_text("expected installer identity", expected_installer) {
        return refused(
            LauncherUpdateRefusalReason::InstallerMismatch,
            error,
            None,
            None,
        );
    }
    let canonical = match canonical_launcher(launcher) {
        Ok(path) => path,
        Err(error) => {
            return refused(
                LauncherUpdateRefusalReason::LauncherUnavailable,
                error.to_string(),
                None,
                None,
            );
        }
    };
    let receipt_path = match receipt_path_from_canonical(&canonical) {
        Ok(path) => path,
        Err(error) => {
            return refused(
                LauncherUpdateRefusalReason::LauncherUnavailable,
                error.to_string(),
                None,
                None,
            );
        }
    };

    let receipt_metadata = match std::fs::symlink_metadata(&receipt_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return refused(
                LauncherUpdateRefusalReason::InvalidReceipt,
                format!(
                    "launcher receipt is not a regular file: {}",
                    receipt_path.display()
                ),
                None,
                None,
            );
        }
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return refused(
                LauncherUpdateRefusalReason::MissingReceipt,
                format!(
                    "no installer-owned launcher receipt exists at {}",
                    receipt_path.display()
                ),
                None,
                None,
            );
        }
        Err(error) => {
            return refused(
                LauncherUpdateRefusalReason::UnreadableReceipt,
                format!(
                    "failed to read launcher receipt at {}: {error}",
                    receipt_path.display()
                ),
                None,
                None,
            );
        }
    };
    let file = match File::open(&receipt_path) {
        Ok(file) => file,
        Err(error) => {
            return refused(
                LauncherUpdateRefusalReason::UnreadableReceipt,
                format!(
                    "failed to read launcher receipt at {}: {error}",
                    receipt_path.display()
                ),
                None,
                None,
            );
        }
    };
    let opened_metadata = match file.metadata() {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => {
            return refused(
                LauncherUpdateRefusalReason::InvalidReceipt,
                format!(
                    "opened launcher receipt is not a regular file: {}",
                    receipt_path.display()
                ),
                None,
                None,
            );
        }
        Err(error) => {
            return refused(
                LauncherUpdateRefusalReason::UnreadableReceipt,
                format!(
                    "failed to inspect opened launcher receipt at {}: {error}",
                    receipt_path.display()
                ),
                None,
                None,
            );
        }
    };
    if !same_opened_receipt(&receipt_metadata, &opened_metadata) {
        return refused(
            LauncherUpdateRefusalReason::InvalidReceipt,
            format!(
                "launcher receipt changed while it was opened: {}",
                receipt_path.display()
            ),
            None,
            None,
        );
    }
    let mut bytes = Vec::new();
    if let Err(error) = file
        .take(MAX_LAUNCHER_RECEIPT_BYTES + 1)
        .read_to_end(&mut bytes)
    {
        return refused(
            LauncherUpdateRefusalReason::UnreadableReceipt,
            format!(
                "failed to read launcher receipt at {}: {error}",
                receipt_path.display()
            ),
            None,
            None,
        );
    }
    if bytes.len() as u64 > MAX_LAUNCHER_RECEIPT_BYTES {
        return refused(
            LauncherUpdateRefusalReason::InvalidReceipt,
            format!(
                "launcher receipt at {} exceeds {MAX_LAUNCHER_RECEIPT_BYTES} bytes",
                receipt_path.display()
            ),
            None,
            None,
        );
    }
    let receipt: LauncherReceipt = match serde_json::from_slice(&bytes) {
        Ok(receipt) => receipt,
        Err(error) => {
            return refused(
                LauncherUpdateRefusalReason::InvalidReceipt,
                format!(
                    "invalid launcher receipt at {}: {error}",
                    receipt_path.display()
                ),
                None,
                None,
            );
        }
    };
    if let Err(error) = validate_receipt(&receipt) {
        return refused(
            LauncherUpdateRefusalReason::InvalidReceipt,
            format!(
                "invalid launcher receipt at {}: {error}",
                receipt_path.display()
            ),
            None,
            None,
        );
    }

    let Some(canonical_text) = canonical.to_str() else {
        return refused(
            LauncherUpdateRefusalReason::LauncherUnavailable,
            "canonical launcher path must contain valid UTF-8".to_string(),
            None,
            None,
        );
    };
    if receipt.launcher.path != canonical_text {
        return refused(
            LauncherUpdateRefusalReason::LauncherPathMismatch,
            format!(
                "launcher receipt names {}, not {}",
                receipt.launcher.path,
                canonical.display()
            ),
            None,
            None,
        );
    }
    let actual_sha256 = match sha256_file(&canonical) {
        Ok(sha256) => sha256,
        Err(error) => {
            return refused(
                LauncherUpdateRefusalReason::LauncherUnavailable,
                format!(
                    "failed to hash launcher at {}: {error}",
                    canonical.display()
                ),
                None,
                None,
            );
        }
    };
    if receipt.launcher.sha256 != actual_sha256 {
        return refused(
            LauncherUpdateRefusalReason::LauncherDigestMismatch,
            format!(
                "launcher at {} no longer matches its installer-owned receipt",
                canonical.display()
            ),
            None,
            None,
        );
    }

    match receipt.ownership {
        LauncherOwnership::Direct {
            installer,
            release_source,
        } => {
            if installer != expected_installer {
                return refused(
                    LauncherUpdateRefusalReason::InstallerMismatch,
                    format!(
                        "launcher receipt belongs to installer {installer}, not {expected_installer}"
                    ),
                    Some(installer),
                    None,
                );
            }
            LauncherUpdateDecision::Allowed(LauncherUpdatePlan {
                launcher_path: canonical,
                receipt_path,
                expected_launcher_sha256: actual_sha256,
                distribution: receipt.distribution,
                installer,
                release_source,
            })
        }
        LauncherOwnership::External {
            installer,
            update_command,
        } => refused(
            LauncherUpdateRefusalReason::ExternallyManaged,
            format!("launcher replacement is managed by {installer}"),
            Some(installer),
            update_command,
        ),
    }
}

/// Re-read a receipt and re-hash its launcher immediately before replacement.
///
/// A downstream updater should call this after it has downloaded and verified
/// a candidate artifact, then compare the returned decision before entering its
/// installer-owned replacement step. This narrows the planning race but does
/// not provide a cross-process filesystem lock.
pub fn revalidate_launcher_update(plan: &LauncherUpdatePlan) -> LauncherUpdateDecision {
    match plan_launcher_update(&plan.launcher_path, &plan.installer) {
        LauncherUpdateDecision::Allowed(current) if current == *plan => {
            LauncherUpdateDecision::Allowed(current)
        }
        LauncherUpdateDecision::Allowed(_) => refused(
            LauncherUpdateRefusalReason::UpdatePlanChanged,
            "launcher update plan changed after update planning".to_string(),
            None,
            None,
        ),
        refusal @ LauncherUpdateDecision::Refused(_) => refusal,
    }
}

fn canonical_launcher(launcher: &Path) -> Result<PathBuf, LauncherReceiptError> {
    let input_metadata =
        std::fs::symlink_metadata(launcher).map_err(|source| LauncherReceiptError::Io {
            operation: "inspect launcher at",
            path: launcher.to_path_buf(),
            source,
        })?;
    if input_metadata.file_type().is_symlink() || !input_metadata.is_file() {
        return Err(LauncherReceiptError::InvalidInput(format!(
            "launcher is not an exact regular-file installation path: {}",
            launcher.display()
        )));
    }
    let canonical = std::fs::canonicalize(launcher).map_err(|source| LauncherReceiptError::Io {
        operation: "canonicalize launcher at",
        path: launcher.to_path_buf(),
        source,
    })?;
    let metadata = std::fs::metadata(&canonical).map_err(|source| LauncherReceiptError::Io {
        operation: "inspect launcher at",
        path: canonical.clone(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(LauncherReceiptError::InvalidInput(format!(
            "launcher is not a regular file: {}",
            canonical.display()
        )));
    }
    let canonical_text = canonical.to_str().ok_or_else(|| {
        LauncherReceiptError::InvalidInput(
            "v1 canonical launcher path must contain valid UTF-8".to_string(),
        )
    })?;
    validate_text("v1 canonical launcher path", canonical_text)
        .map_err(LauncherReceiptError::InvalidInput)?;
    Ok(canonical)
}

fn receipt_path_from_canonical(canonical: &Path) -> Result<PathBuf, LauncherReceiptError> {
    let filename = canonical.file_name().ok_or_else(|| {
        LauncherReceiptError::InvalidInput(format!(
            "canonical launcher path has no filename: {}",
            canonical.display()
        ))
    })?;
    let mut receipt_filename = filename.to_os_string();
    receipt_filename.push(LAUNCHER_RECEIPT_SUFFIX);
    Ok(canonical.with_file_name(receipt_filename))
}

fn validate_receipt(receipt: &LauncherReceipt) -> Result<(), String> {
    if receipt.schema_version != LAUNCHER_RECEIPT_SCHEMA_VERSION {
        return Err(format!(
            "unsupported schema version {}",
            receipt.schema_version
        ));
    }
    validate_distribution(&receipt.distribution)?;
    validate_text("launcher path", &receipt.launcher.path)?;
    if !is_lowercase_sha256(&receipt.launcher.sha256) {
        return Err("launcher SHA-256 must be 64 lowercase hexadecimal characters".to_string());
    }
    let normalized = normalize_ownership(receipt.ownership.clone())?;
    if normalized != receipt.ownership {
        return Err("release source must use canonical URL form".to_string());
    }
    Ok(())
}

fn validate_distribution(distribution: &DistributionIdentity) -> Result<(), String> {
    validate_text("distribution name", &distribution.name)?;
    validate_text("distribution version", &distribution.version)
}

fn normalize_ownership(ownership: LauncherOwnership) -> Result<LauncherOwnership, String> {
    match ownership {
        LauncherOwnership::Direct {
            installer,
            release_source,
        } => {
            validate_text("installer identity", &installer)?;
            let release_source = normalize_https_release_source(&release_source)?;
            Ok(LauncherOwnership::Direct {
                installer,
                release_source,
            })
        }
        LauncherOwnership::External {
            installer,
            update_command,
        } => {
            validate_text("installer identity", &installer)?;
            if let Some(command) = update_command.as_deref() {
                validate_external_update_command(command)?;
            }
            Ok(LauncherOwnership::External {
                installer,
                update_command,
            })
        }
    }
}

fn validate_external_update_command(command: &str) -> Result<(), String> {
    validate_text("external update command", command)?;
    if !command
        .bytes()
        .all(|byte| byte == b' ' || byte.is_ascii_graphic())
    {
        return Err("external update command must contain printable ASCII only".to_string());
    }
    Ok(())
}

fn validate_text(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if value.trim() != value {
        return Err(format!(
            "{label} must not have leading or trailing whitespace"
        ));
    }
    if value.chars().any(is_forbidden_display_character) {
        return Err(format!(
            "{label} must not contain control or formatting characters"
        ));
    }
    Ok(())
}

fn normalize_https_release_source(source: &str) -> Result<String, String> {
    validate_text("release source", source)?;
    let url = reqwest::Url::parse(source)
        .map_err(|error| format!("release source must be an absolute HTTPS URL: {error}"))?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return Err("release source must be an absolute HTTPS URL".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("release source must not contain credentials".to_string());
    }
    if url.fragment().is_some() {
        return Err("release source must not contain a URL fragment".to_string());
    }
    Ok(url.to_string())
}

fn is_forbidden_display_character(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{206f}'
        )
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn refused(
    reason: LauncherUpdateRefusalReason,
    message: String,
    installer: Option<String>,
    update_command: Option<String>,
) -> LauncherUpdateDecision {
    LauncherUpdateDecision::Refused(LauncherUpdateRefusal {
        reason,
        message,
        installer,
        update_command,
    })
}

#[cfg(unix)]
fn same_opened_receipt(before: &std::fs::Metadata, opened: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    before.dev() == opened.dev() && before.ino() == opened.ino()
}

#[cfg(not(unix))]
fn same_opened_receipt(_before: &std::fs::Metadata, _opened: &std::fs::Metadata) -> bool {
    true
}

#[cfg(unix)]
fn unix_receipt_permissions() -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;

    std::fs::Permissions::from_mode(0o644)
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> Result<(), LauncherReceiptError> {
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| LauncherReceiptError::Io {
            operation: "sync launcher directory at",
            path: parent.to_path_buf(),
            source,
        })
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> Result<(), LauncherReceiptError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn direct_ownership() -> LauncherOwnership {
        LauncherOwnership::Direct {
            installer: "demo-installer".to_string(),
            release_source: "https://example.com/demo/releases.json".to_string(),
        }
    }

    fn distribution() -> DistributionIdentity {
        DistributionIdentity {
            name: "Demo Distribution".to_string(),
            version: "1.2.3".to_string(),
        }
    }

    fn launcher(temp: &TempDir) -> PathBuf {
        let path = temp
            .path()
            .join(if cfg!(windows) { "demo.exe" } else { "demo" });
        std::fs::write(&path, b"launcher-v1").unwrap();
        path
    }

    fn refusal(decision: LauncherUpdateDecision) -> LauncherUpdateRefusal {
        match decision {
            LauncherUpdateDecision::Refused(refusal) => refusal,
            LauncherUpdateDecision::Allowed(plan) => {
                panic!(
                    "expected refusal, got plan for {}",
                    plan.launcher_path.display()
                )
            }
        }
    }

    #[test]
    fn receipt_path_appends_suffix_to_full_launcher_filename() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);

        let path = receipt_path_for_launcher(&launcher).unwrap();

        let expected = if cfg!(windows) {
            "demo.exe.conda-ship-receipt.json"
        } else {
            "demo.conda-ship-receipt.json"
        };
        assert_eq!(path.file_name().unwrap(), expected);
    }

    #[test]
    fn matching_direct_receipt_allows_an_update_plan() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);
        let receipt_path =
            write_launcher_receipt(&launcher, distribution(), direct_ownership()).unwrap();

        let LauncherUpdateDecision::Allowed(plan) =
            plan_launcher_update(&launcher, "demo-installer")
        else {
            panic!("matching direct receipt should allow a plan");
        };

        assert_eq!(
            plan.launcher_path,
            std::fs::canonicalize(&launcher).unwrap()
        );
        assert_eq!(plan.receipt_path, receipt_path);
        assert_eq!(plan.distribution, distribution());
        assert_eq!(plan.installer, "demo-installer");
        assert_eq!(
            plan.release_source,
            "https://example.com/demo/releases.json"
        );
        assert_eq!(plan.expected_launcher_sha256.len(), 64);
    }

    #[test]
    fn direct_receipt_refuses_a_different_installer_identity() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);
        write_launcher_receipt(&launcher, distribution(), direct_ownership()).unwrap();

        let refusal = refusal(plan_launcher_update(&launcher, "another-installer"));

        assert_eq!(
            refusal.reason,
            LauncherUpdateRefusalReason::InstallerMismatch
        );
        assert_eq!(refusal.installer.as_deref(), Some("demo-installer"));
    }

    #[test]
    fn external_receipt_refuses_with_display_only_command() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);
        write_launcher_receipt(
            &launcher,
            distribution(),
            LauncherOwnership::External {
                installer: "homebrew".to_string(),
                update_command: Some("brew update && brew upgrade demo".to_string()),
            },
        )
        .unwrap();

        let refusal = refusal(plan_launcher_update(&launcher, "demo-installer"));

        assert_eq!(
            refusal.reason,
            LauncherUpdateRefusalReason::ExternallyManaged
        );
        assert_eq!(refusal.installer.as_deref(), Some("homebrew"));
        assert_eq!(
            refusal.update_command.as_deref(),
            Some("brew update && brew upgrade demo")
        );
    }

    #[test]
    fn missing_receipt_refuses() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);

        let refusal = refusal(plan_launcher_update(&launcher, "demo-installer"));

        assert_eq!(refusal.reason, LauncherUpdateRefusalReason::MissingReceipt);
        assert!(refusal.update_command.is_none());
    }

    #[test]
    fn malformed_receipt_refuses() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);
        let receipt_path = receipt_path_for_launcher(&launcher).unwrap();
        std::fs::write(receipt_path, b"not json").unwrap();

        let refusal = refusal(plan_launcher_update(&launcher, "demo-installer"));

        assert_eq!(refusal.reason, LauncherUpdateRefusalReason::InvalidReceipt);
    }

    #[test]
    fn mismatched_launcher_path_refuses() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);
        let receipt_path =
            write_launcher_receipt(&launcher, distribution(), direct_ownership()).unwrap();
        let mut receipt: LauncherReceipt =
            serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
        receipt.launcher.path = temp.path().join("another-launcher").display().to_string();
        std::fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();

        let refusal = refusal(plan_launcher_update(&launcher, "demo-installer"));

        assert_eq!(
            refusal.reason,
            LauncherUpdateRefusalReason::LauncherPathMismatch
        );
    }

    #[test]
    fn modified_launcher_refuses() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);
        write_launcher_receipt(&launcher, distribution(), direct_ownership()).unwrap();
        std::fs::write(&launcher, b"launcher-v2").unwrap();

        let refusal = refusal(plan_launcher_update(&launcher, "demo-installer"));

        assert_eq!(
            refusal.reason,
            LauncherUpdateRefusalReason::LauncherDigestMismatch
        );
    }

    #[test]
    fn receipt_replacement_records_the_new_launcher_digest() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);
        write_launcher_receipt(&launcher, distribution(), direct_ownership()).unwrap();
        std::fs::write(&launcher, b"launcher-v2").unwrap();

        write_launcher_receipt(&launcher, distribution(), direct_ownership()).unwrap();

        assert!(matches!(
            plan_launcher_update(&launcher, "demo-installer"),
            LauncherUpdateDecision::Allowed(_)
        ));
        let temporary_files = std::fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".conda-ship-receipt-")
            })
            .count();
        assert_eq!(temporary_files, 0);
    }

    #[test]
    fn oversized_receipt_is_not_written() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);
        let oversized = DistributionIdentity {
            name: "x".repeat(MAX_LAUNCHER_RECEIPT_BYTES as usize),
            version: "1.2.3".to_string(),
        };

        let error = write_launcher_receipt(&launcher, oversized, direct_ownership()).unwrap_err();

        assert!(error.to_string().contains("exceeds 65536 bytes"));
        assert!(!receipt_path_for_launcher(&launcher).unwrap().exists());
    }

    #[test]
    fn launcher_path_with_display_formatting_is_refused_before_write() {
        let temp = TempDir::new().unwrap();
        let launcher = temp.path().join("demo\u{2028}launcher");
        std::fs::write(&launcher, b"launcher-v1").unwrap();

        let error =
            write_launcher_receipt(&launcher, distribution(), direct_ownership()).unwrap_err();
        let refusal = refusal(plan_launcher_update(&launcher, "demo-installer"));

        assert!(error.to_string().contains("control or formatting"));
        assert_eq!(
            refusal.reason,
            LauncherUpdateRefusalReason::LauncherUnavailable
        );
        let mut receipt_filename = launcher.file_name().unwrap().to_os_string();
        receipt_filename.push(LAUNCHER_RECEIPT_SUFFIX);
        assert!(!launcher.with_file_name(receipt_filename).exists());
    }

    #[test]
    fn direct_receipt_requires_an_https_release_source() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);

        let error = write_launcher_receipt(
            &launcher,
            distribution(),
            LauncherOwnership::Direct {
                installer: "demo-installer".to_string(),
                release_source: "http://example.com/releases.json".to_string(),
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("HTTPS"));
    }

    #[test]
    fn external_command_must_be_printable_ascii() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);

        for command in [
            "brew update\nbrew upgrade demo",
            "brew upgrade d\u{2028}emo",
            "brew upgrade d\u{202e}emo",
            "brew upgrade démo",
        ] {
            let error = write_launcher_receipt(
                &launcher,
                distribution(),
                LauncherOwnership::External {
                    installer: "homebrew".to_string(),
                    update_command: Some(command.to_string()),
                },
            )
            .unwrap_err();
            assert!(
                error.to_string().contains("control or formatting")
                    || error.to_string().contains("printable ASCII"),
                "{error}"
            );
        }
    }

    #[test]
    fn writer_canonicalizes_the_https_release_source() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);
        let source = "https://TRUSTED.example\\@evil.example/manifest";
        let expected = reqwest::Url::parse(source).unwrap().to_string();

        let receipt_path = write_launcher_receipt(
            &launcher,
            distribution(),
            LauncherOwnership::Direct {
                installer: "demo-installer".to_string(),
                release_source: source.to_string(),
            },
        )
        .unwrap();
        let receipt: LauncherReceipt =
            serde_json::from_slice(&std::fs::read(receipt_path).unwrap()).unwrap();
        let LauncherOwnership::Direct { release_source, .. } = receipt.ownership else {
            panic!("writer should preserve direct ownership");
        };

        assert_eq!(release_source, expected);
        assert!(!release_source.contains('\\'));
    }

    #[test]
    fn planner_refuses_a_noncanonical_release_source() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);
        let receipt_path =
            write_launcher_receipt(&launcher, distribution(), direct_ownership()).unwrap();
        let mut receipt: LauncherReceipt =
            serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
        let LauncherOwnership::Direct { release_source, .. } = &mut receipt.ownership else {
            panic!("test receipt should use direct ownership");
        };
        *release_source = "https://EXAMPLE.com/demo/releases.json".to_string();
        std::fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();

        let refusal = refusal(plan_launcher_update(&launcher, "demo-installer"));

        assert_eq!(refusal.reason, LauncherUpdateRefusalReason::InvalidReceipt);
        assert!(refusal.message.contains("canonical URL form"));
    }

    #[test]
    fn revalidation_refuses_changed_direct_ownership() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);
        write_launcher_receipt(&launcher, distribution(), direct_ownership()).unwrap();
        let LauncherUpdateDecision::Allowed(plan) =
            plan_launcher_update(&launcher, "demo-installer")
        else {
            panic!("initial receipt should allow a plan");
        };
        write_launcher_receipt(
            &launcher,
            distribution(),
            LauncherOwnership::Direct {
                installer: "demo-installer".to_string(),
                release_source: "https://example.com/demo/new-releases.json".to_string(),
            },
        )
        .unwrap();

        let refusal = refusal(revalidate_launcher_update(&plan));

        assert_eq!(
            refusal.reason,
            LauncherUpdateRefusalReason::UpdatePlanChanged
        );
    }

    #[test]
    fn unsupported_schema_version_refuses() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);
        let receipt_path =
            write_launcher_receipt(&launcher, distribution(), direct_ownership()).unwrap();
        let mut receipt: LauncherReceipt =
            serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
        receipt.schema_version = 2;
        std::fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();

        let refusal = refusal(plan_launcher_update(&launcher, "demo-installer"));

        assert_eq!(refusal.reason, LauncherUpdateRefusalReason::InvalidReceipt);
        assert!(refusal.message.contains("unsupported schema version 2"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_launcher_input_is_refused() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);
        let symlink = temp.path().join("demo-link");
        std::os::unix::fs::symlink(&launcher, &symlink).unwrap();

        let write_error =
            write_launcher_receipt(&symlink, distribution(), direct_ownership()).unwrap_err();
        let refusal = refusal(plan_launcher_update(&symlink, "demo-installer"));

        assert_eq!(
            refusal.reason,
            LauncherUpdateRefusalReason::LauncherUnavailable
        );
        assert!(write_error.to_string().contains("exact regular-file"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_receipt_sidecar_is_refused() {
        let temp = TempDir::new().unwrap();
        let launcher = launcher(&temp);
        let receipt_path = receipt_path_for_launcher(&launcher).unwrap();
        let target = temp.path().join("receipt-target.json");
        std::fs::write(&target, b"{}").unwrap();
        std::os::unix::fs::symlink(&target, &receipt_path).unwrap();

        let refusal = refusal(plan_launcher_update(&launcher, "demo-installer"));

        assert_eq!(refusal.reason, LauncherUpdateRefusalReason::InvalidReceipt);
        assert!(refusal.message.contains("not a regular file"));
    }
}
