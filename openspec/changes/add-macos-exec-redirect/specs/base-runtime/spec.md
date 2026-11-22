## ADDED Requirements

### Requirement: Absolute paths resolve as their names do on macOS
On macOS, in a sandbox that is not in native mode, an exec of a base-name path
(`/bin/sh`, `/usr/bin/env`, `/bin/<name>`, `/usr/bin/<name>`) SHALL run the
sandbox's namespace-directory entry for that name — the same executable the
name resolves to through the sandbox `PATH`. An exec of a script whose `#!`
interpreter is such a path SHALL run that entry with the interpreter argument,
the script path and the original arguments. This SHALL hold for `execve`,
`posix_spawn` and the library calls that reach them, including `system()` and
`popen()`. An exec of a base-name path without an entry, or of any other path,
SHALL be unaffected and SHALL remain subject to the sandbox policy.

#### Scenario: system() reaches the resolved shell
- **WHEN** a program in a closure-tier sandbox calls `system("echo ok")`
- **THEN** the sandbox's resolved shell runs it and `ok` is printed

#### Scenario: An absolute utility in a nix sandbox
- **WHEN** a nix sandbox on macOS executes `/bin/ls`
- **THEN** GNU coreutils' `ls` from the closure runs, the same as `ls` by name

#### Scenario: An absolute utility in a minimal flox sandbox
- **WHEN** a flox sandbox on macOS whose environment supplies no coreutils executes `/bin/ls`
- **THEN** the runtime's `ls` runs

#### Scenario: A shebang script
- **WHEN** a sandbox process executes a script starting `#!/bin/sh`
- **THEN** the sandbox's resolved shell interprets it

#### Scenario: An env shebang
- **WHEN** a sandbox process executes a script starting `#!/usr/bin/env python3`
- **THEN** the sandbox's `env` entry runs and executes `python3` found through `PATH`

#### Scenario: A path outside the namespace is not redirected
- **WHEN** a sandbox process executes `/usr/bin/make` or `/usr/bin/python3`, or `/bin/awk` in a sandbox where no source supplies `awk`
- **THEN** the exec fails with the policy's refusal and the host binary does not run

### Requirement: Every sandbox process starts with the redirect
The keeper SHALL make the redirect active in every process it starts inside a
macOS sandbox — sessions, `devcroft exec`, SSH sessions, devcroft hooks,
confined provider activation, and service processes — before that process
starts, through the same environment construction used for all of them.

#### Scenario: A hook gets it
- **WHEN** a `post_create` hook runs `initdb`, which uses `popen` on a shell command
- **THEN** the command runs through the sandbox's resolved shell

#### Scenario: Children inherit it
- **WHEN** a runtime utility or a closure binary started in a session executes `/bin/sh`
- **THEN** the sandbox's resolved shell runs

### Requirement: The redirect never falls back to the host
When the redirect is not active in a process — because the binary is
Apple-signed or uses the hardened runtime without permitting injected
libraries, or because the process cleared its environment — an exec of a base
runtime namespace path SHALL fail with the policy's refusal. devcroft SHALL NOT
grant host shells or utilities to compensate, except through native mode.
These limits SHALL be published.

#### Scenario: An Apple-signed tool
- **WHEN** `/usr/bin/make` or a binary from Xcode runs inside a non-native sandbox and executes `/bin/sh`
- **THEN** the exec fails with `EPERM` and no host shell runs

#### Scenario: A cleared environment
- **WHEN** a sandbox process runs `env -i /bin/sh -c true`
- **THEN** the inner exec fails with `EPERM` unless the runtime's `env` restored the redirect

### Requirement: The redirect is part of the runtime
The redirect library SHALL be built into the `devcroft` binary, written at `up`
beside the runtime, covered by a read grant with origin `runtime:gen-N` shown by
`policy --render`, and included in the runtime generation's identity. It SHALL
NOT be a security boundary and SHALL NOT be described as one.

#### Scenario: Inspectable
- **WHEN** a user runs `devcroft policy --render` on macOS
- **THEN** the library's path appears with origin `runtime:gen-N`
