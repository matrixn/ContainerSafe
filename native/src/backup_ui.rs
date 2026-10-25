//! Manual, server-bound file backups. Repository secrets never enter the UI.
use super::*;
use containersafe::{
    agent::{Action, AgentConfig, JobConfig, Operation},
    backup::{self, AgentEndpoint, Destination, DestinationKind, JobDescription},
    restic::{BackupResult, CheckResult, RepositoryConfig, RestoreResult, SnapshotInfo},
    servers::connection::{Connection, Failure},
};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanConfig {
    #[serde(default)]
    use_agent: bool,
    destination: Destination,
    endpoint: AgentEndpoint,
    restic: String,
    password_file: String,
    restore_root: String,
    secret_reference: Uuid,
}
impl PlanConfig {
    fn new(destination: Destination) -> Self {
        let mut endpoint = AgentEndpoint::default();
        endpoint.config = format!(
            "/home/backup/.config/containersafe/{}.json",
            endpoint.job_id
        );
        Self {
            use_agent: false,
            destination,
            password_file: format!(
                "/home/backup/.config/containersafe/{}.password",
                endpoint.job_id
            ),
            restore_root: "/home/backup/containersafe-restores".into(),
            endpoint,
            restic: "/usr/local/bin/restic".into(),
            secret_reference: Uuid::new_v4(),
        }
    }
    fn validate(&self) -> Result<(), String> {
        self.destination.validate()?;
        let mut paths = vec![&self.restic, &self.password_file, &self.restore_root];
        if self.use_agent {
            paths.extend([&self.endpoint.executable, &self.endpoint.config]);
        }
        for path in paths {
            if !path.starts_with('/') || path.len() > 4096 || path.contains(['\0', '\r', '\n']) {
                return Err(i18n::text("Backup paths must be absolute Linux paths"));
            }
        }
        if self.endpoint.job_id.is_nil() || self.secret_reference.is_nil() {
            return Err("Invalid backup identity".into());
        }
        Ok(())
    }
}
type Plans = BTreeMap<String, PlanConfig>;
fn load<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T, String> {
    match fs::read(path) {
        Ok(data) => serde_json::from_slice(&data).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.to_string()),
    }
}
fn save<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    storage::atomic_write(
        path,
        &serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
}
fn kind(index: i32) -> DestinationKind {
    match index {
        1 => DestinationKind::ThisComputer,
        2 => DestinationKind::Remote,
        _ => DestinationKind::ServerFolder,
    }
}
fn index(kind: DestinationKind) -> i32 {
    match kind {
        DestinationKind::ServerFolder => 0,
        DestinationKind::ThisComputer => 1,
        DestinationKind::Remote => 2,
    }
}
fn show_result(ui: &ContainerSafeWindow, result: Result<String, String>) {
    ui.set_backup_error(result.is_err());
    ui.set_backup_result(
        match result {
            Ok(message) | Err(message) => message,
        }
        .into(),
    );
}

/// Build from approved sources in the current inventory. Never infer approval
/// for a new path or allow a raw database directory as a file source.
fn approved_sources(
    app: &ApplicationPlan,
    selection: &ApplicationSelection,
) -> Result<Vec<PathBuf>, String> {
    if selection
        .databases
        .iter()
        .any(|db| db.decision == "logical_dump")
    {
        return Err(i18n::text("Database dumps are not implemented yet. No backup was started. Choose a file-only plan or disable Logical dump explicitly."));
    }
    let mut sources = BTreeSet::new();
    for selected in selection
        .sources
        .iter()
        .filter(|source| source.decision == "include")
    {
        let source = app
            .sources
            .iter()
            .find(|source| source.path == selected.path)
            .ok_or_else(|| i18n::text("Inventory changed. Refresh and review your selections."))?;
        if !matches!(
            source.policy.as_str(),
            "review_for_backup" | "large_data_requires_opt_in"
        ) {
            return Err(i18n::text(
                "Live database volumes cannot be included in a file backup.",
            ));
        }
        sources.insert(PathBuf::from(&source.path));
    }
    if sources.is_empty() {
        return Err(i18n::text("Select at least one file source with Include."));
    }
    Ok(sources.into_iter().collect())
}

