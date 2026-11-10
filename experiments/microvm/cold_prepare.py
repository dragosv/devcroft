#!/usr/bin/env python3
"""Clean offline agent-closure reconstruction and real guest verification.

Never run alongside a density collector. The dedicated materializer VM has no
host mounts; the complete reconstruction command runs without a network stack.
"""
import argparse
import datetime
import fcntl
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile
import time

ROOT = Path(__file__).resolve().parent


def digest(path):
    with path.open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


def require_no_active_collector(runs):
    for pid_file in runs.glob('*/runner.pid'):
        report_path = pid_file.parent / 'report.json'
        if report_path.exists() and json.loads(report_path.read_text())['status'] == 'running':
            raise RuntimeError(f'density collection still active: {pid_file.parent}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    started = time.monotonic()
    # Stop before any heavy work, not merely before starting another VM.
    require_no_active_collector(ROOT / 'runs')
    lock = (ROOT / 'artifacts/nested-runner.lock').open('a')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    os.umask(0o077)
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=False)
    protocol_path = ROOT / 'fixtures/cold-preparation-protocol.json'
    protocol = json.loads(protocol_path.read_text())
    accepted = ROOT / 'fixtures/no-go-thresholds.json'
    if digest(accepted) != protocol['thresholds_sha256']:
        raise RuntimeError('accepted thresholds changed')
    report = {'schema': 1, 'status': 'running', 'qualification': 'clean offline closure preparation; completed authenticated agent task not evaluated',
              'platform': 'macos-arm64-native', 'host': list(os.uname()), 'protocol': protocol,
              'protocol_sha256': digest(protocol_path), 'raw_evidence': str(args.out), 'phases': []}

    def save():
        (args.out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')

    def phase(name, argv, env=None, timeout=900):
        began = time.monotonic()
        record = {'name': name, 'argv': list(map(str, argv)), 'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat()}
        report['phases'].append(record)
        save()
        with (args.out / f'{name}.stdout').open('wb') as stdout, (args.out / f'{name}.stderr').open('wb') as stderr:
            result = subprocess.run(record['argv'], stdout=stdout, stderr=stderr, env=env, timeout=timeout)
        record.update(seconds=time.monotonic() - began, exit_code=result.returncode)
        save()
        print(json.dumps({'phase': name, 'seconds': record['seconds'], 'exit_code': result.returncode}), flush=True)
        if result.returncode:
            raise RuntimeError(f'{name} failed; see retained stderr')

    env = dict(os.environ, LIMA_HOME='/private/tmp/devcroft-kernel-builder')

    def lima(name, *argv, timeout=900):
        phase(name, ['limactl', *argv], env=env, timeout=timeout)

    remote = '/home/dragos.guest/devcroft-clean-agents-' + hashlib.sha256(str(args.out).encode()).hexdigest()[:12]
    report['builder_namespace'] = remote
    source_receipt = json.loads((ROOT / 'fixtures/agents-aarch64-build.json').read_text())
    archive = ROOT / 'artifacts/agents/cache.tar'
    if digest(archive) != source_receipt['cache.tar']['sha256']:
        raise RuntimeError('downloaded NAR archive changed')
    # Hash-authenticated archive, with a second path/type check before extraction.
    with tarfile.open(archive) as tar:
        for member in tar:
            path = Path(member.name)
            if path.is_absolute() or '..' in path.parts or path.parts[0] != 'cache' or not (member.isdir() or member.isfile()):
                raise RuntimeError('unexpected cache archive member')
    sources = args.out / 'sources'
    sources.mkdir()
    inputs = [(ROOT / 'nix/build.py', 'build.py'), (ROOT / 'agents/flake.nix', 'definition/flake.nix'),
              (ROOT / 'agents/flake.lock', 'definition/flake.lock'),
              (ROOT / 'fixtures/nix-bootstrap-aarch64.json', 'bootstrap.json'),
              (ROOT / 'fixtures/workload-source.json', 'source.json'),
              (ROOT / 'artifacts/workload/xxhash.tar.gz', 'xxhash.tar.gz')]
    report['inputs'] = {name: {'sha256': digest(file), 'path': str(file)} for file, name in inputs}
    report['inputs']['cache.tar'] = {'sha256': digest(archive), 'path': str(archive)}
    report['harness_sha256'] = digest(Path(__file__))
    for file, name in inputs:
        target = sources / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(file.read_bytes())
    report['input_verification_seconds'] = time.monotonic() - started
    builder_started = False
    try:
        lima('builder-precondition', 'list', '--json')
        rows = (args.out / 'builder-precondition.stdout').read_text().splitlines()
        if any(json.loads(line)['status'] != 'Stopped' for line in rows if line):
            raise RuntimeError('dedicated builder must initially be stopped')
        builder_started = True
        lima('builder-start', 'start', '--tty=false', 'kernel-build')
        lima('create-empty-namespace', 'shell', 'kernel-build', 'mkdir', '-p', remote + '/definition')
        lima('reuse-downloaded-bootstrap', 'shell', 'kernel-build', 'cp',
             '/home/dragos.guest/devcroft-nix/nix-bootstrap.tar.xz', remote + '/nix-bootstrap.tar.xz')
        for index, (file, name) in enumerate(inputs):
            lima(f'input-{index}', 'copy', str(file), f'kernel-build:{remote}/{name}')
        lima('downloaded-cache-transfer', 'copy', str(archive), f'kernel-build:{remote}/downloaded-cache.tar')
        lima('cache-receipt', 'shell', 'kernel-build', 'sha256sum', remote + '/downloaded-cache.tar')
        if (args.out / 'cache-receipt.stdout').read_text().split()[0] != source_receipt['cache.tar']['sha256']:
            raise RuntimeError('transferred cache archive changed')
        lima('cache-unpack', 'shell', 'kernel-build', 'tar', '-xf', remote + '/downloaded-cache.tar', '-C', remote)
        # Match the installed builder user, never the host account or its HOME.
        lima('builder-uid', 'shell', 'kernel-build', 'id', '-u')
        lima('builder-gid', 'shell', 'kernel-build', 'id', '-g')
        uid, gid = [(args.out / f'builder-{name}.stdout').read_text().strip() for name in ('uid', 'gid')]
        if not uid.isdigit() or not gid.isdigit():
            raise RuntimeError('unexpected builder identity')
        command = ['sudo', 'unshare', '--net', 'sudo', '-u', '#' + uid, '-g', '#' + gid,
                   'env', 'PYTHONUNBUFFERED=1', 'python3', remote + '/build.py', '--work-dir', remote]
        for binary in ('gcc', 'make', 'redis-server', 'codex', 'claude', 'python3', 'curl'):
            command.extend(['--required-binary', binary])
        lima('offline-materialize-verify-image', 'shell', 'kernel-build', *command)
        for name in ('build.json', 'closure.json'):
            lima('receipt-' + name, 'copy', f'kernel-build:{remote}/{name}', str(args.out / name))
        built = json.loads((args.out / 'build.json').read_text())
        if built['closure_nar_hashes'] != source_receipt['closure_nar_hashes'] or built['profile'] != source_receipt['profile']:
            raise RuntimeError('clean closure differs from frozen definition')
        report['clean_closure_verified'] = True
        # Free reconstructible intermediates before exporting another raw image.
        lima('remove-intermediates', 'shell', 'kernel-build', 'sudo', 'rm', '-rf',
             remote + '/root', remote + '/cache', remote + '/cache.tar', remote + '/downloaded-cache.tar')
        lima('trim-intermediates', 'shell', 'kernel-build', 'sudo', 'fstrim', '-v', '/')
        for name in ('toolchain.img', 'data.seed.img'):
            lima('export-' + name, 'copy', f'kernel-build:{remote}/{name}', str(args.out / name))
            if digest(args.out / name) != built[name]['sha256']:
                raise RuntimeError('clean exported image mismatch')
        lima('remove-builder-image-copies', 'shell', 'kernel-build', 'sudo', 'rm', '-f',
             remote + '/toolchain.img', remote + '/data.seed.img')
        lima('trim-exported-copies', 'shell', 'kernel-build', 'sudo', 'fstrim', '-v', '/')
        lima('builder-stop', 'stop', 'kernel-build')
        builder_started = False
        report['offline_preparation_seconds'] = time.monotonic() - started
        phase('clean-guest-verification', ['python3', ROOT / 'agent_probe.py', '--adapter', 'libkrun',
              '--repetitions', '1', '--image', args.out / 'toolchain.img', '--seed', args.out / 'data.seed.img',
              '--out', args.out / 'verification'])
        report['end_to_end_with_guest_verification_seconds'] = time.monotonic() - started
        limit = json.loads(accepted.read_text())['original_no_go_thresholds']['cold_preparation_seconds']
        report['cold_preparation_limit_seconds'] = limit
        report['cold_preparation_criterion'] = 'passed' if report['end_to_end_with_guest_verification_seconds'] <= limit else 'failed'
        report['status'] = report['cold_preparation_criterion']
    except BaseException as error:
        report.update(status='failed', error=f'{type(error).__name__}: {error}')
    finally:
        if builder_started:
            try:
                lima('builder-finally-stop', 'stop', 'kernel-build')
            except Exception as error:
                report['cleanup_error'] = str(error)
        save()
    print(json.dumps({key: report.get(key) for key in ('status', 'offline_preparation_seconds', 'end_to_end_with_guest_verification_seconds', 'error')}))
    raise SystemExit(report['status'] != 'passed')


if __name__ == '__main__':
    main()
