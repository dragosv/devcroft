#!/usr/bin/env python3
"""Offline upstream build, Redis persistence, and crash during active writes."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import signal
import struct
import subprocess
import threading
import time

from smoke import helper_identity, ROOT, artifact, drain
from ssh_smoke import ENV, SOCKETS, Instance, command, require


def sparse_copy(source, target):
    with source.open("rb") as src, target.open("xb") as dst:
        while chunk := src.read(1024 * 1024):
            if chunk.count(0) == len(chunk):
                dst.seek(len(chunk), 1)
            else:
                dst.write(chunk)
        dst.truncate(src.tell())
    target.chmod(0o600)


HASHES = "/workspace/project/xxhsum /workspace/project/tests/sanity_test /workspace/hashes.txt /workspace/small/SHA256SUMS"


class WorkloadInstance(Instance):
    def __init__(self, label, out, helper, kernel, guest, workload, seed):
        super().__init__(label, out, helper, kernel, guest)
        self.nix = guest.get('nix', False)
        data = self.path / "data.img"
        sparse_copy(seed, data)
        self.launch.update(workload=artifact(workload), data=str(data), memory_mib=512, vcpus=2)
        self.launch_path.write_text(json.dumps(self.launch, indent=2) + "\n")
        (self.out / "launch.json").write_text(self.launch_path.read_text())

    def execute(self, name, script, timeout=120):
        result = command(["ssh", *self.options(), "devcroft@guest", script], self.out,
                         f"boot-{len(self.boots)}-{name}", timeout=timeout)
        require(result.returncode == 0, (name, result.returncode, result.stderr[-4000:]))
        return result

    def start_redis(self):
        check = command(["ssh", *self.options(), "devcroft@guest", "redis-server --check-system"],
            self.out, f"boot-{len(self.boots)}-redis-kernel-check", timeout=20)
        require(b"[madvise-free-fork-bug]...OK" in check.stdout,
                ("ARM64 safety check did not pass", check.returncode, check.stdout, check.stderr))
        result = self.execute("redis-start", """set -eu
mkdir -p /workspace/redis
if ! redis-server --daemonize yes --bind 127.0.0.1 --port 6379 --dir /workspace/redis \
 --appendonly yes --appendfsync always --save '' --maxmemory 48mb --maxmemory-policy noeviction \
 --logfile /workspace/redis/redis.log --pidfile /workspace/redis/redis.pid; then
 cat /workspace/redis/redis.log
 exit 1
fi
for n in 1 2 3 4 5 6 7 8 9 10; do
 if [ "$(redis-cli --raw PING)" = PONG ]; then break; fi
 sleep 0.1
done
test "$(redis-cli --raw PING)" = PONG
redis-server --version
redis-cli --raw CONFIG GET appendfsync
redis-cli --raw CONFIG GET appendonly
redis-cli --raw INFO server | grep '^run_id:'
""")
        require(b"appendfsync\nalways\n" in result.stdout and b"appendonly\nyes\n" in result.stdout, result.stdout)
        run_id = next(x.split(":", 1)[1].strip() for x in result.stdout.decode().splitlines() if x.startswith("run_id:"))
        self.boots[-1]["redis"] = {"ready": True, "run_id": run_id, "appendonly": True, "appendfsync": "always",
            "arm64_madvise_free_fork_check": "OK", "ignored_warnings": [],
            "system_check_exit_code": check.returncode,
            "clocksource_warning": b"Slow system clocksource" in check.stdout,
            "other_system_checks_qualified": False}
        return run_id

    def build(self):
        if self.nix:
            layout = self.execute('nix-closure-layout', '''set -eu
test "$(/bin/busybox readlink /nix)" = toolchain/nix
for name in gcc make redis-server code-server; do
 path=$(/bin/busybox readlink -f /usr/bin/$name)
 case "$path" in /toolchain/nix/store/*) printf '%s %s\\n' "$name" "$path";; *) exit 1;; esac
done
''')
            self.boots[-1]['nix_executables'] = layout.stdout.decode().splitlines()
        result = self.execute("build", """set -eu
