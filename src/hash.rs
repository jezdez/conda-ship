use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn sha256_file(path: &Path) -> io::Result<([u8; 32], u64)> {
    sha256_reader(std::fs::File::open(path)?)
}

pub(crate) fn sha256_reader(mut reader: impl Read) -> io::Result<([u8; 32], u64)> {
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((digest_to_array(hasher.finalize()), bytes))
}

pub(crate) fn digest_to_array(digest: impl AsRef<[u8]>) -> [u8; 32] {
    let mut out = [0_u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}
