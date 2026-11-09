#!/usr/bin/env python3
"""Exercise the guest-served editor through SSH and a real isolated Chrome profile."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shlex
import shutil
import signal
import socket
import subprocess
import threading
import time
import urllib.request

from smoke import ROOT, artifact, drain
from ssh_smoke import ENV, SOCKETS, command, require
from workload_smoke import WorkloadInstance


class Process:
    def __init__(self, argv, out, name):
        self.out, self.name = out, name
        self.stdout, self.stderr, self.overflow = bytearray(), bytearray(), threading.Event()
        self.process = subprocess.Popen(list(map(str, argv)), stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=ENV, close_fds=True, start_new_session=True)
        self.threads = [threading.Thread(target=drain, args=(stream, buf, self.overflow), daemon=True)
            for stream, buf in [(self.process.stdout, self.stdout), (self.process.stderr, self.stderr)]]
        for thread in self.threads:
            thread.start()

    def stop(self):
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGTERM)
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=5)
        for thread in self.threads:
            thread.join(timeout=2)
        (self.out / f"{self.name}.stdout").write_bytes(self.stdout)
        (self.out / f"{self.name}.stderr").write_bytes(self.stderr)


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--kernel', type=Path, default=ROOT / 'artifacts/kernel-landlock/Image')
    p.add_argument('--guest', type=Path, default=ROOT / 'artifacts/stage-b-workload/guest.json')
    p.add_argument('--helper', type=Path, default=ROOT / 'libkrun/target/debug/devcroft-krun-spike')
    p.add_argument('--editor', type=Path, default=ROOT / 'artifacts/editor/editor.img')
    p.add_argument('--seed', type=Path, default=ROOT / 'artifacts/workload/data.seed.img')
    p.add_argument('--out', type=Path, default=ROOT / 'runs' / time.strftime('%Y%m%d-%H%M%S-editor'))
    args = p.parse_args()
    node = shutil.which('node')
    if node is None:
        p.error('host Node is required for the isolated browser client')
    os.umask(0o077)
    args.out.mkdir(parents=True, exist_ok=False)
    guest = json.loads(args.guest.read_text())
    nix = guest.get('nix', False)
    report = {'schema': 1, 'qualification': 'diagnostic-only', 'adapter': 'libkrun',
        'host': list(os.uname()), 'editor_artifact': artifact(args.editor), 'helper_artifact': artifact(args.helper),
        'kernel_artifact': artifact(args.kernel), 'seed_artifact': artifact(args.seed),
        'editor_build': json.loads((args.editor.parent / 'build.json').read_text()),
        'transport': 'host loopback HTTP through pinned-key SSH over vsock; no guest NIC/TSI',
        'fixture': 'Nix flake closure guest-served editor; trusted Remote-SSH not qualified' if nix else
                   'Alpine/npm guest-served editor; Nix closure and trusted Remote-SSH not qualified',
        'guest_build': guest, 'resources': {'vcpus': 2, 'memory_mib': 1024}, 'status': 'failed', 'boots': []}
    vm, server, forward = None, None, None
    password = secrets.token_urlsafe(24)
    try:
        vm = WorkloadInstance('editor', args.out, args.helper, args.kernel, guest, args.editor, args.seed)
        vm.launch['memory_mib'] = 1024
        vm.launch_path.write_text(json.dumps(vm.launch, indent=2) + '\n')
        (vm.out / 'launch.json').write_text(vm.launch_path.read_text())
        report['instance_dir'], report['instance_id'], report['boots'] = str(vm.path), vm.id, vm.boots
        vm.prepare(validation_timeout=120)
        report['sandbox'] = vm.sandbox
        marker = f'editor-{vm.id}'
        source = '#include <stdio.h>\nint main(void) {\n    puts("' + marker + '");\n    return 0;\n}\n'
        expected = (marker + '\n').encode()
        saved_hash = None
        for mode in ('edit', 'reopen'):
            vm.start()
            vm.ready(timeout=120)
            if mode == 'edit':
                vm.execute('seed-editor-project', "mkdir -p /workspace/editor-project; printf 'int main(void) { return 0; }\\n' > /workspace/editor-project/main.c")
                settings = json.dumps({'editor.autoIndent': 'none', 'editor.formatOnPaste': False,
                    'editor.formatOnSave': False, 'editor.formatOnType': False})
                vm.execute('fixture-editor-settings', 'mkdir -p /workspace/.editor/user/User; printf %s ' +
                    shlex.quote(settings) + ' > /workspace/.editor/user/User/settings.json')
            else:
                persisted = vm.execute('verify-before-reopen', 'cat /workspace/editor-project/main.c; cat /workspace/editor-project/terminal-output.txt; cat /workspace/editor-project/machine.txt')
                require(persisted.stdout == source.encode() + expected + b'aarch64\n', persisted.stdout)
            entry = '/usr/bin/code-server' if nix else '/usr/bin/node /toolchain/editor/node_modules/code-server/out/node/entry.js'
            launch = f"""set -eu
