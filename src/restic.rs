//! Local Restic operations shared by the CLI, desktop and future agent.
//! No Docker access, scheduling, shell commands or automatic secret storage.
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepositoryConfig {
    pub path: PathBuf,
    pub secret_reference: Uuid,
}

/// A caller supplies the secret matching the persisted reference. Never serialize it.
pub struct RepositorySecret {
    reference: Uuid,
    password: Zeroizing<String>,
}
impl RepositorySecret {
    pub fn new(reference: Uuid, password: String) -> Result<Self, ResticError> {
        if password.is_empty() || password.contains('\0') || reference.is_nil() {
            return Err(error("credentials", ResticErrorKind::InvalidConfiguration));
        }
        Ok(Self {
            reference,
            password: Zeroizing::new(password),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SnapshotInfo {
    pub id: String,
    pub time: String,
    pub hostname: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BackupResult {
    pub snapshot_id: String,
    pub files_new: u64,
    pub files_changed: u64,
    pub files_unmodified: u64,
    pub total_files_processed: u64,
    pub total_bytes_processed: u64,
    pub data_added: u64,
    pub total_duration: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CheckResult {
    pub all_data_read: bool,
    pub elapsed_seconds: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct RestoreResult {
    pub snapshot_id: String,
    pub target: PathBuf,
    pub files_restored: u64,
    pub bytes_restored: u64,
    /// Restic --verify completed successfully; lab additionally compares bytes.
    pub content_verified: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResticErrorKind {
    InvalidConfiguration,
    SecretMismatch,
    MissingExecutable,
    Io,
    Timeout,
    OutputLimit,
    RepositoryExists,
    RepositoryMissing,
    RepositoryLocked,
    WrongPassword,
    IncompleteBackup,
    CommandFailed,
    InvalidResponse,
    InvalidSnapshot,
    UnsafeSource,
    UnsafeRestore,
}

#[derive(Debug)]
pub struct ResticError {
    pub operation: &'static str,
    pub kind: ResticErrorKind,
    pub exit_code: Option<i32>,
}
fn error(operation: &'static str, kind: ResticErrorKind) -> ResticError {
    ResticError {
        operation,
        kind,
        exit_code: None,
    }
}
impl std::fmt::Display for ResticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Restic {}: {:?}", self.operation, self.kind)?;
        if let Some(code) = self.exit_code {
            write!(f, " (exit {code})")?;
        }
        Ok(())
    }
}
impl std::error::Error for ResticError {}

pub struct ResticEngine {
    executable: PathBuf,
    timeout: Duration,
    output_limit: u64,
}
impl ResticEngine {
    /// Pass a configured path, or explicitly pass "restic" to use PATH lookup.
    pub fn new(executable: impl Into<PathBuf>) -> Result<Self, ResticError> {
        let executable = executable.into();
        if executable.as_os_str().is_empty() {
            return Err(error("configure", ResticErrorKind::InvalidConfiguration));
        }
        Ok(Self {
            executable,
            timeout: Duration::from_secs(300),
            output_limit: 16 * 1024 * 1024,
        })
    }
    pub fn with_timeout(mut self, timeout: Duration) -> Result<Self, ResticError> {
        if timeout.is_zero() {
            return Err(error("configure", ResticErrorKind::InvalidConfiguration));
        }
        self.timeout = timeout;
        Ok(self)
    }

    fn validate(
        &self,
        repo: &RepositoryConfig,
        secret: &RepositorySecret,
    ) -> Result<(), ResticError> {
        if !repo.path.is_absolute() || repo.secret_reference.is_nil() {
            return Err(error("configure", ResticErrorKind::InvalidConfiguration));
        }
        if repo.secret_reference != secret.reference {
            return Err(error("credentials", ResticErrorKind::SecretMismatch));
        }
        Ok(())
    }

    fn command(
        &self,
        repo: &RepositoryConfig,
        secret: &RepositorySecret,
        operation: &'static str,
        args: &[OsString],
    ) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .args(["--json", "--no-cache", "--repo"])
            .arg(restic_path(&repo.path))
            .arg(operation)
            .args(args)
            .env("RESTIC_PASSWORD", secret.password.as_str())
            .env_remove("RESTIC_PASSWORD_FILE")
            .env_remove("RESTIC_PASSWORD_COMMAND")
            .env_remove("RESTIC_REPOSITORY")
            .env_remove("RESTIC_REPOSITORY_FILE")
            .env_remove("RESTIC_HOST")
            .stdin(Stdio::null());
        command
    }

    fn run(
        &self,
        repo: &RepositoryConfig,
        secret: &RepositorySecret,
        operation: &'static str,
        args: &[OsString],
    ) -> Result<Vec<u8>, ResticError> {
        self.validate(repo, secret)?;
        self.execute(self.command(repo, secret, operation, args), operation)
    }

    fn execute(
        &self,
        mut command: Command,
        operation: &'static str,
    ) -> Result<Vec<u8>, ResticError> {
        // File-backed capture avoids full pipes/deadlocks. Raw stderr is never retained
        // or included in errors. Bound capture size and kill/reap on timeout or overflow.
        let io_error = || error(operation, ResticErrorKind::Io);
        let mut capture = tempfile::tempfile().map_err(|_| io_error())?;
        command
            .stdout(Stdio::from(capture.try_clone().map_err(|_| io_error())?))
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(|e| {
            error(
                operation,
                if e.kind() == std::io::ErrorKind::NotFound {
                    ResticErrorKind::MissingExecutable
                } else {
                    ResticErrorKind::Io
                },
            )
        })?;
        let started = Instant::now();
        let status = loop {
            let too_large = capture
                .metadata()
                .map(|m| m.len() > self.output_limit)
                .unwrap_or(true);
            let problem = if too_large {
                Some(ResticErrorKind::OutputLimit)
            } else if started.elapsed() >= self.timeout {
                Some(ResticErrorKind::Timeout)
            } else {
                None
            };
            if let Some(kind) = problem {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error(operation, kind));
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(io_error());
                }
            }
        };
        if !status.success() {
            let kind = match status.code() {
                Some(3) if operation == "backup" => ResticErrorKind::IncompleteBackup,
                Some(10) => ResticErrorKind::RepositoryMissing,
                Some(11) => ResticErrorKind::RepositoryLocked,
                Some(12) => ResticErrorKind::WrongPassword,
                _ => ResticErrorKind::CommandFailed,
            };
            return Err(ResticError {
                operation,
                kind,
                exit_code: status.code(),
            });
        }
        if capture.metadata().map_err(|_| io_error())?.len() > self.output_limit {
            return Err(error(operation, ResticErrorKind::OutputLimit));
        }
        capture.seek(SeekFrom::Start(0)).map_err(|_| io_error())?;
        let mut output = Vec::new();
        capture
            .take(self.output_limit + 1)
            .read_to_end(&mut output)
            .map_err(|_| io_error())?;
        Ok(output)
    }

    pub fn init(
        &self,
        repo: &RepositoryConfig,
        secret: &RepositorySecret,
    ) -> Result<(), ResticError> {
        self.validate(repo, secret)?;
        if repo.path.exists() {
            let mut entries = fs::read_dir(&repo.path)
                .map_err(|_| error("init", ResticErrorKind::RepositoryExists))?;
            if entries.next().is_some() {
                return Err(error("init", ResticErrorKind::RepositoryExists));
            }
        }
        self.run(repo, secret, "init", &[])?;
        Ok(())
    }

    pub fn snapshots(
        &self,
        repo: &RepositoryConfig,
        secret: &RepositorySecret,
    ) -> Result<Vec<SnapshotInfo>, ResticError> {
        let output = self.run(repo, secret, "snapshots", &[])?;
        let snapshots: Vec<SnapshotInfo> = serde_json::from_slice(&output)
            .map_err(|_| error("snapshots", ResticErrorKind::InvalidResponse))?;
        if snapshots.iter().any(|s| !valid_snapshot(&s.id)) {
            return Err(error("snapshots", ResticErrorKind::InvalidResponse));
        }
        Ok(snapshots)
    }

    pub fn backup(
        &self,
        repo: &RepositoryConfig,
        secret: &RepositorySecret,
        sources: &[PathBuf],
    ) -> Result<BackupResult, ResticError> {
        self.validate(repo, secret)?;
        let repository = fs::canonicalize(&repo.path)
            .map_err(|_| error("backup", ResticErrorKind::RepositoryMissing))?;
        if sources.is_empty() {
            return Err(error("backup", ResticErrorKind::UnsafeSource));
        }
        let mut args = vec![OsString::from("--")];
        for source in sources {
            if !source.is_absolute() {
                return Err(error("backup", ResticErrorKind::UnsafeSource));
            }
            let source = fs::canonicalize(source)
                .map_err(|_| error("backup", ResticErrorKind::UnsafeSource))?;
            if source.starts_with(&repository) || repository.starts_with(&source) {
                return Err(error("backup", ResticErrorKind::UnsafeSource));
            }
            args.push(restic_path(&source).into_os_string());
        }
        let output = self.run(repo, secret, "backup", &args)?;
        let result: BackupResult = summary(&output, "backup")?;
        if !valid_snapshot(&result.snapshot_id) {
            return Err(error("backup", ResticErrorKind::InvalidResponse));
        }
        Ok(result)
    }

    pub fn check(
        &self,
        repo: &RepositoryConfig,
        secret: &RepositorySecret,
        all_data: bool,
    ) -> Result<CheckResult, ResticError> {
        let started = Instant::now();
        let args = if all_data {
            vec![OsString::from("--read-data")]
        } else {
            vec![]
        };
        let output = self.run(repo, secret, "check", &args)?;
        #[derive(Deserialize)]
        struct CheckSummary {
            num_errors: u64,
        }
        let result: CheckSummary = summary(&output, "check")?;
        if result.num_errors != 0 {
            return Err(error("check", ResticErrorKind::CommandFailed));
        }
        Ok(CheckResult {
            all_data_read: all_data,
            elapsed_seconds: started.elapsed().as_secs_f64(),
        })
    }

    /// Restore an exact snapshot (optionally a POSIX subtree) into a NEW directory.
    /// No latest selector, overwrite, delete, repair, prune or unlock operations.
    pub fn restore(
        &self,
        repo: &RepositoryConfig,
        secret: &RepositorySecret,
        snapshot_id: &str,
        subtree: Option<&str>,
        target: &Path,
    ) -> Result<RestoreResult, ResticError> {
        self.validate(repo, secret)?;
        if !valid_snapshot(snapshot_id) {
            return Err(error("restore", ResticErrorKind::InvalidSnapshot));
        }
        let mut selector = snapshot_id.to_owned();
        if let Some(subtree) = subtree {
            if !subtree.starts_with('/')
                || subtree.contains(['\0', '\\'])
                || subtree.split('/').any(|s| s == ".." || s == ".")
            {
                return Err(error("restore", ResticErrorKind::InvalidSnapshot));
            }
            selector.push(':');
            selector.push_str(subtree);
        }
        if !target.is_absolute() || target.exists() {
            return Err(error("restore", ResticErrorKind::UnsafeRestore));
        }
        let parent = target
            .parent()
            .and_then(|p| fs::canonicalize(p).ok())
            .ok_or_else(|| error("restore", ResticErrorKind::UnsafeRestore))?;
        let name = target
            .file_name()
            .ok_or_else(|| error("restore", ResticErrorKind::UnsafeRestore))?;
        let target = parent.join(name);
        let repository = fs::canonicalize(&repo.path)
            .map_err(|_| error("restore", ResticErrorKind::RepositoryMissing))?;
        if target.starts_with(&repository) || repository.starts_with(&target) {
            return Err(error("restore", ResticErrorKind::UnsafeRestore));
        }
        // Atomic create refuses a racing existing target. On failure keep partial
        // files for inspection; never label a failed restore as verified.
        fs::create_dir(&target).map_err(|_| error("restore", ResticErrorKind::UnsafeRestore))?;
        let args = vec![
            OsString::from("--target"),
            restic_path(&target).into_os_string(),
            "--overwrite".into(),
            "never".into(),
            "--verify".into(),
            selector.into(),
        ];
        let output = self.run(repo, secret, "restore", &args)?;
        #[derive(Deserialize)]
        struct RestoreSummary {
            files_restored: u64,
            bytes_restored: u64,
            #[serde(default)]
            files_skipped: u64,
        }
        let result: RestoreSummary = summary(&output, "restore")?;
        if result.files_skipped != 0 {
            return Err(error("restore", ResticErrorKind::UnsafeRestore));
        }
        Ok(RestoreResult {
            snapshot_id: snapshot_id.into(),
            target,
            files_restored: result.files_restored,
            bytes_restored: result.bytes_restored,
            content_verified: true,
        })
    }
}

// Rust canonicalize uses extended Windows prefixes, which Restic records as a
// different snapshot tree. Remove only that prefix, preserving non-UTF8 paths.
fn restic_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        let prefix: Vec<u16> = r"\\?\".encode_utf16().collect();
        let unc: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
        if wide.starts_with(&unc) {
            let mut normalized: Vec<u16> = r"\\".encode_utf16().collect();
            normalized.extend_from_slice(&wide[unc.len()..]);
            return OsString::from_wide(&normalized).into();
        }
        if wide.starts_with(&prefix) {
            return OsString::from_wide(&wide[prefix.len()..]).into();
        }
    }
    path.to_owned()
}

fn valid_snapshot(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn summary<T: serde::de::DeserializeOwned>(
    output: &[u8],
    operation: &'static str,
) -> Result<T, ResticError> {
    let mut result = None;
    for line in output
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
    {
        let value: serde_json::Value = serde_json::from_slice(line)
            .map_err(|_| error(operation, ResticErrorKind::InvalidResponse))?;
        if value["message_type"] == "summary" {
            if result.is_some() {
                return Err(error(operation, ResticErrorKind::InvalidResponse));
            }
            result = Some(
                serde_json::from_value(value)
                    .map_err(|_| error(operation, ResticErrorKind::InvalidResponse))?,
            );
        }
    }
    result.ok_or_else(|| error(operation, ResticErrorKind::InvalidResponse))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configuration(directory: &Path) -> (RepositoryConfig, RepositorySecret) {
        let reference = Uuid::new_v4();
        (
            RepositoryConfig {
                path: directory.join("repository"),
                secret_reference: reference,
            },
            RepositorySecret::new(reference, "fixture-secret-never-log".into()).unwrap(),
        )
    }

    #[test]
    fn credentials_are_referenced_and_not_command_arguments() {
        let directory = tempfile::tempdir().unwrap();
        let (repo, secret) = configuration(directory.path());
        let engine = ResticEngine::new("restic").unwrap();
        let json = serde_json::to_string(&repo).unwrap();
        assert!(!json.contains("fixture-secret"));
        let command = engine.command(&repo, &secret, "snapshots", &[]);
        assert!(!format!("{:?}", command.get_args().collect::<Vec<_>>()).contains("fixture-secret"));
        let environment: Vec<_> = command.get_envs().collect();
        assert!(environment
            .iter()
            .any(|(key, value)| *key == "RESTIC_PASSWORD" && value.is_some()));
        for removed in [
            "RESTIC_PASSWORD_COMMAND",
            "RESTIC_PASSWORD_FILE",
            "RESTIC_REPOSITORY_FILE",
        ] {
            assert!(environment
                .iter()
                .any(|(key, value)| *key == removed && value.is_none()));
        }
        let other = RepositorySecret::new(Uuid::new_v4(), "other".into()).unwrap();
        assert_eq!(
            engine.snapshots(&repo, &other).unwrap_err().kind,
            ResticErrorKind::SecretMismatch
        );
        assert!(RepositorySecret::new(repo.secret_reference, String::new()).is_err());
        assert!(engine.with_timeout(Duration::ZERO).is_err());
    }

    #[test]
    fn refuse_existing_restore_targets_overlapping_sources_and_unsafe_selectors() {
        let directory = tempfile::tempdir().unwrap();
        let (repo, secret) = configuration(directory.path());
        fs::create_dir(&repo.path).unwrap();
        let marker = repo.path.join("preserve.txt");
        fs::write(&marker, "existing data").unwrap();
        let engine = ResticEngine::new("must-not-execute").unwrap();
        assert_eq!(
            engine.init(&repo, &secret).unwrap_err().kind,
            ResticErrorKind::RepositoryExists
        );
        for source in [
            repo.path.clone(),
            directory.path().to_owned(),
            marker.clone(),
            PathBuf::from("relative"),
        ] {
            assert_eq!(
                engine.backup(&repo, &secret, &[source]).unwrap_err().kind,
                ResticErrorKind::UnsafeSource
            );
        }
        let id = "a".repeat(64);
        for target in [
            repo.path.clone(),
            repo.path.join("inside"),
            directory.path().to_owned(),
        ] {
            assert_eq!(
                engine
                    .restore(&repo, &secret, &id, None, &target)
                    .unwrap_err()
                    .kind,
                ResticErrorKind::UnsafeRestore
            );
        }
        let target = directory.path().join("new");
        assert_eq!(
            engine
                .restore(&repo, &secret, "latest", None, &target)
                .unwrap_err()
                .kind,
            ResticErrorKind::InvalidSnapshot
        );
        for subtree in ["relative", "/../etc", "/a/./b", "/a\\b"] {
            assert_eq!(
                engine
                    .restore(&repo, &secret, &id, Some(subtree), &target)
                    .unwrap_err()
                    .kind,
                ResticErrorKind::InvalidSnapshot
            );
        }
        assert!(!target.exists());
        assert_eq!(fs::read_to_string(marker).unwrap(), "existing data");
    }

    #[test]
    fn malformed_missing_and_duplicate_summaries_are_errors() {
        #[derive(Debug, Deserialize)]
        struct Report {
            num_errors: u64,
        }
        let valid =
            b"{\"message_type\":\"status\"}\n{\"message_type\":\"summary\",\"num_errors\":0}\n";
        assert_eq!(summary::<Report>(valid, "check").unwrap().num_errors, 0);
        for output in [b"fixture-secret-never-log".as_slice(), b"[]", b"{\"message_type\":\"summary\"}", b"{\"message_type\":\"summary\",\"num_errors\":0}\n{\"message_type\":\"summary\",\"num_errors\":0}"] {
            let error = summary::<Report>(output, "check").unwrap_err();
            assert_eq!(error.kind, ResticErrorKind::InvalidResponse);
            assert!(!error.to_string().contains("fixture-secret"));
        }
    }

    // Spawn this test executable as a controllable child: no Python/shell dependency.
    #[test]
    #[ignore = "child-process fixture; invoked by process_limits_and_exit_statuses"]
    fn child_process_fixture() {
        let Ok(mode) = std::env::var("CONTAINERSAFE_RESTIC_TEST_CHILD") else {
            return;
        };
        match mode.as_str() {
            "sleep" => std::thread::sleep(Duration::from_secs(30)),
            "output" => {
                println!("{}", "x".repeat(100_000));
                std::thread::sleep(Duration::from_secs(30));
            }
            _ => {
                eprintln!("fixture-secret-never-log");
                std::process::exit(mode.parse().unwrap());
            }
        }
    }

    #[test]
    fn process_limits_and_exit_statuses() {
        let mut engine = ResticEngine::new("unused")
            .unwrap()
            .with_timeout(Duration::from_secs(5))
            .unwrap();
        let child = |mode: &str| {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--ignored",
                    "--exact",
                    "restic::tests::child_process_fixture",
                    "--nocapture",
                ])
                .env("CONTAINERSAFE_RESTIC_TEST_CHILD", mode);
            command
        };
        for (code, operation, kind) in [
            ("3", "backup", ResticErrorKind::IncompleteBackup),
            ("3", "check", ResticErrorKind::CommandFailed),
            ("10", "snapshots", ResticErrorKind::RepositoryMissing),
            ("11", "backup", ResticErrorKind::RepositoryLocked),
            ("12", "snapshots", ResticErrorKind::WrongPassword),
        ] {
            let error = engine.execute(child(code), operation).unwrap_err();
            assert_eq!(error.kind, kind);
            assert!(!error.to_string().contains("fixture-secret"));
        }
        engine.output_limit = 1024;
        assert_eq!(
            engine.execute(child("output"), "backup").unwrap_err().kind,
            ResticErrorKind::OutputLimit
        );
        engine.timeout = Duration::from_millis(100);
        assert_eq!(
            engine.execute(child("sleep"), "backup").unwrap_err().kind,
            ResticErrorKind::Timeout
        );
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            engine
                .execute(Command::new(directory.path().join("absent-restic")), "init")
                .unwrap_err()
                .kind,
            ResticErrorKind::MissingExecutable
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_match_restic_snapshot_tree() {
        assert_eq!(
            restic_path(Path::new(r"\\?\C:\data with spaces")),
            PathBuf::from(r"C:\data with spaces")
        );
        assert_eq!(
            restic_path(Path::new(r"\\?\UNC\server\share\data")),
            PathBuf::from(r"\\server\share\data")
        );
    }
}
