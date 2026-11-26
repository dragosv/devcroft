"""Measure temp path interposition under a Seatbelt profile on macOS."""

import argparse
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import tempfile
import time

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]

PYTHON_PROBE = """
import os, tempfile
for directory in (None, '/tmp', '/private/tmp'):
    with tempfile.NamedTemporaryFile(dir=directory) as f:
        f.write(b'python')
        f.flush()
        assert open(f.name, 'rb').read() == b'python'
        assert os.stat(f.name).st_mode & 0o777 == 0o600
        physical = os.path.join(os.environ['DC_TEMP'], os.path.basename(f.name))
        assert os.path.samestat(os.fstat(f.fileno()), os.stat(physical))
"""

NODE_PROBE = """
const fs = require('fs'), os = require('os');
for (const directory of [os.tmpdir(), '/tmp', '/private/tmp']) {
    const dir = fs.mkdtempSync(directory + '/devcroft-local-temp-node-');
    fs.writeFileSync(dir + '/file', 'node', { mode: 0o600 });
    if (fs.readFileSync(dir + '/file', 'utf8') !== 'node') throw Error('contents');
    if ((fs.statSync(dir + '/file').mode & 0o777) !== 0o600) throw Error('mode');
    const physical = process.env.DC_TEMP + '/' + dir.split('/').at(-1);
    if (fs.statSync(dir).ino !== fs.statSync(physical).ino) throw Error('location');
    if (fs.realpathSync.native(dir) !== physical) throw Error('native realpath');
    fs.rmSync(dir, { recursive: true });
}
"""

C_LOOP = """
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc != 4) return 2;
    struct stat info;
    char path[1024];
    int count = atoi(argv[3]);
    for (int i = 0; i < count; ++i) {
        const char *dir = i % 2 == 0 ? argv[1] : argv[2];
        snprintf(path, sizeof path, "%s/bench-%d", dir, i);
        int fd = open(path, O_CREAT | O_WRONLY | O_EXCL, 0600);
        if (fd < 0 || write(fd, "temp", 4) != 4 || close(fd) != 0 ||
            stat(path, &info) != 0 || info.st_size != 4 || unlink(path) != 0) {
            perror(path);
            return 1;
        }
    }
}
"""


def invoke(command, *, work, env=None, timeout=120):
    return subprocess.run(
        [str(arg) for arg in command], cwd=work, env=env,
        text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout,
    )


def require(result):
    if result.returncode:
        raise RuntimeError(result.stdout.strip())
    return result.stdout


