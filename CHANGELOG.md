# Changelog

The project is in active prototype development. These are implementation milestones, not official supported releases.

## 0.23 — 2026-10-25

- Fix splash centering by waiting for the first native redraw event. Previously the positioning call ran before the event loop created the native window and did nothing.
- Preserve physical monitor coordinates, DPI-aware window dimensions, the borderless half-size splash and the completed-loading delay.

## 0.22 — 2026-10-25

- Make ordinary interface text nonselectable, retaining selectable error output and explicit copy actions. Keep editable inputs normal.
- Add a host-and-port copy button per server and row-wide selection with a normal cursor.
- Organize preferences into Appearance & language, Startup & workspace, and New server defaults cards while preserving automatic saves and live animation preview.
- Reduce splash size to 550 × 353, remove native decorations and center it on its monitor using physical monitor coordinates. Retain the completed-loading delay.

## 0.21 — 2026-10-25

- Add Ripple, Pendulum, Lotus, Cascade and Infinity, bringing the loading gallery to ten live options across two pages.
- Share one animation component between gallery, Settings preview and actual connection popup. Add an autosaved Loading animation preference with Breath (option 2) as the default for new and existing settings files.
- Validate all animation choices and preserve rollback on failed saves; keep connection stages, host checks and modal input blocking unchanged.

## 0.20 — 2026-10-25

- Add a standalone loading animation gallery (`--loading-gallery`) with five live native animations: Orbit, Breath, Silk wave, Constellation and Horizon.
- Compare all options at once and highlight a preview choice without changing the connection popup or accessing server profiles.

## 0.19 — 2026-10-25

- Add a centered, borderless connection overlay with an indeterminate spinner and live SSH/host verification/authentication/Docker/discovery/planning stage text in English and Romanian.
- Dim and block underlying workspace/forms while connecting. Close the overlay on success, failure or required host approval; preserve all identity checks and test revision guards.
- Report actual SSH transport stages via an optional callback while preserving the existing Connection::open API.

## 0.18 — 2026-10-25

- Show only plans with an enabled, supported Include or Logical dump selection; remove plans immediately when their last backup is disabled. Preserve canonical Review targets and database protections.
- Disable automatic reconnection by default. At startup show Servers when profiles exist, or Add server when no profiles exist.
- Add an explicit Reconnect to last server preference. When enabled, reconnect to the last successfully opened profile, independently of the default profile. Old v0.17 automatic-default preferences do not opt users into this new behavior.
- Add regression tests for live plan removal, supported methods, settings migration and last-opened server selection.

## 0.17 — 2026-10-25

- Add File → Settings and sidebar navigation with descriptive preference cards, immediate atomic saves, individual success/failure feedback and protection for malformed settings files.
- Implement English (default) and Romanian, bundled into the executable, with live language switching for desktop navigation, dialogs, help, selections and preferences. Preserve original remote technical output.
- Configure startup page/source filter, new-profile SSH port/timeout/Docker defaults, and optional default-server connection at launch. Preserve existing profiles, explicit backup selection saves and mandatory host-key verification.
- Add isolated persistence, invalid-configuration and failed-write tests; increment application/native version to 0.17.

## 0.16 — 2026-10-24

- Show only stacks/standalone containers with at least one selectable Include or Logical dump method in Backup plans; No backup and Exclude do not count as methods. Preserve original Review targets and update plan counts/empty states after filtering.
- Display No backup instead of Pending in selectors, plan summaries and validation messages. Preserve the stored `pending` value and its review requirement for compatibility with existing saved selections.
- Add circled info icons beside backup selectors with option-specific tooltips and copyable help popups. Add a separate warning info icon for database entries without a selectable adapter, explaining why backup is unavailable.
- Keep PostgreSQL logical dump explicitly labeled as a planning strategy whose execution adapter is not implemented. Do not enable raw database copies or unsupported database adapters.
- Add eligibility/index-mapping and tooltip/backward-compatibility tests; increment shared/native versions to 0.16.

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
