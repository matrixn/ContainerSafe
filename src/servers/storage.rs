use super::model::Server;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct Profiles {
    pub servers: Vec<Server>,
    pub default: Option<Uuid>,
    pub hosts: BTreeMap<String, String>,
}

pub fn config_root() -> Result<PathBuf, String> {
    // Explicit process-local override for isolated packaging/startup checks.
    // Windows known-folder APIs do not necessarily follow APPDATA overrides.
    if let Some(value) = std::env::var_os("CONTAINERSAFE_CONFIG_DIR") {
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err("CONTAINERSAFE_CONFIG_DIR must be an absolute path".into());
        }
        return Ok(path);
    }
    directories::BaseDirs::new()
        .map(|d| d.config_dir().join("ContainerSafe"))
        .ok_or_else(|| "No platform configuration directory available".into())
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid configuration path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.error.to_string())?;
    Ok(())
}

impl Profiles {
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join("servers.json");
        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.to_string()),
        };
        let profiles: Self = serde_json::from_slice(&data)
            .map_err(|e| format!("Profiles could not be read; original file preserved: {e}"))?;
        let mut ids = std::collections::HashSet::new();
        if profiles.servers.iter().any(|s| !ids.insert(s.id))
            || profiles.default.is_some_and(|id| !ids.contains(&id))
        {
            return Err(
                "Invalid duplicate/default server identity; original file preserved".into(),
            );
        }
        Ok(profiles)
    }
    pub fn save(&self, root: &Path) -> Result<(), String> {
        atomic_write(
            &root.join("servers.json"),
            &serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
    }
    pub fn upsert(&mut self, server: Server) {
        if self.servers.is_empty() {
            self.default = Some(server.id);
        }
        if let Some(previous) = self.servers.iter_mut().find(|s| s.id == server.id) {
            *previous = server;
        } else {
            self.servers.push(server);
        }
    }
    pub fn remove(&mut self, id: Uuid) {
        self.servers.retain(|s| s.id != id);
        if self.default == Some(id) {
            self.default = None;
        }
    }
    pub fn set_default(&mut self, id: Uuid) -> Result<(), String> {
        if !self.servers.iter().any(|s| s.id == id) {
            return Err("Unknown server".into());
        }
        self.default = Some(id);
        Ok(())
    }
}

pub fn server_dir(root: &Path, id: Uuid) -> PathBuf {
    root.join("servers").join(id.to_string())
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DataIdentity {
    pub server_id: Uuid,
    pub host: String,
    pub port: u16,
    pub host_key: String,
}

impl DataIdentity {
    pub fn new(server: &Server, hosts: &BTreeMap<String, String>) -> Result<Self, String> {
        let endpoint = format!("[{}]:{}", server.host.to_ascii_lowercase(), server.port);
        Ok(Self {
            server_id: server.id,
            host: server.host.to_ascii_lowercase(),
            port: server.port,
            host_key: hosts
                .get(&endpoint)
                .ok_or("Host identity has not been trusted")?
                .clone(),
        })
    }
    pub fn matches(&self, directory: &Path) -> bool {
        std::fs::read(directory.join("server-identity.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Self>(&b).ok())
            .is_some_and(|v| v == *self)
    }
    pub fn save(&self, directory: &Path) -> Result<(), String> {
        atomic_write(
            &directory.join("server-identity.json"),
            &serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
    }
}

/// Explicit import only. Never called during first launch or server creation.
pub fn import_legacy(root: &Path, id: Uuid, legacy: &Path) -> Result<(), String> {
    let target = server_dir(root, id);
    if target.exists() {
        return Err("Import requires a fresh server data directory".into());
    }
    std::fs::create_dir_all(target.parent().ok_or("Invalid server data path")?)
        .map_err(|e| e.to_string())?;
    let staging = tempfile::tempdir_in(target.parent().unwrap()).map_err(|e| e.to_string())?;
    for name in ["backup-plan.json", "backup-selection.json"] {
        let data = std::fs::read(legacy.join(name)).map_err(|e| e.to_string())?;
        serde_json::from_slice::<serde_json::Value>(&data).map_err(|e| e.to_string())?;
        atomic_write(&staging.path().join(name), &data)?;
    }
    std::fs::rename(staging.path(), &target).map_err(|e| e.to_string())?;
    Ok(())
}
