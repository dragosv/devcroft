#!/usr/bin/env python3
"""Reclaim regenerable compilation artifacts, preserving runtime executables."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil

from cold_prepare import require_no_active_collector
from smoke import ROOT


def sha(path):
    with path.open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    require_no_active_collector(ROOT / 'runs')
    lease = (ROOT / 'artifacts/nested-runner.lock').open('a')
    fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
    os.umask(0o077)
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=False)
    workspace = ROOT.parent.parent
    targets = [workspace / 'target/debug', ROOT / 'libkrun/target/debug', ROOT / 'hvi/target/debug',
               ROOT / 'guest-keeper/target/release', ROOT / 'guest-keeper/target/aarch64-unknown-linux-musl/release']
    locks = []
    for target in targets:
        if target.exists():
            file = (target / '.cargo-lock').open('a')
            fcntl.flock(file, fcntl.LOCK_EX | fcntl.LOCK_NB)
            locks.append(file)
    protected = [ROOT / 'libkrun/target/debug/devcroft-krun-spike', ROOT / 'hvi/target/debug/devcroft-hvi-spike',
                 ROOT / 'libkrun/target/aarch64-unknown-linux-musl/debug/devcroft-krun-spike',
                 ROOT / 'guest-keeper/target/aarch64-unknown-linux-musl/release/devcroft-guest-keeper']
    if (workspace / 'target/debug/devcroft').exists():
        protected.append(workspace / 'target/debug/devcroft')
    before = {str(path): sha(path) for path in protected}
    report = {'schema': 1, 'status': 'failed', 'qualification': 'regenerable workspace/experiment compiler cache cleanup; runtime files retained',
              'raw_evidence': str(args.out), 'free_bytes_before': shutil.disk_usage('/private/tmp').free,
              'protected_executable_sha256': before, 'removed_directories': [], 'removed_libraries': []}
    for target in targets:
        if not target.exists():
            continue
        for name in ('incremental', 'build', '.fingerprint'):
            path = target / name
            if path.exists():
                if path.is_symlink() or not path.resolve().is_relative_to(workspace):
                    raise RuntimeError('unexpected compiler cache directory')
                shutil.rmtree(path)
                report['removed_directories'].append(str(path))
        deps = target / 'deps'
        if deps.is_dir() and not deps.is_symlink():
            for path in deps.iterdir():
                # Keep test executables and dynamic libraries. These formats
                # are compiler/linker intermediates, never runtime devices.
                if path.is_file() and not path.is_symlink() and path.suffix in ('.rlib', '.rmeta', '.d', '.o', '.a'):
                    report['removed_libraries'].append({'path': str(path), 'allocated_bytes': path.stat().st_blocks * 512})
                    path.unlink()
    if before != {str(path): sha(path) for path in protected}:
        raise RuntimeError('a protected runtime executable changed')
    report.update(status='passed', free_bytes_after=shutil.disk_usage('/private/tmp').free,
                  runtime_executables_unchanged=True, images_and_evidence_not_in_cleanup_scope=True)
    (args.out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'status': report['status'], 'freed_gib': (report['free_bytes_after'] - report['free_bytes_before']) / 1024**3,
                      'protected_binaries': len(protected)}))


if __name__ == '__main__':
    main()
