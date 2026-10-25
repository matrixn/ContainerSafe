# PC transfer milestones: 0.31–0.34

Baseline: v0.30, commit `456303dd98fb94d7e65acc64618d597fd69f6d81`.
The combined final release is v0.34. No production NAS was used for these tests.

## v0.31: shared transfer core and SFTP pull

Files: `Cargo.toml`, `Cargo.lock`, `native/Cargo.toml`, `native/Cargo.lock`,
`src/lib.rs`, `src/servers/connection.rs`, `src/transfer/mod.rs`,
`src/transfer/model.rs`, `src/transfer/coordinator.rs`, `src/transfer/artifact.rs`,
`src/transfer/export_artifact.sh`, `src/transfer/sftp_pull.rs`,
`tests/pc_transfers.rs`, `scripts/pc_transfer_fixture.py` and this document.

- Export exactly one snapshot whose paths match the currently approved sources.
  Use supported `restic copy` into a NEW encrypted repository, check all data,
  then archive that finalized isolated repository. Never archive a live source repo.
- The tar is a container for encrypted Restic data, not plaintext restored files.
  It excludes the repository password. Export requires temporary NAS space for
  both a separate repository and its archive; exports are not automatically removed.
- Bind metadata to the server UUID, pinned SSH key, endpoint, artifact UUID,
  snapshot ID, remote path, size, mtime and SHA-256 obtained over verified SSH.
- Use the SFTP subsystem of the existing pinned russh session. Validate a regular
  non-symlink source before and after transfer; use bounded 64 KiB buffers.
- Write an exclusive partial file under a canonical PC directory. An OS advisory
  lock prevents competing operations and releases on process failure. Resume only
  explicitly with the same receipt/source/destination and SFTP method.
- Rehash existing partial data and completed disk bytes. Publish through an atomic
  no-clobber hard link only after length/SHA verification. Never overwrite user files.
- Persist typed states/receipts without secrets. Cancellation retains an unfinished
  partial file. Pause is unsupported. SFTP requests have a 30-second timeout and
  the whole transfer has a six-hour deadline.

Verified: existing root/native regression tests and compilation; collision, resume,
truncation, digest, cancellation and post-write corruption guards. The real fixture
creates two Linux snapshots, exports the first, downloads/cancels/resumes on Windows,
checks with Windows Restic, restores and compares original SHA-256. Wrong host pins
are rejected. Exported bytes do not contain the fixture's known plaintext prefix.

## v0.32: framed binary SSH streaming

New files: `src/transfer/ssh_stream.rs`, `src/transfer/stream_artifact.sh`.
Updated: transfer model/module, connection, fixture/tests, version manifests/locks.

`CSAFE-STREAM/1` has an exact ASCII header binding transfer/artifact UUIDs,
length, digest, format and encryption classification, followed by binary payload
and an exact `CSAFE-END/1` trailer bound to the transfer UUID/digest. Completion
also requires successful producer exit and channel completion. Binary payload
never passes through a String. Stderr is discarded with a bounded byte counter.
Fixed buffers/writes apply backpressure. Idle timeout is 30 seconds; overall
limit is six hours. Cancellation is observed during setup and payload transfer.
Streaming cannot resume or silently fall back; retry starts a new transfer.

Verified: byte equality with SFTP for the same encrypted artifact, zero/non-UTF8
bytes, representative framing boundaries including one-byte chunks, cancellation,
missing marker, digest corruption, producer failure and disconnect. Existing
regressions remained successful. GUI controls arrived in the next milestone.

## v0.33: authenticated reverse SSH tunnel

New files: `src/transfer/local_backend.rs`, `src/transfer/reverse_tunnel.rs`,
`src/transfer/pc_backup.rs`. Updated: `src/servers/connection.rs`, `src/agent.rs`,
`src/bin/containersafe-agent.rs`, `src/backup.rs`, `src/direct_backup.rs`,
`src/direct_backup.sh`, `src/restic.rs`, fixture/tests and manifests/locks.

- Use the maintained official restic/rest-server 0.14.0 executable, explicitly
  selected on the PC. Do not implement a new REST storage protocol or install a service.
- Bind local and forwarded listeners to loopback. Inspect the actual remote listener
  in `/proc/net/tcp{,6}` and reject wildcard/unverifiable bindings. Never change sshd.
- Generate a random session-only credential and temporary bcrypt auth file. Pass the
  credential only inside pinned SSH via protected stdin script/agent JSON, never
  a URL, command argument, plan, activity entry or persisted receipt.
- Use private append-only repositories and server/repository binding files. Reject
  symlinks/reparse escapes and competing contexts. Keep Restic's own locks authoritative.
- Both direct execution and explicitly PC-authorized agent jobs use the tunnel.
  Run a full repository data check before reporting successful backup.
- Agent `pc_only` prevents executing a descriptive PC path as a NAS repository.
  Old jobs omit optional PC fields and retain their original semantics.
- Bound active bridges to 32, use bounded copies and tear down helper/forward on
  completion/failure/cancellation. Network bytes have no invented backup total.

