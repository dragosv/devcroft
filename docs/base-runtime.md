# The base runtime: shell and userland

The **base runtime namespace** is the shell and the basic utilities a sandbox
can run — by name through its `PATH`, and by absolute path (`/bin/sh`,
`#!/usr/bin/env`, `system()`, `popen()`). devcroft records, for every sandbox,
where each of them resolves, so you can see whether a sandbox is reproducible
at that level.

The goal this serves: every shell or utility a sandbox can execute comes from
a pinned source — the provider's lock, or (once it exists) a devcroft runtime
the project pins — and never from the host, except named exceptions. This page
documents what is recorded today. The runtime's utilities exist and are on
every sandbox's `PATH` by name and reached by absolute path; a runtime shell
is planned (`openspec/changes/embed-brush-runtime`).

## The base names (revision 1)

| Group | Names |
|---|---|
| Shell and `env` | `sh`, `env` |
| coreutils floor | `ls cat cp mv rm mkdir rmdir ln touch head tail wc sort uniq cut tr echo printf test pwd true false seq basename dirname readlink realpath stat chmod mktemp tee date id uname hostname yes sleep nohup` |
| Common userland | `grep sed awk find xargs diff tar gzip` |

Each name has two lookup forms:

- **By name**: the first executable of that name on the sandbox's `PATH` that
  the sandbox can see.
- **By absolute path**: `/bin/sh` for `sh`, `/usr/bin/env` for `env`, and
  `/bin/<name>` and `/usr/bin/<name>` for every other name.

The list changes only with a new revision, recorded here.

## The runtime (generation 2)

devcroft carries its own utilities, linked into the `devcroft` binary and run
when it is invoked through a link named after one of them (`ls`, `env`, ...).
They are where a sandbox's environment supplies nothing: the provider's own
executables always come first.

