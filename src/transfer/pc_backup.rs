//! Destination adapter. Never persists a temporary endpoint or REST credential.
use super::reverse_tunnel::ReverseSession;
use crate::{
    agent::{Action, PcOperation},
    backup::{self, AgentEndpoint, DestinationKind, JobDescription},
    direct_backup::{self, DirectRequest},
    restic::RestCredentials,
    servers::connection::Connection,
};
use serde_json::Value;
use std::collections::BTreeSet;

pub fn execute(
    connection: &Connection,
    request: DirectRequest<'_>,
    agent: Option<&AgentEndpoint>,
    action: Action,
    session: &ReverseSession<'_>,
) -> Result<Value, String> {
    if request.destination.kind != DestinationKind::LocalPcRepository
        || connection.data_identity()? != session.source_identity()?
    {
        return Err("PC repository context does not match this server".into());
    }
    let is_backup = matches!(action, Action::BackupSelected { .. });
    let result = if let Some(endpoint) = agent {
        let description: JobDescription = serde_json::from_value(backup::call_cancellable(
            connection,
            endpoint,
            Action::Describe {},
            30,
            Some(&session.cancellation),
        )?)
        .map_err(|_| "Invalid agent description")?;
        if description.pc_repository != Some(session.repository_id) {
            return Err("Agent job is not authorized for this PC repository. Export and install the updated agent configuration".into());
        }
        if description.sources.iter().collect::<BTreeSet<_>>()
            != request.sources.iter().collect::<BTreeSet<_>>()
            || description.restore_root != std::path::Path::new(request.restore_root)
        {
            return Err("Agent source selection changed".into());
        }
        let auth = session.credentials();
        let value = backup::call_cancellable(
            connection,
            endpoint,
            Action::PcRepository {
                repository_id: session.repository_id,
                port: session.port,
                auth: RestCredentials {
                    username: auth.username.clone(),
                    password: auth.password.clone(),
                },
                action: PcOperation::from_action(action)?,
            },
            description.timeout_seconds.min(86370) + 30,
            Some(&session.cancellation),
        )?;
        if is_backup {
            session.verifying();
            let auth = session.credentials();
            backup::call_cancellable(
                connection,
                endpoint,
                Action::PcRepository {
                    repository_id: session.repository_id,
                    port: session.port,
                    auth: RestCredentials {
                        username: auth.username.clone(),
                        password: auth.password.clone(),
                    },
                    action: PcOperation::Check { read_data: true },
                },
                description.timeout_seconds.min(86370) + 30,
                Some(&session.cancellation),
            )?;
        }
        value
    } else {
        let value = direct_backup::execute_in_tunnel(
            connection,
            DirectRequest {
                executable: request.executable,
                password_file: request.password_file,
                restore_root: request.restore_root,
                destination: request.destination,
                sources: request.sources,
            },
            action,
            session,
        )?;
        if is_backup {
            session.verifying();
            direct_backup::execute_in_tunnel(
                connection,
                request,
                Action::Check { read_data: true },
                session,
            )?;
        }
        value
    };
    Ok(result)
}