mkdir -p /workspace/.editor/user /workspace/.editor/extensions /tmp/editor-runtime
/bin/busybox chmod 0700 /tmp/editor-runtime
export PASSWORD={shlex.quote(password)}
export XDG_CONFIG_HOME=/workspace/.editor/config XDG_DATA_HOME=/workspace/.editor/data XDG_RUNTIME_DIR=/tmp/editor-runtime
exec {entry} \
 --bind-addr 127.0.0.1:9090 --auth password --disable-telemetry --disable-update-check \
 --user-data-dir /workspace/.editor/user --extensions-dir /workspace/.editor/extensions /workspace/editor-project
"""
            server = Process(['ssh', *vm.options(), 'devcroft@guest', launch], vm.out, f'boot-{len(vm.boots)}-server')
            port = free_port()
            forward = Process(['ssh', *vm.options(), '-o', 'ExitOnForwardFailure=yes', '-N',
                '-L', f'127.0.0.1:{port}:127.0.0.1:9090', 'devcroft@guest'], vm.out, f'boot-{len(vm.boots)}-forward')
            deadline = time.monotonic() + 90
            while True:
                require(server.process.poll() is None, ('editor process exited', bytes(server.stderr[-3000:])))
                require(forward.process.poll() is None, ('forward exited', bytes(forward.stderr[-3000:])))
                try:
                    with urllib.request.urlopen(f'http://127.0.0.1:{port}/healthz', timeout=2) as response:
                        require(response.status == 200, 'editor health endpoint failed')
                    break
                except OSError:
                    require(time.monotonic() < deadline, 'editor did not become ready')
                    time.sleep(0.1)
            browser_out = vm.out / f'boot-{len(vm.boots)}-browser'
            browser_out.mkdir()
            config = vm.path / 'browser-config.json'
            config.write_text(json.dumps({'url': f'http://127.0.0.1:{port}/?folder=/workspace/editor-project',
                'password': password, 'out': str(browser_out), 'source': source, 'marker': marker, 'mode': mode}))
            result = command([node, ROOT / 'editor-browser/probe.mjs', config], vm.out,
                             f'boot-{len(vm.boots)}-browser-client', timeout=240)
            browser = json.loads((browser_out / 'browser-report.json').read_text())
            vm.boots[-1]['browser'] = browser
            require(result.returncode == 0 and browser['status'] == 'passed', browser)
            deadline = time.monotonic() + 20
            while True:
                check = vm.execute('verify-editor-write', 'if test -f /workspace/editor-project/machine.txt; then cat /workspace/editor-project/main.c; cat /workspace/editor-project/terminal-output.txt; cat /workspace/editor-project/machine.txt; fi')
                if check.stdout == source.encode() + expected + b'aarch64\n':
                    break
                require(time.monotonic() < deadline, check.stdout)
                time.sleep(0.1)
            binary = vm.execute('export-editor-binary', 'cat /workspace/editor-project/app').stdout
            digest = hashlib.sha256(binary).hexdigest()
            if saved_hash is not None:
                require(digest == saved_hash, 'binary changed after restart')
            saved_hash = digest
            vm.boots[-1]['remote_files_verified'] = {'source_sha256': hashlib.sha256(source.encode()).hexdigest(),
                'binary_sha256': digest, 'terminal_output': marker, 'machine': 'aarch64'}
            server.stop(); forward.stop()
            require(not server.overflow.is_set() and not forward.overflow.is_set(), 'server/forward log budget exceeded')
            server, forward = None, None
            vm.shutdown()
        report['persisted_editor_file_and_binary_after_restart'] = True
        report['editor_image_unchanged'] = artifact(args.editor) == report['editor_artifact']
        require(report['editor_image_unchanged'], 'read-only editor image changed')
        report['status'] = 'passed'
    except Exception as error:
        report['error'] = f'{type(error).__name__}: {error}'.replace(password, '<redacted>')
        if vm and vm.process and vm.process.poll() is None:
            try:
                vm.execute('editor-failure-diagnostics', '/bin/busybox ps; /bin/busybox ls -la /workspace/editor-project; cat /workspace/editor-project/main.c; cat /workspace/editor-project/terminal-output.txt; true', timeout=20)
            except Exception:
                pass  # Retain the original failure; diagnostic transport may be gone.
    finally:
        if server: server.stop()
        if forward: forward.stop()
        if vm: vm.stop()
        report['cleanup'] = {'owned_helper_reaped': vm is None or vm.process is None,
            'endpoints_removed': vm is None or all(not (vm.path / name).exists() for name in SOCKETS)}
        if not all(report['cleanup'].values()):
            report['status'], report['error'] = 'failed', 'owned resources survived cleanup'
        (args.out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    print(f'Raw evidence: {args.out.resolve()}')
    raise SystemExit(0 if report['status'] == 'passed' else 1)


if __name__ == '__main__':
    main()
