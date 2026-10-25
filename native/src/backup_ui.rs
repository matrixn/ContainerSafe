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
    #[serde(default = "Uuid::new_v4")]
    pc_repository: Uuid,
    #[serde(default)]
    rest_server: String,
    #[serde(default)]
    transfer_directory: String,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    artifact: Option<containersafe::transfer::SourceIdentity>,
    #[serde(default)]
    local_restic: String,
    #[serde(default)]
    local_password_file: String,
    #[serde(default)]
    mirror_directory: String,
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
            pc_repository: Uuid::new_v4(),
            rest_server: String::new(),
            transfer_directory: String::new(),
            stream: false,
            artifact: None,
            local_restic: String::new(),
            local_password_file: String::new(),
            mirror_directory: String::new(),
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
        3 => DestinationKind::LocalPcRepository,
        _ => DestinationKind::ServerFolder,
    }
}
fn index(kind: DestinationKind) -> i32 {
    match kind {
        DestinationKind::ServerFolder => 0,
        DestinationKind::ThisComputer => 1,
        DestinationKind::Remote => 2,
        DestinationKind::LocalPcRepository => 3,
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
    let mut databases = BTreeSet::new();
    for db in selection
        .databases
        .iter()
        .filter(|d| d.decision == "logical_dump")
    {
        let known = app
            .databases
            .iter()
            .find(|d| d.container == db.container)
            .ok_or("Database inventory changed; refresh and review selections")?;
        if !containersafe::database::supported(&known.engine) || !databases.insert(&db.container) {
            return Err("Invalid database selection".into());
        }
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
    if sources.is_empty() && databases.is_empty() {
        return Err(i18n::text("Select at least one file source with Include."));
    }
    Ok(sources.into_iter().collect())
}

fn database_binding(
    app: &ApplicationPlan,
    selection: &ApplicationSelection,
) -> BTreeSet<(String, String, String)> {
    app.databases
        .iter()
        .filter(|db| {
            selection
                .databases
                .iter()
                .any(|d| d.container == db.container && d.decision == "logical_dump")
        })
        .map(|db| {
            (
                db.container.clone(),
                db.engine.clone(),
                db.source_path.clone(),
            )
        })
        .collect()
}

fn execution_sources(
    app: &ApplicationPlan,
    selection: &ApplicationSelection,
    config: &PlanConfig,
) -> Result<Vec<PathBuf>, String> {
    let mut sources = approved_sources(app, selection)?;
    if selection
        .databases
        .iter()
        .any(|d| d.decision == "logical_dump")
    {
        sources.push(containersafe::database::staging_path(
            &config.restore_root,
            config.endpoint.job_id,
        ));
    }
    sources.sort();
    Ok(sources)
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
    value.rest_server = ui.get_backup_rest_server().trim().into();
    value.transfer_directory = ui.get_backup_transfer_directory().to_string();
    value.stream = ui.get_backup_stream();
    value.local_restic = ui.get_backup_local_restic().to_string();
    value.local_password_file = ui.get_backup_local_password_file().to_string();
    value.mirror_directory = ui.get_backup_mirror_directory().to_string();
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

Approved database exports are prepared by the desktop before Restic runs. The private
containersafe-db-<job> source is temporary: never create it or reuse stale contents.
Install the required database client inside each container; see docs/DATABASE_BACKUPS.md.
No container is stopped. MySQL/MariaDB dumps hold a global read lock, blocking writes.
Full application consistency and automatic database import are not provided.
"#,
                restic = config.restic,
                password = config.password_file,
                restores = config.restore_root,
                destination = config.destination.location
            ),
        )
        .map_err(|e| e.to_string())?;
        export_pc_setup(config, &folder)?;
        return Ok(());
    }
    let agent = AgentConfig {
        restic_executable: config.restic.clone().into(),
        timeout_seconds: 3600,
        jobs: vec![JobConfig {
            pc_repository: (config.destination.kind == DestinationKind::LocalPcRepository)
                .then_some(config.pc_repository),
            pc_only: config.destination.kind == DestinationKind::LocalPcRepository,
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
databases require application-specific procedures. The desktop prepares approved database dumps
before invoking this agent; run the desktop for database plans, not standalone agent Backup.
Do not pre-create or reuse a containersafe-db directory. Re-export after source changes.
Compose recovery, database import, scheduling and retention are not implemented.
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
    export_pc_setup(config, &directory)?;
    Ok(())
}
fn export_pc_setup(config: &PlanConfig, directory: &Path) -> Result<(), String> {
    if config.destination.kind != DestinationKind::LocalPcRepository {
        return Ok(());
    }
    fs::write(
        directory.join("PC_BACKUP.md"),
        format!(
            r#"# Direct PC repository

This plan uses repository UUID {id}. The saved PC folder is descriptive on the NAS;
never pass that Windows path to Linux Restic. ContainerSafe creates a fresh,
authenticated loopback reverse SSH tunnel for each operation. The helper is the
official restic/rest-server 0.14.0 executable, selected on the PC, not on the NAS.

Allow reverse TCP forwarding for this SSH account, bound to 127.0.0.1 only.
ContainerSafe refuses wildcard or unverified listeners and never changes sshd.
Keep the PC awake. No inbound SSH/SFTP server on the PC is needed.
Save the plan, initialize once, then back up through ContainerSafe.
Repository files are under <PC folder>/<server UUID>/{id}/repository.

For agent mode, install the exported agent.json with its pc_repository and pc_only
authorization using agent 0.34 or later. Normal NAS repository operations are
disabled for this job. For direct mode no agent/config is required.
The existing NAS password file and approved source permissions still apply.
For local checks/restores, independently create a protected password file on the PC
containing the same repository password; it is never downloaded from the NAS.
Interrupted operations may leave Restic locks: inspect before retrying; never
automatically unlock/prune or restore into live data.
"#,
            id = config.pc_repository
        ),
    )
    .map_err(|e| e.to_string())
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
    use containersafe::transfer::{CancellationToken, ProgressSink, TransferProgress};
    let cancellation = Rc::new(RefCell::new(None::<CancellationToken>));
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
                if !destination.location.is_empty()
                    && destination.kind != DestinationKind::LocalPcRepository
                {
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
            let sources = execution_sources(app, selection, &config);
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
            ui.set_backup_rest_server(config.rest_server.clone().into());
            ui.set_backup_transfer_directory(config.transfer_directory.clone().into());
            ui.set_backup_stream(config.stream);
            ui.set_backup_local_restic(config.local_restic.clone().into());
            ui.set_backup_local_password_file(config.local_password_file.clone().into());
            ui.set_backup_mirror_directory(config.mirror_directory.clone().into());
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
        if operation == "cancel-transfer" {
            if let Some(token) = cancellation.borrow().as_ref() { token.cancel(); }
            ui.set_connection_step(i18n::text("Cancelling transfer… Inspect snapshots before retrying.").into());
            return;
        }
        if matches!(operation.as_str(), "browse-pc" | "browse-backend" | "browse-download" | "browse-mirror") && !ui.get_test_busy() {
            if operation != "browse-backend" {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    if operation == "browse-download" { ui.set_backup_transfer_directory(path.display().to_string().into()); }
                    else if operation == "browse-mirror" { ui.set_backup_mirror_directory(path.display().to_string().into()); }
                    else { ui.set_backup_location(path.display().to_string().into()); }
                }
            } else if let Some(path) = rfd::FileDialog::new().pick_file() { ui.set_backup_rest_server(path.display().to_string().into()); }
            return;
        }
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
        let database_binding = database_binding(app, &selection);
        plan.config = match read_form(&ui, &plan.config) { Ok(value) => value, Err(error) => { show_result(&ui, Err(error)); return; } };
        let sources = match execution_sources(app, &selection, &plan.config) { Ok(value) => value, Err(error) => { show_result(&ui, Err(error)); return; } };
        if operation == "transfer-history" {
            let directory = storage::server_dir(&root,server.id).join("transfers/history");
            let result = (|| -> Result<String,String> {
                let mut lines=Vec::new();
                if directory.exists() {for entry in fs::read_dir(directory).map_err(|e|e.to_string())? {
                    let path=entry.map_err(|e|e.to_string())?.path();
                    if path.extension().is_some_and(|e|e=="json") {
                        let receipt:containersafe::transfer::TransferReceipt=serde_json::from_slice(&fs::read(path).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
                        if receipt.source.server_id==server.id {lines.push(format!("{:?} · {:?} · {} / {} bytes · SHA-256 {}\n{}",receipt.kind,receipt.state,receipt.bytes,receipt.source.size,receipt.sha256_verified,receipt.destination.directory.display()));}
                    }
                }}
                if lines.is_empty(){Ok(i18n::text("No backup activity yet"))}else{Ok(lines.join("\n\n"))}
            })();
            show_result(&ui,result);return;
        }
        if operation == "open-folder" {
            let result=(|| -> Result<String,String> {
                let directory=fs::canonicalize(&plan.config.transfer_directory).map_err(|e|e.to_string())?;
                if !directory.is_dir(){return Err("Choose an existing PC folder".into());}
                #[cfg(target_os="windows")] let executable="explorer.exe";
                #[cfg(target_os="linux")] let executable="xdg-open";
                #[cfg(target_os="macos")] let executable="open";
                std::process::Command::new(executable).arg(directory).spawn().map_err(|e|e.to_string())?;
                Ok(String::new())
            })();show_result(&ui,result);return;
        }
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
            "export-artifact" | "download" | "resume" | "verify-local" | "restore-local" | "mirror" => Action::Snapshots {},
            "restore" => match selected_snapshot.clone() { Some(snapshot_id) => Action::Restore { snapshot_id, directory: format!("test-{}", Uuid::new_v4()) }, None => { show_result(&ui, Err(i18n::text("List snapshots and select one first."))); return; } },
            _ => return,
        };
        let secret = Secret(guard.secret.0.clone());
        let token = CancellationToken::default();
        *cancellation.borrow_mut() = Some(token.clone());
        let progress = Arc::new(Mutex::new(None::<TransferProgress>));
        let updates = progress.clone();
        let sink: ProgressSink = Arc::new(move |value| { if let Ok(mut latest) = updates.lock() { *latest = Some(value); } });
        ui.set_backup_cancellable((plan.config.destination.kind == DestinationKind::LocalPcRepository && selection.databases.iter().all(|d| d.decision != "logical_dump")) || matches!(operation.as_str(), "download" | "resume"));
        guard.busy = true; ui.set_loading(true); ui.set_test_busy(true);
        ui.set_connection_step(i18n::text(match operation.as_str() { "backup" => "Checking sources and creating encrypted backup…", "init" => "Initializing encrypted repository…", "check" => "Reading and checking all repository data…", "restore" | "restore-local" => "Restoring to a new isolated directory…", "export-artifact" => "Preparing encrypted export…", "download" | "resume" => "Connecting for encrypted transfer…", "verify-local" => "Verifying integrity…", "mirror" => "Copying snapshots into PC repository…", _ => "Loading snapshots…" }).into());
        if operation == "backup" && selection.databases.iter().any(|db| db.decision == "logical_dump") {
            ui.set_connection_step(i18n::text("Exporting selected databases, then creating encrypted backup…").into());
        }
        drop(guard);
        let state = state.clone(); let root = root.clone(); let weak = weak.clone(); let opened = opened.clone();
        // Rc stays on the UI thread; worker results are delivered through the event loop.
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| -> Result<serde_json::Value, String> {
                let profiles = storage::Profiles::load(&root)?;
                if matches!(operation.as_str(), "verify-local" | "restore-local" | "mirror") {
                    let endpoint = format!("[{}]:{}",server.host.to_ascii_lowercase(),server.port);
                    let identity=storage::DataIdentity{server_id:server.id,host:server.host.to_ascii_lowercase(),port:server.port,
                        host_key:profiles.hosts.get(&endpoint).cloned().ok_or("No saved verified SSH identity for this server")?};
                    use containersafe::{restic::{ResticEngine, RepositorySecret}, transfer::{local_artifact, local_backend::RepositoryContext, TransferReceipt}};
                    let engine = ResticEngine::new(&plan.config.local_restic).map_err(|e| e.to_string())?.with_timeout(Duration::from_secs(21600)).map_err(|e|e.to_string())?;
                    let password_path = Path::new(&plan.config.local_password_file);
                    let meta = fs::symlink_metadata(password_path).map_err(|_| "Local repository password file is unavailable")?;
                    if !password_path.is_absolute() || !meta.is_file() || meta.file_type().is_symlink() || meta.len()>65536 {return Err("Invalid local password file".into());}
                    let mut password = fs::read_to_string(password_path).map_err(|_| "Cannot read local repository password file")?;
                    while password.ends_with(['\r','\n']) {password.pop();}
                    let secret = RepositorySecret::new(plan.config.secret_reference,password).map_err(|e|e.to_string())?;
                    let mut extracted = None;
                    let mut context = None;
                    let repository = if plan.config.destination.kind == DestinationKind::LocalPcRepository {
                        context = Some(RepositoryContext::open(Path::new(&plan.config.destination.location),plan.config.pc_repository,identity.clone()).map_err(|e|e.to_string())?);
                        context.as_ref().unwrap().repository.clone()
                    } else {
                        let source = plan.config.artifact.as_ref().ok_or("Create and download an encrypted export first")?;
                        let path=storage::server_dir(&root,server.id).join("transfers").join(format!("{}.json",source.artifact_id));
                        let receipt:TransferReceipt=serde_json::from_slice(&fs::read(path).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
                        if &receipt.source != source || source.server_id!=identity.server_id || source.host_key!=identity.host_key || source.endpoint!=endpoint {return Err("Artifact identity changed".into());}
                        extracted=Some(local_artifact::extract(&receipt,&token).map_err(|e|e.to_string())?);
                        extracted.as_ref().unwrap().repository()
                    };
                    let repository=RepositoryConfig{path:repository,secret_reference:plan.config.secret_reference};
                    engine.check(&repository,&secret,true).map_err(|e|e.to_string())?;
                    let snapshots=engine.snapshots(&repository,&secret).map_err(|e|e.to_string())?;
                    let message=if operation=="mirror" {
                        if plan.config.destination.kind == DestinationKind::LocalPcRepository {return Err("Repository is already on this PC".into());}
                        let destination=RepositoryContext::open(Path::new(&plan.config.mirror_directory),plan.config.pc_repository,identity.clone()).map_err(|e|e.to_string())?;
                        let target=RepositoryConfig{path:destination.repository.clone(),secret_reference:plan.config.secret_reference};
                        if !target.path.exists() {engine.init(&target,&secret).map_err(|e|e.to_string())?;}
                        let ids=snapshots.iter().map(|s|s.id.clone()).collect::<Vec<_>>();
                        engine.copy_snapshots(&repository,&secret,&target,&secret,&ids).map_err(|e|e.to_string())?;
                        engine.check(&target,&secret,true).map_err(|e|e.to_string())?;
                        format!("Verified repository copy: {}",target.path.display())
                    } else if operation=="restore-local" {
                        let snapshot=if plan.config.destination.kind == DestinationKind::LocalPcRepository { selected_snapshot.as_deref().ok_or("Select an exact snapshot first")? } else { snapshots.first().ok_or("Empty artifact repository")?.id.as_str() };
                        let base=fs::canonicalize(&plan.config.transfer_directory).map_err(|_|"Choose an existing PC folder for the isolated restore")?;
                        let target=base.join(format!("containersafe-test-restore-{}",Uuid::new_v4()));
                        let restored=engine.restore(&repository,&secret,snapshot,None,&target).map_err(|e|e.to_string())?;
                        if !restored.content_verified {return Err("Restore integrity was not verified".into());}
                        format!("Verified isolated restore: {}",restored.target.display())
                    } else {"All local repository data verified".into()};
                    drop(context);drop(extracted);
                    return Ok(serde_json::json!({"message":message}));
                }
                let connection = Connection::open(&server, &secret, &profiles.hosts).map_err(|e| match e {
                    Failure::Message(message) => message,
                    _ => i18n::text("SSH host identity is unknown or changed. Connection blocked."),
                })?;
                if operation == "export-artifact" {
                    if plan.config.destination.kind == DestinationKind::LocalPcRepository { return Err("PC repositories do not need a NAS export".into()); }
                    let snapshot = selected_snapshot.as_deref().ok_or_else(|| i18n::text("List snapshots and select one first."))?;
                    let source = containersafe::transfer::artifact::create(&connection, &containersafe::transfer::artifact::ExportRequest {
                        plan_id: plan.config.endpoint.job_id, restic: &plan.config.restic, password_file: &plan.config.password_file,
                        restore_root: &plan.config.restore_root, destination: &plan.config.destination, approved_sources: &sources, snapshot_id: snapshot,
                    })?;
                    return serde_json::to_value(source).map_err(|e| e.to_string());
                }
                if matches!(operation.as_str(), "download" | "resume") {
                    use containersafe::transfer::{Coordinator, TransferKind, TransferReceipt};
                    let source = plan.config.artifact.clone().ok_or("Create an encrypted export first")?;
                    let folder = PathBuf::from(&plan.config.transfer_directory);
                    let directory = storage::server_dir(&root, server.id).join("transfers");
                    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
                    let receipt_path = directory.join(format!("{}.json", source.artifact_id));
                    let kind = if plan.config.stream { TransferKind::SshStream } else { TransferKind::SftpPull };
                    let receipt: TransferReceipt = if operation == "resume" {
                        if kind != TransferKind::SftpPull { return Err("Streaming cannot resume. Start a new transfer.".into()); }
                        let old: TransferReceipt = serde_json::from_slice(&fs::read(&receipt_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                        if old.source != source || old.destination.directory != fs::canonicalize(folder).map_err(|e| e.to_string())? { return Err("Resume identity or destination changed".into()); }
                        old
                    } else { TransferReceipt::new(source, folder, kind).map_err(|e| e.to_string())? };
                    let coordinator = Coordinator::open(receipt, &receipt_path, sink, token).map_err(|e| e.to_string())?;
                    let result = if kind == TransferKind::SftpPull { containersafe::transfer::sftp_pull::download(&connection, coordinator) }
                        else { containersafe::transfer::ssh_stream::receive(&connection, coordinator) };
                    if let Ok(bytes) = fs::read(&receipt_path) {
                        if let Ok(receipt) = serde_json::from_slice::<TransferReceipt>(&bytes) {
                            fs::create_dir_all(directory.join("history")).map_err(|e|e.to_string())?;
                            save(&directory.join("history").join(format!("{}.json",receipt.id)),&receipt)?;
                        }
                    }
                    let result=result.map_err(|e|e.to_string())?;
                    return serde_json::to_value(result).map_err(|e| e.to_string());
                }
                let mut staged = None;
                if matches!(action, Action::BackupSelected { .. }) {
                    let containers = containersafe::discover(&connection)?;
                    let current: BackupReport = serde_json::from_value(serde_json::to_value(containersafe::plan::build(&containers)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                    let app = current.applications.iter().find(|app| app.application == plan.application).ok_or_else(|| i18n::text("Inventory changed. Refresh and review your selections."))?;
                    if database_binding != self::database_binding(app, &selection) { return Err(i18n::text("Inventory changed. Refresh and review your selections.")); }
                    if execution_sources(app, &selection, &plan.config)? != sources { return Err(i18n::text("Inventory changed. Refresh and review your selections.")); }
                    let specs = containersafe::database::specs(
                        &serde_json::from_value(serde_json::to_value(app).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?,
                        &serde_json::from_value(serde_json::to_value(&selection).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?, &containers)?;
                    if !specs.is_empty() {
                        staged = Some(containersafe::database::remote::StagedDumps::prepare(&connection,
                            &containersafe::database::staging_path(&plan.config.restore_root, plan.config.endpoint.job_id),
                            &specs, &sources, &plan.config.destination.location, &plan.config.password_file)?);
                    }
                }
                let outcome = (|| {
                if plan.config.destination.kind == DestinationKind::LocalPcRepository {
                    use containersafe::transfer::{pc_backup, reverse_tunnel::ReverseSession};
                    let session = ReverseSession::open(&connection, Path::new(&plan.config.rest_server),
                        Path::new(&plan.config.destination.location), plan.config.pc_repository, token, sink).map_err(|e| e.to_string())?;
                    return pc_backup::execute(&connection, containersafe::direct_backup::DirectRequest {
                        executable: &plan.config.restic, password_file: &plan.config.password_file,
                        restore_root: &plan.config.restore_root, destination: &plan.config.destination, sources: &sources,
                    }, plan.config.use_agent.then_some(&plan.config.endpoint), action, &session);
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
                if let Some(staged) = staged {
                    if let Err(cleanup) = staged.finish() {
                        return Err(match outcome { Ok(_) => format!("Backup completed, but {cleanup}"), Err(error) => format!("{error}\n{cleanup}") });
                    }
                }
                outcome
            })();
            let _ = sender.send((result, root, server.id, plan.application, operation));
        });
        let timer = Rc::new(Timer::default()); let keep = timer.clone();
        timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(latest) = progress.lock() { if let Some(p) = latest.as_ref() {
                    let label=i18n::text(match p.state {containersafe::transfer::TransferState::VerifyingIntegrity=>"Verifying integrity…",_=>"Transferring encrypted data…"});
                    let mut text=format!("{label}\n{:.1} MiB · {:.1} MiB/s · {:.0}s",p.bytes as f64/1048576.0,p.bytes_per_second/1048576.0,p.elapsed_seconds);
                    if p.total>0 {text.push_str(&format!("\n{:.0}% · ETA {:.0}s",p.bytes as f64/p.total as f64*100.0,p.total.saturating_sub(p.bytes) as f64/p.bytes_per_second.max(1.0)));}
                    ui.set_connection_step(text.into());
                } }
            }
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
            ui.set_backup_cancellable(false);
            let message = result.and_then(|value| -> Result<String, String> {
                match operation.as_str() {
                    "verify-local" | "restore-local" | "mirror" => Ok(value["message"].as_str().ok_or("Invalid local operation result")?.to_string()),
                    "export-artifact" => {
                        let source: containersafe::transfer::SourceIdentity = serde_json::from_value(value).map_err(|e| e.to_string())?;
                        if let Some(plan) = opened.borrow_mut().as_mut() {
                            plan.config.artifact = Some(source);
                            let path = plan_path(&root, server_id);
                            let mut plans: Plans = load(&path)?;
                            plans.insert(plan.application.clone(), plan.config.clone()); save(&path, &plans)?;
                        }
                        Ok(i18n::text("Encrypted export ready. Choose a PC folder and download."))
                    }
                    "download" | "resume" => {
                        let receipt: containersafe::transfer::TransferReceipt = serde_json::from_value(value).map_err(|e| e.to_string())?;
                        if !receipt.sha256_verified || receipt.state != containersafe::transfer::TransferState::Completed { return Err("Transfer was not verified".into()); }
                        Ok(format!("SHA-256 verified · {} bytes\n{}", receipt.bytes, receipt.destination.directory.join(format!("{}.csbackup.tar", receipt.source.artifact_id)).display()))
                    }
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
    fn old_sftp_plan_migrates_without_changing_destination_or_secret() {
        let destination = Destination {
            kind: DestinationKind::ThisComputer,
            location: "sftp:backup-pc:/C:/Backup".into(),
        };
        let config = PlanConfig::new(destination.clone());
        let reference = config.secret_reference;
        let mut legacy = serde_json::to_value(config).unwrap();
        for key in [
            "pc_repository",
            "rest_server",
            "transfer_directory",
            "stream",
            "artifact",
            "local_restic",
            "local_password_file",
            "mirror_directory",
        ] {
            legacy.as_object_mut().unwrap().remove(key);
        }
        let migrated: PlanConfig = serde_json::from_value(legacy).unwrap();
        assert_eq!(migrated.destination, destination);
        assert_eq!(migrated.secret_reference, reference);
        assert!(!migrated.pc_repository.is_nil());
        assert!(!migrated.stream);
        assert!(migrated.artifact.is_none());
        assert_eq!(index(migrated.destination.kind), 1);
        assert_eq!(kind(3), DestinationKind::LocalPcRepository);
    }
    #[test]
    fn pc_agent_export_authorizes_only_its_stable_repository() {
        let root = tempfile::tempdir().unwrap();
        let mut config = PlanConfig::new(Destination {
            kind: DestinationKind::LocalPcRepository,
            location: root.path().display().to_string(),
        });
        config.use_agent = true;
        export_setup(&config, &[PathBuf::from("/srv/demo")], root.path()).unwrap();
        let folder = fs::read_dir(root.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let exported: AgentConfig = read_json(&folder.join("agent.json")).unwrap();
        assert!(exported.jobs[0].pc_only);
        assert_eq!(exported.jobs[0].pc_repository, Some(config.pc_repository));
    }
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
    fn blocks_raw_databases_stale_paths_and_binds_database_only_sources() {
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
        app.databases.push(DatabasePlan {
            container: "postgres".into(),
            engine: "PostgreSQL".into(),
            required_method: "logical_dump".into(),
            source_path: String::new(),
        });
        selected.sources[0].decision = "exclude".into();
        let mut config = PlanConfig::new(Destination {
            kind: DestinationKind::ServerFolder,
            location: "/backups/db".into(),
        });
        config.restore_root = "/srv/restores".into();
        let sources = execution_sources(&app, &selected, &config).unwrap();
        assert_eq!(
            sources,
            vec![containersafe::database::staging_path(
                &config.restore_root,
                config.endpoint.job_id
            )]
        );
        config.use_agent = true;
        let directory = tempfile::tempdir().unwrap();
        export_setup(&config, &sources, directory.path()).unwrap();
        let folder = fs::read_dir(directory.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let exported: AgentConfig =
            serde_json::from_slice(&fs::read(folder.join("agent.json")).unwrap()).unwrap();
        assert_eq!(exported.jobs[0].sources, sources);
    }
}
