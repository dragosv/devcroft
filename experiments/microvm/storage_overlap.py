#!/usr/bin/env python3
"""Account pinned overlapping closures separately from deployed raw disks."""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def file_record(path):
    info = path.stat()
    return {'path': str(path), 'logical_bytes': info.st_size, 'allocated_bytes': info.st_blocks * 512}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    definitions = {'nix-baseline': ('nix', 'nix-aarch64'), 'agents': ('agents', 'agents-aarch64')}
    records, closures = {}, {}
    for name, (directory, receipt_name) in definitions.items():
        receipt_path = ROOT / 'fixtures' / f'{receipt_name}-build.json'
        closure_path = ROOT / 'fixtures' / f'{receipt_name}-closure.json'
        receipt = json.loads(receipt_path.read_text())
        closure = json.loads(closure_path.read_text())
        if hashlib.sha256(closure_path.read_bytes()).hexdigest() != receipt['closure.json']['sha256']:
            raise ValueError('closure metadata differs from retained build receipt')
        closures[name] = closure
        deployed = [file_record(ROOT / 'artifacts' / directory / filename) for filename in ('toolchain.img', 'data.seed.img')]
        archives = [file_record(ROOT / 'artifacts' / directory / 'cache.tar')]
        records[name] = {'closure_paths': len(closure), 'nar_bytes': sum(x['narSize'] for x in closure.values()),
                         'profile': receipt['profile'], 'versions': receipt['versions'],
                         'deployed_image_and_seed': deployed, 'retained_download_archive': archives,
                         'deployed_allocated_bytes': sum(x['allocated_bytes'] for x in deployed),
                         'retained_archive_allocated_bytes': sum(x['allocated_bytes'] for x in archives),
                         'builder_output_namespace': '/home/dragos.guest/devcroft-' + directory,
                         'definition_receipt_sha256': hashlib.sha256(receipt_path.read_bytes()).hexdigest()}
    a, b = closures.values()
    common = sorted(set(a) & set(b))
    for path in common:
        if (a[path]['narHash'], a[path]['narSize']) != (b[path]['narHash'], b[path]['narSize']):
            raise ValueError('same store path has inconsistent immutable content')
    shared = sum(a[path]['narSize'] for path in common)
    result = {'schema': 1, 'status': 'passed', 'qualification': 'closure overlap and actual retained image/archive allocation; no APFS shared-extent attribution',
              'platform': 'macos-arm64-native', 'environments': records,
              'overlap': {'same_store_paths': len(common), 'common_nar_bytes': shared,
                          'union_store_paths': len(set(a) | set(b)),
                          'union_nar_bytes': sum(x['narSize'] for x in a.values()) + sum(x['narSize'] for x in b.values()) - shared,
                          'shared_paths': common},
              'allocation_rule': 'Raw images are separate immutable files; charge actual stat blocks to each image. NAR identity overlap does not prove cross-image filesystem extent deduplication. Private disks are measured per VM in density evidence.',
              'cache_rule': 'Each declared fixture has its own captured immutable NAR archive and builder output namespace. Linux Nix store sharing is immutable; runtime writable source/build/service state stays on each private ext4 disk.',
              'excluded_from_this_subtotal': ['native helpers/common kernel/initramfs/runtime disks (counted in density deployed inputs)',
                                            'retained raw diagnostic logs and failed disks',
                                            'dedicated stopped builder root disk, source/Cargo/compiler caches; report separately as host preparation infrastructure'],
              'remaining_qualification': 'These subtotals do not independently satisfy the full per-environment E5 storage gate; combine with runtime/private and global preparation infrastructure measurements.'}
    args.out.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({name: {key: value[key] for key in ('closure_paths', 'nar_bytes', 'deployed_allocated_bytes', 'retained_archive_allocated_bytes')}
                      for name, value in records.items()}))
    print(json.dumps({key: value for key, value in result['overlap'].items() if key != 'shared_paths'}))


if __name__ == '__main__':
    main()
