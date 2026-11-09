#!/usr/bin/env python3
"""Run a labelled Linux ARM64 diagnostic inside the private mount-free VZ builder.

This is never a native Linux or density result. Inputs are copied, not shared.
The builder stops in finally unless --keep-running is explicitly selected.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent
REMOTE = '/home/dragos.guest/microvm-eval'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('row', choices=['stage-a', 'keeper', 'ssh', 'workload', 'nix-ssh', 'nix-workload'])
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--trace', action='store_true')
    parser.add_argument('--keep-running', action='store_true')
    args = parser.parse_args()
    os.umask(0o077)
    # Materialization paths are shared inside this dedicated builder. A second
    # controller must not truncate an artifact another VM is still reading.
    lock = (ROOT / 'artifacts/nested-runner.lock').open('a+')
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        parser.error('another nested runner owns the shared builder')
    args.out.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, LIMA_HOME='/private/tmp/devcroft-kernel-builder')
    receipt = {'schema': 1, 'platform': 'linux-arm64-nested-on-macos-vz', 'row': args.row,
               'native_linux_qualification': False, 'density_collection': False, 'traced': args.trace,
               'outer_host': list(os.uname()), 'status': 'failed'}

    def lima(*argv, capture=False):
        command = ['limactl', *map(str, argv)]
        return subprocess.check_output(command, env=env) if capture else subprocess.run(command, env=env, check=True)

    def python(code):
        return lima('shell', 'kernel-build', 'python3', '-c', code, capture=True)

    def upload(local, remote):
        lima('shell', 'kernel-build', 'mkdir', '-p', str(Path(remote).parent))
        lima('copy', local, 'kernel-build:' + remote)

    helper = ROOT / 'libkrun/target/aarch64-unknown-linux-musl/debug/devcroft-krun-spike'
    if helper.read_bytes()[:4] != b'\x7fELF':
        parser.error('build the static Linux helper with build_linux_helper.py first')
    run_name = args.out.name
    if not run_name or any(c not in 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_' for c in run_name):
        parser.error('output directory name must contain only letters, digits, - or _')
    remote_out = REMOTE + '/runs/' + run_name
    trace_prefix = REMOTE + '/traces/' + run_name
    started = False
    force_stop = False
    try:
        lima('start', '--tty=false', 'kernel-build')
        started = True
        config = json.loads(lima('list', 'kernel-build', '--json', capture=True))
        if not config['config'].get('nestedVirtualization') or config['config'].get('mounts'):
            raise ValueError('requires nestedVirtualization=true and no host mounts')
        python(f'from pathlib import Path; p=Path({remote_out!r}); assert not p.exists(), "remote output already exists"')
        receipt['outer_vm'] = {k: config[k] for k in ('vmType', 'arch', 'cpus', 'memory', 'limaVersion')}
        receipt['linux_kernel'] = lima('shell', 'kernel-build', 'uname', '-a', capture=True).decode().strip()
        receipt['packages'] = lima('shell', 'kernel-build', 'apk', 'info', '-v', capture=True).decode().splitlines()
        upload(helper, REMOTE + '/libkrun/target/debug/devcroft-krun-spike')
        receipt['helper_sha256'] = hashlib.sha256(helper.read_bytes()).hexdigest()
        scripts = ['smoke.py', 'keeper_probe.py', 'ssh_smoke.py', 'workload_smoke.py']
        for script in scripts:
            upload(ROOT / script, REMOTE + '/' + script)
        receipt['harness_sources'] = {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in scripts}
        upload(ROOT / 'artifacts/kernel-landlock/Image', REMOTE + '/artifacts/kernel-landlock/Image')
        directory = ('stage-a-after-adaptable' if args.row == 'stage-a' else
                     'stage-b-nix' if args.row.startswith('nix-') else
                     'stage-b-workload' if args.row == 'workload' else 'stage-b')
        guest = json.loads((ROOT / 'artifacts' / directory / 'guest.json').read_text())
        remote_guest = REMOTE + '/artifacts/' + directory + '/guest.json'
        for key in ('runtime', 'initramfs'):
            target = str(Path(remote_guest).parent / Path(guest[key]['path']).name)
            upload(Path(guest[key]['path']), target)
            guest[key]['path'] = target
        python(f'from pathlib import Path; Path({remote_guest!r}).write_text({json.dumps(guest)!r})')
        script = ('workload_smoke.py' if args.row.endswith('workload') else
                  'ssh_smoke.py' if args.row.endswith('ssh') else 'smoke.py')
        command = ['python3', REMOTE + '/' + script, '--guest', remote_guest, '--out', remote_out]
        if args.row == 'keeper':
            command.append('--keeper')
        if args.row == 'workload' or args.row.startswith('nix-'):
            source = 'nix' if args.row.startswith('nix-') else 'workload'
            for key, filename in [('workload', 'toolchain.img'), ('seed', 'data.seed.img')]:
                target = REMOTE + '/artifacts/' + source + '/' + filename
                upload(ROOT / 'artifacts' / source / filename, target)
                command.extend(['--' + key, target])
            upload(ROOT / 'artifacts' / source / 'build.json', REMOTE + '/artifacts/' + source + '/build.json')
        if args.trace:
            lima('shell', 'kernel-build', 'mkdir', '-p', REMOTE + '/traces')
            command = ['strace', '-ff', '-s', '160', '-o', trace_prefix, *command]
        # A fixed Python exec avoids introducing a shell parser for paths/argv.
        code = f'import os; os.chdir({REMOTE!r}); os.execvp({command[0]!r}, {command!r})'
        with (args.out / 'controller.stdout.log').open('wb') as stdout, (args.out / 'controller.stderr.log').open('wb') as stderr:
            result = subprocess.run(['limactl', 'shell', 'kernel-build', 'python3', '-c', code],
                                    env=env, stdout=stdout, stderr=stderr, timeout=600)
        receipt['exit_code'] = result.returncode
        lima('copy', '-r', 'kernel-build:' + remote_out, args.out / 'raw')
        report = json.loads((args.out / 'raw/report.json').read_text())
        receipt['status'] = report['status']
        if report['helper_artifact']['sha256'] != receipt['helper_sha256']:
            raise ValueError('remote helper differs from uploaded binary')
        if args.trace:
            archive = REMOTE + '/traces/' + run_name + '.tar.gz'
            python(f'import glob,tarfile; files=glob.glob({trace_prefix!r}+".*"); '
                   f't=tarfile.open({archive!r},"w:gz"); [t.add(p,arcname=p.rsplit("/",1)[1]) for p in files if p.rsplit(".",1)[1].isdigit()]; t.close()')
            lima('copy', 'kernel-build:' + archive, args.out / 'traces.tar.gz')
    except Exception as error:
        force_stop = isinstance(error, subprocess.TimeoutExpired)
        receipt.update(status='failed', error=f'{type(error).__name__}: {error}')
    finally:
        if started and (not args.keep_running or force_stop):
            try:
                lima('stop', 'kernel-build')
                receipt['builder_stopped'] = True
            except Exception as error:
                receipt.update(status='failed', builder_stop_error=str(error))
        else:
            receipt['builder_kept_running'] = started
        (args.out / 'nested.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(receipt, indent=2))
    raise SystemExit(receipt['status'] != 'passed')


if __name__ == '__main__':
    main()
