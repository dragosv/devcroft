# Architecture invariants

These rules are load-bearing and non-obvious. Breaking one is a design error,
not a style issue. [AGENTS.md](../AGENTS.md) states each rule in a line or two;
this page keeps the reasoning and the measurements behind it. The original
rationale is in `openspec/changes/add-mvp-core/design.md`.

Code comments cite these rules by name, for example "CLAUDE.md's shell
invariant". `CLAUDE.md` imports `AGENTS.md`, which uses the same names and
links here.

## Listener-before-restriction ordering

Landlock and Seatbelt apply to a process tree and are inherited by children.
There's no API to join an existing sandbox from outside. So `up` creates the
Unix listener sockets first, spawns the keeper with those descriptors
inherited, and then the keeper applies the compiled profile to itself. The
sockets stay reachable from outside only because they predate the restriction.
This descriptor-passing step is the load-bearing risk of the whole design, and
the task order retired it first.

## Two-phase execution

Provider provisioning (package materialization and environment capture) runs
host-side at `up`, before restriction, using the host's own network.
Everything after restriction runs inside the boundary: sessions and
devcroft's own manifest hooks. The manifest's `[network]` and `[filesystem]`
sections govern runtime only, never provisioning. devcroft's `[hooks]` are
project code and never get provisioning privileges, so a hook that needs the
network needs an allowlist entry.

The provisioning phase is trusted because it runs pinned tooling from a
lockfile. That's a goal, not a guarantee, and the gap depends on the provider
(`fix-provisioning-hooks`). A provider's own activation hook is project code
too, and some providers can't produce an environment without running it.
Measured per provider:

- **nix:** fixed. `print-dev-env --json` returns the build environment as
  data, with `shellHook` as an inert string that's never evaluated. Plain
  `print-dev-env` is not a fix: the script it emits ends with
  `eval "${shellHook:-}"`. Never pass a bare flake path, because nix's default
  search falls through to `packages.<system>.default`, and a packaging-only
  flake used to start in its package's build environment. The shell is named
  in full, and its absence fails at layer `provider` (`require-nix-devshell`).
- **devbox:** fixed (`add-devbox-provider`). `shellenv --pure` doesn't run
  `init_hook` in any variant; `devbox run` does. devcroft uses the former.
- **flox:** fixed by devcroft, not by a flox flag. No `flox activate` mode
  suppresses `[hook].on-activate` (not `--mode run`, `--mode dev`, or
  `--no-start-services`). devcroft materializes from a derived, hook-free copy
  of the environment and runs the hook inside the sandbox
  (`flox::derive_hook_free_env`, `hooks::run_activation_script`). Stripping
  `[hook]` yields a byte-identical locked package set and the same store path,
  because a hook isn't a package input. `tests/flox_derived_env.rs` asserts
  this. As a consequence, a flox hook runs under the manifest's own policy, so a
  hook that reaches for host tooling is denied where it used to succeed. That's
  `own-policy-baseline` working as designed.
- **devenv:** the only provider that exposes both a hook-free environment
  (`devenv build shell`) and a separate handle on the hook
  (`devenv eval enterShell`). devcroft captures the hook as data and runs it
  inside.

Since 2026-10-12 (`sandbox-provisioning`), the hook's network isn't the
runtime's. `up` runs the activation script (flox's hook, devenv's
`enterShell`) in `__provision`, under the runtime filesystem policy and
`[provisioning.network] allow` alone, through a proxy of its own. The keeper
then adopts what the script exported. A hook that needs a registry declares
it there, and sessions never get it. Fleet agents do the same since
2026-10-15: the script runs in a short-lived agent of its own, in that agent's
leaf and view, before the keeper (`Supervisor::provision`). A script that
fails there fails `up` at layer `provider`, not `keeper`, and says whether the
policy refused it something (`lifecycle::attribution`). The policy decides
that, not the error text, because a confined hook sees a hidden path as plain
`ENOENT`.

