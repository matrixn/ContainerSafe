"""Validate display/package/lockfile/tag versions before building release assets."""
import argparse
import json
import os
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parent.parent


def metadata(tag=""):
    match = re.search(r'pub const VERSION: &str = "(\d+\.\d{2})";', (ROOT / "src/lib.rs").read_text(encoding="utf-8"))
    if not match:
        raise ValueError("Expected a display version such as 0.09")
    version = match[1]
    major, minor = map(int, version.split("."))
    if major > 255:
        raise ValueError("MSI major version must not exceed 255")
    package_version = f"{major}.{minor}.0"
    expected_tag = f"v{version}"
    if tag and tag != expected_tag:
        raise ValueError(f"Tag {tag!r} does not match the source version; expected {expected_tag}")
    for directory, name in [(ROOT, "containersafe"), (ROOT / "native", "containersafe-native")]:
        manifest = tomllib.loads((directory / "Cargo.toml").read_text(encoding="utf-8"))
        if manifest["package"]["version"] != package_version:
            raise ValueError(f"{name}: Cargo version must be {package_version}")
        lock = tomllib.loads((directory / "Cargo.lock").read_text(encoding="utf-8"))
        versions = {p["name"]: p["version"] for p in lock["package"] if p["name"] in ("containersafe", "containersafe-native")}
        if any(value != package_version for value in versions.values()) or versions.get(name) != package_version:
            raise ValueError(f"{directory / 'Cargo.lock'} is stale; run Cargo checks before tagging")
    return {"version": version, "package_version": package_version, "tag": expected_tag}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", default="")
    args = parser.parse_args()
    result = metadata(args.tag)
    print(json.dumps(result))
    if output := os.environ.get("GITHUB_OUTPUT"):
        with open(output, "a", encoding="utf-8") as file:
            file.writelines(f"{key}={value}\n" for key, value in result.items())
