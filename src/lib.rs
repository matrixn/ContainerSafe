pub mod agent;
#[cfg(feature = "server-management")]
pub mod backup;
pub mod database;
#[cfg(feature = "server-management")]
pub mod direct_backup;
pub mod plan;
#[cfg(feature = "server-management")]
pub mod preflight;
pub mod restic;
pub mod restic_lab;
pub mod selection;
#[cfg(feature = "server-management")]
pub mod transfer;

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[cfg(feature = "server-management")]
pub mod servers;
pub const VERSION: &str = "0.35";

#[derive(Debug, Deserialize)]
pub struct Container {
    #[serde(rename = "Id")]
    id: String,

    #[serde(rename = "Name")]
    name: String,

    #[serde(rename = "Config")]
    config: ContainerConfig,

    #[serde(rename = "State")]
    state: ContainerState,

    #[serde(rename = "Mounts", default)]
    mounts: Vec<Mount>,
}

#[derive(Debug, Deserialize)]
struct ContainerConfig {
    #[serde(rename = "Image")]
    image: String,

    #[serde(rename = "Labels")]
    labels: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct ContainerState {
    #[serde(rename = "Status")]
    status: String,
}

#[derive(Debug, Deserialize)]
struct Mount {
    #[serde(rename = "Type")]
    mount_type: String,

    #[serde(rename = "Source")]
    source: String,

    #[serde(rename = "Destination")]
    destination: String,

    #[serde(rename = "RW")]
    read_write: bool,
}

#[cfg(feature = "server-management")]
pub fn discover(connection: &servers::connection::Connection) -> Result<Vec<Container>, String> {
    let ids = connection.docker("ps -a -q")?;
    let ids: Vec<_> = ids.lines().filter(|s| !s.is_empty()).collect();
    if ids.is_empty() {
        return Ok(vec![]);
    }
    if ids
        .iter()
        .any(|s| s.len() < 12 || s.len() > 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("Invalid Docker container ID".into());
    }
    let json = connection.docker(&format!("inspect --type container {}", ids.join(" ")))?;
    serde_json::from_str(&json).map_err(|_| "Invalid Docker inventory JSON".into())
}

pub fn print_report(containers: &[Container]) {
    let mut projects: BTreeMap<String, Vec<&Container>> = BTreeMap::new();

    for container in containers {
        let project = container
            .config
            .labels
            .as_ref()
            .and_then(|labels| labels.get("com.docker.compose.project"))
            .cloned()
            .unwrap_or_else(|| "Standalone / Unknown".to_string());

        projects.entry(project).or_default().push(container);
    }

    println!();
    println!("ContainerSafe - Docker Discovery");

    println!("Containers: {}", containers.len());
    println!("Projects/groups: {}", projects.len());
    println!("{}", "=".repeat(60));

    for (project, members) in projects {
        println!();
        println!("[PROJECT] {project}");

        for container in members {
            let name = container.name.trim_start_matches('/');

            let service = container
                .config
                .labels
                .as_ref()
                .and_then(|labels| labels.get("com.docker.compose.service"))
                .map(String::as_str)
                .unwrap_or("unknown");

            println!();
            println!("  Container: {name}");
            println!("  ID: {}", &container.id[..12]);
            println!("  Service: {service}");
            println!("  Image: {}", container.config.image);
            println!("  Status: {}", container.state.status);

            if container.mounts.is_empty() {
                println!("  Mounts: none detected");
            } else {
                println!("  Mounts:");

                for mount in &container.mounts {
                    let mode = if mount.read_write { "rw" } else { "ro" };

                    println!(
                        "    [{}] {} -> {} ({})",
                        mount.mount_type, mount.source, mount.destination, mode
                    );
                }
            }

            let image = container.config.image.to_lowercase();

            if image.contains("postgres") || image.contains("mariadb") || image.contains("mysql") {
                println!(
                    "  [WARNING] Database detected: \
                     requires consistent DB backup."
                );
            }
        }
    }
}

fn detect_database(image: &str) -> Option<&'static str> {
    let image = image.to_ascii_lowercase();

    if image.contains("postgres") {
        Some("PostgreSQL")
    } else if image.contains("mariadb") {
        Some("MariaDB")
    } else if image.contains("mysql") {
        Some("MySQL")
    } else if image.contains("mongo") {
        Some("MongoDB")
    } else if image.contains("redis") || image.contains("valkey") {
        Some("Redis/Valkey")
    } else {
        None
    }
}

