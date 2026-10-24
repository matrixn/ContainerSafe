use containersafe::servers::{
    connection::{Connection, ConnectionStage, Failure},
    credentials::Secret,
    model::{Authentication, DockerAccess, Server},
};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
};

struct Fixture {
    child: Child,
    _directory: tempfile::TempDir,
    server: Server,
    hosts: BTreeMap<String, String>,
    key: std::path::PathBuf,
    key_files: BTreeMap<String, std::path::PathBuf>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Fixture {
    fn new() -> Self {
        Self::with_host_key("rsa")
    }
    fn with_host_key(host_key: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut command = Command::new(
            std::env::var("CONTAINERSAFE_TEST_PYTHON").unwrap_or_else(|_| "python".into()),
        );
        command
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/ssh_fixture.py"))
            .arg(directory.path())
            .arg(host_key)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .expect("Install Python and paramiko for the optional SSH integration tests");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&line).expect("SSH fixture must print its endpoint");
        let port = value["port"].as_u64().unwrap() as u16;
        let server = Server {
            name: "Fixture".into(),
            host: "127.0.0.1".into(),
            port,
            username: "fixture".into(),
            ..Server::default()
        };
        let hosts = BTreeMap::from([(
            format!("[127.0.0.1]:{port}"),
            value["fingerprint"].as_str().unwrap().into(),
        )]);
        let key = value["key_path"].as_str().unwrap().into();
        let key_files = serde_json::from_value(value["key_files"].clone()).unwrap();
        Self {
            child,
            _directory: directory,
            server,
            hosts,
            key,
            key_files,
        }
    }
}

#[test]
#[ignore = "Requires Python paramiko. Loopback only; no production server."]
fn real_password_key_authentication_host_identity_and_docker_checks() {
    let mut f = Fixture::new();
    let password = Secret("fixture-password".into());
    let stages = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = stages.clone();
    let initial =
        Connection::open_with_progress(&f.server, &password, &BTreeMap::new(), move |stage| {
            observed.lock().unwrap().push(stage)
        });
    assert_eq!(
        *stages.lock().unwrap(),
        [ConnectionStage::Connecting, ConnectionStage::VerifyingHost]
    );
    assert!(
        matches!(&initial, Err(Failure::UnknownHost(_))),
        "Unexpected SSH result: {:?}\n{}",
        initial.err(),
        std::fs::read_to_string(f._directory.path().join("ssh.log")).unwrap_or_default()
    );
    let changed = BTreeMap::from([(
        format!("[127.0.0.1]:{}", f.server.port),
        "SHA256:wrong".into(),
    )]);
    assert!(matches!(
        Connection::open(&f.server, &password, &changed),
        Err(Failure::ChangedHost(_))
    ));
    assert!(
        !f._directory.path().join("auth-events.log").exists(),
        "Credentials must not be sent before host approval"
    );
    assert!(
        matches!(Connection::open(&f.server,&Secret("wrong".into()),&f.hosts),Err(Failure::Message(m)) if m.contains("Authentication: Failed"))
    );
    stages.lock().unwrap().clear();
    let observed = stages.clone();
    let connection = Connection::open_with_progress(&f.server, &password, &f.hosts, move |stage| {
        observed.lock().unwrap().push(stage)
    })
    .unwrap_or_else(|_| panic!("Password SSH should succeed"));
    assert_eq!(
        *stages.lock().unwrap(),
        [
            ConnectionStage::Connecting,
            ConnectionStage::VerifyingHost,
            ConnectionStage::Authenticating,
            ConnectionStage::CheckingDocker
        ]
    );
    assert_eq!(containersafe::discover(&connection).unwrap().len(), 1);
    f.server.username = "denied".into();
    let denied = Connection::open(&f.server, &password, &f.hosts)
        .err()
        .unwrap();
    assert!(matches!(denied, Failure::Message(m) if
        m.contains("permission denied") && m.contains("sudo requires a password")
        && !m.contains("private-fixture-marker")));

    // Simulate a NAS with no docker in PATH and a package-specific sudo rule.
    f.server.username = "synology".into();
    let connection = Connection::open(&f.server, &password, &f.hosts)
        .unwrap_or_else(|_| panic!("Synology package path with sudo should succeed"));
    assert_eq!(containersafe::discover(&connection).unwrap().len(), 1);
    f.server.docker_access = DockerAccess::Direct;
    assert!(
        matches!(Connection::open(&f.server, &password, &f.hosts), Err(Failure::Message(m)) if m.contains("executable not found"))
    );
    f.server.docker_access = DockerAccess::Sudo;
    assert!(Connection::open(&f.server, &password, &f.hosts).is_ok());
    f.server.docker_access = DockerAccess::Automatic;
    f.server.username = "fixture".into();
    f.server.authentication = Authentication::OpenSsh;
    for (format, passphrase) in [("rsa_pem", ""), ("rsa_pem_encrypted", "fixture-passphrase")] {
        f.server.key_path = f.key_files[format].clone();
        let connection = Connection::open(&f.server, &Secret(passphrase.into()), &f.hosts)
            .unwrap_or_else(|error| panic!("{format} SSH should succeed: {error:?}"));
        assert_eq!(containersafe::discover(&connection).unwrap().len(), 1);
    }
    f.server.key_path = f.key_files["rsa_pem_encrypted"].clone();
    assert!(Connection::open(&f.server, &Secret("wrong-passphrase".into()), &f.hosts).is_err());

    for (format, path) in std::iter::once(("ed25519_openssh", f.key.clone())).chain(
        [
            "ed25519_openssh_plain",
            "rsa_openssh_encrypted",
            "rsa_pem_aes128_encrypted",
            "rsa_pkcs8_encrypted",
            "ecdsa_pem_encrypted",
            "ecdsa_pem_aes128_encrypted",
            "ecdsa_openssh_encrypted",
            "ecdsa_pkcs8_encrypted",
        ]
        .into_iter()
        .map(|name| (name, f.key_files[name].clone())),
    ) {
        f.server.key_path = path;
        let before = std::fs::read(&f.server.key_path).unwrap();
        let passphrase = if format.ends_with("plain") {
            ""
        } else {
            "fixture-passphrase"
        };
        let result = Connection::open(&f.server, &Secret(passphrase.into()), &f.hosts);
        let connection = result.unwrap_or_else(|error| panic!("{format}: {error:?}"));
        assert_eq!(containersafe::discover(&connection).unwrap().len(), 1);
        assert_eq!(std::fs::read(&f.server.key_path).unwrap(), before);
        if !passphrase.is_empty() {
            assert!(
                Connection::open(&f.server, &Secret("wrong-passphrase".into()), &f.hosts).is_err()
            );
        }
    }

    f.server.authentication = Authentication::Putty;
    f.server.key_path = f.key.clone();
    assert!(
        matches!(Connection::open(&f.server, &Secret::default(), &f.hosts),
        Err(Failure::Message(m)) if m.contains("PPK integration"))
    );
}

