#!/usr/bin/env python3
"""Separately labelled TRUSTED VS Code Remote-SSH row against a microVM guest.

Remote-SSH runs a server and extensions on the remote that can drive the local
client, so this row never stands in for the untrusted terminal/guest-served
editor paths. It runs an isolated VS Code (own user-data and extensions dirs,
copied from the installed Remote-SSH), disables workspace trust and allows the
project's folder-open task: both are trusted-project settings, stated in the
report. The guest is offline: the hash-pinned VS Code CLI archive is staged by
this trusted host harness where the installer looks for it.

Functional proof without UI automation: the project's folder-open task compiles
and runs a C program in the remote terminal; the harness reads the result and
exports the ELF over SSH, then repeats after a cold restart.
"""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
import urllib.request

from smoke import helper_identity, ROOT, artifact
from ssh_smoke import command, require, SOCKETS
from workload_smoke import WorkloadInstance

ALIAS = 'rsvm'
INSTALL = '/workspace/.vscode-server'
EXTENSIONS = ('ms-vscode-remote.remote-ssh', 'ms-vscode-remote.remote-ssh-edit', 'ms-vscode.remote-explorer')
TASKS = {'version': '2.0.0', 'tasks': [{
    'label': 'devcroft-remote-ssh-probe', 'type': 'shell',
    'command': 'date > started.txt && gcc -O2 main.c -o app && ./app > out.txt && uname -m >> out.txt && echo task-ok >> out.txt',
    'runOptions': {'runOn': 'folderOpen'}, 'presentation': {'reveal': 'always'}, 'problemMatcher': []}]}
MAIN_C = '#include <stdio.h>\nint main(void) { puts("remote-ssh-compiled"); return 0; }\n'


