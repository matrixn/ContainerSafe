"""Exercise the exact SSH stdin script with real Restic; isolated Linux files only."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import uuid


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--restic", required=True, type=Path)
    args = parser.parse_args()
    restic = args.restic.resolve(strict=True)
    script = (Path(__file__).resolve().parent.parent / "src/direct_backup.sh").read_text()
    with tempfile.TemporaryDirectory(prefix="containersafe-direct-") as temporary:
        root = Path(temporary)
        source = root / "lost parent" / "nested" / "source ' literal; no-shell"
        source.mkdir(parents=True)
        repository = root / "repository ' literal"
        restore_root = root / "restores"
        restore_root.mkdir()
        password = root / "repository.password"
        secret = str(uuid.uuid4())
        password.write_text(secret)
        password.chmod(0o600)
        fixture = source / "data.bin"

        def call(operation, snapshot="", directory="", paths=None, code=0):
            command = ["sh", "-s", "--", operation, str(restic), str(password),
                       str(repository), str(restore_root), snapshot, directory]
            command.extend(str(path) for path in (paths if paths is not None else [source]))
            environment = os.environ.copy()
            environment.update(RESTIC_PASSWORD="wrong-inherited-secret", RESTIC_PASSWORD_COMMAND="echo forbidden")
            process = subprocess.run(command, input=script, text=True, capture_output=True, timeout=90, env=environment)
            assert secret not in process.stdout + process.stderr, "secret leaked"
            assert process.returncode == code, (operation, process.returncode, process.stderr)
            if code:
                return
            if operation == "snapshots":
                return json.loads(process.stdout)
            if operation == "init":
                return
            messages = [json.loads(line) for line in process.stdout.splitlines() if line]
            summaries = [message for message in messages if message.get("message_type") == "summary"]
            assert len(summaries) == 1, messages
            return summaries[0]

        first_bytes, second_bytes = b"ContainerSafe direct SSH v1\x00\n", b"Changed direct SSH v2\x00\n"
        fixture.write_bytes(first_bytes)
        call("init")
        call("init", code=74)
        first = call("backup")["snapshot_id"]
        fixture.write_bytes(second_bytes)
        second = call("backup")["snapshot_id"]
        assert first != second
        assert len(call("snapshots")) == 2
        assert call("check")["num_errors"] == 0
        fixture.unlink()
        source.rmdir()
        source.parent.rmdir()
        source.parent.parent.rmdir()
        assert len(call("snapshots")) == 2
        call("restore", first, "test-first")
        call("restore", second, "test-second")
        relative = fixture.relative_to("/")
        assert (restore_root / "test-first" / relative).read_bytes() == first_bytes
        assert (restore_root / "test-second" / relative).read_bytes() == second_bytes
        call("restore", second, "test-second", code=73)
        password.chmod(0o644)
        call("snapshots", code=71)
        password.chmod(0o600)
        call("backup", paths=[root], code=72)
        call("backup", paths=[repository], code=72)
        call("backup", paths=[Path("/")], code=72)
        print(json.dumps({"status": "PASS", "transport": "direct-restic-script", "snapshots": 2,
                          "verified_restores": 2, "unsafe_paths_and_permissions": "passed",
                          "production_connections": 0}))


if __name__ == "__main__":
    main()
