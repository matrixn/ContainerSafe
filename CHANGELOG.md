# Changelog

The project is in active prototype development. These are implementation milestones, not official supported releases.

## 0.35 — 2026-10-25

- Execute PostgreSQL, MariaDB/MySQL, Redis/Valkey, MongoDB replica-set and explicitly labelled SQLite database exports before encrypted Restic backups. Support database-only plans and preserve protected raw-volume policies.
- Bind selected engines and SQLite file paths to refreshed inventory, then execute only full running container IDs through the existing pinned SSH/Docker access method. Keep database credentials inside the container and expose typed, engine-specific errors without raw stderr.
- Prepare exports in an exclusively created SSH-owned mode-700 directory, add a recovery manifest, reject stale stages and unsafe overlap, and explicitly report cleanup failures after successful or failed Restic operations. No automatic privilege elevation, host-key bypass, container stop, live overwrite or Restic unlock.
- Support the same desktop preparation with direct Restic, optional configured Linux-agent execution and reverse-tunnel PC destinations. Re-export agent setup after enabling database sources. Hide cancellation for database plans because terminating remote dump descendants is not guaranteed.
- Document consistency boundaries: PostgreSQL is consistent per database, MySQL/MariaDB holds a global read lock and blocks writes, Redis Cluster and MongoDB standalone/sharded are rejected, SQLite needs an explicit file label. Database imports remain manual; a Restic file restore is not application recovery.
- Add a disposable Linux Docker/Restic fixture for real exports, encrypted snapshots, full-data checks, byte-verified isolated restores and native-client record verification. Root/native tests, Clippy, agent-without-server-management check, Windows native build, version and 430-entry translation validation passed. Live DSM/database GUI execution and the existing SSH/PC transfer suites were not rerun for this milestone.
- Update README, database setup/recovery and manual/agent instructions, plus the source-derived Wiki. No new tag is created by this milestone's branch-only publication.

## 0.34 — 2026-10-25

- Integrate independent execution, PC destination and SFTP/stream transfer choices in the Slint backup plan. Preserve legacy SFTP push destinations and migrate old plans with defaults for new fields.
- Add folder/helper selection, live transfer progress/cancel, explicit SFTP resume, receipt history, local integrity check, isolated restore, and supported restic copy into a separately bound local repository. Add English/Romanian UI strings.
- Extract only verified encrypted artifacts with a strict Restic layout allowlist; reject traversal, links, duplicate files and nonregular entries. Verify actual disk bytes before final download publication.
- Add isolated Windows/Linux integration fixtures for all three transports and real Windows Restic restore/hash comparison. Update the README and roadmap. Helpers remain explicit external prerequisites; production NAS tests were not performed.

## 0.33 — 2026-10-25

- Add reverse SSH forwarding to an official maintained rest-server backend, using loopback listeners, ephemeral bcrypt-protected session credentials, append-only private repositories and per-server bindings/locks.
- Support both direct SSH and explicitly PC-authorized Linux agent jobs. Run a full repository check after each backup before reporting success. Tear down the helper/forward on completion, failure or cancellation; never weaken SSH identity checks.
- Verify direct and agent modes with two incremental snapshots and byte-checked Windows restores. Explicitly reject denied forwarding and wildcard/nonverifiable remote listeners.

## 0.32 — 2026-10-25

- Add bounded binary SSH artifact streaming with exact transfer/artifact identity, length, digest and completion framing, producer exit checks, timeout and cancellation. Never silently substitute SFTP; streaming retries from zero.
- Test zero bytes, split framing, cancellation, digest corruption, missing completion, producer failure and disconnect using loopback SSH.

## 0.31 — 2026-10-25

- Add typed transfer states, identities, capabilities and receipts with collision guards, private partial files and atomic no-clobber publication.
- Export one approved snapshot through supported Restic copy into a separate encrypted repository; download the immutable artifact over SFTP on the verified SSH session.
- Support explicit resume only for the unchanged artifact/server/destination; test real encryption, cancellation/resume, host fingerprint rejection, metadata changes and malformed/missing/truncated sources.

## 0.30 — 2026-10-25

- Make direct Restic-over-SSH the default backup executor; no ContainerSafe agent or agent.json is required. Keep the agent as an explicit advanced option and migrate existing plans to the direct default without discarding paths.
- Adapt each plan's form to its execution mode and destination: hide agent executable/config/job fields for direct SSH and show destination-specific labels/help. Export matching setup instructions instead of an unnecessary agent configuration.
- Support DSM 7.4 with POSIX/BusyBox-compatible preflight commands, private server-side password files, safe path quoting, canonical source/repository overlap checks and isolated verified restores after source loss. Never inherit Docker sudo permissions for Restic or weaken SSH/SFTP host verification.
- Verify DSM 7.4 x64 and required NAS utilities read-only. Validate direct script backup/restore with real Restic in isolated Linux storage and pinned SSH transport through the loopback fixture; no production NAS backups are executed.

## 0.29 — 2026-10-25

- Connect Backup plans to the Linux stdio agent for manual encrypted file backups, snapshots, full data checks and isolated verified test restores. Keep raw database volumes blocked and reject unimplemented logical dumps explicitly.
- Add an automatically saved default destination in Settings and per-plan overrides: server folder, this computer via SFTP, and remote SFTP / HTTPS S3. Require destination SSH host verification; keep repository passwords and backend credentials on the server.
- Bind execution to the selected server UUID, fresh Docker source policies and the agent's exact approved source/repository configuration. Persist per-server activity and export reviewed setup files without secrets.
- Add Windows SSH transport tests and real Linux Restic incremental backup/restore checks. Deployment and real remote backend verification remain separate setup steps; see docs/MANUAL_BACKUPS.md.

## 0.28 — 2026-10-25

- Move Edit and Remove into colored, keyboard-accessible icon actions on each server row. Replace the default marker with a clickable gold star and separate the remaining footer buttons.
- Persist the default profile and refresh the server list with the favorite first, keeping canonical profile indices so editing, removal and opening still target the correct UUID after sorting.
- Add regression coverage for favorite ordering, persistence and action identity.

## 0.27 — 2026-10-25

- Center the main desktop window on its monitor at its first native redraw, using actual outer dimensions and display scaling. Share the existing first-frame centering helper with the startup splash and Help preview.

## 0.26 — 2026-10-25

- Add Help → Preview splash screen. The centered preview remains open until Esc and returns focus to the main application. Reuse the startup splash without running startup work or connecting to servers.

## 0.25 — 2026-10-25

- Replace the splash's scaled baked-in text with native DPI-aware branding, description, labels and progress text. Preserve its compact 550 × 353 size, borderless appearance, centering and loading delay.

## 0.24 — 2026-10-25

- Replace the host copy text button with a compact clipboard icon, hover/focus feedback, tooltip and green check confirmation. Preserve copying the complete host and port and keyboard accessibility.

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
