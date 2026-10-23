# Server management — ContainerSafe 0.09

## Start and configure

```powershell
cargo run --manifest-path .\native\Cargo.toml
```

After the existing splash, a fresh installation opens Docker Server setup. Cancelling leaves a disconnected workspace. Enter a name, hostname/IP, port, username and authentication details. Docker access can be Automatic, Direct or passwordless `sudo -n`. The timeout is configurable from 1 to 120 seconds.

Test Connection validates inputs, establishes SSH, checks host identity **before authentication**, authenticates, discovers Docker access, reads the engine version and enumerates/inspects containers. It never starts or modifies containers. Unknown identities require explicit fingerprint approval. A changed fingerprint blocks authentication and opens a review showing old/new fingerprints; replacement requires explicit approval. Verify fingerprints using an independent trusted channel.

Save becomes available only for the exact tested form revision. Editing any connection field, cancelling or starting another attempt invalidates earlier results. Save also compares the complete form and credential with the tested snapshot. A cancellation discards worker results; the underlying socket finishes according to its timeout. OS DNS lookup can take longer than the configured socket timeout.

File → Servers offers New, Edit, Remove (with confirmation), Set Default and Open. The first saved server is default; Open never changes that preference. Removing the default leaves no default and startup opens the manager. Save backup selections before switching/editing servers. Last-opened updates only after inventory and profile persistence succeed. Saved statuses are historical; only the active connection context is labelled Connected.

Use Retry / Refresh to reconnect. If startup needs a session-only password or an unavailable/encrypted-key passphrase, enter it in the masked form, test and save again. A failed default remains saved and the main application stays open. Help contains documentation and About with the Slint attribution widget. Keyboard shortcuts: Ctrl+N, Ctrl+Shift+S and Ctrl+Q; Escape closes the current dialog.

## Credentials and keys

- **Password:** authenticated through libssh2, in process, without putting a password in command arguments. Password authentication must be enabled by the SSH server. Keyboard-interactive/PAM-only accounts are currently unsupported.
- **OpenSSH private key:** encrypted or unencrypted Ed25519 and compatible RSA/ECDSA keys via libssh2/OpenSSL. Public `.pub` files are rejected. Files remain at the selected path.
- **PuTTY PPK:** direct parsing is unsupported by the selected SSH library. Install PuTTYgen from the official PuTTY distribution, load the PPK, use Conversions → Export OpenSSH key, preserve/set a passphrase, save separately and select **OpenSSH Private Key** in ContainerSafe. Never export an unencrypted key as a workaround. The PPK option explains this integration and blocks a misleading successful test. ContainerSafe never runs a converter or writes an unencrypted key.

Remember stores the password/passphrase under service `ContainerSafe.SSH`, account `<server UUID>` using `keyring`: Windows Credential Manager, macOS Keychain or Linux Secret Service. Remember failures fail the save. Without Remember, secrets are held only in the application session; Rust secret holders are zeroized on drop. The Slint masked input and SSH library may maintain internal memory copies; this is not a guarantee of zeroized process memory. Profile JSON contains metadata, key paths, UUID references and public host fingerprints, never secret values or private-key contents. Edit opens blank secret fields; credentials are never revealed. Deleting a profile deletes its stored credential and preserves local cached backup data and remote data.

## Storage and migration

Windows uses `%APPDATA%\ContainerSafe`; Linux/macOS use the OS configuration directory from `directories::BaseDirs`. `servers.json` contains profiles, one optional default UUID and pinned endpoint fingerprints. Corrupt/invalid profile JSON is preserved and management writes are disabled until repaired. Writes use a same-directory temporary file, flush/sync and atomic replacement. Concurrent writers from multiple running app instances are not supported; run one instance while managing profiles.

`servers/<uuid>/` contains `backup-plan.json`, `backup-selection.json` and `server-identity.json`. Cached approvals require matching UUID, hostname, port and pinned host key. Switching servers never loads another namespace; identity changes reset decisions to Pending. Existing root JSON is never associated automatically.

Explicit legacy import for a configured, trusted server:

```powershell
cargo run -- import-legacy --server UUID --from C:\path\to\legacy --confirm-server UUID
```

