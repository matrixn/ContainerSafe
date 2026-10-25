//! Real encrypted repositories through disposable Linux SSH. No NAS access.
use containersafe::{
    agent::Action,
    backup::{Destination, DestinationKind},
    direct_backup,
    servers::{
        connection::{Connection, Failure},
        credentials::Secret,
        model::Server,
    },
    transfer::*,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdout, Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;
// WSL localhost forwarding initializes asynchronously; avoid racing several
// disposable distro launchers. Restic still exercises concurrent SSH channels.
static FIXTURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Fixture {
    child: Child,
    output: BufReader<ChildStdout>,
    info: Value,
    server: Server,
    hosts: BTreeMap<String, String>,
    _serial: std::sync::MutexGuard<'static, ()>,
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
impl Fixture {
    fn new(deny: bool) -> Self {
        let serial = FIXTURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let manifest = env!("CARGO_MANIFEST_DIR").replace('\\', "/");
        let mut command = if cfg!(windows) {
            let linux = format!(
                "/mnt/{}/{}",
                manifest[..1].to_ascii_lowercase(),
                &manifest[3..]
            );
            let mut command = Command::new("wsl");
            command.args(["-d", "Ubuntu-22.04", "--", "bash", "-lc"]);
            command.arg(format!(
                "exec \"$HOME/.cache/containersafe-transfer-fixture/bin/python\" {} --restic {} --agent {} --windows-loopback-relay {}",
                quote(&format!("{linux}/scripts/pc_transfer_fixture.py")),
                quote(&format!("{linux}/target/linux-agent/restic")),
                quote(&format!("{linux}/target/linux-agent/containersafe-agent")),
                if deny { "--deny-forwarding" } else { "" }
            ));
            command
        } else {
            let mut command = Command::new(
                std::env::var("CONTAINERSAFE_FIXTURE_PYTHON").unwrap_or_else(|_| "python3".into()),
            );
            command.args([
                format!("{manifest}/scripts/pc_transfer_fixture.py"),
                "--restic".into(),
                format!("{manifest}/target/linux-agent/restic"),
            ]);
            if deny {
                command.arg("--deny-forwarding");
            }
            command
        };
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let info: Value = serde_json::from_str(&line)
            .expect("Fixture must print endpoint; install its isolated paramiko venv first");
        let server = Server {
            name: "Disposable transfer fixture".into(),
            host: "127.0.0.1".into(),
            port: info["port"].as_u64().unwrap() as u16,
            username: "fixture".into(),
            timeout_seconds: 30,
            ..Server::default()
        };
        let hosts = BTreeMap::from([(
            format!("[127.0.0.1]:{}", server.port),
            info["fingerprint"].as_str().unwrap().into(),
        )]);
        Self {
            child,
            output,
            info,
            server,
            hosts,
            _serial: serial,
        }
    }
    fn connection(&self) -> Connection {
        // The WSL port relay can lag behind the fixture's listen()/stdout.
        // Retry only transport startup, always against the same pinned host key.
        for attempt in 0..3 {
            match Connection::open(
                &self.server,
                &Secret("fixture-password".into()),
                &self.hosts,
            ) {
                Ok(connection) => return connection,
                Err(Failure::Message(message))
                    if attempt < 2
                        && message.starts_with("SSH: connection or handshake failed") =>
                {
                    std::thread::sleep(Duration::from_millis(500))
                }
                Err(error) => panic!("Fixture connection failed: {error:?}"),
            }
        }
        unreachable!()
    }
    fn field(&self, key: &str) -> &str {
        self.info[key].as_str().unwrap()
    }
    fn sources(&self) -> Vec<PathBuf> {
        vec![self.field("source").into()]
    }
    fn destination(&self) -> Destination {
        Destination {
            kind: DestinationKind::ServerFolder,
            location: self.field("repository").into(),
        }
    }
    fn call(&self, c: &Connection, action: Action) -> Value {
        direct_backup::execute(
            c,
            self.field("restic"),
            self.field("password_file"),
            self.field("restores"),
            &self.destination(),
            &self.sources(),
            action,
        )
        .unwrap()
    }
    fn backup(&self, c: &Connection) -> String {
        self.call(
            c,
            Action::BackupSelected {
                expected_sources: self.sources(),
                expected_repository: self.field("repository").into(),
            },
        )["snapshot_id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn export(&self, c: &Connection, snapshot: &str) -> SourceIdentity {
        artifact::create(
            c,
            &artifact::ExportRequest {
                plan_id: Uuid::new_v4(),
                restic: self.field("restic"),
                password_file: self.field("password_file"),
                restore_root: self.field("restores"),
                destination: &self.destination(),
                approved_sources: &self.sources(),
                snapshot_id: snapshot,
            },
        )
        .unwrap()
    }
    fn change(&mut self) -> String {
        let input = self.child.stdin.as_mut().unwrap();
        writeln!(input, "change").unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        serde_json::from_str::<Value>(&line).unwrap()["file_sha256"]
            .as_str()
            .unwrap()
            .into()
    }
    fn fault(&mut self, mode: &str) {
        let input = self.child.stdin.as_mut().unwrap();
        writeln!(input, "fault {mode}").unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["mode"], mode);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut input) = self.child.stdin.take() {
            let _ = writeln!(input, "quit");
            let _ = input.flush();
        }
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn job(source: SourceIdentity, path: &Path, kind: TransferKind) -> TransferReceipt {
    TransferReceipt::new(source, path.into(), kind).unwrap()
}

fn rest_server() -> PathBuf {
    std::env::var_os("CONTAINERSAFE_REST_SERVER").map(PathBuf::from).unwrap_or_else(||PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"),"/target/pc-transfer-tools/rest-server/rest-server_0.14.0_windows_amd64/rest-server.exe")))
}
#[test]
#[ignore = "Requires isolated SSH fixture, official rest-server and real Restic; never accesses NAS"]
fn reverse_tunnel_direct_and_agent_create_two_verified_windows_snapshots() {
    for use_agent in [false, true] {
        let mut fixture = Fixture::new(false);
        let connection = fixture.connection();
        let local = tempfile::tempdir().unwrap();
        let repository_id = Uuid::parse_str(fixture.field("pc_repository")).unwrap();
        let session = reverse_tunnel::ReverseSession::open(
            &connection,
            &rest_server(),
            local.path(),
            repository_id,
            CancellationToken::default(),
            Arc::new(|_| {}),
        )
        .unwrap_or_else(|e| panic!("Reverse setup failed: {e}"));
        let destination = Destination {
            kind: DestinationKind::LocalPcRepository,
            location: local.path().to_string_lossy().into_owned(),
        };
        let sources = fixture.sources();
        let restic = fixture.field("restic").to_owned();
        let password = fixture.field("password_file").to_owned();
        let restores = fixture.field("restores").to_owned();
        let endpoint = containersafe::backup::AgentEndpoint {
            executable: fixture.field("agent").into(),
            config: fixture.field("agent_config").into(),
            job_id: Uuid::parse_str(fixture.field("job_id")).unwrap(),
        };
        let call = |action| {
            pc_backup::execute(
                &connection,
                direct_backup::DirectRequest {
                    executable: &restic,
                    password_file: &password,
                    restore_root: &restores,
                    destination: &destination,
                    sources: &sources,
                },
                use_agent.then_some(&endpoint),
                action,
                &session,
            )
        };
        call(Action::Init {}).unwrap();
        let backup = || Action::BackupSelected {
            expected_sources: sources.clone(),
            expected_repository: destination.location.clone().into(),
        };
        let first = call(backup()).unwrap()["snapshot_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let changed = fixture.change();
        let second = call(backup()).unwrap()["snapshot_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(first, second);
        let snapshots = call(Action::Snapshots {}).unwrap();
        assert_eq!(snapshots.as_array().unwrap().len(), 2);
        let repository = session.repository_path();
        drop(session);
        assert!(
            connection
                .docker("version --format '{{.Server.Version}}'")
                .is_ok(),
            "Forward teardown must preserve SSH connection"
        );
        let reference = Uuid::new_v4();
        let config = containersafe::restic::RepositoryConfig {
            path: repository,
            secret_reference: reference,
        };
        let secret = containersafe::restic::RepositorySecret::new(
            reference,
            "fixture-repository-password".into(),
        )
        .unwrap();
        let engine = containersafe::restic::ResticEngine::new(
            std::env::var("CONTAINERSAFE_RESTIC")
                .unwrap_or_else(|_| "C:/Users/radut/bin/restic.exe".into()),
        )
        .unwrap();
        assert!(engine.check(&config, &secret, true).unwrap().all_data_read);
        for (id, expected, name) in [
            (first, fixture.field("file_sha256").to_owned(), "first"),
            (second, changed, "second"),
        ] {
            let target = local.path().join(name);
            engine
                .restore(&config, &secret, &id, None, &target)
                .unwrap();
            let recovered = target
                .join(fixture.field("source").trim_start_matches('/'))
                .join("fixture.bin");
            assert_eq!(
                format!("{:x}", Sha256::digest(std::fs::read(recovered).unwrap())),
                expected
            );
        }
    }
}

#[test]
#[ignore = "Requires isolated SSH fixture and official rest-server; never accesses NAS"]
fn reverse_forwarding_denied_is_explicit_and_backend_context_is_exclusive() {
    let fixture = Fixture::new(true);
    let connection = fixture.connection();
    let local = tempfile::tempdir().unwrap();
    assert!(matches!(
        reverse_tunnel::ReverseSession::open(
            &connection,
            &rest_server(),
            local.path(),
            Uuid::new_v4(),
            CancellationToken::default(),
            Arc::new(|_| {})
        ),
        Err(TransferError::ForwardingDenied)
    ));
    let repository_id = Uuid::new_v4();
    let context = local_backend::RepositoryContext::open(
        local.path(),
        repository_id,
        connection.data_identity().unwrap(),
    )
    .unwrap();
    assert!(matches!(
        local_backend::RepositoryContext::open(
            local.path(),
            repository_id,
            connection.data_identity().unwrap()
        ),
        Err(TransferError::Collision)
    ));
    let backend = local_backend::LocalBackend::start(&rest_server(), context).unwrap();
    assert_eq!(backend.status("/repository/config", false).unwrap(), 401);
    assert_ne!(backend.status("/other/config", true).unwrap(), 200);
    assert_ne!(
        backend
            .status("/repository/../server-binding.json", true)
            .unwrap(),
        200
    );
}

#[test]
#[ignore = "Requires loopback SSH fixture, real Restic and official rest-server"]
fn reverse_cancel_mid_upload_keeps_repository_checkable_and_allows_retry() {
    let mut fixture = Fixture::new(false);
    writeln!(fixture.child.stdin.as_mut().unwrap(), "large").unwrap();
    fixture.child.stdin.as_mut().unwrap().flush().unwrap();
    let mut line = String::new();
    fixture.output.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["control"],
        "large"
    );
    let connection = fixture.connection();
    let local = tempfile::tempdir().unwrap();
    let id = Uuid::parse_str(fixture.field("pc_repository")).unwrap();
    let destination = Destination {
        kind: DestinationKind::LocalPcRepository,
        location: local.path().display().to_string(),
    };
    let sources = fixture.sources();
    let token = CancellationToken::default();
    let cancel = token.clone();
    let session = reverse_tunnel::ReverseSession::open(
        &connection,
        &rest_server(),
        local.path(),
        id,
        token,
        Arc::new(move |p| {
            if p.bytes > 524288 && p.state == TransferState::Transferring {
                cancel.cancel();
            }
        }),
    )
    .unwrap();
    let call = |action, session: &reverse_tunnel::ReverseSession<'_>| {
        pc_backup::execute(
            &connection,
            direct_backup::DirectRequest {
                executable: fixture.field("restic"),
                password_file: fixture.field("password_file"),
                restore_root: fixture.field("restores"),
                destination: &destination,
                sources: &sources,
            },
            None,
            action,
            session,
        )
    };
    call(Action::Init {}, &session).unwrap();
    let backup = || Action::BackupSelected {
        expected_sources: sources.clone(),
        expected_repository: destination.location.clone().into(),
    };
    assert!(
        call(backup(), &session).is_err(),
        "Cancelled upload cannot report success"
    );
    drop(session);
    // The fixture producer observes the closed tunnel; give Restic time to release its own lock.
    let session = reverse_tunnel::ReverseSession::open(
        &connection,
        &rest_server(),
        local.path(),
        id,
        CancellationToken::default(),
        Arc::new(|_| {}),
    )
    .unwrap();
    let mut checked = false;
    for _ in 0..20 {
        if call(Action::Check { read_data: true }, &session).is_ok() {
            checked = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(checked, "No automatic unlock is allowed after interruption");
    assert!(call(backup(), &session).is_ok());
}

#[test]
#[ignore = "Requires isolated WSL/Linux paramiko fixture and real Restic; never accesses NAS"]
fn sftp_exports_real_encrypted_snapshot_resumes_and_rejects_bad_identity() {
    let mut fixture = Fixture::new(false);
    let connection = fixture.connection();
    let mut wrong = fixture.hosts.clone();
    wrong.values_mut().for_each(|v| *v = "SHA256:wrong".into());
    assert!(matches!(
        Connection::open(&fixture.server, &Secret("fixture-password".into()), &wrong),
        Err(Failure::ChangedHost(_))
    ));
    fixture.call(&connection, Action::Init {});
    let snapshot = fixture.backup(&connection);
    fixture.change();
    let second = fixture.backup(&connection);
    assert_ne!(snapshot, second);
    let artifact = fixture.export(&connection, &snapshot);
    let local = tempfile::tempdir().unwrap();
    let receipt = job(artifact.clone(), local.path(), TransferKind::SftpPull);
    let path = local.path().join("receipt.json");
    let cancellation = CancellationToken::default();
    let cancel = cancellation.clone();
    let sink = Arc::new(move |p: TransferProgress| {
        if p.bytes > 262144 {
            cancel.cancel();
        }
    });
    let coordinator = Coordinator::open(receipt.clone(), &path, sink, cancellation).unwrap();
    assert_eq!(
        sftp_pull::download(&connection, coordinator).unwrap_err(),
        TransferError::Cancelled
    );
    let partial: TransferReceipt = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(partial.bytes > 0 && partial.bytes < artifact.size);
    let coordinator = Coordinator::open(
        receipt.clone(),
        &path,
        Arc::new(|_| {}),
        CancellationToken::default(),
    )
    .unwrap();
    let done = sftp_pull::download(&connection, coordinator).unwrap();
    assert!(done.sha256_verified && done.state == TransferState::Completed);
    let bytes = std::fs::read(
        local
            .path()
            .join(format!("{}.csbackup.tar", artifact.artifact_id)),
    )
    .unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), artifact.sha256);
    assert!(!bytes
        .windows(b"ContainerSafe PRIVATE fixture".len())
        .any(|w| w == b"ContainerSafe PRIVATE fixture"));
    let extracted =
        containersafe::transfer::local_artifact::extract(&done, &CancellationToken::default())
            .unwrap();
    let secret_id = Uuid::new_v4();
    let secret = containersafe::restic::RepositorySecret::new(
        secret_id,
        "fixture-repository-password".into(),
    )
    .unwrap();
    let repo = containersafe::restic::RepositoryConfig {
        path: extracted.repository(),
        secret_reference: secret_id,
    };
    let engine = containersafe::restic::ResticEngine::new(
        std::env::var("CONTAINERSAFE_RESTIC")
            .unwrap_or_else(|_| "C:/Users/radut/bin/restic.exe".into()),
    )
    .unwrap();
    assert!(engine.check(&repo, &secret, true).unwrap().all_data_read);
    let snapshots = engine.snapshots(&repo, &secret).unwrap();
    assert_eq!(snapshots.len(), 1);
    let restored = local.path().join("restored");
    engine
        .restore(&repo, &secret, &snapshots[0].id, None, &restored)
        .unwrap();
    let recovered = restored
        .join(fixture.field("source").trim_start_matches('/'))
        .join("fixture.bin");
    assert_eq!(
        format!("{:x}", Sha256::digest(std::fs::read(recovered).unwrap())),
        fixture.field("file_sha256")
    );
    // Supported repository copy keeps both exported snapshots, not a filesystem merge.
    let mirror = containersafe::restic::RepositoryConfig {
        path: local.path().join("mirror"),
        secret_reference: secret_id,
    };
    engine.init(&mirror, &secret).unwrap();
    engine
        .copy_snapshots(&repo, &secret, &mirror, &secret, &[snapshots[0].id.clone()])
        .unwrap();
    let second_artifact = fixture.export(&connection, &second);
    let second_folder = tempfile::tempdir().unwrap();
    let coordinator = Coordinator::open(
        job(
            second_artifact.clone(),
            second_folder.path(),
            TransferKind::SftpPull,
        ),
        &second_folder.path().join("receipt.json"),
        Arc::new(|_| {}),
        CancellationToken::default(),
    )
    .unwrap();
    let second_receipt = sftp_pull::download(&connection, coordinator).unwrap();
    let second_extracted = containersafe::transfer::local_artifact::extract(
        &second_receipt,
        &CancellationToken::default(),
    )
    .unwrap();
    let second_repo = containersafe::restic::RepositoryConfig {
        path: second_extracted.repository(),
        secret_reference: secret_id,
    };
    let second_snapshots = engine.snapshots(&second_repo, &secret).unwrap();
    engine
        .copy_snapshots(
            &second_repo,
            &secret,
            &mirror,
            &secret,
            &[second_snapshots[0].id.clone()],
        )
        .unwrap();
    assert!(engine.check(&mirror, &secret, true).unwrap().all_data_read);
    assert_eq!(engine.snapshots(&mirror, &secret).unwrap().len(), 2);
    let streamed = tempfile::tempdir().unwrap();
    let receipt = job(artifact.clone(), streamed.path(), TransferKind::SshStream);
    let coordinator = Coordinator::open(
        receipt,
        &streamed.path().join("receipt.json"),
        Arc::new(|_| {}),
        CancellationToken::default(),
    )
    .unwrap();
    assert!(
        ssh_stream::receive(&connection, coordinator)
            .unwrap()
            .sha256_verified
    );
    let stream_bytes = std::fs::read(
        streamed
            .path()
            .join(format!("{}.csbackup.tar", artifact.artifact_id)),
    )
    .unwrap();
    assert_eq!(bytes, stream_bytes);
    let cancelled = tempfile::tempdir().unwrap();
    let token = CancellationToken::default();
    let cancel = token.clone();
    let coordinator = Coordinator::open(
        job(artifact.clone(), cancelled.path(), TransferKind::SshStream),
        &cancelled.path().join("r.json"),
        Arc::new(move |p: TransferProgress| {
            if p.bytes > 262144 {
                cancel.cancel();
            }
        }),
        token,
    )
    .unwrap();
    assert_eq!(
        ssh_stream::receive(&connection, coordinator).unwrap_err(),
        TransferError::Cancelled
    );
    for (mode, expected) in [
        ("marker", TransferError::Incomplete),
        ("digest", TransferError::DigestMismatch),
        ("exit", TransferError::RemoteFailed),
        ("disconnect", TransferError::RemoteFailed),
    ] {
        fixture.fault(mode);
        let local = tempfile::tempdir().unwrap();
        let coordinator = Coordinator::open(
            job(artifact.clone(), local.path(), TransferKind::SshStream),
            &local.path().join("r.json"),
            Arc::new(|_| {}),
            CancellationToken::default(),
        )
        .unwrap();
        assert_eq!(
            ssh_stream::receive(&connection, coordinator).unwrap_err(),
            expected,
            "fault {mode}"
        );
        assert!(!local
            .path()
            .join(format!("{}.csbackup.tar", artifact.artifact_id))
            .exists());
    }
    for (mode, expected) in [
        ("missing", TransferError::SourceChanged),
        ("modified", TransferError::SourceChanged),
        ("symlink", TransferError::SourceChanged),
        ("truncated", TransferError::Incomplete),
    ] {
        fixture.fault(mode);
        let local = tempfile::tempdir().unwrap();
        let coordinator = Coordinator::open(
            job(artifact.clone(), local.path(), TransferKind::SftpPull),
            &local.path().join("r.json"),
            Arc::new(|_| {}),
            CancellationToken::default(),
        )
        .unwrap();
        assert_eq!(
            sftp_pull::download(&connection, coordinator).unwrap_err(),
            expected,
            "fault {mode}"
        );
        assert!(!local
            .path()
            .join(format!("{}.csbackup.tar", artifact.artifact_id))
            .exists());
    }
    fixture.fault("none");
    let other = tempfile::tempdir().unwrap();
    let mut wrong = artifact.clone();
    wrong.server_id = Uuid::new_v4();
    let coordinator = Coordinator::open(
        job(wrong, other.path(), TransferKind::SftpPull),
        &other.path().join("r.json"),
        Arc::new(|_| {}),
        CancellationToken::default(),
    )
    .unwrap();
    assert_eq!(
        sftp_pull::download(&connection, coordinator).unwrap_err(),
        TransferError::SourceChanged
    );
}