| Generation | Upstream | Version | Utilities |
|---|---|---|---|
| 1 | [uutils coreutils](https://github.com/uutils/coreutils) (MIT) | 0.13.0 | `env ls cat cp mv rm mkdir rmdir ln touch head tail wc sort uniq cut tr echo printf test pwd true false seq basename dirname readlink realpath stat chmod mktemp tee date id uname hostname yes sleep nohup` |
| 2 | the same, and [brush](https://github.com/reubeno/brush) (MIT) | 0.13.0; `brush-core` 0.5.0 | the same, and `sh` |

The runtime has no `grep`, `sed`, `awk`, `find`, `xargs`,
`diff`, `tar` or `gzip`: those come from the environment or not at all. uutils
is not GNU coreutils; where an option behaves differently, a project that
wants GNU declares `coreutils` in its environment and gets it.

A generation identifies the runtime's behaviour (its utilities, their
upstream version and the absolute-path rules), not the devcroft release, and
changes only when one of those does.

## Provenance

`up` records one of these for every name and lookup form:

| Provenance | Meaning |
|---|---|
| `closure:<path>` | inside the provider's closure (the Nix store), pinned by its lock |
| `provider:<name>:<path>` | inside an artifact provider's own install (mise, pixi) |
| `runtime:gen-<N>:<path>` | devcroft's runtime, at the generation the sandbox started with; `path` is the link whose name selects the utility |
| `native:<provider>:<path>` | a host file in a sandbox whose provider declares native mode (swift), on purpose and with a stated reason |
| `host:<path>` | a host file — not reproducible at this level |
| `absent (not present)` | nothing by that name on `PATH`, or no file at that path |
| `absent (refused by policy)` | the file exists on the host and the policy does not grant it (macOS) |
| `absent (not in the view)` | the file exists on the host and the sandbox's view does not contain it (Linux) |

It is computed from what the sandbox is actually given — its `PATH`, its
resolved grants, and on Linux the entries its view writes — without spawning
anything, and a test runs every name in both forms inside a real sandbox to
check that the record matches.

## The shell

A sandbox's shell is, in order: one on the environment's `PATH`, inside a
grant; one from the closure's requisites; the runtime's `sh`. The host's is
never granted. The runtime's is reached only where an environment has none,
which today is a mise or pixi project that installs none: `up` and `status`
say so there.

The runtime's shell is `brush_core` in a process of its own, always in POSIX
mode. It evaluates nothing before the command: no profile, no rc file, not
`ENV` or `BASH_ENV`, no brush configuration. It is not a complete POSIX shell;
[known-gaps.md](known-gaps.md) lists what it does not do, and the remedy is a
shell in the environment (pixi: `pixi add bash`). It never serves `bash`.

Qualification, `brush-core` 0.5.0, 2026-11-24, `spikes/brush-runtime/qualify.py`
against dash, the same on macOS arm64 and Linux aarch64:

| Behaviour | Qualified |
|---|---|
| POSIX mode as `sh`; `-c`, `-s`, `-e`, scripts, positional arguments, exit status | yes |
| `trap '' INT` | yes |
| a `trap` handler on a real `TERM`; `trap '' TERM`, `trap '' HUP` | no |
| `$!`; `wait PID`; `kill` with no signal, `kill -0` | no |
| startup, median, release | Linux 0.86 ms (a closure's bash 0.76, dash 0.17); macOS 5.5 ms (6.0, 1.8) |

## Native mode

One provider runs on the host's shell and userland on purpose: swift, whose
toolchain runs the host's `/bin/sh` (`xcrun` runs `sh -c` and reads
`/var/select/sh`) and whose signing tools live in `/usr/bin`. `up`, `status`
and `policy --render` say so with that reason, and its host files are
recorded `native:swift` rather than warned about one by one. No other
provider declares it.

A manifest cannot do the same one file at a time: a `filesystem.allow` or
`filesystem.read` entry naming a base name directly under a host `bin`
directory (`/usr/bin/grep`, `/bin/sh`) is refused at layer `config`, unless
the provider is native. To pin your own `grep`, add it to the environment.

## Where to see it

- `devcroft policy --render` lists every name and lookup form, then totals.
- `devcroft status` shows the totals and names every `host:` entry.
- `up` prints one warning when any name resolves to the host. It does not
  refuse to start.

## What a sandbox gets today

Measured 2026-11-22 on macOS arm64, by name, in real sandboxes:

| Provider | Shell | Userland by name |
|---|---|---|
| nix | bash from stdenv | GNU coreutils, grep, sed, gawk, findutils, diffutils, tar, gzip |
| devbox | bash from stdenv | the same, with or without a toolchain |
| flox | a bash from the closure's requisites | none |
| mise, pixi | the environment's own `sh` if any, else the runtime's | only what the environment installs |
| swift | the host's (native mode) | the host's `/usr/bin` (native mode) |

Since generation 1, every name the environment does not supply and the
runtime carries resolves to the runtime by name (`runtime:gen-2`), after the
environment's own executables and the shell's directory. The table above is
what the environment supplies before that.

On Linux (measured 2026-11-22, aarch64) the same holds by name for nix,
devbox and flox, and devenv supplies the full GNU userland as nix does, even
for a project that declares only two unrelated packages. `hostname` is absent
everywhere: coreutils does not ship it. No sandbox but swift's resolves a base name to the host.

By absolute path, a base name runs what it resolves to by name. On Linux the
view, a fleet agent's included, holds a link for each at `/bin` and
`/usr/bin`, and nothing else there but what a grant puts there. On macOS,
outside native mode, an exec of such a path is redirected. The exceptions on
both are in [known-gaps.md](known-gaps.md).

A process inside cannot modify what this rests on: `.devcroft/<name>/` is
read-only to it, except `home` (its `HOME`) and `run` (the service
supervisor's socket and log, an agent's report). Read-only mounts on Linux, a
write denial on macOS, listed as `filesystem.write_deny` by `policy --render`.
On Linux a process can still move `.devcroft` itself aside, since it sits in
the project: that takes the runtime off its own `PATH`, and the absolute paths
keep their targets. macOS refuses the move.
