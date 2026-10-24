use super::{
    credentials::Secret,
    model::{Authentication, DockerAccess, Server},
};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
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

pub struct Connection {
    server: Server,
    session: ssh2::Session,
    docker_prefix: &'static str,
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
        server.validate()?;
        let timeout = Duration::from_secs(server.timeout_seconds as u64);
        // DNS resolution uses the OS resolver. The worker remains cancellable by revision.
        let address = (server.host.as_str(), server.port)
            .to_socket_addrs()
            .map_err(|_| "SSH: hostname resolution failed".to_string())?
            .next()
            .ok_or("SSH: no address found".to_string())?;
        let stream = TcpStream::connect_timeout(&address, timeout)
            .map_err(|e| format!("SSH: connection failed or timed out ({})", e.kind()))?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|_| "SSH: cannot set timeout".to_string())?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|_| "SSH: cannot set timeout".to_string())?;
        let mut session =
            ssh2::Session::new().map_err(|_| "SSH: initialization failed".to_string())?;
        session.set_timeout(server.timeout_seconds * 1000);
        session.set_tcp_stream(stream);
        session
            .handshake()
            .map_err(|e| format!("SSH: handshake failed or timed out ({e})"))?;
        let (key, kind) = session
            .host_key()
            .ok_or("SSH: missing host identity".to_string())?;
        let identity = HostIdentity {
            endpoint: format!("[{}]:{}", server.host.to_ascii_lowercase(), server.port),
            key_type: format!("{kind:?}"),
            fingerprint: format!(
                "SHA256:{}",
                base64::engine::general_purpose::STANDARD_NO_PAD.encode(Sha256::digest(key))
            ),
        };
        verify_host(&identity, hosts.get(&identity.endpoint).map(String::as_str))?;
        // Host identity is checked BEFORE sending any credentials.
        let auth = match server.authentication {
            Authentication::Password => {
                if secret.0.is_empty() {
                    return Err(Failure::Message("Authentication: password required".into()));
                }
                session.userauth_password(&server.username, &secret.0)
            }
            Authentication::OpenSsh => session.userauth_pubkey_file(
                &server.username,
                None,
                &server.key_path,
                (!secret.0.is_empty()).then_some(secret.0.as_str()),
            ),
            Authentication::Putty => {
                return Err(Failure::Message(
                    "Use the documented encrypted PuTTYgen export integration".into(),
                ))
            }
        };
        auth.map_err(|_| "SSH: Connected\nHost identity: Verified\nAuthentication: Failed (check credentials, key format/passphrase and server authentication policy)".to_string())?;
        let mut connection = Self {
            server: server.clone(),
            session,
            docker_prefix: "docker",
        };
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
        let started = Instant::now();
        let mut channel = self
            .session
            .channel_session()
            .map_err(|_| "SSH: channel failed or timed out")?;
        channel
            .handle_extended_data(ssh2::ExtendedData::Merge)
            .map_err(|_| "SSH: output setup failed")?;
        channel
            .exec(&format!("{} {}", self.docker_prefix, arguments))
            .map_err(|_| "Docker: command failed or timed out")?;
        let mut output = Vec::new();
        // Bound remote output. Errors never echo remote output or authentication data.
        let mut buffer = [0_u8; 8192];
        loop {
            if started.elapsed() > Duration::from_secs(self.server.timeout_seconds as u64) {
                return Err("Docker: command timed out".into());
            }
            let count = channel
                .read(&mut buffer)
                .map_err(|_| "Docker: output failed or timed out")?;
            if count == 0 {
                break;
            }
            output.extend_from_slice(&buffer[..count]);
            if output.len() > 32 * 1024 * 1024 {
                return Err("Docker: output limit exceeded".into());
            }
        }
        channel
            .wait_close()
            .map_err(|_| "SSH: close failed or timed out")?;
        let status = channel
            .exit_status()
            .map_err(|_| "Docker: missing exit status")?;
        if status != 0 {
            return Err(docker_failure(status, &output));
        }
        String::from_utf8(output).map_err(|_| "Docker: invalid UTF-8 output".into())
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
