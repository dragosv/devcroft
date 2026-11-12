#!/usr/bin/env python3
"""Matched native macOS ARM64 launch/build/idle measurements with full raw costs."""
import argparse
import base64
import fcntl
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import time

from observe_macos import Observer
from smoke import ROOT, artifact, helper_identity
from ssh_smoke import require
from workload_smoke import WorkloadInstance


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def parallel(items, fn):
    with ThreadPoolExecutor(max_workers=len(items)) as pool:
        return list(pool.map(fn, items))


def usage(paths):
    seen, logical, physical = set(), 0, 0
    files = []
    for root in paths:
        for path in ([root] if root.is_file() else sorted(root.rglob('*'))):
            if not path.is_file() or path.is_symlink():
                continue
            stat = path.stat()
            identity = stat.st_dev, stat.st_ino
            if identity in seen:
                continue
            seen.add(identity)
            logical += stat.st_size
            physical += stat.st_blocks * 512
            files.append({'path': str(path), 'logical_bytes': stat.st_size, 'allocated_bytes': stat.st_blocks * 512})
    return {'logical_bytes': logical, 'allocated_bytes': physical, 'files': files,
            'method': 'stat size and blocks*512; unique inodes only; no claim of APFS shared-extent deduplication'}


class Cohort:
    def __init__(self, adapter, fixture, count, out):
        require(shutil.disk_usage('/private/tmp').free >= 2 * 1024**3, 'host storage reserve exhausted before cohort preparation')
        self.adapter, self.fixture, self.count, self.out = adapter, fixture, count, out
        out.mkdir(parents=True, exist_ok=False)
        self.helper = ROOT / ('hvi/target/debug/devcroft-hvi-spike' if adapter == 'hvi' else 'libkrun/target/debug/devcroft-krun-spike')
        directory = 'nix' if fixture == 'nix' else 'workload'
        guest_dir = 'stage-b-nix' if fixture == 'nix' else 'stage-b-workload'
        self.image = ROOT / 'artifacts' / directory / 'toolchain.img'
        self.seed = ROOT / 'artifacts' / directory / 'data.seed.img'
        self.required_host_headroom_bytes = 2 * 1024**3 + count * (512 * 1024**2 + self.seed.stat().st_size + 16 * 1024**2)
        self.host_free_bytes_before = shutil.disk_usage('/private/tmp').free
        require(self.host_free_bytes_before >= self.required_host_headroom_bytes,
                f'host headroom exhausted: free={self.host_free_bytes_before} required={self.required_host_headroom_bytes} instances={count}; reserve plus configured guest RAM and private disk capacity')
        self.kernel = ROOT / 'artifacts/kernel-landlock/Image'
        self.guest = json.loads((ROOT / 'artifacts' / guest_dir / 'guest.json').read_text())
        self.vms = []
        self.latest_sample_complete = False
        self.setup_samples = []
        started = time.monotonic()
        try:
            for index in range(count):
                setup_started = time.monotonic()
                vm = WorkloadInstance(f'vm-{index}', out, self.helper, self.kernel, self.guest, self.image, self.seed)
                self.vms.append(vm)
                vm.prepare(validation_timeout=90)
                self.setup_samples.append({'instance': vm.label, 'seconds': time.monotonic() - setup_started,
                                           'scope': 'private seed/bootstrap/test keys plus helper validation; no real credentials'})
        except BaseException:
            self.stop()
            raise
        self.preflight_seconds = time.monotonic() - started
        self.inputs = {'kernel': artifact(self.kernel), 'guest': self.guest, 'image': artifact(self.image),
                       'seed': artifact(self.seed), 'helper': artifact(self.helper), **helper_identity(self.helper),
                       'host_free_bytes_before': self.host_free_bytes_before,
                       'required_host_headroom_bytes': self.required_host_headroom_bytes,
                       'private_setup': self.setup_samples, 'total_preflight_seconds': self.preflight_seconds}
        write(out / 'inputs.json', self.inputs)

    def launch(self):
        started = time.monotonic()
        for vm in self.vms:
            require(vm.process is None, 'refusing to overwrite a live helper handle')
            vm.start()
        parallel(self.vms, lambda vm: vm.ready(timeout=120))
        ready = time.monotonic()

        def session(vm):
            begin = time.monotonic()
            response = vm.ssh('session-readiness', "printf 'SESSION_READY\\n'")
            require(response.returncode == 0 and response.stdout == b'SESSION_READY\n', response.stderr)
            return {'instance': vm.label, 'upper_bound_seconds': time.monotonic() - begin,
                    'kind': 'new pinned-key SSH guest command session; not AI-agent completion'}

        sessions = parallel(self.vms, session)
        return {'cohort_to_ready_seconds': ready - started, 'project_ready_seconds': time.monotonic() - started,
                'instances': [{'instance': vm.label, 'ready_seconds': vm.boots[-1]['ready_seconds']} for vm in self.vms],
                'sessions': sessions}

    def shutdown(self):
        parallel(self.vms, lambda vm: vm.shutdown())

    def stop(self):
        for vm in self.vms:
            vm.stop()

    def build(self):
        def build_one(vm):
            started = time.monotonic()
            vm.import_sources()
            imported = time.monotonic()
            hashes = vm.build(import_sources=False)
            built = time.monotonic()
            vm.start_redis()
            vm.write_acknowledged()
            vm.verify_values()
            expected = vm.acknowledged_values()
            return {'instance': vm.label, 'source_import_seconds': imported - started,
                    'build_and_verification_seconds': built - imported,
                    'service_and_durable_check_seconds': time.monotonic() - built,
                    'hashes': hashes, 'acknowledged_keys': len(expected)}

        return parallel(self.vms, build_one)

    def controlled(self):
        source = (ROOT / 'memory_stress.c').read_bytes()
        encoded = base64.b64encode(source).decode()
        prepare = ("set -eu; printf '%s' " + encoded + " | /bin/busybox base64 -d > /workspace/memory-stress.c; "
                   "cc -O2 -Wall -Wextra -Werror /workspace/memory-stress.c -o /workspace/memory-stress")
        parallel(self.vms, lambda vm: vm.execute('memory-prepare', prepare))
        return source

    def touch_free(self):
        def run(vm):
            result = vm.execute('memory-touch-free', '/workspace/memory-stress')
            require(result.stdout == b'TOUCHED 268435456\nFREED 268435456\n', result.stdout)
        parallel(self.vms, run)

    def cleanup(self, successful):
        self.stop()
        private_sizes = [usage([vm.path, vm.controller_dir]) for vm in self.vms]
        storage = usage([path for vm in self.vms for path in (vm.path, vm.controller_dir)])
        controller_storage = usage([vm.controller_dir for vm in self.vms])
        endpoints_gone = all(not (vm.path / name).exists() for vm in self.vms
                             for name in ('control.sock', 'keeper.sock', 'ssh.sock'))
        require(endpoints_gone, 'stale endpoint remains after cleanup')
        if successful:
            for vm in self.vms:
                require(vm.path.parent == Path('/private/tmp') and vm.path.name.startswith('ds-')
                        and not vm.path.is_symlink() and vm.path.stat().st_uid == os.getuid(), 'unexpected private directory')
                shutil.rmtree(vm.path)
                require(vm.controller_dir.parent == Path('/private/tmp') and vm.controller_dir.name.startswith('dc-c-')
                        and not vm.controller_dir.is_symlink() and vm.controller_dir.stat().st_uid == os.getuid(), 'unexpected controller directory')
                shutil.rmtree(vm.controller_dir)
        write(self.out / 'instances.json', [{'instance': vm.label, 'boots': vm.boots} for vm in self.vms])
        common = usage([self.helper, self.kernel, Path(self.guest['initramfs']['path']),
                        Path(self.guest['runtime']['path']), self.image, self.seed])
        return {'private_before_cleanup': storage, 'common_deployed_inputs': common,
                'controller_storage_included_in_private': controller_storage,
                'cohort_total_allocated_bytes': storage['allocated_bytes'] + common['allocated_bytes'],
                'per_environment_amortized_allocated_bytes': (storage['allocated_bytes'] + common['allocated_bytes']) / self.count,
                'standalone_per_environment_upper_bound_allocated_bytes': common['allocated_bytes'] + max((p['allocated_bytes'] for p in private_sizes), default=0),
                'successful_private_directories_removed': successful, 'failed_private_disks_retained': not successful,
                'all_helpers_reaped': all(vm.process is None for vm in self.vms), 'endpoints_removed': endpoints_gone}


