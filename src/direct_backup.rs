//! Restic over the existing pinned SSH session. No installed ContainerSafe agent.
use crate::{agent::Action, backup::Destination, servers::connection::Connection};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

// POSIX sh + BusyBox-compatible utilities used by DSM. No bash, Python or jq.
pub const SCRIPT: &str = include_str!("direct_backup.sh");

pub fn execute(
    connection: &Connection,
    executable: &str,
    password_file: &str,
    restore_root: &str,
    destination: &Destination,
    sources: &[PathBuf],
    action: Action,
) -> Result<Value, String> {
    destination.validate()?;
    for value in [executable, password_file, restore_root] {
        if !value.starts_with('/') || value.contains(['\0', '\r', '\n']) {
            return Err("Use absolute Linux paths for Restic, password and restore folder".into());
        }
    }
    let (operation, snapshot, directory) = match action {
        Action::Init {} => ("init", String::new(), String::new()),
        Action::BackupSelected {
            expected_sources,
            expected_repository,
        } => {
            if expected_sources != sources
                || expected_repository != Path::new(&destination.location)
                || sources.is_empty()
            {
                return Err("Backup plan changed; review approved sources".into());
            }
            ("backup", String::new(), String::new())
        }
        Action::Snapshots {} => ("snapshots", String::new(), String::new()),
        Action::Check { read_data: true } => ("check", String::new(), String::new()),
        Action::Restore {
            snapshot_id,
            directory,
        } => {
            if snapshot_id.len() != 64
                || !snapshot_id.bytes().all(|c| c.is_ascii_hexdigit())
                || directory.is_empty()
                || directory.len() > 64
                || !directory
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            {
                return Err("An exact snapshot and a new restore directory are required".into());
            }
            ("restore", snapshot_id, directory)
        }
        _ => return Err("Unsupported direct Restic operation".into()),
    };
    let mut arguments = vec![
        operation.into(),
        executable.into(),
        password_file.into(),
        destination.location.clone(),
        restore_root.into(),
        snapshot.clone(),
        directory.clone(),
    ];
    for source in sources {
        let value = source.to_str().ok_or("Invalid source encoding")?;
        if !value.starts_with('/') {
            return Err("Backup source must be an absolute Linux path".into());
        }
        arguments.push(value.into());
    }
    let started = Instant::now();
    let output = connection.restic_script(SCRIPT, &arguments, 3600)?;
    match operation {
        "init" => Ok(json!({"initialized":true})),
        "snapshots" => {
            serde_json::from_slice(&output).map_err(|_| "Invalid Restic snapshot list".into())
        }
        "backup" => summary(&output),
        "check" => {
            let value = summary(&output)?;
            if value["num_errors"].as_u64() != Some(0) {
                return Err("Repository integrity check failed".into());
            }
            Ok(json!({"all_data_read":true,"elapsed_seconds":started.elapsed().as_secs_f64()}))
        }
        "restore" => {
            let result = summary(&output)?;
            if result["files_skipped"].as_u64().unwrap_or(0) != 0 {
                return Err("Restore skipped files; not verified".into());
            }
            Ok(
                json!({"snapshot_id":snapshot,"target":format!("{}/{}",restore_root.trim_end_matches('/'),directory),
                "files_restored":result["files_restored"].as_u64().ok_or("Missing restore file count")?,
                "bytes_restored":result["bytes_restored"].as_u64().ok_or("Missing restore byte count")?,"content_verified":true}),
            )
        }
        _ => unreachable!(),
    }
}
fn summary(output: &[u8]) -> Result<Value, String> {
    let mut summary = None;
    for line in output
        .split(|c| *c == b'\n')
        .filter(|line| !line.is_empty())
    {
        let value: Value =
            serde_json::from_slice(line).map_err(|_| "Invalid Restic JSON output")?;
        if value["message_type"] == "summary" {
            if summary.is_some() {
                return Err("Duplicate Restic summary".into());
            }
            summary = Some(value);
        }
    }
    summary.ok_or_else(|| "Missing Restic summary".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summaries_are_strict_and_script_never_interpolates_secrets() {
        assert!(summary(br#"{"message_type":"summary","snapshot_id":"x"}"#).is_ok());
        assert!(summary(b"not json").is_err());
        assert!(
            summary(b"{\"message_type\":\"summary\"}\n{\"message_type\":\"summary\"}").is_err()
        );
        assert!(!SCRIPT.contains("sudo"));
        assert!(!SCRIPT.contains("eval "));
        assert!(SCRIPT.contains("StrictHostKeyChecking=yes"));
    }
}