def measure(name, commands, work, iterations):
    timings = [[], []]
    for turn in range(iterations + 1):
        for index in (0, 1) if turn % 2 == 0 else (1, 0):
            command, env, clear = commands[index]
            if clear:
                shutil.rmtree(clear, ignore_errors=True)
            start = time.perf_counter()
            require(invoke(command, work=work, env=env))
            if turn:
                timings[index].append(time.perf_counter() - start)
    baseline, prototype = map(statistics.median, timings)
    print(f"{name}: {baseline:.4f}s vs {prototype:.4f}s "
          f"({(prototype / baseline - 1) * 100:+.1f}%)", flush=True)
    print(f"  baseline: {[round(t, 4) for t in timings[0]]}", flush=True)
    print(f"  prototype: {[round(t, 4) for t in timings[1]]}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for tool in ("bash", "coreutils", "python", "node", "process-compose", "git", "initdb"):
        parser.add_argument("--" + tool, type=Path, required=True)
    parser.add_argument("--go", default="go")
    parser.add_argument("--cargo", default="cargo")
    parser.add_argument("--devenv", default="devenv")
    parser.add_argument("--ssh", default="/usr/bin/ssh")
    parser.add_argument("--mysql-install-db", type=Path)
    parser.add_argument("--skip-bench", action="store_true")
    args = parser.parse_args()

    failures = []

    def case(label, run, expected=0):
        result = run()
        success = result.returncode == expected
        print(f"{label}: {'PASS' if success else 'FAIL'} (exit {result.returncode})", flush=True)
        lines = result.stdout.rstrip().splitlines()
        if success and len(lines) > 24:
            print("\n".join(lines[:24]) + f"\n… {len(lines) - 24} more lines", flush=True)
        elif lines:
            print(result.stdout.rstrip(), flush=True)
        if not success:
            failures.append(label)

    with (
        tempfile.TemporaryDirectory(prefix="dc-mt-", dir="/private/tmp") as location,
        tempfile.TemporaryDirectory(prefix="dc-host-", dir="/private/tmp") as host_dir,
    ):
        work = Path(location)
        logical = "/tmp/" + work.name + "-literal"
        temporary = work / "temp"
        temporary.mkdir(mode=0o700)
        (work / "home").mkdir()
        (work / "state").mkdir()
        (work / "temp" / Path(logical).name).mkdir()
        lib = work / "temp.dylib"
        require(invoke(
            ["clang", "-Wall", "-Wextra", "-Werror", "-O2", "-dynamiclib",
             "-o", lib, HERE / "interpose.c"], work=work,
        ))
        exec_lib = work / "exec.dylib"
        require(invoke(
            ["clang", "-Wall", "-Wextra", "-Werror", "-O2", "-dynamiclib",
             "-o", exec_lib, REPO / "src/runtime/redirect.c"], work=work,
        ))
        namespace = work / "ns"
        namespace.mkdir()
        (namespace / "sh").symlink_to(args.bash)
        build_env = os.environ | {"GOCACHE": str(work / "gocache"), "CGO_ENABLED": "0"}
        require(invoke([args.go, "build", "-o", work / "go-probe", HERE / "probe.go"],
                       work=work, env=build_env))

        for filename in ("devenv.nix", "devenv.yaml", "devenv.lock"):
            shutil.copy(REPO / "samples" / "devenv-sample" / filename, work / filename)
        hook = json.loads(require(invoke([args.devenv, "--offline", "eval", "enterShell"],
                                         work=work)))["enterShell"]
        (work / "enter-shell").write_text(hook)

        profile = work / "sandbox.sb"
        profile.write_text(
            '(version 1)\n(allow default)\n'
            '(deny file-read* file-write* (subpath "/private/tmp"))\n'
            '(allow file-read-metadata (literal "/private/tmp"))\n'
            f'(allow file-read* file-write* (subpath "{work}"))\n'
        )
        original = {
            "PATH": f"{args.coreutils}:{args.bash.parent}:/usr/bin:/bin",
            "HOME": str(work / "home"),
            "TMPDIR": str(temporary),
            "DEVENV_DOTFILE": str(work / ".devenv"),
            "DEVENV_PROFILE": str(work / "profile"),
            "DEVENV_STATE": str(work / "state"),
            "DC_WORK": f"/{work.name}",
            "DC_TEMP": str(temporary),
            "DC_TAG": work.name,
            "DEVCROFT_NS": str(namespace),
            "LC_ALL": "C",
        }
        injected = original | {"DYLD_INSERT_LIBRARIES": str(lib), "DC_TRACE": "1"}

        def sandbox(command, env=injected, seatbelt=profile):
            # env receives the dyld variable as an argument after Apple strips
            # it from sandbox-exec and env themselves.
            return invoke(
                ["/usr/bin/sandbox-exec", "-f", seatbelt, "/usr/bin/env", "-i",
                 *(f"{key}={value}" for key, value in env.items()), *command],
                work=work, timeout=30,
            )

        sentinel = Path(host_dir) / "sentinel"
        sentinel.write_text("host\n")
        for path in (sentinel, Path("/tmp") / sentinel.parent.name / sentinel.name):
            case(f"host sentinel via {path}", lambda: sandbox(
                [args.bash, "-ec", 'read -r contents < "$1"', "host-probe", path], original),
                expected=1)
        assert sentinel.read_text() == "host\n"
        case("TMPDIR with no library", lambda: sandbox(
            [args.bash, "-ec", 'echo x > "$TMPDIR/control"'], original))
        case("devenv enterShell and subsequent .devenv/run access", lambda: sandbox(
            [args.bash, "-ec", 'source ./enter-shell; echo entry > .devenv/run/probe; '
             'test -s .devcroft-sample/enter-shell-ran']))
        case("python temp files", lambda: sandbox([args.python, "-c", PYTHON_PROBE]))
        case("node temp files", lambda: sandbox([args.node, "-e", NODE_PROBE]))
        case("go temp files and unix socket", lambda: sandbox([work / "go-probe"]))

        other_tmp = work / "other-temp"
        other_tmp.mkdir(mode=0o700)
        first_profile = work / "first.sb"
        second_profile = work / "second.sb"
        for seatbelt, hidden in ((first_profile, other_tmp), (second_profile, temporary)):
            seatbelt.write_text(profile.read_text() +
                                f'(deny file-read* file-write* (subpath "{hidden}"))\n')
        first = injected
        second = injected | {"DC_TEMP": str(other_tmp), "TMPDIR": str(other_tmp)}
        case("devenv second sandbox over the same checkout", lambda: sandbox(
            [args.bash, "-ec", 'source ./enter-shell; echo second > .devenv/run/probe'],
            second, second_profile))
        print(f"shared run symlink points to {os.readlink(work / '.devenv/run')}", flush=True)
        case("devenv first sandbox still finds its own run directory", lambda: sandbox(
            [args.bash, "-ec", 'read -r marker < .devenv/run/probe; test "$marker" = entry'],
            first, first_profile))

        (work / "process-compose.yaml").write_text(
            f"processes:\n  once:\n    command: \"echo ready > {logical}/pc-marker\"\n"
        )
        case("process-compose default literal log and child output", lambda: sandbox(
            [args.bash, "-ec", f'"$1" up --no-server -t=false; '
             f'test -s /tmp/process-compose-*.log; test -s {logical}/pc-marker',
             "pc", args.process_compose],
            injected | {"PC_SHELL_PATH": str(args.bash), "TMPDIR": "/tmp"}))
        case("postgres initdb literal data directory", lambda: sandbox(
            [args.initdb, "-D", logical + "/postgres", "-A", "trust", "--no-locale"],
            original | {"DYLD_INSERT_LIBRARIES": f"{lib}:{exec_lib}"}))
        if args.mysql_install_db:
            case("mysql literal temp", lambda: sandbox(
                [args.mysql_install_db, "--datadir=" + str(work / "mysql"),
                 "--tmpdir=" + logical]))
        else:
            print("mysql_install_db: SKIP (no executable supplied)", flush=True)

        case("git repository at a literal temp path", lambda: sandbox(
            [args.git, "init", logical + "/git"]))
        (temporary / Path(logical).name / "ssh-config").write_text("Host *\n  User devcroft\n")
        case("Apple ssh literal config refused", lambda: sandbox(
            [args.ssh, "-G", "-F", logical + "/ssh-config", "localhost"]), expected=255)
        case("ssh physical config control", lambda: sandbox(
            [args.ssh, "-G", "-F", temporary / Path(logical).name / "ssh-config", "localhost"],
            original))

        if not args.skip_bench:
            loop = work / "loop"
            (work / "loop.c").write_text(C_LOOP)
            require(invoke(["clang", "-O2", "-o", loop, work / "loop.c"], work=work))
            bench_env = os.environ | original | {"DYLD_INSERT_LIBRARIES": str(exec_lib)}
            bench_injected = bench_env | {"DYLD_INSERT_LIBRARIES": f"{lib}:{exec_lib}"}
            measure("file loop (4,000 iterations, half literal temp)",
                    [([loop, work, temporary / Path(logical).name, 4000], bench_env, None),
                     ([loop, work, logical, 4000], bench_injected, None)], work, 7)
            project = work / "cargo-project"
            sample = REPO / "samples" / "devbox-citytime-sample"
            project.mkdir()
            for filename in ("Cargo.toml", "Cargo.lock"):
                shutil.copy(sample / filename, project / filename)
            shutil.copytree(sample / "src", project / "src")
            cargo_target = work / "cargo-target"
            cargo_env = bench_env | {
                "PATH": f"{Path(args.cargo).parent}:{os.environ['PATH']}",
                "CARGO_TARGET_DIR": str(cargo_target),
            }
            command = [args.cargo, "build", "--offline", "--quiet"]
            measure("clean devbox-citytime-sample cargo build",
                    [(command, cargo_env, cargo_target),
                     (command, cargo_env | {
                         "DYLD_INSERT_LIBRARIES": f"{lib}:{exec_lib}",
                     }, cargo_target)],
                    project, 9)
    if failures:
        raise SystemExit("Failed probes: " + ", ".join(failures))


if __name__ == "__main__":
    main()
