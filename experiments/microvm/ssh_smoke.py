#!/usr/bin/env python3
"""Run real OpenSSH/SFTP/forwarding against two simultaneously confined VMs.

This opt-in fixture is diagnostic, not a production credential channel or density
benchmark. Every credential is fresh test material in a private instance disk.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import secrets
import select
import shlex
import signal
import struct
import subprocess
import tempfile
import termios
import threading
import time

import keeper_probe
from smoke import helper_identity, LOG_LIMIT, ROOT, artifact, connect, drain, line

ENV = {"PATH": "/usr/bin:/bin", "LC_ALL": "C"}
SOCKETS = ("control.sock", "keeper.sock", "ssh.sock")


def command(argv, out, name, data=b"", timeout=20):
    """Bound output, execution time and descendants for every native client."""
    stdout, stderr, overflow = bytearray(), bytearray(), threading.Event()
    process = subprocess.Popen(list(map(str, argv)), stdin=subprocess.PIPE,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=ENV, close_fds=True,
        start_new_session=True)
    def write():
        try:
            process.stdin.write(data)
            process.stdin.close()
        except (BrokenPipeError, OSError):
            pass  # Expected for authentication/channel refusals; outputs checked below.

    threads = [threading.Thread(target=drain, args=(stream, buf, overflow), daemon=True)
               for stream, buf in [(process.stdout, stdout), (process.stderr, stderr)]]
    threads.append(threading.Thread(target=write, daemon=True))
    for thread in threads:
        thread.start()
    try:
        code = process.wait(timeout=timeout)
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)
        for thread in threads:
            thread.join(timeout=2)
        (out / f"{name}.stdout").write_bytes(stdout)
        (out / f"{name}.stderr").write_bytes(stderr)
    if overflow.is_set():
        raise RuntimeError(f"{name}: client output budget exceeded")
    return subprocess.CompletedProcess(argv, code, bytes(stdout), bytes(stderr))


def require(ok, detail):
    if not ok:
        raise AssertionError(detail)


class Instance:
    def __init__(self, label, out, helper, kernel, guest):
        self.label, self.helper = label, helper.resolve()
        self.out = out / label
        self.out.mkdir()
        self.path = Path(tempfile.mkdtemp(prefix="ds-", dir="/private/tmp" if os.uname().sysname == "Darwin" else "/tmp"))
        self.id = secrets.token_hex(16)
        self.process = None
        self.boots = []
        for key in ("client", "host", "wrong-client"):
            result = command(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "devcroft-microvm-test",
                              "-f", self.path / key], self.out, f"keygen-{key}", timeout=5)
            require(result.returncode == 0, f"key generation failed: {result.stderr!r}")
        seed = json.dumps({"schema": 1, "instance_id": self.id,
            "host_key": (self.path / "host").read_text(),
            "authorized_key": (self.path / "client.pub").read_text()}).encode()
        require(0 < len(seed) <= 16384, "SSH seed exceeds guest budget")
        state = self.path / "state.img"
        with state.open("xb") as f:
            f.truncate(16 * 1024 * 1024)
            f.seek(8192)
            f.write(b"DCSHKEY1" + struct.pack(">I", len(seed)) + seed)
        state.chmod(0o600)
        self.host_alias = f"guest-{self.id}"
        self.known_hosts = self.path / "known_hosts"
        self.known_hosts.write_text(self.host_alias + " " + (self.path / "host.pub").read_text())
        self.launch = {"schema": 1, "kernel": artifact(kernel), "initramfs": guest["initramfs"],
            "runtime": guest["runtime"], "state": str(state), "instance_dir": str(self.path),
            "vcpus": 1, "memory_mib": 256}
        self.launch_path = self.path / "launch.json"
        self.launch_path.write_text(json.dumps(self.launch, indent=2) + "\n")
        (self.out / "launch.json").write_text(self.launch_path.read_text())

    def options(self, key="client", known_hosts=None, endpoint=None):
        # OpenSSH itself prefixes ProxyCommand with exec before invoking sh.
        proxy = "/usr/bin/nc -U " + shlex.quote(str((endpoint or self.path) / "ssh.sock"))
        return ["-F", "/dev/null", "-o", f"ProxyCommand={proxy}",
                "-o", "BatchMode=yes", "-o", "IdentitiesOnly=yes", "-o", "IdentityAgent=none",
                "-o", "ForwardAgent=no", "-o", "StrictHostKeyChecking=yes",
                "-o", f"UserKnownHostsFile={known_hosts or self.known_hosts}",
                "-o", "GlobalKnownHostsFile=/dev/null", "-o", f"HostKeyAlias={self.host_alias}",
                "-o", "ConnectTimeout=5", "-o", "ConnectionAttempts=1", "-i", str(self.path / key)]

    def ssh(self, name, remote=None, data=b"", extra=(), **kwargs):
        argv = ["ssh", *self.options(**kwargs), *extra, "devcroft@guest"]
        if remote is not None:
            argv.append(remote)
        return command(argv, self.out, f"boot-{len(self.boots)}-{name}", data=data)

    def prepare(self, validation_timeout=10):
        check = command([self.helper, "check", self.launch_path], self.out, "check", timeout=validation_timeout)
        require(check.returncode == 0, check.stderr)
        sentinel = self.out / "outside-sentinel"
        sentinel.write_text("controller-owned sentinel\n")
        test = command([self.helper, "sandbox-test", self.launch_path, sentinel.resolve()], self.out, "seatbelt", timeout=validation_timeout)
        require(test.returncode == 0, test.stderr)
        self.sandbox = json.loads(test.stdout)
        for name in SOCKETS:
            (self.path / name).unlink(missing_ok=True)

    def start(self):
        self.stdout, self.stderr, self.overflow = bytearray(), bytearray(), threading.Event()
        self.started = time.monotonic()
        self.process = subprocess.Popen([str(self.helper), "run", str(self.launch_path)],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            env=ENV, close_fds=True, start_new_session=True)
        self.threads = [threading.Thread(target=drain, args=(stream, buf, self.overflow), daemon=True)
            for stream, buf in [(self.process.stdout, self.stdout), (self.process.stderr, self.stderr)]]
        for thread in self.threads:
            thread.start()

    def ready(self, timeout=30):
        sock, greeting = connect(self.path / "control.sock", self.process, self.started + timeout, self.overflow)
        sock.close()
        index = len(self.boots) + 1
        require(greeting == f"READY 1 state={index} ro=1 landlock=6 policy=1\n".encode(), greeting)
        self.boots.append({"boot": index, "greeting": greeting.decode().strip(),
                           "ready_seconds": time.monotonic() - self.started})

    def stop(self):
        if self.process is None:
            return
        forced = self.process.poll() is None
        if forced:
            os.killpg(self.process.pid, signal.SIGKILL)
        self.process.wait(timeout=5)
        for thread in self.threads:
            thread.join(timeout=2)
        index = len(self.boots)
        (self.out / f"boot-{index}.stdout.log").write_bytes(self.stdout)
        (self.out / f"boot-{index}.stderr.log").write_bytes(self.stderr)
        if self.boots:
            self.boots[-1].update(exit_code=self.process.returncode, forced_cleanup=forced,
                log_overflow=self.overflow.is_set(),
                ssh_console_captured=b"guest SSH: nono applied; serving port 1026" in self.stderr + self.stdout,
                guest_console_stream="stdout" if b"guest SSH: nono applied; serving port 1026" in self.stdout else "stderr")
        self.process = None
        for name in SOCKETS:
            (self.path / name).unlink(missing_ok=True)

    def shutdown(self):
        sock, _ = connect(self.path / "control.sock", self.process, time.monotonic() + 5, self.overflow)
        with sock:
            sock.sendall(b"SHUTDOWN\n")
            require(line(sock) == b"BYE\n", "missing poweroff acknowledgement")
        code = self.process.wait(timeout=10)
        self.stop()
        require(code == 0 and not self.boots[-1]["log_overflow"], f"unclean shutdown: {code}")
        require(self.boots[-1]["ssh_console_captured"], "SSH console positive control missing")

    def forward(self, name="forward", port=8080):
        return self.ssh(name, data=b"GET / HTTP/1.1\r\nHost: guest\r\nConnection: close\r\n\r\n",
                        extra=["-W", f"127.0.0.1:{port}"])

    def terminal(self):
        """A real terminal drives OpenSSH window-change and remote Ctrl-C."""
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
        process = subprocess.Popen(["ssh", *self.options(), "-tt", "devcroft@guest"],
            stdin=slave, stdout=slave, stderr=subprocess.PIPE, env=ENV,
            close_fds=True, start_new_session=True)
        os.close(slave)
        output, error, overflow = bytearray(), bytearray(), threading.Event()
        stderr = threading.Thread(target=drain, args=(process.stderr, error, overflow), daemon=True)
        stderr.start()
        deadline = time.monotonic() + 20

        def read_until(needle):
            while needle not in output:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not select.select([master], [], [], max(0, remaining))[0]:
                    raise TimeoutError(f"terminal never emitted {needle!r}")
                chunk = os.read(master, 65536)
                if not chunk:
                    raise EOFError("OpenSSH terminal closed before completing probe")
                output.extend(chunk)
                require(len(output) <= LOG_LIMIT and not overflow.is_set(), "terminal log budget exceeded")

        try:
            os.write(master, b"stty -echo; stty size; printf '\\nPTY_READY\\n'; read token; "
                     b"while [ \"$(stty size)\" != '41 132' ]; do sleep 0.02; done; "
                     b"stty size; printf '\\nRESIZED=%s\\n' \"$token\"; "
                     b"printf '\\nSIGNAL_READY\\n'; sleep 30\n")
            read_until(b"\r\nPTY_READY\r\n")
            require(b"24 80\r\n" in output, bytes(output))
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 41, 132, 0, 0))
            os.kill(process.pid, signal.SIGWINCH)
            os.write(master, b"resize-token\n")
            read_until(b"\r\nSIGNAL_READY\r\n")
            require(b"41 132\r\n" in output and b"RESIZED=resize-token\r\n" in output, bytes(output))
            # Let sleep take the foreground before generating terminal SIGINT.
            time.sleep(0.1)
            os.write(master, b"\x03")
            time.sleep(0.1)
            os.write(master, b"printf '\\nINTERRUPTED=%s\\n' \"$?\"; exit 17\n")
            read_until(b"\r\nINTERRUPTED=130\r\n")
            require(process.wait(timeout=max(0.1, deadline - time.monotonic())) == 17,
                    "unexpected interactive shell exit")
            return {"initial": [24, 80], "resized": [41, 132], "ctrl_c_status": 130, "shell_exit": 17}
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)
            stderr.join(timeout=2)
            os.close(master)
            (self.out / f"boot-{len(self.boots)}-terminal.stdout").write_bytes(output)
            (self.out / f"boot-{len(self.boots)}-terminal.stderr").write_bytes(error)

    def probes(self):
        result = {"instance_id": self.id, "status": "failed"}
        self.boots[-1]["probes"] = result
        phase = "ssh-exec"
        try:
            execution = self.ssh("exec", "printf ssh-out; printf ssh-err >&2; exit 29")
            require((execution.returncode, execution.stdout, execution.stderr) == (29, b"ssh-out", b"ssh-err"),
                    (execution.returncode, execution.stdout, execution.stderr))
            result["exec_stdout_stderr_exit"] = True
            phase = "ssh-stdin"
            data = bytes(range(256)) * 512
            echo = self.ssh("stdin", "cat", data=data)
            require(echo.returncode == 0 and echo.stdout == data and not echo.stderr, (echo.returncode, len(echo.stdout), echo.stderr))
            result["stdin_binary_eof_bytes"] = len(data)
            phase = "ssh-terminal-shell"
            result["openssh_terminal_shell_pty"] = self.terminal()
            phase = "sftp"
            source, target = self.path / "upload", self.path / "download"
            source.write_bytes(data)
            batch = self.path / "sftp.batch"
            batch.write_text(f"mkdir /workspace/sftp-dir\nput {source} /workspace/sftp-dir/upload\n"
                             f"rename /workspace/sftp-dir/upload /workspace/sftp-dir/renamed\n"
                             f"get /workspace/sftp-dir/renamed {target}\nrm /workspace/sftp-dir/renamed\n"
                             "rmdir /workspace/sftp-dir\n")
            transfer = command(["sftp", *self.options(), "-q", "-b", batch, "devcroft@guest"], self.out, "sftp")
            require(transfer.returncode == 0 and target.read_bytes() == data, (transfer.returncode, transfer.stderr))
            result["sftp_put_rename_get_remove_bytes"] = len(data)
            for label, instruction in [("sftp-private-denied", f"get /private/credential {self.path / 'denied'}\n"),
                                       ("sftp-runtime-denied", f"put {source} /bin/readonly-data\n")]:
                batch.write_text(instruction)
                denied = command(["sftp", *self.options(), "-q", "-b", batch, "devcroft@guest"], self.out, label)
                require(denied.returncode != 0 and b"Permission denied" in denied.stderr, (label, denied.returncode, denied.stderr))
            require(not (self.path / "denied").exists(), "denied SFTP read created output")
            result["sftp_nono_denials"] = True
            phase = "private-bootstrap-custody"
            disk = self.ssh("raw-state-denied", "head -c 1 /dev/vdb")
            require(disk.returncode != 0 and not disk.stdout and b"Permission denied" in disk.stderr,
                    (disk.returncode, disk.stdout, disk.stderr))
            result["private_bootstrap_disk_read_denied"] = True
            phase = "authentication"
            bad = self.ssh("wrong-client", "printf unexpected", key="wrong-client")
            require(bad.returncode == 255 and b"Permission denied" in bad.stderr, (bad.returncode, bad.stderr))
            result["wrong_client_key_rejected"] = True
            phase = "forwarding"
            forward = self.forward()
            require(forward.returncode == 0 and forward.stdout.endswith(f"instance={self.id}\n".encode())
                    and forward.stdout.startswith(b"HTTP/1.1 200 OK\r\n"), (forward.returncode, forward.stdout, forward.stderr))
            denied = self.forward("forward-denied", port=8081)
            require(denied.returncode != 0 and b"open failed" in denied.stderr, (denied.returncode, denied.stdout, denied.stderr))
            result["guest_loopback_forward"] = {"port": 8080, "instance_id": self.id, "live_port_8081_denied": True}
            phase = "keeper-regression"
            result["keeper"] = keeper_probe.run(self.path / "keeper.sock")
            require(result["keeper"]["status"] == "passed", result["keeper"])
            result["status"] = "passed"
        except Exception as e:
            result.update(phase=phase, error=f"{type(e).__name__}: {e}")
        return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--kernel", type=Path, default=ROOT / "artifacts/kernel-landlock/Image")
    p.add_argument("--guest", type=Path, default=ROOT / "artifacts/stage-b/guest.json")
    p.add_argument("--helper", type=Path, default=ROOT / "libkrun/target/debug/devcroft-krun-spike")
    p.add_argument("--workload", type=Path, help="optional immutable toolchain/closure disk")
    p.add_argument("--seed", type=Path, help="private ext4 seed, paired with --workload")
    p.add_argument("--out", type=Path, default=ROOT / "runs" / time.strftime("%Y%m%d-%H%M%S-ssh"))
    args = p.parse_args()
    if (args.workload is None) != (args.seed is None):
        p.error('--workload and --seed must be provided together')
    os.umask(0o077)
    args.out.mkdir(parents=True, exist_ok=False)
    guest = json.loads(args.guest.read_text())
    if guest.get('workload') and args.workload is None:
        p.error('this guest requires --workload and --seed')
    report = {"schema": 1, **helper_identity(args.helper), "host": list(os.uname()),
        "fixture": "real keeper/SSH/SFTP + private raw bootstrap + guest HTTP identity fixture",
        "qualification": "diagnostic-only", "guest_build": guest, "helper_artifact": artifact(args.helper),
        "client": {"implementation": "system OpenSSH", "known_hosts": "pinned from per-instance generated host keys"},
        "status": "failed", "instances": []}
    if args.workload:
        report.update(fixture='Nix flake closure' if guest.get('nix') else 'Alpine workload fixture',
            workload_artifact=artifact(args.workload), seed_artifact=artifact(args.seed),
            resources_per_vm={'vcpus': 2, 'memory_mib': 512})
    sources = [Path(__file__), ROOT / "smoke.py", ROOT / "keeper_probe.py",
               *sorted((ROOT / report["adapter"] / "src").glob("*.rs")), ROOT / report["adapter"] / "Cargo.lock"]
    if args.workload:
        sources.append(ROOT / 'workload_smoke.py')
    report["harness_sources"] = {str(f.relative_to(ROOT)): hashlib.sha256(f.read_bytes()).hexdigest() for f in sources}
    instances = []
    try:
        version = command(["ssh", "-V"], args.out, "openssh-version", timeout=5)
        report["client"]["version"] = version.stderr.decode().strip()
        provenance = command([args.helper.resolve(), "provenance"], args.out, "helper-provenance", timeout=5)
        require(provenance.returncode == 0, provenance.stderr)
        report["helper_provenance"] = json.loads(provenance.stdout)
        for label in ("a", "b"):
            if args.workload:
                # Imported after this module's Instance definition: reuse the
                # exact workload disk preparation without a second copy path.
                from workload_smoke import WorkloadInstance
                vm = WorkloadInstance(label, args.out, args.helper, args.kernel, guest, args.workload, args.seed)
            else:
                vm = Instance(label, args.out, args.helper, args.kernel, guest)
            instances.append(vm)
            report["instances"].append({"label": label, "instance_dir": str(vm.path), "instance_id": vm.id, "boots": vm.boots})
            vm.prepare(validation_timeout=120 if args.workload else 10)
            report["instances"][-1]["sandbox"] = vm.sandbox
        for vm in instances:
            vm.start()
        for vm in instances:
            vm.ready()
        require(all(vm.process.poll() is None for vm in instances), "pair not simultaneously live")
        report["two_simultaneous_instances"] = True
        for vm in instances:
            require(vm.probes()["status"] == "passed", vm.boots[-1]["probes"])
        a, b = instances
        sibling_key = a.ssh("sibling-client-key", "printf unexpected", key=b.path / "client")
        require(sibling_key.returncode == 255 and b"Permission denied" in sibling_key.stderr,
                (sibling_key.returncode, sibling_key.stderr))
        report["sibling_client_key_rejected"] = True
        crossed = a.ssh("sibling-host-key", "printf unexpected", endpoint=b.path)
        require(crossed.returncode == 255 and b"Host key verification failed" in crossed.stderr,
                (crossed.returncode, crossed.stderr))
        report["sibling_endpoint_host_key_rejected"] = True
        a.shutdown()
        a.start()
        a.ready()
        require(a.probes()["status"] == "passed", a.boots[-1]["probes"])
        sibling = b.forward("sibling-survives-restart")
        require(sibling.returncode == 0 and sibling.stdout.endswith(f"instance={b.id}\n".encode()), sibling.stderr)
        report["private_host_key_survives_cold_restart"] = True
        report["sibling_survives_restart"] = True
        for vm in instances:
            vm.shutdown()
        report["runtime_unchanged"] = artifact(Path(guest["runtime"]["path"])) == guest["runtime"]
        require(report["runtime_unchanged"], "shared runtime changed")
        if args.workload:
            report['workload_unchanged'] = artifact(args.workload) == report['workload_artifact']
            require(report['workload_unchanged'], 'shared workload changed')
        report["status"] = "passed"
    except Exception as e:
        report["error"] = f"{type(e).__name__}: {e}"
    finally:
        for vm in instances:
            vm.stop()
        report["cleanup"] = {"all_owned_helpers_reaped": all(vm.process is None for vm in instances),
            "endpoints_removed": all(not (vm.path / name).exists() for vm in instances for name in SOCKETS)}
        if not all(report['cleanup'].values()):
            report['status'], report['error'] = 'failed', 'owned resources survived cleanup'
        (args.out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    print(f"Raw evidence: {args.out.resolve()}")
    raise SystemExit(0 if report["status"] == "passed" else 1)


if __name__ == "__main__":
    main()
