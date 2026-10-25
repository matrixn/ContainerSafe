use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use uuid::Uuid;

pub type TransferId = Uuid;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferKind {
    SftpPull,
    SshStream,
    ReverseTunnel,
}
pub struct TransferCapabilities {
    pub can_resume: bool,
    pub can_pause: bool,
    pub can_cancel: bool,
}
impl TransferKind {
    pub fn capabilities(self) -> TransferCapabilities {
        TransferCapabilities {
            can_resume: self == Self::SftpPull,
            can_pause: false,
            can_cancel: true,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferState {
    Pending,
    Connecting,
    VerifyingSource,
    Transferring,
    Paused,
    VerifyingIntegrity,
    Completed,
    Cancelled,
    Failed,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceIdentity {
    pub server_id: Uuid,
    pub endpoint: String,
    pub host_key: String,
    pub artifact_id: Uuid,
    pub snapshot_id: String,
    pub remote_path: String,
    pub size: u64,
    pub modified: u32,
    pub sha256: String,
}
impl SourceIdentity {
    pub fn validate(&self) -> Result<(), TransferError> {
        if self.server_id.is_nil()
            || self.artifact_id.is_nil()
            || self.endpoint.is_empty()
            || self.host_key.is_empty()
            || self.size == 0
            || !hex_digest(&self.sha256)
            || !hex_digest(&self.snapshot_id)
            || !self.remote_path.starts_with('/')
            || self.remote_path.contains(['\0', '\n', '\r'])
            || self
                .remote_path
                .split('/')
                .any(|part| part == ".." || part == ".")
            || !self
                .remote_path
                .ends_with(&format!("/{}.csbackup.tar", self.artifact_id))
        {
            return Err(TransferError::InvalidReceipt);
        }
        Ok(())
    }
}
pub fn hex_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationIdentity {
    pub directory: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferProgress {
    pub id: TransferId,
    pub state: TransferState,
    pub bytes: u64,
    pub total: u64,
    pub bytes_per_second: f64,
    pub elapsed_seconds: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferReceipt {
    pub protocol_version: u32,
    pub id: TransferId,
    pub kind: TransferKind,
    pub source: SourceIdentity,
    pub destination: DestinationIdentity,
    pub state: TransferState,
    pub bytes: u64,
    pub sha256_verified: bool,
    pub format: String,
    pub encrypted: bool,
}
impl TransferReceipt {
    pub fn new(
        source: SourceIdentity,
        directory: PathBuf,
        kind: TransferKind,
    ) -> Result<Self, TransferError> {
        source.validate()?;
        Ok(Self {
            protocol_version: 1,
            id: Uuid::new_v4(),
            kind,
            source,
            destination: DestinationIdentity {
                directory: std::fs::canonicalize(directory)?,
            },
            state: TransferState::Pending,
            bytes: 0,
            sha256_verified: false,
            format: "restic-repository-tar-v1".into(),
            encrypted: true,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferError {
    Io,
    Collision,
    InvalidReceipt,
    PathViolation,
    SourceChanged,
    DigestMismatch,
    Cancelled,
    Timeout,
    Incomplete,
    RemoteFailed,
    SftpUnavailable,
    ForwardingDenied,
    BackendUnavailable,
}
impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Transfer: {self:?}")
    }
}
impl std::error::Error for TransferError {}
impl From<std::io::Error> for TransferError {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}
#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn check(&self) -> Result<(), TransferError> {
        if self.0.load(Ordering::SeqCst) {
            Err(TransferError::Cancelled)
        } else {
            Ok(())
        }
    }
    pub async fn cancelled(&self) {
        while self.check().is_ok() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}
pub type ProgressSink = Arc<dyn Fn(TransferProgress) + Send + Sync>;
