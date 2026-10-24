use containersafe::{
    plan, selection,
    servers::{
        connection::{Connection, Failure},
        credentials,
        storage::{self, Profiles},
    },
};
use std::{error::Error, io, path::PathBuf};
fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("discover");
    if mode == "--version" {
        println!("ContainerSafe {}", containersafe::VERSION);
        return Ok(());
    }
    if mode == "lab-backup" {
        return match &args[1..] {
            [] => containersafe::restic_lab::run(),
            [option, path] if option == "--restic" => {
                containersafe::restic_lab::run_with_executable(path)
            }
            _ => Err("Usage: containersafe lab-backup [--restic PATH_TO_EXECUTABLE]".into()),
        };
    }
    let root = storage::config_root().map_err(io::Error::other)?;
    let profiles = Profiles::load(&root).map_err(io::Error::other)?;
    let explicit = args
        .iter()
        .position(|a| a == "--server")
        .and_then(|i| args.get(i + 1));
    let id = explicit
        .map(|s| s.parse())
        .transpose()?
        .or(profiles.default)
        .ok_or(
            "Select a server with --server UUID, or configure a default in the native application",
        )?;
    let server = profiles
        .servers
        .iter()
        .find(|s| s.id == id)
        .ok_or("Unknown server UUID")?;
    let data = storage::server_dir(&root, id);
    let identity = storage::DataIdentity::new(server, &profiles.hosts).map_err(io::Error::other)?;
    if mode == "import-legacy" {
        let source = args
            .iter()
            .position(|s| s == "--from")
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
            .ok_or("Use import-legacy --server UUID --from DIRECTORY --confirm-server UUID")?;
        let confirmation = args
            .iter()
            .position(|s| s == "--confirm-server")
            .and_then(|i| args.get(i + 1));
        if confirmation != Some(&id.to_string()) {
            return Err("Explicit --confirm-server UUID is required".into());
        }
        storage::import_legacy(&root, id, &source).map_err(io::Error::other)?;
        identity.save(&data).map_err(io::Error::other)?;
        return Ok(());
    }
    if matches!(mode, "draft" | "validate" | "preflight") {
        if !identity.matches(&data) {
            return Err("Backup data is not bound to this server identity; discover again or explicitly import legacy data".into());
        }
        let report: plan::BackupReport =
            serde_json::from_slice(&std::fs::read(data.join("backup-plan.json"))?)?;
        if mode == "draft" {
            println!(
                "{}",
                serde_json::to_string_pretty(&selection::draft(&report))?
            );
            return Ok(());
        }
        let choices: selection::BackupSelection =
            serde_json::from_slice(&std::fs::read(data.join("backup-selection.json"))?)?;
        if mode == "validate" {
            let errors = selection::validate(&report, &choices);
            if errors.is_empty() {
                println!("Selection valid. Backup execution NOT authorized.");
                return Ok(());
            }
            return Err(errors.join("\n").into());
        }
        let secret = if server.remember {
            credentials::read(id).map_err(io::Error::other)?
        } else {
            credentials::Secret::default()
        };
        let connection = Connection::open(server, &secret, &profiles.hosts).map_err(failure)?;
        return containersafe::preflight::run(&connection, &report, &choices);
    }
    if !matches!(mode, "discover" | "audit" | "plan") {
        return Err("Usage: containersafe [discover|audit|plan|draft|validate|preflight|lab-backup|import-legacy] [--server UUID]".into());
    }
    let secret = if server.remember {
        credentials::read(id).map_err(io::Error::other)?
    } else {
        credentials::Secret::default()
    };
    let connection = Connection::open(server, &secret, &profiles.hosts).map_err(failure)?;
    let containers = containersafe::discover(&connection).map_err(io::Error::other)?;
    match mode {
        "audit" => containersafe::print_backup_readiness(&containers),
        "plan" => {
            let json = serde_json::to_vec_pretty(&plan::build(&containers))?;
            storage::atomic_write(&data.join("backup-plan.json"), &json)
                .map_err(io::Error::other)?;
            identity.save(&data).map_err(io::Error::other)?;
            println!("{}", String::from_utf8(json)?);
        }
        _ => containersafe::print_report(&containers),
    }
    Ok(())
}
fn failure(f: Failure) -> io::Error {
    io::Error::other(match f {
        Failure::Message(m) => m,
        Failure::UnknownHost(h) => format!(
            "Untrusted host {} {} {}. Review in the native application.",
            h.endpoint, h.key_type, h.fingerprint
        ),
        Failure::ChangedHost(h) => {
            format!("HOST KEY CHANGED for {}. Connection blocked.", h.endpoint)
        }
    })
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
