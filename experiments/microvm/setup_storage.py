"""Scoped guest filesystem counters and sampled host private-file allocation."""
import re
import threading
import time

from density import usage

SPACE = re.compile(rb'guest: test-only setup storage point=([a-z-]+) '
                   rb'block_size=(\d+) blocks=(\d+) free=(\d+) available=(\d+)\r?\n')


def filesystem_space(block_size, blocks, free, available):
    if block_size <= 0 or blocks <= 0 or not 0 <= available <= free <= blocks:
        raise ValueError('invalid guest filesystem counters')
    return {'block_size': block_size, 'blocks': blocks, 'free_blocks': free,
            'available_blocks': available, 'capacity_bytes': block_size * blocks,
            'used_bytes': block_size * (blocks - free), 'free_bytes': block_size * free,
            'available_bytes': block_size * available,
            'method': 'guest filesystem-wide statvfs counters; not isolated staging-tree bytes or host allocation'}


def checkpoint_space(console, point):
    rows = [row for row in SPACE.findall(console) if row[0] == point.encode()]
    if len(rows) != 1:
        raise ValueError('missing or duplicate guest storage checkpoint: ' + point)
    return filesystem_space(*map(int, rows[0][1:]))


def runtime_space(output):
    fields = output.split()
    if len(fields) != 5 or fields[0] != b'setup-space':
        raise ValueError('invalid runtime filesystem observation')
    return filesystem_space(*map(int, fields[1:]))


class HostAllocationSampler:
    def __init__(self, paths, interval=0.02):
        if not 0.005 <= interval <= 1:
            raise ValueError('storage sampling interval outside 5..1000 ms')
        self.paths = list(paths)
        self.interval = interval
        self.started = time.monotonic()
        self.lock = threading.Lock()
        self.stopping = threading.Event()
        self.thread = None
        self.records = {'sample_count': 0, 'maximum_completed_sample_gap_seconds': 0,
                        'requested_interval_seconds': interval, 'errors': []}
        self.last_time = None

    def capture(self, label):
        record = usage(self.paths)
        with self.lock:
            now = time.monotonic()
            record.update(seconds=now - self.started, label=label)
            self.records['sample_count'] += 1
            if self.last_time is not None:
                self.records['maximum_completed_sample_gap_seconds'] = max(
                    self.records['maximum_completed_sample_gap_seconds'], now - self.last_time)
            self.last_time = now
            self.records.setdefault('first', record)
            self.records['last'] = record
            if record['allocated_bytes'] > self.records.get('observed_peak', {}).get('allocated_bytes', -1):
                self.records['observed_peak'] = record
        return record

    def start(self):
        self.capture('before-first-boot')

        def run():
            while not self.stopping.wait(self.interval):
                try:
                    self.capture('periodic')
                except Exception as error:
                    with self.lock:
                        self.records['errors'].append(f'{type(error).__name__}: {error}')
                    return

        self.thread = threading.Thread(target=run, daemon=True)
        self.thread.start()

    def finish(self):
        self.stopping.set()
        if self.thread is not None:
            self.thread.join(timeout=3)
            if self.thread.is_alive():
                raise RuntimeError('storage sampler did not stop')
        self.capture('before-owned-resource-removal')
        if self.records['errors']:
            raise RuntimeError('storage sampling failed: ' + repr(self.records['errors']))
        self.records['qualification'] = (
            'observed maximum of private image/state/controller stat allocations; sub-interval peaks, '
            'unique APFS extents, global preparation and compiler/evidence storage are not measured')
        return self.records