#[derive(Clone)]
struct OpenPlan {
    server_id: Uuid,
    application: String,
    config: PlanConfig,
    snapshots: Vec<SnapshotInfo>,
}
fn read_form(ui: &ContainerSafeWindow, previous: &PlanConfig) -> Result<PlanConfig, String> {
    let mut value = previous.clone();
    value.use_agent = ui.get_backup_use_agent();
    value.destination = Destination {
        kind: kind(ui.get_backup_kind()),
        location: ui.get_backup_location().trim().into(),
    };
    value.endpoint.executable = ui.get_backup_executable().trim().into();
    value.endpoint.config = ui.get_backup_config().trim().into();
    value.restic = ui.get_backup_restic().trim().into();
    value.password_file = ui.get_backup_password_file().trim().into();
    value.restore_root = ui.get_backup_restore_root().trim().into();
    value.validate()?;
    Ok(value)
}
fn plan_path(root: &Path, id: Uuid) -> PathBuf {
    storage::server_dir(root, id).join("backup-execution.json")
}

fn export_setup(config: &PlanConfig, sources: &[PathBuf], directory: &Path) -> Result<(), String> {
    if !config.use_agent {
        let folder = directory.join(format!("containersafe-ssh-setup-{}", Uuid::new_v4()));
        fs::create_dir(&folder).map_err(|e| e.to_string())?;
        save(
            &folder.join("backup-plan.json"),
            &serde_json::json!({"transport":"restic-over-ssh",
            "restic":config.restic,"password_file":config.password_file,"restore_root":config.restore_root,
            "destination":config.destination,"approved_sources":sources}),
        )?;
        fs::write(
            folder.join("SETUP.md"),
            format!(
                r#"# Restic directly over SSH — Synology DSM 7.4

No ContainerSafe agent, service or agent.json is required.

1. Install a Linux Restic executable matching your NAS CPU at `{restic}`.
   Use Restic 0.19.1 or a compatible version. Confirm executable permission and `restic version`.
2. Under the SAME SSH account used by ContainerSafe, create a private directory
   for `{password}` (parent mode 700). Create a strong repository password in that
   file (one line, mode 600, regular file, not a symlink). Keep a separate secure copy.
   Do not put the password in shell command arguments, this plan, or source control.
3. Create `{restores}` outside source folders and repository, writable by the SSH user.
   DSM homes commonly live under /var/services/homes/<username>; home service must
   be enabled to use that location. Choose a dedicated writable share if it is disabled.
4. Destination: `{destination}`. For a local NAS repository, its parent must exist
   and be writable. No recursive chmod/chown or sudo policy changes are performed.
5. Check the approved sources in backup-plan.json. The SSH account must be able
   to read the files. Being a DSM administrator or having sudo -n docker access
   does NOT automatically grant Restic read/write access. Direct mode does not use sudo.
6. In ContainerSafe save the plan, initialize the repository ONCE, back up,
   list snapshots, check all data and test an isolated restore.

For SFTP (including backup to this PC), enable an SSH/SFTP server at the destination,
authorize a dedicated key for the NAS SSH user, and independently verify/pin its host
key in that account's known_hosts. Use ~/.ssh/config aliases for ports/key paths.
For S3 HTTPS, configure the NAS SSH user's protected ~/.aws/credentials (mode 600).
Never disable host identity or TLS verification. Credentials are not stored in this export.

Only approved files are backed up. Logical database dumps and full application
consistency are not provided. No running container is stopped or modified.
"#,
                restic = config.restic,
                password = config.password_file,
                restores = config.restore_root,
                destination = config.destination.location
            ),
        )
        .map_err(|e| e.to_string())?;
        return Ok(());
    }
    let agent = AgentConfig {
        restic_executable: config.restic.clone().into(),
        timeout_seconds: 3600,
        jobs: vec![JobConfig {
            id: config.endpoint.job_id,
            repository: RepositoryConfig {
                path: config.destination.location.clone().into(),
                secret_reference: config.secret_reference,
            },
            secret_file: config.password_file.clone().into(),
            sources: sources.to_vec(),
            restore_root: config.restore_root.clone().into(),
            allowed_operations: vec![
                Operation::Init,
                Operation::Backup,
                Operation::Snapshots,
                Operation::Check,
                Operation::Restore,
            ],
        }],
    };
    // Export to a newly created subfolder, never overwrite an existing setup.
    let directory = directory.join(format!("containersafe-setup-{}", Uuid::new_v4()));
    fs::create_dir(&directory).map_err(|e| e.to_string())?;
    save(&directory.join("agent.json"), &agent)?;
    fs::write(
        directory.join("SETUP.md"),
        format!(
            r#"# ContainerSafe manual backup setup

This contains no passwords. Review all paths before installing it on the Docker server.

1. Build/install the Linux agent {version} as `{executable}` and Restic >= 0.19.1 as `{restic}`.
   From a Linux checkout: `cargo build --locked --no-default-features --bin containersafe-agent`.
2. Run the agent as the SSH user. Grant that user read access ONLY to approved sources.
   Do not run the agent with sudo/root to work around permissions.
3. Copy `agent.json` to `{config}`. Create its parent directory privately (mode 700),
   and chmod the configuration 600. Do not overwrite another job's configuration.
4. Create a strong unique repository password in `{password}` (one line, chmod 600,
   owned by the SSH user). Keep a secure independent copy: losing it loses the backup.
   ContainerSafe does not generate, upload or remember this password.
5. Create `{restores}` as a private directory OUTSIDE selected sources and repository.
6. Destination: `{destination}`. For a server folder, create its parent with sufficient free space.
   For This computer or remote SFTP: enable SSH/SFTP at the destination, authorize a dedicated
   key for the agent user, and verify/pin the destination host key in that user's known_hosts.
   Configure port and IdentityFile in that user's ~/.ssh/config using a host alias if needed.
   Host checking is strict and interactive password prompts are disabled.
   For S3: create the bucket and configure ~/.aws/credentials (chmod 600) for the agent user,
   or its existing secure execution environment. HTTPS certificate checks stay enabled.
7. Back in ContainerSafe: Save plan, Initialize repository ONCE, Back up now,
   List snapshots, Check all data, then select an exact snapshot and Test restore.

Restores go to NEW directories under `{restores}` on the Docker server. No live data is replaced.
File backups are not guaranteed to be consistent while applications write. SQLite and external
databases require application-specific procedures. PostgreSQL/MySQL/Redis dumps, Compose recovery,
scheduling and retention are not implemented by this file backup workflow.
Backups on the same disk are not protection against loss of that disk.
"#,
            version = containersafe::VERSION,
            executable = config.endpoint.executable,
            restic = config.restic,
            config = config.endpoint.config,
            password = config.password_file,
            restores = config.restore_root,
            destination = config.destination.location
        ),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn activity(root: &Path, id: Uuid) -> Result<String, String> {
    let records: Vec<String> = load(&storage::server_dir(root, id).join("backup-activity.json"))?;
    if records.is_empty() {
        Ok(i18n::text("No backup activity yet"))
    } else {
        Ok(records.join("\n\n"))
    }
}
pub(super) fn refresh_activity(ui: &ContainerSafeWindow, root: &Path, id: Uuid) {
    ui.set_backup_activity(activity(root, id).unwrap_or_else(|error| error).into());
}
fn record(
    root: &Path,
    id: Uuid,
    application: &str,
    operation: &str,
    result: &Result<String, String>,
) -> Result<(), String> {
    let path = storage::server_dir(root, id).join("backup-activity.json");
    let mut records: Vec<String> = load(&path)?;
    let (status, message) = match result {
        Ok(message) => ("OK", message),
        Err(message) => ("ERROR", message),
    };
    records.insert(
        0,
        format!(
            "{} · {application} · {operation} · {status}\n{message}",
            chrono::DateTime::<chrono::Utc>::from_timestamp(
                containersafe::servers::model::now() as i64,
                0
            )
            .map(|value| value.to_rfc3339())
            .unwrap_or_default()
        ),
    );
    records.truncate(50);
    save(&path, &records)
}

pub(super) fn install(ui: &ContainerSafeWindow, root: PathBuf, state: SharedState) {
    let default_path = root.join("backup-default.json");
    let defaults: Result<Destination, String> = load(&default_path);
    if let Ok(defaults) = &defaults {
        ui.set_backup_default_kind(index(defaults.kind));
        ui.set_backup_default_location(defaults.location.clone().into());
    } else {
        ui.set_backup_default_status(
            i18n::text("Could not load backup defaults. Original file preserved.").into(),
        );
    }
    let editable = defaults.is_ok();
    let weak = ui.as_weak();
    ui.on_backup_default_edited(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if !editable {
            return;
        }
        let destination = Destination {
            kind: kind(ui.get_backup_default_kind()),
            location: ui.get_backup_default_location().to_string(),
        };
        if !destination.location.is_empty() && destination.validate().is_err() {
            ui.set_backup_default_status(
                i18n::text("Destination incomplete or invalid; not saved yet.").into(),
            );
            return;
        }
        let result = save(&default_path, &destination);
        ui.set_backup_default_status(
            i18n::text(if result.is_ok() {
                "Saved automatically"
            } else {
                "Could not save settings:"
            })
            .into(),
        );
    });

    let opened: Rc<RefCell<Option<OpenPlan>>> = Rc::new(RefCell::new(None));
    {
        let weak = ui.as_weak();
        let state = state.clone();
        let root = root.clone();
        let opened = opened.clone();
        ui.on_open_backup(move |index| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Ok(guard) = state.lock() else {
                return;
            };
            if guard.busy || ui.get_dialog() != 0 {
                return;
            }
            let Some(server) = &guard.server else {
                return;
            };
            let Some(app) = guard.report.applications.get(index.max(0) as usize) else {
                return;
            };
            let Some(selection) = guard
                .selection
                .applications
                .iter()
                .find(|selection| selection.application == app.application)
            else {
                return;
            };
            let sources = approved_sources(app, selection);
            let configs: Plans = match load(&plan_path(&root, server.id)) {
                Ok(value) => value,
                Err(error) => {
                    set_status_message(&ui, error.into(), true);
                    return;
                }
            };
            let config = configs.get(&app.application).cloned().unwrap_or_else(|| {
                let mut destination =
                    load::<Destination>(&root.join("backup-default.json")).unwrap_or_default();
                if !destination.location.is_empty() {
                    // Separate repositories per plan; do not mix container backups.
                    let suffix = app
                        .application
                        .bytes()
                        .map(|c| {
                            if c.is_ascii_alphanumeric() || c == b'-' || c == b'_' {
                                c as char
                            } else {
                                '_'
                            }
                        })
                        .collect::<String>();
                    destination.location = format!(
                        "{}/{}-{}-{}",
                        destination.location.trim_end_matches('/'),
                        server.id,
                        suffix,
                        Uuid::new_v4()
                    );
                }
                let mut config = PlanConfig::new(destination);
                let home = format!("/var/services/homes/{}", server.username);
                config.password_file = format!(
                    "{home}/.config/containersafe/{}.password",
                    config.endpoint.job_id
                );
                config.restore_root = format!("{home}/containersafe-restores");
                config
            });
            ui.set_backup_use_agent(config.use_agent);
            ui.set_backup_title(app.application.clone().into());
            ui.set_backup_sources(
                sources
                    .as_ref()
                    .map(|s| {
                        s.iter()
                            .map(|s| s.display().to_string())
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default()
                    .into(),
            );
            ui.set_backup_kind(index_kind(&config));
            ui.set_backup_location(config.destination.location.clone().into());
            ui.set_backup_executable(config.endpoint.executable.clone().into());
            ui.set_backup_config(config.endpoint.config.clone().into());
            ui.set_backup_job(config.endpoint.job_id.to_string().into());
            ui.set_backup_restic(config.restic.clone().into());
            ui.set_backup_password_file(config.password_file.clone().into());
            ui.set_backup_restore_root(config.restore_root.clone().into());
            ui.set_backup_acknowledged(false);
            ui.set_backup_snapshots(ModelRc::default());
            ui.set_backup_snapshot_index(0);
            show_result(
                &ui,
                sources.map(|_| {
                    i18n::text("Configure the destination and export setup for the first run.")
                }),
            );
            *opened.borrow_mut() = Some(OpenPlan {
                server_id: server.id,
                application: app.application.clone(),
                config,
                snapshots: vec![],
            });
            ui.set_dialog(6);
        });
    }
    let weak = ui.as_weak();
    ui.on_backup_action(move |operation| {
        let Some(ui) = weak.upgrade() else { return; };
        let operation = operation.to_string();
        if operation == "activity" {
            if let Ok(guard) = state.lock() {
                let message = guard.server.as_ref().map(|s| activity(&root, s.id)).unwrap_or_else(|| Ok(i18n::text("Connect a server first.")));
                ui.set_backup_activity(message.unwrap_or_else(|error| error).into());
            }
            return;
        }
        if operation == "close" && ui.get_dialog() == 6 && !ui.get_test_busy() {
            ui.set_dialog(0); *opened.borrow_mut() = None; return;
        }
        let Some(mut plan) = opened.borrow().clone() else { return; };
        let Ok(mut guard) = state.lock() else { return; };
        if guard.busy || ui.get_dialog() != 6 { return; }
        if operation == "close" { ui.set_dialog(0); *opened.borrow_mut() = None; return; }
        let Some(server) = guard.server.as_ref().filter(|server| server.id == plan.server_id).cloned() else {
            show_result(&ui, Err(i18n::text("Server context changed. Reopen the backup plan."))); return;
        };
        let Some(app) = guard.report.applications.iter().find(|a| a.application == plan.application) else { return; };
        let Some(selection) = guard.selection.applications.iter().find(|a| a.application == plan.application) else { return; };
        let selection = selection.clone();
        let sources = match approved_sources(app, &selection) { Ok(value) => value, Err(error) => { show_result(&ui, Err(error)); return; } };
        plan.config = match read_form(&ui, &plan.config) { Ok(value) => value, Err(error) => { show_result(&ui, Err(error)); return; } };
        if matches!(operation.as_str(), "backup" | "init" | "restore") && !ui.get_backup_acknowledged() { return; }
        let persist = || -> Result<(), String> {
            let path = plan_path(&root, plan.server_id);
            let mut plans: Plans = load(&path)?; plans.insert(plan.application.clone(), plan.config.clone()); save(&path, &plans)?;
            let server_root = storage::server_dir(&root, server.id);
            save(&server_root.join("backup-selection.json"), &guard.selection)
        };
        if let Err(error) = persist() { show_result(&ui, Err(error)); return; }
        guard.dirty = false;
        *opened.borrow_mut() = Some(plan.clone());
        if operation == "save" { show_result(&ui, Ok(i18n::text("Backup plan saved."))); render(&ui, &guard); return; }
        if operation == "export" {
            drop(guard);
            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                show_result(&ui, export_setup(&plan.config, &sources, &folder).map(|_| i18n::text("Setup exported. Follow SETUP.md on the Docker server before starting a backup.")));
            }
            return;
        }
        let selected_snapshot = plan.snapshots.get(ui.get_backup_snapshot_index().max(0) as usize).map(|s| s.id.clone());
        let action = match operation.as_str() {
            "backup" => Action::BackupSelected { expected_sources: sources.clone(), expected_repository: plan.config.destination.location.clone().into() },
            "init" => Action::Init {}, "snapshots" => Action::Snapshots {}, "check" => Action::Check { read_data: true },
            "restore" => match selected_snapshot { Some(snapshot_id) => Action::Restore { snapshot_id, directory: format!("test-{}", Uuid::new_v4()) }, None => { show_result(&ui, Err(i18n::text("List snapshots and select one first."))); return; } },
            _ => return,
        };
        let secret = Secret(guard.secret.0.clone());
        guard.busy = true; ui.set_loading(true); ui.set_test_busy(true);
        ui.set_connection_step(i18n::text(match operation.as_str() { "backup" => "Checking sources and creating encrypted backup…", "init" => "Initializing encrypted repository…", "check" => "Reading and checking all repository data…", "restore" => "Restoring to a new isolated directory…", _ => "Loading snapshots…" }).into());
        drop(guard);
        let state = state.clone(); let root = root.clone(); let weak = weak.clone(); let opened = opened.clone();
        // Rc stays on the UI thread; worker results are delivered through the event loop.
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| -> Result<serde_json::Value, String> {
                let profiles = storage::Profiles::load(&root)?;
                let connection = Connection::open(&server, &secret, &profiles.hosts).map_err(|e| match e {
                    Failure::Message(message) => message,
                    _ => i18n::text("SSH host identity is unknown or changed. Connection blocked."),
                })?;
                if matches!(action, Action::BackupSelected { .. }) {
                    let containers = containersafe::discover(&connection)?;
                    let current: BackupReport = serde_json::from_value(serde_json::to_value(containersafe::plan::build(&containers)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                    let app = current.applications.iter().find(|app| app.application == plan.application).ok_or_else(|| i18n::text("Inventory changed. Refresh and review your selections."))?;
                    if approved_sources(app, &selection)? != sources { return Err(i18n::text("Inventory changed. Refresh and review your selections.")); }
                }
                if !plan.config.use_agent {
                    return containersafe::direct_backup::execute(&connection, &plan.config.restic,
                        &plan.config.password_file, &plan.config.restore_root,
                        &plan.config.destination, &sources, action);
                }
                let description: JobDescription = serde_json::from_value(backup::call(&connection, &plan.config.endpoint, Action::Describe {}, 30)?).map_err(|_| "Invalid agent job description")?;
                let expected: BTreeSet<_> = sources.iter().collect(); let actual: BTreeSet<_> = description.sources.iter().collect();
                if actual != expected || description.repository != Path::new(&plan.config.destination.location)
                    || description.restore_root != Path::new(&plan.config.restore_root) {
                    return Err(i18n::text("Agent configuration differs from this plan. Export and install the updated setup; nothing was started."));
                }
                backup::call(&connection, &plan.config.endpoint, action, description.timeout_seconds.saturating_add(30).min(86400))
            })();
            let _ = sender.send((result, root, server.id, plan.application, operation));
        });
        let timer = Rc::new(Timer::default()); let keep = timer.clone();
        timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
            let (result, root, server_id, application, operation) = match receiver.try_recv() {
                Ok(value) => value,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    keep.stop();
                    if let Ok(mut guard) = state.lock() { guard.busy = false; }
                    if let Some(ui) = weak.upgrade() {
                        ui.set_loading(false); ui.set_test_busy(false);
                        show_result(&ui, Err(i18n::text("Backup worker stopped unexpectedly. Inspect snapshots before retrying.")));
                    }
                    return;
                }
            };
            keep.stop();
            if let Ok(mut guard) = state.lock() { guard.busy = false; }
            let Some(ui) = weak.upgrade() else { return; };
            ui.set_loading(false); ui.set_test_busy(false);
            let message = result.and_then(|value| -> Result<String, String> {
                match operation.as_str() {
                    "backup" => {
                        let backup: BackupResult = serde_json::from_value(value).map_err(|_| "Invalid backup result")?;
                        if !valid_snapshot(&backup.snapshot_id) { return Err("Invalid snapshot identity".into()); }
                        Ok(i18n::format("Snapshot {0}\n{1} files · {2} bytes processed · {3} seconds", &[&backup.snapshot_id, &backup.total_files_processed.to_string(), &backup.total_bytes_processed.to_string(), &format!("{:.1}", backup.total_duration)]))
                    }
                    "snapshots" => {
                        let snapshots: Vec<SnapshotInfo> = serde_json::from_value(value).map_err(|_| "Invalid snapshot list")?;
                        if snapshots.iter().any(|snapshot| !valid_snapshot(&snapshot.id)) { return Err("Invalid snapshot identity".into()); }
                        let labels = snapshots.iter().map(|s| format!("{} · {}", s.time, &s.id[..s.id.len().min(12)]).into()).collect::<Vec<slint::SharedString>>();
                        ui.set_backup_snapshots(ModelRc::new(VecModel::from(labels))); ui.set_backup_snapshot_index(0);
                        let count = snapshots.len(); if let Some(plan) = opened.borrow_mut().as_mut() { plan.snapshots = snapshots; }
                        Ok(i18n::format("Snapshots found: {0}", &[&count.to_string()]))
                    }
                    "init" if value["initialized"] == true => Ok(i18n::text("Encrypted repository initialized. You can start the first backup.")),
                    "check" => {
                        let checked: CheckResult = serde_json::from_value(value).map_err(|_| "Invalid check result")?;
                        if !checked.all_data_read { return Err("Full data check was not completed".into()); }
                        Ok(i18n::text("All repository data checked successfully."))
                    }
                    "restore" => {
                        let restored: RestoreResult = serde_json::from_value(value).map_err(|_| "Invalid restore result")?;
                        if !restored.content_verified || !valid_snapshot(&restored.snapshot_id) { return Err("Restore was not verified".into()); }
                        Ok(i18n::format("Test restore verified at: {0}", &[&restored.target.display().to_string()]))
                    }
                    _ => Err("Unknown backup operation".into()),
                }
            });
            let log_result = record(&root, server_id, &application, &operation, &message);
            let message = match log_result { Ok(()) => message, Err(error) => Err(format!("{}\nActivity log could not be saved: {error}", match message { Ok(value) | Err(value) => value })) };
            show_result(&ui, message);
            ui.set_backup_activity(activity(&root, server_id).unwrap_or_else(|e| e).into());
        });
    });
}
fn index_kind(config: &PlanConfig) -> i32 {
    index(config.destination.kind)
}
fn valid_snapshot(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn direct_mode_is_default_migrates_old_plans_and_exports_no_agent_config() {
        let root = tempfile::tempdir().unwrap();
        let mut config = PlanConfig::new(Destination {
            kind: DestinationKind::ServerFolder,
            location: "/backups/demo".into(),
        });
        assert!(!config.use_agent);
        let mut legacy = serde_json::to_value(&config).unwrap();
        legacy.as_object_mut().unwrap().remove("use_agent");
        assert!(
            !serde_json::from_value::<PlanConfig>(legacy)
                .unwrap()
                .use_agent
        );
        config.endpoint.executable = "not needed".into();
        config.endpoint.config = "not needed".into();
        assert!(config.validate().is_ok());
        export_setup(&config, &[PathBuf::from("/srv/demo")], root.path()).unwrap();
        let folder = fs::read_dir(root.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(folder.join("SETUP.md").is_file());
        assert!(folder.join("backup-plan.json").is_file());
        assert!(!folder.join("agent.json").exists());
    }
    #[test]
    fn default_destination_and_plan_overrides_remain_separate_per_server() {
        let root = tempfile::tempdir().unwrap();
        let default_path = root.path().join("backup-default.json");
        let default = Destination {
            kind: DestinationKind::ServerFolder,
            location: "/backups".into(),
        };
        save(&default_path, &default).unwrap();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let mut plan = PlanConfig::new(load(&default_path).unwrap());
        plan.destination = Destination {
            kind: DestinationKind::Remote,
            location: "s3:https://storage/bucket/plan".into(),
        };
        save(
            &plan_path(root.path(), first),
            &BTreeMap::from([("standalone/demo".to_owned(), plan)]),
        )
        .unwrap();
        assert_eq!(load::<Destination>(&default_path).unwrap(), default);
        assert!(load::<Plans>(&plan_path(root.path(), second))
            .unwrap()
            .is_empty());
        let loaded = load::<Plans>(&plan_path(root.path(), first)).unwrap();
        assert_eq!(
            loaded["standalone/demo"].destination.kind,
            DestinationKind::Remote
        );
    }
    #[test]
    fn exported_setup_contains_only_approved_sources_and_secret_references() {
        let root = tempfile::tempdir().unwrap();
        let mut config = PlanConfig::new(Destination {
            kind: DestinationKind::ServerFolder,
            location: "/backups/demo".into(),
        });
        config.use_agent = true;
        export_setup(&config, &[PathBuf::from("/srv/demo")], root.path()).unwrap();
        let folders = fs::read_dir(root.path())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(folders.len(), 1);
        let exported: AgentConfig = read_json(&folders[0].path().join("agent.json")).unwrap();
        assert_eq!(exported.jobs[0].sources, vec![PathBuf::from("/srv/demo")]);
        assert_eq!(exported.jobs[0].id, config.endpoint.job_id);
        assert_eq!(exported.jobs[0].repository.path, Path::new("/backups/demo"));
        assert!(folders[0].path().join("SETUP.md").is_file());
        export_setup(&config, &[PathBuf::from("/srv/demo")], root.path()).unwrap();
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }
    #[test]
    fn blocks_raw_databases_stale_paths_and_unimplemented_dumps() {
        let mut app = ApplicationPlan {
            application: "standalone/test".into(),
            status: String::new(),
            containers: vec![],
            sources: vec![SourcePlan {
                path: "/data".into(),
                mount_type: "bind".into(),
                destination: "/data".into(),
                policy: "review_for_backup".into(),
            }],
            databases: vec![],
            warnings: vec![],
        };
        let mut selected = ApplicationSelection {
            application: app.application.clone(),
            sources: vec![SourceSelection {
                path: "/data".into(),
                decision: "include".into(),
            }],
            databases: vec![],
        };
        assert_eq!(
            approved_sources(&app, &selected).unwrap(),
            vec![PathBuf::from("/data")]
        );
        app.sources[0].policy = "requires_logical_dump".into();
        assert!(approved_sources(&app, &selected).is_err());
        app.sources[0].policy = "review_for_backup".into();
        selected.sources[0].path = "/stale".into();
        assert!(approved_sources(&app, &selected).is_err());
        selected.sources[0].path = "/data".into();
        selected.databases.push(DatabaseSelection {
            container: "postgres".into(),
            decision: "logical_dump".into(),
        });
        assert!(approved_sources(&app, &selected).is_err());
    }
}
