#!/usr/bin/env python3
"""Materialize a locked Linux Nix closure in the mount-free experiment builder."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

WORK = Path('/home/dragos.guest/devcroft-nix')


def digest(path):
    with path.open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


def run(*argv, env=None, capture=False, timeout=1800, cwd=None):
    result = subprocess.run(list(map(str, argv)), env=env, cwd=cwd, check=True, timeout=timeout,
                            stdout=subprocess.PIPE if capture else None, text=capture)
    return result.stdout if capture else None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--work-dir', type=Path, default=WORK)
    parser.add_argument('--required-binary', action='append')
    args = parser.parse_args()
    work = args.work_dir
    if not work.is_absolute() or work.is_symlink():
        raise RuntimeError('builder work directory must be an absolute non-symlink path')
    materialize(work, args.required_binary or ('gcc', 'make', 'redis-server', 'code-server'))


def materialize(WORK, required_binaries):
    os.chdir(WORK)
    pin = json.loads((WORK / 'bootstrap.json').read_text())
    archive = WORK / 'nix-bootstrap.tar.xz'
    if not archive.exists():
        run('curl', '--fail', '--location', '--max-time', '180', pin['url'], '-o', archive)
    if digest(archive) != pin['sha256']:
        raise RuntimeError('Nix bootstrap archive mismatch')
    nix = Path.home() / '.nix-profile/bin/nix'
    if not nix.exists():
        unpack = WORK / 'bootstrap'
        if unpack.exists():
            run('chmod', '-R', 'u+w', unpack)
            shutil.rmtree(unpack)
        unpack.mkdir()
        run('tar', '-xJf', archive, '--strip-components=1', '-C', unpack)
        run('sudo', 'mkdir', '-p', '/nix')
        run('sudo', 'chown', f'{os.getuid()}:{os.getgid()}', '/nix')
        run('sh', unpack / 'install', '--no-daemon', '--no-channel-add', '--no-modify-profile', cwd=unpack,
            env=dict(os.environ, NIX_CONFIG='build-users-group =\n'))
    version = run(nix, '--version', capture=True).strip()
    if version != f"nix (Nix) {pin['version']}":
        raise RuntimeError(f'unexpected Nix builder: {version}')
    env = dict(os.environ, PATH=f'{nix.parent}:{os.environ["PATH"]}', NIX_REMOTE='local',
        NIX_SSL_CERT_FILE='/etc/ssl/certs/ca-certificates.crt', NIX_CONFIG='\n'.join([
            'experimental-features = nix-command flakes', 'accept-flake-config = false',
            'build-users-group =', 'sandbox = false', 'max-jobs = 2', 'cores = 2',
            'substituters = https://cache.nixos.org', 'connect-timeout = 20', 'stalled-download-timeout = 60']))
    definition = WORK / 'definition'
    before = {name: digest(definition / name) for name in ('flake.nix', 'flake.lock')}
    ref = f'path:{definition}'
    flags = ['--no-update-lock-file', '--no-write-lock-file']
    metadata = json.loads(run(nix, 'flake', 'metadata', '--json', *flags, ref, env=env, capture=True))
    built = json.loads(run(nix, 'build', '--json', '--no-link', *flags,
        ref + '#packages.aarch64-linux.default', env=env, capture=True))
    profile = built[0]['outputs']['out']
    if not profile.startswith('/nix/store/'):
        raise RuntimeError('unexpected closure root')
    versions = json.loads(run(nix, 'eval', '--json', *flags,
        ref + '#fixtureVersions.aarch64-linux', env=env, capture=True))
    run(nix, 'store', 'verify', '--recursive', profile, env=env)
    paths = json.loads(run(nix, 'path-info', '--json', '--recursive', profile, env=env, capture=True))
    (WORK / 'closure.json').write_text(json.dumps(paths, indent=2) + '\n')
    cache = WORK / 'cache'
    cache.mkdir(exist_ok=True)
    run(nix, 'copy', '--to', f'file://{cache}', profile, env=env)
    # An empty store reconstructs the runtime filesystem solely from the captured
    # NAR cache. The locally built top-level environment is intentionally unsigned;
    # source verification above checks cache signatures and local build trust.
    root = WORK / 'root'
    if root.exists():
        run('sudo', 'rm', '-rf', root)
    root.mkdir()
    offline = dict(env, NIX_CONFIG=env['NIX_CONFIG'] + '\nsubstituters =\n')
    isolated = ['sudo', 'unshare', '--net', 'sudo', '-u', f'#{os.getuid()}', '-g', f'#{os.getgid()}',
                '--', 'env', *[f'{key}={value}' for key, value in sorted(offline.items())]]
    run(*isolated, nix, 'copy', '--offline', '--from', f'file://{cache}', '--to', root,
        '--no-check-sigs', profile)
    run(*isolated, nix, 'store', 'verify', '--store', root, '--recursive', '--no-trust', profile)
    copied = json.loads(run(nix, 'path-info', '--store', root, '--json', '--recursive', profile,
                            env=offline, capture=True))
    def nar_hashes(records):
        return {path: value['narHash'] for path, value in records.items()}
    if nar_hashes(paths) != nar_hashes(copied):
        raise RuntimeError('fresh offline store differs from the source closure')
    (root / 'usr').symlink_to(profile)
    (root / 'lib').symlink_to(profile + '/lib')
    etc = root / 'etc'
    etc.mkdir()
    (etc / 'passwd').write_text('root:x:0:0:root:/home/devcroft:/bin/sh\ndevcroft:x:0:0:fixture:/home/devcroft:/bin/sh\n')
    (etc / 'group').write_text('root:x:0:\n')
    (etc / 'nsswitch.conf').write_text('passwd: files\ngroup: files\nhosts: files\n')
    (etc / 'hosts').write_text('127.0.0.1 localhost\n::1 localhost\n')
    fixture = root / 'fixture'
    fixture.mkdir()
    source = json.loads((WORK / 'source.json').read_text())
    if digest(WORK / 'xxhash.tar.gz') != source['sha256']:
        raise RuntimeError('build source mismatch')
    (fixture / 'xxHash').mkdir()
    run('tar', '-xzf', WORK / 'xxhash.tar.gz', '--strip-components=1', '-C', fixture / 'xxHash')
    small = fixture / 'small'
    small.mkdir()
    checksums = []
    for number in range(source['small_files']):
        name = f'd{number % 32:02d}/file-{number:04d}.txt'
        file = small / name
        file.parent.mkdir(exist_ok=True)
        file.write_text(f'microvm-small-file-v1 {number}\n' + hashlib.sha256(str(number).encode()).hexdigest() * 4 + '\n')
        checksums.append(f'{digest(file)}  {name}\n')
    (small / 'SHA256SUMS').write_text(''.join(checksums))
    # Required binaries are closure links, never copied from Alpine's /usr.
    for binary in required_binaries:
        resolved = os.path.realpath(root / 'usr/bin' / binary)
        if not resolved.startswith('/nix/store/'):
            raise RuntimeError(f'{binary} is not a Nix closure executable')
    size = sum(value['narSize'] for value in paths.values())
    image_bytes = max(512 * 1024**2, ((int(size * 1.25) + 128 * 1024**2) // (64 * 1024**2) + 1) * 64 * 1024**2)
    if image_bytes > 3 * 1024**3:
        raise RuntimeError('closure exceeds the 3 GiB experiment image budget')
    run('truncate', '-s', image_bytes, WORK / 'toolchain.img')
    run('mkfs.ext4', '-F', '-q', '-m', '0', '-O', '^orphan_file', '-U',
        'd31c0003-0000-4000-8000-000000000003', '-d', root, WORK / 'toolchain.img')
    run('truncate', '-s', '128M', WORK / 'data.seed.img')
    run('mkfs.ext4', '-F', '-q', '-m', '0', '-O', '^orphan_file', '-U',
        'd31c0004-0000-4000-8000-000000000004', WORK / 'data.seed.img')
    with tarfile.open(WORK / 'cache.tar', 'w') as out:
        out.add(cache, arcname='cache')
    if before != {name: digest(definition / name) for name in before}:
        raise RuntimeError('flake definition or lock changed')
    receipt = {'schema': 1, 'architecture': 'aarch64-linux', 'bootstrap': pin, 'nix_version': version,
        'flake_sha256': before, 'flake_metadata': metadata, 'profile': profile, 'versions': versions,
        'closure_paths': len(paths), 'nar_bytes': size, 'closure_nar_hashes': nar_hashes(paths),
        'offline_materialization': {'fresh_store': True, 'network_namespace': 'unshare --net',
            'nar_contents_verified': True, 'source_closure_trust_verified': True,
            'destination_signature_check': 'disabled only for private locally produced cache; NAR hashes independently verified'},
        'builder': {'host_mounts': [], 'nix_build_sandbox': False, 'isolation': 'dedicated Lima Linux VM; trusted experiment recipe',
            'packages': run('apk', 'info', '-v', capture=True).splitlines()},
        'build_script_sha256': digest(Path(__file__)), 'project_source': source,
        'qualification': 'pinned closure materialization; not full source rebuild or fleet eligibility'}
    for name in ('toolchain.img', 'data.seed.img', 'cache.tar', 'closure.json'):
        file = WORK / name
        receipt[name] = {'sha256': digest(file), 'bytes': file.stat().st_size}
    (WORK / 'build.json').write_text(json.dumps(receipt, indent=2) + '\n')


if __name__ == '__main__':
    main()