def sample(cohort, out, kind, repetition, idle, controlled):
    out.mkdir(parents=True, exist_ok=False)
    # A constructor/interrupt failure before sample() returns must never make
    # the launch-cohort cleanup treat unrecorded work as successful.
    cohort.latest_sample_complete = False
    result = {'adapter': cohort.adapter, 'fixture': cohort.fixture, 'instances_requested': cohort.count,
              'kind': kind, 'repetition': repetition, 'status': 'failed', 'raw': str(out)}
    observer = Observer(out, cohort.vms)
    try:
        observer.phase = 'launch'
        result['launch'] = cohort.launch()
        if kind == 'build-idle':
            observer.phase = 'build'
            result['workload'] = cohort.build()
            result['post_build_idle'] = observer.observe_idle('post-build-idle', idle)
            if controlled:
                observer.phase = 'controlled-prepare'
                source = cohort.controlled()
                observer.phase = 'controlled-touch-free'
                cohort.touch_free()
                result['controlled_source_sha256'] = hashlib.sha256(source).hexdigest()
                result['controlled_idle'] = observer.observe_idle('controlled-idle', idle)
            observer.phase = 'service-stop'
            parallel(cohort.vms, lambda vm: vm.execute('redis-stop', 'redis-cli SHUTDOWN'))
        observer.phase = 'shutdown'
        cohort.shutdown()
        result['status'] = 'passed'
    except BaseException as error:
        result['error'] = f'{type(error).__name__}: {error}'
        result['interrupted'] = isinstance(error, KeyboardInterrupt)
        cohort.stop()
    finally:
        try:
            observer.stop()
        except Exception as error:
            result.update(status='failed', observer_error=f'{type(error).__name__}: {error}')
        result['observer_calibration'] = observer.calibration
        if observer.samples:
            result['sampled_peak_total_footprint_bytes'] = max(s['total_physical_footprint_bytes'] for s in observer.samples)
        write(out / 'sample.json', result)
    cohort.latest_sample_complete = result['status'] == 'passed'
    if result.get('interrupted'):
        raise KeyboardInterrupt(result['error'])
    print(json.dumps({k: result[k] for k in ('adapter', 'fixture', 'instances_requested', 'kind', 'repetition', 'status')}), flush=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--calibration', action='store_true', help='one instance/repetition with full idle windows; excluded from comparison')
    parser.add_argument('--fixture', choices=['alpine', 'nix', 'both'], default='both')
    parser.add_argument('--calibration-instances', type=int, choices=[1, 4, 8], default=1)
    args = parser.parse_args()
    builder_lease = (ROOT / 'artifacts/nested-runner.lock').open('a')
    fcntl.flock(builder_lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
    def interrupt(_signal, _frame):
        raise KeyboardInterrupt('evaluation interrupted')
    signal.signal(signal.SIGTERM, interrupt)
    require(platform.system() == 'Darwin' and platform.machine() == 'arm64', 'native macOS ARM64 only')
    os.umask(0o077)
    args.out.mkdir(parents=True, exist_ok=False)
    (args.out / 'runner.pid').write_text(str(os.getpid()) + '\n')
    source_names = ['density.py', 'observe_macos.py', 'measure_macos.c', 'memory_stress.c',
                    'smoke.py', 'ssh_smoke.py', 'ssh_transport.py', 'workload_smoke.py', 'keeper_probe.py']
    frozen = args.out / 'sources'
    frozen.mkdir()
    source_hashes = {}
    for name in source_names:
        content = (ROOT / name).read_bytes()
        (frozen / name).write_bytes(content)
        source_hashes[name] = hashlib.sha256(content).hexdigest()
    protocol = json.loads((ROOT / 'fixtures/evaluation-protocol.json').read_text())
    thresholds = artifact(ROOT / 'fixtures/no-go-thresholds.json')
    require(thresholds['sha256'] == protocol['thresholds']['sha256'], 'threshold record changed after protocol freeze')
    env = dict(os.environ, LIMA_HOME='/private/tmp/devcroft-kernel-builder')
    builders = subprocess.check_output(['limactl', 'list', '--json'], env=env, text=True).splitlines()
    require(all(json.loads(line)['status'] == 'Stopped' for line in builders if line), 'stop the private builder before density collection')
    report = {'schema': 1, 'qualification': 'calibration-only' if args.calibration else 'native functional density experiment',
              'platform': 'macos-arm64-native', 'host': list(os.uname()), 'protocol': protocol, 'thresholds': thresholds,
              'power': subprocess.check_output(['pmset', '-g', 'batt'], text=True),
              'hardware': subprocess.check_output(['sysctl', 'hw.memsize', 'hw.physicalcpu', 'hw.logicalcpu', 'machdep.cpu.brand_string'], text=True),
              'raw_evidence': str(args.out), 'harness_sources': source_hashes,
              'observer_artifact': artifact(ROOT / 'artifacts/measure-macos'),
              'samples': [], 'status': 'running'}
    write(args.out / 'report.json', report)
    require(args.calibration or args.calibration_instances == 1, 'instance override is calibration-only')
    counts, launches, builds, idle = ([args.calibration_instances], 1, 1, 60) if args.calibration else (protocol['instances'], 10, 5, 60)
    fixtures = ['alpine', 'nix'] if args.fixture == 'both' else [args.fixture]
    report['planned_samples'] = [dict(fixture=f, adapter=a, instances_requested=n, kind=k, repetition=r)
                                for f in fixtures for n in counts for a in protocol['adapters']
                                for k, total in [('launch', launches), ('build-idle', builds)] for r in range(total)]
    report['agent_start_criterion'] = 'not evaluated by SSH marker; real agent CLI probe remains separate'
    awake = subprocess.Popen(['/usr/bin/caffeinate', '-i', '-w', str(os.getpid())])
    try:
        for fixture in fixtures:
            for count in counts:
                cohorts = {}
                try:
                    for adapter in protocol['adapters']:
                        cohorts[adapter] = Cohort(adapter, fixture, count, args.out / f'{fixture}-{count}-{adapter}-launch-cohort')
                    for repetition in range(launches):
                        order = protocol['adapters'] if repetition % 2 == 0 else list(reversed(protocol['adapters']))
                        for adapter in order:
                            cohort = cohorts[adapter]
                            result = sample(cohort, args.out / f'{fixture}-{count}-{adapter}-launch-{repetition}', 'launch', repetition, idle, False)
                            report['samples'].append(result)
                            write(args.out / 'report.json', report)
                finally:
                    for cohort in cohorts.values():
                        write(cohort.out / 'storage-cleanup.json', cohort.cleanup(successful=cohort.latest_sample_complete and all(
                            s['status'] == 'passed' for s in report['samples'] if s['adapter'] == cohort.adapter and s['fixture'] == fixture and s['instances_requested'] == count)))
                for repetition in range(builds):
                    order = protocol['adapters'] if repetition % 2 == 0 else list(reversed(protocol['adapters']))
                    for adapter in order:
                        cohort = Cohort(adapter, fixture, count, args.out / f'{fixture}-{count}-{adapter}-build-cohort-{repetition}')
                        result = None
                        try:
                            result = sample(cohort, args.out / f'{fixture}-{count}-{adapter}-build-{repetition}', 'build-idle', repetition, idle, repetition == 0)
                            report['samples'].append(result)
                        finally:
                            cleanup = cohort.cleanup(successful=result is not None and result['status'] == 'passed')
                            if result is not None:
                                result['storage_cleanup'] = cleanup
                            write(args.out / 'report.json', report)
        report['status'] = 'passed' if report['samples'] and all(s['status'] == 'passed' for s in report['samples']) else 'failed'
    except BaseException as error:
        report.update(status='failed', error=f'{type(error).__name__}: {error}')
    finally:
        awake.terminate()
        awake.wait(timeout=3)
        recorded = {s['raw'] for s in report['samples']}
        for path in sorted(args.out.glob('*/sample.json')):
            saved = json.loads(path.read_text())
            if saved['raw'] not in recorded:
                report['samples'].append(saved)
        done = {(s['fixture'], s['adapter'], s['instances_requested'], s['kind'], s['repetition']) for s in report['samples']}
        report['not_run_samples'] = [s for s in report['planned_samples'] if
                                    (s['fixture'], s['adapter'], s['instances_requested'], s['kind'], s['repetition']) not in done]
        write(args.out / 'report.json', report)
    raise SystemExit(report['status'] != 'passed')


if __name__ == '__main__':
    main()