What still runs host-side with the daemon is resolution itself: the evaluator
over repository-controlled definitions. It was measured not to work without
the daemon for any provider (`sandbox-provisioning` design.md P6, P6a), and
`policy --render` states it. `docs/flox-confined-activation-issue.md` is the
upstream request that would make flox's derived copy unnecessary.

**Rule for a new provider:** prefer the entry point that hands back an
environment over the one that runs a command inside it, because the latter
runs hooks in every provider measured so far. Where no such entry point
exists, report it; never let it pass silently.

## Environment resolves once, at up

Provider activation runs once and is captured as an environment diff, then
injected into the keeper. Sessions inherit it. Changing the provider's
manifest requires `up --recreate`. Don't propose per-session activation: it
would force the profile to grant provider internals forever.

## Provider resolution must not widen the policy

If activation would need write access outside the project root, `up` fails
and names the path instead of silently granting it.

An artifact-tier provider is host-linked by definition, so it can't inherit
library access from the baseline. It declares those grants itself, rendered
with a `provider:<name>` origin. The baseline stopped carrying host libraries
when `own-policy-baseline` measured that a full Rust build from a flox closure
needs only the project root, `/tmp`, `/nix/store` and 19 `/dev` and `/proc`
entries, with `/usr/bin/gcc` and `/bin/ls` denied.

## Policy is deterministic and inspectable

Manifest, provider grants and baseline compile byte-identically into
`<state>/<name>/profile.json`. Every rule carries an origin. A manifest writes
only `manifest:<key>`. devcroft writes the others:

| Origin | Rule |
|---|---|
| `manifest:<key>` | A key in `devcroft.toml` |
| `provider:<name>` | A grant the environment provider declares |
| `baseline` | devcroft's fixed rules |
| `allocated` | A port devcroft chose |
| `trust-store` | The CA bundle the provider's environment names |
| `pid-namespace` | A procfs the sandbox owns |
| `fleet:clone` | A fleet agent's clone root |
| `tools:<provider>` | The user's tooling layer |
| `identity` | The `/etc/passwd`, `/etc/group` and `/etc/hosts` devcroft writes into the view (`fleet::mount::Identity`); the view's `/bin` and `/usr/bin` links need no rule, because their targets have one |
| `runtime:gen-N` | devcroft's own utility directory (`add-own-userland`) |

The base-runtime record beside the policy has its own provenance kinds, among
them `runtime:gen-N` and `native:<provider>`. The latter exists only for the
one provider that declares native mode with a reason (swift), whose host
userland is stated once rather than warned about per name.

Nothing goes to the backend that `policy --render` can't show. Baseline
denials always win, including the denial of devcroft's own data directory.

## Scratch is scoped to the sandbox

Linux mounts its own `/tmp`. On macOS, Seatbelt grants a short 0700 directory
under `/private/tmp/dc-<uid>` and protects other sandboxes' directories, even
when the manifest opts in to the host's `/tmp`. The redirect library maps
literal temp paths for its processes. `TMPDIR` is the portable entry point,
even for binaries that lose the library. devenv capture uses per-sandbox
dotfile and runtime paths, so two keepers over one checkout never race over
`.devenv/run`. Project code uses `$DEVENV_RUNTIME` or `$DEVENV_DOTFILE/run`.

## SSH is reachable only on a Unix socket

The keeper embeds russh, listening on a 0600 socket in a 0700 state directory.
The filesystem permissions are the access boundary, not the process's
location. SSH exists for editor protocol compatibility, never for network
security. Clients reach it through `ProxyCommand devcroft proxy %n`.

This used to depend on the tier, because the removed hardened tier ran the SSH
server host-side and dispatched sessions through the backend's own exec-into
primitive. The invariant was the same in both cases, which is why removing the
tier didn't change it. A second backend would face the same choice.

## Degraded capabilities are surfaced, never silent

If the host can't enforce a requested aspect (for example, domain allowlists
on macOS Seatbelt), `up` prints exactly one warning that names the aspect, the
reason and the fallback. Never drop a capability quietly.

## devcroft's shell is devcroft's dependency, not the project's

