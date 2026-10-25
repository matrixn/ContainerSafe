//! Lifecycle wrapper for the maintained official restic/rest-server binary.
//! Uses private repositories, append-only writes and ephemeral bcrypt credentials.
use super::*;
use crate::{restic::RestCredentials, servers::storage::DataIdentity};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};
use uuid::Uuid;

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Binding {
    repository_id: Uuid,
    source: DataIdentity,
}
pub struct RepositoryContext {
    pub directory: PathBuf,
    pub repository: PathBuf,
    _lock: File,
}
impl RepositoryContext {
    pub fn open(
        base: &Path,
        repository_id: Uuid,
        source: DataIdentity,
    ) -> Result<Self, TransferError> {
        if !base.is_absolute() || repository_id.is_nil() {
            return Err(TransferError::PathViolation);
        }
        let base = fs::canonicalize(base)?;
        let server_directory = base.join(source.server_id.to_string());
        if fs::symlink_metadata(&server_directory).is_ok() {
            if fs::canonicalize(&server_directory)? != server_directory {
                return Err(TransferError::PathViolation);
            }
        } else {
            fs::create_dir(&server_directory)?;
        }
        let directory = base
            .join(source.server_id.to_string())
            .join(repository_id.to_string());
        fs::create_dir_all(&directory)?;
        if fs::canonicalize(&directory)? != directory {
            return Err(TransferError::PathViolation);
        }
        check_tree(&directory, 0)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(".operation.lock"))?;
        lock.try_lock().map_err(|_| TransferError::Collision)?;
        let binding = Binding {
            repository_id,
            source,
        };
        let identity = directory.join("server-binding.json");
        if identity.exists() {
            let old: Binding = serde_json::from_slice(&fs::read(&identity)?)
                .map_err(|_| TransferError::InvalidReceipt)?;
            if old != binding {
                return Err(TransferError::SourceChanged);
            }
        } else {
            if fs::read_dir(&directory)?
                .any(|entry| entry.is_ok_and(|entry| entry.file_name() != ".operation.lock"))
            {
                return Err(TransferError::Collision);
            }
            crate::servers::storage::atomic_write(
                &identity,
                &serde_json::to_vec(&binding).map_err(|_| TransferError::InvalidReceipt)?,
            )
            .map_err(|_| TransferError::Io)?;
        }
        let repository = directory.join("repository");
        Ok(Self {
            directory,
            repository,
            _lock: lock,
        })
    }
}
fn check_tree(path: &Path, depth: usize) -> Result<(), TransferError> {
    if depth > 8 {
        return Err(TransferError::PathViolation);
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(TransferError::PathViolation);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(TransferError::PathViolation);
        }
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            check_tree(&entry?.path(), depth + 1)?;
        }
    } else if !metadata.is_file() {
        return Err(TransferError::PathViolation);
    }
    Ok(())
}

pub struct LocalBackend {
    child: Child,
    _auth_file: tempfile::TempDir,
    pub context: RepositoryContext,
    pub credentials: RestCredentials,
    pub port: u16,
    readers: Vec<std::thread::JoinHandle<()>>,
}
impl LocalBackend {
    pub fn start(executable: &Path, context: RepositoryContext) -> Result<Self, TransferError> {
        if !executable.is_absolute() || !executable.is_file() {
            return Err(TransferError::BackendUnavailable);
        }
        let credentials = RestCredentials {
            username: "repository".into(),
            password: format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
        };
        let hash = bcrypt::hash(&credentials.password, 12)
            .map_err(|_| TransferError::BackendUnavailable)?;
        let auth = tempfile::tempdir()?;
        let auth_path = auth.path().join("session.htpasswd");
        fs::write(&auth_path, format!("repository:{hash}\n"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&auth_path, fs::Permissions::from_mode(0o600))?;
        }
        let mut command = Command::new(executable);
        command
            .args([
                "--listen",
                "127.0.0.1:0",
                "--private-repos",
                "--append-only",
                "--path",
            ])
            .arg(&context.directory)
            .arg("--htpasswd-file")
            .arg(&auth_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .map_err(|_| TransferError::BackendUnavailable)?;
        let (sender, receiver) = mpsc::sync_channel(2);
        let mut readers = Vec::new();
        for stream in [
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let sender = sender.clone();
            readers.push(std::thread::spawn(move || {
                let mut stream = stream;
                let mut buffer = [0u8; 1024];
                let mut line = Vec::new();
                let mut seen = 0usize;
                while let Ok(n) = stream.read(&mut buffer) {
                    if n == 0 {
                        break;
                    }
                    for byte in &buffer[..n] {
                        seen = seen.saturating_add(1);
                        if *byte == b'\n' {
                            if let Ok(text) = std::str::from_utf8(&line) {
                                if let Some(port) = text
                                    .trim()
                                    .strip_prefix("start server on 127.0.0.1:")
                                    .and_then(|p| p.parse::<u16>().ok())
                                {
                                    let _ = sender.try_send(port);
                                }
                            }
                            line.clear();
                        } else if line.len() < 4096 && seen < 65536 {
                            line.push(*byte);
                        }
                    }
                }
            }));
        }
        let mut backend = Self {
            child,
            _auth_file: auth,
            context,
            credentials,
            port: 0,
            readers,
        };
        backend.port = receiver
            .recv_timeout(Duration::from_secs(15))
            .map_err(|_| TransferError::BackendUnavailable)?;
        if backend.port == 0 || backend.child.try_wait()?.is_some() {
            return Err(TransferError::BackendUnavailable);
        }
        // Verify authentication is enabled, and one permitted repository context.
        if backend.status("/repository/config", false)? != 401 {
            return Err(TransferError::BackendUnavailable);
        }
        if !matches!(backend.status("/repository/config", true)?, 200 | 404) {
            return Err(TransferError::BackendUnavailable);
        }
        if backend.status("/other/config", true)? == 200 {
            return Err(TransferError::PathViolation);
        }
        Ok(backend)
    }
    pub fn status(&self, path: &str, authorized: bool) -> Result<u16, TransferError> {
        if path.contains(['\r', '\n']) {
            return Err(TransferError::PathViolation);
        }
        let mut socket = TcpStream::connect_timeout(
            &SocketAddr::from(([127, 0, 0, 1], self.port)),
            Duration::from_secs(2),
        )?;
        socket.set_read_timeout(Some(Duration::from_secs(2)))?;
        socket.set_write_timeout(Some(Duration::from_secs(2)))?;
        let auth = if authorized {
            STANDARD.encode(format!(
                "{}:{}",
                self.credentials.username, self.credentials.password
            ))
        } else {
            STANDARD.encode("repository:invalid")
        };
        write!(socket,"GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Basic {auth}\r\nConnection: close\r\n\r\n")?;
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        while line.len() < 1024 {
            if socket.read(&mut byte)? == 0 {
                break;
            }
            line.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }
        std::str::from_utf8(&line)
            .ok()
            .and_then(|text| text.lines().next())
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .ok_or(TransferError::BackendUnavailable)
    }
    pub fn alive(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_none()
    }
}
impl Drop for LocalBackend {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
    }
}