fn is_backup_candidate(mount: &Mount) -> bool {
    // Aceste montari sunt resurse ale sistemului,
    // nu directoare de date ale aplicatiei.
    let excluded = [
        "/var/run/docker.sock",
        "/etc/localtime",
        "/etc/timezone",
        "/etc/hosts",
        "/etc/hostname",
        "/etc/resolv.conf",
    ];

    (mount.mount_type == "bind" || mount.mount_type == "volume")
        && !excluded.contains(&mount.source.as_str())
}

pub fn print_backup_readiness(containers: &[Container]) {
    let mut projects: BTreeMap<String, Vec<&Container>> = BTreeMap::new();

    for container in containers {
        let project = container
            .config
            .labels
            .as_ref()
            .and_then(|labels| labels.get("com.docker.compose.project"))
            .cloned()
            .unwrap_or_else(|| "Standalone / Unknown".to_string());

        projects.entry(project).or_default().push(container);
    }

    println!();
    println!("==========================================");
    println!("ContainerSafe v{} - Backup Readiness", VERSION);
    println!("==========================================");
    println!("Mode: READ-ONLY");
    println!("No backups have been created or verified.");

    for (project, members) in projects {
        let mut sources = BTreeSet::new();
        let mut databases = Vec::new();
        let mut stopped = Vec::new();
        let mut docker_socket = Vec::new();

        for container in &members {
            let name = container.name.trim_start_matches('/');

            if container.state.status != "running" {
                stopped.push(name.to_string());
            }

            if let Some(db) = detect_database(&container.config.image) {
                databases.push(format!("{name}: {db}"));
            }

            for mount in &container.mounts {
                if mount.source == "/var/run/docker.sock" {
                    docker_socket.push(name.to_string());
                }

                if is_backup_candidate(mount) {
                    sources.insert(mount.source.clone());
                }
            }
        }

        println!();
        println!("------------------------------------------");
        println!("PROJECT: {project}");
        println!("Containers: {}", members.len());
        println!("Candidate storage paths: {}", sources.len());

        if sources.is_empty() {
            println!("  No persistent mounts detected.");
            println!("  [REVIEW] Check for external state.");
        } else {
            println!();
            println!("  Candidate backup sources:");

            for source in &sources {
                println!("    - {source}");
            }
        }

        if !databases.is_empty() {
            println!();
            println!("  [WARNING] Database services detected:");

            for database in &databases {
                println!("    - {database}");
            }

            println!(
                "  Active database files must not be \
                 blindly copied as a consistent backup."
            );
            println!(
                "  A database-specific backup method \
                 is required."
            );
        }

        if !stopped.is_empty() {
            println!();
            println!("  [INFO] Stopped containers:");

            for name in &stopped {
                println!("    - {name}");
            }

            println!("  Discovery will not start these containers.");
        }

        if !docker_socket.is_empty() {
            println!();
            println!("  [SECURITY] Docker socket mounts:");

            for name in &docker_socket {
                println!("    - {name}");
            }

            println!(
                "  Docker socket must not be copied \
                 into a backup."
            );
        }

        println!();

        if project == "Standalone / Unknown" {
            println!(
                "  [REVIEW] No Compose project label. \
                 Recovery configuration is unknown."
            );
        } else {
            println!(
                "  [REVIEW] Compose files and environment \
                 dependencies not yet verified."
            );
        }

        println!("  STATUS: BACKUP PLAN REQUIRED");
    }

    println!();
    println!("==========================================");
    println!("Readiness analysis completed.");
    println!("No files or containers were modified.");
    println!("==========================================");
}

#[cfg(test)]
mod readiness_tests {
    use super::*;

    #[test]
    fn detects_postgresql() {
        assert_eq!(detect_database("postgres:17-alpine"), Some("PostgreSQL"));
    }

    #[test]
    fn detects_immich_postgresql() {
        assert_eq!(
            detect_database("ghcr.io/immich-app/postgres:14"),
            Some("PostgreSQL")
        );
    }

    #[test]
    fn excludes_docker_socket() {
        let mount = Mount {
            mount_type: "bind".to_string(),
            source: "/var/run/docker.sock".to_string(),
            destination: "/var/run/docker.sock".to_string(),
            read_write: true,
        };

        assert!(!is_backup_candidate(&mount));
    }

    #[test]
    fn includes_application_storage() {
        let mount = Mount {
            mount_type: "bind".to_string(),
            source: "/volume1/docker/immich/library".to_string(),
            destination: "/data".to_string(),
            read_write: true,
        };

        assert!(is_backup_candidate(&mount));
    }
}
