# Design — Sandbox Provider Resolution

## P0 — Why activation runs on the host today

Not a limitation of the sandbox layer — a bootstrap cycle in the current
ordering. The runtime sandbox is built from the compiled policy; the compiled
policy needs the store grants; the store grants are derived from the resolved
environment; resolving the environment means activating. Activation ended up
outside because it was the only place the information existed.

The cycle breaks by giving provisioning its own policy (P2) rather than deriving
one. A provisioning profile needs no store grants — it is not running the
project's code against a resolved toolchain, it is executing activation in order
to obtain one. Nothing about it depends on the output it produces.

Worth recording because the constraint reads as fundamental and is not.

## P1 — Confine the execution rather than avoid it

**Decision.** Run provider activation inside a sandbox and read the resulting
environment out as data, instead of trying to suppress the hook.

**Rationale.** Suppression was measured and does not exist: against flox 1.14.0,
no activation mode skips `[hook].on-activate`. Nor should it — a project whose
hook installs its dependencies is broken without it. The nix provider's
structured-read approach is not portable to flox or devbox, which have no
equivalent of evaluating a dev shell without running its hook.

What is portable is the boundary. The existing architecture already builds
sandboxes and reads structured output back across them; provisioning becomes
another instance of that, not a new mechanism.

## P2 — Provisioning is a separate policy profile, not a variant of the runtime one

**Decision.** The manifest declares two profiles. Runtime policy governs the
agent; provisioning policy governs activation. Neither is derived from the
other.

**Rationale.** Their needs genuinely differ and the difference is not a matter of
degree. Provisioning legitimately needs network access and writable package
caches; a runtime sandbox often should have neither. Deriving one from the other
would mean either denying provisioning what it needs, or granting the runtime
what it does not.

Making them separate also makes the trade visible: `policy --render` shows what
provisioning may reach, so "wider than runtime" is an inspectable fact rather
than an implementation detail.

## P2a — Materializing a Nix environment is not "`/nix` read-write"

**Decision.** The provisioning profile keeps `/nix/store` **read-only**.
Materialization happens through the `nix-daemon` socket, modelled as a distinct
*package-manager capability*, never as a broad write grant on `/nix`.

**Rationale.** The two are easy to conflate and are not the same thing at all.
Writing to `/nix/store` directly would let activation place arbitrary content in
a store every other environment on the machine reads from. Talking to the daemon
is a request to a service that validates what it is asked to realise, and that
can — in principle — be mediated per operation.

Granting `/nix` read-write because "materialization needs to write to the store"
would be the single largest silent widening available in this change, and it
would be invisible in exactly the way this project's policy invariants exist to
prevent: it renders as an ordinary filesystem grant. Daemon authority is
therefore modelled and rendered as its own thing (see the `policy` delta), so a
profile that has it is visibly different from one that merely reads the store.

## P2b — Trusted materialization does not hand daemon authority to project code

**Decision.** Project-controlled activation code never receives the daemon
socket. A provider qualifies for fully confined activation only if it can
separate materialization from hook execution, or if devcroft can mediate the
daemon through a proven operation-scoped interface.

A hook that itself requires daemon-backed materialization — installing a package
at activation time rather than declaring it — **fails closed** at layer
`provider`. It does not silently receive a writable `/nix` as a fallback.

**Corrected once already, and corrected again now that the fix has landed.**
This paragraph used to read "a writable `/nix` or the daemon socket", which
asserted something devcroft did not yet enforce: Landlock mediates TCP but not
AF_UNIX, so a sandbox connected to `/nix/var/nix/daemon-socket/socket` — mode
`srw-rw-rw-` under nix's multi-user model — whether or not `/nix` was granted.
That gap is why the sentence was walked back to "devcroft never grants" rather
than "cannot reach": the refusal was devcroft's own, not the kernel's.

**`add-mount-isolation` closed it.** Every sandbox now gets its own mount
namespace and filesystem view (`fleet::mount::construct_view`); the daemon
socket's path simply does not resolve inside a view that never contains
`/nix/var`, and `connect()` fails with `No such file or directory` rather than
succeeding against whatever authority the daemon extends to a local user.
Measured live, not assumed: `tests/unix_socket_not_mediated.rs` (inverted
alongside this correction) asserts the refusal, including a real `up` session
against a real nix daemon.

**So the original sentence, "does not silently receive a writable `/nix` or
the daemon socket", is true again — now for the reason it originally claimed.**
P2a/P2b's "agents must not hold package-manager authority" is a kernel-enforced
boundary on any host running a nix daemon, not only a design intent devcroft
happens to uphold by never granting the path. `/nix/store` stays visible
(read-only) for the toolchain the sandbox actually needs; `/nix/var` — the
daemon socket included — does not exist in the view at all.

