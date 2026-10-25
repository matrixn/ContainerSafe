//! Desktop-to-agent backup transport and destination validation.
use crate::{
    agent::{Action, AgentError, Request, PROTOCOL_VERSION},
    servers::connection::Connection,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationKind {
    #[default]
    ServerFolder,
    ThisComputer,
    Remote,
    LocalPcRepository,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    pub kind: DestinationKind,
    pub location: String,
}
impl Destination {
    pub fn validate(&self) -> Result<(), String> {
        let value = self.location.as_str();
        if value.is_empty() || value.len() > 4096 || value.contains(['\0', '\r', '\n']) {
            return Err("Enter a backup destination".into());
        }
        let valid = match self.kind {
            DestinationKind::ServerFolder => value.starts_with('/') && !value.contains("/../"),
            DestinationKind::ThisComputer => value.starts_with("sftp:") && value.contains(":/"),
            DestinationKind::Remote => {
                (value.starts_with("sftp:") && value.contains(":/"))
                    || value.starts_with("s3:https://")
            }
            DestinationKind::LocalPcRepository => std::path::Path::new(value).is_absolute(),
        };
        if !valid {
            return Err("Use an absolute server folder, sftp:user@host:/path, or s3:https://endpoint/bucket/path".into());
        }
        if let Some(remote) = value.strip_prefix("sftp:") {
            let authority = remote.split(":/").next().unwrap_or("");
            if authority.is_empty() || authority.contains([':', '/', '\\', ' ', '\t']) {
                return Err("Use an SSH host alias for SFTP ports and keys; do not embed passwords in the URL".into());
            }
        }
        if value.starts_with("s3:")
            && (value.contains('@') || value.contains('?') || value.contains('#'))
        {
            return Err("S3 credentials belong in the agent account's protected AWS credentials file, never in the destination URL".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentEndpoint {
    pub executable: String,
    pub config: String,
    pub job_id: Uuid,
}
impl Default for AgentEndpoint {
    fn default() -> Self {
        Self {
            executable: "/usr/local/bin/containersafe-agent".into(),
            config: "/home/backup/.config/containersafe/agent.json".into(),
            job_id: Uuid::new_v4(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct JobDescription {
    pub repository: PathBuf,
    pub sources: Vec<PathBuf>,
    pub restore_root: PathBuf,
    pub timeout_seconds: u64,
    #[serde(default)]
    pub pc_repository: Option<Uuid>,
}

pub fn call(
    connection: &Connection,
    endpoint: &AgentEndpoint,
    action: Action,
    timeout: u64,
) -> Result<serde_json::Value, String> {
    call_cancellable(connection, endpoint, action, timeout, None)
}
pub fn call_cancellable(
    connection: &Connection,
    endpoint: &AgentEndpoint,
    action: Action,
    timeout: u64,
    cancellation: Option<&crate::transfer::CancellationToken>,
) -> Result<serde_json::Value, String> {
    let request = Request {
        protocol_version: PROTOCOL_VERSION,
        request_id: Uuid::new_v4(),
        job_id: endpoint.job_id,
        action,
    };
    let data = zeroize::Zeroizing::new(
        serde_json::to_vec(&request).map_err(|_| "Cannot encode agent request")?,
    );
    let output = connection.agent_cancellable(
        &endpoint.executable,
        &endpoint.config,
        &data,
        timeout,
        cancellation,
    )?;
    parse_response(&output, request.request_id)
}
fn parse_response(output: &[u8], expected: Uuid) -> Result<serde_json::Value, String> {
    let value: serde_json::Value = serde_json::from_slice(output)
        .map_err(|_| "Invalid agent response; upgrade the Linux agent")?;
    if value["protocol_version"].as_u64() != Some(PROTOCOL_VERSION as u64)
        || value["request_id"].as_str() != Some(expected.to_string().as_str())
    {
        return Err("Agent response identity/protocol mismatch".into());
    }
    match value["status"].as_str() {
        Some("ok") => value
            .get("result")
            .cloned()
            .ok_or_else(|| "Agent result missing".into()),
        Some("error") => {
            let error: AgentError = serde_json::from_value(value["error"].clone())
                .map_err(|_| "Unknown agent error")?;
            // Only expose typed codes. Remote free-form text could include secrets.
            let detail = match error.restic_kind.as_deref() {
                Some("MissingExecutable") => " — Restic executable not found",
                Some("WrongPassword") => " — repository password does not match",
                Some("RepositoryMissing") => " — initialize the repository first",
                Some("RepositoryExists") => " — repository already exists; do not initialize again",
                Some("RepositoryLocked") => " — another Restic operation holds the repository lock",
                Some("IncompleteBackup") => {
                    " — some files could not be saved; backup is incomplete"
                }
                Some("UnsafeSource") => {
                    " — source missing, inaccessible or overlaps the repository"
                }
                Some("Timeout") => " — server-side operation timed out",
                _ => "",
            };
            Err(format!(
                "Agent: {:?}{}{}",
                error.code,
                detail,
                error
                    .exit_code
                    .map(|code| format!(" (exit {code})"))
                    .unwrap_or_default()
            ))
        }
        _ => Err("Invalid agent response status".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn destinations_require_matching_transport_and_no_url_credentials() {
        for (kind, location, valid) in [
            (DestinationKind::ServerFolder, "/volume1/backups/app", true),
            (DestinationKind::ServerFolder, "relative", false),
            (DestinationKind::ThisComputer, "C:/backup", false),
            (
                DestinationKind::ThisComputer,
                "sftp:pc-backup:/C:/Backups/app",
                true,
            ),
            (
                DestinationKind::Remote,
                "s3:https://storage.example/bucket/app",
                true,
            ),
            (
                DestinationKind::Remote,
                "s3:https://user:secret@storage/bucket",
                false,
            ),
            (DestinationKind::Remote, "s3:http://storage/bucket", false),
            (
                DestinationKind::ThisComputer,
                "sftp:user:password@pc:/backups",
                false,
            ),
        ] {
            assert_eq!(
                Destination {
                    kind,
                    location: location.into()
                }
                .validate()
                .is_ok(),
                valid
            );
        }
    }
    #[test]
    fn replies_are_bound_to_request_and_typed_errors() {
        let id = Uuid::new_v4();
        let response = serde_json::json!({"protocol_version":1,"request_id":id,"status":"ok","result":{"ready":true}});
        assert!(parse_response(&serde_json::to_vec(&response).unwrap(), id).is_ok());
        assert!(parse_response(&serde_json::to_vec(&response).unwrap(), Uuid::new_v4()).is_err());
        let response = serde_json::json!({"protocol_version":1,"request_id":id,"status":"error","error":{"code":"plan_changed","restic_kind":"SECRET-MARKER"}});
        let error = parse_response(&serde_json::to_vec(&response).unwrap(), id).unwrap_err();
        assert!(error.contains("PlanChanged"));
        assert!(!error.contains("SECRET-MARKER"));
    }
}
