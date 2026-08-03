# Change: add-mvp-core

## Why

No tool today offers persistent, SSH-reachable dev environments over
process-level sandbox primitives. Every existing tool with an SSH endpoint
gets there via a container or microVM, because starting sshd inside one is
trivial — while Landlock/Seatbelt have no `exec into` primitive. This change
builds the missing piece: a keeper process that lives inside the sandbox
boundary and spawns sessions on demand, plus the declarative config and CLI
around it.

Scope is deliberately cut to one backend (nono) and one provider (flox).
Reproducibility is mandatory: there is no passthrough provider. Multi-backend abstraction is the most expensive and most easily
commoditized part of the design; it is deferred until the core is proven.

## What Changes

- New capability `config`: `devcroft.toml` manifest, parsing, validation.
- New capability `env-provider`: flox activation with the fixed
  composition order (env inside, sandbox outside); reproducibility
  enforced — no passthrough.
- New capability `policy`: compile manifest into a nono profile; render and
  explain compiled policy.
- New capability `lifecycle`: keeper process (spawn server), `up`, `down`,
  `status`, idempotent restart, orphan cleanup.
- New capability `exec`: one-shot `exec` and interactive `shell` sessions
  through the keeper.
- New capability `ssh`: embedded SSH endpoint on a unix socket, `proxy`
  subcommand for ProxyCommand, `ssh-config` emitter.
- New capability `cli`: command surface, naming resolution, `doctor`,
  `init`, exit codes.

## Impact

- Affected specs: all new (greenfield).
- Affected code: new crate, `src/` workspace.
- External deps: nono binary on PATH (checked by `doctor`), flox optional.
- No breaking changes (nothing exists yet).

## Success Criteria

- `devcroft init && devcroft up && devcroft ssh` works end to end on Linux
  (Landlock) and macOS (Seatbelt) with the flox provider.
- `ssh myproj.devcroft` from a stock OpenSSH client reaches a shell inside
  the sandbox after `devcroft ssh-config --write`.
- Two sandboxes on one host run concurrently without interfering.
- A denied file access is explainable via `devcroft why` in under a minute.