Note what P2d changes about the scope of that rule, since the two are easy to
read as contradicting: P2d removes the need to refuse flox *for having a hook*,
because materialization no longer runs the hook at all. What stays refused is
narrower and correct — a hook that needs materialization authority **while
running as project code**. Declaring the package in `[install]`, where it
belongs, resolves it.

**Rationale.** This is the whole point of the split. Materialization is trusted
because it runs pinned tooling from a lockfile; a hook is project code and is
trusted exactly as much as the repository is — which, for the case this change
exists to serve, is not at all. Fusing them would mean the untrusted half
inherits the trusted half's authority, and a host-global one at that.

Failing closed is the uncomfortable option and is chosen deliberately. The
alternative is a fallback that grants the authority anyway, which would make the
requirement decorative: a rule with a fallback that triggers on exactly the
cases it was written for is not a rule.

## P2c — Providers differ in what they expose, and flox needs help

**Decision.** Eligibility for confined activation is per provider, decided by
whether the provider can be asked for an environment without running the
project's hook.

| provider | hook-free path | hook runs? | eligible |
| --- | --- | --- | --- |
| Nix flakes | `nix print-dev-env --json` | no | yes |
| Devbox | `devbox shellenv --pure` | no | yes |
| Flox with `hook.on-activate` | none *provided by flox* | always, via any flox invocation | **yes — see P2d** |

Flox exposes no public materialize path, no pre-hook context, and no separate
hook runner — measured across `--mode dev`, `--mode run` and
`--no-start-services`, none of which suppress the hook
(`fix-provisioning-hooks`). **That remains true and is no longer disqualifying**:
P2d constructs the split devcroft needs instead of waiting for flox to expose
one. An earlier version of this decision listed flox as blocked; that was
correct about the interface and wrong about the conclusion.

**Rationale, and what this is not.** This is not a judgement about flox, and
`hook.on-activate` is not an abuse of the format — it is where people put
everything Nix does not do. It is a statement about what each provider hands
back, and what devcroft has to construct for itself as a result.

An earlier version of this section concluded that "every available workaround is
a security compromise" and therefore that flox had to be refused pending
upstream. That was wrong, and worth recording as wrong: the workarounds
considered were all variants of *granting the hook more authority* (a writable
store, the daemon socket), and every one of those is indeed a compromise. The
option not considered was **taking authority away from the materialization step
instead** — deriving a hook-free environment, which needs no new authority at
all. P2d is that option.

The asymmetry that made this urgent: flox is the provider devcroft recommends
by default and the one `init` scaffolds. Leaving the default provider
unconfinable would have made confined provisioning a feature almost nobody
got — which is why "refuse flox until upstream moves" was not an acceptable
resting place, and why P2d exists.

## P2d — Materialize from a devcroft-derived, hook-free manifest

**Decision.** For a flox environment declaring `hook.on-activate`, devcroft
materializes from a **derived environment it owns**: a copy of the project's
flox environment with the `[hook]` table removed, activated to realise packages
and capture the environment. The project's hook then runs **inside the
provisioning sandbox**, against that already-materialized environment.

This gives devcroft the materialize/hook split flox does not expose, without
granting project code daemon authority and without refusing the default
provider.

**Measured, not assumed.** Verified live against real flox:

| check | result |
|---|---|
| Does stripping `[hook]` change the resolved closure? | **No** — identical store path (`jq-1.8.1`) |
| Do the locked package sets differ? | **No** — byte-identical `packages` |
| Does the hook run during derived activation? | **No** |
| Does the hook still work when run inside the derived environment afterwards? | **Yes** — packages on `PATH`, flox context present |

The closure is unchanged because `[hook]` is not a package input: it contributes
nothing to resolution, so removing it cannot alter what gets realised. That is
what makes this a *split* rather than a *different environment*.

**The derived environment is devcroft-owned and lives outside the project.** It
is not written into `.flox/`, so this does not mutate project state and does not
interact with the lockfile-integrity requirement — the project's own manifest
and lock are read, never rewritten.

**Enumerated caveat: flox context variables point at the derived directory.**
Measured, the differing variables are exactly `FLOX_ENV`, `FLOX_ENV_PROJECT`,
`FLOX_ENV_DIRS`, `FLOX_ENV_DESCRIPTION` and `FLOX_PROMPT_ENVIRONMENTS`. Of
these `FLOX_ENV_PROJECT` is the one that matters: it is how a hook idiomatically
finds the project root, and left uncorrected a hook would resolve paths into
devcroft's scratch directory instead of the user's project. devcroft therefore
sets these to the project's real values when running the hook. Copying the
environment's `env.json` preserves the environment *name*, so
`FLOX_ENV_DESCRIPTION` is already correct.

