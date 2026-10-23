"""Confirm the packaged GUI stays running with a disposable first-run config."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

with tempfile.TemporaryDirectory(prefix="containersafe-startup-") as directory:
    root = Path(directory)
    environment = os.environ.copy()
    environment["SLINT_BACKEND"] = "winit-software"
    environment["CONTAINERSAFE_CONFIG_DIR"] = str(root / "config")
    for variable, child in [("APPDATA", "roaming"), ("LOCALAPPDATA", "local"), ("XDG_CONFIG_HOME", "config")]:
        folder = root / child
        folder.mkdir()
        environment[variable] = str(folder)
    process = subprocess.Popen([str(Path(sys.argv[1]).resolve())], env=environment, stderr=subprocess.PIPE)
    try:
        process.wait(timeout=8)
    except subprocess.TimeoutExpired:
        print("GUI startup smoke check passed: process stayed alive with fresh configuration")
    else:
        raise RuntimeError(f"GUI exited before the smoke check completed (code {process.returncode}): {process.stderr.read().decode(errors='replace')}")
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=10)
