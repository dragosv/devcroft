#!/usr/bin/env python3
"""Inventory retained experiment/preparation storage outside deployed VM disks."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import stat

ROOT = Path(__file__).resolve().parent
GIB = 1024**3


def inventory(path):
    seen = set()
    result = {'path': str(path), 'regular_files': 0, 'logical_bytes': 0,
              'allocated_bytes': 0, 'symlinks_not_followed': 0, 'errors': []}

    def visit(current):
        try:
            info = current.lstat()
            if stat.S_ISLNK(info.st_mode):
                result['symlinks_not_followed'] += 1
            elif stat.S_ISDIR(info.st_mode):
                with os.scandir(current) as entries:
                    for entry in entries:
                        visit(Path(entry.path))
            elif stat.S_ISREG(info.st_mode) and (info.st_dev, info.st_ino) not in seen:
                seen.add((info.st_dev, info.st_ino))
                result['regular_files'] += 1
                result['logical_bytes'] += info.st_size
                result['allocated_bytes'] += info.st_blocks * 512
        except OSError as error:
            result['errors'].append({'path': str(current), 'error': str(error)})

    visit(path)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--builder-disk', type=Path,
                        default=Path('/private/tmp/devcroft-kernel-builder/kernel-build/disk'))
    args = parser.parse_args()
    thresholds_path = ROOT / 'fixtures/no-go-thresholds.json'
    thresholds = json.loads(thresholds_path.read_text())
    buckets = {'stopped_builder_disk': inventory(args.builder_disk),
               'retained_raw_evidence': inventory(ROOT / 'runs'),
               'libkrun_compilation_cache': inventory(ROOT / 'libkrun/target'),
               'hvi_compilation_cache': inventory(ROOT / 'hvi/target'),
               'guest_keeper_compilation_cache': inventory(ROOT / 'guest-keeper/target')}
    limit = thresholds['original_no_go_thresholds']['physical_storage_per_environment_gib']
    builder = buckets['stopped_builder_disk']
    result = {'schema': 1, 'status': 'failed' if any(x['errors'] for x in buckets.values()) else 'passed',
              'qualification': 'retained global experiment storage inventory; not a full per-environment storage pass',
              'platform': 'macos-arm64-native', 'measured_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'buckets': buckets, 'thresholds_sha256': hashlib.sha256(thresholds_path.read_bytes()).hexdigest(),
              'original_storage_limit_gib': limit,
              'conservative_single_environment_builder_charge': {
                  'allocated_gib': builder['allocated_bytes'] / GIB,
                  'criterion': 'not-run' if builder['errors'] else 'failed' if builder['allocated_bytes'] > limit * GIB else 'not-run',
                  'scope': 'charge the existing dedicated kernel/materializer builder to one environment; excludes runtime, archives, compiler caches and logs, so it is only a lower bound for that charging scenario'},
              'allocation_rule': 'Report global buckets separately. Count each regular inode once within its bucket; do not follow symlinks. Never silently amortize preparation infrastructure across an invented fleet size to pass E5.',
              'limitations': [
                  'The builder also retains kernel-building and multiple-fixture infrastructure; this is not a fresh minimal per-project materializer.',
                  'stat st_blocks*512 is allocated file storage, not a measurement of unique APFS extents across files or snapshots.',
                  'Buckets are not summed into a per-environment cost: runtime/private accounting and overlapping immutable archives have separate reports.',
                  'Failed VM disks retained under private temporary paths are described in their raw reports; they are outside these bucket subtotals.',
                  'A full E5 storage pass still needs a bounded minimal preparation footprint and an explicit global allocation/retention policy.'
              ]}
    args.out.write_text(json.dumps(result, indent=2, allow_nan=False) + '\n')
    print(json.dumps({'status': result['status'], 'builder_gib': builder['allocated_bytes'] / GIB,
                      'single_environment_builder_charge': result['conservative_single_environment_builder_charge']['criterion']}))


if __name__ == '__main__':
    main()
