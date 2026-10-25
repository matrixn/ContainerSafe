use super::{
    credentials::Secret,
    model::{Authentication, DockerAccess, Server},
};
use russh::{
    client,
    keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
    ChannelMsg,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostIdentity {
    pub endpoint: String,
    pub key_type: String,
    pub fingerprint: String,
}
#[derive(Debug)]
pub enum Failure {
    Message(String),
    UnknownHost(HostIdentity),
    ChangedHost(HostIdentity),
}
impl From<String> for Failure {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionStage {
    Connecting,
    VerifyingHost,
    Authenticating,
    CheckingDocker,
}
type Progress = Arc<dyn Fn(ConnectionStage) + Send + Sync>;

pub struct Connection {
    server: Server,
    session: client::Handle<PinnedHost>,
    runtime: tokio::runtime::Runtime,
    docker_prefix: &'static str,
    host_key: String,
    forwarding: Arc<Mutex<Option<crate::transfer::reverse_tunnel::ForwardTarget>>>,
}

pub(crate) struct PinnedHost {
    endpoint: String,
    fingerprint: Option<String>,
    rejection: Arc<Mutex<Option<Failure>>>,
    progress: Progress,
    forwarding: Arc<Mutex<Option<crate::transfer::reverse_tunnel::ForwardTarget>>>,
}
impl client::Handler for PinnedHost {
    type Error = russh::Error;
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let target = self
            .forwarding
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .filter(|target| {
                connected_address == "127.0.0.1" && target.remote_port == connected_port
            });
        let Some(target) = target else {
            return Ok(());
        };
        let Ok(slot) = target.slots.clone().try_acquire_owned() else {
            return Ok(());
        };
        let stream = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::TcpStream::connect(("127.0.0.1", target.local_port)),
        )
        .await;
        let Ok(Ok(mut stream)) = stream else {
            return Ok(());
        };
        reply.accept().await;
        tokio::spawn(async move {
            let _slot = slot;
            let mut remote = crate::transfer::reverse_tunnel::Counted {
                inner: channel.into_stream(),
                bytes: target.bytes,
            };
            tokio::select! {_=target.cancellation.cancelled()=>{}, _=tokio::io::copy_bidirectional(&mut remote,&mut stream)=>{}}
            use tokio::io::AsyncWriteExt;
            let _ = tokio::time::timeout(Duration::from_secs(1), remote.shutdown()).await;
        });
        Ok(())
    }
    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        (self.progress)(ConnectionStage::VerifyingHost);
        // Pin the same SSH wire-format SHA256 fingerprint used by the old backend.
        // Certificates require a separate CA policy; never implicitly trust their key.
        if key.certificate().is_some() {
            if let Ok(mut rejection) = self.rejection.lock() {
                *rejection = Some(Failure::Message("SSH: host certificates require a separate CA trust policy and are not supported by this interface".into()));
            }
            return Ok(false);
        }
        let key = key.public_key();
        let identity = HostIdentity {
            endpoint: self.endpoint.clone(),
            key_type: key.algorithm().to_string(),
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
        };
        match verify_host(&identity, self.fingerprint.as_deref()) {
            Ok(()) => Ok(true),
            Err(failure) => {
                if let Ok(mut rejection) = self.rejection.lock() {
                    *rejection = Some(failure);
                }
                Ok(false)
            }
        }
    }
}
const DIRECT: [&str; 5] = [
    "docker",
    "/usr/bin/docker",
    "/usr/local/bin/docker",
    "/var/packages/ContainerManager/target/usr/bin/docker",
    "/var/packages/Docker/target/usr/bin/docker",
];
const SUDO: [&str; 5] = [
    "sudo -n docker",
    "sudo -n /usr/bin/docker",
    "sudo -n /usr/local/bin/docker",
    "sudo -n /var/packages/ContainerManager/target/usr/bin/docker",
    "sudo -n /var/packages/Docker/target/usr/bin/docker",
];