Verified: forwarding permitted/denied, REST auth, unauthorized repository access,
context collision, direct and agent two-snapshot backups, full Windows checks,
isolated Windows restores with SHA comparison, and SSH usability after teardown.
A 16 MiB upload test cancels after network bytes flow, requires failure, checks
without automatic unlock and successfully retries. Command cancellation allows a
bounded three-second REST-bridge grace period for the remote process to release
its own lock. The fixture models SSH closure by signalling its disposable process group.

## v0.34: native integration and local repository operations

New files: `src/transfer/local_artifact.rs`, `scripts/get-rest-server.ps1`.
Updated: `native/src/backup_ui.rs`, `native/ui/backup-panel.slint`,
`native/ui/main.slint`, `native/translations/ro.json`, generated Romanian `.po`,
`src/restic.rs`, `src/servers/storage.rs`, transfer coordinator/SFTP/stream code,
fixture/tests, README, CHANGELOG, MANUAL_BACKUPS and version manifests/locks.

- Separate backup execution, storage destination and artifact download choices.
  Serialized `this_computer` remains NAS-to-PC SFTP push. The NEW
  `local_pc_repository` means a direct PC repository through reverse SSH forwarding.
- Add folder/helper selection, immutable export, SFTP/stream download, explicit
  SFTP resume, byte/rate/elapsed/known-total ETA, cancellation, open-folder and
  persistent artifact receipt history. One operation executes at a time.
- Use workers/timers so Slint remains responsive. Exports/local checks show a busy
  overlay without cancellation; SFTP, streaming and reverse operations offer cancel.
- Local operations require an explicitly supplied PC Restic executable/password file.
  Password contents are not fetched from the NAS, serialized or sent to the NAS.
  Use the saved pinned identity and local binding without opening another SSH session.
- Rehash before extracting only allowlisted Restic paths into a temporary directory.
  Reject traversal, absolute/ADS paths, links, duplicate files and special entries.
  Test restore uses a NEW UUID directory; no live data is replaced.
- Import snapshots through supported `restic copy` into a separately bound local
  repository, then check all data. Preserve existing snapshots; do not recursively
  merge active repositories, mirror deletions, prune or unlock automatically.
- The optional Windows helper script verifies a pinned official SHA-256, refuses
  an existing destination, does not change PATH and does not install a service.
  Restic/rest-server binaries are external prerequisites, not bundled in the MSI.

Verified: old-plan migration retains SFTP meaning, secrets and server separation;
PC agent exports authorize their stable repository; local strict archive extraction,
real supported copy of two exports into one PC repository preserving both snapshots;
root/native regression tests, Clippy, translations and native compilation. The Windows
form was visually inspected with synthetic data, including destination-specific fields.

## Reproducing isolated tests

Use real Windows Restic (`CONTAINERSAFE_RESTIC`, default development path
`C:/Users/radut/bin/restic.exe`) and official rest-server (`CONTAINERSAFE_REST_SERVER`).
The Windows launcher uses WSL Ubuntu-22.04 and Python/Paramiko at
`$HOME/.cache/containersafe-transfer-fixture/bin/python`. Place Linux Restic in
`target/linux-agent/restic` and a Linux agent built with `--no-default-features`
in `target/linux-agent/containersafe-agent`. These are test prerequisites, not
system installations. On Linux set `CONTAINERSAFE_FIXTURE_PYTHON`.
The fixture binds only loopback and creates disposable files/passwords/host keys.

```powershell
cargo test --locked --test pc_transfers -- --ignored --nocapture
cargo test --locked --test ssh_integration -- --ignored --nocapture
cargo test --locked
cargo test --locked --manifest-path native/Cargo.toml
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --manifest-path native/Cargo.toml --all-targets -- -D warnings
python scripts/update-translations.py --check
```

For the existing Windows SSH fixture, set `CONTAINERSAFE_TEST_PYTHON` to a Python
with Paramiko installed (the local test environment is `target/ssh-test-env/Scripts/python.exe`).
Windows/WSL tests serialize fixture launchers and retry only relay startup against
the SAME pinned identity; authentication/trust failures never trigger a fallback.

## Remaining coverage and limits

Real OS disk-full, multi-gigabyte runs, forced backend crashes, full SSH disconnect
recovery and end-to-end automation of every desktop action need broader platform tests.
Cancellation cannot prove every remote descendant exited: inspect locks/snapshots
before retrying. No automatic unlock/prune or restore into live sources occurs.
The GUI executes one operation at a time; multi-item queues, pause and automatic
crash recovery are future work. Production DSM execution, database consistency,
scheduling/retention, full application recovery and macOS packaging remain unvalidated
or unimplemented. The integration tests never access production NAS services.

Primary references: [official rest-server](https://github.com/restic/rest-server),
[Restic repository copy](https://restic.readthedocs.io/en/stable/045_working_with_repos.html),
[Restic backends](https://restic.readthedocs.io/en/stable/030_preparing_a_new_repo.html).
