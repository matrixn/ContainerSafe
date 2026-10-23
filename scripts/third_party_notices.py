"""Collect Cargo dependency license declarations and bundled license/notice texts."""
import argparse
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def generate(target, output):
    packages = {}
    for manifest in [ROOT / "Cargo.toml", ROOT / "native/Cargo.toml"]:
        data = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", target, "--manifest-path", str(manifest)], cwd=ROOT))
        active = {node["id"] for node in data["resolve"]["nodes"]}
        for package in data["packages"]:
            if package["id"] in active and package["source"] is not None:
                packages[package["id"]] = package
    sections = ["ContainerSafe — Third-party notices\n\nContainerSafe's own project license has not been selected. See README.md.\nDependencies below retain their respective licenses. License expressions describe upstream alternatives; inclusion here does not change them.\n\nSlint attribution: https://slint.dev/ — the desktop About dialog includes the MadeWithSlint widget.\n"]
    for package in sorted(packages.values(), key=lambda p: (p["name"], p["version"])):
        base = Path(package["manifest_path"]).parent
        files = set()
        for path in base.iterdir():
            if path.name.lower().startswith(("license", "licence", "copying", "copyright", "notice")):
                if path.is_file():
                    files.add(path)
                elif path.is_dir():
                    files.update(p for p in path.rglob("*") if p.is_file())
        if package.get("license_file"):
            files.add(base / package["license_file"])
        sections.append(f"\n{'=' * 78}\n{package['name']} {package['version']}\nLicense: {package.get('license') or 'See bundled license file'}\nSource: {package.get('repository') or 'https://crates.io/crates/' + package['name']}\n")
        for path in sorted(files):
            if path.is_file():
                sections.append(f"\n--- {path.relative_to(base)} ---\n{path.read_text(encoding='utf-8', errors='replace')}\n")
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text("".join(sections), encoding="utf-8")
    print(f"Collected notices for {len(packages)} dependencies: {output}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    generate(args.target, args.output)
