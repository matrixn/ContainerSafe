"""Loopback-only disposable Linux SSH/SFTP/forwarding fixture. Never uses the NAS."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import shlex
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import threading
import time
import uuid
import paramiko

parser = argparse.ArgumentParser()
parser.add_argument("--restic", required=True)
parser.add_argument("--deny-forwarding", action="store_true")
parser.add_argument("--agent")
parser.add_argument("--windows-loopback-relay", action="store_true")
args = parser.parse_args()
temporary = tempfile.TemporaryDirectory(prefix="containersafe-pc-fixture-")
root = Path(temporary.name)
source = root / "source ' literal"
source.mkdir()
initial_data = b"ContainerSafe PRIVATE fixture\x00" + os.urandom(2 * 1024 * 1024)
(source / "fixture.bin").write_bytes(initial_data)
restores = root / "restores"
restores.mkdir()
password = root / "password.secret"
password.write_text("fixture-repository-password")
password.chmod(0o600)
job_id, pc_repository = str(uuid.uuid4()), str(uuid.uuid4())
agent_config = root / "agent.json"
agent_config.write_text(json.dumps({"restic_executable": args.restic, "timeout_seconds": 120,
    "jobs": [{"id": job_id, "repository": {"path": str(root / "repository"), "secret_reference": str(uuid.uuid4())},
              "secret_file": str(password), "sources": [str(source)], "restore_root": str(restores),
              "allowed_operations": ["init", "backup", "snapshots", "check", "restore"], "pc_repository": pc_repository}]}))
agent_config.chmod(0o600)
host_key = paramiko.RSAKey.generate(2048)
stop = threading.Event()
transports = []
children = []
listeners = []
fault = "none"


class Sftp(paramiko.SFTPServerInterface):
    def _path(self, path):
        p = Path(path)
        if not p.is_absolute() or ".." in p.parts or not p.is_relative_to(restores):
            raise PermissionError()
        # Reject symlinks at every component and paths escaping the approved fixture tree.
        cursor = p
        while cursor != root:
            if cursor.is_symlink():
                raise PermissionError()
            cursor = cursor.parent
        return p

    def stat(self, path):
        try:
            if fault == "missing":
                return paramiko.SFTP_NO_SUCH_FILE
            result = paramiko.SFTPAttributes.from_stat(self._path(path).stat())
            if fault == "modified":
                result.st_mtime += 1
            if fault == "symlink":
                result.st_mode = stat.S_IFLNK | 0o777
            return result
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno or 13)

    lstat = stat

    def canonicalize(self, path):
        try:
            return str(self._path(path).resolve())
        except OSError:
            return "/denied"

    def open(self, path, flags, attr):
        if flags & (os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC):
            return paramiko.SFTP_PERMISSION_DENIED
        try:
            stream = self._path(path).open("rb")
            class Handle(paramiko.SFTPHandle):
                def read(self, offset, length):
                    if fault == "truncated" and offset >= 65536:
                        return b""
                    return super().read(offset, length)
            handle = Handle(flags)
            handle.readfile = stream
            return handle
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno or 13)


def bridge(left, right):
    def copy(a, b):
        try:
            while True:
                data = a.recv(65536)
                if not data:
                    break
                b.sendall(data)
        except (OSError, EOFError):
            pass
        finally:
            try:
                b.shutdown_write() if hasattr(b, "shutdown_write") else b.shutdown(socket.SHUT_WR)
            except OSError:
                pass
    a = threading.Thread(target=copy, args=(left, right), daemon=True)
    a.start()
    copy(right, left)
    a.join(timeout=5)
    left.close()
    right.close()


class Server(paramiko.ServerInterface):
    def __init__(self):
        self.transport = None
        self.forwarders = {}

    def get_allowed_auths(self, username):
        return "password"

    def check_auth_password(self, username, secret):
        return paramiko.AUTH_SUCCESSFUL if username == "fixture" and secret == "fixture-password" else paramiko.AUTH_FAILED

    def check_channel_request(self, kind, chanid):
        return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

    def check_port_forward_request(self, address, port):
        if args.deny_forwarding or address != "127.0.0.1":
            return False
        listener = socket.socket()
        listener.bind(("127.0.0.1", port))
        listener.listen()
        listener.settimeout(0.5)
        selected = listener.getsockname()[1]
        self.forwarders[(address, selected)] = listener
        listeners.append(listener)

        def accept():
            while not stop.is_set() and self.transport.is_active():
                try:
                    connection, origin = listener.accept()
                except socket.timeout:
                    continue
                except OSError:
                    break
                try:
                    channel = self.transport.open_forwarded_tcpip_channel(origin, (address, selected))
                    threading.Thread(target=bridge, args=(connection, channel), daemon=True).start()
                except Exception:
                    connection.close()
        threading.Thread(target=accept, daemon=True).start()
        return selected

    def cancel_port_forward_request(self, address, port):
        listener = self.forwarders.pop((address, port), None)
        if listener:
            listener.close()

    def check_channel_exec_request(self, channel, command):
        def execute():
            command_text = command.decode()
            process = None
            try:
                argv = shlex.split(command_text)
                is_agent = args.agent and argv == [args.agent, "--config", str(agent_config)]
                if "version --format" in command_text:
                    channel.sendall(b"28.0.0\n")
                    channel.send_exit_status(0)
                elif command_text.startswith("sh -s --") or is_agent:
                    script = bytearray()
                    while True:
                        part = channel.recv(8192)
                        if not part:
                            break
                        script.extend(part)
                        if len(script) > 65536:
                            raise ValueError("request limit")
                    process = subprocess.Popen(argv if is_agent else ["sh", "-s", "--", *argv[3:]], stdin=subprocess.PIPE,
                                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=root, start_new_session=True)
                    children.append(process)
                    def monitor(child=process):
                        while child.poll() is None:
                            if channel.closed or not self.transport.is_active():
                                try:
                                    os.killpg(child.pid, signal.SIGTERM)
                                except ProcessLookupError:
                                    pass
                                break
                            time.sleep(0.05)
                    threading.Thread(target=monitor, daemon=True).start()
                    process.stdin.write(script)
                    process.stdin.close()

                    def stderr():
                        while True:
                            data = process.stderr.read(8192)
                            if not data:
                                break
                            try:
                                channel.send_stderr(data)
                            except OSError:
                                break
                    reader = threading.Thread(target=stderr, daemon=True)
                    reader.start()
                    streaming = b"CSAFE-STREAM/1" in script
                    withheld = b""
                    changed = False
                    while True:
                        data = process.stdout.read(65536)
                        if not data:
                            break
                        if streaming and fault == "disconnect":
                            channel.sendall(data)
                            return
                        if streaming and fault == "digest" and not changed:
                            offset = data.find(b"\nencrypted\n\n")
                            if offset >= 0:
                                offset += len(b"\nencrypted\n\n")
                                data = data[:offset] + bytes([data[offset] ^ 1]) + data[offset + 1:]
                                changed = True
                        if streaming and fault == "marker":
                            data = withheld + data
                            withheld = data[-512:]
                            data = data[:-512]
                        channel.sendall(data)
                    if withheld:
                        position = withheld.rfind(b"\nCSAFE-END/1")
                        channel.sendall(withheld[:position] if position >= 0 else withheld)
                    reader.join(timeout=5)
                    status = process.wait()
                    channel.send_exit_status(7 if streaming and fault == "exit" else status)
                else:
                    channel.send_exit_status(127)
            except (OSError, EOFError, ValueError):
                if process and process.poll() is None:
                    process.terminate()
            finally:
                if process and process.poll() is None:
                    process.terminate()
                    process.wait(timeout=10)
                channel.close()
        threading.Thread(target=execute, daemon=True).start()
        return True


listener = socket.socket()
if args.windows_loopback_relay:
    # Linux ephemeral ports may fall inside Windows excluded relay ranges.
    # Select a test-only low loopback port; never change host firewall/rules.
    for attempt in range(100):
        try:
            listener.bind(("127.0.0.1", 18000 + uuid.uuid4().int % 10000))
            break
        except OSError:
            if attempt == 99:
                raise
else:
    listener.bind(("127.0.0.1", 0))
listener.listen()
listener.settimeout(0.5)
print(json.dumps({"port": listener.getsockname()[1], "fingerprint": "SHA256:" + base64.b64encode(hashlib.sha256(host_key.asbytes()).digest()).decode().rstrip("="),
                  "root": str(root), "source": str(source), "repository": str(root / "repository"),
                  "restores": str(restores), "password_file": str(password), "restic": args.restic,
                  "file_sha256": hashlib.sha256(initial_data).hexdigest(), "agent": args.agent,
                  "agent_config": str(agent_config), "job_id": job_id, "pc_repository": pc_repository}), flush=True)


def controls():
    global fault
    for line in sys.stdin:
        if line.strip() == "quit":
            break
        if line.strip() == "change":
            data = b"Changed PRIVATE fixture\x00" + os.urandom(2 * 1024 * 1024)
            (source / "fixture.bin").write_bytes(data)
            print(json.dumps({"control": "changed", "file_sha256": hashlib.sha256(data).hexdigest()}), flush=True)
        if line.strip() == "large":
            data = os.urandom(16 * 1024 * 1024)
            (source / "fixture.bin").write_bytes(data)
            print(json.dumps({"control": "large", "file_sha256": hashlib.sha256(data).hexdigest()}), flush=True)
        if line.startswith("fault "):
            fault = line.strip().split(" ", 1)[1]
            print(json.dumps({"control": "fault", "mode": fault}), flush=True)
    stop.set()


threading.Thread(target=controls, daemon=True).start()
try:
    while not stop.is_set():
        try:
            sock, _ = listener.accept()
        except socket.timeout:
            continue
        transport = paramiko.Transport(sock)
        security = transport.get_security_options()
        security.kex = tuple(name for name in security.kex if "group-exchange" not in name)
        transport.add_server_key(host_key)
        transport.set_subsystem_handler("sftp", paramiko.SFTPServer, Sftp)
        server = Server()
        server.transport = transport
        transports.append(transport)
        threading.Thread(target=lambda t=transport, s=server: t.start_server(server=s), daemon=True).start()
finally:
    stop.set()
    listener.close()
    for transport in transports:
        transport.close()
    for listener in listeners:
        listener.close()
    for process in children:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=10)
    temporary.cleanup()
