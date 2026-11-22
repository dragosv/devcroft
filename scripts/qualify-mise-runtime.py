#!/usr/bin/env python3
"""Restore real Node/Go HTTP artifacts, then optionally exercise compiled policy.

Uses only upstream binary archives and verifies lock checksums against upstream
checksum metadata. Nothing from the installed environment runs until the policy
helper executes it. --target permits lock-generation probes for another target;
--helper is only valid on the native target. The root must be a fresh directory.
"""
import argparse
import hashlib
import importlib.util
import json
import platform
from pathlib import Path
import shlex
import subprocess
import tomllib
import urllib.request

spec = importlib.util.spec_from_file_location("install_probe", Path(__file__).with_name("qualify-mise-install.py"))
install_probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(install_probe)


def fetch(url):
    with urllib.request.urlopen(url, timeout=60) as response:
        return response.read()


def invoke(binary, args, cwd, env):
    result = subprocess.run([str(binary), *args], cwd=cwd, env=env,
                            capture_output=True, text=True, timeout=240)
    if result.returncode:
        raise RuntimeError(f'{args}: {result.stderr}')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mise", type=Path)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--target")
    parser.add_argument("--helper", type=Path)
    parser.add_argument("--reuse", action="store_true", help="reuse a previously prepared probe directory")
    args = parser.parse_args()
    binary = args.mise.resolve(strict=True)
    native = ("macos" if platform.system() == "Darwin" else "linux") + "-" + {
        "arm64": "arm64", "aarch64": "arm64", "x86_64": "x64",
    }[platform.machine()]
    target = args.target or native
    if args.helper and target != native:
        parser.error("policy execution requires the native target")
    root = args.root.resolve()
    root.mkdir(parents=True, exist_ok=args.reuse)
    environment = root / "environment"
    environment.mkdir(exist_ok=args.reuse)
    env = install_probe.isolated_env(environment)
    env["MISE_LOCKED"] = "0"  # lock creation only; install below is strict.
    env["MISE_LOCKFILE"] = "1"
    os_name, arch = target.split("-")
    node_os = "darwin" if os_name == "macos" else "linux"
    node_file = f'node-v24.0.0-{node_os}-{arch}.tar.gz'
    node_url = f'https://nodejs.org/dist/v24.0.0/{node_file}'
    node_checksums = fetch('https://nodejs.org/dist/v24.0.0/SHASUMS256.txt').decode()
    node_hash = next(line.split()[0] for line in node_checksums.splitlines() if line.split()[-1] == node_file)
    go_arch = "amd64" if arch == "x64" else "arm64"
    go_file = f'go1.24.0.{node_os}-{go_arch}.tar.gz'
    releases = json.loads(fetch('https://go.dev/dl/?mode=json&include=all'))
    go_hash = next(f['sha256'] for r in releases if r['version'] == 'go1.24.0'
                   for f in r['files'] if f['filename'] == go_file)
    go_url = f'https://dl.google.com/go/{go_file}'
    config = ('[tools]\n'
              f'"http:node" = {{ version = "24.0.0", url = "{node_url}" }}\n'
              f'"http:go" = {{ version = "1.24.0", url = "{go_url}" }}\n')
    # Verify the leading-dot spelling pairs with mise.lock as well.
    (environment / ".mise.toml").write_text(config)
    version = invoke(binary, ["--version"], environment, env).stdout.strip()
    if not args.reuse:
        invoke(binary, ["lock", "--platform", target], environment, env)
    lock_path = environment / "mise.lock"
    lock_before = lock_path.read_bytes()
    lock = tomllib.loads(lock_before.decode())
    for tool, expected in [("http:node", node_hash), ("http:go", go_hash)]:
        entry = lock['tools'][tool][0]
        actual = entry[f'platforms.{target}']['checksum']
        if actual != 'sha256:' + expected:
            raise RuntimeError(f'{tool}: lock checksum differs from upstream: {actual}')
    if target != native:
        result = {'mise_version': version, 'target': target, 'native_target': native,
                  'method': 'cross-target lock generation only; no installation or execution',
                  'manifest_name': '.mise.toml', 'lock': lock,
                  'upstream_sha256': {'node': node_hash, 'go': go_hash}}
        (root / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps({'target': target, 'method': result['method']}, indent=2))
        return
    env["MISE_LOCKED"] = "1"
    installed = invoke(binary, ["install", "--locked"], environment, env)
    if lock_path.read_bytes() != lock_before or (environment / ".mise.toml").read_text() != config:
        raise RuntimeError('strict install changed input bytes')
    capture = json.loads(invoke(binary, ["env", "--json"], environment, env).stdout)
    tool_paths = [Path(p) for p in capture['PATH'].split(':') if Path(p).resolve().is_relative_to(environment)]
    executables = {tool: next((p / tool).resolve() for p in tool_paths if (p / tool).is_file())
                   for tool in ['node', 'go']}
    result = {'mise_version': version, 'target': target, 'native_target': native,
              'mise_binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'lock_unchanged': True, 'manifest_name': '.mise.toml',
              'upstream_sha256': {'node': node_hash, 'go': go_hash},
              'lock': lock, 'installation_stderr': installed.stderr,
              'policy_probe': None}
    if args.helper:
        project = root / 'project'
        project.mkdir(exist_ok=args.reuse)
        for directory in ['home', '.cache/go', '.tmp']:
            (project / directory).mkdir(parents=True, exist_ok=args.reuse)
        (project / 'go.mod').write_text('module example.com/misequalification\n\ngo 1.24.0\n')
        (project / 'main.go').write_text('package main\nimport "fmt"\nfunc main() { fmt.Println("mise-build-ok") }\n')
        shell = Path('/bin/sh').resolve()
        grants = [str(environment / 'data/installs')]
        grants += ['/bin', '/var/select'] if platform.system() == 'Darwin' else [str(shell)]
        # Official Node 24 consults the host OpenSSL config at startup. This
        # provider dependency is measured explicitly, never a broad /etc grant.
        if platform.system() == 'Linux' and Path('/etc/ssl/openssl.cnf').is_file():
            grants.append('/etc/ssl/openssl.cnf')
        runtime_env = {'PATH': ':'.join(str(p) for p in tool_paths), 'HOME': str(project / 'home'),
                       'GOCACHE': str(project / '.cache/go'), 'GOTMPDIR': str(project / '.tmp'),
                       'GOENV': 'off', 'GOTOOLCHAIN': 'local', 'CGO_ENABLED': '0',
                       'GOROOT': str(executables['go'].parent.parent)}
        script = 'set -eu\n'
        for tool in ['cc', 'gcc', 'ld', 'git']:
            script += f'if command -v {tool}; then echo HOST_TOOL_RESOLVED; exit 40; fi\n'
            if Path('/usr/bin', tool).is_file():
                script += f'if /usr/bin/{tool} --version; then echo HOST_TOOL_EXECUTED; exit 42; fi\n'
        script += f'{shlex.quote(str(executables["node"]))} -e \'console.log("node-runtime-ok")\'\n'
        script += f'{shlex.quote(str(executables["go"]))} build -buildvcs=false -o app .\n./app\n'
        script += f'if (printf changed >> {shlex.quote(str(executables["node"]))}); then echo STORE_WRITABLE; exit 41; fi\n'
        script += 'printf "policy-probe-ok\\n"\n'
        (project / 'probe.sh').write_text(script)
        request = {'grants': grants, 'env': runtime_env, 'argv': [str(shell), str(project / 'probe.sh')]}
        (root / 'request.json').write_text(json.dumps(request, indent=2) + '\n')
        child = subprocess.run([str(args.helper.resolve()), str(project), str(root / 'request.json')],
                               capture_output=True, text=True, timeout=180)
        result['policy_probe'] = {'exit_code': child.returncode, 'stdout': child.stdout, 'stderr': child.stderr,
                                  'request': request}
    # Raw native installation paths are necessary to reproduce grants locally.
    (root / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'target': target, 'lock_unchanged': True, 'policy_probe': result['policy_probe']}, indent=2))


if __name__ == '__main__':
    main()
