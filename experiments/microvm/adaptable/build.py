#!/usr/bin/env python3
"""Linux builder: immutable offline setup inputs and an empty private base seed."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile

from recipe import digest, encoded, load, verify_inputs, ORIGIN

WORK = Path('/home/dragos.guest/devcroft-adaptable')


def run(*args, capture=False):
    result = subprocess.run(list(map(str, args)), check=True, timeout=300,
        stdout=subprocess.PIPE if capture else None, text=capture)
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--freeze-inputs', action='store_true')
    args = parser.parse_args()
    os.chdir(WORK)
    apks = WORK / 'apks'
    apks.mkdir(exist_ok=True)
    if args.freeze_inputs:
        def freeze(names):
            with tempfile.TemporaryDirectory(prefix='resolve-', dir=WORK) as fresh:
                urls = run('apk', 'fetch', '--from', 'repositories', '--recursive', '--url', '--output', fresh,
                           *names, capture=True).splitlines()
                if not urls:
                    raise RuntimeError('empty APK closure')
                result = []
                for url in sorted(set(urls)):
                    if not url.startswith(ORIGIN):
                        raise RuntimeError('unexpected APK origin')
                    file = apks / url.rsplit('/', 1)[1]
                    shutil.copyfile(Path(fresh) / file.name, file)
                    result.append({'file': file.name, 'url': url, 'sha256': digest(file)})
                return result
        value = {'schema': 1, 'architecture': 'aarch64-linux', 'mechanism': 'alpine-apk-offline',
            'base_sha256': digest(WORK / 'base.img'), 'source_sha256': digest(WORK / 'probe.c'),
            'manager_packages': freeze(['apk-tools', 'alpine-keys']), 'packages': freeze(['zlib-dev'])}
        (WORK / 'recipe.json').write_bytes(encoded(value))
    value = load(WORK / 'recipe.json')
    if value['base_sha256'] != digest(WORK / 'base.img') or value['source_sha256'] != digest(WORK / 'probe.c'):
        raise RuntimeError('base/source mismatch')
    for item in value['manager_packages'] + value['packages']:
        file = apks / item['file']
        if not file.exists():
            run('curl', '--fail', '--location', '--max-time', '120', item['url'], '-o', file)
    verify_inputs(value, apks)
    image = WORK / 'toolchain.img'
    shutil.copyfile(WORK / 'base.img', image)
    root = WORK / 'root'
    root.mkdir(exist_ok=True)
    run('sudo', 'mount', '-o', 'loop', image, root)
    try:
        run('sudo', 'apk', '--root', root, '--no-network', '--no-scripts', '--force-non-repository',
            '--keys-dir', '/etc/apk/keys', 'add', *[apks / item['file'] for item in value['manager_packages']])
        if (root / 'usr/include/zlib.h').exists():
            raise RuntimeError('baseline already contains the dependency header')
        inputs = root / 'adaptable'
        run('sudo', 'mkdir', '-p', inputs / 'apks')
        run('sudo', 'chown', '-R', f'{os.getuid()}:{os.getgid()}', inputs)
        (inputs / 'recipe.json').write_bytes(encoded(value))
        shutil.copyfile(WORK / 'probe.c', inputs / 'probe.c')
        lines = []
        for item in value['packages']:
            shutil.copyfile(apks / item['file'], inputs / 'apks' / item['file'])
            lines.append(f"{item['sha256']}  /toolchain/adaptable/apks/{item['file']}\n")
        (inputs / 'SHA256SUMS').write_text(''.join(lines))
        run('sync')
    finally:
        run('sudo', 'umount', root)
    run('truncate', '-s', '768M', WORK / 'data.seed.img')
    run('mkfs.ext4', '-F', '-q', '-m', '0', '-O', '^orphan_file', '-U',
        'd31c0005-0000-4000-8000-000000000005', WORK / 'data.seed.img')
    with tarfile.open(WORK / 'packages.tar', 'w') as out:
        for item in {x['file']: x for x in value['manager_packages'] + value['packages']}.values():
            out.add(apks / item['file'], arcname=item['file'])
    receipt = {'schema': 1, 'recipe': value, 'recipe_sha256': digest(WORK / 'recipe.json'),
        'build_script_sha256': digest(Path(__file__)), 'recipe_validator_sha256': digest(WORK / 'recipe.py'),
        'baseline_dependency_header_absent': True,
        'filesystem': 'vdc immutable baseline/offline inputs; vdd private rootfs and separate workspace subtree',
        'builder_packages': run('apk', 'info', '-v', capture=True).splitlines()}
    for name in ('toolchain.img', 'data.seed.img', 'packages.tar'):
        file = WORK / name
        receipt[name] = {'sha256': digest(file), 'bytes': file.stat().st_size}
    (WORK / 'build.json').write_text(json.dumps(receipt, indent=2) + '\n')


if __name__ == '__main__':
    main()
