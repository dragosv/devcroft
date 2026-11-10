#!/usr/bin/env python3
"""Prove sampling continues while only its owned Python controller is stopped."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

from observe_macos import Observer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--controller', action='store_true')
    args = parser.parse_args()
    if args.controller:
        observer = Observer(args.out, [])
        try:
            observer.phase = 'controller-pause-probe'
            (args.out / 'ready').write_text(str(os.getpid()))
            deadline = time.monotonic() + 10
            while not (args.out / 'finish').exists() and time.monotonic() < deadline:
                time.sleep(0.02)
        finally:
            observer.stop()
        return
    args.out.mkdir(parents=True, exist_ok=False)
    process = subprocess.Popen([sys.executable, __file__, '--controller', '--out', str(args.out)],
                               start_new_session=True)
    paused = False
    try:
        deadline = time.monotonic() + 5
        while not (args.out / 'ready').exists():
            if process.poll() is not None or time.monotonic() >= deadline:
                raise RuntimeError('owned controller did not initialize')
            time.sleep(0.01)
        os.killpg(process.pid, signal.SIGSTOP)
        paused = True
        began = time.monotonic()
        time.sleep(2)
        ended = time.monotonic()
        os.killpg(process.pid, signal.SIGCONT)
        paused = False
        (args.out / 'finish').touch()
        if process.wait(timeout=5):
            raise RuntimeError('owned controller failed cleanup')
        samples = [json.loads(line) for line in (args.out / 'memory-cpu.jsonl').read_text().splitlines()]
        during = [sample for sample in samples if began + 0.1 <= sample['at'] <= ended - 0.1]
        gaps = [b['at'] - a['at'] for a, b in zip(during, during[1:])]
        passed = len(during) >= 7 and gaps and max(gaps) < 0.3
        result = {'schema': 1, 'status': 'passed' if passed else 'failed',
                  'qualification': 'observer timing control only; not VM or host suspend/resume',
                  'platform': 'macos-arm64-native', 'raw_evidence': str(args.out),
                  'controller_sigstop_seconds': ended - began, 'samples_while_controller_stopped': len(during),
                  'maximum_gap_seconds': max(gaps) if gaps else None,
                  'observer_independent_session': True, 'controller_reaped': True}
        result['stopped_scope'] = 'only the freshly created controller process group; native observer is in its own session'
        (args.out / 'report.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps(result))
        if not passed:
            raise RuntimeError('native collector depended on controller wakeups')
    finally:
        if paused:
            os.killpg(process.pid, signal.SIGCONT)
        if process.poll() is None:
            process.kill()
            process.wait(timeout=3)


if __name__ == '__main__':
    main()
