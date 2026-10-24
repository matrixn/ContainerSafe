use super::*;
use containersafe::servers::{
    connection::{Connection, ConnectionStage, Failure, HostIdentity},
    credentials,
    model::{now, Authentication, DockerAccess, TestGate},
    storage::Profiles,
};

struct Tested {
    server: Server,
    secret: Secret,
    report: BackupReport,
    summary: String,
}
struct Controller {
    profiles: Profiles,
    editable: bool,
    active: Option<Server>,
    draft: Server,
    gate: TestGate,
    tested: Option<Tested>,
    pending_host: Option<HostIdentity>,
    opening: bool,
}
type Control = Arc<Mutex<Controller>>;

fn rows(ui: &ContainerSafeWindow, c: &Controller) {
    ui.set_servers(ModelRc::new(VecModel::from(
        c.profiles
            .servers
            .iter()
            .map(|s| ServerRow {
                name: s.name.clone().into(),
                host: format!("{}:{}", s.host, s.port).into(),
                status: if c.active.as_ref().is_some_and(|active| active.id == s.id) {
                    i18n::text("Connected").into()
                } else if s.last_status == "Connected" {
                    i18n::text("Last success").into()
                } else {
                    i18n::text(&s.last_status).into()
                },
                last_opened: s
                    .last_opened
                    .and_then(|v| chrono::DateTime::from_timestamp(v as i64, 0))
                    .map(|v| v.format("%Y-%m-%d %H:%M UTC").to_string())
                    .unwrap_or_else(|| i18n::text("Never"))
                    .into(),
                default_server: c.profiles.default == Some(s.id),
            })
            .collect::<Vec<_>>(),
    )));
}

fn populate(ui: &ContainerSafeWindow, c: &mut Controller, server: Server, secret: Secret) {
    c.gate.invalidate();
    c.tested = None;
    c.pending_host = None;
    c.draft = server.clone();
    ui.set_form_name(server.name.into());
    ui.set_form_host(server.host.into());
    ui.set_form_port(server.port.to_string().into());
    ui.set_form_user(server.username.into());
    ui.set_form_auth(match server.authentication {
        Authentication::Password => 0,
        Authentication::Putty => 1,
        Authentication::OpenSsh => 2,
    });
    ui.set_form_key(server.key_path.to_string_lossy().as_ref().into());
    ui.set_form_secret(secret.0.as_str().into());
    ui.set_form_remember(server.remember);
    ui.set_form_docker(match server.docker_access {
        DockerAccess::Automatic => 0,
        DockerAccess::Direct => 1,
        DockerAccess::Sudo => 2,
    });
    ui.set_form_timeout(server.timeout_seconds.to_string().into());
    ui.set_connection_error(false);
    ui.set_test_passed(false);
    ui.set_test_busy(false);
    ui.set_connection_result(crate::i18n::text("Not tested").into());
}

fn form(ui: &ContainerSafeWindow, c: &Controller) -> Result<(Server, Secret), String> {
    let mut s = c.draft.clone();
    s.name = ui.get_form_name().trim().into();
    s.host = ui.get_form_host().trim().into();
    s.port = ui
        .get_form_port()
        .parse()
        .map_err(|_| i18n::text("SSH port must be between 1 and 65535"))?;
    s.username = ui.get_form_user().trim().into();
    s.authentication = match ui.get_form_auth() {
        1 => Authentication::Putty,
        2 => Authentication::OpenSsh,
        _ => Authentication::Password,
    };
    s.key_path = ui.get_form_key().as_str().into();
    s.remember = ui.get_form_remember();
    s.docker_access = match ui.get_form_docker() {
        1 => DockerAccess::Direct,
        2 => DockerAccess::Sudo,
        _ => DockerAccess::Automatic,
    };
    s.timeout_seconds = ui
        .get_form_timeout()
        .parse()
        .map_err(|_| i18n::text("Timeout must be 1-120 seconds"))?;
    s.validate().map_err(|e| i18n::text(&e))?;
    let secret = Secret(ui.get_form_secret().to_string());
    if s.authentication == Authentication::Password && secret.0.is_empty() {
        return Err(i18n::text("Password required"));
    }
    Ok((s, secret))
}

