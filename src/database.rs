//! Explicit database selection and private, short-lived server-side exports.
use crate::{plan::ApplicationPlan, selection::ApplicationSelection, Container};
use serde::Serialize;
use std::path::PathBuf;
use uuid::Uuid;

pub fn supported(engine: &str) -> bool {
    matches!(
        engine,
        "PostgreSQL" | "MariaDB" | "MySQL" | "Redis/Valkey" | "MongoDB" | "SQLite"
    )
}

pub fn container_engine(container: &Container) -> Option<&'static str> {
    if container
        .config
        .labels
        .as_ref()
        .and_then(|l| l.get("containersafe.database"))
        .is_some_and(|v| v == "sqlite")
    {
        Some("SQLite")
    } else {
        crate::detect_database(&container.config.image)
    }
}

pub fn selected<'a>(
    app: &'a ApplicationPlan,
    selection: &ApplicationSelection,
) -> Result<Vec<&'a crate::plan::DatabasePlan>, String> {
    if app.application != selection.application {
        return Err("Database application binding changed".into());
    }
    let mut result = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for selected in &selection.databases {
        if !seen.insert(&selected.container) {
            return Err("Duplicate database selection".into());
        }
        if selected.decision != "logical_dump" {
            continue;
        }
        let db = app
            .databases
            .iter()
            .find(|db| db.container == selected.container)
            .ok_or("Database inventory changed; refresh and review selections")?;
        if !supported(&db.engine) {
            return Err("Database adapter is unavailable".into());
        }
        result.push(db);
    }
    Ok(result)
}

pub fn staging_path(restore_root: &str, job: Uuid) -> PathBuf {
    PathBuf::from(format!(
        "{}/containersafe-db-{job}",
        restore_root.trim_end_matches('/')
    ))
}

#[derive(Debug, Serialize)]
pub struct DumpSpec {
    pub container_id: String,
    pub container: String,
    pub engine: String,
    pub sqlite_path: String,
}

pub fn specs(
    app: &ApplicationPlan,
    selection: &ApplicationSelection,
    inventory: &[Container],
) -> Result<Vec<DumpSpec>, String> {
    selected(app, selection)?.iter().map(|db| {
        let container = inventory.iter().find(|c| c.name.trim_start_matches('/') == db.container).ok_or("Database container disappeared")?;
        if container.state.status != "running" || container_engine(container) != Some(db.engine.as_str()) {
            return Err("Database engine changed or container is not running; refresh and review selections".into());
        }
        if container.id.len() != 64 || !container.id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid database container identity".into());
        }
        let sqlite_path = if db.engine == "SQLite" {
            let path = container.config.labels.as_ref().and_then(|l| l.get("containersafe.sqlite.path")).ok_or("SQLite requires the containersafe.sqlite.path label")?;
            if !path.starts_with('/') || path.len() > 4096 || path.contains(['\0', '\r', '\n', '?', '#']) || path.split('/').any(|v| v == "..") {
                return Err("SQLite label must name an absolute in-container database file".into());
            }
            if path != &db.source_path { return Err("SQLite path changed; refresh and review selections".into()); }
            path.clone()
        } else { String::new() };
        Ok(DumpSpec { container_id: container.id.clone(), container: db.container.clone(), engine: db.engine.clone(), sqlite_path })
    }).collect()
}

#[cfg(feature = "server-management")]
pub mod remote {
    use super::*;
    use crate::servers::connection::Connection;
    pub const SCRIPT: &str = include_str!("database_dump.sh");
    /// The caller must finish this guard after the backup, including error paths.
    /// Drop retries cleanup during unwinding; failed/disconnected cleanup is never
    /// interpreted as permission to reuse a stale export on the next run.
    pub struct StagedDumps<'a> {
        connection: &'a Connection,
        arguments: Vec<String>,
        active: bool,
    }
    impl<'a> StagedDumps<'a> {
        pub fn prepare(
            connection: &'a Connection,
            path: &std::path::Path,
            specs: &[DumpSpec],
            sources: &[PathBuf],
            repository: &str,
            password: &str,
        ) -> Result<Self, String> {
            let mut arguments = vec![
                "prepare".into(),
                path.to_str().ok_or("Invalid dump directory")?.into(),
                connection.docker_prefix().into(),
                repository.into(),
                password.into(),
                specs.len().to_string(),
            ];
            for spec in specs {
                arguments.extend([
                    spec.container_id.clone(),
                    spec.engine.clone(),
                    spec.sqlite_path.clone(),
                    spec.container.clone(),
                ]);
            }
            arguments.extend(sources.iter().map(|p| p.to_string_lossy().into_owned()));
            connection.restic_script(SCRIPT, &arguments, 3600)?;
            arguments[0] = "cleanup".into();
            Ok(Self {
                connection,
                arguments,
                active: true,
            })
        }
        pub fn finish(mut self) -> Result<(), String> {
            self.connection.restic_script(SCRIPT, &self.arguments, 30).map_err(|_| "Database staging cleanup failed. Private plaintext exports may remain on the server; inspect the plan's containersafe-db directory before retrying".to_string())?;
            self.active = false;
            Ok(())
        }
    }
    impl Drop for StagedDumps<'_> {
        fn drop(&mut self) {
            if self.active {
                let _ = self.connection.restic_script(SCRIPT, &self.arguments, 30);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binds_live_identity_and_rejects_unsafe_sqlite_labels() {
        let mut value = serde_json::json!({"Id":"a".repeat(64),"Name":"/db","Config":{"Image":"app","Labels":{"containersafe.database":"sqlite","containersafe.sqlite.path":"/data/live.sqlite"}},"State":{"Status":"running"},"Mounts":[]});
        let selection = ApplicationSelection {
            application: "standalone/db".into(),
            sources: vec![],
            databases: vec![crate::selection::DatabaseSelection {
                container: "db".into(),
                decision: "logical_dump".into(),
            }],
        };
        for (path, status, valid) in [
            ("/data/live.sqlite", "running", true),
            ("/data/../secret", "running", false),
            ("/data/a?mode=rw", "running", false),
            ("/data/live.sqlite", "exited", false),
        ] {
            value["Config"]["Labels"]["containersafe.sqlite.path"] = path.into();
            value["State"]["Status"] = status.into();
            let inventory: Vec<Container> =
                serde_json::from_value(serde_json::json!([value])).unwrap();
            let report = crate::plan::build(&inventory);
            assert_eq!(
                specs(&report.applications[0], &selection, &inventory).is_ok(),
                valid
            );
        }
    }
}