**Why this is a workaround and the upstream request still stands.** devcroft is
reconstructing a boundary by transforming someone else's configuration format,
which means tracking that format: a future flox schema change could alter where
hooks live or add a second place project code can run, and this would need to
follow. A supported API would make the split flox's contract rather than
devcroft's inference. The request at
[docs/flox-confined-activation-issue.md](../../../docs/flox-confined-activation-issue.md)
is therefore still worth filing — but it is now an ergonomics and
maintenance-burden ask, not a blocker.

**What it does not fix.** `[profile]` scripts, if flox runs them on the same
path, would need the same treatment; that is unmeasured and is a task, not an
assumption. And the hook still runs — inside a sandbox, which is the entire
point, but a hook that legitimately needs materialization authority (installing
a package at activation time) will fail there, correctly, per P2b.

## P3 — The environment crosses the boundary as data

**Decision.** Activation writes the resulting environment to a descriptor the
supervisor holds. The supervisor parses it as data, applies the same
fixed-baseline diff already used today, and never sources it as shell.

**Rationale.** This preserves the property that makes the nix provider safe: the
result is read, not executed. A hook that emits shell intended to be evaluated
by the caller does not get that evaluation.

The baseline diff, store-grant derivation and staleness fingerprinting are
unchanged — they operate on the captured environment regardless of where the
capture happened.

## P4 — Home is substituted, not hidden

**Decision.** Provisioning gets a private home directory. Paths that must
persist across activations — package manager caches, the provider's own state —
are declared and bound in individually.

**Rationale.** A blanket denial of `$HOME` breaks provisioning for most real
projects, since `npm`, `pip`, `cargo` and the providers themselves all keep
state there. A substituted home makes the default outcome "the write goes
somewhere harmless" rather than "the activation fails", while declared paths
keep caching effective.

Consequence worth stating: a hook that writes to `~/.gitconfig` or drops a
symlink in `$HOME` will appear to succeed and have no effect on the real home.
That is the intended behaviour, and it must be documented, because it is a
behavioural difference from running the hook by hand.

## P5 — Provisioning must be able to fail loudly

**Decision.** When activation fails inside the provisioning sandbox, `up` fails
with the denied path or interface named — not with the provider's raw error.

**Rationale.** This is the change's main risk. A hook that worked yesterday on
the host now fails with a package manager's unhelpful error, and the sandbox is
invisible in that message. If a user cannot tell a policy denial from a broken
hook, the feature costs more than it gives.

## P6 — Measured: what each provider's resolution needs once materialized (2026-10-09)

The spike group asks what confined resolution needs. Measured on this
devcontainer (nix 2.31.5, Linux 7.0.14) in a user and mount namespace with
the daemon's socket directory covered by an empty one, no network namespace
route, and `NIX_REMOTE=daemon`, so any Nix call *must* use the hidden socket
and fails if it tries (`nix store info` was checked to fail that way).