#[test]
#[ignore = "Requires Python paramiko. Loopback only."]
fn modern_host_keys_and_command_failures() {
    for kind in ["ed25519", "ecdsa"] {
        let mut f = Fixture::with_host_key(kind);
        let password = Secret("fixture-password".into());
        assert!(matches!(
            Connection::open(&f.server, &password, &BTreeMap::new()),
            Err(Failure::UnknownHost(_))
        ));
        let changed = BTreeMap::from([(
            format!("[127.0.0.1]:{}", f.server.port),
            "SHA256:wrong".into(),
        )]);
        assert!(matches!(
            Connection::open(&f.server, &password, &changed),
            Err(Failure::ChangedHost(_))
        ));
        assert!(!f._directory.path().join("auth-events.log").exists());
        f.server.timeout_seconds = 1;
        let connection = Connection::open(&f.server, &password, &f.hosts)
            .unwrap_or_else(|error| panic!("{kind}: {error:?}"));
        assert_eq!(containersafe::discover(&connection).unwrap().len(), 1);
        assert!(connection
            .docker("fixture-no-status")
            .unwrap_err()
            .contains("missing exit status"));
        let started = std::time::Instant::now();
        assert!(connection
            .docker("fixture-hang")
            .unwrap_err()
            .contains("timed out"));
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
        assert_eq!(containersafe::discover(&connection).unwrap().len(), 1);
        f.server.timeout_seconds = 10;
        let connection = Connection::open(&f.server, &password, &f.hosts)
            .unwrap_or_else(|error| panic!("{kind}: {error:?}"));
        assert!(connection
            .docker("fixture-large")
            .unwrap_err()
            .contains("output limit exceeded"));
    }
}
