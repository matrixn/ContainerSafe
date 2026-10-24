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
}

struct PinnedHost {
    endpoint: String,
    fingerprint: Option<String>,
    rejection: Arc<Mutex<Option<Failure>>>,
    progress: Progress,
}
impl client::Handler for PinnedHost {
    type Error = russh::Error;
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
        let handler = PinnedHost {
            fingerprint: hosts.get(&endpoint).cloned(),
            endpoint,
            rejection: rejection.clone(),
            progress: progress.clone(),
        };
        let config = Arc::new(client::Config::default());
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
