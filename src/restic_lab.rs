//! Disposable local demonstration of the shared Restic engine.
use crate::restic::{RepositoryConfig, RepositorySecret, ResticEngine};
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub fn run() -> Result<(), Box<dyn Error>> {
    run_with_executable(
        std::env::var_os("CONTAINERSAFE_RESTIC")
            .map(PathBuf::from)
            .unwrap_or_else(|| "restic".into()),
    )
}

// Only a path within the snapshot, never a host restore destination.
fn subtree(path: &str) -> String {
    if cfg!(windows) {
        let path = path.strip_prefix(r"\\?\").unwrap_or(path);
        format!(
            "/{}",
            path.replace('\\', "/")
                .replace(':', "")
                .trim_start_matches('/')
        )
    } else {
        path.to_owned()
    }
}

pub fn run_with_executable(executable: impl Into<PathBuf>) -> Result<(), Box<dyn Error>> {
    let lab = tempfile::Builder::new()
        .prefix("containersafe-lab-")
        .tempdir()?
        .keep();
    let source = lab.join("source data; literal");
    fs::create_dir(&source)?;
    let original = source.join("containersafe-fixture.txt");
    let first = b"ContainerSafe: backup and restore test.\n";
    let second = b"ContainerSafe: backup and restore test.\nVersion 2.\n";
    fs::write(&original, first)?;

    let reference = Uuid::new_v4();
    let config = RepositoryConfig {
        path: lab.join("repository"),
        secret_reference: reference,
    };
    // Random, session-only credentials for synthetic data. Never printed or saved.
    let secret = RepositorySecret::new(reference, format!("{}{}", Uuid::new_v4(), Uuid::new_v4()))?;
    let engine = ResticEngine::new(executable)?;
    println!(
        "ContainerSafe {} - reusable Restic Engine lab",
        crate::VERSION
    );
    println!("Disposable local files: {}", lab.display());
    println!("No SSH connection or production Docker data is used.");

    println!("[1/7] Initialize encrypted repository");
    engine.init(&config, &secret)?;
    if !engine.snapshots(&config, &secret)?.is_empty() {
        return Err("New repository unexpectedly contains snapshots".into());
    }

    println!("[2/7] Create first snapshot");
    let backup1 = engine.backup(&config, &secret, std::slice::from_ref(&source))?;
    println!("[3/7] Verify all repository data");
    let check1 = engine.check(&config, &secret, true)?;
    let snapshots = engine.snapshots(&config, &secret)?;
    let snapshot1 = snapshots
        .iter()
        .find(|s| s.id == backup1.snapshot_id)
        .ok_or("First snapshot missing")?;
    let source_tree = subtree(
        snapshot1
            .paths
            .first()
            .ok_or("Snapshot source path missing")?,
    );

    println!("[4/7] Restore first snapshot to a new directory and compare bytes");
    let restore1 = engine.restore(
        &config,
        &secret,
        &backup1.snapshot_id,
        Some(&source_tree),
        &lab.join("restored-v1"),
    )?;
    compare(&restore1.target.join("containersafe-fixture.txt"), first)?;

    println!("[5/7] Modify synthetic data and create incremental snapshot");
    fs::write(&original, second)?;
    let backup2 = engine.backup(&config, &secret, std::slice::from_ref(&source))?;
    if backup1.snapshot_id == backup2.snapshot_id {
        return Err("Second snapshot did not get a new identity".into());
    }
    println!("[6/7] Verify repository and restore the second snapshot separately");
    let check2 = engine.check(&config, &secret, true)?;
    let restore2 = engine.restore(
        &config,
        &secret,
        &backup2.snapshot_id,
        Some(&source_tree),
        &lab.join("restored-v2"),
    )?;
    compare(&restore2.target.join("containersafe-fixture.txt"), second)?;
    compare(&restore1.target.join("containersafe-fixture.txt"), first)?;

    println!("[7/7] Restore the older snapshot again after the incremental backup");
    let old = engine.restore(
        &config,
        &secret,
        &backup1.snapshot_id,
        Some(&source_tree),
        &lab.join("restored-old-again"),
    )?;
    compare(&old.target.join("containersafe-fixture.txt"), first)?;
    let snapshots = engine.snapshots(&config, &secret)?;
    if snapshots.len() != 2 {
        return Err("Expected exactly two snapshots".into());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "snapshots": snapshots, "backups": [backup1, backup2],
            "repository_checks": [check1, check2], "restores": [restore1, restore2, old],
            "byte_comparisons_passed": true
        }))?
    );
    println!("[PASS] Two snapshots, full repository checks and three isolated restores verified.");
    println!("Fixture files retained at: {}", lab.display());
    println!("The disposable repository password was not saved; this repository is only for this lab run.");
    Ok(())
}

fn compare(path: &Path, expected: &[u8]) -> Result<(), Box<dyn Error>> {
    if fs::read(path)? != expected {
        return Err("Restored fixture content differs from the expected snapshot".into());
    }
    Ok(())
}
