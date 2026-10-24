use containersafe::servers::{
    connection::{Connection, Failure},
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
        let directory = tempfile::tempdir().unwrap();
        let mut command = Command::new(
            std::env::var("CONTAINERSAFE_TEST_PYTHON").unwrap_or_else(|_| "python".into()),
        );
        command
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/ssh_fixture.py"))
            .arg(directory.path())
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
    let initial = Connection::open(&f.server, &password, &BTreeMap::new());
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
        matches!(Connection::open(&f.server,&Secret("wrong".into()),&f.hosts),Err(Failure::Message(m)) if m.contains("Authentication: Failed"))
    );
    let connection = Connection::open(&f.server, &password, &f.hosts)
        .unwrap_or_else(|_| panic!("Password SSH should succeed"));
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
            "rsa_openssh_encrypted",
            "rsa_pkcs8_encrypted",
            "ecdsa_pem_encrypted",
        ]
        .into_iter()
        .map(|name| (name, f.key_files[name].clone())),
    ) {
        f.server.key_path = path;
        let result = Connection::open(&f.server, &Secret("fixture-passphrase".into()), &f.hosts);
        if cfg!(windows) {
            assert!(
                matches!(result, Err(Failure::Message(m)) if m.contains("WinCNG")),
                "{format} must report the WinCNG limitation explicitly"
            );
            println!("WinCNG unsupported format verified: {format}");
        } else {
            let connection = result.unwrap_or_else(|error| panic!("{format}: {error:?}"));
            assert_eq!(containersafe::discover(&connection).unwrap().len(), 1);
        }
    }

    f.server.authentication = Authentication::Putty;
    f.server.key_path = f.key.clone();
    assert!(
        matches!(Connection::open(&f.server, &Secret::default(), &f.hosts), Err(Failure::Message(m)) if m.contains("PPK integration"))
    );
}
