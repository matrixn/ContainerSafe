//! One request per process, designed for invocation over an authenticated SSH channel.
//! All paths and permissions come from the host's configuration, never the request.
use crate::restic::{RepositoryConfig, RepositorySecret, ResticEngine, ResticError};
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use std::io::Read;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;
use zeroize::Zeroizing;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_REQUEST_BYTES: u64 = 64 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub restic_executable: PathBuf,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    pub jobs: Vec<JobConfig>,
}
fn default_timeout() -> u64 {
    300
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JobConfig {
    pub id: Uuid,
    pub repository: RepositoryConfig,
    pub secret_file: PathBuf,
    pub sources: Vec<PathBuf>,
    pub restore_root: PathBuf,
    #[serde(default)]
    pub allowed_operations: Vec<Operation>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Init,
    Backup,
    Snapshots,
    Check,
    Restore,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub protocol_version: u32,
    pub request_id: Uuid,
    pub job_id: Uuid,
    pub action: Action,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Describe {},
    BackupSelected {
        expected_sources: Vec<PathBuf>,
        expected_repository: PathBuf,
    },
    Init {},
    Backup {},
    Snapshots {},
    Check {
        #[serde(default)]
        read_data: bool,
    },
    Restore {
        snapshot_id: String,
        directory: String,
    },
}
impl Action {
    fn operation(&self) -> Operation {
        match self {
            Self::Describe {} | Self::BackupSelected { .. } => Operation::Backup,
            Self::Init {} => Operation::Init,
            Self::Backup {} => Operation::Backup,
            Self::Snapshots {} => Operation::Snapshots,
            Self::Check { .. } => Operation::Check,
            Self::Restore { .. } => Operation::Restore,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    UnsupportedProtocol,
    InvalidConfig,
    InsecureFile,
    UnsupportedPlatform,
    UnknownJob,
    OperationDenied,
    UnsafeRestore,
    SecretUnavailable,
    ResticFailed,
    Internal,
    PlanChanged,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentError {
    pub code: ErrorCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restic_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}
impl From<ErrorCode> for AgentError {
    fn from(code: ErrorCode) -> Self {
        Self {
            code,
            restic_kind: None,
            exit_code: None,
        }
    }
}
impl From<ResticError> for AgentError {
    fn from(error: ResticError) -> Self {
        Self {
            code: ErrorCode::ResticFailed,
            restic_kind: Some(format!("{:?}", error.kind)),
            exit_code: error.exit_code,
        }
    }
}

#[derive(Serialize)]
pub struct Response {
    pub protocol_version: u32,
    pub application_version: &'static str,
    pub request_id: Option<Uuid>,
    #[serde(flatten)]
    pub outcome: Outcome,
}
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    Ok { result: serde_json::Value },
    Error { error: AgentError },
}
impl Response {
    pub fn is_success(&self) -> bool {
        matches!(self.outcome, Outcome::Ok { .. })
    }
    pub fn failure(request_id: Option<Uuid>, error: impl Into<AgentError>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            application_version: crate::VERSION,
            request_id,
            outcome: Outcome::Error {
                error: error.into(),
            },
        }
    }
}

pub fn parse_request(input: &[u8]) -> Result<Request, AgentError> {
    if input.len() as u64 > MAX_REQUEST_BYTES {
        return Err(ErrorCode::InvalidRequest.into());
    }
    let request: Request =
        serde_json::from_slice(input).map_err(|_| AgentError::from(ErrorCode::InvalidRequest))?;
    if request.protocol_version != PROTOCOL_VERSION {
        return Err(ErrorCode::UnsupportedProtocol.into());
    }
    if request.request_id.is_nil() || request.job_id.is_nil() {
        return Err(ErrorCode::InvalidRequest.into());
    }
    Ok(request)
}

pub struct Agent {
    config: AgentConfig,
    engine: ResticEngine,
    config_path: Option<PathBuf>,
}
impl Agent {
    pub fn load(path: &Path) -> Result<Self, AgentError> {
        if !path.is_absolute() {
            return Err(ErrorCode::InvalidConfig.into());
        }
        let input = protected_read(path, false, 1024 * 1024)?;
        let config = serde_json::from_slice(&input)
            .map_err(|_| AgentError::from(ErrorCode::InvalidConfig))?;
        let mut agent = Self::new(config)?;
        agent.config_path = Some(fs::canonicalize(path).map_err(|_| ErrorCode::InvalidConfig)?);
        Ok(agent)
    }
    pub fn new(config: AgentConfig) -> Result<Self, AgentError> {
        if !config.restic_executable.is_absolute()
            || config.timeout_seconds == 0
            || config.timeout_seconds > 86400
        {
            return Err(ErrorCode::InvalidConfig.into());
        }
        let mut ids = std::collections::HashSet::new();
        for job in &config.jobs {
            if job.id.is_nil()
                || !ids.insert(job.id)
                || job.repository.secret_reference.is_nil()
                || !crate::restic::valid_repository(&job.repository.path)
                || !job.secret_file.is_absolute()
                || !job.restore_root.is_absolute()
                || job.sources.iter().any(|p| !p.is_absolute())
            {
                return Err(ErrorCode::InvalidConfig.into());
            }
        }
        let engine = ResticEngine::new(&config.restic_executable)?
            .with_timeout(Duration::from_secs(config.timeout_seconds))?;
        Ok(Self {
            config,
            engine,
            config_path: None,
        })
    }
    pub fn handle(&self, request: Request) -> Response {
        let request_id = Some(request.request_id);
        match self.execute(request) {
            Ok(result) => Response {
                protocol_version: PROTOCOL_VERSION,
                application_version: crate::VERSION,
                request_id,
                outcome: Outcome::Ok { result },
            },
            Err(error) => Response::failure(request_id, error),
        }
    }
    fn execute(&self, request: Request) -> Result<serde_json::Value, AgentError> {
        if request.protocol_version != PROTOCOL_VERSION {
            return Err(ErrorCode::UnsupportedProtocol.into());
        }
        if request.request_id.is_nil() || request.job_id.is_nil() {
            return Err(ErrorCode::InvalidRequest.into());
        }
        let job = self
            .config
            .jobs
            .iter()
            .find(|job| job.id == request.job_id)
            .ok_or(ErrorCode::UnknownJob)?;
        if !job.allowed_operations.contains(&request.action.operation()) {
            return Err(ErrorCode::OperationDenied.into());
        }
        if matches!(request.action, Action::Describe {}) {
            return Ok(serde_json::json!({"repository": job.repository.path,
                "sources": job.sources, "restore_root": job.restore_root,
                "timeout_seconds": self.config.timeout_seconds}));
        }
        if let Action::BackupSelected {
            expected_sources,
            expected_repository,
        } = &request.action
        {
            let actual: std::collections::BTreeSet<_> = job.sources.iter().collect();
            let expected: std::collections::BTreeSet<_> = expected_sources.iter().collect();
            if actual != expected
                || expected.is_empty()
                || expected_repository != &job.repository.path
            {
                return Err(ErrorCode::PlanChanged.into());
            }
        }
        if matches!(
            request.action,
            Action::Backup {} | Action::BackupSelected { .. }
        ) {
            let mut protected = Vec::new();
            for configured_job in &self.config.jobs {
                if let Ok(path) = fs::canonicalize(&configured_job.secret_file) {
                    protected.push(path);
                }
            }
            if let Some(path) = &self.config_path {
                protected.push(path.clone());
            }
            for source in &job.sources {
                let source = fs::canonicalize(source).map_err(|_| ErrorCode::InvalidConfig)?;
                if protected.iter().any(|path| path.starts_with(&source)) {
                    return Err(ErrorCode::InvalidConfig.into());
                }
            }
        }
        // Authorize before opening the password file or executing any child process.
        let target = match &request.action {
            Action::Restore { directory, .. } => Some(restore_target(job, directory)?),
            _ => None,
        };
        let bytes = protected_read(&job.secret_file, true, 16 * 1024)?;
        let password = std::str::from_utf8(&bytes).map_err(|_| ErrorCode::SecretUnavailable)?;
        let password = password
            .strip_suffix("\r\n")
            .or_else(|| password.strip_suffix('\n'))
            .unwrap_or(password);
        if password.contains(['\r', '\n']) {
            return Err(ErrorCode::SecretUnavailable.into());
        }
        let secret = RepositorySecret::new(job.repository.secret_reference, password.to_owned())?;
        let value = match request.action {
            Action::Describe {} => unreachable!(),
            Action::Init {} => {
                self.engine.init(&job.repository, &secret)?;
                Ok(serde_json::json!({"initialized": true}))
            }
            Action::Backup {} | Action::BackupSelected { .. } => {
                serde_json::to_value(self.engine.backup(&job.repository, &secret, &job.sources)?)
            }
            Action::Snapshots {} => {
                serde_json::to_value(self.engine.snapshots(&job.repository, &secret)?)
            }
            Action::Check { read_data } => {
                serde_json::to_value(self.engine.check(&job.repository, &secret, read_data)?)
            }
            Action::Restore { snapshot_id, .. } => serde_json::to_value(self.engine.restore(
                &job.repository,
                &secret,
                &snapshot_id,
                None,
                target.as_ref().ok_or(ErrorCode::Internal)?,
            )?),
        };
        value.map_err(|_| ErrorCode::Internal.into())
    }
}

fn restore_target(job: &JobConfig, directory: &str) -> Result<PathBuf, AgentError> {
    if directory.is_empty()
        || directory.len() > 64
        || !directory
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(ErrorCode::UnsafeRestore.into());
    }
    let root = fs::canonicalize(&job.restore_root).map_err(|_| ErrorCode::UnsafeRestore)?;
    if !root.is_dir() {
        return Err(ErrorCode::UnsafeRestore.into());
    }
    let target = root.join(directory);
    // Restoring into a configured source would contaminate a subsequent backup.
    for source in &job.sources {
        let source = source_location(source)?;
        if target.starts_with(&source) || source.starts_with(&target) {
            return Err(ErrorCode::UnsafeRestore.into());
        }
    }
    Ok(target)
}

// A lost source must not prevent recovery. Resolve existing ancestors, but refuse
// dangling symlinks, unreadable components and ambiguous trailing path segments.
fn source_location(path: &Path) -> Result<PathBuf, AgentError> {
    let mut candidate = path;
    let mut missing = Vec::new();
    loop {
        match fs::canonicalize(candidate) {
            Ok(mut resolved) => {
                for component in missing.into_iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if fs::symlink_metadata(candidate).is_ok() {
                    return Err(ErrorCode::UnsafeRestore.into());
                }
                missing.push(
                    candidate
                        .file_name()
                        .ok_or(ErrorCode::UnsafeRestore)?
                        .to_owned(),
                );
                candidate = candidate.parent().ok_or(ErrorCode::UnsafeRestore)?;
            }
            Err(_) => return Err(ErrorCode::UnsafeRestore.into()),
        }
    }
}

/// Configuration must not be writable by others; secrets must be private files.
/// Linux-only until equivalent file-ownership/ACL checks exist on other systems.
fn protected_read(path: &Path, secret: bool, limit: u64) -> Result<Zeroizing<Vec<u8>>, AgentError> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        let failure = if secret {
            ErrorCode::SecretUnavailable
        } else {
            ErrorCode::InvalidConfig
        };
        let before = fs::symlink_metadata(path).map_err(|_| failure)?;
        if !before.is_file() || before.file_type().is_symlink() {
            return Err(ErrorCode::InsecureFile.into());
        }
        let mut file = fs::File::open(path).map_err(|_| failure)?;
        let metadata = file.metadata().map_err(|_| failure)?;
        let uid = fs::metadata("/proc/self").map_err(|_| failure)?.uid();
        if metadata.ino() != before.ino()
            || metadata.dev() != before.dev()
            || (metadata.uid() != uid && metadata.uid() != 0)
            || metadata.mode() & (if secret { 0o077 } else { 0o022 }) != 0
            || !metadata.is_file()
        {
            return Err(ErrorCode::InsecureFile.into());
        }
        if metadata.len() > limit {
            return Err(failure.into());
        }
        let mut bytes = Zeroizing::new(Vec::new());
        (&mut file)
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| failure)?;
        if bytes.len() as u64 > limit {
            return Err(failure.into());
        }
        Ok(bytes)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, secret, limit);
        Err(ErrorCode::UnsupportedPlatform.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(root: &Path) -> AgentConfig {
        AgentConfig {
            restic_executable: root.join("never-execute"),
            timeout_seconds: 30,
            jobs: vec![JobConfig {
                id: Uuid::new_v4(),
                repository: RepositoryConfig {
                    path: root.join("repository"),
                    secret_reference: Uuid::new_v4(),
                },
                secret_file: root.join("password.secret"),
                sources: vec![root.join("source")],
                restore_root: root.join("restores"),
                allowed_operations: vec![],
            }],
        }
    }
    fn request(job_id: Uuid, action: Action) -> Request {
        Request {
            protocol_version: PROTOCOL_VERSION,
            request_id: Uuid::new_v4(),
            job_id,
            action,
        }
    }
    fn code(response: Response) -> ErrorCode {
        match response.outcome {
            Outcome::Error { error } => error.code,
            _ => panic!("unexpected execution"),
        }
    }

    #[test]
    fn desktop_plan_binding_rejects_unapproved_sources_and_destinations_before_secrets() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = config(directory.path());
        let id = config.jobs[0].id;
        let repository = config.jobs[0].repository.path.clone();
        let sources = config.jobs[0].sources.clone();
        config.jobs[0].allowed_operations = vec![Operation::Backup];
        let agent = Agent::new(config).unwrap();
        assert!(agent.handle(request(id, Action::Describe {})).is_success());
        assert_eq!(
            code(agent.handle(request(
                id,
                Action::BackupSelected {
                    expected_sources: vec![directory.path().join("not-approved")],
                    expected_repository: repository,
                }
            ))),
            ErrorCode::PlanChanged
        );
        assert_eq!(
            code(agent.handle(request(
                id,
                Action::BackupSelected {
                    expected_sources: sources,
                    expected_repository: directory.path().join("other-repository"),
                }
            ))),
            ErrorCode::PlanChanged
        );
    }

    #[test]
    fn requests_are_versioned_bounded_and_reject_injected_paths_or_commands() {
        let base = serde_json::json!({"protocol_version":1,"request_id":Uuid::new_v4(),
            "job_id":Uuid::new_v4(),"action":{"operation":"backup"}});
        assert!(parse_request(&serde_json::to_vec(&base).unwrap()).is_ok());
        for action in [
            serde_json::json!({"operation":"backup","sources":["/etc"]}),
            serde_json::json!({"operation":"exec","command":"anything"}),
            serde_json::json!({"operation":"restore","snapshot_id":"latest","directory":"x","target":"/etc"}),
        ] {
            let mut input = base.clone();
            input["action"] = action;
            assert_eq!(
                parse_request(&serde_json::to_vec(&input).unwrap())
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidRequest
            );
        }
        let mut input = base.clone();
        input["protocol_version"] = 99.into();
        assert_eq!(
            parse_request(&serde_json::to_vec(&input).unwrap())
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedProtocol
        );
        input = base;
        input["password"] = "secret-must-not-appear".into();
        let error = parse_request(&serde_json::to_vec(&input).unwrap()).unwrap_err();
        assert!(!serde_json::to_string(&error)
            .unwrap()
            .contains("secret-must"));
        assert_eq!(
            parse_request(&vec![b' '; MAX_REQUEST_BYTES as usize + 1])
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn policy_denies_before_secrets_or_processes_and_preserves_request_identity() {
        let directory = tempfile::tempdir().unwrap();
        let config = config(directory.path());
        let id = config.jobs[0].id;
        let agent = Agent::new(config).unwrap();
        assert_eq!(
            code(agent.handle(request(id, Action::Init {}))),
            ErrorCode::OperationDenied
        );
        assert_eq!(
            code(agent.handle(request(Uuid::new_v4(), Action::Snapshots {}))),
            ErrorCode::UnknownJob
        );
        let request = request(id, Action::Backup {});
        let identity = request.request_id;
        let response = agent.handle(request);
        assert_eq!(response.request_id, Some(identity));
        assert_eq!(code(response), ErrorCode::OperationDenied);
        assert!(!directory.path().join("repository").exists());
    }

    #[test]
    fn duplicate_jobs_relative_paths_and_unbounded_timeouts_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let mut value = config(directory.path());
        value.jobs[0].sources = vec!["relative".into()];
        assert!(matches!(
            Agent::new(value),
            Err(AgentError {
                code: ErrorCode::InvalidConfig,
                ..
            })
        ));
        let mut value = config(directory.path());
        value.timeout_seconds = 86401;
        assert!(Agent::new(value).is_err());
        let mut value = config(directory.path());
        let duplicate: JobConfig = serde_json::from_value(serde_json::json!({"id":value.jobs[0].id,
            "repository":{"path":directory.path().join("repo2"),"secret_reference":Uuid::new_v4()},
            "secret_file":directory.path().join("another.secret"),"sources":[],"restore_root":directory.path()})).unwrap();
        value.jobs.push(duplicate);
        assert!(Agent::new(value).is_err());
    }

    #[test]
    fn restore_targets_cannot_escape_or_contaminate_sources() {
        let directory = tempfile::tempdir().unwrap();
        let config = config(directory.path());
        let job = &config.jobs[0];
        fs::create_dir(&job.restore_root).unwrap();
        fs::create_dir(&job.sources[0]).unwrap();
        for name in ["", "..", "../escape", "/etc", "C:\\data", "a/b", "a b"] {
            assert_eq!(
                restore_target(job, name).unwrap_err().code,
                ErrorCode::UnsafeRestore
            );
        }
        assert!(restore_target(job, "verification-1")
            .unwrap()
            .starts_with(fs::canonicalize(&job.restore_root).unwrap()));
        fs::remove_dir(&job.sources[0]).unwrap();
        assert!(restore_target(job, "recover-missing-source").is_ok());
        fs::create_dir(&job.sources[0]).unwrap();
        let mut job = config.jobs.into_iter().next().unwrap();
        job.restore_root = job.sources[0].clone();
        assert_eq!(
            restore_target(&job, "verification-1").unwrap_err().code,
            ErrorCode::UnsafeRestore
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn protected_files_require_ownership_permissions_and_no_symlinks() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("test.secret");
        fs::write(&file, "synthetic-password\n").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(protected_read(&file, true, 100).is_ok());
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            protected_read(&file, true, 100).unwrap_err().code,
            ErrorCode::InsecureFile
        );
        assert!(protected_read(&file, false, 100).is_ok());
        fs::set_permissions(&file, fs::Permissions::from_mode(0o666)).unwrap();
        assert_eq!(
            protected_read(&file, false, 100).unwrap_err().code,
            ErrorCode::InsecureFile
        );
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        let link = directory.path().join("link");
        symlink(&file, &link).unwrap();
        assert_eq!(
            protected_read(&link, true, 100).unwrap_err().code,
            ErrorCode::InsecureFile
        );
        assert_eq!(
            protected_read(&file, true, 2).unwrap_err().code,
            ErrorCode::SecretUnavailable
        );
    }
}