test ! -e /workspace/project
cp -R /toolchain/fixture/xxHash /workspace/project
cp -R /toolchain/fixture/small /workspace/small
uname -sm
gcc --version
make --version
cd /workspace/project
make -j2 xxhsum
make -C tests test_sanity
./xxhsum --version
cd /workspace/small
sha256sum -c SHA256SUMS > /workspace/small-verify.log
find d* -type f | sort | xargs /workspace/project/xxhsum > /workspace/hashes.txt
test "$(wc -l < /workspace/hashes.txt)" -eq 2048
sha256sum """ + HASHES, timeout=180)
        hashes = self.parse_hashes(result.stdout)
        require(len(hashes) == 4, result.stdout[-3000:])
        self.boots[-1]["build"] = {"project": "xxHash 0.8.3", "compiler_runs_in_guest": True,
            "network": "no NIC/TSI; fixed source and toolchain pre-materialized", "upstream_test_sanity": "passed",
            "small_files_verified_and_hashed": 2048, "sha256": hashes}
        exported = self.execute("export-xxhsum", "cat /workspace/project/xxhsum")
        require(exported.stdout.startswith(b"\x7fELF"), "build did not produce an ELF executable")
        require(hashlib.sha256(exported.stdout).hexdigest() == hashes["/workspace/project/xxhsum"], "exported artifact mismatch")
        if self.nix:
            data = exported.stdout
            require(data[:6] == b'\x7fELF\x02\x01' and data[18:20] == b'\xb7\x00', 'not an AArch64 ELF')
            offset = struct.unpack_from('<Q', data, 32)[0]
            size, count = struct.unpack_from('<HH', data, 54)
            interpreters = []
            for index in range(count):
                header = offset + size * index
                if struct.unpack_from('<I', data, header)[0] == 3:
                    start = struct.unpack_from('<Q', data, header + 8)[0]
                    length = struct.unpack_from('<Q', data, header + 32)[0]
                    interpreters.append(data[start:start+length].rstrip(b'\0').decode())
            require(len(interpreters) == 1 and interpreters[0].startswith('/nix/store/'), interpreters)
            self.boots[-1]['build']['nix_elf_interpreter'] = interpreters[0]
        (self.out / "xxhsum.aarch64.elf").write_bytes(exported.stdout)
        return hashes

    @staticmethod
    def parse_hashes(data):
        found = {}
        for line in data.decode().splitlines():
            fields = line.split()
            if len(fields) == 2 and len(fields[0]) == 64 and fields[1].startswith("/workspace/"):
                found[fields[1]] = fields[0]
        return found

    def verify_build(self, expected):
        result = self.execute("verify-persisted-build", """set -eu
