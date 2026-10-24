//! ContainerSafe Native UI. Development preview: no backup or restore operations.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]
use containersafe::servers::{credentials::Secret, model::Server, storage};
use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
mod server_ui;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

slint::include_modules!();

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupReport {
    schema_version: u32,
    read_only: bool,
    applications: Vec<ApplicationPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApplicationPlan {
    application: String,
    status: String,
    containers: Vec<String>,
    sources: Vec<SourcePlan>,
    databases: Vec<DatabasePlan>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SourcePlan {
    path: String,
    mount_type: String,
    destination: String,
    policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DatabasePlan {
    container: String,
    engine: String,
    required_method: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupSelection {
    schema_version: u32,
    applications: Vec<ApplicationSelection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApplicationSelection {
    application: String,
    sources: Vec<SourceSelection>,
    databases: Vec<DatabaseSelection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SourceSelection {
    path: String,
    decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DatabaseSelection {
    container: String,
    decision: String,
}

struct State {
    report: BackupReport,
    selection: BackupSelection,
    active: usize,
    selected_container: Option<String>,
    expanded: BTreeSet<String>,
    dirty: bool,
    busy: bool,
    server: Option<Server>,
    secret: Secret,
}

type SharedState = Arc<Mutex<State>>;

fn empty_report() -> BackupReport {
    BackupReport {
        schema_version: 1,
        read_only: true,
        applications: vec![],
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    serde_json::from_str(&contents)
        .map_err(|error| format!("Invalid JSON in {}: {error}", path.display()))
}

fn draft(report: &BackupReport) -> BackupSelection {
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
                        decision: "pending".into(),
                    })
                    .collect(),
                databases: app
                    .databases
                    .iter()
                    .map(|db| DatabaseSelection {
                        container: db.container.clone(),
                        decision: "pending".into(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

// Keep decisions only for sources that still exist AND have the same backup policy.
// This avoids silently carrying an old approval into a changed backup method.
fn reconcile(
    new_report: &BackupReport,
    old_report: &BackupReport,
    previous: &BackupSelection,
) -> BackupSelection {
    let mut result = draft(new_report);
    for selected_app in &mut result.applications {
        let Some(prev_app) = previous
            .applications
            .iter()
            .find(|app| app.application == selected_app.application)
        else {
            continue;
        };
        let Some(old_app) = old_report
            .applications
            .iter()
            .find(|app| app.application == selected_app.application)
        else {
            continue;
        };
        let Some(new_app) = new_report
            .applications
            .iter()
            .find(|app| app.application == selected_app.application)
        else {
            continue;
        };

        for choice in &mut selected_app.sources {
            let old_source = old_app.sources.iter().find(|s| s.path == choice.path);
            let new_source = new_app.sources.iter().find(|s| s.path == choice.path);
            let prev_choice = prev_app.sources.iter().find(|s| s.path == choice.path);
            if let (Some(old), Some(new), Some(prev)) = (old_source, new_source, prev_choice) {
                if old.policy == new.policy
                    && old.mount_type == new.mount_type
                    && old.destination == new.destination
                {
                    choice.decision = prev.decision.clone();
                }
            }
        }
        for choice in &mut selected_app.databases {
            let old_db = old_app
                .databases
                .iter()
                .find(|db| db.container == choice.container);
            let new_db = new_app
                .databases
                .iter()
                .find(|db| db.container == choice.container);
            let prev_choice = prev_app
                .databases
                .iter()
                .find(|db| db.container == choice.container);
            if let (Some(old), Some(new), Some(prev)) = (old_db, new_db, prev_choice) {
                if old.engine == new.engine && old.required_method == new.required_method {
                    choice.decision = prev.decision.clone();
                }
            }
        }
    }
    result
}

fn load_state<F: FnMut(i32, &'static str)>(root: &Path, mut progress: F) -> (State, String) {
    progress(22, "Loading server configuration...");
    let message = match storage::Profiles::load(root) {
        Ok(_) => "Disconnected - select a Docker server".into(),
        Err(e) => e,
    };
    let report = empty_report();
    let selection = draft(&report);
    progress(70, "Preparing disconnected workspace...");
    (
        State {
            report,
            selection,
            active: 0,
            selected_container: None,
            expanded: BTreeSet::from(["compose".into(), "standalone".into()]),
            dirty: false,
            busy: false,
            server: None,
            secret: Secret::default(),
        },
        message,
    )
}

fn source_label(path: &str) -> String {
    path.rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn display_decision(value: &str) -> &'static str {
    match value {
        "include" => "Include",
        "exclude" => "Exclude",
        "logical_dump" => "Logical dump",
        _ => "Pending",
    }
}

fn next_source_decision(current: &str, policy: &str) -> &'static str {
    if matches!(
        policy,
        "requires_logical_dump" | "requires_persistence_review"
    ) {
        if current == "exclude" {
            "pending"
        } else {
            "exclude"
        }
    } else {
        match current {
            "pending" => "include",
            "include" => "exclude",
            _ => "pending",
        }
    }
}

fn next_database_decision(current: &str, engine: &str) -> &'static str {
    if engine == "PostgreSQL" {
        match current {
            "pending" => "logical_dump",
            "logical_dump" => "exclude",
            _ => "pending",
        }
    } else if current == "exclude" {
        "pending"
    } else {
        "exclude"
    }
}

fn selected_count(state: &State) -> usize {
    state
        .selection
        .applications
        .iter()
        .map(|app| {
            app.sources
                .iter()
                .filter(|source| source.decision == "include")
                .count()
                + app
                    .databases
                    .iter()
                    .filter(|db| db.decision == "logical_dump")
                    .count()
        })
        .sum()
}

fn pending_count(state: &State) -> usize {
    state
        .selection
        .applications
        .iter()
        .map(|app| {
            app.sources
                .iter()
                .filter(|s| s.decision == "pending")
                .count()
                + app
                    .databases
                    .iter()
                    .filter(|db| db.decision == "pending")
                    .count()
        })
        .sum()
}

fn decision_values(database: bool, policy: &str) -> Vec<&'static str> {
    let mut values = vec!["pending"];
    loop {
        let current = values.last().copied().unwrap_or("pending");
        let next = if database {
            next_database_decision(current, policy)
        } else {
            next_source_decision(current, policy)
        };
        if values.contains(&next) {
            break;
        }
        values.push(next);
    }
    values
}
fn decision_options(database: bool, policy: &str) -> ModelRc<slint::SharedString> {
    ModelRc::new(VecModel::from(
        decision_values(database, policy)
            .iter()
            .map(|value| display_decision(value).into())
            .collect::<Vec<_>>(),
    ))
}

fn container_tree(
    report: &BackupReport,
    expanded: &BTreeSet<String>,
    filter: &str,
    active: usize,
    selected: Option<&str>,
) -> Vec<TreeEntry> {
    let filter = filter.trim().to_lowercase();
    let mut rows = Vec::new();
    for (group, compose) in [("compose", true), ("standalone", false)] {
        let mut projects: Vec<_> = report
            .applications
            .iter()
            .enumerate()
            .filter(|(_, app)| {
                app.application.starts_with("compose/") == compose
                    && (filter.is_empty()
                        || app.application.to_lowercase().contains(&filter)
                        || app
                            .containers
                            .iter()
                            .any(|name| name.to_lowercase().contains(&filter)))
            })
            .collect();
        projects.sort_by_key(|(_, app)| app.application.to_lowercase());
        if projects.is_empty() {
            continue;
        }
        let is_open = !filter.is_empty() || expanded.contains(group);
        rows.push(TreeEntry {
            key: group.into(),
            name: if compose { "Compose" } else { "Standalone" }.into(),
            note: projects
                .iter()
                .map(|(_, app)| app.containers.len())
                .sum::<usize>()
                .to_string()
                .into(),
            depth: 0,
            branch: true,
            expanded: is_open,
            selected: false,
            app_index: -1,
            container: "".into(),
        });
        if !is_open {
            continue;
        }
        for (index, app) in projects {
            let key = format!("project:{}", app.application);
            let is_open = !filter.is_empty() || expanded.contains(&key);
            if compose {
                rows.push(TreeEntry {
                    key: key.into(),
                    name: app
                        .application
                        .strip_prefix("compose/")
                        .unwrap_or(&app.application)
                        .into(),
                    note: app.containers.len().to_string().into(),
                    depth: 1,
                    branch: true,
                    expanded: is_open,
                    selected: active == index && selected.is_none(),
                    app_index: index as i32,
                    container: "".into(),
                });
                if !is_open {
                    continue;
                }
            }
            let project_match = app.application.to_lowercase().contains(&filter);
            let mut containers: Vec<_> = app
                .containers
                .iter()
                .filter(|name| {
                    filter.is_empty() || project_match || name.to_lowercase().contains(&filter)
                })
                .collect();
            containers.sort_by_key(|name| name.to_lowercase());
            for name in containers {
                rows.push(TreeEntry {
                    key: format!("container:{}:{name}", app.application).into(),
                    name: name.trim_start_matches('/').into(),
                    note: "".into(),
                    depth: if compose { 2 } else { 1 },
                    branch: false,
                    expanded: false,
                    selected: active == index
                        && (selected == Some(name.as_str()) || (!compose && selected.is_none())),
                    app_index: index as i32,
                    container: name.as_str().into(),
                });
            }
        }
    }
    rows
}

fn render(ui: &ContainerSafeWindow, state: &State) {
    let plans = plan_overview(&state.report, &state.selection);
    ui.set_selected_operations(plans.iter().map(|plan| plan.selected).sum());
    ui.set_pending_decisions(plans.iter().map(|plan| plan.pending).sum());
    ui.set_excluded_decisions(plans.iter().map(|plan| plan.excluded).sum());
    ui.set_plan_model(ModelRc::new(VecModel::from(plans)));
    let tree = container_tree(
        &state.report,
        &state.expanded,
        ui.get_filter_text().as_str(),
        state.active,
        state.selected_container.as_deref(),
    );
    let current_tree = ui.get_tree_model();
    if current_tree.row_count() == tree.len()
        && tree.iter().enumerate().all(|(i, row)| {
            current_tree
                .row_data(i)
                .is_some_and(|current| current.key == row.key)
        })
    {
        if let Some(model) = current_tree.as_any().downcast_ref::<VecModel<TreeEntry>>() {
            for (i, row) in tree.into_iter().enumerate() {
                model.set_row_data(i, row);
            }
        } else {
            ui.set_tree_model(ModelRc::new(VecModel::from(tree)));
        }
    } else {
        ui.set_tree_model(ModelRc::new(VecModel::from(tree)));
    }
    ui.set_total_apps(state.report.applications.len() as i32);
    ui.set_total_containers(
        state
            .report
            .applications
            .iter()
            .map(|a| a.containers.len())
            .sum::<usize>() as i32,
    );
    ui.set_total_sources(
        state
            .report
            .applications
            .iter()
            .map(|a| a.sources.len())
            .sum::<usize>() as i32,
    );
    ui.set_total_dbs(
        state
            .report
            .applications
            .iter()
            .map(|a| a.databases.len())
            .sum::<usize>() as i32,
    );
    ui.set_dirty(state.dirty);
    ui.set_loading(state.busy);

    let Some(app) = state.report.applications.get(state.active) else {
        ui.set_active_index(-1);
        ui.set_entries(ModelRc::new(VecModel::from(Vec::<DataEntry>::new())));
        ui.set_selected_name(
            if state.server.is_none() {
                "No server connected"
            } else {
                "No containers detected"
            }
            .into(),
        );
        ui.set_selected_summary(
            if state.server.is_none() {
                "Connect a Docker server from File > New Server or Servers"
            } else {
                "Refresh to query the selected server"
            }
            .into(),
        );
        ui.set_selected_warning("Backup execution is disabled".into());
        return;
    };
    ui.set_active_index(state.active as i32);
    let project = app
        .application
        .split_once('/')
        .map(|(_, name)| name)
        .unwrap_or(&app.application);
    let container = state
        .selected_container
        .as_deref()
        .filter(|name| app.containers.iter().any(|c| c == name));
    ui.set_selected_name(container.unwrap_or(project).trim_start_matches('/').into());
    ui.set_selected_summary(
        format!(
            "{} · {} containers · {} storage sources · {} databases",
            if app.application.starts_with("compose/") {
                format!("Compose / {project} · project-wide storage")
            } else {
                "Standalone container".into()
            },
            app.containers.len(),
            app.sources.len(),
            app.databases.len()
        )
        .into(),
    );
    ui.set_selected_warning(
        format!(
            "{} items need recovery review · no backup created",
            app.warnings.len()
        )
        .into(),
    );
    let choice = state.selection.applications.get(state.active);
    let mut entries: Vec<DataEntry> = Vec::new();
    for (source_index, source) in app.sources.iter().enumerate() {
        let decision = choice
            .and_then(|c| c.sources.iter().find(|s| s.path == source.path))
            .map(|s| s.decision.as_str())
            .unwrap_or("pending");
        let protected = matches!(
            source.policy.as_str(),
            "requires_logical_dump" | "requires_persistence_review"
        );
        let detail = match source.policy.as_str() {
            "requires_logical_dump" => "Raw DB backup prohibited · select DB logical dump",
            "requires_persistence_review" => "Persistent state requires manual review",
            "large_data_requires_opt_in" => "Large storage · explicit opt-in",
            _ => "Candidate persistent source · review before backup",
        };
        entries.push(DataEntry {
            row_index: source_index as i32,
            title: source_label(&source.path).into(),
            location: source.path.as_str().into(),
            detail: format!("{}  ·  {}", detail, source.destination).into(),
            decision: display_decision(decision).into(),
            options: decision_options(false, &source.policy),
            choice: decision_values(false, &source.policy)
                .iter()
                .position(|value| *value == decision)
                .unwrap_or(0) as i32,
            database: false,
            caution: protected,
        });
    }
    for (database_index, db) in app.databases.iter().enumerate() {
        let decision = choice
            .and_then(|c| c.databases.iter().find(|d| d.container == db.container))
            .map(|d| d.decision.as_str())
            .unwrap_or("pending");
        entries.push(DataEntry {
            row_index: (app.sources.len() + database_index) as i32,
            title: db.container.as_str().into(),
            location: db.engine.as_str().into(),
            detail: format!("Adapter: {} · not executable yet", db.required_method).into(),
            decision: display_decision(decision).into(),
            options: decision_options(true, &db.engine),
            choice: decision_values(true, &db.engine)
                .iter()
                .position(|value| *value == decision)
                .unwrap_or(0) as i32,
            database: true,
            caution: db.engine != "PostgreSQL",
        });
    }
    let kind = ui.get_entry_kind();
    entries.retain(|entry| kind == 0 || (kind == 2) == entry.database);
    ui.set_entries(ModelRc::new(VecModel::from(entries)));
}

fn plan_overview(report: &BackupReport, selection: &BackupSelection) -> Vec<PlanEntry> {
    report
        .applications
        .iter()
        .enumerate()
        .map(|(index, app)| {
            let selected = selection
                .applications
                .iter()
                .find(|selected| selected.application == app.application);
            let decisions: Vec<&str> = app
                .sources
                .iter()
                .map(|source| {
                    selected
                        .and_then(|selected| {
                            selected.sources.iter().find(|s| s.path == source.path)
                        })
                        .map(|s| s.decision.as_str())
                        .unwrap_or("pending")
                })
                .chain(app.databases.iter().map(|db| {
                    selected
                        .and_then(|selected| {
                            selected
                                .databases
                                .iter()
                                .find(|d| d.container == db.container)
                        })
                        .map(|d| d.decision.as_str())
                        .unwrap_or("pending")
                }))
                .collect();
            PlanEntry {
                name: app
                    .application
                    .split_once('/')
                    .map(|(_, name)| name)
                    .unwrap_or(&app.application)
                    .into(),
                kind: if app.application.starts_with("compose/") {
                    "Compose"
                } else {
                    "Standalone"
                }
                .into(),
                containers: app.containers.len() as i32,
                selected: decisions
                    .iter()
                    .filter(|value| matches!(**value, "include" | "logical_dump"))
                    .count() as i32,
                excluded: decisions
                    .iter()
                    .filter(|value| **value == "exclude")
                    .count() as i32,
                pending: decisions
                    .iter()
                    .filter(|value| **value == "pending")
                    .count() as i32,
                app_index: index as i32,
            }
        })
        .collect()
}

fn validate_saved(root: &Path) -> Result<(bool, String), String> {
    let report: containersafe::plan::BackupReport = read_json(&root.join("backup-plan.json"))?;
    let selection: containersafe::selection::BackupSelection =
        read_json(&root.join("backup-selection.json"))?;
    let errors = containersafe::selection::validate(&report, &selection);
    Ok((
        errors.is_empty(),
        if errors.is_empty() {
            "Selection valid; backup execution NOT authorized".into()
        } else {
            errors.join("; ")
        },
    ))
}

// This runs only on the Slint UI thread, after background startup completes.
fn create_main_window(
    root: PathBuf,
    initial: State,
    first_status: String,
    preview: bool,
) -> Result<ContainerSafeWindow, Box<dyn std::error::Error>> {
    let state: SharedState = Arc::new(Mutex::new(initial));
    let ui = ContainerSafeWindow::new()?;
    {
        let guard = state.lock().expect("state mutex poisoned");
        render(&ui, &guard);
    }
    ui.set_app_version(containersafe::VERSION.into());
    ui.set_ssh_key_compatibility(containersafe::servers::model::SSH_KEY_COMPATIBILITY.into());
    ui.set_status_message(first_status.into());
    ui.set_validation_message("Selection not verified for execution".into());

    {
        let weak = ui.as_weak();
        let state = Arc::clone(&state);
        ui.on_select_node(move |index, container| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Ok(mut guard) = state.lock() else {
                return;
            };
            if ui.get_dialog() != 0 {
                return;
            }
            if index < 0 || index as usize >= guard.report.applications.len() {
                return;
            }
            if !container.is_empty()
                && !guard.report.applications[index as usize]
                    .containers
                    .iter()
                    .any(|name| name == container.as_str())
            {
                return;
            }
            guard.active = index as usize;
            guard.selected_container = (!container.is_empty()).then(|| container.to_string());
            let app = &guard.report.applications[index as usize];
            if app.application.starts_with("compose/") {
                let key = format!("project:{}", app.application);
                guard.expanded.insert(key);
            }
            render(&ui, &guard);
        });
    }

    {
        let weak = ui.as_weak();
        let state = Arc::clone(&state);
        ui.on_choose_decision(move |index, selected| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Ok(mut guard) = state.lock() else {
                return;
            };
            if guard.busy || ui.get_dialog() != 0 {
                return;
            }
            let active = guard.active;
            let Some(plan_app) = guard.report.applications.get(active).cloned() else {
                return;
            };
            let Some(choices) = guard.selection.applications.get_mut(active) else {
                return;
            };
            if index < 0 || selected < 0 {
                return;
            }
            let i = index as usize;
            if i < plan_app.sources.len() {
                if let (Some(source), Some(choice)) =
                    (plan_app.sources.get(i), choices.sources.get_mut(i))
                {
                    let values = decision_values(false, &source.policy);
                    let Some(value) = values.get(selected as usize) else {
                        return;
                    };
                    choice.decision = (*value).into();
                }
            } else {
                let db_index = i - plan_app.sources.len();
                if let (Some(db), Some(choice)) = (
                    plan_app.databases.get(db_index),
                    choices.databases.get_mut(db_index),
                ) {
                    let values = decision_values(true, &db.engine);
                    let Some(value) = values.get(selected as usize) else {
                        return;
                    };
                    choice.decision = (*value).into();
                }
            }
            guard.dirty = true;
            render(&ui, &guard);
            ui.set_status_message("Unsaved selection changes".into());
            ui.set_validation_message("Validation needed".into());
        });
    }

    {
        let weak = ui.as_weak();
        let state = Arc::clone(&state);
        let root = root.clone();
        ui.on_save_selection(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Ok(mut guard) = state.lock() else {
                return;
            };
            if guard.busy || ui.get_dialog() != 0 {
                return;
            }
            let Some(server) = &guard.server else {
                return;
            };
            let server_root = storage::server_dir(&root, server.id);
            let save_result = serde_json::to_vec_pretty(&guard.selection)
                .map_err(|error| error.to_string())
                .and_then(|data| {
                    storage::atomic_write(&server_root.join("backup-selection.json"), &data)
                        .map_err(|error| error.to_string())
                });
            match save_result {
                Ok(()) => {
                    guard.dirty = false;
                    render(&ui, &guard);
                    ui.set_status_message(
                        format!(
                            "Selection saved · {} selected operations · {} pending",
                            selected_count(&guard),
                            pending_count(&guard)
                        )
                        .into(),
                    );
                    ui.set_validation_message("Saved; requires validation".into());
                }
                Err(error) => ui.set_status_message(format!("SAVE ERROR: {error}").into()),
            }
        });
    }

    {
        let weak = ui.as_weak();
        let state = Arc::clone(&state);
        let root = root.clone();
        ui.on_validate_selection(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            {
                let Ok(mut guard) = state.lock() else {
                    return;
                };
                if guard.busy || ui.get_dialog() != 0 {
                    return;
                }
                if guard.dirty {
                    ui.set_status_message("Save changes before validation".into());
                    return;
                }
                if guard.server.is_none() {
                    return;
                }
                guard.busy = true;
                render(&ui, &guard);
                ui.set_status_message("Validating selection with existing Rust CLI...".into());
            }
            let weak = weak.clone();
            let state = Arc::clone(&state);
            let Some(id) = state
                .lock()
                .ok()
                .and_then(|s| s.server.as_ref().map(|s| s.id))
            else {
                return;
            };
            let root = storage::server_dir(&root, id);
            std::thread::spawn(move || {
                let result = validate_saved(&root);
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    let Ok(mut guard) = state.lock() else {
                        return;
                    };
                    guard.busy = false;
                    render(&ui, &guard);
                    match result {
                        Ok((true, info)) => {
                            ui.set_status_message(
                                format!("{info}. Restore and preflight still required.").into(),
                            );
                            ui.set_validation_message(
                                "Selection valid · no backup authorized".into(),
                            );
                        }
                        Ok((false, info)) => {
                            ui.set_status_message(info.into());
                            ui.set_validation_message("Needs review".into());
                        }
                        Err(error) => {
                            ui.set_status_message(format!("VALIDATE ERROR: {error}").into())
                        }
                    }
                });
            });
        });
    }

    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_toggle_node(move |key| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Ok(mut state) = state.lock() else {
                return;
            };
            if ui.get_dialog() != 0 {
                return;
            }
            if !state.expanded.remove(key.as_str()) {
                state.expanded.insert(key.to_string());
            }
            render(&ui, &state);
        });
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_filter_changed(move || {
            if let (Some(ui), Ok(state)) = (weak.upgrade(), state.lock()) {
                render(&ui, &state);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        ui.on_filter_entries(move || {
            if let (Some(ui), Ok(state)) = (weak.upgrade(), state.lock()) {
                render(&ui, &state);
            }
        });
    }
    if !preview {
        server_ui::install(&ui, state, root);
    }
    Ok(ui)
}

// Progress is based on completed operations, not on an artificial counter.
enum StartupEvent {
    Progress(i32, String),
    Loaded(Box<State>, String),
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    // Explicit development-only visual fixture. Never read profiles or connect
    // to a server in preview mode; its configuration lives in a disposable dir.
    #[cfg(debug_assertions)]
    if arguments
        .first()
        .is_some_and(|arg| arg == "--preview-report")
    {
        if arguments.len() != 2 {
            return Err(
                "Usage: containersafe-native --preview-report FILE (debug builds only)".into(),
            );
        }
        let path = &arguments[1];
        let report: BackupReport = read_json(Path::new(path))?;
        let directory = tempfile::tempdir()?;
        let selection = draft(&report);
        let state = State {
            report,
            selection,
            active: 0,
            selected_container: None,
            expanded: BTreeSet::from(["compose".into(), "standalone".into()]),
            dirty: false,
            busy: false,
            server: None,
            secret: Secret::default(),
        };
        let ui = create_main_window(
            directory.path().to_owned(),
            state,
            "Visual preview · synthetic fixture data".into(),
            true,
        )?;
        ui.set_active_server("Local preview · no server connection".into());
        return ui.run().map_err(Into::into);
    }
    if !arguments.is_empty() {
        return Err(
            "Unknown arguments; --preview-report FILE is available only in debug builds".into(),
        );
    }
    let root = storage::config_root().map_err(std::io::Error::other)?;
    let splash = SplashWindow::new()?;
    splash.set_loading_progress(5);
    splash.set_loading_message("Pornesc ContainerSafe...".into());
    splash.show()?;

    // All disk I/O is performed off the UI thread.
    let (sender, receiver) = mpsc::channel::<StartupEvent>();
    let worker_root = root.clone();
    std::thread::spawn(move || {
        let (state, message) = load_state(&worker_root, |percent, text| {
            let _ = sender.send(StartupEvent::Progress(percent, text.into()));
        });
        let _ = sender.send(StartupEvent::Loaded(Box::new(state), message));
    });

    // Slint components must be created and modified on the UI thread.
    let main_window: Rc<RefCell<Option<ContainerSafeWindow>>> = Rc::new(RefCell::new(None));
    let keep_main = Rc::clone(&main_window);
    let splash_weak = splash.as_weak();
    let mut ready_at: Option<Instant> = None;
    let mut loaded: Option<(State, String)> = None;
    let mut finished = false;

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(60), move || {
        if finished {
            return;
        }
        while let Ok(event) = receiver.try_recv() {
            let Some(splash) = splash_weak.upgrade() else {
                return;
            };
            match event {
                StartupEvent::Progress(percent, message) => {
                    splash.set_loading_progress(percent);
                    splash.set_loading_message(message.into());
                }
                StartupEvent::Loaded(state, message) => {
                    splash.set_loading_progress(100);
                    splash.set_loading_message("Gata".into());
                    ready_at = Some(Instant::now());
                    loaded = Some((*state, message));
                }
            }
        }

        // Keep the completed splash visible for 2.5 seconds after loading,
        // without blocking the event loop or delaying disk I/O.
        if !ready_at.is_some_and(|ready| ready.elapsed() >= Duration::from_millis(2500)) {
            return;
        }

        if let Some((state, message)) = loaded.take() {
            let Some(splash) = splash_weak.upgrade() else {
                return;
            };
            match create_main_window(root.clone(), state, message, false) {
                Ok(ui) => match ui.show() {
                    Ok(()) => {
                        *keep_main.borrow_mut() = Some(ui);
                        splash.set_loading_progress(100);
                        splash.set_loading_message("Gata".into());
                        let _ = splash.hide();
                        finished = true;
                    }
                    Err(error) => {
                        splash
                            .set_loading_message(format!("Nu pot afisa fereastra: {error}").into());
                    }
                },
                Err(error) => {
                    splash.set_loading_message(format!("Eroare la initializare: {error}").into());
                }
            }
        }
    });

    // Both windows share a single native Slint event loop.
    // When the main window is closed, the event loop ends.
    slint::run_event_loop()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_preserves_original_indices_and_search_exposes_only_matching_members() {
        let report: BackupReport =
            serde_json::from_str(include_str!("../tests/fixtures/ui-report.json")).unwrap();
        let mut expanded = BTreeSet::from(["compose".into(), "standalone".into()]);
        let closed = container_tree(&report, &expanded, "", 0, None);
        assert_eq!(closed.iter().filter(|row| row.depth == 0).count(), 2);
        assert!(!closed.iter().any(|row| row.name == "immich_postgres"));
        expanded.insert("project:compose/immich".into());
        let opened = container_tree(&report, &expanded, "", 0, Some("immich_postgres"));
        let leaf = opened
            .iter()
            .find(|row| row.name == "immich_postgres")
            .unwrap();
        assert!(leaf.selected);
        assert_eq!(leaf.app_index, 0);
        assert_eq!(leaf.depth, 2);
        let standalone = opened.iter().find(|row| row.name == "vaultwarden").unwrap();
        assert_eq!(standalone.app_index, 10);
        assert_eq!(standalone.depth, 1);
        let filtered = container_tree(&report, &BTreeSet::new(), "immich_postgres", 0, None);
        assert_eq!(filtered.iter().filter(|row| !row.branch).count(), 1);
        assert_eq!(filtered.last().unwrap().container, "immich_postgres");
        assert!(container_tree(&report, &expanded, "nonexistent-name", 0, None).is_empty());
    }

    #[test]
    fn plan_overview_counts_decisions_by_identity_and_respects_protected_choices() {
        let report: BackupReport =
            serde_json::from_str(include_str!("../tests/fixtures/ui-report.json")).unwrap();
        let mut selection = draft(&report);
        selection.applications[0].sources[0].decision = "include".into();
        selection.applications[0].databases[0].decision = "logical_dump".into();
        selection.applications[0].databases[1].decision = "exclude".into();
        selection.applications.reverse();
        let plans = plan_overview(&report, &selection);
        assert_eq!(plans[0].selected, 2);
        assert_eq!(plans[0].excluded, 1);
        assert_eq!(plans[0].pending, 1);
        assert_eq!(
            decision_values(false, "requires_logical_dump"),
            ["pending", "exclude"]
        );
        assert_eq!(
            decision_values(true, "PostgreSQL"),
            ["pending", "logical_dump", "exclude"]
        );
        assert_eq!(
            decision_values(true, "Redis/Valkey"),
            ["pending", "exclude"]
        );
    }

    #[test]
    fn raw_postgres_volume_may_not_be_included() {
        assert_eq!(
            next_source_decision("pending", "requires_logical_dump"),
            "exclude"
        );
        assert_eq!(
            next_source_decision("exclude", "requires_logical_dump"),
            "pending"
        );
    }

    #[test]
    fn postgres_dump_is_selectable_but_redis_dump_is_not() {
        assert_eq!(
            next_database_decision("pending", "PostgreSQL"),
            "logical_dump"
        );
        assert_eq!(next_database_decision("pending", "Redis/Valkey"), "exclude");
    }

    #[test]
    fn draft_contains_every_source_and_database() {
        let report = BackupReport {
            schema_version: 1,
            read_only: true,
            applications: vec![ApplicationPlan {
                application: "compose/test".into(),
                status: "needs_review".into(),
                containers: vec!["example".into()],
                sources: vec![SourcePlan {
                    path: "/test".into(),
                    mount_type: "bind".into(),
                    destination: "/data".into(),
                    policy: "review_for_backup".into(),
                }],
                databases: vec![],
                warnings: vec![],
            }],
        };
        assert_eq!(
            draft(&report).applications[0].sources[0].decision,
            "pending"
        );
    }
}
