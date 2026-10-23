use super::{
    connection::{verify_host, Connection, Failure, HostIdentity},
    credentials::Secret,
    model::{Authentication, Server, TestGate},
    storage::{self, Profiles},
};
use std::{
    collections::BTreeMap,
    net::TcpListener,
    time::{Duration, Instant},
};

fn server() -> Server {
    Server {
        name: "Test".into(),
        host: "127.0.0.1".into(),
        username: "user".into(),
        ..Server::default()
    }
}

#[test]
fn create_edit_default_remove_and_timestamp_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mut profiles = Profiles::default();
    let mut first = server();
    let second = server();
    profiles.upsert(first.clone());
    assert_eq!(profiles.default, Some(first.id));
    profiles.upsert(second.clone());
    profiles.set_default(second.id).unwrap();
    first.name = "Edited".into();
    first.last_opened = Some(42);
    profiles.upsert(first.clone());
    assert_eq!(profiles.servers.len(), 2);
    assert_eq!(profiles.default, Some(second.id));
    profiles.save(dir.path()).unwrap();
    let loaded = Profiles::load(dir.path()).unwrap();
    assert_eq!(loaded.servers[0], first);
    assert_eq!(loaded.servers[0].last_opened, Some(42));
    profiles.remove(second.id);
    assert_eq!(profiles.default, None);
    assert_eq!(profiles.servers.len(), 1);
    assert!(profiles.set_default(second.id).is_err());
}

#[test]
fn corrupt_profiles_remain_untouched() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("servers.json"), b"{broken").unwrap();
    assert!(Profiles::load(dir.path()).is_err());
    assert_eq!(
        std::fs::read(dir.path().join("servers.json")).unwrap(),
        b"{broken"
    );
}

#[test]
fn first_run_and_no_cross_server_or_legacy_plan_mixing() {
    let dir = tempfile::tempdir().unwrap();
    let p = Profiles::load(dir.path()).unwrap();
    assert!(p.servers.is_empty());
    assert_eq!(p.default, None);
    let a = server();
    let b = server();
    let ap = storage::server_dir(dir.path(), a.id);
    let bp = storage::server_dir(dir.path(), b.id);
    storage::atomic_write(&ap.join("backup-plan.json"), b"{}").unwrap();
    assert!(!bp.join("backup-plan.json").exists());
    std::fs::write(dir.path().join("backup-selection.json"), b"legacy").unwrap();
    assert!(!ap.join("backup-selection.json").exists());
}

#[test]
fn secrets_are_not_serializable_profile_fields() {
    let mut p = Profiles::default();
    let mut s = server();
    s.remember = true;
    p.upsert(s);
    let json = serde_json::to_string(&p).unwrap();
    assert!(!json.contains("password"));
    assert!(!json.contains("passphrase"));
    assert!(!json.contains("private_key"));
    assert!(!json.contains("secret"));
}

#[test]
fn cached_decisions_require_the_same_verified_identity() {
    let root = tempfile::tempdir().unwrap();
    let mut s = server();
    let mut hosts = BTreeMap::from([(format!("[{}]:{}", s.host, s.port), "SHA256:first".into())]);
    let original = storage::DataIdentity::new(&s, &hosts).unwrap();
    let data = storage::server_dir(root.path(), s.id);
    original.save(&data).unwrap();
    assert!(original.matches(&data));
    hosts.insert(format!("[{}]:{}", s.host, s.port), "SHA256:changed".into());
    assert!(!storage::DataIdentity::new(&s, &hosts)
        .unwrap()
        .matches(&data));
    s.id = uuid::Uuid::new_v4();
    assert!(!storage::DataIdentity::new(&s, &hosts)
        .unwrap()
        .matches(&data));
}

#[test]
#[ignore = "Exercises the real OS credential store with a disposable UUID; no user credentials."]
fn os_credential_roundtrip_and_removal() {
    use super::credentials;
    let id = uuid::Uuid::new_v4();
    let secret = Secret("containersafe-disposable-credential-test".into());
    credentials::write(id, &secret).unwrap();
    let read = credentials::read(id).unwrap();
    credentials::remove(id).unwrap();
    assert!(read.0 == secret.0);
    assert!(credentials::read(id).is_err());
    credentials::remove(id).unwrap();
}

#[test]
fn legacy_import_is_explicit_validated_and_never_partial() {
    let root = tempfile::tempdir().unwrap();
    let legacy = tempfile::tempdir().unwrap();
    let id = uuid::Uuid::new_v4();
    std::fs::write(legacy.path().join("backup-plan.json"), b"{}").unwrap();
    std::fs::write(legacy.path().join("backup-selection.json"), b"broken").unwrap();
    assert!(storage::import_legacy(root.path(), id, legacy.path()).is_err());
    assert!(!storage::server_dir(root.path(), id).exists());
    std::fs::write(legacy.path().join("backup-selection.json"), b"{}").unwrap();
    storage::import_legacy(root.path(), id, legacy.path()).unwrap();
    assert!(storage::import_legacy(root.path(), id, legacy.path()).is_err());
}

#[test]
fn input_and_private_key_validation() {
    let mut s = server();
    assert!(s.validate().is_ok());
    s.port = 0;
    assert!(s.validate().is_err());
    s.port = 22;
    s.host = "host; touch pwned".into();
    assert!(s.validate().is_err());
    s.host = "::1".into();
    assert!(s.validate().is_ok());
    s.username.clear();
    assert!(s.validate().is_err());
    s.username = "user".into();
    s.authentication = Authentication::OpenSsh;
    s.key_path = "id_ed25519.pub".into();
    assert!(s.validate().unwrap_err().contains("private key"));
}

#[test]
fn unknown_host_and_mismatch_are_blocked() {
    let h = HostIdentity {
        endpoint: "host:22".into(),
        key_type: "Ed25519".into(),
        fingerprint: "SHA256:key".into(),
    };
    assert!(matches!(
        verify_host(&h, None),
        Err(Failure::UnknownHost(_))
    ));
    assert!(matches!(
        verify_host(&h, Some("different")),
        Err(Failure::ChangedHost(_))
    ));
    assert!(verify_host(&h, Some(&h.fingerprint)).is_ok());
}

#[test]
fn edits_and_cancellation_reject_late_success_and_double_submit() {
    let mut gate = TestGate::default();
    let revision = gate.revision;
    gate.complete(revision, true);
    assert!(gate.can_save());
    gate.invalidate();
    assert!(!gate.can_save());
    gate.complete(revision, true);
    assert!(!gate.can_save());
    gate.complete(gate.revision, true);
    assert!(gate.can_save());
    gate.invalidate();
    assert!(!gate.can_save());
}

#[test]
fn offline_default_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut s = server();
    s.port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut profiles = Profiles::default();
    profiles.upsert(s.clone());
    profiles.save(dir.path()).unwrap();
    assert!(matches!(
        Connection::open(&s, &Secret("invalid".into()), &BTreeMap::new()),
        Err(Failure::Message(_))
    ));
    assert_eq!(Profiles::load(dir.path()).unwrap().default, Some(s.id));
}

#[test]
fn silent_ssh_peer_times_out_without_production_access() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut s = server();
    s.port = listener.local_addr().unwrap().port();
    s.timeout_seconds = 1;
    let worker = std::thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        std::thread::sleep(Duration::from_secs(2));
    });
    let started = Instant::now();
    assert!(matches!(
        Connection::open(&s, &Secret("not-sent".into()), &BTreeMap::new()),
        Err(Failure::Message(_))
    ));
    assert!(started.elapsed() < Duration::from_secs(5));
    worker.join().unwrap();
}