fn probe(
    server: Server,
    secret: Secret,
    hosts: std::collections::BTreeMap<String, String>,
    progress: impl Fn(&'static str) + Send + Sync + 'static,
) -> Result<Tested, Failure> {
    let progress = Arc::new(progress);
    let ssh_progress = progress.clone();
    let connection = Connection::open_with_progress(&server, &secret, &hosts, move |stage| {
        ssh_progress(match stage {
            ConnectionStage::Connecting => "Connecting to SSH…",
            ConnectionStage::VerifyingHost => "Verifying server identity…",
            ConnectionStage::Authenticating => "Authenticating securely…",
            ConnectionStage::CheckingDocker => "Checking Docker access…",
        });
    })?;
    let version = connection.docker("version --format '{{.Server.Version}}'")?;
    progress("Discovering containers…");
    let containers = containersafe::discover(&connection)?;
    progress("Preparing backup plans…");
    let report = serde_json::from_value(
        serde_json::to_value(containersafe::plan::build(&containers))
            .map_err(|_| "Cannot build plan".to_string())?,
    )
    .map_err(|_| "Cannot read plan".to_string())?;
    let summary = i18n::format("SSH: Connected\nHost identity: Verified\nAuthentication: Successful\nDocker: Available\nEngine: {0}\nPermissions: Granted\nContainers detected: {1}\nServer ready to save.", &[version.trim(), &containers.len().to_string()]);
    Ok(Tested {
        server,
        secret,
        report,
        summary,
    })
}

fn activate(
    ui: &ContainerSafeWindow,
    state: &SharedState,
    root: &Path,
    c: &mut Controller,
    mut tested: Tested,
) -> Result<(), String> {
    let dir = storage::server_dir(root, tested.server.id);
    let identity = storage::DataIdentity::new(&tested.server, &c.profiles.hosts)?;
    let (mut previous, _) = load_state_for_server(&dir);
    if !identity.matches(&dir) {
        previous.report = empty_report();
        previous.selection = draft(&previous.report);
    }
    let selection = reconcile(&tested.report, &previous.report, &previous.selection);
    storage::atomic_write(
        &dir.join("backup-plan.json"),
        &serde_json::to_vec_pretty(&tested.report).map_err(|e| e.to_string())?,
    )?;
    identity.save(&dir)?;
    tested.server.last_opened = Some(now());
    tested.server.last_success = Some(now());
    tested.server.last_status = "Connected".into();
    tested.server.last_error = None;
    let mut next = c.profiles.clone();
    next.upsert(tested.server.clone());
    next.save(root)?;
    c.profiles = next;
    c.active = Some(tested.server.clone());
    let mut s = state.lock().map_err(|_| "Workspace state unavailable")?;
    s.server = Some(tested.server.clone());
    s.secret = tested.secret;
    s.report = tested.report;
    s.selection = selection;
    s.active = 0;
    s.selected_container = None;
    s.expanded = std::collections::BTreeSet::from(["compose".into(), "standalone".into()]);
    ui.set_filter_text("".into());
    s.busy = false;
    s.dirty = true;
    render(ui, &s);
    ui.set_active_server(format!("{} · {}", tested.server.name, tested.server.host).into());
    set_status_message(
        ui,
        i18n::format(
            "Connected to {0} · save reconciled backup selections",
            &[&tested.server.name],
        )
        .into(),
        false,
    );
    ui.set_validation_message(crate::i18n::text("Plan changed; validation required").into());
    ui.set_dialog(0);
    ui.set_form_secret("".into());
    rows(ui, c);
    Ok(())
}

fn load_state_for_server(dir: &Path) -> (State, String) {
    let report: BackupReport = read_json(&dir.join("backup-plan.json"))
        .ok()
        .filter(|r: &BackupReport| r.schema_version == 1 && r.read_only)
        .unwrap_or_else(empty_report);
    let selection = read_json::<BackupSelection>(&dir.join("backup-selection.json"))
        .ok()
        .filter(|s| s.schema_version == 1)
        .map(|s| reconcile(&report, &report, &s))
        .unwrap_or_else(|| draft(&report));
    (
        State {
            report,
            selection,
            active: 0,
            selected_container: None,
            expanded: std::collections::BTreeSet::from(["compose".into(), "standalone".into()]),
            dirty: false,
            busy: false,
            server: None,
            secret: Secret::default(),
        },
        String::new(),
    )
}

fn begin(ui: &ContainerSafeWindow, control: &Control, state: &SharedState, root: &Path) {
    let (server, secret, hosts, revision) = {
        let mut c = control.lock().unwrap();
        let (server, secret) = match form(ui, &c) {
            Ok(v) => v,
            Err(e) => {
                ui.set_connection_error(true);
                ui.set_connection_result(e.into());
                return;
            }
        };
        c.gate.invalidate();
        c.tested = None;
        c.pending_host = None;
        (server, secret, c.profiles.hosts.clone(), c.gate.revision)
    };
    ui.set_connection_error(false);
    ui.set_test_passed(false);
    ui.set_connection_step(i18n::text("Connecting to SSH…").into());
    ui.set_test_busy(true);
    ui.set_connection_result(
        "Connecting to SSH; checking identity, authentication and Docker...".into(),
    );
    {
        let mut s = state.lock().unwrap();
        s.busy = true;
        render(ui, &s);
    }
    let weak = ui.as_weak();
    let control = control.clone();
    let state = state.clone();
    let root = root.to_path_buf();
    std::thread::spawn(move || {
        let progress_ui = Arc::new(Mutex::new(weak.clone()));
        let progress_control = control.clone();
        let result = probe(server, secret, hosts, move |step| {
            let weak = progress_ui.lock().unwrap().clone();
            let control = progress_control.clone();
            let _ = weak.upgrade_in_event_loop(move |ui| {
                if ui.get_test_busy() && control.lock().unwrap().gate.revision == revision {
                    ui.set_connection_step(i18n::text(step).into());
                }
            });
        });
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let mut c = control.lock().unwrap();
            if c.gate.revision != revision { return; }
            ui.set_test_busy(false);
            { let mut s = state.lock().unwrap(); s.busy = false; render(&ui,&s); }
            match result {
                Ok(tested) => {
                    c.gate.complete(revision,true);
                    ui.set_connection_result(tested.summary.clone().into()); ui.set_test_passed(true);
                    if c.opening {
                        if let Err(e) = activate(&ui,&state,&root,&mut c,tested) {
                            set_status_message(&ui, i18n::format("Open failed: {0}", &[&e]).into(), true); ui.set_dialog(1);
                        }
                    } else { c.tested = Some(tested); }
                }
                Err(Failure::Message(e)) => {
                    if c.opening { ui.set_active_server(crate::i18n::text("Disconnected").into()); }
                    c.gate.complete(revision,false); ui.set_connection_error(true); ui.set_connection_result(e.clone().into());
                    set_status_message(&ui, i18n::format("Connection failed: {0}. Retry, edit this server, or File > Servers.", &[&e]).into(), true);
                    if c.opening {
                        let id = c.draft.id;
                        if let Some(s) = c.profiles.servers.iter_mut().find(|s| s.id == id) { s.last_status = "Offline".into(); s.last_error = Some(e); }
                        if let Err(e) = c.profiles.save(&root) { set_status_message(&ui, e.into(), true); }
                        rows(&ui,&c); ui.set_dialog(1);
                    }
                }
                Err(Failure::UnknownHost(h)) | Err(Failure::ChangedHost(h)) => {
                    let changed = c.profiles.hosts.contains_key(&h.endpoint);
                    ui.set_host_changed(changed);
                    let previous = c.profiles.hosts.get(&h.endpoint).map(|v| i18n::format("\nPreviously trusted: {0}", &[v])).unwrap_or_default();
                    ui.set_host_question(i18n::format("Host: {0}\nKey type: {1}\nFingerprint: {2}{3}\n\nVerify this fingerprint through a trusted channel before approval. Credentials have not been sent.", &[&h.endpoint, &h.key_type, &h.fingerprint, &previous]).into());
                    c.pending_host = Some(h); ui.set_dialog(3);
                }
            }
        });
    });
}

