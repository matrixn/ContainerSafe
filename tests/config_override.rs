use std::process::Command;

#[test]
fn startup_configuration_override_rejects_relative_paths() {
    let result = Command::new(env!("CARGO_BIN_EXE_containersafe"))
        .arg("validate")
        .env("CONTAINERSAFE_CONFIG_DIR", "relative-config")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("must be an absolute path"));
}

#[test]
fn isolated_configuration_is_read_without_using_the_real_profile_directory() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("servers.json");
    std::fs::write(&path, b"{broken-isolated-test-config").unwrap();
    // Validation does not make network connections, even if an override regresses.
    let result = Command::new(env!("CARGO_BIN_EXE_containersafe"))
        .arg("validate")
        .env("CONTAINERSAFE_CONFIG_DIR", directory.path())
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Profiles could not be read"));
    assert_eq!(
        std::fs::read(path).unwrap(),
        b"{broken-isolated-test-config"
    );
}
