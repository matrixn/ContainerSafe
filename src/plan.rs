use crate::{is_backup_candidate, Container};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupReport {
    pub schema_version: u32,
    pub read_only: bool,
    pub applications: Vec<ApplicationPlan>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApplicationPlan {
    pub application: String,
    pub status: String,
    pub containers: Vec<String>,
    pub sources: Vec<SourcePlan>,
    pub databases: Vec<DatabasePlan>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourcePlan {
    pub path: String,
    pub mount_type: String,
    pub destination: String,
    pub policy: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DatabasePlan {
    pub container: String,
    pub engine: String,
    pub required_method: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_path: String,
}

fn classify_source(image: &str, path: &str) -> &'static str {
    match crate::detect_database(image) {
        Some("PostgreSQL" | "MariaDB" | "MySQL" | "MongoDB" | "SQLite") => "requires_logical_dump",
        Some("Redis/Valkey") => "requires_persistence_review",
        _ if path.starts_with("/volume1/Media/") || path == "/volume1/RuTorrent" => {
            "large_data_requires_opt_in"
        }
        _ => "review_for_backup",
    }
}

fn priority(policy: &str) -> u8 {
    match policy {
        "requires_logical_dump" => 4,
        "requires_persistence_review" => 3,
        "large_data_requires_opt_in" => 2,
        _ => 1,
    }
}

pub fn build(containers: &[Container]) -> BackupReport {
    let mut groups: BTreeMap<String, Vec<&Container>> = BTreeMap::new();

    for container in containers {
        let name = container.name.trim_start_matches('/');

        let group = container
            .config
            .labels
            .as_ref()
            .and_then(|labels| labels.get("com.docker.compose.project"))
            .filter(|project| !project.is_empty())
            .map(|project| format!("compose/{project}"))
            .unwrap_or_else(|| format!("standalone/{name}"));

        groups.entry(group).or_default().push(container);
    }

    let mut applications = Vec::new();

    for (group, members) in groups {
        let mut sources: BTreeMap<String, SourcePlan> = BTreeMap::new();

        let mut databases = Vec::new();
        let mut warnings = Vec::new();
        let mut container_names = Vec::new();

        for container in members {
            let name = container.name.trim_start_matches('/');
            container_names.push(name.to_string());

            if container.state.status != "running" {
                warnings.push(format!(
                    "{name}: container is not running; do not start automatically"
                ));
            }

            if let Some(engine) = crate::database::container_engine(container) {
                let required_method = match engine {
                    "PostgreSQL" | "MariaDB" | "MySQL" | "MongoDB" | "SQLite" | "Redis/Valkey" => {
                        "logical_dump"
                    }
                    _ => "manual_review",
                };

                databases.push(DatabasePlan {
                    container: name.to_string(),
                    engine: engine.to_string(),
                    required_method: required_method.to_string(),
                    source_path: if engine == "SQLite" {
                        container
                            .config
                            .labels
                            .as_ref()
                            .and_then(|l| l.get("containersafe.sqlite.path"))
                            .cloned()
                            .unwrap_or_default()
                    } else {
                        String::new()
                    },
                });
            }

            for mount in &container.mounts {
                if mount.source == "/var/run/docker.sock" {
                    warnings.push(format!("{name}: privileged Docker socket mount excluded"));
                    continue;
                }

                if !is_backup_candidate(mount) {
                    continue;
                }

                let policy = if crate::database::container_engine(container) == Some("SQLite") {
                    "requires_logical_dump"
                } else {
                    classify_source(&container.config.image, &mount.source)
                };

                let candidate = SourcePlan {
                    path: mount.source.clone(),
                    mount_type: mount.mount_type.clone(),
                    destination: mount.destination.clone(),
                    policy: policy.to_string(),
                };

                match sources.get(&mount.source) {
                    Some(existing) if priority(&existing.policy) >= priority(policy) => {}
                    _ => {
                        sources.insert(mount.source.clone(), candidate);
                    }
                }
            }
        }

        if sources.is_empty() {
            warnings.push(
                "No persistent mounts detected; external or internal state may exist".to_string(),
            );
        }

        if group.starts_with("compose/") {
            warnings.push(
                "Compose YAML, overrides and .env have not been located or verified".to_string(),
            );
        } else {
            warnings
                .push("Standalone container: recreation configuration not verified".to_string());
        }

        warnings.push(
            "External databases, named dependencies and application consistency not verified"
                .to_string(),
        );

        applications.push(ApplicationPlan {
            application: group,
            status: "needs_review".to_string(),
            containers: container_names,
            sources: sources.into_values().collect(),
            databases,
            warnings,
        });
    }

    BackupReport {
        schema_version: 1,
        read_only: true,
        applications,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_requires_dump() {
        assert_eq!(
            classify_source("postgres:17-alpine", "/volume1/docker/db"),
            "requires_logical_dump"
        );
    }

    #[test]
    fn media_requires_opt_in() {
        assert_eq!(
            classify_source("linuxserver/sonarr", "/volume1/Media/Seriale"),
            "large_data_requires_opt_in"
        );
    }

    #[test]
    fn regular_app_data_needs_review() {
        assert_eq!(
            classify_source(
                "ghcr.io/immich-app/immich-server:v3",
                "/volume1/docker/immich/library"
            ),
            "review_for_backup"
        );
    }
}