// Remote output is untrusted and can contain sensitive data. Only return fixed
// diagnostic categories, never the raw output (including in failed probes).
fn docker_failure(status: i32, output: &[u8]) -> String {
    let text = String::from_utf8_lossy(&output[..output.len().min(16384)]).to_lowercase();
    let reason = if text.contains("a password is required")
        || text.contains("no password was provided")
        || text.contains("a terminal is required")
    {
        "sudo requires a password; passwordless sudo -n is not configured for this command"
    } else if text.contains("must have a tty") || text.contains("no tty present") {
        "sudo requires an interactive terminal; this SSH connection is non-interactive"
    } else if text.contains("not in the sudoers")
        || text.contains("not allowed to execute")
        || text.contains("may not run sudo")
    {
        "sudo policy does not allow this user to run the command"
    } else if status == 127 || text.contains("command not found") || text.contains(": not found") {
        "Docker or sudo executable not found at this path"
    } else if text.contains("permission denied") || text.contains("access denied") {
        if text.contains("sudo:") {
            "sudo permission denied before Docker could run; check the remote sudo policy"
        } else if text.contains("socket") || text.contains("docker daemon") {
            "permission denied for the Docker daemon socket"
        } else {
            "remote shell or executable permission denied; Docker may not have started"
        }
    } else if text.contains("cannot connect to the docker daemon")
        || text.contains("is the docker daemon running")
        || text.contains("connection refused")
    {
        "cannot reach the Docker daemon; check Container Manager / Docker service and endpoint"
    } else {
        "command failed; check Docker installation, service and account permissions"
    };
    format!("Docker: {reason} (exit {status})")
}

pub fn verify_host(identity: &HostIdentity, pinned: Option<&str>) -> Result<(), Failure> {
    match pinned {
        Some(key) if key == identity.fingerprint => Ok(()),
        Some(_) => Err(Failure::ChangedHost(identity.clone())),
        None => Err(Failure::UnknownHost(identity.clone())),
    }
}

