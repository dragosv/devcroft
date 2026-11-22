# The base runtime: shell and userland

The **base runtime namespace** is the shell and the basic utilities a sandbox
can run — by name through its `PATH`, and by absolute path (`/bin/sh`,
`#!/usr/bin/env`, `system()`, `popen()`). devcroft records, for every sandbox,
where each of them resolves, so you can see whether a sandbox is reproducible
at that level.

The goal this serves: every shell or utility a sandbox can execute comes from
a pinned source — the provider's lock, or (once it exists) a devcroft runtime
the project pins — and never from the host, except named exceptions. This page
documents what is recorded today. The runtime itself is planned
(`openspec/changes/add-own-userland`, `add-macos-exec-redirect`,
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

## Provenance

`up` records one of these for every name and lookup form:

| Provenance | Meaning |
|---|---|
| `closure:<path>` | inside the provider's closure (the Nix store), pinned by its lock |
| `provider:<name>:<path>` | inside an artifact provider's own install (mise, pixi) |
| `host:<path>` | a host file — not reproducible at this level |
| `absent (not present)` | nothing by that name on `PATH`, or no file at that path |
| `absent (refused by policy)` | the file exists on the host and the policy does not grant it (macOS) |
| `absent (not in the view)` | the file exists on the host and the sandbox's view does not contain it (Linux) |

It is computed from what the sandbox is actually given — its `PATH`, its
resolved grants, and on Linux the entries its view writes — without spawning
anything, and a test runs every name in both forms inside a real sandbox to
check that the record matches.

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
| swift | the host's (declared by the provider) | the host's `/usr/bin` (declared) |

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
same file as the view's `/bin/sh`. On macOS, every absolute path is refused unless
granted, so `#!/bin/sh` and `#!/usr/bin/env` scripts fail — see
[known-gaps.md](known-gaps.md).