| Provider | Hook-free resolution | No daemon | No network | Substituted home | Writes |
|---|---|---|---|---|---|
| devbox | `devbox shellenv --pure` | works, byte-identical | works | works (only `HOME` and devbox's hash of the env differ) | nothing |
| flox | `flox activate` on the derived, hook-free copy (P2d) | works, same `PATH` and `FLOX_ENV` | works | works **only with the XDG variables set**: with `HOME` alone flox still wrote its env registry into the real home | its registry in the substituted home; logs and `run/` in the derived copy (`.devcroft/`, the project's) |
| nix | `nix print-dev-env --json` | **fails** | — | — | the project's own source, into the store |
| devenv | `devenv build shell`, `devenv eval enterShell`, `devenv eval processes` | **fails** once its own evaluation cache is bypassed | needs its inputs from Nix's fetcher cache, which lives in the real home | **fails**: an empty home means re-downloading nixpkgs | `.devenv/` in the project (its evaluation cache) |

What this establishes:

- **devbox and flox split cleanly.** Materialization (`devbox install`,
  flox's build of the derived copy) is the trusted half, which needs the
  daemon and, unless the store already holds everything, the network.
  Measured: `devbox install` runs no `init_hook`. Resolution is the other
  half, and needs neither, nor the real home: the minimal set is the
  project read (and `.devcroft/` written, for flox), `/nix/store` read, and
  an empty substituted home. That is the opposite of "most of `$HOME`", so
  the stop condition in the spike group does not trigger.
- **An unmaterialized environment fails the confined half, not silently.**
  devbox, run confined on a copy that was never installed, tried to
  materialize and failed on the network (`cache.nixos.org` unreachable). The
  error names the network rather than "not materialized", so P5's
  attribution is devcroft's job: run materialization first, and treat a
  confined resolution that reaches for the network or the daemon as "not
  materialized".
- **nix cannot resolve without store-write authority at all.** Evaluating a
  local flake copies the project's source tree into the store
  (`/nix/store/…-source`), taking a store lock to do it, even when every
  output already exists; read-only local store mode fails on that lock. So
  nix's resolution *is* its trusted half: one step, holding daemon
  authority, running the pure evaluator over repository-controlled Nix and
  no repository-controlled shell. That is the property the proposal already
  claims for nix, now with the reason it cannot be split further.
- **devenv is nix's case, not devbox's** (measured with devenv 2.4.0,
  fetched for the measurement through `nix shell`, since this host has no
  devenv installed and its tests skip). Every command devcroft runs
  evaluates the project's `devenv.nix`, and evaluation needs the daemon:
  `eval enterShell` failed on the hidden socket, and so did
  `build shell` once `--no-eval-cache` took devenv's own cache out of the
  way. Without that flag `build shell` *succeeded* confined, because the
  cache answered without evaluating, which is exactly the kind of result
  that must not be read as a qualification: it holds only until the next
  edit to `devenv.nix`. Evaluation also reads Nix's fetcher cache from the
  real home, so with an empty substituted home devenv re-downloads its
  nixpkgs input. What devenv does keep is the property that matters: no
  project shell runs during resolution. The environment is a store file
  devcroft reads as data, and `enterShell` is captured as a string and run
  inside the sandbox, so devenv's trusted half is its evaluator, as for nix.
- **A substituted home has a path-length budget.** flox puts a services
  socket under `XDG_CACHE_HOME`, and a long substituted home overran the
  OS's limit for a unix socket path, the same constraint `devenvsvc` hit.

What is still unmeasured: `[profile]` scripts, other devbox plugins, and the
trusted half's own minimal grants beyond "daemon plus network".

## Rejected Alternatives

**Keep warning.** Adequate for a developer opening their own repository, useless
for the fleet case, where nobody reads the second warning.

**Make nix the only provider.** It sidesteps the problem rather than solving it,
and abandons the multi-provider property that was deliberately built and
validated by adding a third provider on a different substrate.

**Run provisioning inside the runtime sandbox.** Provisioning needs network and
writable caches that the runtime sandbox should not have. Merging them widens
the runtime policy to fit provisioning, which is backwards.

## Open Questions

1. **~~Can Flox separate materialization from `hook.on-activate`?~~ —
   RESOLVED, by not needing flox to.**

   **The answer to the question as posed is no**, and it is still no. What
   changed is that the question was the wrong one: devcroft can construct the
   split itself by deriving a hook-free environment (P2d), measured to produce
   an identical closure. The framing below assumed the only way to get a split
   was for flox to provide one, and would have concluded "refuse flox" from a
   premise that is entirely true. Kept for that reason — it is a clean example
   of a correct measurement leading to a wrong conclusion because of how the
   question was scoped.

   Original text follows.

   This question used to read "what does `flox activate` actually require?",
   which assumed the answer was a list of grants to measure. It isn't. The
   measurement was done (`fix-provisioning-hooks`) and produced a harder
   result: no documented invocation yields an environment without running the
   project's hook, so there is no set of grants that makes confined Flox
   activation safe — the hook would hold whatever authority materialization
   needs, including the `nix-daemon` socket.

   **This must not be answered by granting the hook the daemon socket or a
   writable `/nix`.** Both would "work" and both defeat P2a/P2b. If the answer
   turns out to be no, the correct outcome is the fail-closed behaviour P2b
   specifies, not a fallback.

   The unblocking path is upstream, drafted at
   [docs/flox-confined-activation-issue.md](../../../docs/flox-confined-activation-issue.md).
   Until it lands, this change ships with Flox-with-a-hook blocked and the
   other two providers eligible — which is a real, useful subset, not a
   stalemate.
2. **Network during provisioning — RESOLVED as a dependency, not a question.**
   Denying it breaks `npm ci`; allowing it opens an outbound channel from
   unreviewed code, which is the exact thing this change exists to close.
   Neither is acceptable, so the answer is a domain allowlist for the
   provisioning context — which does not exist today. **`add-egress-proxy` is a
   hard prerequisite**: without it, this change confines the filesystem and
   leaves egress open, which is confinement in name only for the fleet use case.
   Do not ship this change with a binary network on/off and call the problem
   solved.
3. **Cache sharing across agents.** A shared package cache is the difference
   between one download and N. But a shared writable cache is also a channel
   between agents that are otherwise isolated. Read-only sharing plus a
   per-agent overlay is the likely answer; needs a decision before fleet.
4. **macOS fidelity.** Seatbelt's ability to substitute a home directory and
   confine activation differs from Landlock's. Decide the honest degradation
   statement rather than claiming parity.
5. **Does the captured environment stay reproducible?** If a hook installs into
   `node_modules/.bin` and the captured `PATH` references it, the environment is
   no longer a pure function of the manifest. Existing staleness fingerprinting
   may already cover this; confirm rather than assume.
