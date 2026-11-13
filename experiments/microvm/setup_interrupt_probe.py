#!/usr/bin/env python3
"""Interrupt private setup at fixed points and classify what sessions then see.

A test-only setup fixture stalls PID 1 once per private disk at each named
point. The host kills the helper there (guest power loss; host page cache
survives, so this is not host power-loss evidence), then boots the ordinary
runtime guest and classifies the fixed package/header/marker signature as
`baseline`, `applied`, or `partial`. Atomic setup must never expose `partial`, and an
interrupted baseline copy must not leave the private disk unbootable.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import threading
import time
import traceback

from adaptable.recipe import encoded, load
from adaptable_smoke import AdaptableInstance
from cold_prepare import require_no_active_collector
from smoke import helper_identity, ROOT, artifact, connect, LOG_LIMIT
from ssh_smoke import require, SOCKETS, ENV
from setup_storage import HostAllocationSampler, checkpoint_space, runtime_space


def live_drain(stream, data, overflow):
    while chunk := stream.read1(4096):
        room = LOG_LIMIT - len(data)
        data.extend(chunk[:room])
        if len(chunk) > room:
            overflow.set()

STALL = re.compile(rb'guest: test-only setup stall point=([a-z-]+)\r?\n')
DISCARD = re.compile(rb'guest: discarded interrupted (\S+)\r?\n')
CLASSIFY = '''for f in /usr/include/zlib.h /private-base/rootfs/.recipe-applied; do
  if test -e "$f"; then echo "present $f"; else echo "absent $f"; fi
done
if test -r /lib/apk/db/installed; then
  echo "zlib-dev-db $(grep -c '^P:zlib-dev$' /lib/apk/db/installed || true)"
else echo "zlib-dev-db missing"; fi
if test -e /private-base/rootfs/.recipe-applied; then echo "marker $(cat /private-base/rootfs/.recipe-applied)"; fi
'''


def classify_output(data, recipe_id):
    lines = data.decode().splitlines()
    rows = dict(line.split(' ', 1) for line in lines)
    header = 'present /usr/include/zlib.h' in lines
    marker = 'present /private-base/rootfs/.recipe-applied' in lines
    if (not header and not marker and rows.get('zlib-dev-db') == '0'
            and 'absent /usr/include/zlib.h' in lines
            and 'absent /private-base/rootfs/.recipe-applied' in lines):
        return 'baseline'
    if header and marker and rows.get('zlib-dev-db') == '1' and rows.get('marker') == recipe_id:
        return 'applied'
    return 'partial'


def validate_guests(guests, recipe_id, mode):
    runtime, setup, fault = (guests[name] for name in ('runtime', 'setup', 'fault'))
    require(all(g.get('setup_recipe_sha256') == recipe_id and g.get('adaptable') is True
                and g.get('arch') == 'aarch64' for g in guests.values()), 'guest/recipe mismatch')
    require(runtime.get('setup_window') is False and setup.get('setup_window') is True
            and fault.get('setup_window') is True and fault.get('setup_fault_probe') is True
            and setup.get('setup_fault_probe', False) is False
            and runtime.get('setup_fault_probe', False) is False, 'guest roles do not match fixture receipts')
    for key in ('keeper_sha256', 'busybox_sha256'):
        require(runtime.get(key) and all(g.get(key) == runtime[key] for g in guests.values()),
                'guest payload mismatch: ' + key)
    require(len({g['runtime']['sha256'] for g in guests.values()}) == 1, 'runtime disk mismatch')
    require(len({g['initramfs']['sha256'] for g in guests.values()}) == 3, 'guest roles require distinct initramfs')
    if mode == 'atomic':
        require(runtime.get('source_sha256') and len({g.get('source_sha256') for g in guests.values()}) == 1,
                'atomic guest init sources differ')


def validate_inputs(recipe, build, inputs, seed):
    require(build.get('recipe_sha256') == hashlib.sha256(encoded(recipe)).hexdigest()
            and encoded(build['recipe']) == encoded(recipe), 'build receipt does not match the selected recipe')
    require(inputs['sha256'] == build['toolchain.img']['sha256'], 'immutable input bundle mismatch')
    require(seed['sha256'] == build['data.seed.img']['sha256'], 'private seed mismatch')


class InterruptInstance(AdaptableInstance):
    def storage_state(self, name):
        output = self.execute(name, "/bin/busybox stat -f -c 'setup-space %S %b %f %a' /private-base").stdout
        return {'guest': runtime_space(output), 'host': self.storage_sampler.capture(name)}

    def begin(self, kind, guest):
        if self.guest is not guest:
            self.select(guest)
        self.boots.append({'boot': len(self.boots) + 1, 'guest': kind,
                           'guest_initramfs': guest['initramfs']})
        self.start()

    def ready(self, timeout=150):
        sock, greeting = connect(self.path / 'control.sock', self.process, self.started + timeout, self.overflow)
        sock.close()
        self.boots[-1].update(greeting=greeting.decode().strip(),
                              ready_seconds=time.monotonic() - self.started)
        return greeting

    def start(self):
        # The shared drain uses read(4096), which blocks until 4 KiB or EOF;
        # a stall line must be visible while the helper is still alive.
        self.stdout, self.stderr, self.overflow = bytearray(), bytearray(), threading.Event()
        self.started = time.monotonic()
        self.process = subprocess.Popen([str(self.helper), 'run', str(self.launch_path)],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            env=ENV, close_fds=True, start_new_session=True)
        self.threads = [threading.Thread(target=live_drain, args=(stream, buf, self.overflow), daemon=True)
            for stream, buf in [(self.process.stdout, self.stdout), (self.process.stderr, self.stderr)]]
        for thread in self.threads:
            thread.start()

    def console(self):
        return bytes(self.stdout) + bytes(self.stderr)

    def kill_at(self, point, apply_recipe=None, timeout=150):
        """Wait for the named stall, then SIGKILL the helper's process group."""
        if apply_recipe:
            self.ready()
            sock, _ = connect(self.path / 'control.sock', self.process, time.monotonic() + 5, self.overflow)
            sock.sendall(f'SETUP APPLY {apply_recipe}\n'.encode())
        deadline = time.monotonic() + timeout
        try:
            while time.monotonic() < deadline:
                found = STALL.findall(self.console())
                if found:
                    require(found == [point.encode()], ('unexpected stall points', found))
                    break
                require(self.process.poll() is None, f'helper exited before stall {point}: {self.process.returncode}')
                time.sleep(0.05)
            else:
                raise TimeoutError(f'stall {point} not observed')
            if getattr(self, 'storage_sampler', None):
                self.boots[-1]['storage_at_stall'] = {
                    'guest': checkpoint_space(self.console(), point),
                    'host': self.storage_sampler.capture('stall-' + point)}
            os.killpg(self.process.pid, signal.SIGKILL)
            code = self.process.wait(timeout=10)
        finally:
            if apply_recipe:
                sock.close()
        self.stop()
        self.boots[-1].update(stall_point=point, helper_signal='SIGKILL', exit_code=code)
        require(code == -signal.SIGKILL, f'helper did not die from SIGKILL: {code}')

    def observe(self, runtime):
        """Boot the ordinary runtime guest; classify its base or record its failure."""
        self.begin('runtime', runtime)
        record = self.boots[-1]
        try:
            self.ready()
        except RuntimeError as error:
            self.process.wait(timeout=10)
            self.stop()
            console = self.console().decode(errors='replace')
            record.update(reached_ready=False, error=str(error),
                          guest_failure=[l for l in console.splitlines() if l.startswith('guest: ')][-3:])
            return 'unbootable'
        record['reached_ready'] = True
        result = self.execute('classify', CLASSIFY)
        lines = result.stdout.decode().splitlines()
        state = classify_output(result.stdout, self.recipe_id)
        record.update(classification=state, observed=lines)
        if getattr(self, 'storage_sampler', None):
            record['storage_after_recovery'] = self.storage_state('runtime-after-recovery')
        self.shutdown()
        record['discarded_on_boot'] = [d.decode() for d in DISCARD.findall(self.console())]
        return state


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=['before', 'atomic'], required=True)
    parser.add_argument('--kernel', type=Path, default=ROOT / 'artifacts/kernel-landlock/Image')
    parser.add_argument('--helper', type=Path, default=ROOT / 'libkrun/target/debug/devcroft-krun-spike')
    parser.add_argument('--inputs', type=Path, default=ROOT / 'artifacts/adaptable/toolchain.img')
    parser.add_argument('--seed', type=Path, default=ROOT / 'artifacts/adaptable/data.seed.img')
    parser.add_argument('--runtime-guest', type=Path, required=True)
    parser.add_argument('--setup-guest', type=Path, required=True)
    parser.add_argument('--fault-guest', type=Path, required=True)
    parser.add_argument('--measure-storage', action='store_true',
                        help='sample host allocation and fixed guest filesystem checkpoints (atomic mode only)')
    parser.add_argument('--out', type=Path, default=ROOT / 'runs' / time.strftime('%Y%m%d-%H%M%S-setup-interrupt'))
    args = parser.parse_args()
    if args.measure_storage and args.mode != 'atomic':
        parser.error('--measure-storage requires --mode atomic and the instrumented fault guest')
    require_no_active_collector(ROOT / 'runs')
    lease = (ROOT / 'artifacts/nested-runner.lock').open('a')
    fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
    os.umask(0o077)
    args.out.mkdir(parents=True, exist_ok=False)
    recipe = load(ROOT / 'fixtures/adaptable-recipe-aarch64.json')
    recipe_id = hashlib.sha256(encoded(recipe)).hexdigest()
    guests = {name: json.loads((path / 'guest.json').read_text())
              for name, path in [('runtime', args.runtime_guest), ('setup', args.setup_guest), ('fault', args.fault_guest)]}
    runtime, setup, fault = guests['runtime'], guests['setup'], guests['fault']
    validate_guests(guests, recipe_id, args.mode)
    if args.measure_storage and (type(fault.get('setup_storage_schema')) is not int
                                 or fault['setup_storage_schema'] != 1):
        parser.error('--measure-storage requires a fault guest with setup_storage_schema=1')
    build = json.loads((args.inputs.parent / 'build.json').read_text())
    inputs, seed = artifact(args.inputs), artifact(args.seed)
    validate_inputs(recipe, build, inputs, seed)
    sources = ['setup_interrupt_probe.py', 'adaptable_smoke.py', 'guest/init.c', 'build_guest.py',
               'build_keeper_guest.py', 'smoke.py', 'ssh_smoke.py', 'ssh_transport.py', 'workload_smoke.py',
               'cold_prepare.py', 'adaptable/recipe.py']
    sources.extend(['setup_storage.py', 'density.py'])
    for name in sources:
        target = args.out / 'sources' / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes((ROOT / name).read_bytes())
    report = {'schema': 1, **helper_identity(args.helper), 'qualification': 'diagnostic-only', 'mode': args.mode,
        'status': 'failed', 'host': list(os.uname()), 'kernel_artifact': artifact(args.kernel),
        'helper_artifact': artifact(args.helper), 'recipe_sha256': recipe_id, 'inputs': inputs,
        'seed': seed, 'build': build, 'guests': guests, 'raw_evidence': str(args.out),
        'classification_scope': 'fixed zlib header, apk database entry and recipe marker signature; final four artifact hashes compared to a fresh uninterrupted reconstruction, not a full-tree equivalence proof',
        'probe_sources': {s: hashlib.sha256((ROOT / s).read_bytes()).hexdigest() for s in sources},
        'interruption': 'helper SIGKILL at a test-only PID 1 stall; host page cache survives, so this is guest '
                        'power loss, not host power loss', 'cases': []}
    instances = []

    def save():
        (args.out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')

    def new(label):
        vm = InterruptInstance(label, args, runtime)
        vm.recipe_id = recipe_id
        instances.append(vm)
        vm.prepare(validation_timeout=30)
        case = {'label': label, 'id': vm.id, 'directory': str(vm.path), 'boots': vm.boots}
        report['cases'].append(case)
        if args.measure_storage:
            vm.storage_sampler = HostAllocationSampler([vm.path, vm.controller_dir])
            vm.storage_sampler.start()
        return vm, case

    try:
        if args.mode == 'before':
            # A: interrupt the first baseline copy, then try to boot anything.
            a, case = new('baseline-copy')
            a.begin('fault-setup', fault); a.kill_at('baseline-copy')
            case['runtime_after'] = a.observe(runtime)
            a.begin('setup', setup)
            try:
                a.ready(); case['setup_after'] = 'ready'
            except RuntimeError as error:
                a.process.wait(timeout=10); a.stop(); case['setup_after'] = f'unbootable: {error}'
            # B: interrupt after the package manager wrote the private base, before the marker.
            b, case = new('recipe-installed')
            case['runtime_before'] = b.observe(runtime)
            b.begin('fault-setup', fault); b.kill_at('recipe-installed', apply_recipe=recipe_id)
            case['runtime_after'] = b.observe(runtime)
            report['atomic'] = (report['cases'][0]['runtime_after'] != 'unbootable'
                                and case['runtime_after'] in ('baseline', 'applied'))
            report['status'] = 'passed'
        else:
            points = ['baseline-copy', 'recipe-staged', 'recipe-installed', 'recipe-marked', 'recipe-exchanged']
            c, case = new('sequential-interruptions')
            case['points'] = []
            for point in points:
                c.begin('fault-setup', fault)
                c.kill_at(point, apply_recipe=None if point == 'baseline-copy' else recipe_id)
                state = c.observe(runtime)
                discarded = c.boots[-1].get('discarded_on_boot', [])
                expected = 'applied' if point == 'recipe-exchanged' else 'baseline'
                case['points'].append({'point': point, 'runtime_classification': state,
                                       'expected': expected, 'discarded_on_boot': discarded})
                save()
                require(state == expected, (point, state))
            # The fault fixture now passes every point: setup is already applied and only closes.
            c.begin('fault-setup', fault); c.ready()
            c.admin('SETUP APPLY ' + recipe_id, 'ERR setup applied', 'reapply-after-recovery-refused')
            c.admin('SETUP CLOSE', 'OK setup closed', 'setup-closed-after-recovery')
            c.shutdown()
            c.begin('runtime', runtime); c.ready()
            recovered = c.verify(recipe_id)
            if args.measure_storage:
                c.boots[-1]['storage_final'] = c.storage_state('recovered-final-storage')
            c.shutdown()
            # An uninterrupted fresh reconstruction on its own disk is the reference.
            d, ref = new('uninterrupted-reference')
            d.begin('setup', setup); d.ready(); d.apply(recipe_id)
            expected = d.verify(recipe_id)
            if args.measure_storage:
                d.boots[-1]['storage_final'] = d.storage_state('uninterrupted-final-storage')
            d.shutdown()
            if args.measure_storage:
                report['storage_measurement_scope'] = (
                    '20-ms requested host private-allocation sampling and guest statvfs at synchronized stalls '
                    'and ordinary recovery; observed maxima, not a full 8-GiB preparation/storage gate')
            report['recovered_artifacts_match_uninterrupted'] = recovered == expected
            report['artifact_sha256'] = expected
            require(recovered == expected, 'recovered reconstruction differs from the uninterrupted reference')
            report['atomic'] = True
            report['immutable_inputs_unchanged'] = artifact(args.inputs) == report['inputs']
            require(report['immutable_inputs_unchanged'], 'immutable inputs changed')
            report['status'] = 'passed'
    except Exception as error:
        report['error'] = f'{type(error).__name__}: {error}'
        report['traceback'] = traceback.format_exc()
    finally:
        for vm in instances:
            vm.stop()
            sampler = getattr(vm, 'storage_sampler', None)
            if sampler:
                try:
                    record = sampler.finish()
                    next(c for c in report['cases'] if c['id'] == vm.id)['host_storage_samples'] = record
                except Exception as error:
                    report.update(status='failed', storage_error=f'{type(error).__name__}: {error}')
        report['cleanup'] = {'helpers_reaped': all(vm.process is None for vm in instances),
            'endpoints_removed': all(not (vm.path / name).exists() for vm in instances for name in SOCKETS)}
        if not all(report['cleanup'].values()):
            report['status'], report['error'] = 'failed', 'owned resources survived cleanup'
        # Instance disks are up to ~1 GiB each and hold no evidence (logs live under --out).
        for vm in instances:
            shutil.rmtree(vm.path, ignore_errors=True)
            shutil.rmtree(vm.controller_dir, ignore_errors=True)
        report['cleanup']['instance_disks_removed'] = all(not vm.path.exists() for vm in instances)
        report['cleanup']['controller_dirs_removed'] = all(not vm.controller_dir.exists() for vm in instances)
        if not all(report['cleanup'].values()):
            report['status'], report['error'] = 'failed', 'owned resources survived cleanup'
        save()
    print(json.dumps({k: report.get(k) for k in ('status', 'error', 'atomic', 'mode')}, indent=2))
    print(json.dumps([{k: v for k, v in c.items() if k != 'boots'} for c in report['cases']], indent=2))
    print(f'Raw evidence: {args.out.resolve()}')
    raise SystemExit(0 if report['status'] == 'passed' else 1)


if __name__ == '__main__':
    main()
