#!/usr/bin/env python3
"""Count post-filter helper syscalls and inherited threads in strace -ff archives.

Only successful functional samples are used. Controller/startup/probe processes
are excluded; the trace starts at successful seccomp installation, then follows
clone results from those filtered threads. No archive paths are extracted.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re
import tarfile


def collect(directory):
    report = json.loads((directory / 'raw/report.json').read_text())
    if report['status'] != 'passed':
        raise ValueError(f'cannot derive an accepted inventory from failed run: {directory}')
    archive = directory / 'traces.tar.gz'
    files = {}
    with tarfile.open(archive) as tar:
        for member in tar.getmembers():
            if member.isfile() and member.name.rsplit('.', 1)[-1].isdigit():
                files[int(member.name.rsplit('.', 1)[-1])] = tar.extractfile(member).read().decode(errors='replace').splitlines()
    roots = {}
    for pid, lines in files.items():
        if any(line.startswith('execve(') and 'devcroft-krun-spike' in line and '"run"' in line for line in lines):
            installs = [i for i, line in enumerate(lines) if line.startswith('seccomp(SECCOMP_SET_MODE_FILTER,') and line.endswith('= 0')]
            if len(installs) != 1:
                raise ValueError(f'expected one successful inherited filter installation in {pid}')
            roots[pid] = installs[0] + 1
    if not roots:
        raise ValueError('no helper run/filter installation found')
    pending = list(roots.items())
    visited = set()
    counts = Counter()
    clone_flags = set()
    while pending:
        pid, begin = pending.pop()
        if pid in visited:
            continue
        visited.add(pid)
        if pid not in files:
            raise ValueError(f'missing inherited thread trace {pid}')
        for line in files[pid][begin:]:
            match = re.match(r'([a-zA-Z0-9_]+)\(', line)
            if match:
                counts[match[1]] += 1
            if line.startswith('clone(') or line.startswith('<... clone resumed>'):
                child = re.search(r'= ([1-9][0-9]*)$', line)
                if child:
                    pending.append((int(child[1]), 0))
                flags = re.search(r'flags=([^,]+)', line)
                if flags:
                    clone_flags.add(flags[1])
    return {'run': str(directory), 'trace_sha256': hashlib.sha256(archive.read_bytes()).hexdigest(),
            'helper_artifact': report['helper_artifact'], 'helper_provenance': report['helper_provenance'],
            'filtered_process_roots': sorted(roots), 'filtered_threads': len(visited),
            'clone_flags': sorted(clone_flags), 'syscalls': dict(sorted(counts.items()))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('runs', nargs='+', type=Path)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    samples = [collect(directory) for directory in args.runs]
    counts = Counter()
    for sample in samples:
        counts.update(sample['syscalls'])
    result = {'schema': 1, 'platform': 'linux-arm64-nested-on-macos-vz',
              'collection_mode': 'kill-on-miss; no log/permissive filter',
              'scope': 'post-filter helper run roots and their inherited threads; no controller or probe syscalls',
              'samples': samples, 'syscalls': dict(sorted(counts.items()))}
    args.out.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'samples': len(samples), 'syscalls': sorted(counts)}))


if __name__ == '__main__':
    main()
