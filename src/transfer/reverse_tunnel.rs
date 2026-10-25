//! Session-scoped, loopback-only reverse forwarding to the official rest-server.
use super::*;
use crate::{restic::RestCredentials, servers::connection::Connection};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU8},
    time::{Duration, Instant},
};
use std::{
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct ForwardTarget {
    pub local_port: u16,
    pub remote_port: u32,
    pub cancellation: CancellationToken,
    pub bytes: Arc<AtomicU64>,
    pub slots: Arc<tokio::sync::Semaphore>,
}

pub struct ReverseSession<'a> {
    connection: &'a Connection,
    backend: super::local_backend::LocalBackend,
    pub port: u16,
    pub repository_id: Uuid,
    pub cancellation: CancellationToken,
    state: Arc<AtomicU8>,
    stop: Arc<AtomicBool>,
    ticker: Option<std::thread::JoinHandle<()>>,
}
impl<'a> ReverseSession<'a> {
    pub fn open(
        connection: &'a Connection,
        backend_executable: &Path,
        base: &Path,
        repository_id: Uuid,
        cancellation: CancellationToken,
        sink: ProgressSink,
    ) -> Result<Self, TransferError> {
        cancellation.check()?;
        let identity = connection
            .data_identity()
            .map_err(|_| TransferError::SourceChanged)?;
        let context = super::local_backend::RepositoryContext::open(base, repository_id, identity)?;
        let backend = super::local_backend::LocalBackend::start(backend_executable, context)?;
        let bytes = Arc::new(AtomicU64::new(0));
        // Keep REST bridges alive briefly for Restic's lock cleanup after command
        // cancellation. Forward teardown owns this distinct lifecycle token.
        let port = connection.start_reverse_forward(
            backend.port,
            CancellationToken::default(),
            bytes.clone(),
        )? as u16;
        let state = Arc::new(AtomicU8::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let mut session = Self {
            connection,
            backend,
            port,
            repository_id,
            cancellation,
            state: state.clone(),
            stop: stop.clone(),
            ticker: None,
        };
        session.verify_remote_loopback()?;
        let id = Uuid::new_v4();
        let started = Instant::now();
        session.ticker = Some(std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let elapsed = started.elapsed().as_secs_f64();
                let bytes = bytes.load(Ordering::Relaxed);
                sink(TransferProgress {
                    id,
                    state: if state.load(Ordering::Relaxed) == 1 {
                        TransferState::VerifyingIntegrity
                    } else {
                        TransferState::Transferring
                    },
                    bytes,
                    total: 0,
                    bytes_per_second: bytes as f64 / elapsed.max(0.001),
                    elapsed_seconds: elapsed,
                });
                std::thread::sleep(Duration::from_millis(100));
            }
        }));
        Ok(session)
    }
    fn verify_remote_loopback(&self) -> Result<(), TransferError> {
        let script =
            "set -eu\ncat /proc/net/tcp\nif test -r /proc/net/tcp6; then cat /proc/net/tcp6; fi\n";
        let bytes = self
            .connection
            .restic_script(script, &[], 15)
            .map_err(|_| TransferError::ForwardingDenied)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| TransferError::ForwardingDenied)?;
        let mut found = false;
        for line in text.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 4 || fields[3] != "0A" {
                continue;
            }
            let Some((address, port)) = fields[1].split_once(':') else {
                continue;
            };
            if u16::from_str_radix(port, 16).ok() != Some(self.port) {
                continue;
            }
            if address != "0100007F" && address != "00000000000000000000000001000000" {
                return Err(TransferError::ForwardingDenied);
            }
            found = true;
        }
        if found {
            Ok(())
        } else {
            Err(TransferError::ForwardingDenied)
        }
    }
    pub fn repository_path(&self) -> PathBuf {
        self.backend.context.repository.clone()
    }
    pub fn source_identity(&self) -> Result<crate::servers::storage::DataIdentity, String> {
        self.connection.data_identity()
    }
    pub fn uri(&self) -> String {
        format!("rest:http://127.0.0.1:{}/repository/", self.port)
    }
    pub fn credentials(&self) -> &RestCredentials {
        &self.backend.credentials
    }
    pub fn verifying(&self) {
        self.state.store(1, Ordering::Relaxed);
    }
    pub fn alive(&mut self) -> bool {
        self.backend.alive()
    }
}
impl Drop for ReverseSession<'_> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.connection.stop_reverse_forward(self.port as u32);
        if let Some(ticker) = self.ticker.take() {
            let _ = ticker.join();
        }
    }
}
pub(crate) struct Counted<T> {
    pub inner: T,
    pub bytes: Arc<AtomicU64>,
}
impl<T: AsyncRead + Unpin> AsyncRead for Counted<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buffer.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buffer);
        if matches!(result, Poll::Ready(Ok(()))) {
            self.bytes
                .fetch_add((buffer.filled().len() - before) as u64, Ordering::Relaxed);
        }
        result
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for Counted<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buffer)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
