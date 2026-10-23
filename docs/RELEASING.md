# GitHub releases — Windows and Linux

The `Build installers` workflow builds **Windows x64 MSI**, **Linux amd64 DEB** and a **Linux x64 tar.gz**. macOS packaging is intentionally omitted.

## First release: 0.09

After the workflow on `main` passes, use either path:

1. In GitHub, open **Releases → Draft a new release**.
2. Choose/create tag **`v0.09`**, targeting the current `main` commit that contains this workflow.
3. Enter the title **ContainerSafe 0.09**, review the release notes, and select **Publish release**.
4. Open **Actions → Build installers**. Windows and Linux build independently; assets are attached when both succeed. A published release can initially have no assets while compilation runs.

Alternatively, push an annotated tag from the checked-out release commit:

```powershell
git tag -a v0.09 -m "ContainerSafe 0.09"
git push origin v0.09
```

That builds the installers and creates a **draft** release with generated notes and assets. Review that draft before selecting **Publish release**. Publishing triggers another validation/build; the same asset names and checksums are updated. No tag is created by normal commits or by the version script.

Expected assets:

```text
ContainerSafe-0.09-windows-x64.msi
ContainerSafe-0.09-linux-amd64.deb
ContainerSafe-0.09-linux-x64.tar.gz
SHA256SUMS.txt
```

## Later releases

```powershell
.\scripts\bump-version.ps1          # 0.09 -> 0.10; both Cargo manifests -> 0.10.0
cargo check                        # Refresh the root lockfile
cargo check --manifest-path native/Cargo.toml
python scripts/release_metadata.py --tag v0.10
```

Review the changelog, commit and push the manifests, both updated lockfiles, shared display version and changelog, then create `v0.10`. Subsequent versions are `v0.11`, `v0.12`, etc. The workflow rejects a tag that differs from the display version or stale Cargo lockfiles. MSI requires a numeric product version: display `0.09` maps to installer/Cargo version `0.9.0`.

Do not move or overwrite a published tag. Fix the issue, advance by 0.01 and release a new tag.

## Test builds without publishing

Every push to `main` builds and tests both installer targets, then stores the files as Actions artifacts for 14 days. It does not create a release. The workflow also has **Run workflow** for manual verification from a selected branch. Tag pushes and `release.published` attach assets to GitHub Releases only after both platform jobs pass.

No personal access token or publishing secret is required. The workflow uses GitHub's short-lived `GITHUB_TOKEN`; only the final release job requests `contents: write`. Build jobs have read-only repository access. GitHub Actions must be enabled for the repository. Third-party actions are pinned to verified commit SHAs, Rust is pinned in `rust-toolchain.toml`, Cargo builds use `--locked`, and WiX is pinned to 5.0.2. `SHA256SUMS.txt` covers all three release files.

## Installation

**Windows:** open the MSI. It installs the desktop app and CLI under Program Files and adds a ContainerSafe Start Menu shortcut. Windows Installer provides upgrade/uninstall handling; downgrade is blocked. Installation requires administrator privileges. A signing certificate has not been configured, so the MSI and executables are **unsigned**. There is no custom EULA dialog because the project has not selected its own license. Third-party license texts and README are installed beside the executables. Application profiles and credentials remain in the current user's configuration/credential store and are not removed by uninstall.

Windows CI builds use the static MSVC CRT, so a separately installed Visual C++ runtime is not required. The graphical release executable uses the Windows GUI subsystem and does not open a console window.

**Ubuntu/Debian:** download the DEB and install with `sudo apt install ./ContainerSafe-0.09-linux-amd64.deb`. The desktop app appears in the application launcher; CLI: `containersafe`, GUI command: `containersafe-gui`. The build targets Ubuntu 22.04/glibc 2.35 or newer x86-64 distributions. A graphical X11/Wayland session and desktop libraries are required. Remembered credentials require a running/unlocked Secret Service provider such as GNOME Keyring. The tar.gz contains a `usr/` hierarchy for manual installation; it is not an AppImage or a fully static portable bundle and does not install dependencies automatically. Linux GUI startup is smoke-tested under Xvfb, without a production SSH server.

Neither installer configures Docker, grants remote sudo access, imports local backup plans, bundles private credentials, or installs an SSH key. The local Restic laboratory requires a separately installed `restic` executable; Restic is not bundled.

The GUI startup check uses a temporary `CONTAINERSAFE_CONFIG_DIR` so it cannot load existing server profiles, including on Windows where known-folder APIs can ignore an `APPDATA` environment override. This optional process-local override requires an absolute path and leaves the normal OS configuration directory unchanged.

## Local package validation

Windows build prerequisites are documented in [SERVER_MANAGEMENT.md](SERVER_MANAGEMENT.md). For this prepared workspace, optionally dot-source `scripts/use-local-build-tools.ps1 -UseOpenSslCache` first. Use Python 3.11+ for the release helper scripts.

```powershell
cargo build --release --locked
$env:CARGO_TARGET_DIR = "$PWD\target"
cargo build --release --locked --manifest-path native/Cargo.toml
dotnet tool install wix --version 5.0.2 --tool-path target/wix
.\scripts\package-windows.ps1 -Wix "$PWD\target\wix\wix.exe"
```

WiX may require `DOTNET_ROLL_FORWARD=Major` when only a newer .NET runtime is installed. The CI runner installs .NET 8 and native Windows Perl. `package-windows.ps1 -BinaryDirectory ...` can use another build directory for packaging-only checks; such checks do not substitute for the release build in CI.

On Linux, install the development packages listed in the workflow, build both binaries into the same `CARGO_TARGET_DIR`, and run `bash scripts/package-linux.sh`. Packaging scripts collect dependency notices from Cargo's resolved dependency graph and local crate license files. The repository's own license remains undecided, as stated in README; the workflow does not choose a project license.

References: [GitHub workflow events](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows), [WiX Package schema](https://docs.firegiant.com/wix/schema/wxs/package/).
