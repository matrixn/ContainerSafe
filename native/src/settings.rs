//! Local preferences. Never contains credentials or changes existing server profiles.
use super::*;
use containersafe::servers::model::DockerAccess;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Preferences {
    schema_version: u32,
    language: usize,
    start_page: usize,
    source_filter: usize,
    ssh_port: usize,
    timeout: usize,
    docker: usize,
    loading_animation: usize,
    auto_connect_last_server: usize,
    #[serde(default, skip_serializing)]
    auto_connect: Option<usize>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            schema_version: 1,
            language: 0,
            start_page: 0,
            source_filter: 0,
            ssh_port: 0,
            timeout: 0,
            docker: 0,
            loading_animation: 1,
            auto_connect_last_server: 0,
            auto_connect: None,
        }
    }
}
const COUNTS: [usize; 8] = [2, 2, 3, 3, 4, 3, 2, 10];
impl Preferences {
    pub(super) fn language(&self) -> usize {
        self.language
    }
    fn choices(&self) -> [usize; 8] {
        [
            self.language,
            self.start_page,
            self.source_filter,
            self.ssh_port,
            self.timeout,
            self.docker,
            self.auto_connect_last_server,
            self.loading_animation,
        ]
    }
    fn from_choices(values: [usize; 8]) -> Result<Self, String> {
        if values.iter().zip(COUNTS).any(|(v, count)| *v >= count) {
            return Err("Invalid preference value".into());
        }
        Ok(Self {
            schema_version: 1,
            language: values[0],
            start_page: values[1],
            source_filter: values[2],
            ssh_port: values[3],
            timeout: values[4],
            docker: values[5],
            auto_connect_last_server: values[6],
            loading_animation: values[7],
            auto_connect: None,
        })
    }
    pub(super) fn load(root: &Path) -> Result<Self, String> {
        let path = root.join("settings.json");
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.to_string()),
        };
        let result: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if result.schema_version != 1 {
            return Err("Unsupported settings schema".into());
        }
        Self::from_choices(result.choices())
    }
    fn save(&self, root: &Path) -> Result<(), String> {
        let data = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        storage::atomic_write(&root.join("settings.json"), &data)
    }
}
fn rows(value: &Preferences, states: &[i32; 8]) -> ModelRc<PreferenceEntry> {
    let specs: [(&str, &str, &[&str]); 8] = [
        ("Language", "Choose the interface language. Changes apply immediately.", &["English", "Română"]),
        ("Start page", "Choose the workspace shown when ContainerSafe starts.", &["Containers", "Backup plans"]),
        ("Initial source filter", "Start with all sources, storage only, or databases only. You can change the filter in Containers.", &["All sources", "Storage", "Databases"]),
        ("Default SSH port", "Used when adding a new server. Existing profiles keep their own port.", &["22", "2022", "2222"]),
        ("Default connection timeout", "Maximum wait for SSH network operations on new profiles. Existing profiles are unchanged.", &["10 seconds", "30 seconds", "60 seconds", "120 seconds"]),
        ("Default Docker access", "Automatic tries direct Docker, then passwordless sudo. Direct uses the SSH account. Sudo requires existing passwordless permissions.", &["Automatic (recommended)", "Direct Docker CLI", "Passwordless sudo -n"]),
        ("Reconnect to last server", "Off by default: show your saved servers at startup, or Add server if none exist. On: reconnect to the last successfully opened server. Host identity is always checked.", &["Off", "On"]),
        ("Loading animation", "Choose the animation used while connecting. Preview updates immediately; Breath is the default. Your choice is saved automatically.", &["1 · Orbit", "2 · Breath", "3 · Silk wave", "4 · Constellation", "5 · Horizon", "6 · Ripple", "7 · Pendulum", "8 · Lotus", "9 · Cascade", "10 · Infinity"]),
    ];
    ModelRc::new(VecModel::from(
        specs
            .iter()
            .enumerate()
            .map(|(i, (title, description, options))| PreferenceEntry {
                title: crate::i18n::text(title).into(),
                description: crate::i18n::text(description).into(),
                options: ModelRc::new(VecModel::from(
                    options
                        .iter()
                        .map(|v| crate::i18n::text(v).into())
                        .collect::<Vec<slint::SharedString>>(),
                )),
                choice: value.choices()[i] as i32,
                state: states[i],
            })
            .collect::<Vec<_>>(),
    ))
}
pub(super) fn choice(ui: &ContainerSafeWindow, index: usize) -> usize {
    ui.get_preferences()
        .row_data(index)
        .map(|row| row.choice.max(0) as usize)
        .unwrap_or(0)
}
pub(super) fn new_server(ui: &ContainerSafeWindow) -> Server {
    Server {
        port: [22, 2022, 2222][choice(ui, 3).min(2)],
        timeout_seconds: [10, 30, 60, 120][choice(ui, 4).min(3)],
        docker_access: [
            DockerAccess::Automatic,
            DockerAccess::Direct,
            DockerAccess::Sudo,
        ][choice(ui, 5).min(2)],
        ..Server::default()
    }
}
pub(super) fn install(ui: &ContainerSafeWindow, root: PathBuf, state: SharedState) {
    let loaded = Preferences::load(&root);
    let editable = loaded.is_ok();
    let initial = loaded.as_ref().cloned().unwrap_or_else(|_| Preferences {
        auto_connect_last_server: 0,
        ..Preferences::default()
    });
    crate::i18n::set_language(initial.language);
    slint::select_bundled_translation(if initial.language == 1 { "ro" } else { "" })
        .expect("English and Romanian are bundled");
    ui.set_page(initial.start_page as i32);
    ui.set_entry_kind(initial.source_filter as i32);
    ui.set_preferences(rows(&initial, &[0; 8]));
    ui.set_loading_animation(initial.loading_animation as i32);
    ui.set_settings_editable(editable);
    if let Err(e) = loaded {
        ui.set_settings_error(format!("{} {e}", crate::i18n::text("Settings could not be loaded. The original file is preserved; editing is disabled.")).into());
    }
    let current = Rc::new(RefCell::new(initial));
    let saved = Rc::new(RefCell::new([0; 8]));
    let weak = ui.as_weak();
    ui.on_preference_changed(move |index, value| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if !editable || index < 0 || index as usize >= 8 || value < 0 {
            return;
        }
        let index = index as usize;
        let mut choices = current.borrow().choices();
        choices[index] = value as usize;
        let Ok(next) = Preferences::from_choices(choices) else {
            return;
        };
        if next == *current.borrow() {
            return;
        }
        match next.save(&root) {
            Ok(()) => {
                *current.borrow_mut() = next.clone();
                saved.borrow_mut()[index] = 1;
                ui.set_settings_error("".into());
                if index == 0 {
                    crate::i18n::set_language(next.language);
                    slint::select_bundled_translation(if next.language == 1 { "ro" } else { "" })
                        .expect("English and Romanian are bundled");
                    ui.set_status_message(
                        i18n::retranslate(ui.get_status_message().as_str()).into(),
                    );
                    ui.set_validation_message(
                        i18n::retranslate(ui.get_validation_message().as_str()).into(),
                    );
                    ui.set_active_server(i18n::retranslate(ui.get_active_server().as_str()).into());
                    ui.set_ssh_key_compatibility(
                        i18n::text(containersafe::servers::model::SSH_KEY_COMPATIBILITY).into(),
                    );
                }
            }
            Err(e) => {
                saved.borrow_mut()[index] = 2;
                ui.set_settings_error(
                    format!("{} {e}", crate::i18n::text("Could not save settings:")).into(),
                );
            }
        }
        ui.set_loading_animation(current.borrow().loading_animation as i32);
        ui.set_preferences(rows(&current.borrow(), &saved.borrow()));
        if let Ok(state) = state.lock() {
            render(&ui, &state);
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let defaults = Preferences::load(dir.path()).unwrap();
        assert_eq!(defaults, Preferences::default());
        let changed = Preferences::from_choices([1, 1, 2, 1, 3, 2, 0, 9]).unwrap();
        changed.save(dir.path()).unwrap();
        assert_eq!(Preferences::load(dir.path()).unwrap(), changed);
    }
    #[test]
    fn reconnect_requires_explicit_opt_in_after_upgrade() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(Preferences::default().choices()[6], 0);
        fs::write(&path, r#"{"language":1,"auto_connect":1}"#).unwrap();
        let migrated = Preferences::load(dir.path()).unwrap();
        assert_eq!(migrated.language(), 1);
        assert_eq!(migrated.choices()[6], 0);
        fs::write(&path, r#"{"auto_connect_last_server":1}"#).unwrap();
        assert_eq!(Preferences::load(dir.path()).unwrap().choices()[6], 1);
    }
    #[test]
    fn invalid_files_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        for content in [
            "broken",
            r#"{"language":2}"#,
            r#"{"schema_version":2}"#,
            r#"{"unknown":true}"#,
        ] {
            fs::write(dir.path().join("settings.json"), content).unwrap();
            assert!(Preferences::load(dir.path()).is_err());
            assert_eq!(
                fs::read_to_string(dir.path().join("settings.json")).unwrap(),
                content
            );
        }
    }
    #[test]
    fn animation_defaults_migration_and_validation() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Preferences::load(dir.path()).unwrap().loading_animation, 1);
        fs::write(dir.path().join("settings.json"), r#"{"language":1}"#).unwrap();
        assert_eq!(Preferences::load(dir.path()).unwrap().loading_animation, 1);
        for animation in 0..10 {
            let value = Preferences {
                loading_animation: animation,
                ..Preferences::default()
            };
            value.save(dir.path()).unwrap();
            assert_eq!(
                Preferences::load(dir.path()).unwrap().loading_animation,
                animation
            );
        }
        fs::write(
            dir.path().join("settings.json"),
            r#"{"loading_animation":10}"#,
        )
        .unwrap();
        assert!(Preferences::load(dir.path()).is_err());
    }
    #[test]
    fn saving_failure_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("settings.json")).unwrap();
        assert!(Preferences::default().save(dir.path()).is_err());
    }
}
