## Why

**devcroft does not decide what is in a sandbox's environment. The user's shell
does.** Measured on a real session (macOS 15.7.4, flox provider):

- **180 variables** reach the sandbox.
- **101 of them pass through byte-identical from the invoking shell.** The
  provider's genuine contribution is 74 new plus 5 modified.

A sample of what arrives verbatim, from the shell that happened to run `up`:

```
CLAUDE_CODE_MESSAGING_TOKEN=fa94351fa93f0f5cdfc7e4d6ad6b341d
CLAUDE_CODE_MESSAGING_SOCKET=/tmp/cc-socks/80377.sock
SSH_AUTH_SOCK=/private/tmp/com.apple.launchd.Ph1HS1PJjm/Listeners
__CFBundleIdentifier=com.microsoft.VSCode
__MISE_SESSION=…
CLAUDE_PID=80377
```

There is **no environment filtering anywhere in `src/`** — no allowlist, no
denylist, no handling of sensitive names. So `AWS_SECRET_ACCESS_KEY`,
`GITHUB_TOKEN`, `NPM_TOKEN` and anything else the user exports are inside every
sandbox, by the same mechanism.

**That is asymmetric with what devcroft already does on disk, and the asymmetry
is the point.** `policy::compile` carefully baseline-denies `~/.aws`, `~/.ssh`,
`~/.config/gh` — and the credentials those directories hold flow freely through
the environment. One half of the credential boundary is enforced and the other
does not exist.

**Two specific consequences, both live.**

`[ssh] forward_agent` is parsed, validated, and has a scenario in
`add-mvp-core`'s `ssh` spec — *"no agent socket exists inside the sandbox"* —
and **no implementation anywhere in `src/`**. `SSH_AUTH_SOCK` reaches the
sandbox whatever it is set to. (Whether the socket is *reachable* depends on
filesystem and network policy and has not been measured; the manifest key does
nothing either way.)

`HOME` inside the sandbox is `/Users/dragos` — the host home, which the
baseline denies. Every tool that writes to `$HOME` fails there, confusingly,
and an agent's official installer cannot run at all.

**Why now, ahead of `adopt-nono-proxy`.** That change brokers one credential so
it never enters the sandbox. Doing that while 101 shell variables pass through
unfiltered is a vault beside an open window. Closing the environment is the
precondition that makes brokering worth its 66 crates — and it delivers the
simple version of the same guarantee immediately, with no new dependency.

## What Changes

- **NEW** `sandbox-environment`: devcroft decides what the sandbox's
  environment contains. Variables inherited from the invoking shell are removed
  unless a project declares otherwise; what the provider contributed stays.
- `[env] forward` — the declared escape hatch, and the simple credential path:
  `forward = ["ANTHROPIC_API_KEY"]` puts exactly that variable in, visibly, in
  a committed file.
- **`HOME` becomes sandbox-local**, under the existing per-sandbox artifact
  directory, so tools that write to it work and `[hooks]` can install things.
- `[ssh] forward_agent` gains an implementation, or is removed. A manifest key
  that does nothing is worse than an absent one.
- **BREAKING for some projects**, and deliberately: a project relying on a
  variable it never declared will stop seeing it. That is the change.

## Capabilities

### New Capabilities

- `sandbox-environment`: what a sandbox's environment contains, where each
  variable comes from, how a project declares an exception, and what `HOME`
  points at.

### Modified Capabilities

- (none — `openspec/specs/` holds no synced specs. The `ssh` capability's
  agent-forwarding scenario lives in the unarchived `add-mvp-core`; this change
  makes it true rather than altering it.)

## Impact

- **Affected code**: `lifecycle::up` (the environment handed to `spawn_keeper`,
  and the existing `unset` channel it already uses for provider-removed keys),
  `config` (the `[env] forward` key), `services::artifact_dir` (a second
  consumer, for `HOME`).
- **This is a behaviour change users will notice**, which is why it needs a
  migration story rather than a flag: the failure mode is a tool that worked
  yesterday not finding a variable today. The error has to name the variable
  and the manifest key that would restore it.
- **It changes what devcroft can honestly claim.** Today the README and
  `docs/threat-model.md` describe filesystem confinement without mentioning
  that the environment is unfiltered. After this, the credential boundary is
  one story instead of two.
- **`adopt-nono-proxy` is unaffected and stays open.** Its `[[broker]]` surface
  is forward-compatible: once the environment is closed, a brokered route means
  "not even this variable enters", which is a claim worth making. Today it
  would not be.

## Non-Goals

- **Not a secrets manager.** `forward` names a variable the user already has;
  where it came from is their business. Brokering is the stronger answer and is
  its own change.
- **Not scrubbing the provider's own output.** What `flox activate` or `nix
  print-dev-env` produces is the environment the project declared, and devcroft
  does not second-guess it.
- **Not sandboxing `up` itself.** devcroft's own process still sees the host
  environment; that is what `sandbox-provisioning` is for.
