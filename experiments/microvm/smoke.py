#!/usr/bin/env python3
"""Run two confined microVM boots, with optional real keeper integration probes.

This is a functional diagnostic, not a density benchmark or backend qualification.
Raw disks test the block device, not filesystem/application crash consistency.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import signal
import socket
import subprocess
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parent
LOG_LIMIT = 1024 * 1024


def artifact(path):
    path = path.resolve(strict=True)
    with path.open("rb") as f:
        digest = hashlib.file_digest(f, "sha256").hexdigest()
    return {"path": str(path), "sha256": digest}



def helper_identity(binary):
    """Attribute evidence to the actual pinned helper, rather than the default."""
    provenance = json.loads(subprocess.check_output(
        [str(binary.resolve()), "provenance"], env={"PATH": "/usr/bin:/bin"}, timeout=5))
    pins = {"6151710f4ea627cf0e5dbce78e4192fb4d64c2e9": "libkrun",
            "351d1a15728e998486232e4815ba075a22e44f56": "hvi"}
    adapter = pins[provenance["revision"]]
    if provenance.get("adapter", adapter) != adapter:
        raise ValueError("helper provenance adapter disagrees with its revision")
    return {"adapter": adapter, "helper_provenance": provenance}


def line(sock):
    result = bytearray()
    while len(result) <= 1024:
        data = sock.recv(1)
        if not data:
            raise ValueError("guest closed before completing a frame")
        result += data
        if data == b"\n":
            return bytes(result)
    raise ValueError("guest frame exceeds 1024 bytes")


def drain(stream, data, overflow):
    while chunk := stream.read(4096):
        room = LOG_LIMIT - len(data)
        data.extend(chunk[:room])
        if len(chunk) > room:
            overflow.set()


def connect(path, process, deadline, overflow):
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"helper exited before READY: {process.returncode}")
        if overflow.is_set():
            raise RuntimeError("helper log budget exceeded")
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(min(1.0, max(0.01, deadline - time.monotonic())))
        try:
            sock.connect(str(path))
            greeting = line(sock)
            if not re.fullmatch(rb"READY 1 state=[0-9]+ ro=1 landlock=[0-9]+ policy=[01](?: setup=(?:open|closed))?\n", greeting):
                raise ValueError(f"invalid READY: {greeting!r}")
            sock.settimeout(5)
            return sock, greeting
        except (OSError, ValueError):
            sock.close()
            time.sleep(0.05)
    raise TimeoutError("guest did not reach READY before deadline")


def boot(binary, launch, instance, out, index, timeout, keeper=False):
    stdout, stderr = bytearray(), bytearray()
    overflow = threading.Event()
    start = time.monotonic()
    process = subprocess.Popen([str(binary), "run", str(launch)], stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, close_fds=True,
        start_new_session=True, env={"PATH": "/usr/bin:/bin"})
    threads = [threading.Thread(target=drain, args=(stream, data, overflow), daemon=True)
               for stream, data in [(process.stdout, stdout), (process.stderr, stderr)]]
    for thread in threads:
        thread.start()
    result = {"boot": index, "status": "failed"}
    try:
        sock, greeting = connect(instance / "control.sock", process, start + timeout, overflow)
        with sock:
            result["ready_seconds"] = time.monotonic() - start
            result["greeting"] = greeting.decode().strip()
            expected = f"READY 1 state={index} ro=1 ".encode()
            if not greeting.startswith(expected):
                raise AssertionError(f"persistence mismatch: {greeting!r}, expected prefix {expected!r}")
            abi = int(re.search(rb"landlock=([0-9]+)", greeting).group(1))
            result["landlock_abi"] = abi
            result["guest_filesystem_policy"] = abi >= 4 and b"policy=1\n" in greeting
            if not result["guest_filesystem_policy"]:
                result["guest_policy_error"] = "Landlock ABI >=4 and positive/negative policy probes required"
            nonce = secrets.token_hex(16).encode()
            result["phase"] = "ping"
            sock.sendall(b"PING " + nonce + b"\n")
            if line(sock) != b"PONG " + nonce + b"\n":
                raise AssertionError("ping response mismatch")
            result["ping"] = True
            result["readonly_backend"] = True
            result["phase"] = "half-close"
            pending = [(f"{n:04d}:" + "x" * 500).encode() for n in range(256)]
            sock.sendall(b"".join(b"PING " + item + b"\n" for item in pending))
            sock.shutdown(socket.SHUT_WR)
            try:
                for item in pending:
                    if line(sock) != b"PONG " + item + b"\n":
                        raise AssertionError("pending data lost or reordered at half-close")
                if line(sock) != b"EOF\n":
                    raise AssertionError("half-close response mismatch")
                result["half_close"] = True
                result["half_close_pending_bytes"] = sum(len(item) + 6 for item in pending)
            except (TimeoutError, OSError, ValueError, AssertionError) as e:
                # Retain the failure but still exercise independent stop/restart
                # behavior. An upstream transport defect must not hide those rows.
                result["half_close"] = False
                result["half_close_error"] = str(e)
        # Reconnect proves the listener survives the first connection's EOF.
        result["phase"] = "reconnect"
        sock, again = connect(instance / "control.sock", process, time.monotonic() + 15, overflow)
        with sock:
            sock.sendall(b"x" * 1025)
            if line(sock) != b"ERR frame\n":
                raise AssertionError("oversized frame was not rejected")
            result["oversized_frame_rejected"] = True
        if keeper:
            import keeper_probe
            result["phase"] = "keeper-probes"
            result["keeper"] = keeper_probe.run(instance / "keeper.sock")
        sock, again = connect(instance / "control.sock", process, time.monotonic() + 5, overflow)
        with sock:
            if again != greeting:
                raise AssertionError("reconnected to a different guest generation")
            result["phase"] = "shutdown"
            sock.sendall(b"SHUTDOWN\n")
            if line(sock) != b"BYE\n":
                raise AssertionError("missing guest shutdown acknowledgement")
        result["phase"] = "wait-for-exit"
        code = process.wait(timeout=10)
        if code != 0 or overflow.is_set():
            raise RuntimeError(f"unclean helper exit/log overflow: {code}")
        passed = (result.get("half_close") and result.get("guest_filesystem_policy")
                  and (not keeper or result.get("keeper", {}).get("status") == "passed"))
        result.update(status="passed" if passed else "failed",
                      clean_shutdown=True, exit_code=code)
    except Exception as e:
        result["error"] = str(e)
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)
            result["forced_cleanup"] = True
        for thread in threads:
            thread.join(timeout=2)
        if keeper:
            result["guest_console_captured"] = b"guest keeper: nono applied; serving port 1025" in stderr
            if not result["guest_console_captured"]:
                result.update(status="failed", console_error="missing keeper console positive control")
        for name, data in [("stdout", stdout), ("stderr", stderr)]:
            (out / f"boot-{index}.{name}.log").write_bytes(data)
        # Only after our owned process has exited. Never unlink a live listener.
        (instance / "control.sock").unlink(missing_ok=True)
        (instance / "keeper.sock").unlink(missing_ok=True)
        (instance / "ssh.sock").unlink(missing_ok=True)
    if result["status"] != "passed":
        print(stderr.decode(errors="replace")[-8000:])
        print(stdout.decode(errors="replace")[-8000:])
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--kernel", type=Path, default=ROOT / "artifacts/kernel-landlock/Image")
    p.add_argument("--guest", type=Path, default=ROOT / "artifacts/stage-a/guest.json")
    p.add_argument("--helper", type=Path, default=ROOT / "libkrun/target/debug/devcroft-krun-spike")
    p.add_argument("--out", type=Path, default=ROOT / "runs" / time.strftime("%Y%m%d-%H%M%S"))
    p.add_argument("--timeout", type=float, default=30)
    p.add_argument("--keeper", action="store_true", help="require the real keeper + nono Stage-B probes")
    args = p.parse_args()
    if not 1 <= args.timeout <= 120:
        p.error("timeout must be between 1 and 120 seconds")
    args.out.mkdir(parents=True, exist_ok=False)
    guest = json.loads(args.guest.read_text())
    if args.keeper and guest["fixture"] != "stage-b-real-keeper":
        p.error("--keeper requires artifacts/stage-b/guest.json")
    # Short private socket path, independent of repository location.
    instance = Path(tempfile.mkdtemp(prefix="dk-", dir="/private/tmp" if os.uname().sysname == "Darwin" else "/tmp"))
    state = instance / "state.img"
    with state.open("xb") as f:
        f.truncate(16 * 1024 * 1024)
    launch = {"schema": 1, "kernel": artifact(args.kernel),
              "initramfs": guest["initramfs"], "runtime": guest["runtime"],
              "state": str(state), "instance_dir": str(instance), "vcpus": 1, "memory_mib": 256}
    path = instance / "launch.json"
    path.write_text(json.dumps(launch, indent=2) + "\n")
    (args.out / "launch.json").write_text(path.read_text())
    report = {"schema": 1, **helper_identity(args.helper), "host": list(os.uname()),
              "fixture": guest["fixture"], "qualification": "diagnostic-only",
              "instance_dir": str(instance), "boots": []}
    report["helper_artifact"] = artifact(args.helper)
    report["guest_build"] = guest
    original_runtime = launch["runtime"]["sha256"]
    try:
        subprocess.run([str(args.helper.resolve()), "check", str(path)], check=True,
                       env={"PATH": "/usr/bin:/bin"}, capture_output=True)
        sentinel = args.out.resolve() / "outside-sentinel"
        sentinel.write_text("controller-owned denial fixture\n")
        check = subprocess.run([str(args.helper.resolve()), "sandbox-test", str(path), str(sentinel)],
            env={"PATH": "/usr/bin:/bin"}, capture_output=True, timeout=10)
        (args.out / "sandbox.stdout.log").write_bytes(check.stdout)
        (args.out / "sandbox.stderr.log").write_bytes(check.stderr)
        if check.returncode:
            raise RuntimeError(f"confinement selftest failed: {check.stderr.decode(errors='replace')}")
        report["sandbox"] = json.loads(check.stdout)
        (instance / "control.sock").unlink(missing_ok=True)
        (instance / "keeper.sock").unlink(missing_ok=True)
        (instance / "ssh.sock").unlink(missing_ok=True)
        for index in (1, 2):
            result = boot(args.helper.resolve(), path, instance, args.out, index, args.timeout, keeper=args.keeper)
            report["boots"].append(result)
            if not result.get("clean_shutdown"):
                break
        report["runtime_unchanged"] = artifact(Path(launch["runtime"]["path"]))["sha256"] == original_runtime
        report["status"] = "passed" if (len(report["boots"]) == 2 and
            all(x["status"] == "passed" for x in report["boots"]) and report["runtime_unchanged"]) else "failed"
    except Exception as e:
        report.update(status="failed", error=str(e))
    (args.out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    print(f"Raw evidence: {args.out.resolve()}")
    # Retain instance disk and raw logs for diagnosis; no user data was imported.
    raise SystemExit(0 if report["status"] == "passed" else 1)


if __name__ == "__main__":
    main()
