"""Loopback-only SSH/Docker fixture. pip install paramiko; no real Docker or NAS."""
import base64
import hashlib
import json
from pathlib import Path
import socket
import sys
import threading
import time

import paramiko
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

root = Path(sys.argv[1])
host_key = paramiko.RSAKey.generate(2048)
private = Ed25519PrivateKey.generate()
key_path = root / "fixture_ed25519"
key_path.write_bytes(private.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.OpenSSH,
                                         serialization.BestAvailableEncryption(b"fixture-passphrase")))
client_key = paramiko.Ed25519Key.from_private_key_file(str(key_path), "fixture-passphrase")


class Server(paramiko.ServerInterface):
    def get_allowed_auths(self, username):
        return "password,publickey"

    def check_auth_password(self, username, password):
        return paramiko.AUTH_SUCCESSFUL if username in ("fixture", "denied", "synology") and password == "fixture-password" else paramiko.AUTH_FAILED

    def check_auth_publickey(self, username, key):
        return paramiko.AUTH_SUCCESSFUL if username == "fixture" and key == client_key else paramiko.AUTH_FAILED

    def check_channel_request(self, kind, chanid):
        return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

    def check_channel_exec_request(self, channel, command):
        def execute():
            command_text = command.decode()
            username = channel.get_transport().get_username()
            synology_command = command_text.startswith("sudo -n /var/packages/ContainerManager/target/usr/bin/docker ")
            if username == "denied":
                channel.send_stderr(b"sudo: a password is required\nprivate-fixture-marker\n" if command_text.startswith("sudo ") else b"permission denied\nprivate-fixture-marker\n")
                status = 1
            elif username == "synology" and not synology_command:
                channel.send_stderr(b"sh: docker: command not found\n")
                status = 127
            elif "version --format" in command_text:
                channel.send(b"28.0.0\n")
                status = 0
            elif command_text.endswith("ps -a -q"):
                channel.send(b"a" * 64 + b"\n")
                status = 0
            elif "inspect --type container " in command_text:
                channel.send(json.dumps([{"Id": "a" * 64, "Name": "/fixture", "Config": {"Image": "nginx", "Labels": {}}, "State": {"Status": "running"}, "Mounts": []}]).encode())
                status = 0
            else:
                status = 1
            channel.send_exit_status(status)
            channel.shutdown_write()
            channel.close()
        threading.Thread(target=execute, daemon=True).start()
        return True


listener = socket.socket()
listener.bind(("127.0.0.1", 0))
listener.listen()
print(json.dumps({"port": listener.getsockname()[1], "fingerprint": "SHA256:" + base64.b64encode(hashlib.sha256(host_key.asbytes()).digest()).decode().rstrip("="), "key_path": str(key_path)}), flush=True)


def serve(sock):
    transport = paramiko.Transport(sock)
    transport.add_server_key(host_key)
    try:
        transport.start_server(server=Server())
        channels = []
        while transport.is_active():
            channel = transport.accept(1)
            if channel is not None:
                channels.append(channel)
    except (EOFError, OSError, paramiko.SSHException):
        pass
    finally:
        transport.close()


while True:
    sock, _ = listener.accept()
    threading.Thread(target=serve, args=(sock,), daemon=True).start()
