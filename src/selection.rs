use crate::plan::BackupReport;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupSelection {
    pub schema_version: u32,
    pub applications: Vec<ApplicationSelection>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApplicationSelection {
    pub application: String,
    pub sources: Vec<SourceSelection>,
    pub databases: Vec<DatabaseSelection>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SourceSelection {
    pub path: String,
    pub decision: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DatabaseSelection {
    pub container: String,
    pub decision: String,
}

pub fn draft(report: &BackupReport) -> BackupSelection {
    BackupSelection {
        schema_version: 1,
        applications: report
            .applications
            .iter()
            .map(|app| ApplicationSelection {
                application: app.application.clone(),
                sources: app
                    .sources
                    .iter()
                    .map(|source| SourceSelection {
                        path: source.path.clone(),
                        decision: "pending".to_string(),
                    })
                    .collect(),
                databases: app
                    .databases
                    .iter()
                    .map(|db| DatabaseSelection {
                        container: db.container.clone(),
                        decision: "pending".to_string(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub fn validate(report: &BackupReport, selection: &BackupSelection) -> Vec<String> {
    let mut errors = Vec::new();
    let mut selected_count = 0usize;

    if report.schema_version != 1 || selection.schema_version != 1 {
        errors.push("Unsupported schema version".to_string());
        return errors;
    }

    let mut seen_apps = BTreeSet::new();

    for chosen in &selection.applications {
        if !seen_apps.insert(&chosen.application) {
            errors.push(format!("Duplicate application: {}", chosen.application));
        }

        let Some(app) = report
            .applications
            .iter()
            .find(|a| a.application == chosen.application)
        else {
            errors.push(format!("Unknown application: {}", chosen.application));
            continue;
        };

        let mut seen_sources = BTreeSet::new();

        for selected in &chosen.sources {
            if !seen_sources.insert(&selected.path) {
                errors.push(format!("Duplicate source: {}", selected.path));
            }

            let Some(source) = app.sources.iter().find(|s| s.path == selected.path) else {
                errors.push(format!("Unknown source: {}", selected.path));
                continue;
            };

            match selected.decision.as_str() {
                "include" => {
                    if source.policy == "requires_logical_dump"
                        || source.policy == "requires_persistence_review"
                    {
                        errors.push(format!(
                            "Raw backup prohibited without special \
                             handling: {}",
                            selected.path
                        ));
                    } else {
                        selected_count += 1;
                    }
                }
                "exclude" => {}
                "pending" => errors.push(format!(
                    "No backup method selected; source review required: {}",
                    selected.path
                )),
                _ => errors.push(format!("Invalid source decision: {}", selected.path)),
            }
        }

        for source in &app.sources {
            if !seen_sources.contains(&source.path) {
                errors.push(format!("Missing decision for source: {}", source.path));
            }
        }

        let mut seen_databases = BTreeSet::new();

        for selected in &chosen.databases {
            if !seen_databases.insert(&selected.container) {
                errors.push(format!("Duplicate database: {}", selected.container));
            }

            let Some(db) = app
                .databases
                .iter()
                .find(|d| d.container == selected.container)
            else {
                errors.push(format!("Unknown database: {}", selected.container));
                continue;
            };

            match selected.decision.as_str() {
                "logical_dump" if crate::database::supported(&db.engine) => {
                    selected_count += 1;
                }
                "exclude" => {}
                "pending" => errors.push(format!(
                    "No backup method selected; database review required: {}",
                    selected.container
                )),
                _ => errors.push(format!(
                    "Unsupported database decision for {}: {}",
                    selected.container, selected.decision
                )),
            }
        }

        for db in &app.databases {
            if !seen_databases.contains(&db.container) {
                errors.push(format!("Missing decision for database: {}", db.container));
            }
        }
    }

    for app in &report.applications {
        if !seen_apps.contains(&app.application) {
            errors.push(format!("Missing application: {}", app.application));
        }
    }

    if selected_count == 0 {
        errors.push("Nothing selected for backup".to_string());
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_selection_is_not_automatically_approved() {
        let report = BackupReport {
            schema_version: 1,
            read_only: true,
            applications: vec![],
        };

        let selection = draft(&report);
        let errors = validate(&report, &selection);

        assert!(!errors.is_empty());
    }

    #[test]
    fn rejects_unknown_application() {
        let report = BackupReport {
            schema_version: 1,
            read_only: true,
            applications: vec![],
        };

        let selection = BackupSelection {
            schema_version: 1,
            applications: vec![ApplicationSelection {
                application: "unknown".to_string(),
                sources: vec![],
                databases: vec![],
            }],
        };

        let errors = validate(&report, &selection);
        assert!(errors.iter().any(|e| e.contains("Unknown application")));
    }
}
