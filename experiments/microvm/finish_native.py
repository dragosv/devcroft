#!/usr/bin/env python3
"""Finish the native evaluation after an existing density collector exits.

Serializes clean preparation and fault probes without competing with the density
workload. A failed phase is retained and does not erase independent evidence.
This controller never unlocks production runtime adoption.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--density-run', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    args.density_run = args.density_run.resolve()
    args.out = args.out.resolve()
    # Validate before creating a controller that would wait forever for a typo.
    density_path = args.density_run / 'report.json'
    density = json.loads(density_path.read_text())
    if density['platform'] != 'macos-arm64-native':
        raise RuntimeError('expected the native macOS ARM64 density row')
    args.out.mkdir(parents=True, exist_ok=False)
    report = {'schema': 1, 'status': 'running', 'qualification': 'serial native evaluation controller; production adoption remains gated',
              'raw_evidence': str(args.out), 'density_run': str(args.density_run),
              'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'phase': 'waiting-for-density', 'phases': [], 'published': []}
    child = None

    def save():
        target = args.out / 'report.json'
        temporary = args.out / 'report.json.tmp'
        temporary.write_text(json.dumps(report, indent=2) + '\n')
        temporary.replace(target)

    def interrupted(_signum, _frame):
        raise KeyboardInterrupt('native evaluation controller interrupted')

    signal.signal(signal.SIGTERM, interrupted)
    save()
    # runner.pid is reserved for density/calibration collectors by the shared
    # guard. This coordinator must not identify itself as an active collector.
    (args.out / 'controller.pid').write_text(str(os.getpid()) + '\n')
    (args.out / 'finish_native.py').write_bytes(Path(__file__).read_bytes())

    def phase(name, argv):
        nonlocal child
        report['phase'] = name
        record = {'name': name, 'argv': list(map(str, argv)), 'status': 'running',
                  'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat()}
        report['phases'].append(record)
        save()
        began = time.monotonic()
        with (args.out / f'{name}.stdout').open('wb') as stdout, (args.out / f'{name}.stderr').open('wb') as stderr:
            child = subprocess.Popen(record['argv'], stdout=stdout, stderr=stderr,
                                     start_new_session=True, close_fds=True)
            code = child.wait()
            child = None
        record.update(exit_code=code, status='passed' if code == 0 else 'failed', seconds=time.monotonic() - began)
        save()
        print(json.dumps(record), flush=True)
        return code == 0

    def publish(name, source):
        if source.exists():
            # Keep even failing reports; missing files are explicitly recorded.
            value = json.loads(source.read_text())
            target = ROOT / 'results' / name
            target.write_text(json.dumps(value, indent=2, allow_nan=False) + '\n')
            report['published'].append({'path': str(target), 'status': value['status'], 'source': str(source)})
        else:
            report['published'].append({'path': name, 'status': 'not-run', 'reason': f'no retained report at {source}'})
        save()

    try:
        while density['status'] == 'running':
            # No VM, materializer, compilation or image hashing in this window.
            # Read only the last atomically completed collector report. Its
            # writer currently replaces content in place, so retry partial JSON.
            time.sleep(15)
            try:
                density = json.loads(density_path.read_text())
            except json.JSONDecodeError:
                continue
        report['density_status'] = density['status']
        # Acquiring this lease observes the collector's final cleanup, including
        # its final report write. Release before children acquire the same lock.
        import fcntl
        with (ROOT / 'artifacts/nested-runner.lock').open('a') as lease:
            fcntl.flock(lease, fcntl.LOCK_EX)
        phase('density-summary', [sys.executable, ROOT / 'summarize_density.py', '--run', args.density_run,
                                  '--out', ROOT / 'results/macos-arm64-density.json'])
        clean = args.out / 'cold-preparation'
        phase('cold-preparation', [sys.executable, ROOT / 'cold_prepare.py', '--out', clean])
        publish('macos-arm64-cold-preparation.json', clean / 'report.json')
        for adapter in ('libkrun', 'hvi'):
            out = args.out / f'{adapter}-faults'
            phase(f'{adapter}-faults', [sys.executable, ROOT / 'fault_probe.py', '--adapter', adapter, '--out', out])
            publish(f'{adapter}-macos-arm64-faults.json', out / 'report.json')
        report['phase'] = 'finished'
        report['status'] = 'passed' if density['status'] == 'passed' and all(
            item['status'] == 'passed' for item in report['phases'] + report['published']) else 'failed'
    except BaseException as error:
        report.update(status='failed', error=f'{type(error).__name__}: {error}')
    finally:
        if child is not None and child.poll() is None:
            # Python probes catch KeyboardInterrupt and run their owned cleanup.
            os.killpg(child.pid, signal.SIGINT)
            try:
                child.wait(timeout=45)
            except subprocess.TimeoutExpired:
                report['cleanup_error'] = 'probe did not finish graceful cleanup; inspect its owned resources'
        report['finished_utc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save()
    print(json.dumps({'status': report['status'], 'phase': report['phase'], 'error': report.get('error')}), flush=True)
    raise SystemExit(report['status'] != 'passed')


if __name__ == '__main__':
    main()
