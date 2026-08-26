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

1. **What does `flox activate` actually require?** The provisioning policy's
   default grants have to be measured against real projects, not assumed —
   likely the provider's own config and data directories and the nix daemon
   socket, possibly more. **Spike this before writing the default profile;** it
   determines whether the change is small or large.
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
