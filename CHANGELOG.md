# Changelog

The project is in active prototype development. These are implementation milestones, not official supported releases.

## 0.15 — 2026-10-24

- Redesign the desktop workspace with a light slate/white palette, teal primary actions, readable typography, section cards, consistent spacing and the actual application logo beside the name.
- Rename Applications to Containers. Add a searchable expandable Compose → project → container and Standalone → container explorer, preserving original backend application identities and source-selection scope.
- Separate Backup plans from the inventory view: show selected operations, excluded items and pending decisions per stack/container, with Review links back to the correct project.
- Replace cycling decision buttons with explicit policy-aware dropdowns. Keep canonical source/database indices across Storage/Databases filters and retain protection against including raw database directories.
- Add shared light buttons, selectors and checkboxes with pointer cursors, hover/pressed/disabled/focus feedback; update server setup, manager and modal colors while preserving selectable/copyable detail text.
- Add focused tree/plan/policy tests, a 25-container synthetic visual fixture and debug-only `--preview-report` mode that never loads server profiles or opens network connections.

## 0.14 — 2026-10-24

- Replace libssh2/WinCNG with pinned `russh` and the explicit `ring` backend for desktop/CLI SSH on all platforms; remove OpenSSL/libssh2 and avoid the default AWS-LC backend.
- Restore encrypted and plain OpenSSH Ed25519 login, modern RSA and ECDSA key containers and encrypted PKCS#8. Preserve encrypted RSA/EC PEM AES-128/256-CBC files with a read-only RustCrypto compatibility decoder; unsupported legacy encryption is reported explicitly.
- Preserve endpoint SHA256 fingerprint pins and require approval before authentication/key decryption. Accept modern Ed25519/ECDSA host keys, reject untrusted/changed keys, and keep host certificates disabled without a separate CA policy.
- Preserve Docker command discovery, Synology paths, passwordless sudo diagnostics, bounded output and timeouts. Reject missing exit status and terminated commands.
- Verify password/key login and Docker discovery against loopback SSH fixtures with RSA/Ed25519/ECDSA host keys, wrong passwords/passphrases, immutable source keys, output overflow and stalled commands. Keep PPK on the explicit manual encrypted export workflow; direct PPK, hardware keys and keyboard-interactive remain unsupported by this interface.
- Update shared/native version metadata and make the agent lab follow the current executable version.

## 0.13 — 2026-10-24

- Add `containersafe-agent`, a Linux single-request stdio executable designed for SSH invocation, reusing the shared Restic engine without listening on a network port.
- Allow an agent-only `--no-default-features` build without desktop SSH/keyring dependencies, OpenSSL/libssh2 or D-Bus; preserve the normal CLI/desktop defaults.
- Define a versioned, bounded JSON protocol with request/job UUIDs, strict request fields and structured secret-free errors; reject unknown protocol versions and operations.
- Authorize operations from host-controlled job configuration. Client requests cannot select executables, repositories, passwords, backup paths or arbitrary restore targets.
- Require protected Linux configuration/password files, reject symlinks and unsafe permissions, and refuse configured backups that contain known agent secret/config files.
- Constrain restores to fresh child directories of the configured restore root and reject targets overlapping configured backup sources. Existing Restic snapshot-ID, repository-overlap and verification checks remain in force.
- Add policy/path/permission tests and a disposable Linux process-level lab covering two real snapshots, two verified restores, denied operations, injected fields, unsafe targets and insecure secrets. Document Linux build, protocol, SSH invocation and test instructions in Romanian.
- Desktop integration, database consistency, automatic deployment, scheduling and agent packaging remain future work.

## 0.12 — 2026-10-24

- Implement the reusable Restic engine planned in the original v0.8 discussion: typed local init, backup, snapshots, check and isolated restore operations shared by the CLI, desktop core and future agent.
- Separate repository configuration from session secrets, configure the executable explicitly, parse JSON results, bound child-process execution and return structured errors without raw process output or passwords.
- Require exact snapshot IDs and new restore directories, reject repository/source overlap, and verify restored content with Restic. Normalize Windows extended path prefixes for consistent snapshot trees.
- Refactor the local lab onto the shared engine, replace the hard-coded password with a random ephemeral secret, and compare restored bytes for both snapshot versions plus an older snapshot restored again.
- Add process/error/path guard tests, an optional real-Restic integration test and Romanian test instructions. Production Docker backups, Linux agent, destination UI and scheduling remain future milestones.

## 0.11 — 2026-10-24

- Use native WinCNG for Windows SSH, removing Windows OpenSSL/Perl build requirements while preserving vendored OpenSSL on Linux/macOS. Report WinCNG key-format limitations explicitly and test encrypted RSA PEM authentication without weakening host-key verification.
- Embed the ContainerSafe icon and Windows application metadata in the native executable.
- Run installer automation only on version-tag pushes (`v*`); normal branch pushes, release publication and manual dispatch no longer start builds.
- Selectable read-only display text across the desktop, with Ctrl+A/Ctrl+C, full-value copying for shortened labels, a Copy report action and click-to-open copyable field help.
- Probe Synology Container Manager and legacy Docker package paths when Docker is absent from the SSH PATH.
- Report per-command Docker access diagnostics, distinguishing missing executables, denied access, sudo password/policy requirements and an unreachable daemon without displaying raw remote output.
- Cover Synology package-path fallback and denied sudo access with the loopback SSH fixture.

## 0.10 — 2026-10-24

- Redesigned server setup with Connection, Sign-in and Docker & test tabs, a wider responsive dialog and clearer spacing.
- Authentication-specific fields, PuTTY export guidance, contextual help tooltips and explanations for Docker access and credential storage.
- Clear the entered secret when changing authentication methods.
- Keep the completed splash visible for 2.5 seconds before opening the main window.
- Added VS Code desktop run/build tasks with an explicit Windows Cargo launcher path and documented the distinction between desktop and CLI startup.
- Updated the shared application version and Cargo packages to 0.10 / 0.10.0.

## 0.09 — 2026-10-23

- Automated GitHub installer builds for Windows x64 MSI and Linux amd64 DEB/tar.gz, tag/version validation, dependency notices and SHA-256 checksums. Tag pushes prepare a draft release; publishing attaches rebuilt assets. Main/manual runs create test artifacts only; macOS is deferred.

- Native File/Help menus, modal server form and server manager with add/edit/remove/default/open.
- Asynchronous SSH/Docker testing, explicit host fingerprint approval and changed-key review.
- Password and encrypted OpenSSH authentication; documented encrypted PuTTYgen export path for PPK.
- OS credential storage, UUID profiles, atomic configuration writes and per-server backup namespaces.
- Shared CLI/native Rust core; removed the implicit `zion` target and hardcoded-only Docker access.
- First-run setup after the splash, default-server startup and disconnected failure states.
- Version 0.09 in the window title, application bar and About. Releases increment by 0.01.
- Added isolated policy/storage tests and optional loopback SSH and OS credential integration checks.

## Earlier development milestones

### Native interface
- Slint desktop preview with application/source lists and selection editing.
- Startup splash screen with stage-based status and progress indicator.

### CLI prototype
- Read-only Docker discovery, Compose grouping and mount inventory.
- Backup readiness analysis, backup-plan JSON and selection validation.
- Basic remote preflight checks.
- Local-only Restic lab: encrypted repository, initial and incremental snapshots, integrity checks and restored-content comparisons.

### Not yet implemented
- Production Docker backup scheduling, a host agent, consistent full-stack recovery and automated restore drills.
