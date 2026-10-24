//! Optional real Restic contract test; uses only this test's temporary files.
use containersafe::restic::{RepositoryConfig, RepositorySecret, ResticEngine, ResticErrorKind};
use std::{fs, path::PathBuf};
use uuid::Uuid;

#[test]
#[ignore = "requires CONTAINERSAFE_RESTIC pointing to a real Restic executable"]
fn real_repository_password_backup_restore_and_guards() {
    let executable = std::env::var_os("CONTAINERSAFE_RESTIC")
        .expect("Set CONTAINERSAFE_RESTIC to the Restic executable");
    let directory = tempfile::tempdir().unwrap();
    let reference = Uuid::new_v4();
    let repo = RepositoryConfig {
        path: directory.path().join("repo"),
        secret_reference: reference,
    };
    let secret = RepositorySecret::new(reference, Uuid::new_v4().to_string()).unwrap();
    let wrong = RepositorySecret::new(reference, Uuid::new_v4().to_string()).unwrap();
    let engine = ResticEngine::new(PathBuf::from(executable)).unwrap();
    engine.init(&repo, &secret).unwrap();
    assert!(engine.snapshots(&repo, &secret).unwrap().is_empty());
    assert_eq!(
        engine.snapshots(&repo, &wrong).unwrap_err().kind,
        ResticErrorKind::WrongPassword
    );
    assert_eq!(
        engine.init(&repo, &secret).unwrap_err().kind,
        ResticErrorKind::RepositoryExists
    );

    let source = directory.path().join("source data; literal");
    fs::create_dir(&source).unwrap();
    let bytes = b"original\0bytes\r\n";
    fs::write(source.join("fixture.bin"), bytes).unwrap();
    let backup = engine.backup(&repo, &secret, &[source]).unwrap();
    assert_eq!(backup.total_files_processed, 1);
    let snapshots = engine.snapshots(&repo, &secret).unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].id, backup.snapshot_id);
    assert!(engine.check(&repo, &secret, true).unwrap().all_data_read);

    let mut subtree = snapshots[0].paths[0].clone();
    if cfg!(windows) {
        subtree = format!(
            "/{}",
            subtree
                .replace('\\', "/")
                .replace(':', "")
                .trim_start_matches('/')
        );
    }
    let target = directory.path().join("restored");
    let restored = engine
        .restore(&repo, &secret, &backup.snapshot_id, Some(&subtree), &target)
        .unwrap();
    assert!(restored.content_verified);
    assert_eq!(restored.files_restored, 1);
    assert_eq!(restored.bytes_restored, bytes.len() as u64);
    assert_eq!(fs::read(target.join("fixture.bin")).unwrap(), bytes);
    assert_eq!(
        engine
            .restore(&repo, &secret, &backup.snapshot_id, Some(&subtree), &target)
            .unwrap_err()
            .kind,
        ResticErrorKind::UnsafeRestore
    );
    assert_eq!(fs::read(target.join("fixture.bin")).unwrap(), bytes);
}
