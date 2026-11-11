#!/usr/bin/env python3
"""Remove remote trace duplicates only after byte-verifying their host archives."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile

from cold_prepare import require_no_active_collector
from smoke import ROOT


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
    archives = {}
    for archive in sorted((ROOT / 'runs').glob('*/traces.tar.gz')):
        run = archive.parent.name
        members = {}
        with tarfile.open(archive) as tar:
            for member in tar:
                if not member.isfile() or Path(member.name).name != member.name or not member.name.startswith(run + '.') or not member.name.rsplit('.', 1)[1].isdigit():
                    raise RuntimeError('unexpected retained trace archive member')
                with tar.extractfile(member) as file:
                    members[member.name] = {'size': member.size, 'sha256': hashlib.file_digest(file, 'sha256').hexdigest()}
        with archive.open('rb') as file:
            archives[run] = {'archive_sha256': hashlib.file_digest(file, 'sha256').hexdigest(), 'members': members}
    (args.out / 'archive-manifest.json').write_text(json.dumps(archives, indent=2) + '\n')
    report = {'schema': 1, 'status': 'failed', 'qualification': 'verified remote trace duplicate cleanup; host archives retained',
              'raw_evidence': str(args.out)}
    env = dict(os.environ, LIMA_HOME='/private/tmp/devcroft-kernel-builder')
    disk = Path('/private/tmp/devcroft-kernel-builder/kernel-build/disk')
    report['before_allocated_bytes'] = disk.stat().st_blocks * 512
    started = False

    def lima(name, *argv):
        with (args.out / f'{name}.stdout').open('wb') as stdout, (args.out / f'{name}.stderr').open('wb') as stderr:
            subprocess.run(['limactl', *map(str, argv)], env=env, check=True, stdout=stdout, stderr=stderr, timeout=600)

    try:
        lima('start', 'start', '--tty=false', 'kernel-build')
        started = True
        lima('manifest-copy', 'copy', args.out / 'archive-manifest.json', 'kernel-build:/home/dragos.guest/devcroft-trace-manifest.json')
        program = '''import hashlib,json,os,pathlib,stat
base=pathlib.Path('/home/dragos.guest/microvm-eval/traces')
manifest=json.loads(pathlib.Path('/home/dragos.guest/devcroft-trace-manifest.json').read_text()); removed=[]
def sha(path):
 h=hashlib.sha256()
 with path.open('rb') as f:
  offset=0
  while data:=f.read(2*1024**2):
   h.update(data); os.posix_fadvise(f.fileno(),offset,len(data),os.POSIX_FADV_DONTNEED); offset+=len(data)
 return h.hexdigest()
for run,record in manifest.items():
 archive=base/(run+'.tar.gz')
 if not archive.exists(): continue
 if archive.is_symlink() or sha(archive)!=record['archive_sha256']: raise RuntimeError('remote archive differs: '+run)
 for name,expected in record['members'].items():
  p=base/name
  if not p.exists(): continue
  info=p.lstat()
  if not stat.S_ISREG(info.st_mode) or info.st_size!=expected['size'] or sha(p)!=expected['sha256']: raise RuntimeError('remote trace differs: '+name)
  p.unlink(); removed.append({'path':str(p),'bytes':info.st_size})
 archive.unlink()
print(json.dumps({'removed':removed,'logical_bytes_removed':sum(x['bytes'] for x in removed)}))
'''
        lima('verify-and-remove', 'shell', 'kernel-build', 'python3', '-c', program)
        report['removed'] = json.loads((args.out / 'verify-and-remove.stdout').read_text())
        lima('trim', 'shell', 'kernel-build', 'sudo', 'fstrim', '-v', '/')
        lima('stop', 'stop', 'kernel-build')
        started = False
        report.update(status='passed', after_allocated_bytes=disk.stat().st_blocks * 512,
                      host_archives_retained=True, unmatched_traces_retained=True)
    except BaseException as error:
        report['error'] = f'{type(error).__name__}: {error}'
    finally:
        if started:
            try:
                lima('finally-stop', 'stop', 'kernel-build')
            except Exception as error:
                report['cleanup_error'] = str(error)
        (args.out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({key: report.get(key) for key in ('status', 'before_allocated_bytes', 'after_allocated_bytes', 'error')}), flush=True)


if __name__ == '__main__':
    main()
