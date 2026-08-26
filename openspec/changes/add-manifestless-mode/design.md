# Design — Manifestless Mode

## M1 — One resolution order, cited everywhere

**Decision.** Environment configuration resolves in a fixed order:

1. explicit flags (`--provider`, `--with`) — always win;
2. `devcroft.toml`;
3. a detected provider signature file;
4. error.

**Rationale.** This becomes the spine of the command. Every "what am I running
and why" question and every failure message resolves against it, so it has to be
one order, written once, and the same on every path. Flags above the manifest
rather than below it because the manifest is a project's default and the flag is
a deliberate override, including on projects that do have one.

## M2 — Detection is by signature file, at the worktree root only

**Decision.** Each provider has a signature file. Detection checks for it in the
worktree root and nowhere else.

**Rationale.** Detection is an existence check, not a heuristic, so it should not
be built like one. Restricting to the root matters more than it looks: walking up
parent directories picks up a sibling subproject's environment in a monorepo, or
worse, a file outside the agent's worktree entirely — which in fleet means one
agent silently running another's environment.

## M3 — Ambiguity resolves by fixed precedence, never by prompting

**Decision.** When several signature files are present, a documented precedence
order decides, and the choice is reported. `--provider` overrides it.

**Rationale.** Multiple files is the common case, not the edge: flox layered over
a flake, or a flake alongside a devbox config. Prompting is not available —
there is nobody to answer in a fleet — so the behaviour must be deterministic
and predictable rather than interactive.

## M4 — Ad-hoc environments are not reproducible unless something is pinned

**Decision.** State plainly that an environment assembled from flags is not
bit-reproducible. Offer to write out what worked.

**Rationale.** Reproducibility is the project's central claim and this mode does
not have it. Pretending otherwise damages the claim everywhere else.

The useful move is to make the mode a path *into* reproducibility: after a
successful ad-hoc run, offer to write a `devcroft.toml` capturing what resolved.
That turns the escape hatch into onboarding — the user gets a working manifest
derived from something they have already seen work, rather than writing one
blind.

## M5 — The default policy here is stricter, not looser

**Decision.** Without a manifest, the policy is narrower than the manifest path's
default: the worktree writable, everything else denied, minimal network.

**Rationale.** The instinct is to be permissive because nothing was declared —
which is exactly backwards. A manifest is a statement by someone who knows the
project. Its absence means nobody has vouched for anything, and this mode is
specifically aimed at repositories nobody has read. Least information should mean
least access.

Consequence: some repositories will fail in this mode and work with a manifest.
That is the correct incentive.

## M6 — A detected file is an intention, not an environment

**Decision.** Failures distinguish "detected X, activating it failed with Y" from
devcroft's own errors.

**Rationale.** A `flake.nix` that does not evaluate or a `devbox.json` naming a
missing package is a project problem surfacing through devcroft. If the message
does not make that clear, it reads as a devcroft bug — and in a mode designed for
unfamiliar repositories, that will be the first impression. This is the same
attribution requirement as `sandbox-provisioning`, arriving earlier.

## Rejected Alternatives

**Restrict the mode to a single provider.** Considered as a simplification, on
the grounds that detection needs a project file. It does not follow: an explicit
`--provider` removes the need for detection without removing provider choice, at
no extra cost.

**Fall back to a default environment when nothing is found.** Produces a working
sandbox containing the wrong environment, silently. An error naming what was
looked for is better.

**Prompting on ambiguity.** Unavailable in fleet, and non-deterministic
elsewhere.

## Open Questions

1. **What `--with` accepts.** Package names are provider-specific. Whether this
   is a thin pass-through to one provider or a common vocabulary across them
   determines how much work this is. A pass-through with the provider named
   explicitly is the smaller first move.
2. **Whether ad-hoc runs may write to the repository at all.** Writing a
   `devcroft.toml` (M4) is a repository modification, so it must be opt-in. Any
   caching or lock this mode wants should probably live outside the worktree.
3. **Relationship to fleet.** Can a fleet be started in this mode, or does fleet
   require a manifest? Manifest-required is more defensible — fan-out over a
   non-reproducible environment undercuts the point — but the external-PR case
   is exactly where both are wanted at once. Decide before fleet ships.
4. **Precedence order for M3.** Needs to be chosen and written down; a defensible
   ordering is more specific-to-devcroft first, general-purpose last.
