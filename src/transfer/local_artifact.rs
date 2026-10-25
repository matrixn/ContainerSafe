//! Verified encrypted artifact extraction, never a general archive extractor.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
};

pub struct ExtractedArtifact {
    pub temporary: tempfile::TempDir,
}
impl ExtractedArtifact {
    pub fn repository(&self) -> std::path::PathBuf {
        self.temporary.path().join("repository")
    }
}
pub fn extract(
    receipt: &TransferReceipt,
    cancellation: &CancellationToken,
) -> Result<ExtractedArtifact, TransferError> {
    if receipt.state != TransferState::Completed
        || !receipt.sha256_verified
        || !receipt.encrypted
        || receipt.format != "restic-repository-tar-v1"
    {
        return Err(TransferError::InvalidReceipt);
    }
    receipt.source.validate()?;
    let path = receipt
        .destination
        .directory
        .join(format!("{}.csbackup.tar", receipt.source.artifact_id));
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() != receipt.source.size
    {
        return Err(TransferError::SourceChanged);
    }
    let mut file = File::open(&path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        cancellation.check()?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    if format!("{:x}", hash.finalize()) != receipt.source.sha256.to_ascii_lowercase() {
        return Err(TransferError::DigestMismatch);
    }
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(0))?;
    let temporary = tempfile::tempdir()?;
    let mut archive = tar::Archive::new(file);
    let mut count = 0u64;
    let mut total = 0u64;
    for entry in archive.entries()? {
        cancellation.check()?;
        let mut entry = entry?;
        count += 1;
        if count > 1_000_000 {
            return Err(TransferError::PathViolation);
        }
        let raw = entry.path_bytes();
        let raw = std::str::from_utf8(&raw).map_err(|_| TransferError::PathViolation)?;
        if !allowed(raw, entry.header().entry_type().is_dir()) {
            return Err(TransferError::PathViolation);
        }
        let path = temporary.path().join(raw.trim_end_matches('/'));
        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(path)?;
            continue;
        }
        if !entry.header().entry_type().is_file() {
            return Err(TransferError::PathViolation);
        }
        total = total
            .checked_add(entry.size())
            .ok_or(TransferError::PathViolation)?;
        if total > receipt.source.size {
            return Err(TransferError::PathViolation);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = OpenOptions::new().create_new(true).write(true).open(path)?;
        loop {
            cancellation.check()?;
            let n = entry.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            output.write_all(&buffer[..n])?;
        }
        output.sync_all()?;
    }
    if !temporary.path().join("repository/config").is_file() {
        return Err(TransferError::Incomplete);
    }
    Ok(ExtractedArtifact { temporary })
}
fn allowed(raw: &str, directory: bool) -> bool {
    if raw.contains(['\\', ':', '\0']) || raw.starts_with('/') {
        return false;
    }
    let parts: Vec<_> = raw.trim_end_matches('/').split('/').collect();
    if parts.first() != Some(&"repository")
        || parts
            .iter()
            .any(|p| p.is_empty() || *p == "." || *p == "..")
    {
        return false;
    }
    if directory {
        return parts.len() == 1
            || (parts.len() == 2
                && matches!(parts[1], "data" | "index" | "keys" | "snapshots" | "locks"))
            || (parts.len() == 3
                && parts[1] == "data"
                && parts[2].len() == 2
                && parts[2].bytes().all(|b| b.is_ascii_hexdigit()));
    }
    if parts.len() == 2 && parts[1] == "config" {
        return true;
    }
    (parts.len() == 3
        && matches!(parts[1], "index" | "keys" | "snapshots" | "locks")
        && hex_digest(parts[2]))
        || (parts.len() == 4
            && parts[1] == "data"
            && parts[2].len() == 2
            && parts[2].bytes().all(|b| b.is_ascii_hexdigit())
            && hex_digest(parts[3]))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restricts_restic_layout_and_rejects_escape() {
        for p in [
            "../x",
            "repository/../x",
            "/repository/config",
            "repository/a:stream",
            "repository/data/xx/a",
            "repository/evil",
            "repository/keys/../../../x",
        ] {
            assert!(!allowed(p, false), "{p}");
        }
        assert!(allowed("repository/config", false));
        assert!(allowed(
            &format!("repository/data/ab/{}", "a".repeat(64)),
            false
        ));
    }
}
