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
const DIRECT: [&str; 3] = ["docker", "/usr/bin/docker", "/usr/local/bin/docker"];
const SUDO: [&str; 3] = [
    "sudo -n docker",
    "sudo -n /usr/bin/docker",
    "sudo -n /usr/local/bin/docker",
];

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
        for prefix in candidates {
            connection.docker_prefix = prefix;
            if connection
                .docker("version --format '{{.Server.Version}}'")
                .is_ok()
            {
                return Ok(connection);
            }
        }
        Err(Failure::Message("SSH: Connected\nAuthentication: Successful\nHost identity: Verified\nDocker: Unavailable or permission denied. Check Docker installation, daemon and group access, or select configured passwordless sudo -n.".into()))
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
        if channel
            .exit_status()
            .map_err(|_| "Docker: missing exit status")?
            != 0
        {
            return Err(
                "Docker command failed: unavailable, permission denied, or daemon stopped".into(),
            );
        }
        String::from_utf8(output).map_err(|_| "Docker: invalid UTF-8 output".into())
    }
}