def digest(path):
    with open(path, 'rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def vscode_identity():
    lines = subprocess.check_output(['code', '--version'], text=True, timeout=30).split()
    extensions = {}
    for name in EXTENSIONS:
        found = sorted(Path.home().glob(f'.vscode/extensions/{name}-[0-9]*'))
        require(found, f'installed extension missing: {name}')
        source = found[-1]
        manifest = json.loads((source / 'package.json').read_text())
        extensions[name] = {'version': manifest['version'], 'source': str(source),
                            'package_json_sha256': digest(source / 'package.json')}
    return {'version': lines[0], 'commit': lines[1], 'extensions': extensions}


def pinned_archive(identity, slug, pins_path, cache):
    """Download (host side, trusted) and SHA-256 pin one VS Code archive for this commit."""
    commit = identity['commit']
    url = f'https://update.code.visualstudio.com/commit:{commit}/{slug}/stable'
    local = cache / f'{slug}-{commit}.tar.gz'
    if not local.exists():
        with urllib.request.urlopen(url, timeout=300) as response:
            data = response.read(512 * 1024 * 1024 + 1)
        require(len(data) <= 512 * 1024 * 1024, 'archive exceeds budget')
        local.write_bytes(data)
    pins = json.loads(pins_path.read_text()) if pins_path.exists() else {'schema': 1, 'archives': {}}
    key = f'{commit}/{slug}'
    actual = digest(local)
    if key not in pins['archives']:
        pins['archives'][key] = {'url': url, 'sha256': actual, 'bytes': local.stat().st_size,
                                 'frozen_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}
        pins_path.write_text(json.dumps(pins, indent=2) + '\n')
    require(pins['archives'][key]['sha256'] == actual, f'pinned VS Code archive changed: {slug}')
    return local, pins['archives'][key]


class RemoteSshInstance(WorkloadInstance):
    def ssh_config(self):
        opts = iter(self.options()[2:])
        cfg = [f'Host {ALIAS}', '  HostName guest', '  User devcroft']
        for flag in opts:
            value = next(opts)
            if flag == '-o':
                key, val = value.split('=', 1)
                cfg.append(f'  {key} {val}')
            elif flag == '-i':
                cfg.append(f'  IdentityFile {value}')
        return '\n'.join(cfg) + '\n'


class Editor:
    """An isolated VS Code process; never the user's own profile."""
    def __init__(self, vm, out, identity, tag, folder):
        self.dir = Path(tempfile.mkdtemp(prefix='dc-rs-', dir='/private/tmp'))
        self.out, self.tag = out, tag
        (self.dir / 'u/User').mkdir(parents=True)
        (self.dir / 'e').mkdir()
        for name, record in identity['extensions'].items():
            shutil.copytree(record['source'], self.dir / 'e' / Path(record['source']).name, symlinks=True)
        (self.dir / 'ssh_config').write_text(vm.ssh_config())
        self.settings = {
            'remote.SSH.configFile': str(self.dir / 'ssh_config'),
            'remote.SSH.localServerDownload': 'always',
            'remote.SSH.serverInstallPath': {ALIAS: INSTALL},
            'remote.SSH.serverPickPortsFromRange': {ALIAS: '9091-9091'},
            'remote.SSH.remotePlatform': {ALIAS: 'linux'},
            'security.workspace.trust.enabled': False,
            'task.allowAutomaticTasks': 'on',
            'telemetry.telemetryLevel': 'off', 'update.mode': 'none',
            'extensions.autoUpdate': False, 'extensions.autoCheckUpdates': False,
            'workbench.startupEditor': 'none'}
        (self.dir / 'u/User/settings.json').write_text(json.dumps(self.settings, indent=2))
        self.log = (out / f'{tag}-code.log').open('wb')
        self.process = subprocess.Popen(['code', '--user-data-dir', self.dir / 'u', '--extensions-dir', self.dir / 'e',
            '--new-window', '--wait', '--remote', f'ssh-remote+{ALIAS}', folder],
            stdout=self.log, stderr=subprocess.STDOUT, start_new_session=True)
        self.started = time.monotonic()

    def remote_log(self):
        logs = sorted(self.dir.glob('u/logs/*/window*/exthost/output_logging_*/*Remote - SSH.log'))
        return logs[-1].read_text(errors='replace') if logs else ''

    def close(self):
        # The isolated instance's Electron main process carries this user-data dir.
        subprocess.run(['pkill', '-f', '--', f'--user-data-dir {self.dir}/u'], check=False)
        if self.process.poll() is None:
            try:
                os.killpg(self.process.pid, signal.SIGTERM)
                self.process.wait(timeout=20)
            except (ProcessLookupError, PermissionError):
                pass
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=10)
        self.log.close()
        if (self.dir / 'u/logs').exists():
            shutil.copytree(self.dir / 'u/logs', self.out / f'{self.tag}-vscode-logs', dirs_exist_ok=True)
        shutil.rmtree(self.dir, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--fixture', choices=['nix', 'alpine'], required=True)
    parser.add_argument('--guest', type=Path, required=True, help='guest artifact directory')
    parser.add_argument('--toolchain', type=Path, required=True)
    parser.add_argument('--seed', type=Path, default=ROOT / 'artifacts/seed-3g/data.seed.img',
                        help='empty 3 GiB ext4 (make_seed.py); the 638 MiB server does not fit smaller seeds')
    parser.add_argument('--kernel', type=Path, default=ROOT / 'artifacts/kernel-landlock/Image')
    parser.add_argument('--helper', type=Path, default=ROOT / 'libkrun/target/debug/devcroft-krun-spike')
    parser.add_argument('--pins', type=Path, default=ROOT / 'fixtures/vscode-remote-ssh-aarch64.json')
    parser.add_argument('--timeout', type=int, default=420)
    parser.add_argument('--diagnostic-clear-remote-state', choices=['workspace-storage', 'user-and-machine'],
                        help='diagnostic only: drop persisted remote server state before reconnecting')
    parser.add_argument('--out', type=Path, default=ROOT / 'runs' / time.strftime('%Y%m%d-%H%M%S-remote-ssh'))
    args = parser.parse_args()
    os.umask(0o077)
    args.out.mkdir(parents=True, exist_ok=False)
    guest = json.loads((args.guest / 'guest.json').read_text())
    require(guest.get('remote_ssh_fixture', {}).get('extra_loopback_port') == 9091, 'not a Remote-SSH fixture guest')
    require(bool(guest.get('nix')) == (args.fixture == 'nix'), 'fixture kind does not match guest receipt')
    identity = vscode_identity()
    cache = ROOT / 'artifacts/vscode-cli'
    cache.mkdir(parents=True, exist_ok=True)
    # The exec-server installer requests the static musl CLI (cli-alpine-<arch>) on
    # every Linux; the server the CLI then runs is the glibc/musl-specific part.
    platform = 'alpine'
    server_slug = 'server-linux-arm64' if args.fixture == 'nix' else 'server-alpine-arm64'
    report = {'schema': 1, **helper_identity(args.helper), 'status': 'failed', 'fixture': args.fixture,
        'qualification': 'separately labelled TRUSTED Remote-SSH row; not an untrusted-project path, '
                         'not fleet eligibility; diagnostic-only',
        'trusted_settings': ['security.workspace.trust.enabled=false', 'task.allowAutomaticTasks=on'],
        'host': list(os.uname()), 'vscode': identity, 'kernel_artifact': artifact(args.kernel),
        'helper_artifact': artifact(args.helper), 'guest': guest, 'toolchain': artifact(args.toolchain),
        'seed': artifact(args.seed), 'probe_sha256': digest(__file__), 'sessions': []}
    vm = editor = None
    try:
        name = f"vscode-cli-{identity['commit']}.tar.gz"
        archive, pin = pinned_archive(identity, f'cli-{platform}-arm64', args.pins, cache)
        server_archive, server_pin = pinned_archive(identity, server_slug, args.pins, cache)
        report['cli_archive'] = {'slug': f'cli-{platform}-arm64', **pin}
        report['server_archive'] = {'slug': server_slug, **server_pin}
        vm = RemoteSshInstance('remote-ssh', args.out, args.helper, args.kernel, guest, args.toolchain, args.seed)
        vm.launch['memory_mib'] = 1024
        vm.launch_path.write_text(json.dumps(vm.launch, indent=2) + '\n')
        report['resources_per_vm'] = {'vcpus': vm.launch['vcpus'], 'memory_mib': 1024}
        vm.prepare(validation_timeout=30)
        for session in ('cold-install', 'after-cold-restart'):
            vm.start(); vm.ready(timeout=150)
            folder = '/workspace/project' if session == 'cold-install' else '/workspace/project-reopen'
            record = {'session': session, 'boot': len(vm.boots), 'folder': folder}
            report['sessions'].append(record)
            tasks = json.dumps(TASKS, indent=2)
            vm.execute(f'{session}-stage-folder', f'''set -eu
mkdir -p {folder}/.vscode {INSTALL}/.vscode-server
printf %s '{MAIN_C}' > {folder}/main.c
printf %s '{tasks}' > {folder}/.vscode/tasks.json''')
            if session == 'cold-install':
                staged = command(['scp', *vm.options(), str(archive), f'devcroft@guest:{INSTALL}/.vscode-server/{name}'],
                                 vm.out, 'stage-cli-archive', timeout=120)
                require(staged.returncode == 0, staged.stderr)
                vm.execute('stage-cli-flag', f"printf 'finished\\n' > {INSTALL}/.vscode-server/{name}.done; "
                           f"sha256sum {INSTALL}/.vscode-server/{name}")
                # The guest is offline: the CLI would otherwise download the server itself.
                staged = command(['scp', *vm.options(), str(server_archive), 'devcroft@guest:/workspace/server.tar.gz'],
                                 vm.out, 'stage-server-archive', timeout=300)
                require(staged.returncode == 0, staged.stderr)
                server_dir = f"{INSTALL}/.vscode-server/cli/servers/Stable-{identity['commit']}/server"
                checked = vm.execute('stage-server', f'''set -eu
test "$(sha256sum /workspace/server.tar.gz | cut -d' ' -f1)" = {server_pin['sha256']}
mkdir -p {server_dir}
tar -xzf /workspace/server.tar.gz -C {server_dir} --strip-components=1
rm /workspace/server.tar.gz
test -x {server_dir}/bin/code-server && test -x {server_dir}/node''', timeout=300)
                record['server_staged'] = server_dir
            else:
                # The first session's edit and build must survive the cold restart unchanged.
                first = report['sessions'][0]
                kept = command(['ssh', *vm.options(), 'devcroft@guest', 'cat /workspace/project/out.txt; sha256sum /workspace/project/app'],
                               vm.out, 'persisted-first-session', timeout=30).stdout.decode().splitlines()
                require(kept[:3] == first['task_output'] and kept[3].split()[0] == first['app_sha256'],
                        ('first session output/app did not persist', kept))
                record['first_session_output_and_app_persisted'] = True
                # VS Code's persisted remote state (data/User, data/Machine) suppresses a folder-open
                # task when the same folder is reopened; a second folder proves a new remote action.
                if args.diagnostic_clear_remote_state:
                    data = f'{INSTALL}/.vscode-server/data'
                    target = (f'{data}/User/workspaceStorage' if args.diagnostic_clear_remote_state == 'workspace-storage'
                              else f'{data}/User {data}/Machine')
                    vm.execute('diagnostic-clear-remote-state', f'find {data} -maxdepth 2 | sort; rm -rf {target}')
                    record['diagnostic_cleared_remote_state'] = args.diagnostic_clear_remote_state
            editor = Editor(vm, args.out, identity, session, folder)
            deadline = time.monotonic() + args.timeout
            out = b''
            while time.monotonic() < deadline:
                require(vm.process.poll() is None, 'helper exited during Remote-SSH session')
                probe = command(['ssh', *vm.options(), 'devcroft@guest', f'cat {folder}/out.txt 2>/dev/null'],
                                vm.out, f'{session}-poll', timeout=20)
                out = probe.stdout
                if b'task-ok' in out:
                    break
                log = editor.remote_log()
                require('[Session Summary]' not in log or 'Resolver error' not in log,
                        'Remote-SSH resolver failed: ' + log[log.rfind('[Session Summary]'):][:600])
                time.sleep(5)
            diagnostics = command(['ssh', *vm.options(), 'devcroft@guest', f'''cat {folder}/started.txt 2>&1
ps -eo pid,ppid,args 2>&1 | cut -c1-200 | head -40
for f in $(find {INSTALL} -path '*data/logs*' -name '*.log' 2>/dev/null | sort | tail -12); do echo "== $f"; tail -25 "$f"; done'''],
                vm.out, f'{session}-guest-diagnostics', timeout=60)
            record['guest_diagnostics'] = diagnostics.stdout.decode(errors='replace')[-20000:]
            full_log = editor.remote_log()
            record['remote_ssh_log_tail'] = full_log[-6000:]
            requested = sorted(set(re.findall(r'artifact==(\S+?)==', full_log)))
            record['installer_requested_artifacts'] = requested
            # The staged file name is platform-independent; a mismatch would pass silently.
            require(requested in ([], [f'cli-{platform}-arm64']), ('installer requested another platform', requested))
            record['seconds_to_task_output'] = time.monotonic() - editor.started
            require(out.splitlines() == [b'remote-ssh-compiled', b'aarch64', b'task-ok'], ('task output', out))
            exported = command(['ssh', *vm.options(), 'devcroft@guest', f'cat {folder}/app'],
                               vm.out, f'{session}-export-app', timeout=30).stdout
            require(exported[:4] == b'\x7fELF' and exported[18:20] == b'\xb7\x00', 'exported app is not an AArch64 ELF')
            record.update(task_output=out.decode().splitlines(), app_sha256=hashlib.sha256(exported).hexdigest(),
                          app_bytes=len(exported))
            installed = vm.execute(f'{session}-server-inventory',
                f'find {INSTALL} -maxdepth 3 -mindepth 1 | sort | head -40; du -sk {INSTALL}').stdout.decode()
            record['server_install_inventory'] = installed.splitlines()
            # A reconnect reports the existing install and downloads nothing.
            record['reused_existing_install'] = ('Found existing installation' in full_log
                                                 and 'didLocalDownload==0' in full_log)
            editor.close(); editor = None
            vm.shutdown()
        require(report['sessions'][1]['reused_existing_install'], 'server was not reused after cold restart')
        report['status'] = 'passed'
    except Exception as error:
        report['error'] = f'{type(error).__name__}: {error}'
    finally:
        if editor:
            report['sessions'][-1:] and report['sessions'][-1].setdefault('remote_ssh_log_tail', editor.remote_log()[-6000:])
            editor.close()
        if vm:
            vm.stop()
            report['cleanup'] = {'helper_reaped': vm.process is None,
                                 'endpoints_removed': all(not (vm.path / n).exists() for n in SOCKETS)}
            shutil.rmtree(vm.path, ignore_errors=True)
            shutil.rmtree(vm.controller_dir, ignore_errors=True)
            report['boots'] = vm.boots
        (args.out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: report.get(k) for k in ('status', 'error', 'fixture')}, indent=2))
    for s in report['sessions']:
        print(json.dumps({k: s.get(k) for k in ('session', 'task_output', 'seconds_to_task_output', 'reused_existing_install')}))
    print(f'Raw evidence: {args.out.resolve()}')
    raise SystemExit(0 if report['status'] == 'passed' else 1)


if __name__ == '__main__':
    main()
