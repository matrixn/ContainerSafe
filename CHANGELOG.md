# Changelog

The project is in active prototype development. These are implementation milestones, not official supported releases.

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
