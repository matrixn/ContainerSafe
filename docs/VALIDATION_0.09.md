# Validation — 0.09, 2026-10-23

Validated on Windows/MSVC in `C:\Workspace\containersafe`. No production NAS was accessed.

| Check | Result |
|---|---|
| Root `cargo fmt --check` | Passed |
| Native `cargo fmt --check` | Passed |
| Root `cargo test` | 21 passed; two optional integration tests skipped by default |
| Root `cargo test -- --ignored` | Both passed: disposable Windows credential and real loopback SSH fixture |
| Native `cargo test` | 3 passed |
| Root `cargo clippy --all-targets -- -D warnings` | Passed |
| Native `cargo clippy -- -D warnings` | Passed |
| Native `cargo run` | Opened the existing splash followed by the server setup form |
| CLI `--version` | `ContainerSafe 0.09` |
| Local Restic lab | Two snapshots, both integrity checks, both restores and content comparisons passed |
| Version increment helper | Verified on disposable copies: 0.09 → 0.10, Cargo 0.10.0 |
| `git diff --check` | Passed |

26 automated tests passed in total when including the explicitly run optional integrations. The loopback fixture exercised password login, encrypted OpenSSH Ed25519 login, unknown host rejection, host-key mismatch blocking, authentication failure, Docker permissions and container discovery. Additional tests cover configuration mutation/persistence, corrupt JSON preservation, default selection, timestamps, secret exclusion, stale test results, offline defaults, timeout handling and backup identity/namespace isolation.

Native UI checks covered first launch, disabled Save before testing, cancellation into a disconnected workspace, File menu entries/accelerators, Server Manager opening, Ctrl+Shift+S, Ctrl+Q, Help/About and the visible 0.09 version and Slint attribution. Dialog contrast was adjusted and visually rechecked with the Windows dark theme.

The build used portable Strawberry Perl plus locally compiled static OpenSSL under ignored build directories. OpenSSL was compiled with the Qt JOM build tool to finish the initial Windows build in parallel. Reuse the prepared cache by dot-sourcing `scripts/use-local-build-tools.ps1 -UseOpenSslCache` in a development shell. The root checks reused `native/target` via `CARGO_TARGET_DIR` to avoid duplicating artifacts. The optional SSH test used Python 3.10 with Paramiko in an isolated `target/build-tools/ssh-fixture-deps` directory. Downloaded tooling was not installed globally or added to Git.

Not validated: production NAS access, real multi-server CRUD through authenticated UI sessions, Linux/macOS builds or credential stores, keyboard-interactive-only SSH, and direct PPK parsing. PPK is supported through the documented encrypted PuTTYgen-to-OpenSSH workflow. Concurrent application instances are unsupported for profile management. Production backup execution remains outside this change.

Existing uncommitted work was retained. No commit, push, release tag or deployment was performed.