SSH login sessions, `devcroft shell`'s fallback, and the command
process-compose runs each service through all need a POSIX shell. No real
flox, nix or devbox manifest declares one, because under a plain
`flox activate` the host's `PATH` still resolves `sh`. `own-policy-baseline`
removed host toolchain access without replacing it, so all three silently
began resolving to a host path the policy denies.

`src/shell.rs` resolves an absolute shell at `up`, from the closure. It takes
the resolved `PATH` only where the hit is inside the store or inside a path
the provider declared in `read_only_grants`, and otherwise a `bin/sh` from the
closure's requisites. That guard is the whole correctness: a provider's `PATH`
ends in the host's own directories, and the first version of this code picked
`/usr/bin/dash`.

The guard used to say `/nix/store` literally. `add-test-runtime-fixture`
generalized it to the declared grants, because the rule it enforces is "the
shell is inside something the sandbox is granted and can execute". This isn't
a widening in practice: all three providers get their grants from
`capture::store_grants`, which returns a store-rooted path in every branch.
Both halves are asserted
(`a_host_shell_is_still_refused_when_the_provider_grants_the_store` and its
control `a_shell_inside_a_declared_grant_is_accepted`), because a guard that
refuses everything would pass the first test alone.

The path is recorded in `Meta.shell` and handed to the keeper as
`DEVCROFT_SHELL`. Its store root is folded into the provider grants in `up()`,
so `Meta` and the compiled profile can't disagree; `policy --render` renders
from `Meta`. Never reintroduce a bare `"sh"` at any of the three call sites,
and never require the project to declare a shell. That would fail the first
`up` of every existing project for a dependency devcroft introduced.

## A sandbox has its own user

On Linux everything inside runs as `devcroft` (`[identity] user`), uid 1000,
never 0, mapped onto the caller, so files stay the caller's. macOS runs as the
host user and says so. A manifest never names a host id and never selects
`separate`, which isn't built. `.devcroft/<name>/` is read-only inside except
`home` and `run` (`filesystem.write_deny`). Source: `src/identity.rs`,
`add-sandbox-identity`.

## Error contract

Every error names its layer: `config`, `provider`, `backend`, `keeper` or
`ssh`. Exit codes are stable:

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | Runtime |
| 2 | Usage or config |
| 3 | Environment or provider |
| 4 | Backend |
| 5 | Keeper or connection |

Never prompt when stdout isn't a TTY. `rm` and `up --recreate` need `--yes`
non-interactively.

## Network topology and the egress proxy

On Linux, a `network.default = "deny"` sandbox gets a loopback-only network
namespace, and egress leaves through a Unix-socket relay to the proxy. The
route-less namespace is the boundary, for every protocol and address.
Landlock's `NetPort` is only a port filter inside it. The `agent-networking`
requirements depend on that invariant, and require the port filter only of a
topology that gives agents a route.

Fleet's D9 gate ("no proxy work starts until the seccomp handoff resolves") is
conditional, not struck. It assumed a userspace network helper, which would
make proxy variables cooperative. The shipped design has none. Adding
slirp4netns or pasta reinstates the gate in full.

Landlock doesn't mediate `AF_UNIX`, so sandboxes used to reach any
world-accessible Unix socket, the nix daemon's included. On Linux, every
sandbox now gets its own mount namespace and view
(`fleet::mount::construct_view`), and an ungranted socket fails with `ENOENT`.
On macOS, Seatbelt's deny-default network mode refuses it with `EPERM`.
`tests/unix_socket_not_mediated.rs` now passes because the gap is closed. The
fix was a mount namespace, not seccomp (measured).

`nono-proxy` 0.82 is the proxy's HTTP side (owner decision 2026-11-15,
`add-agent-workload` 4b.0b), taken for credential brokering. devcroft keeps
the spawn, the Unix-socket bridge, the namespace relay and the log format.
What's observable:

- An allowlist refusal is `403` naming the host.
- The child binds the TCP port and reports it over a pipe before `up` compiles
  the policy (`proxy::spawn_at`). The Unix socket is still bound before exec.
