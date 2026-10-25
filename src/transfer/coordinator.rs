use super::model::*;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::Instant,
};

pub struct Coordinator {
    pub receipt: TransferReceipt,
    file: File,
    part: PathBuf,
    final_path: PathBuf,
    receipt_path: PathBuf,

    _lock: File,
    hash: Sha256,
    started: Instant,
    start_bytes: u64,
    sink: ProgressSink,
    pub cancellation: CancellationToken,
}
impl Coordinator {
    pub fn open(
        receipt: TransferReceipt,
        receipt_path: &Path,
        sink: ProgressSink,
        cancellation: CancellationToken,
    ) -> Result<Self, TransferError> {
        receipt.source.validate()?;
        if receipt.protocol_version != 1
            || receipt.id.is_nil()
            || !receipt.encrypted
            || receipt.format != "restic-repository-tar-v1"
            || !receipt.destination.directory.is_absolute()
        {
            return Err(TransferError::InvalidReceipt);
        }
        let directory = fs::canonicalize(&receipt.destination.directory)?;
        if directory != receipt.destination.directory || !directory.is_dir() {
            return Err(TransferError::PathViolation);
        }
        let final_path = directory.join(format!("{}.csbackup.tar", receipt.source.artifact_id));
        if fs::symlink_metadata(&final_path).is_ok() {
            return Err(TransferError::Collision);
        }
        let lock_path = directory.join(format!(
            ".containersafe-{}.lock",
            receipt.source.artifact_id
        ));
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|_| TransferError::Collision)?;
        lock.try_lock().map_err(|_| TransferError::Collision)?;
        let outcome = (|| {
            let part = directory.join(format!(".containersafe-{}.part", receipt.id));
            let mut file = if part.exists() || fs::symlink_metadata(&part).is_ok() {
                let old: TransferReceipt = serde_json::from_slice(&fs::read(receipt_path)?)
                    .map_err(|_| TransferError::InvalidReceipt)?;
                if old.source != receipt.source
                    || old.destination != receipt.destination
                    || old.id != receipt.id
                    || old.kind != receipt.kind
                    || receipt.kind != TransferKind::SftpPull
                {
                    return Err(TransferError::SourceChanged);
                }
                let meta = fs::symlink_metadata(&part)?;
                if !meta.is_file()
                    || meta.file_type().is_symlink()
                    || meta.len() > receipt.source.size
                {
                    return Err(TransferError::PathViolation);
                }
                OpenOptions::new().read(true).write(true).open(&part)?
            } else {
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&part)?
            };
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
            let bytes = file.seek(SeekFrom::End(0))?;
            let mut value = Self {
                receipt,
                file,
                part,
                final_path,
                receipt_path: receipt_path.into(),

                _lock: lock,
                hash,
                started: Instant::now(),
                start_bytes: bytes,
                sink,
                cancellation,
            };
            value.receipt.bytes = bytes;
            value.state(TransferState::VerifyingSource)?;
            Ok(value)
        })();
        if outcome.is_err() {
            // Advisory lock releases automatically, including after process failure.
        }
        outcome
    }
    pub fn state(&mut self, state: TransferState) -> Result<(), TransferError> {
        self.receipt.state = state;
        self.persist()?;
        self.progress();
        Ok(())
    }
    fn persist(&self) -> Result<(), TransferError> {
        let bytes =
            serde_json::to_vec_pretty(&self.receipt).map_err(|_| TransferError::InvalidReceipt)?;
        crate::servers::storage::atomic_write(&self.receipt_path, &bytes)
            .map_err(|_| TransferError::Io)
    }
    fn progress(&self) {
        let elapsed = self.started.elapsed().as_secs_f64();
        (self.sink)(TransferProgress {
            id: self.receipt.id,
            state: self.receipt.state,
            bytes: self.receipt.bytes,
            total: self.receipt.source.size,
            bytes_per_second: self.receipt.bytes.saturating_sub(self.start_bytes) as f64
                / elapsed.max(0.001),
            elapsed_seconds: elapsed,
        });
    }
    pub fn write(&mut self, buffer: &[u8]) -> Result<(), TransferError> {
        self.cancellation.check()?;
        if self.receipt.bytes.saturating_add(buffer.len() as u64) > self.receipt.source.size {
            return Err(TransferError::Incomplete);
        }
        self.file.write_all(buffer)?;
        self.hash.update(buffer);
        self.receipt.bytes += buffer.len() as u64;
        self.progress();
        Ok(())
    }
    pub fn finish(mut self) -> Result<TransferReceipt, TransferError> {
        self.check_cancel()?;
        self.state(TransferState::VerifyingIntegrity)?;
        if self.receipt.bytes != self.receipt.source.size {
            return Err(TransferError::Incomplete);
        }
        if format!("{:x}", self.hash.clone().finalize())
            != self.receipt.source.sha256.to_ascii_lowercase()
        {
            return Err(TransferError::DigestMismatch);
        }
        self.file.sync_all()?;
        self.file.seek(SeekFrom::Start(0))?;
        let mut disk_hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        let mut bytes = 0u64;
        loop {
            self.check_cancel()?;
            let n = self.file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            bytes += n as u64;
            disk_hash.update(&buffer[..n]);
        }
        if bytes != self.receipt.source.size
            || format!("{:x}", disk_hash.finalize())
                != self.receipt.source.sha256.to_ascii_lowercase()
        {
            return Err(TransferError::DigestMismatch);
        }
        // Same filesystem: atomic no-clobber publication; never rename over a user file.
        fs::hard_link(&self.part, &self.final_path).map_err(|_| TransferError::Collision)?;
        fs::remove_file(&self.part)?;
        self.receipt.sha256_verified = true;
        self.state(TransferState::Completed)?;
        Ok(self.receipt.clone())
    }
    fn check_cancel(&mut self) -> Result<(), TransferError> {
        if let Err(error) = self.cancellation.check() {
            self.receipt.state = TransferState::Cancelled;
            return Err(error);
        }
        Ok(())
    }
    pub fn fail(&mut self, error: TransferError) {
        let _ = self.state(if error == TransferError::Cancelled {
            TransferState::Cancelled
        } else {
            TransferState::Failed
        });
    }
}
impl Drop for Coordinator {
    fn drop(&mut self) {
        let _ = self.file.sync_all();
        if !matches!(
            self.receipt.state,
            TransferState::Completed | TransferState::Cancelled | TransferState::Failed
        ) {
            self.receipt.state = TransferState::Failed;
        }
        let _ = self.persist();
        let _ = self._lock.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use uuid::Uuid;
    fn receipt(path: &Path, data: &[u8]) -> TransferReceipt {
        let id = Uuid::new_v4();
        TransferReceipt::new(
            SourceIdentity {
                server_id: Uuid::new_v4(),
                endpoint: "[fixture]:22".into(),
                host_key: "SHA256:fixture".into(),
                artifact_id: id,
                snapshot_id: "b".repeat(64),
                remote_path: format!("/exports/{id}.csbackup.tar"),
                size: data.len() as u64,
                modified: 1,
                sha256: format!("{:x}", Sha256::digest(data)),
            },
            path.into(),
            TransferKind::SftpPull,
        )
        .unwrap()
    }
    #[test]
    fn resume_digest_collision_and_cancel_are_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let data = b"binary\0fixture";
        let receipt = receipt(dir.path(), data);
        let record = dir.path().join("receipt.json");
        let token = CancellationToken::default();
        let mut coordinator =
            Coordinator::open(receipt.clone(), &record, Arc::new(|_| {}), token.clone()).unwrap();
        assert!(matches!(
            Coordinator::open(receipt.clone(), &record, Arc::new(|_| {}), token.clone()),
            Err(TransferError::Collision)
        ));
        coordinator.write(&data[..3]).unwrap();
        token.cancel();
        assert_eq!(
            coordinator.write(&data[3..]).unwrap_err(),
            TransferError::Cancelled
        );
        coordinator.fail(TransferError::Cancelled);
        drop(coordinator);
        let mut changed = receipt.clone();
        changed.source.modified = 2;
        assert!(matches!(
            Coordinator::open(
                changed,
                &record,
                Arc::new(|_| {}),
                CancellationToken::default()
            ),
            Err(TransferError::SourceChanged)
        ));
        let mut resumed = Coordinator::open(
            receipt.clone(),
            &record,
            Arc::new(|_| {}),
            CancellationToken::default(),
        )
        .unwrap();
        assert_eq!(resumed.receipt.bytes, 3);
        resumed.write(&data[3..]).unwrap();
        let result = resumed.finish().unwrap();
        assert!(result.sha256_verified);
        assert!(matches!(
            Coordinator::open(
                receipt,
                &record,
                Arc::new(|_| {}),
                CancellationToken::default()
            ),
            Err(TransferError::Collision)
        ));
    }
    #[test]
    fn truncation_and_wrong_digest_never_publish_a_final_file() {
        for complete in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let receipt = receipt(dir.path(), b"expected");
            let final_path = dir
                .path()
                .join(format!("{}.csbackup.tar", receipt.source.artifact_id));
            let mut job = Coordinator::open(
                receipt,
                &dir.path().join("r.json"),
                Arc::new(|_| {}),
                CancellationToken::default(),
            )
            .unwrap();
            job.write(if complete { b"modified" } else { b"mod" })
                .unwrap();
            assert_eq!(
                job.finish().unwrap_err(),
                if complete {
                    TransferError::DigestMismatch
                } else {
                    TransferError::Incomplete
                }
            );
            assert!(!final_path.exists());
        }
    }
    #[test]
    fn readback_detects_disk_changes_before_final_publication() {
        let dir = tempfile::tempdir().unwrap();
        let data = b"original";
        let receipt = receipt(dir.path(), data);
        let final_path = dir
            .path()
            .join(format!("{}.csbackup.tar", receipt.source.artifact_id));
        let mut job = Coordinator::open(
            receipt,
            &dir.path().join("r.json"),
            Arc::new(|_| {}),
            CancellationToken::default(),
        )
        .unwrap();
        job.write(data).unwrap();
        job.file.seek(SeekFrom::Start(0)).unwrap();
        job.file.write_all(b"modified").unwrap();
        assert_eq!(job.finish().unwrap_err(), TransferError::DigestMismatch);
        assert!(!final_path.exists());
    }
}
