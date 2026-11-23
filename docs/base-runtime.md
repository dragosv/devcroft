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
every sandbox's `PATH` by name, and on macOS by absolute path too
(`openspec/changes/add-macos-exec-redirect`); the absolute-path namespace on
Linux and a runtime shell are planned (`add-own-userland`,
`embed-brush-runtime`).

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

## The runtime (generation 1)

devcroft carries its own utilities, linked into the `devcroft` binary and run
when it is invoked through a link named after one of them (`ls`, `env`, ...).
They are where a sandbox's environment supplies nothing: the provider's own
executables always come first.

| Generation | Upstream | Version | Utilities |
|---|---|---|---|
| 1 | [uutils coreutils](https://github.com/uutils/coreutils) (MIT) | 0.13.0 | `env ls cat cp mv rm mkdir rmdir ln touch head tail wc sort uniq cut tr echo printf test pwd true false seq basename dirname readlink realpath stat chmod mktemp tee date id uname hostname yes sleep nohup` |

The runtime has no shell yet, and no `grep`, `sed`, `awk`, `find`, `xargs`,
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

## The host shell, until the runtime has one

An environment that resolves no POSIX shell gets the host's, from one place
for every provider (`src/runtime/layer.rs`): a link
`.devcroft/<name>/host-shell/sh`, so the sandbox's `PATH` gains `sh` and
nothing else from the host, and grants with their own origin, `host-shell`,
in `policy --render`. On Linux those are the shell file and the C library
files it loads; on macOS, `/bin` and `/var/select`. The shell is recorded
`host:` and `up` warns about it. Today that is mise and pixi environments
that install no shell.

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
| mise, pixi | the environment's own `sh` if any, else the host's | only what the environment installs |
| swift | the host's (native mode) | the host's `/usr/bin` (native mode) |

Since generation 1, every name the environment does not supply and the
runtime carries resolves to the runtime by name (`runtime:gen-1`), after the
environment's own executables and the shell's directory. The table above is
what the environment supplies before that.

On Linux (measured 2026-11-22, aarch64) the same holds by name for nix,
devbox and flox, and devenv supplies the full GNU userland as nix does, even
for a project that declares only two unrelated packages. `hostname` is absent
everywhere: coreutils does not ship it. mise and pixi sandboxes resolve `sh`
(and, for mise, `env`) to the host's, granted by the provider, and are the
only ones whose `up` prints the host warning.

By absolute path: on Linux, the view writes `/bin/sh` (the resolved shell)
and `/usr/bin/env` (only where the environment's `PATH` has an `env` inside
a provider grant, see [known-gaps.md](known-gaps.md)); everything
else is absent unless granted. On a merged-`/usr` host `/usr/bin/sh` is the
same file as the view's `/bin/sh`. On macOS, outside native mode,
an exec of a base name's absolute path is redirected to what the name resolves
to by name; the programs that lose the redirect are listed in
[known-gaps.md](known-gaps.md).
