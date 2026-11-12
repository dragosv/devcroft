#!/usr/bin/env python3
"""Deliberately refresh the experimental SSH source pin and reproducible diffs."""
import argparse
import difflib
import hashlib
import json
from pathlib import Path
import tarfile

from smoke import ROOT


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--crate', type=Path)
    args = parser.parse_args()
    manifest = ROOT / 'fixtures/ssh-consumption-windows.json'
    record = json.loads(manifest.read_text())
    archives = [args.crate] if args.crate else list((Path.home() / '.cargo/registry/cache').glob('*/russh-0.62.5.crate'))
    archives = [path for path in archives if sha(path.read_bytes()) == record['upstream_crate_sha256']]
    if not archives:
        raise RuntimeError('fixed upstream crate archive is not available')
    changed, patch = {}, []
    with tarfile.open(archives[0]) as archive:
        for member in archive:
            if not member.isfile():
                continue
            relative = member.name.removeprefix('russh-0.62.5/')
            path = Path(relative)
            if path.is_absolute() or '..' in path.parts:
                raise RuntimeError('unexpected upstream archive member')
            original, current = archive.extractfile(member).read(), (ROOT / 'russh' / relative).read_bytes()
            if original != current:
                changed[relative] = {'original_sha256': sha(original), 'patched_sha256': sha(current)}
                patch.extend(difflib.unified_diff(original.decode().splitlines(True), current.decode().splitlines(True),
                                               fromfile='a/' + relative, tofile='b/' + relative))
    patch_path = ROOT / 'patches/russh-consumed-input-windows.patch'
    patch_path.write_text(''.join(patch))
    record.update(patch={'path': str(patch_path.relative_to(ROOT)), 'sha256': sha(patch_path.read_bytes())}, modified_upstream_files=changed)
    baseline = (ROOT.parent.parent / 'src/ssh/server.rs').read_bytes()
    if sha(baseline) != record['common_server_baseline']['sha256']:
        raise RuntimeError('common production server changed; review the fork baseline deliberately')
    fork = ROOT / 'guest-keeper/src/ssh_server.rs'
    keeper_patch = ROOT / 'patches/keeper-ssh-consumed-input.patch'
    keeper_patch.write_text(''.join(difflib.unified_diff(baseline.decode().splitlines(True), fork.read_text().splitlines(True),
                                                      fromfile='a/ssh_server.rs', tofile='b/ssh_server.rs')))
    files = {}
    for path in sorted((ROOT / 'russh').rglob('*')):
        if 'target' in path.relative_to(ROOT / 'russh').parts:
            continue
        if path.is_symlink():
            raise RuntimeError('vendored source may not contain symlinks')
        if path.is_file():
            files[str(path.relative_to(ROOT / 'russh'))] = sha(path.read_bytes())
    record.update(vendor_files=files, keeper_fork={'path': str(fork.relative_to(ROOT)), 'sha256': sha(fork.read_bytes()),
                  'patch': str(keeper_patch.relative_to(ROOT)), 'patch_sha256': sha(keeper_patch.read_bytes()),
                  'reconstruction': 'Copy the recorded common-server baseline to the experimental ssh_server.rs, then apply the keeper patch there; do not patch the production source'})
    manifest.write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps({'vendor_files': len(files), 'modified_upstream_files': list(changed), 'keeper_sha256': record['keeper_fork']['sha256']}))


if __name__ == '__main__':
    main()
