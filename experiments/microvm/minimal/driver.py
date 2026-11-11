#!/usr/bin/env python3
"""Run only inside the fresh private materializer, under unshare --net."""
import hashlib
import json
import os
from pathlib import Path
import pwd
import shutil
import socket
import subprocess
import sys
import tarfile


def run(*argv, **kwargs):
    subprocess.run(list(map(str, argv)), check=True, timeout=600, **kwargs)


def chown_tree(path, uid, gid):
    os.chown(path, uid, gid, follow_symlinks=False)
    for directory, dirs, files in os.walk(path, followlinks=False):
        for name in dirs + files:
            os.chown(Path(directory) / name, uid, gid, follow_symlinks=False)


def main():
    work = Path(sys.argv[1])
    if os.geteuid() != 0 or [name for _, name in socket.if_nameindex()] != ['lo']:
        raise RuntimeError('materializer must run as guest root in its own network namespace')
    publication = json.loads((work / 'publication.json').read_text())
    identity = pwd.getpwnam('devcroft')
    (work / 'cache').mkdir()
    run('mount', '-t', 'tmpfs', '-o', 'size=512M,mode=0755', 'tmpfs', work / 'cache')
    with tarfile.open(work / 'inputs.tar') as archive:
        for member in archive:
            path = Path(member.name)
            if path.is_absolute() or '..' in path.parts or not (member.isdir() or member.isfile()):
                raise RuntimeError('unexpected immutable input bundle member')
        archive.extractall(work, filter='data')
    (work / 'inputs.tar').unlink()
    apks = sorted((work / 'apk').glob('*.apk'))
    if not apks:
        raise RuntimeError('no pinned tool APKs in downloaded bundle')
    run('apk', 'add', '--no-cache', '--no-network', *apks)
    shutil.rmtree(work / 'apk')
    bootstrap_pin = json.loads((work / 'bootstrap.json').read_text())
    with (work / 'nix-bootstrap.tar.xz').open('rb') as file:
        if hashlib.file_digest(file, 'sha256').hexdigest() != bootstrap_pin['sha256']:
            raise RuntimeError('Nix bootstrap archive mismatch')
    bootstrap = work / 'bootstrap'
    bootstrap.mkdir()
    run('tar', '-xJf', work / 'nix-bootstrap.tar.xz', '--strip-components=1', '-C', bootstrap)
    Path('/nix').mkdir(exist_ok=True)
    os.chown('/nix', identity.pw_uid, identity.pw_gid)
    chown_tree(work, identity.pw_uid, identity.pw_gid)
    user = ['sudo', '-H', '-u', '#' + str(identity.pw_uid), '-g', '#' + str(identity.pw_gid), '--',
            'env', 'NIX_REMOTE=local', 'NIX_CONFIG=experimental-features = nix-command flakes\nbuild-users-group =\n']
    run(*user, 'sh', bootstrap / 'install', '--no-daemon', '--no-channel-add', '--no-modify-profile', cwd=bootstrap)
    shutil.rmtree(bootstrap)
    fetcher = Path(identity.pw_dir) / '.cache/nix'
    fetcher.mkdir(parents=True, exist_ok=True)
    for name in publication['fetcher_databases']:
        if Path(name).name != name or not name.startswith('fetcher-cache-') or not name.endswith('.sqlite'):
            raise RuntimeError('unexpected fetch metadata name')
        for suffix in ('', '-wal', '-shm'):
            (fetcher / (name + suffix)).unlink(missing_ok=True)
        shutil.copyfile(work / name, fetcher / name)
    chown_tree(Path(identity.pw_dir) / '.cache', identity.pw_uid, identity.pw_gid)
    nix = Path(identity.pw_dir) / '.nix-profile/bin/nix'
    run(*user, nix, 'copy', '--offline', '--no-check-sigs', '--from', 'file://' + str(work / 'cache'),
        publication['original_profile'], publication['nixpkgs']['path'])
    (work / 'root').mkdir()
    run('mount', '-t', 'tmpfs', '-o', 'size=2G,mode=0755', 'tmpfs', work / 'root')
    os.chown(work / 'root', identity.pw_uid, identity.pw_gid)
    command = [*user, 'python3', work / 'build.py', '--work-dir', work, '--offline-receipt', work / 'publication.json']
    for binary in ('gcc', 'make', 'redis-server', 'codex', 'claude', 'python3', 'curl'):
        command.extend(['--required-binary', binary])
    run(*command)
    (work / 'materialization-memory.txt').write_text(Path('/proc/meminfo').read_text())
    run('umount', work / 'root')
    run('umount', work / 'cache')
    print(json.dumps({'offline_interfaces': socket.if_nameindex(), 'user_uid': identity.pw_uid,
                      'user_home': identity.pw_dir, 'source_trust': 'frozen publication; fresh content and definition evaluation verified'}))


if __name__ == '__main__':
    main()
