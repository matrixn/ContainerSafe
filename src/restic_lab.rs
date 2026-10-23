use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

// EXCLUSIV pentru date fictive, create local.
// Nu folosim aceasta parola pentru backupuri reale.
const LAB_PASSWORD: &str = "containersafe-disposable-lab-only";

fn restic(repo: &Path, args: &[&OsStr]) -> Result<(), Box<dyn Error>> {
    let output = Command::new("restic")
        .arg("--repo")
        .arg(repo)
        .args(args)
        .env("RESTIC_PASSWORD", LAB_PASSWORD)
        .output()?;

    if !output.status.success() {
        return Err(io::Error::other(format!(
            "Restic failed (exit code: {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ))
        .into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.trim().is_empty() {
        println!("{stdout}");
    }

    Ok(())
}

pub fn run() -> Result<(), Box<dyn Error>> {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();

    // Director nou si unic, in spatiul temporar Windows.
    let lab = std::env::temp_dir().join(format!(
        "containersafe-lab-{}-{timestamp}",
        std::process::id()
    ));

    fs::create_dir(&lab)?;

    let source = lab.join("source");
    let repo = lab.join("repository");
    let restored = lab.join("restored");

    fs::create_dir(&source)?;
    fs::create_dir(&restored)?;

    let filename = "containersafe-fixture.txt";
    let original = source.join(filename);
    let expected = b"ContainerSafe: backup and restore test.\n";

    fs::write(&original, expected)?;

    println!("ContainerSafe v{} - Restic Lab", crate::VERSION);
    println!("Local test directory: {}", lab.display());
    println!("No Synology connection will be used.");

    // 1. Repository criptat.
    println!("\n[1/4] Initializing repository...");
    restic(&repo, &[OsStr::new("init")])?;

    // 2. Backup fisier fictiv.
    println!("\n[2/4] Creating encrypted backup...");
    restic(&repo, &[OsStr::new("backup"), source.as_os_str()])?;

    // 3. Citim si verificam datele repository-ului.
    println!("\n[3/4] Checking repository integrity...");
    restic(&repo, &[OsStr::new("check"), OsStr::new("--read-data")])?;

    // 4. Restaurare izolata.

    // Convertim calea Windows intr-o cale din snapshot.
    // Exemplu:
    // C:\Users\...\source -> /C/Users/.../source
    //
    // Aceasta conversie este specifica laboratorului Windows.
    let source_in_snapshot = source.to_string_lossy().replace('\\', "/").replace(':', "");

    let snapshot_subtree = format!("latest:/{}", source_in_snapshot.trim_start_matches('/'));

    println!("Restoring subtree: {snapshot_subtree}");

    restic(
        &repo,
        &[
            OsStr::new("restore"),
            OsStr::new(&snapshot_subtree),
            OsStr::new("--target"),
            restored.as_os_str(),
        ],
    )?;

    let recovered = restored.join(filename);

    if !recovered.is_file() {
        return Err("Restored test file was not found".into());
    }
    let actual = fs::read(&recovered)?;

    if actual != expected {
        return Err(io::Error::other("Restored content differs from the original").into());
    }

    // 5. Simulam o modificare a fisierului.
    println!("\n[5/7] Updating test file...");

    let updated_content = b"ContainerSafe: backup and restore test.\nVersion 2.\n";

    fs::write(&original, updated_content)?;

    // 6. Cream un al doilea snapshot in acelasi repository.
    println!("\n[6/7] Creating incremental snapshot...");

    restic(&repo, &[OsStr::new("backup"), source.as_os_str()])?;

    // Verificam si datele celui de-al doilea snapshot.
    restic(&repo, &[OsStr::new("check"), OsStr::new("--read-data")])?;

    // 7. Restauram ultima versiune separat de prima.
    println!("\n[7/7] Restoring latest version...");

    let restored_v2 = lab.join("restored-v2");
    fs::create_dir(&restored_v2)?;

    restic(
        &repo,
        &[
            OsStr::new("restore"),
            OsStr::new(&snapshot_subtree),
            OsStr::new("--target"),
            restored_v2.as_os_str(),
        ],
    )?;

    let recovered_v2 = restored_v2.join(filename);

    if !recovered_v2.is_file() {
        return Err("Second restored file was not found".into());
    }

    if fs::read(&recovered_v2)? != updated_content {
        return Err("Second snapshot content mismatch".into());
    }

    // Prima restaurare trebuie sa ramana neschimbata.
    if fs::read(&recovered)? != expected {
        return Err("First restored version was modified".into());
    }

    println!("\n[PASS] Incremental snapshot created");
    println!("[PASS] Latest version restored");
    println!("[PASS] Previous restored version preserved");

    println!("\n==================================");
    println!("[PASS] Repository initialized");
    println!("[PASS] Backup created");
    println!("[PASS] Repository data checked");
    println!("[PASS] File restored and compared");
    println!("==================================");

    println!("Test files retained at:");
    println!("{}", lab.display());

    Ok(())
}
