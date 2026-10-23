use crate::plan::BackupReport;
use crate::selection::{self, BackupSelection};
use crate::{servers::connection::Connection, Container};

use std::error::Error;
use std::io;

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn application_of(container: &Container) -> String {
    let name = container.name.trim_start_matches('/');

    container
        .config
        .labels
        .as_ref()
        .and_then(|labels| labels.get("com.docker.compose.project"))
        .filter(|project| !project.is_empty())
        .map(|project| format!("compose/{project}"))
        .unwrap_or_else(|| format!("standalone/{name}"))
}

fn check_readable(connection: &Connection, container: &Container, path: &str) -> bool {
    let name = container.name.trim_start_matches('/');

    // Folosim doar comenzi de verificare, fara scriere.
    // Argumentele sunt escapate pentru shell-ul SSH.
    let command = format!(
        "exec {} sh -c \
         'test -e \"$1\" && test -r \"$1\"' sh {}",
        shell_quote(name),
        shell_quote(path)
    );

    connection.docker(&command).is_ok()
}

pub fn run(
    connection: &Connection,
    report: &BackupReport,
    selection: &BackupSelection,
) -> Result<(), Box<dyn Error>> {
    let errors = selection::validate(report, selection);

    if !errors.is_empty() {
        for error in errors {
            eprintln!("[BLOCK] {error}");
        }

        return Err(io::Error::other("Backup selection is not valid").into());
    }

    // Recitim inventarul Docker, nu ne bazam doar
    // pe JSON-ul generat anterior.
    let containers = crate::discover(connection).map_err(io::Error::other)?;

    let mut selected_count = 0;
    let mut passed = 0;
    let mut blocked = 0;

    println!("========================================");
    println!("ContainerSafe v{} - Backup Preflight", crate::VERSION);
    println!("========================================");
    println!("Mode: READ-ONLY");
    println!("No backup operations will be executed.");

    for chosen_app in &selection.applications {
        let Some(plan_app) = report
            .applications
            .iter()
            .find(|a| a.application == chosen_app.application)
        else {
            continue;
        };

        for chosen in &chosen_app.sources {
            if chosen.decision != "include" {
                continue;
            }

            selected_count += 1;

            println!();
            println!("[APPLICATION] {}", chosen_app.application);
            println!("[SOURCE] {}", chosen.path);

            let Some(source_plan) = plan_app.sources.iter().find(|s| s.path == chosen.path) else {
                println!("  [BLOCK] Source missing from plan");
                blocked += 1;
                continue;
            };

            // Reconfirmam ca mount-ul exista in inventarul
            // Docker actual, in aplicatia corecta.
            let matching = containers.iter().find_map(|container| {
                if application_of(container) != chosen_app.application {
                    return None;
                }

                container
                    .mounts
                    .iter()
                    .find(|mount| {
                        mount.source == chosen.path
                            && mount.destination == source_plan.destination
                            && mount.mount_type == source_plan.mount_type
                    })
                    .map(|mount| (container, mount))
            });

            let Some((container, mount)) = matching else {
                println!("  [BLOCK] Mount not found in current Docker state");
                blocked += 1;
                continue;
            };

            println!("  [PASS] Docker mount still present");
            println!("  Container: {}", container.name);
            println!("  Destination: {}", mount.destination);

            if container.state.status != "running" {
                println!("  [REVIEW] Container is not running");
                println!("  [REVIEW] Host path access not checked");
                blocked += 1;
                continue;
            }

            if check_readable(connection, container, &mount.destination) {
                println!("  [PASS] Mounted path exists and is readable");
                passed += 1;
            } else {
                // Unele imagini nu contin /bin/sh.
                // Nu afirmam ca datele lipsesc fara alt test.
                println!(
                    "  [REVIEW] Could not verify readability \
                     through docker exec"
                );
                blocked += 1;
            }

            println!("  [PENDING] Size estimation");
            println!("  [PENDING] Snapshot consistency");
        }

        for db in &chosen_app.databases {
            if db.decision == "logical_dump" {
                println!();
                println!("[DATABASE] {}", db.container);
                println!("  [BLOCK] Logical dump adapter not implemented");
                blocked += 1;
            }
        }
    }

    println!();
    println!("========================================");
    println!("Selected sources: {selected_count}");
    println!("Basic checks passed: {passed}");
    println!("Checks requiring review: {blocked}");
    println!();
    println!("[PENDING] Compose files and environment");
    println!("[PENDING] External dependencies");
    println!("[PENDING] Destination and available space");
    println!("[PENDING] Restore verification");
    println!();
    println!("RESULT: NOT READY FOR BACKUP EXECUTION");
    println!("========================================");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_shell_arguments() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}
