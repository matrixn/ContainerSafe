use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

pub const SSH_KEY_COMPATIBILITY: &str = if cfg!(windows) {
    "Windows WinCNG supports RSA private keys in traditional PEM format (BEGIN RSA PRIVATE KEY), including encrypted keys. Ed25519, ECDSA, PKCS#8 and modern BEGIN OPENSSH PRIVATE KEY files are unsupported by this build. Existing keys are never converted automatically."
} else {
    "Linux/macOS use libssh2 with OpenSSL: encrypted OpenSSH Ed25519 and compatible RSA/ECDSA keys are supported. Existing keys are never converted automatically."
};

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Authentication {
    Password,
    Putty,
    OpenSsh,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DockerAccess {
    Automatic,
    Direct,
    Sudo,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    pub id: Uuid,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub authentication: Authentication,
    pub key_path: PathBuf,
    pub remember: bool,
    pub docker_access: DockerAccess,
    pub created_at: u64,
    pub last_success: Option<u64>,
    pub last_opened: Option<u64>,
    pub last_status: String,
    pub last_error: Option<String>,
    pub timeout_seconds: u32,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::new(),
            host: String::new(),
            port: 22,
            username: String::new(),
            authentication: Authentication::Password,
            key_path: PathBuf::new(),
            remember: false,
            docker_access: DockerAccess::Automatic,
            created_at: now(),
            last_success: None,
            last_opened: None,
            last_status: "Unknown".into(),
            last_error: None,
            timeout_seconds: 10,
        }
    }
}

impl Server {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() || self.host.is_empty() || self.username.is_empty() {
            return Err("Server name, hostname and username are required".into());
        }
        if self.name.len() > 128 || self.name.chars().any(char::is_control) {
            return Err(
                "Server name must be at most 128 bytes and contain no control characters".into(),
            );
        }
        if self.host.len() > 253
            || !self
                .host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-:_".contains(&b))
        {
            return Err(
                "Enter a hostname or IP address without a scheme, spaces or shell characters"
                    .into(),
            );
        }
        if self.username.chars().any(char::is_control)
            || self.port == 0
            || !(1..=120).contains(&self.timeout_seconds)
        {
            return Err("Invalid username, SSH port (1-65535), or timeout (1-120 seconds)".into());
        }
        if self.authentication != Authentication::Password {
            if self.key_path.extension().is_some_and(|e| e == "pub") {
                return Err("Select a private key, not a .pub file".into());
            }
            let file = std::fs::File::open(&self.key_path)
                .map_err(|_| "Private key is missing or inaccessible")?;
            use std::io::Read;
            let mut header = String::new();
            file.take(128)
                .read_to_string(&mut header)
                .map_err(|_| "Invalid private-key format")?;
            if self.authentication == Authentication::Putty {
                return Err(format!("PPK integration: open the key in PuTTYgen, export an encrypted OpenSSH private key, then select OpenSSH here. ContainerSafe never converts or decrypts key files automatically. {SSH_KEY_COMPATIBILITY}"));
            }
            #[cfg(windows)]
            if !header.starts_with("-----BEGIN RSA PRIVATE KEY-----") {
                return Err(SSH_KEY_COMPATIBILITY.into());
            }
            if !header.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----")
                && !header.starts_with("-----BEGIN RSA PRIVATE KEY-----")
                && !header.starts_with("-----BEGIN EC PRIVATE KEY-----")
                && !header.starts_with("-----BEGIN ENCRYPTED PRIVATE KEY-----")
                && !header.starts_with("-----BEGIN PRIVATE KEY-----")
            {
                return Err("Unrecognized OpenSSH/PEM private key".into());
            }
        }
        Ok(())
    }
}

/// A test result is valid only for the exact form revision that produced it.
#[derive(Default)]
pub struct TestGate {
    pub revision: u64,
    passed: Option<u64>,
}
impl TestGate {
    pub fn invalidate(&mut self) {
        self.revision += 1;
        self.passed = None;
    }
    pub fn complete(&mut self, revision: u64, success: bool) {
        if revision == self.revision {
            self.passed = success.then_some(revision);
        }
    }
    pub fn can_save(&self) -> bool {
        self.passed == Some(self.revision)
    }
}