/workspace/project/tests/sanity_test
cd /workspace/small
sha256sum -c SHA256SUMS > /workspace/recovery-small-verify.log
sha256sum """ + HASHES)
        found = self.parse_hashes(result.stdout)
        require(found == expected, (found, expected))
        self.boots[-1]["persisted_build_verified"] = {"sha256": found, "small_files": 2048,
            "existing_binary_test_sanity": "passed", "rebuilt_before_check": False}

    def acknowledged_values(self):
        return {f"durable:{n}": f"{self.id}:{n}:" + hashlib.sha256(f"{self.id}:{n}".encode()).hexdigest()
                for n in range(32)}

    def write_acknowledged(self):
        script = ["set -eu"]
        for key, value in self.acknowledged_values().items():
            script.append(f"test \"$(redis-cli --raw SET {shlex.quote(key)} {shlex.quote(value)})\" = OK")
            script.append(f"printf 'ACK {key}\\n'")
        result = self.execute("acknowledged-write", "\n".join(script))
        require(result.stdout.count(b"ACK durable:") == 32, result.stdout)
        self.boots[-1]["acknowledged_keys_written"] = 32

    def verify_values(self, name="verify-redis"):
        expected = self.acknowledged_values()
        result = self.execute(name, "redis-cli --raw MGET " + " ".join(expected))
        require(result.stdout.decode().splitlines() == list(expected.values()), result.stdout)
        checksum = hashlib.sha256(result.stdout).hexdigest()
        self.boots[-1]["acknowledged_redis_keys_verified"] = {"count": 32, "sha256": checksum}
        return checksum

    def set_commands_processed(self):
        # Count SET only: polling INFO must not count as write progress.
        result = self.execute("redis-progress", "redis-cli --raw INFO commandstats", timeout=10)
        for line in result.stdout.decode().splitlines():
            if line.startswith("cmdstat_set:"):
                return int(line.split("calls=", 1)[1].split(",", 1)[0])
        return 0  # A freshly restarted server has no SET entry until its first SET.

    def clean_shutdown(self):
        self.execute("redis-stop", "redis-cli SHUTDOWN")
        self.shutdown()


class Writer:
    def __init__(self, vm):
        self.vm = vm
        self.stdout, self.stderr, self.overflow = bytearray(), bytearray(), threading.Event()
        self.process = subprocess.Popen(["ssh", *vm.options(), "devcroft@guest",
            "exec redis-benchmark -h 127.0.0.1 -p 6379 -t set -n 10000000 -r 1000 -d 256 -P 8 --csv"],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            env=ENV, close_fds=True, start_new_session=True)
        self.threads = [threading.Thread(target=drain, args=(stream, buf, self.overflow), daemon=True)
            for stream, buf in [(self.process.stdout, self.stdout), (self.process.stderr, self.stderr)]]
        for thread in self.threads:
            thread.start()

    def stop(self):
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            os.killpg(self.process.pid, signal.SIGKILL)
            self.process.wait(timeout=5)
        for thread in self.threads:
            thread.join(timeout=2)
        (self.vm.out / "crash-writer.stdout").write_bytes(self.stdout)
        (self.vm.out / "crash-writer.stderr").write_bytes(self.stderr)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--kernel", type=Path, default=ROOT / "artifacts/kernel-landlock/Image")
    p.add_argument("--guest", type=Path, default=ROOT / "artifacts/stage-b-workload/guest.json")
    p.add_argument("--helper", type=Path, default=ROOT / "libkrun/target/debug/devcroft-krun-spike")
    p.add_argument("--workload", type=Path, default=ROOT / "artifacts/workload/toolchain.img")
    p.add_argument("--seed", type=Path, default=ROOT / "artifacts/workload/data.seed.img")
    p.add_argument("--out", type=Path, default=ROOT / "runs" / time.strftime("%Y%m%d-%H%M%S-workload"))
    args = p.parse_args()
    os.umask(0o077)
    args.out.mkdir(parents=True, exist_ok=False)
    guest = json.loads(args.guest.read_text())
    report = {"schema": 1, **helper_identity(args.helper), "qualification": "diagnostic-only",
        "fixture": "Nix flake closure" if guest.get('nix') else "Alpine APK closure",
        "host": list(os.uname()), "guest_build": guest, "helper_artifact": artifact(args.helper),
        "workload_artifact": artifact(args.workload), "seed_artifact": artifact(args.seed),
        "workload_build": json.loads((args.workload.parent / "build.json").read_text()),
        "resources_per_vm": {"vcpus": 2, "memory_mib": 512}, "instances": [], "status": "failed"}
    instances, writer = [], None
    try:
        for label in ("a", "b"):
            vm = WorkloadInstance(label, args.out, args.helper, args.kernel, guest, args.workload, args.seed)
            instances.append(vm)
            report["instances"].append({"label": label, "instance_id": vm.id,
                "instance_dir": str(vm.path), "boots": vm.boots})
            vm.prepare(validation_timeout=90)
            report["instances"][-1]["sandbox"] = vm.sandbox
        for vm in instances:
            vm.start()
        for vm in instances:
            vm.ready(timeout=120)
        a, b = instances
        builds, checksums, run_ids = {}, {}, {}
        for vm in instances:
            builds[vm.label] = vm.build()
            run_ids[vm.label] = vm.start_redis()
            vm.write_acknowledged()
            checksums[vm.label] = vm.verify_values()
            reply = vm.ssh("redis-forward", data=b"*1\r\n$4\r\nPING\r\n*1\r\n$4\r\nQUIT\r\n",
                           extra=["-W", "127.0.0.1:6379"])
            require(reply.returncode == 0 and reply.stdout == b"+PONG\r\n+OK\r\n", (reply.returncode, reply.stdout, reply.stderr))
        report["two_real_redis_services_same_guest_port"] = 6379
        a.clean_shutdown()
        a.start()
        a.ready(timeout=120)
        a.verify_build(builds["a"])
        a.start_redis()
        require(a.verify_values() == checksums["a"], "clean restart changed acknowledged values")
        report["clean_restart_persistence"] = True
        before = a.set_commands_processed()
        writer = Writer(a)
        deadline = time.monotonic() + 20
        while True:
            require(writer.process.poll() is None, "write workload exited before forced stop")
            after = a.set_commands_processed()
            if after >= before + 200:
                break
            require(time.monotonic() < deadline, "Redis write workload made no progress")
            time.sleep(0.05)
        a.boots[-1]["forced_stop"] = {"expected": True, "signal": "SIGKILL",
            "redis_set_calls_before": before, "redis_set_calls_at_stop": after,
            "writer_still_running": writer.process.poll() is None}
        require(writer.process.poll() is None, "writer finished before forced stop")
        a.stop()
        require(a.boots[-1]["exit_code"] == -signal.SIGKILL, "helper was not killed as expected")
        writer.stop()
        require(not writer.overflow.is_set(), "writer log budget exceeded")
        a.boots[-1]["forced_stop"]["writer_exit_code"] = writer.process.returncode
        writer = None
        require(b.verify_values("sibling-after-crash") == checksums["b"], "sibling data changed during crash")
        sibling_id = b.execute("sibling-run-id", "redis-cli --raw INFO server | grep '^run_id:'").stdout.decode().strip().split(":")[1]
        require(sibling_id == run_ids["b"], "sibling Redis restarted")
        report["sibling_service_survives_crash"] = True
        a.start()
        a.ready(timeout=120)
        a.verify_build(builds["a"])
        a.start_redis()
        require(a.verify_values() == checksums["a"], "crash recovery lost acknowledged values")
        report["active_write_crash_recovery"] = True
        report["acknowledged_keys_recovered"] = 32
        for vm in instances:
            vm.clean_shutdown()
        report["immutable_toolchain_unchanged"] = artifact(args.workload) == report["workload_artifact"]
        require(report["immutable_toolchain_unchanged"], "immutable toolchain changed")
        report["status"] = "passed"
    except Exception as e:
        report["error"] = f"{type(e).__name__}: {e}"
    finally:
        for vm in instances:
            vm.stop()
        if writer:
            writer.stop()
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
