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
from cryptography.hazmat.primitives.asymmetric import rsa, ec

root = Path(sys.argv[1])
paramiko.util.log_to_file(str(root / "ssh.log"))
host_key = paramiko.RSAKey.generate(2048)
private = Ed25519PrivateKey.generate()
key_path = root / "fixture_ed25519"
key_path.write_bytes(private.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.OpenSSH,
                                         serialization.BestAvailableEncryption(b"fixture-passphrase")))
client_key = paramiko.Ed25519Key.from_private_key_file(str(key_path), "fixture-passphrase")
key_files = {}
rsa_private = rsa.generate_private_key(public_exponent=65537, key_size=2048)
ecdsa_private = ec.generate_private_key(ec.SECP256R1())
for name, private_key, key_format, encrypted in [
    ("rsa_pem", rsa_private, serialization.PrivateFormat.TraditionalOpenSSL, False),
    ("rsa_pem_encrypted", rsa_private, serialization.PrivateFormat.TraditionalOpenSSL, True),
    ("rsa_openssh_encrypted", rsa_private, serialization.PrivateFormat.OpenSSH, True),
    ("rsa_pkcs8_encrypted", rsa_private, serialization.PrivateFormat.PKCS8, True),
    ("ecdsa_pem_encrypted", ecdsa_private, serialization.PrivateFormat.TraditionalOpenSSL, True),
]:
    destination = root / name
    destination.write_bytes(private_key.private_bytes(serialization.Encoding.PEM, key_format,
        serialization.BestAvailableEncryption(b"fixture-passphrase") if encrypted else serialization.NoEncryption()))
    key_files[name] = str(destination)
client_keys = [client_key, paramiko.RSAKey.from_private_key_file(key_files["rsa_pem"]),
               paramiko.ECDSAKey.from_private_key_file(key_files["ecdsa_pem_encrypted"], "fixture-passphrase")]


class Server(paramiko.ServerInterface):
    def get_allowed_auths(self, username):
        return "password,publickey"

    def check_auth_password(self, username, password):
        return paramiko.AUTH_SUCCESSFUL if username in ("fixture", "denied", "synology") and password == "fixture-password" else paramiko.AUTH_FAILED

    def check_auth_publickey(self, username, key):
        return paramiko.AUTH_SUCCESSFUL if username == "fixture" and key in client_keys else paramiko.AUTH_FAILED

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
print(json.dumps({"port": listener.getsockname()[1], "fingerprint": "SHA256:" + base64.b64encode(hashlib.sha256(host_key.asbytes()).digest()).decode().rstrip("="), "key_path": str(key_path), "key_files": key_files}), flush=True)


def serve(sock):
    transport = paramiko.Transport(sock)
    # This fixture has no DH moduli file: do not advertise group exchange and
    # then negotiate a different algorithm. Keep the supported strong defaults.
    algorithms = transport.get_security_options()
    algorithms.kex = tuple(name for name in algorithms.kex if "group-exchange" not in name)
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