fn open(
    ui: &ContainerSafeWindow,
    control: &Control,
    state: &SharedState,
    root: &Path,
    server: Server,
) {
    {
        let mut s = state.lock().unwrap();
        if s.busy || s.dirty {
            set_status_message(
                ui,
                "Save selections and wait for active work before switching servers".into(),
                false,
            );
            return;
        }
        let secret = if s.server.as_ref().is_some_and(|v| v.id == server.id) {
            s.secret.clone()
        } else if server.remember {
            credentials::read(server.id).unwrap_or_default()
        } else {
            Secret::default()
        };
        let mut c = control.lock().unwrap();
        c.opening = true;
        c.active = None;
        populate(ui, &mut c, server.clone(), secret);
        s.report = empty_report();
        s.selection = draft(&s.report);
        s.server = None;
        s.secret = Secret::default();
        render(ui, &s);
    }
    ui.set_dialog(0);
    ui.set_active_server(i18n::format("Connecting: {0}", &[&server.name]).into());
    set_status_message(
        ui,
        crate::i18n::text("Connecting to selected server...").into(),
        false,
    );
    if server.authentication == Authentication::Password && ui.get_form_secret().is_empty() {
        ui.set_dialog(1);
        ui.set_connection_result(
            "Enter this server's session password, then Test Connection".into(),
        );
    } else {
        begin(ui, control, state, root);
    }
}

