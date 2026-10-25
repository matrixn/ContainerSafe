//! Export a single authorized snapshot via supported `restic copy`, not a live repo mirror.
use super::*;
use crate::{
    agent::Action, backup::Destination, direct_backup, restic::SnapshotInfo,
    servers::connection::Connection,
};
use std::{collections::BTreeSet, path::PathBuf};
use uuid::Uuid;

pub struct ExportRequest<'a> {
    pub plan_id: Uuid,
    pub restic: &'a str,
    pub password_file: &'a str,
    pub restore_root: &'a str,
    pub destination: &'a Destination,
    pub approved_sources: &'a [PathBuf],
    pub snapshot_id: &'a str,
}
pub fn create(
    connection: &Connection,
    request: &ExportRequest<'_>,
) -> Result<SourceIdentity, String> {
    if request.plan_id.is_nil()
        || !hex_digest(request.snapshot_id)
        || request.approved_sources.is_empty()
    {
        return Err("Select an exact approved snapshot first".into());
    }
    let value = direct_backup::execute(
        connection,
        request.restic,
        request.password_file,
        request.restore_root,
        request.destination,
        request.approved_sources,
        Action::Snapshots {},
    )?;
    let snapshots: Vec<SnapshotInfo> =
        serde_json::from_value(value).map_err(|_| "Invalid snapshot metadata")?;
    let snapshot = snapshots
        .iter()
        .find(|s| s.id == request.snapshot_id)
        .ok_or("Snapshot not found in this plan's repository")?;
    let expected: BTreeSet<_> = request
        .approved_sources
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    if snapshot.paths.iter().cloned().collect::<BTreeSet<_>>() != expected {
        return Err("Snapshot sources differ from the currently approved file sources".into());
    }
    let id = Uuid::new_v4();
    let arguments = vec![
        request.restic.into(),
        request.password_file.into(),
        request.destination.location.clone(),
        request.snapshot_id.into(),
        request.restore_root.into(),
        id.to_string(),
    ];
    let output = connection.restic_script(include_str!("export_artifact.sh"), &arguments, 21600)?;
    let output = String::from_utf8(output).map_err(|_| "Invalid artifact metadata")?;
    let lines: Vec<_> = output.lines().collect();
    if lines.len() != 4 {
        return Err("Incomplete encrypted artifact export".into());
    }
    let identity = connection.data_identity()?;
    let source = SourceIdentity {
        server_id: identity.server_id,
        endpoint: format!("[{}]:{}", identity.host, identity.port),
        host_key: identity.host_key,
        artifact_id: id,
        snapshot_id: request.snapshot_id.into(),
        remote_path: lines[3].into(),
        size: lines[1].parse().map_err(|_| "Invalid artifact size")?,
        modified: lines[2]
            .parse()
            .map_err(|_| "Invalid artifact modification identity")?,
        sha256: lines[0].into(),
    };
    source.validate().map_err(|e| e.to_string())?;
    Ok(source)
}
