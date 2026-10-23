mod plan;
mod preflight;
mod restic_lab;
mod selection;

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::io;
use std::process::Command;

// Pentru MVP folosim aliasul definit in ~/.ssh/config.
const SSH_HOST: &str = "zion";
const DOCKER: &str = "sudo -n /usr/local/bin/docker";

#[derive(Debug, Deserialize)]
struct Container {
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

fn ssh_command(remote_command: &str) -> Result<String, Box<dyn Error>> {
    let output = Command::new("ssh")
        .args([
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "ConnectTimeout=8",
            SSH_HOST,
            remote_command,
        ])
        .output()?;

    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);

        return Err(io::Error::other(format!("SSH/Docker failed: {}", message.trim())).into());
    }

    Ok(String::from_utf8(output.stdout)?)
}

fn discover() -> Result<Vec<Container>, Box<dyn Error>> {
    // 1. Obtinem lista ID-urilor, inclusiv containerele oprite.
    let command = format!("{DOCKER} ps -a -q");
    let raw_ids = ssh_command(&command)?;

    let ids: Vec<&str> = raw_ids
        .lines()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .collect();

    if ids.is_empty() {
        return Ok(Vec::new());
    }

    // Acceptam doar ID-uri Docker hexazecimale.
    // Nu introducem nume sau text arbitrar in comanda SSH.
    for id in &ids {
        if id.len() < 12 || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(io::Error::other("Invalid container ID returned by Docker").into());
        }
    }

    // 2. Inspectam containerele intr-o singura operatiune.
    let command = format!("{DOCKER} inspect --type container {}", ids.join(" "));

    let json = ssh_command(&command)?;

    // Retinem doar campurile definite in structurile Rust.
    let containers: Vec<Container> = serde_json::from_str(&json)?;

    Ok(containers)
}

fn print_report(containers: &[Container]) {
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
    println!("Host: {SSH_HOST}");
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

fn print_backup_readiness(containers: &[Container]) {
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
    println!("ContainerSafe v0.3 - Backup Readiness");
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

fn main() {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "discover".to_string());

    if mode == "lab-backup" {
        if let Err(error) = restic_lab::run() {
            eprintln!("Backup lab failed: {error}");
            std::process::exit(1);
        }

        return;
    }

    if mode == "preflight" {
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let plan_json = std::fs::read_to_string("backup-plan.json")?;

            let selection_json = std::fs::read_to_string("backup-selection.json")?;

            let report: plan::BackupReport = serde_json::from_str(&plan_json)?;

            let choices: selection::BackupSelection = serde_json::from_str(&selection_json)?;

            preflight::run(&report, &choices)?;

            Ok(())
        })();

        if let Err(error) = result {
            eprintln!("Preflight failed: {error}");
            std::process::exit(1);
        }

        return;
    }

    if mode == "draft" || mode == "validate" {
        let plan_json =
            std::fs::read_to_string("backup-plan.json").expect("Cannot read backup-plan.json");

        let report: plan::BackupReport =
            serde_json::from_str(&plan_json).expect("Invalid backup-plan.json");

        if mode == "draft" {
            let draft = selection::draft(&report);

            println!(
                "{}",
                serde_json::to_string_pretty(&draft).expect("Cannot serialize selection")
            );

            return;
        }

        let selection_json = std::fs::read_to_string("backup-selection.json")
            .expect("Cannot read backup-selection.json");

        let choices: selection::BackupSelection =
            serde_json::from_str(&selection_json).expect("Invalid backup-selection.json");

        let errors = selection::validate(&report, &choices);

        if errors.is_empty() {
            println!("Selection decisions are valid.");
            println!("Backup execution is NOT yet authorized.");
            println!("Compose dependencies and consistency");
            println!("still require verification.");
        } else {
            println!("Selection requires review:");

            for error in errors {
                println!("  - {error}");
            }

            std::process::exit(1);
        }

        return;
    }

    if !matches!(mode.as_str(), "discover" | "audit" | "plan") {
        eprintln!("Usage: containersafe [discover|audit|plan]");
        std::process::exit(2);
    }

    // Pe stderr, astfel incat comanda "plan" sa
    // poata produce JSON valid direct in stdout.
    eprintln!("Connecting to Synology via SSH...");

    match discover() {
        Ok(containers) => match mode.as_str() {
            "discover" => print_report(&containers),
            "audit" => print_backup_readiness(&containers),
            "plan" => {
                let report = plan::build(&containers);

                match serde_json::to_string_pretty(&report) {
                    Ok(json) => println!("{json}"),
                    Err(error) => {
                        eprintln!("JSON serialization failed: {error}");
                        std::process::exit(1);
                    }
                }
            }
            _ => unreachable!(),
        },
        Err(error) => {
            eprintln!("Discovery failed: {error}");
            std::process::exit(1);
        }
    }
}
