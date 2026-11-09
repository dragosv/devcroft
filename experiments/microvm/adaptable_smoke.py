#!/usr/bin/env python3
"""Private setup window, persisted adaptation and fresh recipe reconstruction."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import time

from adaptable.recipe import encoded, load
from smoke import helper_identity, ROOT, artifact, connect, line
from ssh_smoke import command, require, SOCKETS
from workload_smoke import WorkloadInstance


class AdaptableInstance(WorkloadInstance):
    def __init__(self, label, args, guest):
        super().__init__(label, args.out, args.helper, args.kernel, guest, args.inputs, args.seed)
        self.guest = guest

    def select(self, guest):
        require(self.process is None, 'cannot replace a running guest')
        self.guest = guest
        self.launch.update(initramfs=guest['initramfs'], runtime=guest['runtime'])
        self.launch_path.write_text(json.dumps(self.launch, indent=2) + '\n')
        self.prepare(validation_timeout=30)

    def ready(self, timeout=150):
        sock, greeting = connect(self.path / 'control.sock', self.process, self.started + timeout, self.overflow)
        sock.close()
        index = len(self.boots) + 1
        phase = 'open' if self.guest['setup_window'] else 'closed'
        require(greeting == f'READY 1 state={index} ro=1 landlock=6 policy=1 setup={phase}\n'.encode(), greeting)
        self.boots.append({'boot': index, 'greeting': greeting.decode().strip(),
            'initial_setup_phase': phase, 'guest_initramfs': self.guest['initramfs'],
            'ready_seconds': time.monotonic() - self.started})

    def admin(self, request, expected, name, timeout=65):
        sock, greeting = connect(self.path / 'control.sock', self.process, time.monotonic() + 5, self.overflow)
        with sock:
            sock.settimeout(timeout)
            sock.sendall(request.encode() + b'\n')
            reply = line(sock)
        (self.out / f'boot-{len(self.boots)}-{name}.json').write_text(json.dumps({
            'greeting': greeting.decode().strip(), 'request': request, 'reply': reply.decode().strip()}, indent=2) + '\n')
        require(reply == expected.encode() + b'\n', (name, reply))

    def baseline(self, name):
        result = self.execute(name, '''set -eu
test ! -e /usr/include/zlib.h
printf 'int main(void) { return 0; }\\n' > /workspace/control.c
gcc -O2 /workspace/control.c -o /workspace/control
/workspace/control
cp /toolchain/adaptable/probe.c /workspace/probe.c
if gcc -O2 -Wall -Werror /workspace/probe.c -lz -o /workspace/app > /workspace/missing.log 2>&1; then exit 1; fi
grep 'zlib.h' /workspace/missing.log
test ! -e /workspace/app
''')
        self.boots[-1]['dependency_absent_build_failure'] = {'positive_compiler_control': True,
            'missing_header': 'zlib.h', 'output': result.stdout.decode().strip()}

    def apply(self, recipe_id):
        denied = command(['ssh', *self.options(), 'devcroft@guest', 'printf unexpected'],
            self.out, f'boot-{len(self.boots)}-ssh-before-setup-close', timeout=10)
        require(denied.returncode != 0 and b'unexpected' not in denied.stdout, denied.stdout)
        self.admin('SETUP CLOSE', 'ERR setup unapplied', 'early-close-refused')
        wrong = ('0' if recipe_id[0] != '0' else '1') + recipe_id[1:]
        self.admin('SETUP APPLY ' + wrong, 'ERR setup recipe', 'wrong-recipe-refused')
        self.admin('SETUP APPLY ../../other', 'ERR setup recipe', 'path-injection-refused')
        self.admin('X' * 1024, 'ERR frame', 'oversized-frame-refused')
        self.admin('SETUP APPLY ' + recipe_id, 'OK setup applied', 'recipe-applied')
        self.admin('SETUP APPLY ' + recipe_id, 'ERR setup applied', 'duplicate-install-refused')
        self.admin('SETUP CLOSE', 'OK setup closed', 'setup-closed')
        self.admin('SETUP APPLY ' + recipe_id, 'ERR setup closed', 'reopen-refused')
        self.boots[-1]['setup'] = {'recipe_sha256': recipe_id, 'ssh_withheld_until_close': True,
            'wrong_recipe_path_injection_and_oversize_refused': True, 'duplicate_apply_refused': True,
            'closed_irreversibly_before_project_sessions': True}

    def verify(self, recipe_id, rebuild=True):
        script = 'set -eu\n'
        if rebuild:
            script += '''cp /toolchain/adaptable/probe.c /workspace/probe.c
gcc -O2 -Wall -Werror /workspace/probe.c -lz -o /workspace/app
'''
        script += '/workspace/app\nsha256sum /usr/include/zlib.h /usr/lib/libz.so.1 /workspace/probe.c /workspace/app\n'
        result = self.execute('native-build' if rebuild else 'verify-existing-build', script)
        require(result.stdout.startswith(b'zlib=1.4 roundtrip=ok bytes='), result.stdout)
        hashes = {}
        for row in result.stdout.decode().splitlines():
            parts = row.split()
            if len(parts) == 2 and len(parts[0]) == 64:
                hashes[parts[1]] = parts[0]
        require(len(hashes) == 4, result.stdout)
        marker = self.execute('recipe-marker', 'cat /private-base/rootfs/.recipe-applied').stdout
        require(marker == recipe_id.encode(), marker)
        self.execute('base-write-denials', '''set -eu
if (printf forbidden >> /usr/include/zlib.h) 2>/workspace/header-denied; then exit 1; fi
grep 'Permission denied' /workspace/header-denied
if /private-base/rootfs/sbin/apk --root /private-base/rootfs --no-network --no-scripts del zlib-dev > /workspace/apk-denied 2>&1; then exit 1; fi
grep -i 'permission denied' /workspace/apk-denied
test -w /workspace
''')
        unchanged = self.execute('header-still-unchanged', 'sha256sum /usr/include/zlib.h').stdout.decode().split()[0]
        require(unchanged == hashes['/usr/include/zlib.h'], 'denial probe changed the header')
        self.admin('SETUP APPLY ' + recipe_id, 'ERR setup closed', 'runtime-admin-refused')
        self.boots[-1]['native_dependency'] = {'stdout': result.stdout.decode().splitlines()[0],
            'sha256': hashes, 'rebuilt': rebuild, 'base_writes_and_package_removal_denied': True}
        return hashes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--kernel', type=Path, default=ROOT / 'artifacts/kernel-landlock/Image')
    parser.add_argument('--helper', type=Path, default=ROOT / 'libkrun/target/debug/devcroft-krun-spike')
    parser.add_argument('--inputs', type=Path, default=ROOT / 'artifacts/adaptable/toolchain.img')
    parser.add_argument('--seed', type=Path, default=ROOT / 'artifacts/adaptable/data.seed.img')
    parser.add_argument('--out', type=Path, default=ROOT / 'runs' / time.strftime('%Y%m%d-%H%M%S-adaptable'))
    args = parser.parse_args()
    os.umask(0o077)
    args.out.mkdir(parents=True, exist_ok=False)
    recipe = load(ROOT / 'fixtures/adaptable-recipe-aarch64.json')
    recipe_id = hashlib.sha256(encoded(recipe)).hexdigest()
    runtime = json.loads((ROOT / 'artifacts/stage-c-runtime/guest.json').read_text())
    setup = json.loads((ROOT / 'artifacts/stage-c-setup/guest.json').read_text())
    require(runtime['setup_recipe_sha256'] == setup['setup_recipe_sha256'] == recipe_id, 'guest/recipe mismatch')
    report = {'schema': 1, **helper_identity(args.helper), 'qualification': 'diagnostic-only', 'status': 'failed',
        'host': list(os.uname()), 'kernel_artifact': artifact(args.kernel), 'helper_artifact': artifact(args.helper),
        'recipe': recipe, 'recipe_sha256': recipe_id, 'inputs': artifact(args.inputs), 'seed': artifact(args.seed),
        'guests': {'runtime': runtime, 'setup': setup}, 'resources_per_vm': {'vcpus': 2, 'memory_mib': 512},
        'build': json.loads((args.inputs.parent / 'build.json').read_text()), 'instances': []}
    require(report['build']['recipe_sha256'] == recipe_id and encoded(report['build']['recipe']) == encoded(recipe),
            'build receipt does not match the selected recipe')
    require(report['inputs']['sha256'] == report['build']['toolchain.img']['sha256'], 'immutable input bundle mismatch')
    require(report['seed']['sha256'] == report['build']['data.seed.img']['sha256'], 'private seed mismatch')
    instances = []
    def new(label, guest):
        vm = AdaptableInstance(label, args, guest)
        instances.append(vm)
        report['instances'].append({'label': label, 'id': vm.id, 'directory': str(vm.path), 'boots': vm.boots})
        vm.prepare(validation_timeout=30)
        report['instances'][-1]['sandbox'] = vm.sandbox
        vm.start(); vm.ready()
        return vm
    try:
        local = new('local', runtime)
        local.baseline('before-install')
        local.shutdown()
        local.select(setup); local.start(); local.ready(); local.apply(recipe_id)
        expected = local.verify(recipe_id)
        local.execute('private-local-marker', 'printf %s ' + shlex.quote(local.id) + ' > /workspace/local-only')
        local.shutdown()
        local.select(runtime); local.start(); local.ready()
        require(local.verify(recipe_id, rebuild=False) == expected, 'local cold restart changed artifacts')
        require(local.execute('persistent-local-marker', 'cat /workspace/local-only').stdout == local.id.encode(), 'marker lost')
        local.shutdown()
        report['local_adaptation_survives_restart'] = True
        rebuilt = new('reconstructed', setup)
        rebuilt.apply(recipe_id)
        rebuilt.execute('fresh-private-workspace', 'set -eu; test ! -e /workspace/local-only; test ! -e /workspace/app')
        require(rebuilt.verify(recipe_id) == expected, 'fresh recipe reconstruction differs')
        rebuilt.shutdown()
        report['fresh_recipe_reconstruction_matches'] = True
        report['reconstruction_copied_local_disk'] = False
        omitted = new('omitted-recipe', runtime)
        omitted.baseline('omitted-dependency-negative-control')
        omitted.shutdown()
        report['omitted_recipe_reconstruction_rejected'] = True
        report['immutable_inputs_unchanged'] = artifact(args.inputs) == report['inputs']
        require(report['immutable_inputs_unchanged'], 'immutable inputs changed')
        report['status'] = 'passed'
    except Exception as error:
        report['error'] = f'{type(error).__name__}: {error}'
    finally:
        for vm in instances:
            vm.stop()
        report['cleanup'] = {'helpers_reaped': all(vm.process is None for vm in instances),
            'endpoints_removed': all(not (vm.path / name).exists() for vm in instances for name in SOCKETS)}
        if not all(report['cleanup'].values()):
            report['status'], report['error'] = 'failed', 'owned resources survived cleanup'
        (args.out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    print(f'Raw evidence: {args.out.resolve()}')
    raise SystemExit(0 if report['status'] == 'passed' else 1)


if __name__ == '__main__':
    main()
