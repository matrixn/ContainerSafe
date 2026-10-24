"""Exercise the Linux stdio agent against disposable local files only."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import uuid


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--agent", required=True, type=Path)
    parser.add_argument("--restic", required=True, type=Path)
    args = parser.parse_args()
    if not sys.platform.startswith("linux"):
        parser.error("The agent file-permission boundary currently supports Linux only")
    agent, restic = args.agent.resolve(strict=True), args.restic.resolve(strict=True)
    capabilities = subprocess.run([str(agent), "--capabilities"], check=True, text=True, capture_output=True)
    version = json.loads(capabilities.stdout)["application_version"]
    with tempfile.TemporaryDirectory(prefix="containersafe-agent-lab-") as temporary:
        root = Path(temporary)
        source, restores = root / "source data; literal", root / "restores"
        source.mkdir()
        restores.mkdir()
        secret = root / "password.secret"
        secret.write_text(str(uuid.uuid4()), encoding="utf-8")
        secret.chmod(0o600)
        job_id = str(uuid.uuid4())
        config = {
            "restic_executable": str(restic),
            "timeout_seconds": 60,
            "jobs": [{
                "id": job_id,
                "repository": {"path": str(root / "repository"), "secret_reference": str(uuid.uuid4())},
                "secret_file": str(secret), "sources": [str(source)], "restore_root": str(restores),
                "allowed_operations": ["init", "backup", "snapshots", "check", "restore"],
            }],
        }
        config_path = root / "agent.json"

        def save_config():
            config_path.write_text(json.dumps(config), encoding="utf-8")
            config_path.chmod(0o600)

        def call(action, error=None, **overrides):
            request = {"protocol_version": 1, "request_id": str(uuid.uuid4()), "job_id": job_id, "action": action}
            request.update(overrides)
            process = subprocess.run([str(agent), "--config", str(config_path)],
                                     input=json.dumps(request), text=True, capture_output=True, timeout=75)
            assert not process.stderr, "unexpected agent stderr"
            assert secret.read_text() not in process.stdout, "secret leaked in response"
            response = json.loads(process.stdout)
            assert response["protocol_version"] == 1
            assert response["application_version"] == version
            if error:
                assert process.returncode == 1, response
                assert response["status"] == "error", response
                assert response["error"]["code"] == error, response
                return response
            assert process.returncode == 0 and response["status"] == "ok", response
            assert response["request_id"] == request["request_id"]
            return response["result"]

        save_config()
        original, changed = b"ContainerSafe agent fixture\x00version 1\n", b"Changed agent fixture\x00version 2\n"
        fixture = source / "fixture.bin"
        fixture.write_bytes(original)
        call({"operation": "init"})
        assert call({"operation": "snapshots"}) == []
        first = call({"operation": "backup"})["snapshot_id"]
        fixture.write_bytes(changed)
        second = call({"operation": "backup"})["snapshot_id"]
        assert first != second
        assert len(call({"operation": "snapshots"})) == 2
        assert call({"operation": "check", "read_data": True})["all_data_read"]
        # Recovery must work after the original source has been lost.
        shutil.rmtree(source)
        for name, snapshot, expected in [("first", first, original), ("second", second, changed)]:
            result = call({"operation": "restore", "snapshot_id": snapshot, "directory": name})
            assert result["content_verified"] and result["files_restored"] >= 1, result
            assert result["bytes_restored"] == len(expected), result
            recovered = restores / name / source.relative_to(source.anchor) / fixture.name
            assert recovered.read_bytes() == expected
        call({"operation": "restore", "snapshot_id": first, "directory": "first"}, error="restic_failed")
        call({"operation": "restore", "snapshot_id": first, "directory": "../escape"}, error="unsafe_restore")
        call({"operation": "snapshots"}, error="unknown_job", job_id=str(uuid.uuid4()))
        call({"operation": "backup", "sources": ["/etc"]}, error="invalid_request")
        secret.chmod(0o644)
        call({"operation": "snapshots"}, error="insecure_file")
        secret.chmod(0o600)
        # A source containing a configured password file must not be backed up.
        config["jobs"][0]["sources"] = [str(root)]
        save_config()
        call({"operation": "backup"}, error="invalid_config")
        config["jobs"][0]["sources"] = [str(source)]
        config["jobs"][0]["allowed_operations"] = ["snapshots"]
        save_config()
        call({"operation": "backup"}, error="operation_denied")
        assert len(call({"operation": "snapshots"})) == 2
        print(json.dumps({"status": "PASS", "version": version, "snapshots": 2, "verified_restores": 2,
                          "policy_and_permissions": "passed", "production_connections": 0}))


if __name__ == "__main__":
    main()