pub(super) fn install(ui: &ContainerSafeWindow, state: SharedState, root: PathBuf) {
    let loaded = Profiles::load(&root);
    let editable = loaded.is_ok();
    let control = Arc::new(Mutex::new(Controller {
        profiles: loaded.unwrap_or_default(),
        editable,
        active: None,
        draft: Server::default(),
        gate: TestGate::default(),
        tested: None,
        pending_host: None,
        opening: false,
    }));
    rows(ui, &control.lock().unwrap());
    {
        let weak = ui.as_weak();
        let control = control.clone();
        let state = state.clone();
        ui.on_form_edited(move || {
            if let Some(ui) = weak.upgrade() {
                if ui.get_test_busy() {
                    return;
                }
                let mut c = control.lock().unwrap();
                c.gate.invalidate();
                c.tested = None;
                c.pending_host = None;
                c.opening = false;
                ui.set_connection_error(false);
                ui.set_test_passed(false);
                ui.set_test_busy(false);
                ui.set_connection_result(
                    crate::i18n::text("Changed — test this configuration again").into(),
                );
                let mut s = state.lock().unwrap();
                s.busy = false;
                render(&ui, &s);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let control = control.clone();
        let state = state.clone();
        let root = root.clone();
        ui.on_test_server(move || {
            if let Some(ui) = weak.upgrade() {
                if !ui.get_test_busy() {
                    begin(&ui, &control, &state, &root);
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        let control = control.clone();
        let state = state.clone();
        let root = root.clone();
        ui.on_trust_host(move || {
            if let Some(ui) = weak.upgrade() {
                let mut c = control.lock().unwrap();
                let Some(h) = c.pending_host.take() else {
                    return;
                };
                let mut next = c.profiles.clone();
                next.hosts.insert(h.endpoint, h.fingerprint);
                if let Err(e) = next.save(&root) {
                    ui.set_host_question(e.into());
                    return;
                }
                c.profiles = next;
                ui.set_dialog(if c.opening { 0 } else { 1 });
                drop(c);
                begin(&ui, &control, &state, &root);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let control = control.clone();
        let state = state.clone();
        let root = root.clone();
        ui.on_save_server(move || {
            if let Some(ui) = weak.upgrade() {
                let mut c = control.lock().unwrap();
                if !c.editable || !c.gate.can_save() || ui.get_test_busy() {
                    return;
                }
                let Ok((current, secret)) = form(&ui, &c) else {
                    return;
                };
                let Some(tested) = c.tested.as_ref() else {
                    return;
                };
                if tested.server != current || tested.secret.0 != secret.0 {
                    c.gate.invalidate();
                    ui.set_connection_error(false);
                    ui.set_test_passed(false);
                    return;
                }
                let mut tested = c.tested.take().unwrap();
                let previous = match credentials::read_optional(current.id) {
                    Ok(value) => value,
                    Err(e)
                        if c.profiles
                            .servers
                            .iter()
                            .any(|s| s.id == current.id && s.remember) =>
                    {
                        ui.set_connection_error(true);
                        ui.set_connection_result(e.into());
                        c.tested = Some(tested);
                        return;
                    }
                    Err(_) => None,
                };
                let credential_result = if current.remember {
                    credentials::write(current.id, &secret)
                } else if c
                    .profiles
                    .servers
                    .iter()
                    .any(|s| s.id == current.id && s.remember)
                {
                    credentials::remove(current.id)
                } else {
                    Ok(())
                };
                if let Err(e) = credential_result {
                    ui.set_connection_error(true);
                    ui.set_connection_result(e.into());
                    c.tested = Some(tested);
                    return;
                }
                tested.server.last_success = Some(now());
                let old = c.profiles.clone();
                let mut next = old.clone();
                next.upsert(tested.server.clone());
                if let Err(e) = next.save(&root) {
                    let rollback = if let Some(old_secret) = previous {
                        credentials::write(current.id, &old_secret)
                    } else {
                        credentials::remove(current.id)
                    };
                    ui.set_connection_error(true);
                    ui.set_connection_result(
                        format!(
                            "Profile save failed: {e}; credential rollback: {}",
                            if rollback.is_ok() {
                                "done"
                            } else {
                                "failed — review OS credential store"
                            }
                        )
                        .into(),
                    );
                    c.tested = Some(tested);
                    return;
                }
                c.profiles = next;
                if let Err(e) = activate(&ui, &state, &root, &mut c, tested) {
                    ui.set_connection_error(true);
                    ui.set_connection_result(
                        i18n::format("Profile saved, but opening failed: {0}", &[&e]).into(),
                    );
                    rows(&ui, &c);
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        let control = control.clone();
        ui.on_browse_key(move || {
            let weak = weak.clone();
            let control = control.clone();
            let revision = control.lock().unwrap().gate.revision;
            std::thread::spawn(move || {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Select SSH private key")
                    .pick_file()
                {
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        if ui.get_dialog() != 1 || control.lock().unwrap().gate.revision != revision
                        {
                            return;
                        }
                        ui.set_form_key(path.to_string_lossy().as_ref().into());
                        ui.invoke_form_edited();
                    });
                }
            });
        });
    }
    {
        let weak = ui.as_weak();
        let control = control.clone();
        let state = state.clone();
        let root = root.clone();
        ui.on_server_action(move |action| { let Some(ui)=weak.upgrade() else {return;};
            if action=="quit" { let _=slint::quit_event_loop(); return; }
            if action=="docs" {
                std::thread::spawn(|| {
                    #[cfg(windows)] let _=std::process::Command::new("explorer.exe").arg("https://github.com/matrixn/ContainerSafe#readme").spawn();
                    #[cfg(target_os="linux")] let _=std::process::Command::new("xdg-open").arg("https://github.com/matrixn/ContainerSafe#readme").spawn();
                    #[cfg(target_os="macos")] let _=std::process::Command::new("open").arg("https://github.com/matrixn/ContainerSafe#readme").spawn();
                }); return;
            }
            if ui.get_test_busy() { return; }
            let mut c=control.lock().unwrap();
            if action=="cancel" {
                c.gate.invalidate(); c.tested=None; c.pending_host=None; c.opening=false;
                ui.set_form_secret("".into()); ui.set_test_busy(false); ui.set_test_passed(false); ui.set_dialog(0);
                let mut s=state.lock().unwrap(); s.busy=false; render(&ui,&s);
                if s.server.is_none() { ui.set_active_server(crate::i18n::text("Disconnected").into()); set_status_message(&ui, crate::i18n::text("Disconnected — File > New Server or Servers").into(), false); }
                return;
            }
            if !c.editable { set_status_message(&ui, crate::i18n::text("Configuration is damaged; repair servers.json before making changes. Original file preserved.").into(), true); return; }
            if state.lock().unwrap().busy { return; }
            if matches!(action.as_str(), "new" | "edit") && state.lock().unwrap().dirty {
                set_status_message(&ui, crate::i18n::text("Save backup selections before editing server profiles").into(), false); return;
            }
            if action=="servers" { rows(&ui,&c); ui.set_selected_server(-1); ui.set_dialog(2); return; }
            if action=="new" { c.opening=false; populate(&ui,&mut c,settings::new_server(&ui),Secret::default()); ui.set_dialog(1); return; }
            let Some(server)=c.profiles.servers.get(ui.get_selected_server() as usize).cloned() else {return;};
            match action.as_str() {
                "edit" => { c.opening=false; populate(&ui,&mut c,server,Secret::default()); ui.set_dialog(1); }
                "default" => { let mut next=c.profiles.clone(); if let Err(e)=next.set_default(server.id).and_then(|_|next.save(&root)) {set_status_message(&ui, e.into(), true);} else {c.profiles=next; rows(&ui,&c);} }
                "remove" => {ui.set_host_question(i18n::format("Remove {0} ({1}) and its stored credentials? Remote data and private-key files stay intact.", &[&server.name,&server.host]).into()); ui.set_dialog(5);}
                "confirm-remove" => {
                    let mut s=state.lock().unwrap();
                    if s.busy || s.dirty {set_status_message(&ui, crate::i18n::text("Save selections before removing a server").into(), false); return;}
                    let previous=match credentials::read_optional(server.id) {
                        Ok(value) => value,
                        Err(e) if server.remember => {ui.set_host_question(e.into()); return;}
                        Err(_) => None,
                    };
                    if server.remember || previous.is_some() {
                        if let Err(e)=credentials::remove(server.id) {ui.set_host_question(e.into()); return;}
                    }
                    let mut next=c.profiles.clone(); next.remove(server.id);
                    if let Err(e)=next.save(&root) { if let Some(secret)=previous { let _=credentials::write(server.id,&secret); } ui.set_host_question(e.into()); return; }
                    c.profiles=next;
                    if s.server.as_ref().is_some_and(|v|v.id==server.id) {c.active=None; s.server=None; s.secret=Secret::default(); s.report=empty_report(); s.selection=draft(&s.report); render(&ui,&s); ui.set_active_server(crate::i18n::text("Disconnected").into());}
                    rows(&ui,&c); ui.set_selected_server(-1); ui.set_dialog(2);
                }
                "open" => {drop(c); open(&ui,&control,&state,&root,server);}
                _ => {}
            }
        });
    }
    {
        let weak = ui.as_weak();
        let control = control.clone();
        let state = state.clone();
        let root = root.clone();
        ui.on_refresh_inventory(move || {
            if let Some(ui) = weak.upgrade() {
                if ui.get_dialog() != 0 {
                    return;
                }
                let server = state.lock().unwrap().server.clone().or_else(|| {
                    let c = control.lock().unwrap();
                    c.profiles
                        .servers
                        .iter()
                        .find(|s| Some(s.id) == c.profiles.default)
                        .cloned()
                });
                if let Some(server) = server {
                    open(&ui, &control, &state, &root, server);
                } else {
                    ui.invoke_server_action("servers".into());
                }
            }
        });
    }
    // Scheduled after the main window has been shown; the splash never waits for SSH.
    let weak = ui.as_weak();
    slint::Timer::single_shot(Duration::from_millis(1), move || {
        if let Some(ui) = weak.upgrade() {
            let c = control.lock().unwrap();
            if !c.editable {
                return;
            }
            let reconnect = if settings::choice(&ui, 6) == 1 {
                last_opened_server(&c.profiles)
            } else {
                None
            };
            let empty = c.profiles.servers.is_empty();
            drop(c);
            if let Some(server) = reconnect {
                open(&ui, &control, &state, &root, server);
            } else {
                ui.invoke_server_action(if empty { "new" } else { "servers" }.into());
            }
        }
    });
}

// Only a successfully opened profile is eligible for automatic reconnection.
fn last_opened_server(profiles: &Profiles) -> Option<Server> {
    profiles
        .servers
        .iter()
        .filter(|server| server.last_opened.is_some())
        .max_by_key(|server| server.last_opened)
        .cloned()
}
#[cfg(test)]
mod startup_tests {
    use super::*;
    #[test]
    fn reconnect_uses_last_opened_instead_of_default_or_failed_attempt() {
        let older = Server {
            last_opened: Some(10),
            ..Server::default()
        };
        let recent = Server {
            last_opened: Some(30),
            ..Server::default()
        };
        let never_opened = Server::default();
        let mut profiles = Profiles {
            default: Some(older.id),
            servers: vec![older, recent.clone(), never_opened],
            ..Profiles::default()
        };
        assert_eq!(last_opened_server(&profiles).unwrap().id, recent.id);
        profiles.servers.clear();
        assert!(last_opened_server(&profiles).is_none());
        profiles.servers.push(Server::default());
        assert!(last_opened_server(&profiles).is_none());
    }
}