- An authentication failure is logged as `requested=<host>:<port>`, never
  `host=`, because `lifecycle::attribution` reads `host=` on a `refuse` line as
  an allowlist decision.
- `strict_connect_auth` and `strict_filter` are set, because `nono-proxy`'s
  defaults are the opposite. Without the first, an unauthenticated `CONNECT` to
  an allowed host is tunneled (measured): an open relay that lends the
  sandbox's allowlist to any local process.
- Inside an isolated sandbox, the per-session token doesn't travel as proxy-URL
  userinfo. Five of eleven coding agents couldn't use that form
  (`docs/agents.md`), so the proxy's Unix-socket bridge presents the token for
  them (`proxy::server::bridge_unix_to_proxy`). The socket's 0700 directory is
  that path's boundary, as for SSH. The TCP listener's token check is
  unchanged.

Three `nono-proxy` capabilities are off by decision, not by omission: TLS
interception (an explicit non-goal), SPIFFE, and AWS routing. Enabling any of
them changes what devcroft claims; it isn't a configuration tweak.

## Framing rules

These govern how devcroft describes itself, in docs, messages and code
comments:

- **One isolation tier, and it's accident protection.** Landlock and Seatbelt
  are not a security boundary against hostile code. The gVisor-backed
  `hardened` tier was built and removed (`remove-gvisor-backend`; the code is
  recoverable at the tag `gvisor-backend-last`): Landlock can't mediate
  `mount()`, which `runsc` requires, so the two couldn't be stacked. The
  supported answer for a stronger boundary is running devcroft inside a VM.
  See [the threat model](threat-model.md).
- **Guarantee tiers are always visible.** `closure` and `artifact` are
  different guarantees; never present them under one word.
- **No non-reproducible mode.** `host` and `none` providers are out of scope
  by design, and the answer for a project without an environment is
  `flox init`. Rejection messages distinguish "not yet supported" (hermit),
  "out of scope by design" (`host`, `none`) and "fails the qualification
  test" (version managers). The owner accepted a file-only workflow for 1.0
  (2026-11-27). `add-file-only-workflow` narrows this rule to "no
  non-reproducible execution" when it lands; until then the rule stands as
  written.
- **Services work for flox, devenv and devbox; nix has no service concept.**
  devbox's services come from generated per-plugin files with the plugin
  author's definitions, a choice the owner made against a stated objection
  that [the decisions](decisions.md) keep. They're read from
  `.devbox/virtenv/<package>/process-compose.yaml`, never through
  `devbox services ls`, which runs `shell.init_hook`. That directory is stale
  cache, so the packages `devbox.json` declares decide what's read. devenv's
  `processes` are read through `devenv eval processes`, which runs none.
  devcroft refuses by name what it won't carry, never drops it. Ordering is the
  subtle case: devenv's `before` and `after` are task-graph edges, and a bare
  process name is a silent no-op in devenv itself.
- **Readiness is carried, and a dependency waits for it**
  (`add-service-readiness`). A service with a probe is reported ready only
  after the probe passes, and a dependent of such a target is emitted as
  `process_healthy` rather than `process_started`, but only where the target
  declares a probe. `ready.notify` and `ready.timeout` stay refused: devcroft
  won't simulate systemd's READY protocol or approximate a wall-clock deadline
  with attempt thresholds.
- **Known limitations are published** in [the known gaps](known-gaps.md):
  - no process-visibility separation between sandboxes on macOS, or on a Linux
    host that can't mount a fresh procfs;
  - no cgroup resource limits outside fleet;
  - egress bounded to destinations, not content.

  Don't describe domain filtering as cooperative. It's enforced on Linux and
  was measured enforced on macOS (`add-egress-proxy`).
- **Techniques, never tools.** Ideas from other projects are recorded in
  [the prior art](prior-art.md). Two standing requirements make this a rule:
  the keeper "SHALL NOT be executed as a child of a separate sandboxing
  binary", and "the process tier requires no external backend binary". So
  bubblewrap and `sandlock` are references, not candidates.
