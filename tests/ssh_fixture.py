"""Loopback-only SSH/Docker fixture. pip install paramiko; no real Docker or NAS."""
import base64
import hashlib
import json
import os
import shlex
from pathlib import Path
import socket
import sys
import threading
import time

import paramiko
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.asymmetric import rsa, ec
from cryptography.hazmat.primitives import padding
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes

root = Path(sys.argv[1])
paramiko.util.log_to_file(str(root / "ssh.log"))
host_kind = sys.argv[2] if len(sys.argv) > 2 else "rsa"
if host_kind == "ed25519":
    host_path = root / "host_ed25519"
    host_path.write_bytes(Ed25519PrivateKey.generate().private_bytes(serialization.Encoding.PEM,
        serialization.PrivateFormat.OpenSSH, serialization.NoEncryption()))
    host_key = paramiko.Ed25519Key.from_private_key_file(str(host_path))
elif host_kind == "ecdsa":
    host_key = paramiko.ECDSAKey.generate()
else:
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
    ("ed25519_openssh_plain", private, serialization.PrivateFormat.OpenSSH, False),
    ("rsa_pem", rsa_private, serialization.PrivateFormat.TraditionalOpenSSL, False),
    ("rsa_pem_encrypted", rsa_private, serialization.PrivateFormat.TraditionalOpenSSL, True),
    ("rsa_openssh_encrypted", rsa_private, serialization.PrivateFormat.OpenSSH, True),
    ("rsa_pkcs8_encrypted", rsa_private, serialization.PrivateFormat.PKCS8, True),
    ("ecdsa_pem_encrypted", ecdsa_private, serialization.PrivateFormat.TraditionalOpenSSL, True),
    ("ecdsa_openssh_encrypted", ecdsa_private, serialization.PrivateFormat.OpenSSH, True),
    ("ecdsa_pkcs8_encrypted", ecdsa_private, serialization.PrivateFormat.PKCS8, True),
]:
    destination = root / name
    destination.write_bytes(private_key.private_bytes(serialization.Encoding.PEM, key_format,
        serialization.BestAvailableEncryption(b"fixture-passphrase") if encrypted else serialization.NoEncryption()))
    key_files[name] = str(destination)

# The cryptography serializer above uses AES-256 for legacy PEM. Exercise the
# AES-128 compatibility branch independently with the same generated keypairs.
for name, private_key, label in [("rsa_pem_aes128_encrypted", rsa_private, "RSA"),
                                ("ecdsa_pem_aes128_encrypted", ecdsa_private, "EC")]:
    iv = os.urandom(16)
    aes_key = hashlib.md5(b"fixture-passphrase" + iv[:8]).digest()
    der = private_key.private_bytes(serialization.Encoding.DER, serialization.PrivateFormat.TraditionalOpenSSL,
                                   serialization.NoEncryption())
    padder = padding.PKCS7(128).padder()
    padded = padder.update(der) + padder.finalize()
    encryptor = Cipher(algorithms.AES(aes_key), modes.CBC(iv)).encryptor()
    encoded = base64.b64encode(encryptor.update(padded) + encryptor.finalize()).decode()
    body = "\n".join(encoded[i:i+64] for i in range(0, len(encoded), 64))
    destination = root / name
    destination.write_text(f"-----BEGIN {label} PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nDEK-Info: AES-128-CBC,{iv.hex().upper()}\n\n{body}\n-----END {label} PRIVATE KEY-----\n", encoding="ascii")
    key_files[name] = str(destination)
client_keys = [client_key, paramiko.RSAKey.from_private_key_file(key_files["rsa_pem"]),
               paramiko.ECDSAKey.from_private_key_file(key_files["ecdsa_pem_encrypted"], "fixture-passphrase")]


class Server(paramiko.ServerInterface):
    def get_allowed_auths(self, username):
        return "password,publickey"

    def check_auth_password(self, username, password):
        with (root / "auth-events.log").open("a") as event:
            event.write("password\n")
        return paramiko.AUTH_SUCCESSFUL if username in ("fixture", "denied", "synology") and password == "fixture-password" else paramiko.AUTH_FAILED

    def check_auth_publickey(self, username, key):
        with (root / "auth-events.log").open("a") as event:
            event.write("publickey\n")
        return paramiko.AUTH_SUCCESSFUL if username == "fixture" and key in client_keys else paramiko.AUTH_FAILED

    def check_channel_request(self, kind, chanid):
        return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

    def check_channel_exec_request(self, channel, command):
        def execute():
            command_text = command.decode()
            if command_text.startswith("sh -s --"):
                argv = shlex.split(command_text)
                assert argv[:3] == ["sh", "-s", "--"], argv
                assert argv[4] in ("/fixture/restic 'literal; name", "/fixture/restic-incomplete"), argv
                received = bytearray()
                while True:
                    part = channel.recv(8192)
                    if not part:
                        break
                    received.extend(part)
                    assert len(received) <= 65536
                assert b"StrictHostKeyChecking=yes" in received
                channel.send_stderr(b"private-fixture-marker\n")
                incomplete = argv[4] == "/fixture/restic-incomplete"
                channel.sendall(b"{}" if incomplete else b"[]")
                channel.send_exit_status(3 if incomplete else 0)
                channel.shutdown_write()
                channel.close()
                return
            if command_text.startswith("'/fixture/agent"):
                argv = shlex.split(command_text)
                request_data = bytearray()
                while True:
                    part = channel.recv(8192)
                    if not part:
                        break
                    request_data.extend(part)
                    if len(request_data) > 65536:
                        channel.close()
                        return
                request = json.loads(request_data)
                assert argv == ["/fixture/agent 'literal; name", "--config", "/fixture/config 'literal; name"], argv
                response = {"protocol_version": 1, "request_id": request["request_id"],
                            "status": "ok", "result": {"repository": "/backup", "sources": ["/data"],
                            "restore_root": "/restores", "timeout_seconds": 60}}
                channel.send_stderr(b"private-fixture-marker\n")
                channel.sendall(json.dumps(response).encode())
                channel.send_exit_status(0)
                channel.shutdown_write()
                channel.close()
                return
            username = channel.get_transport().get_username()
            synology_command = command_text.startswith("sudo -n /var/packages/ContainerManager/target/usr/bin/docker ")
            if "fixture-hang" in command_text:
                time.sleep(3)
                channel.close()
                return
            if "fixture-no-status" in command_text:
                channel.close()
                return
            if "fixture-large" in command_text:
                try:
                    for _ in range(33):
                        channel.sendall(b"x" * 1024 * 1024)
                    channel.send_exit_status(0)
                    channel.close()
                except (EOFError, OSError):
                    pass
                return
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
