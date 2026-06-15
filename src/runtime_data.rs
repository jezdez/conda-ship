//! Runtime data stamped onto generated conda-ship artifacts.
//!
//! This module is shared by the builder and runtime binaries. The builder uses
//! the writer path, while the runtime uses the reader path.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use sha2::{Digest, Sha256};

const FOOTER_MAGIC: &[u8; 16] = b"CONDA_SHIP_V0001";
const FORMAT_VERSION: u32 = 1;
const FOOTER_LEN: usize = 8 + 8 + 32 + 32 + 4 + FOOTER_MAGIC.len();
#[allow(dead_code)]
const MAX_HEADER_LEN: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RuntimeConfig {
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub packages: Vec<String>,
}

impl RuntimeConfig {
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty() && self.packages.is_empty()
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    clap::ValueEnum,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Eq,
)]
#[serde(rename_all = "kebab-case")]
pub enum InstallScheme {
    #[default]
    CondaHome,
    UserData,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RuntimeDataHeader {
    pub schema_version: u32,
    pub runtime_name: String,
    pub runtime_version: String,
    pub embedded_runtime_name: String,
    pub delegate: String,
    pub display_name: String,
    #[serde(default)]
    pub install_scheme: InstallScheme,
    pub install_name: String,
    pub metadata_file: String,
    pub bundle_env_var: String,
    pub offline_env_var: String,
    pub docs_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_method: Option<String>,
    #[serde(default)]
    pub runtime_config: RuntimeConfig,
    #[serde(default)]
    pub runtime_lock: String,
}

impl RuntimeDataHeader {
    pub fn for_name(name: &str) -> Self {
        Self {
            schema_version: FORMAT_VERSION,
            runtime_name: name.to_string(),
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
            embedded_runtime_name: name.to_string(),
            delegate: "conda".to_string(),
            display_name: name.to_string(),
            install_scheme: InstallScheme::CondaHome,
            install_name: name.to_string(),
            metadata_file: format!(".{name}.json"),
            bundle_env_var: runtime_env_var(name, "BUNDLE"),
            offline_env_var: runtime_env_var(name, "OFFLINE"),
            docs_url: "https://jezdez.github.io/conda-ship/".to_string(),
            install_method: None,
            runtime_config: RuntimeConfig::default(),
            runtime_lock: String::new(),
        }
    }
}

impl Default for RuntimeDataHeader {
    fn default() -> Self {
        Self::for_name("cs-template")
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct EmbeddedBundle {
    executable: PathBuf,
    offset: u64,
    len: u64,
    sha256: [u8; 32],
}

#[allow(dead_code)]
impl EmbeddedBundle {
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn open(&self) -> io::Result<std::io::Take<File>> {
        let mut file = File::open(&self.executable)?;
        file.seek(SeekFrom::Start(self.offset))?;
        Ok(file.take(self.len))
    }

    pub fn verify(&self) -> io::Result<()> {
        let mut file = File::open(&self.executable)?;
        let actual = hash_file_range(&mut file, self.offset, self.len)?;
        if actual != self.sha256 {
            return Err(invalid_data("embedded bundle checksum mismatch"));
        }
        Ok(())
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct RuntimeData {
    pub header: RuntimeDataHeader,
    pub bundle: Option<EmbeddedBundle>,
    pub stamped: bool,
}

#[allow(dead_code)]
static CURRENT_RUNTIME_DATA: LazyLock<RuntimeData> = LazyLock::new(|| match from_current_exe() {
    Ok(Some(data)) => data,
    Ok(None) => RuntimeData::default(),
    Err(err) => {
        eprintln!("error: invalid conda-ship runtime data: {err}");
        std::process::exit(1);
    }
});

#[allow(dead_code)]
pub fn current() -> &'static RuntimeData {
    &CURRENT_RUNTIME_DATA
}

#[allow(dead_code)]
pub fn append_to_binary(
    binary: &Path,
    header: &RuntimeDataHeader,
    bundle: Option<&Path>,
) -> io::Result<()> {
    if header.schema_version != FORMAT_VERSION {
        return Err(invalid_data(format!(
            "unsupported runtime data schema version: {}",
            header.schema_version
        )));
    }

    let header_bytes = serde_json::to_vec(header).map_err(invalid_data)?;
    let header_len = u64::try_from(header_bytes.len())
        .map_err(|_| invalid_data("runtime data header is too large"))?;
    let bundle_len = match bundle {
        Some(path) => std::fs::metadata(path)?.len(),
        None => 0,
    };

    let header_sha256 = digest_to_array(Sha256::digest(&header_bytes));
    let mut bundle_hasher = Sha256::new();

    let mut output = OpenOptions::new().append(true).open(binary)?;
    output.write_all(&header_bytes)?;

    if let Some(path) = bundle {
        let mut input = File::open(path)?;
        let mut buf = [0_u8; 64 * 1024];
        loop {
            let read = input.read(&mut buf)?;
            if read == 0 {
                break;
            }
            bundle_hasher.update(&buf[..read]);
            output.write_all(&buf[..read])?;
        }
    }

    let bundle_sha256 = digest_to_array(bundle_hasher.finalize());
    output.write_all(&encode_footer(
        header_len,
        bundle_len,
        header_sha256,
        bundle_sha256,
    ))?;
    Ok(())
}

#[allow(dead_code)]
pub fn from_current_exe() -> io::Result<Option<RuntimeData>> {
    let exe = std::env::current_exe()?;
    read_from_path(&exe)
}

#[allow(dead_code)]
pub fn read_from_path(path: &Path) -> io::Result<Option<RuntimeData>> {
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();
    if file_len < FOOTER_LEN as u64 {
        return Ok(None);
    }

    file.seek(SeekFrom::End(-(FOOTER_LEN as i64)))?;
    let mut footer = [0_u8; FOOTER_LEN];
    file.read_exact(&mut footer)?;

    let Some(decoded) = decode_footer(&footer)? else {
        return Ok(None);
    };

    if decoded.header_len > MAX_HEADER_LEN {
        return Err(invalid_data(format!(
            "runtime data header is too large: {} bytes",
            decoded.header_len
        )));
    }

    let payload_len = decoded
        .header_len
        .checked_add(decoded.bundle_len)
        .ok_or_else(|| invalid_data("runtime data length overflow"))?;
    let trailer_len = payload_len
        .checked_add(FOOTER_LEN as u64)
        .ok_or_else(|| invalid_data("runtime data trailer length overflow"))?;
    if trailer_len > file_len {
        return Err(invalid_data(
            "runtime data footer points before start of file",
        ));
    }
    let payload_start = file_len - trailer_len;

    file.seek(SeekFrom::Start(payload_start))?;
    let header_len = usize::try_from(decoded.header_len)
        .map_err(|_| invalid_data("runtime data header does not fit in memory"))?;
    let mut header_bytes = vec![0_u8; header_len];
    file.read_exact(&mut header_bytes)?;
    let actual_header_sha256 = digest_to_array(Sha256::digest(&header_bytes));
    if actual_header_sha256 != decoded.header_sha256 {
        return Err(invalid_data("runtime data header checksum mismatch"));
    }
    let header: RuntimeDataHeader = serde_json::from_slice(&header_bytes).map_err(invalid_data)?;
    if header.schema_version != FORMAT_VERSION {
        return Err(invalid_data(format!(
            "unsupported runtime data schema version: {}",
            header.schema_version
        )));
    }

    let bundle = (decoded.bundle_len > 0).then(|| EmbeddedBundle {
        executable: path.to_path_buf(),
        offset: payload_start + decoded.header_len,
        len: decoded.bundle_len,
        sha256: decoded.bundle_sha256,
    });

    Ok(Some(RuntimeData {
        header,
        bundle,
        stamped: true,
    }))
}

#[allow(dead_code)]
struct DecodedFooter {
    header_len: u64,
    bundle_len: u64,
    header_sha256: [u8; 32],
    bundle_sha256: [u8; 32],
}

#[allow(dead_code)]
fn encode_footer(
    header_len: u64,
    bundle_len: u64,
    header_sha256: [u8; 32],
    bundle_sha256: [u8; 32],
) -> [u8; FOOTER_LEN] {
    let mut footer = [0_u8; FOOTER_LEN];
    footer[0..8].copy_from_slice(&header_len.to_le_bytes());
    footer[8..16].copy_from_slice(&bundle_len.to_le_bytes());
    footer[16..48].copy_from_slice(&header_sha256);
    footer[48..80].copy_from_slice(&bundle_sha256);
    footer[80..84].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    footer[84..100].copy_from_slice(FOOTER_MAGIC);
    footer
}

#[allow(dead_code)]
fn decode_footer(footer: &[u8; FOOTER_LEN]) -> io::Result<Option<DecodedFooter>> {
    if &footer[84..100] != FOOTER_MAGIC {
        return Ok(None);
    }

    let version = u32::from_le_bytes(footer[80..84].try_into().unwrap());
    if version != FORMAT_VERSION {
        return Err(invalid_data(format!(
            "unsupported runtime data footer version: {version}"
        )));
    }

    let mut header_sha256 = [0_u8; 32];
    header_sha256.copy_from_slice(&footer[16..48]);
    let mut bundle_sha256 = [0_u8; 32];
    bundle_sha256.copy_from_slice(&footer[48..80]);

    Ok(Some(DecodedFooter {
        header_len: u64::from_le_bytes(footer[0..8].try_into().unwrap()),
        bundle_len: u64::from_le_bytes(footer[8..16].try_into().unwrap()),
        header_sha256,
        bundle_sha256,
    }))
}

#[allow(dead_code)]
fn hash_file_range(file: &mut File, offset: u64, len: u64) -> io::Result<[u8; 32]> {
    file.seek(SeekFrom::Start(offset))?;
    let mut remaining = len;
    let mut buf = [0_u8; 64 * 1024];
    let mut hasher = Sha256::new();
    while remaining > 0 {
        let chunk_len = remaining.min(buf.len() as u64) as usize;
        file.read_exact(&mut buf[..chunk_len])?;
        hasher.update(&buf[..chunk_len]);
        remaining -= chunk_len as u64;
    }
    Ok(digest_to_array(hasher.finalize()))
}

fn digest_to_array(digest: impl AsRef<[u8]>) -> [u8; 32] {
    let mut out = [0_u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}

fn runtime_env_var(name: &str, suffix: &str) -> String {
    let prefix: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("{prefix}_{suffix}")
}

fn invalid_data(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn test_missing_runtime_data_returns_none() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"plain binary").unwrap();

        let data = read_from_path(tmp.path()).unwrap();
        assert!(data.is_none());
    }

    #[test]
    fn test_append_and_read_runtime_data_without_bundle() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"binary").unwrap();

        let mut header = RuntimeDataHeader::for_name("snek");
        header.runtime_lock = "lock data".to_string();
        header.runtime_config.channels = vec!["conda-forge".to_string()];

        append_to_binary(tmp.path(), &header, None).unwrap();
        let data = read_from_path(tmp.path()).unwrap().unwrap();

        assert_eq!(data.header.runtime_name, "snek");
        assert_eq!(data.header.delegate, "conda");
        assert_eq!(data.header.install_scheme, InstallScheme::CondaHome);
        assert_eq!(data.header.install_name, "snek");
        assert_eq!(data.header.runtime_lock, "lock data");
        assert!(data.bundle.is_none());
        assert!(data.stamped);
    }

