#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
version=$(python3 scripts/release_metadata.py | python3 -c 'import sys,json; print(json.load(sys.stdin)["version"])')
package_version=$(python3 scripts/release_metadata.py | python3 -c 'import sys,json; print(json.load(sys.stdin)["package_version"])')
binary_directory=${1:-target/release}
stage=$(mktemp -d "${TMPDIR:-/tmp}/containersafe-package.XXXXXX")
trap 'rm -rf -- "$stage"' EXIT
mkdir -p dist "$stage/usr/bin" "$stage/usr/share/applications" "$stage/usr/share/icons/hicolor/scalable/apps" "$stage/usr/share/doc/containersafe" "$stage/DEBIAN"
install -m 755 "$binary_directory/containersafe-native" "$stage/usr/bin/containersafe-gui"
install -m 755 "$binary_directory/containersafe" "$stage/usr/bin/containersafe"
install -m 644 packaging/linux/containersafe.desktop "$stage/usr/share/applications/"
install -m 644 packaging/linux/containersafe.svg "$stage/usr/share/icons/hicolor/scalable/apps/"
install -m 644 README.md "$stage/usr/share/doc/containersafe/"
python3 scripts/third_party_notices.py --target x86_64-unknown-linux-gnu --output "$stage/usr/share/doc/containersafe/THIRD-PARTY-NOTICES.txt"
ln -s THIRD-PARTY-NOTICES.txt "$stage/usr/share/doc/containersafe/copyright"
cat > "$stage/DEBIAN/control" <<EOF
Package: containersafe
Version: $package_version
Section: utils
Priority: optional
Architecture: amd64
Maintainer: ContainerSafe contributors <noreply@github.com>
Homepage: https://github.com/matrixn/ContainerSafe
Depends: libc6 (>= 2.35), libgcc-s1, libstdc++6, libdbus-1-3, libxkbcommon0, libfontconfig1, libwayland-client0, libx11-6, libx11-xcb1, libxcb1, libxcursor1, libxi6, libxrandr2, libegl1, libgl1, zlib1g
Recommends: gnome-keyring, xdg-desktop-portal
Description: Native Docker server and backup readiness manager
 Rust and Slint desktop application with SSH server profiles, Docker discovery,
 host-key verification and read-only backup planning. Includes the CLI.
EOF
dpkg-deb --root-owner-group --build "$stage" "dist/ContainerSafe-$version-linux-amd64.deb"
tar --sort=name --owner=0 --group=0 --numeric-owner -C "$stage" -czf "dist/ContainerSafe-$version-linux-x64.tar.gz" usr
dpkg-deb --info "dist/ContainerSafe-$version-linux-amd64.deb"
dpkg-deb --contents "dist/ContainerSafe-$version-linux-amd64.deb"