Both JSON files must parse; import stages them together and refuses an existing data directory. For a server already opened in the GUI, close ContainerSafe and move its `servers/<uuid>/` directory to a separate archival location first. This prevents overwriting cached plans. The explicit matching UUID confirmation authorizes associating the legacy files with that server. Review imported selections and validate again. Malformed typed plans/selections are rejected when loaded; only approvals matching the newly discovered source policy survive reconciliation.

CLI commands use the saved default or `--server UUID`. CLI does not prompt for secrets or trust approval; use the desktop for those. Preflight retains the existing checks and read-only policy. The disposable local Restic laboratory has no remote server context and does not access configured hosts. Production backup execution remains disabled.

## Build and verify

Windows needs MSVC/SDK and a native Windows Perl for vendored OpenSSL. Git's minimal MSYS Perl is insufficient. Set `OPENSSL_SRC_PERL` to Strawberry Perl's executable if it is not on PATH. No OpenSSL DLL installation is needed. Linux needs its normal C compiler, OpenSSL build prerequisites and Secret Service development libraries; Linux/macOS desktop builds have not been validated in this change.

This workspace has a portable Perl under the ignored `target/build-tools/` directory and a compiled static OpenSSL cache. Reuse that prepared environment in a PowerShell session before the commands below:

```powershell
. .\scripts\use-local-build-tools.ps1 -UseOpenSslCache
```

This changes only the current process environment. Omit `-UseOpenSslCache` after dependency upgrades or when building from scratch; the helper then selects the portable Perl, if available, and Cargo builds its vendored OpenSSL normally. A clean checkout needs its own build prerequisites. No toolchain directories or downloaded libraries are included in Git.

```powershell
cargo fmt
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --manifest-path .\native\Cargo.toml
cargo test --manifest-path .\native\Cargo.toml
cargo clippy --manifest-path .\native\Cargo.toml -- -D warnings
cargo run --manifest-path .\native\Cargo.toml
cargo run -- lab-backup
```

Optional SSH integration fixture (Python with `paramiko`; loopback only) and disposable OS credential roundtrip:

```powershell
python -m pip install paramiko
cargo test -- --ignored
```

Set `CONTAINERSAFE_TEST_PYTHON` if the interpreter is not named `python`. The SSH fixture tests real password authentication, encrypted Ed25519 private-key authentication, unknown/changed host blocking, bad credentials, container discovery and Docker permissions without a NAS or Docker daemon.

Manual UI checks: first launch/cancel; enter invalid fields; test and approve/reject fingerprint; save; add a second server; set default; open a nondefault server; edit fields after a successful test; cancel a slow test; remove a profile; relaunch with an offline default; verify version/About and keyboard menus. Live production SSH access and Linux/macOS credential integration require separate validation.

## Versioning

The application displays **0.09**. Cargo requires semantic versions without leading zeroes, so both packages use **0.9.0**. To advance the display by exactly 0.01 and synchronize package versions:

```powershell
.\scripts\bump-version.ps1  # 0.09 -> 0.10, Cargo 0.10.0
```

The display value is shared by GUI and CLI from `containersafe::VERSION`. Refresh both lockfiles through Cargo validation before releasing. This task does not create a Git tag or publish a release.

## Implementation map

- `src/lib.rs`, `src/plan.rs`, `src/selection.rs`, `src/preflight.rs`: shared existing discovery/readiness/backup logic.
- `src/servers/{model,storage,credentials,connection}.rs`: profiles, atomic persistence, identity binding, OS secrets and real SSH.
- `src/main.rs`: CLI with explicit server context.
- `native/src/main.rs`: existing splash and backup workspace; `server_ui.rs`: server lifecycle and worker callbacks.
- `native/ui/main.slint`, `dialogs/server-form.slint`, `dialogs/server-manager.slint`: native menus and modal views.
- `src/servers/tests.rs`, `tests/ssh_integration.rs`, `tests/ssh_fixture.py`: policy, persistence and optional live loopback checks.

Library references: [ssh2 Session](https://docs.rs/ssh2/latest/ssh2/struct.Session.html), [keyring 3](https://docs.rs/keyring/3.6.3/keyring/), [PuTTY documentation](https://www.chiark.greenend.org.uk/~sgtatham/putty/latest.html), [Strawberry Perl](https://strawberryperl.com/releases.html).