    #[test]
    fn test_append_and_read_runtime_data_with_bundle() {
        let binary = tempfile::NamedTempFile::new().unwrap();
        let bundle = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(binary.path(), b"binary").unwrap();
        std::fs::write(bundle.path(), b"bundle data").unwrap();

        let header = RuntimeDataHeader::for_name("snek");
        append_to_binary(binary.path(), &header, Some(bundle.path())).unwrap();

        let data = read_from_path(binary.path()).unwrap().unwrap();
        let embedded = data.bundle.unwrap();
        embedded.verify().unwrap();
        let mut contents = String::new();
        embedded
            .open()
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();

        assert_eq!(embedded.len(), "bundle data".len() as u64);
        assert_eq!(contents, "bundle data");
    }

    #[test]
    fn test_corrupt_runtime_data_is_rejected() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"binary").unwrap();

        let header = RuntimeDataHeader::for_name("snek");
        append_to_binary(tmp.path(), &header, None).unwrap();

        let mut file = OpenOptions::new().write(true).open(tmp.path()).unwrap();
        file.seek(SeekFrom::End(-(FOOTER_LEN as i64) - 1)).unwrap();
        file.write_all(b"!").unwrap();

        let err = read_from_path(tmp.path()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("header checksum mismatch"));
    }

    #[test]
    fn test_corrupt_bundle_is_rejected_when_verified() {
        let binary = tempfile::NamedTempFile::new().unwrap();
        let bundle = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(binary.path(), b"binary").unwrap();
        std::fs::write(bundle.path(), b"bundle data").unwrap();

        let header = RuntimeDataHeader::for_name("snek");
        append_to_binary(binary.path(), &header, Some(bundle.path())).unwrap();

        let data = read_from_path(binary.path()).unwrap().unwrap();
        let embedded = data.bundle.unwrap();

        let mut file = OpenOptions::new().write(true).open(binary.path()).unwrap();
        file.seek(SeekFrom::Start(embedded.offset)).unwrap();
        file.write_all(b"!").unwrap();

        let err = embedded.verify().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("bundle checksum mismatch"));
    }
}