impl Connection {
    pub(crate) fn start_reverse_forward(
        &self,
        local_port: u16,
        cancellation: crate::transfer::CancellationToken,
        bytes: Arc<std::sync::atomic::AtomicU64>,
    ) -> Result<u32, crate::transfer::TransferError> {
        if local_port == 0
            || self
                .forwarding
                .lock()
                .map_err(|_| crate::transfer::TransferError::Io)?
                .is_some()
        {
            return Err(crate::transfer::TransferError::Collision);
        }
        let port = self.runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(15),
                self.session.tcpip_forward("127.0.0.1", 0),
            )
            .await
            .map_err(|_| crate::transfer::TransferError::Timeout)?
            .map_err(|_| crate::transfer::TransferError::ForwardingDenied)
        })?;
        if port == 0 || port > 65535 {
            return Err(crate::transfer::TransferError::ForwardingDenied);
        }
        *self
            .forwarding
            .lock()
            .map_err(|_| crate::transfer::TransferError::Io)? =
            Some(crate::transfer::reverse_tunnel::ForwardTarget {
                local_port,
                remote_port: port,
                cancellation,
                bytes,
                slots: Arc::new(tokio::sync::Semaphore::new(32)),
            });
        Ok(port)
    }
    pub(crate) fn stop_reverse_forward(&self, port: u32) {
        if let Ok(mut target) = self.forwarding.lock() {
            if let Some(target) = target.take() {
                target.cancellation.cancel();
            }
        }
        let _ = self.runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(2),
                self.session.cancel_tcpip_forward("127.0.0.1", port),
            )
            .await
        });
    }
    pub fn data_identity(&self) -> Result<super::storage::DataIdentity, String> {
        Ok(super::storage::DataIdentity {
            server_id: self.server.id,
            host: self.server.host.to_ascii_lowercase(),
            port: self.server.port,
            host_key: self.host_key.clone(),
        })
    }
    pub(crate) fn transfer_runtime(&self) -> &tokio::runtime::Runtime {
        &self.runtime
    }
    pub(crate) fn transfer_session(&self) -> &client::Handle<PinnedHost> {
        &self.session
    }
    pub(crate) fn verify_transfer_source(
        &self,
        source: &crate::transfer::SourceIdentity,
    ) -> Result<(), crate::transfer::TransferError> {
        let identity = self
            .data_identity()
            .map_err(|_| crate::transfer::TransferError::InvalidReceipt)?;
        if source.server_id != identity.server_id
            || source.endpoint != format!("[{}]:{}", identity.host, identity.port)
            || source.host_key != identity.host_key
        {
            return Err(crate::transfer::TransferError::SourceChanged);
        }
        source.validate()
    }
    /// Invoke only the configured stdio agent. Credentials stay in SSH; the
    /// request carries job IDs/approved source paths, never repository passwords.
    pub fn agent(
        &self,
        executable: &str,
        config: &str,
        input: &[u8],
        timeout_seconds: u64,
    ) -> Result<Vec<u8>, String> {
        self.agent_cancellable(executable, config, input, timeout_seconds, None)
    }
    pub(crate) fn agent_cancellable(
        &self,
        executable: &str,
        config: &str,
        input: &[u8],
        timeout_seconds: u64,
        cancellation: Option<&crate::transfer::CancellationToken>,
    ) -> Result<Vec<u8>, String> {
        fn quote(value: &str) -> Result<String, String> {
            if !value.starts_with('/') || value.contains(['\0', '\r', '\n']) || value.len() > 4096 {
                return Err("Agent and configuration paths must be absolute Linux paths".into());
            }
            Ok(format!("'{}'", value.replace('\'', "'\\''")))
        }
        if input.len() as u64 > crate::agent::MAX_REQUEST_BYTES
            || !(1..=86400).contains(&timeout_seconds)
        {
            return Err("Invalid agent request size or timeout".into());
        }
        let command = format!("{} --config {}", quote(executable)?, quote(config)?);
        self.input_command(command, input, timeout_seconds, true, cancellation)
    }

    pub(crate) fn restic_script(
        &self,
        script: &str,
        arguments: &[String],
        timeout_seconds: u64,
    ) -> Result<Vec<u8>, String> {
        self.restic_script_cancellable(script, arguments, timeout_seconds, None)
    }
    pub(crate) fn restic_script_cancellable(
        &self,
        script: &str,
        arguments: &[String],
        timeout_seconds: u64,
        cancellation: Option<&crate::transfer::CancellationToken>,
    ) -> Result<Vec<u8>, String> {
        if script.len() > 64 * 1024
            || !(1..=86400).contains(&timeout_seconds)
            || arguments
                .iter()
                .any(|arg| arg.contains(['\0', '\r', '\n']) || arg.len() > 4096)
        {
            return Err("Invalid remote Restic request".into());
        }
        let mut command = String::from("sh -s --");
        for argument in arguments {
            command.push_str(&format!(" '{}'", argument.replace('\'', "'\\''")));
        }
        self.input_command(
            command,
            script.as_bytes(),
            timeout_seconds,
            false,
            cancellation,
        )
    }

    fn input_command(
        &self,
        command: String,
        input: &[u8],
        timeout_seconds: u64,
        agent: bool,
        cancellation: Option<&crate::transfer::CancellationToken>,
    ) -> Result<Vec<u8>, String> {
        self.runtime.block_on(async {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_seconds);
            let mut channel = tokio::time::timeout_at(deadline, self.session.channel_open_session()).await
                .map_err(|_| "SSH: SSH channel timeout")?.map_err(|_| "SSH: SSH channel unavailable")?;
            let operation = async {
                channel.exec(true, command).await.map_err(|_| "SSH: execution failed")?;
                channel.data(input).await.map_err(|_| "SSH: request transmission failed")?;
                channel.eof().await.map_err(|_| "SSH: EOF failed")?;
                let mut output = Vec::new();
                let mut stderr_size = 0usize;
                let mut status = None;
                while let Some(message) = channel.wait().await {
                    match message {
                        ChannelMsg::Data { data } => {
                            if output.len().saturating_add(data.len()) > 16 * 1024 * 1024 {
                                return Err("SSH: output limit exceeded".to_string());
                            }
                            output.extend_from_slice(&data);
                        }
                        ChannelMsg::ExtendedData { data, .. } => {
                            stderr_size = stderr_size.saturating_add(data.len());
                            if stderr_size > 64 * 1024 { return Err("SSH: diagnostic limit exceeded".into()); }
                        }
                        ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                        ChannelMsg::ExitSignal { .. } => return Err("SSH: process terminated".into()),
                        _ => {}
                    }
                }
                match status {
                    Some(0) => Ok(output),
                    Some(1) if agent && !output.is_empty() => Ok(output),
                    _ if agent => Err("Agent unavailable: install the Linux agent and check executable/configuration permissions".into()),
                    Some(code) => Err(match code {
                        3 => "Restic: incomplete backup; some files were not readable. Snapshot is not a complete backup",
                        10 => "Restic: repository missing; initialize it first",
                        11 => "Restic: repository locked by another operation",
                        12 => "Restic: incorrect repository password",
                        70 => "Restic executable missing. Install Restic on the NAS or specify its absolute path",
                        71 => "Password file must be a non-symlink regular file, owned by the SSH user or root, with mode 600 or 400",
                        72 => "Unsafe or unreadable source, or overlap with repository/password file",
                        73 => "Unsafe restore directory: choose a separate existing folder outside sources and repository",
                        74 => "Repository already contains files; do not initialize it again",
                        75 => "Remote path unavailable or access denied. DSM administrator/Docker sudo access does not automatically grant file permissions",
                        76 => "Required NAS utilities unavailable: sh, stat, readlink and id are required",
                        _ => "Restic failed. Check repository access, source permissions and destination credentials on the NAS",
                    }.to_string()),
                    None => Err("SSH: remote command ended without an exit status".into()),
                }
            };
            let result = if let Some(cancellation)=cancellation {
                tokio::select! {
                    _=cancellation.cancelled()=>Ok(Err("Transfer: Cancelled".to_string())),
                    result=tokio::time::timeout_at(deadline,operation)=>result,
                }
            } else {tokio::time::timeout_at(deadline, operation).await};
            if !matches!(&result, Ok(Ok(_))) {
                let _ = tokio::time::timeout(Duration::from_secs(1), channel.signal(russh::Sig::TERM)).await;
                let _ = tokio::time::timeout(Duration::from_secs(1), channel.close()).await;
                if cancellation.is_some_and(|token| token.check().is_err()) {
                    // A remote Restic process receiving TERM/HUP needs the live
                    // REST bridge to remove its own lock. Never unlock for it.
                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
            }
            result.map_err(|_| "SSH operation timed out. The remote process may still be running; inspect snapshots before retrying".to_string())?
        })
    }
    pub fn open(
        server: &Server,
        secret: &Secret,
        hosts: &std::collections::BTreeMap<String, String>,
    ) -> Result<Self, Failure> {
        Self::open_with_progress(server, secret, hosts, |_| {})
    }

    pub fn open_with_progress(
        server: &Server,
        secret: &Secret,
        hosts: &std::collections::BTreeMap<String, String>,
        progress: impl Fn(ConnectionStage) + Send + Sync + 'static,
    ) -> Result<Self, Failure> {
        server.validate()?;
        let progress: Progress = Arc::new(progress);
        progress(ConnectionStage::Connecting);
        let timeout = Duration::from_secs(server.timeout_seconds as u64);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|_| "SSH: runtime initialization failed".to_string())?;
        let endpoint = format!("[{}]:{}", server.host.to_ascii_lowercase(), server.port);
        let rejection = Arc::new(Mutex::new(None));
        let forwarding = Arc::new(Mutex::new(None));
        let handler = PinnedHost {
            fingerprint: hosts.get(&endpoint).cloned(),
            endpoint,
            rejection: rejection.clone(),
            progress: progress.clone(),
            forwarding: forwarding.clone(),
        };
        let config = Arc::new(client::Config {
            keepalive_interval: Some(Duration::from_secs(15)),
            ..client::Config::default()
        });
        let connected = runtime.block_on(async {
            tokio::time::timeout(
                timeout,
                client::connect(config, (server.host.as_str(), server.port), handler),
            )
            .await
        });
        let mut session = match connected {
            Ok(Ok(session)) => session,
            _ => {
                if let Some(failure) = rejection.lock().ok().and_then(|mut r| r.take()) {
                    return Err(failure);
                }
                return Err(Failure::Message("SSH: connection or handshake failed/timed out (check address, port and server algorithms)".into()));
            }
        };
        progress(ConnectionStage::Authenticating);
        // The server key has been verified BEFORE key decryption or credentials.
        let authentication_error = || {
            Failure::Message("SSH: Connected\nHost identity: Verified\nAuthentication: Failed (check credentials, key format/passphrase and server authentication policy)".into())
        };
        let key = match server.authentication {
            Authentication::Password => None,
            Authentication::OpenSsh | Authentication::Putty => Some(
                super::private_key::load(
                    &server.key_path,
                    (!secret.0.is_empty()).then_some(secret.0.as_str()),
                )
                .map_err(Failure::Message)?,
            ),
        };
        if key.is_none() && secret.0.is_empty() {
            return Err(Failure::Message("Authentication: password required".into()));
        }
        let authenticated = runtime.block_on(async {
            tokio::time::timeout(timeout, async {
                if let Some(key) = key {
                    // Never fall back to legacy RSA/SHA-1 authentication.
                    let hash = session
                        .best_supported_rsa_hash()
                        .await?
                        .flatten()
                        .unwrap_or(HashAlg::Sha256);
                    session
                        .authenticate_publickey(
                            &server.username,
                            PrivateKeyWithHashAlg::new(Arc::new(key), Some(hash)),
                        )
                        .await
                } else {
                    session
                        .authenticate_password(&server.username, secret.0.as_str())
                        .await
                }
            })
            .await
        });
        match authenticated {
            Ok(Ok(result)) if result.success() => {}
            _ => return Err(authentication_error()),
        }
        let mut connection = Self {
            server: server.clone(),
            session,
            runtime,
            docker_prefix: "docker",
            host_key: hosts
                .get(&format!(
                    "[{}]:{}",
                    server.host.to_ascii_lowercase(),
                    server.port
                ))
                .cloned()
                .ok_or_else(|| "Missing trusted host identity".to_string())?,
            forwarding,
        };
        progress(ConnectionStage::CheckingDocker);
        let candidates: Vec<_> = match server.docker_access {
            DockerAccess::Direct => DIRECT.to_vec(),
            DockerAccess::Sudo => SUDO.to_vec(),
            DockerAccess::Automatic => DIRECT.into_iter().chain(SUDO).collect(),
        };
        let mut diagnostics = Vec::new();
        for prefix in candidates {
            connection.docker_prefix = prefix;
            match connection.docker("version --format '{{.Server.Version}}'") {
                Ok(_) => return Ok(connection),
                Err(reason) => diagnostics.push(format!("{prefix}: {reason}")),
            }
        }
        Err(Failure::Message(format!(
            "SSH: Connected\nAuthentication: Successful\nHost identity: Verified\nDocker: Access check failed\n\n{}\n\nSSH login does not grant Docker permissions. Passwordless sudo -n never prompts for or uses your SSH password. Check the reported cause on the server, then test again.",
            diagnostics.join("\n")
        )))
    }

    pub fn docker(&self, arguments: &str) -> Result<String, String> {
        self.runtime.block_on(async {
            let deadline = tokio::time::Instant::now()
                + Duration::from_secs(self.server.timeout_seconds as u64);
            let mut channel =
                tokio::time::timeout_at(deadline, self.session.channel_open_session())
                    .await
                    .map_err(|_| "SSH: channel timed out".to_string())?
                    .map_err(|_| "SSH: channel failed or timed out".to_string())?;
            let operation = async {
                channel
                    .exec(true, format!("{} {}", self.docker_prefix, arguments))
                    .await
                    .map_err(|_| "Docker: command failed or timed out".to_string())?;
                channel
                    .eof()
                    .await
                    .map_err(|_| "SSH: EOF failed".to_string())?;
                let mut output = Vec::new();
                let mut status = None;
                while let Some(message) = channel.wait().await {
                    match message {
                        ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                            if output.len().saturating_add(data.len()) > 32 * 1024 * 1024 {
                                return Err("Docker: output limit exceeded".into());
                            }
                            output.extend_from_slice(&data);
                        }
                        ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                        ChannelMsg::ExitSignal { .. } => {
                            return Err("Docker: remote command terminated by signal".into())
                        }
                        _ => {}
                    }
                }
                let status = status.ok_or("Docker: missing exit status")?;
                if status != 0 {
                    return Err(docker_failure(
                        status.try_into().unwrap_or(i32::MAX),
                        &output,
                    ));
                }
                String::from_utf8(output).map_err(|_| "Docker: invalid UTF-8 output".into())
            };
            let result = tokio::time::timeout_at(deadline, operation).await;
            // Close failed/timed-out channels as well; do not leave a command channel open.
            if !matches!(&result, Ok(Ok(_))) {
                let _ = tokio::time::timeout(Duration::from_secs(1), channel.close()).await;
            }
            result.map_err(|_| "Docker: command timed out".to_string())?
        })
    }
}

#[cfg(test)]
mod tests {
    use super::docker_failure;

    #[test]
    fn reports_distinct_actionable_causes_without_remote_output() {
        let cases: &[(i32, &[u8], &str)] = &[
            (1, b"sudo: a password is required\nprivate-fixture-marker", "sudo requires a password"),
            (1, b"user is not in the sudoers file\nprivate-fixture-marker", "sudo policy"),
            (1, b"sudo: sorry, you must have a tty\nprivate-fixture-marker", "interactive terminal"),
            (127, b"sh: docker: command not found\nprivate-fixture-marker", "executable not found"),
            (1, b"permission denied while trying to connect to the Docker daemon socket\nprivate-fixture-marker", "permission denied"),
            (1, b"Cannot connect to the Docker daemon. Is the docker daemon running?\nprivate-fixture-marker", "cannot reach"),
            (2, b"private-fixture-marker", "command failed"),
        ];
        for (status, output, expected) in cases {
            let error = docker_failure(*status, output);
            assert!(error.contains(expected), "{error}");
            assert!(!error.contains("private-fixture-marker"));
        }
    }
}
