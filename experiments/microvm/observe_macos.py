"""Sample the entire owned process tree, keeping footprint/RSS/CPU distinct."""
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parent


class Observer:
    def __init__(self, out, instances, interval=0.25):
        if interval != 0.25:
            raise ValueError('native observer implements the frozen 250-ms cadence')
        self.out, self.instances, self.interval = out, instances, interval
        self.error = None
        self.phases = []
        self._phase = 'prepare'
        self.phase = 'prepare'
        self.raw = out / 'memory-cpu.native.jsonl'
        self.output = self.raw.open('wb')
        self.stderr = (out / 'observer.stderr').open('wb')
        self.process = subprocess.Popen([str(ROOT / 'artifacts/measure-macos'), 'watch', str(os.getpid())],
                                        stdin=subprocess.DEVNULL, stdout=self.output, stderr=self.stderr,
                                        start_new_session=True)
        deadline = time.monotonic() + 5
        while True:
            with self.raw.open() as stream:
                line = stream.readline()
            if line.endswith('\n'):
                self.calibration = json.loads(line)
                break
            if self.process.poll() is not None or time.monotonic() >= deadline:
                self.stop()
                raise RuntimeError('native observer did not calibrate')
            time.sleep(0.005)
        if (self.calibration.get('event') != 'calibration' or not self.calibration.get('autonomous')
                or abs(self.calibration['monotonic_ns'] / 1e9 - time.monotonic()) > 1):
            self.stop()
            raise ValueError('native observer clock/calibration mismatch')

    @property
    def phase(self):
        return self._phase

    @phase.setter
    def phase(self, value):
        self._phase = value
        self.phases.append({'at': time.monotonic(), 'phase': value,
                            'helper_pids': [vm.process.pid for vm in self.instances if vm.process is not None]})
        (self.out / 'observer-phases.json').write_text(json.dumps(self.phases, indent=2) + '\n')

    @property
    def samples(self):
        samples = []
        with self.raw.open() as stream:
            for line in stream:
                # An in-flight final line is retried on the next read. The
                # native writer flushes each complete snapshot independently.
                if not line.endswith('\n'):
                    break
                data = json.loads(line)
                if data.get('event') == 'calibration':
                    continue
                data['at'] = data['native_monotonic_ns'] / 1e9
                data['wall_at'] = data['wall_time_ns'] / 1e9
                phase = self.phases[0]
                for marker in self.phases:
                    if marker['at'] > data['at']:
                        break
                    phase = marker
                data.update(phase=phase['phase'], helper_pids=phase['helper_pids'])
                records = [*data['processes'], data['observer']]
                data['total_physical_footprint_bytes'] = sum(p.get('physical_footprint_bytes', 0) for p in records)
                data['total_resident_bytes'] = sum(p.get('resident_bytes', 0) for p in records)
                samples.append(data)
        return samples

    def observe_idle(self, label, seconds):
        self.phase = label
        started = time.monotonic()
        deadline = started + seconds
        # Short interruptible waits preserve bounded supervision of live VMs.
        while time.monotonic() < deadline:
            if self.error:
                raise RuntimeError(self.error)
            for vm in self.instances:
                if vm.process is None or vm.process.poll() is not None:
                    raise RuntimeError('a required instance died during idle observation')
            time.sleep(min(0.25, max(0, deadline - time.monotonic())))
        samples = [sample for sample in self.samples if sample['phase'] == label and started <= sample['at'] <= deadline]
        if len(samples) < max(1, int(seconds / self.interval) - 3):
            raise RuntimeError(f'idle observation has missing samples: {len(samples)} in {seconds} seconds')
        expected = {vm.process.pid for vm in self.instances}
        for sample in samples:
            present = {p['pid'] for p in sample['processes'] if 'error' not in p}
            if not expected <= present:
                raise RuntimeError('observer missed a required live helper')
        first, last = samples[0], samples[-1]
        cpu = []
        for pid in sorted(expected):
            a = next(p for p in first['processes'] if p['pid'] == pid)
            b = next(p for p in last['processes'] if p['pid'] == pid)
            if a['start_abstime'] != b['start_abstime']:
                raise RuntimeError('PID identity changed during idle observation')
            cpu.append({'pid': pid, 'cpu_seconds': (b['user_cpu_ns'] + b['system_cpu_ns'] -
                                                  a['user_cpu_ns'] - a['system_cpu_ns']) / 1e9})
        span = last['at'] - first['at']
        first_all = {p['pid']: p for p in [*first['processes'], first['observer']] if 'error' not in p}
        last_all = {p['pid']: p for p in [*last['processes'], last['observer']] if 'error' not in p}
        stable = set(first_all) & set(last_all)
        all_cpu = sum(last_all[pid]['user_cpu_ns'] + last_all[pid]['system_cpu_ns'] -
                      first_all[pid]['user_cpu_ns'] - first_all[pid]['system_cpu_ns']
                      for pid in stable if first_all[pid]['start_abstime'] == last_all[pid]['start_abstime']) / 1e9
        maximum_per_environment = 0
        for sample in samples:
            helpers = [p for p in sample['processes'] if p['pid'] in expected]
            helper_total = sum(p['physical_footprint_bytes'] for p in helpers)
            overhead_share = (sample['total_physical_footprint_bytes'] - helper_total) / len(expected)
            maximum_per_environment = max(maximum_per_environment,
                                          max(p['physical_footprint_bytes'] for p in helpers) + overhead_share)
        return {'window_seconds': seconds, 'sample_span_seconds': span, 'samples': len(samples),
                'maximum_total_footprint_bytes': max(s['total_physical_footprint_bytes'] for s in samples),
                'end_total_footprint_bytes': last['total_physical_footprint_bytes'],
                'end_total_resident_bytes': last['total_resident_bytes'],
                'maximum_per_environment_footprint_bytes_with_shared_observer_cost': maximum_per_environment,
                'end_processes': last['processes'], 'observer_overhead': last['observer'],
                'helper_cpu': cpu, 'helper_cpu_percent_one_core': 100 * sum(p['cpu_seconds'] for p in cpu) / span if span else None,
                'all_owned_cpu_percent_one_core': 100 * all_cpu / span if span else None,
                'cpu_process_set_stable': set(first_all) == set(last_all)}

    def stop(self):
        if self.process.poll() is None:
            self.process.terminate()
        self.process.wait(timeout=3)
        self.output.close()
        self.stderr.close()
        if self.process.returncode:
            raise RuntimeError('native observer did not stop cleanly')
        # Preserve the familiar enriched JSONL in addition to native snapshots
        # and the independent phase markers, so every timestamp can be audited.
        with (self.out / 'memory-cpu.jsonl').open('w') as stream:
            for sample in self.samples:
                stream.write(json.dumps(sample, separators=(',', ':')) + '\n')
