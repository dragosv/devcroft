## ADDED Requirements

### Requirement: The runtime shell runs in its own process with no ambient startup
The runtime SHALL contain a shell built on `brush_core`. Each use SHALL run in a
dedicated child process started from the `devcroft` binary; project-controlled
shell code SHALL never run inside the keeper or another long-lived devcroft
process. The runtime shell SHALL NOT evaluate profile files, rc files, files
named by `ENV` or `BASH_ENV`, or brush's own configuration file, except as the
interactive requirement below allows. When invoked through a link named `sh`,
it SHALL apply POSIX `sh` semantics.

#### Scenario: No ambient startup
- **WHEN** the sandbox `HOME` contains a `.bashrc` that writes a marker, `BASH_ENV` names a file that writes a marker, and the runtime shell runs `-c true`
- **THEN** no marker is written

#### Scenario: Invoked as sh
- **WHEN** the runtime shell is invoked as `sh`
- **THEN** POSIX mode is on

### Requirement: The runtime shell is the last step of shell resolution
The shell resolved for a sandbox that is not in native mode SHALL be, in order:
a shell on the provider environment's `PATH` inside the sandbox's grants; else a
shell from the closure's requisites; else the runtime shell. No host shell,
host `/bin` or `/var/select` SHALL be granted to obtain a shell. The resolved
shell SHALL be the namespace entry for `sh`, so `/bin/sh` and `sh` run it.

#### Scenario: A pixi environment without bash
- **WHEN** `up` brings up a pixi sandbox whose environment installs no shell
- **THEN** `sh` and `/bin/sh` record `runtime:gen-N`
- **AND** no host shell path appears in `policy --render`

#### Scenario: A flox environment
- **WHEN** `up` brings up a flox sandbox whose environment declares no shell
- **THEN** `/bin/sh` is the bash from the closure's requisites, not the runtime shell

#### Scenario: Declaring a shell in the environment takes precedence
- **WHEN** a pixi project adds `bash` from conda-forge to its environment
- **THEN** `/bin/sh` and `sh` resolve to that bash

### Requirement: The runtime shell never displaces a shell the environment supplies
The runtime shell SHALL NOT be used for `/bin/sh`, for `sh` by name, for
devcroft hooks or for services in a sandbox whose environment or closure
supplies a shell.

#### Scenario: A nix sandbox keeps stdenv's bash
- **WHEN** a nix sandbox executes `/bin/sh -c 'echo $BASH_VERSION'`
- **THEN** the closure's bash prints its version

### Requirement: The runtime shell is the interactive fallback
`devcroft shell` and SSH login sessions SHALL use the runtime shell when no
shell is selected through `$SHELL` under the existing `exec` contract. An
interactive runtime shell SHALL keep its history under the sandbox `HOME`,
SHALL save it when the client disconnects and when the sandbox is stopped with
`down`, within a bounded time, and SHALL fall back to basic line input on
terminals that do not answer cursor-position queries.

#### Scenario: History survives a disconnect
- **WHEN** a user runs a command in an SSH session and the client disconnects without `exit`
- **THEN** the next session's history contains that command

#### Scenario: A pty without a terminal emulator
- **WHEN** an interactive runtime shell starts on a pty that never answers a cursor-position query
- **THEN** it starts with basic line input instead of hanging

### Requirement: Known runtime-shell defects are published until qualified
For the pinned brush revision, devcroft SHALL keep a record of each of these
behaviours as qualified or not: signal handlers installed with `trap` run on
real signals; `trap '' SIG` ignores the signal; `$!` holds the background
process identity; `wait PID` returns that process's status; `kill PID` sends
`TERM`; POSIX mode applies when invoked as `sh`; startup time meets a declared
budget. Every behaviour not yet qualified SHALL be listed in the published
known gaps for sandboxes whose `/bin/sh` is the runtime shell, with the remedy
of declaring a shell in the environment.

#### Scenario: A defect is visible where it applies
- **WHEN** `up` brings up a sandbox whose `/bin/sh` is the runtime shell and trap delivery is not yet qualified
- **THEN** `status` and the known-gaps documentation say that `trap` on real signals is not supported there, and how to declare a shell instead

### Requirement: Explicit interpreters are never redirected to the runtime shell
A request for a specific interpreter — `/bin/bash` by absolute path, `bash` by
name, `#!/usr/bin/env bash` — SHALL NOT be served by the runtime shell.
`/bin/bash` SHALL NOT be a base name.

#### Scenario: env bash reaches the environment's bash
- **WHEN** a nix sandbox runs a script starting `#!/usr/bin/env bash`
- **THEN** the closure's bash interprets it

#### Scenario: No bash anywhere
- **WHEN** a pixi sandbox without bash runs a script starting `#!/bin/bash`
- **THEN** it fails, and no runtime shell or host bash runs it
