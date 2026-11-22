# Locked pixi environment with a native extension

A Conda-only pixi workspace: Python 3.12 and numpy from conda-forge, locked
for **macOS ARM64** and **Linux glibc ARM64**. Those are the only qualified
runtimes; x64 and musl hosts are refused. Install pixi **0.84.2 or later** on
the host. A newer release draws a warning until it is qualified.

```sh
devcroft up
devcroft exec -- python check.py
devcroft policy --render
devcroft status
devcroft rm --yes
```

`up` and `status` print:

```text
guarantee: artifact (identical downloaded artifacts; runtime links against host libraries, so behavior depends on this host)
```

## What happens at `up`

1. devcroft reads `pixi.lock` before running pixi. It refuses:
   - a lock format other than version 7;
   - a missing environment or platform;
   - any PyPI package: installing one runs the environment's own Python on
     the host, which was measured with pixi 0.84.2;
   - source packages;
   - any channel host other than `conda.anaconda.org` or `repo.prefix.dev`.

   The host restriction exists because the macOS pixi binary reads stored
   credentials from the keychain, and devcroft supports no authenticated
   channel.
2. pixi runs host-side, in an environment devcroft builds:
   - no user or system configuration;
   - an empty `HOME`;
   - only the proxy and CA variables forwarded.

   It installs a byte copy of `pixi.toml` and `pixi.lock` into
   `~/.local/share/devcroft-artifacts/pixi/<object-id>/` with
   `install --locked`, and devcroft checks that both are unchanged. Your
   project's `.pixi/` directory is never read, written or removed.
3. pixi emits the activation as text. That form was measured not to run
   anything. devcroft rewrites its staging paths to this directory, then runs
   it once **inside the sandbox**: `scripts/activate.sh` and
   `[activation].env` are in effect, and `check.py` prints them.

Sandboxes with the same recipe share one read-only object, each with its own
home. The last `rm` reclaims it; `down` keeps it.

## What `policy --render` shows

Every grant is attributed to `provider:pixi`:

- the environment prefix, read-only;
- a POSIX shell: `/bin` and `/var/select` on macOS, or the shell's file and
  its glibc libraries on Linux.

On macOS, the system libraries and frameworks the environment links
(`/usr/lib`, `/System/Library`) are already part of devcroft's baseline, so
pixi adds nothing for them.

`/usr/bin` is never granted. A host `cc`, `gcc` or `git` does not run inside;
a compiler comes from the environment (`c-compiler`) or not at all. The
manifest grants only this project for writing.

On Linux, `up` warns that tk's libraries need `libX11`, which is outside that
inventory. It is not granted, so tkinter will not load there. Nothing else in
the sample is affected. macOS prints no such warning.

## What is not available

- **Tasks.** `pixi run check` is unavailable inside: the host's pixi binary is
  not granted. The `[tasks]` entry is deliberately not carried. Run
  `python check.py` directly.
- **PyPI dependencies, editable or path packages, and the default
  `pixi init --format pyproject` layout.** All of these are refused, and the
  refusal says why.
- **Services.** pixi has no service concept, so `[network.services]` under
  `provider = "pixi"` fails at `config`.

## Changing a dependency

Edit `pixi.toml`, then run `pixi lock` on a host that can solve for both
platforms. Review the lock and run `devcroft up --recreate`. Provisioning
downloads the locked packages using the host network; the sandbox's runtime
has no network access.
