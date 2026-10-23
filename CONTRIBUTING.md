# Contributing to ContainerSafe

Thank you for your interest in ContainerSafe.

ContainerSafe is an early-stage Rust/Slint prototype. Before large changes, please open an issue to discuss scope and compatibility. Project licensing is still being decided; do not assume a general redistribution/contribution license is available yet.

## Development setup

```powershell
cargo fmt --check
cargo test
cargo clippy -- -D warnings

cargo fmt --manifest-path .\native\Cargo.toml --check
cargo test --manifest-path .\native\Cargo.toml
cargo clippy --manifest-path .\native\Cargo.toml -- -D warnings
```

To start the desktop preview:

```powershell
cargo run --manifest-path .\native\Cargo.toml
```

## Contribution guidance

- Prefer small, focused PRs with clear descriptions.
- Include reproducible tests for parsing, planning, and safety behavior.
- Use disposable test fixtures for any backup/restore change.
- Never introduce commands that alter Docker containers on discovery or audit paths.
- Keep secrets, `.env` values, personal NAS paths and generated manifests out of commits and logs.
- Don't describe a backup as verified unless an actual restore test is recorded.

## Reporting bugs

Please include platform, Rust and Slint versions, steps to reproduce and **redacted** logs. Remove usernames, hostnames, secret paths, tokens and repository passwords.

For vulnerabilities, follow [SECURITY.md](SECURITY.md) instead of filing a public issue.
