# Design: add-devbox-provider

## Context

See `proposal.md` — Why. What shapes the approach here is what already
exists rather than what is new: `add-nix-provider` generalized provider
dispatch, and the shape a third provider must fit into is fixed.

Three constraints do the work:

- **`Resolution` is the whole contract.** A provider returns an env diff,
  a list of unsets, read-only grants, and its service story. Everything
  downstream — policy compilation, keeper injection, staleness, `status`
  — is keyed off the provider name in `provider::mod` and nowhere else.
  A provider that needs more than this is a finding, not a feature.
- **The canonical baseline is shared, and that is deliberate.** flox and
  nix both diff activation against the same fixed pre-activation
  environment (a real `HOME`, a conventional `PATH`, nothing else), which
  is what makes the captured diff independent of whoever ran `up`. devbox
  reuses it unchanged; a provider that needs its own baseline would be
  reintroducing the non-reproducibility that baseline exists to close.
- **The closure tier grants nothing from the host.** `own-policy-baseline`
  measured this: a full build from a flox or nix closure needs the project
  root, `/tmp`, and the store — nothing from `/lib`, `/usr/lib`, or
  `/usr/bin`. devbox inherits that only if its resolved environment really
  is a self-contained closure, which is a claim to verify, not assume.

The material difference from `add-nix-provider`: that change was written
against tooling the author could run. **devbox is not installed in this
repo's devcontainer**, so every statement about its CLI comes from
documentation. The task ordering below is the response to that.

## Goals / Non-Goals

**Goals:**

- A third closure-tier provider that adds no new concepts — no new
  guarantee tier, no new store model, no manifest translation.
- Confirm the `Provider` trait generalizes to a provider that is *not*
  built on the same activation mechanism as the previous two.
- Keep every devbox-specific fact in `src/provider/devbox.rs`, with
  `provider::mod` gaining only dispatch arms.

**Non-Goals:**

- **devbox services.** See proposal — Impact. The declarations come from
  plugin-supplied process-compose configs rather than a documented schema
  in `devbox.json`, which is the shape `add-flox-services` decision 1
  rejected. `ServiceSupport::Unsupported` in this change.
- **Multiple named environments.** devbox has an environment concept
  beyond the default; out of scope for the first cut exactly as
  non-default `devShells` were for nix.
- **Translating `devbox.json` into anything.** devcroft reads it only to
  fingerprint it and to check it exists. devbox owns its own format.
- **Making devbox available in the devcontainer for end users.** Adding
  it to the image is a development-environment decision (task group 0),
  not part of what this change ships.

## Decisions

### 1. Capture via `shellenv --pure`, evaluated in a controlled shell

**Measured against devbox 0.18.0 (task 0.2). The first draft of this
decision chose the opposite mechanism, and measurement inverted it.**

The draft preferred `devbox run -- sh -c 'env -0 > <tmp>'`, reusing the
trick `nix.rs` already uses twice, and rejected `devbox shellenv` as
fragile shell-parsing. Both halves turned out wrong:

- **`devbox run` runs the project's init hook** — measured, including
  with `--pure`. That is project code executing during the trusted
  host-side phase, which the two-phase rule forbids outright. It is
  disqualified on a correctness ground the draft never considered, not
  on the fragility ground it was chosen to avoid.
- **`devbox shellenv` does not run the hook**, under any variant tried
  (default, `--pure`, and even `--init-hook`). `--init-hook` does not
  execute anything: it appends one `. .devbox/gen/scripts/.hooks.sh`
  line to the emitted text, leaving execution to whoever evaluates it.
  devcroft simply never passes the flag.

The draft's fragility concern about `shellenv` was nonetheless correct —
its output is *not* a clean list of assignments. Measured, it contains
multi-line values whose contents are themselves shell (nixpkgs'
`mkShell` `$out`-recording snippet), and it ends with a real
`if ! type refresh …; then alias refresh=…; fi` block plus `hash -r`.
Parsing that line-by-line would silently produce a wrong environment.

**Chosen: evaluate, then dump.** Run
`sh -c 'eval "$(devbox shellenv --pure)"; env -0 > <tmp>'` from
devcroft's canonical baseline environment. This keeps the `env -0`
machine-readable capture the other two providers already use — no shell
parsing anywhere — while letting devbox's own shell code set the
environment up, and never sourcing the hook.

`--pure` is **mandatory, not a refinement**. Without it, `shellenv`
re-exports the operator's entire ambient environment into its output:
measured, the capture carried `CLAUDECODE`, `AI_AGENT`, and a
`BROWSER` pointing into a VS Code server install. With it, a decoy
`PATH` prepend and decoy variables did not survive into the capture, and
two runs from different polluted shells produced identical results.

Alternative still rejected: reimplementing devbox's resolution by
reading `devbox.lock` directly and materializing store paths ourselves.
It would remove the CLI dependency, but devcroft would own a second
implementation of devbox's semantics, which will drift. The provider
contract is "run the provider's own activation and capture it".

### 1a. Store grants: `capture::store_grants` reused unchanged, no profile resolution needed

Measured, and different in shape from flox and nix: a devbox project's
*declared* packages are not on `PATH` as bare store paths. They arrive via
`<project>/.devbox/nix/profile/default`, a symlink chain
(`default-1-link` → `/nix/store/…-profile`) rooted inside the project.
The store paths themselves appear in `HOST_PATH`, not `PATH`.

**A first pass at this decision (recorded here, then corrected by
measurement) concluded from that alone that grant derivation would have
to resolve through the profile link, or it would "grant the stdenv
closure and miss every package the project actually declared."** Running
`capture::store_grants` — unmodified — against a real captured `PATH`
disproves the second half. The function does not collect individual
package paths; it scans `PATH` for the first entry containing
`/nix/store` and returns only the root prefix, `/nix/store` itself (see
its doc comment and `store_grants_reads_root_from_activated_path`). Every
devbox `PATH`, with or without declared packages, carries devbox's own
stdenv wrapper (`gcc-wrapper`, `coreutils`, `binutils`, …) as literal
`/nix/store/...` entries ahead of the profile-symlink entry — measured
for both a ripgrep-declaring project and an empty one. So the scrape
always finds a match and always returns the same coarse `/nix/store`
root that flox and nix already get. That root, granted read-only, covers
*everything* under it — including whatever the profile symlink resolves
to — because the grant is a directory prefix, not an enumeration of
specific paths. There is nothing narrower to miss.

**Task 1.3 therefore reuses `capture::store_grants` unchanged**, matching
what `proposal.md`'s own "Why" claimed before this decision complicated
it ("devbox reuses the store-grant derivation ... unchanged"). The
profile-symlink finding stays recorded above because it is true and
explains *why* declared packages aren't directly visible on `PATH` — it
just isn't a reason to touch grant derivation. Verified with a package
outside the stdenv closure (ripgrep) specifically so the claim is
falsifiable rather than assumed: if the grant were narrower than the
whole store root, `rg`'s store path would not be covered and the
toolchain-under-`network.default=deny` test (3.4) would catch it.

The `.devbox` directory itself needs no gitignore note: `devbox init`
does not write one, and nothing here depends on it being ignored — the
project root is already granted read-write regardless.

### 1b. The lockfile precondition checks key presence, not per-system coverage

**Corrected by measurement; an earlier draft of this precondition was
narrower than what devbox actually needs and would have rejected working
projects.** The draft required each declared package's `devbox.lock`
entry to cover the system `up` is running on, reasoning that an entry
resolved only for another platform leaves the current one unresolved —
plausible from the lockfile's shape (resolutions recorded per system),
but not tested against real devbox behavior.

Tested directly, using the exact capture command decision 1 chose: a
`devbox.lock` entry for `ripgrep@latest` containing only an
`x86_64-darwin` systems entry, run on an `aarch64-linux` host, resolves
and materializes `rg` successfully — and `devbox.lock` is byte-identical
before and after. devbox resolves the current system from the entry's
pinned `resolved` commit reference (a fixed nixpkgs commit shared across
all systems for that entry), not from the systems cache; the cache
records what has been resolved before, not what is resolvable now. There
is no per-system gap to close.

**What the same session then confirmed does violate the two-phase
rule:** a package declared in `devbox.json` with **no key at all** in
`devbox.lock` — e.g. added by hand without `devbox install` — causes the
identical capture command to resolve it live against `nixpkgs-unstable`
(a floating branch reference, not a pinned commit) and **write the new
resolution into `devbox.lock` on disk**.

**One leg of that original argument was wrong, and adversarial review
removed it.** It also cited the `cache.nixos.org` fetch as evidence.
A cold-store measurement — `cowsay`, never materialized on this host,
locked only for `x86_64-darwin` while running `aarch64-linux` — shows
the *permitted* case downloads 13 MiB from `cache.nixos.org` too, and
still leaves the lockfile byte-identical. Fetching a pinned store path is
precisely what "materializing already-pinned packages is permitted and
expected" describes. The cache fetch therefore discriminates nothing;
the lockfile write is the whole signal. Stating it as evidence made the
conclusion look better supported than it was.

So the precondition is "does the project's declared package have a key
in `devbox.lock`'s `packages` map", full stop — no per-system check.

### 1c. A byte comparison of the lockfile, after capture, is the real enforcement

**Found by adversarial review of the shipped implementation, not during
design** — decision 1b's precondition is necessary but not sufficient,
and the gap is structural rather than an oversight in how it was coded.

`devbox.lock` carries more than the project's declared packages: it also
carries devbox's own **base nixpkgs entry**. That entry is not a declared
package, so no per-package precondition can see it. Measured: a project
whose every declared package was fully resolved, but whose lockfile
lacked a `github:NixOS/nixpkgs/…` entry, passed every precondition — and
`devcroft up` then resolved that entry against the floating
`nixpkgs-unstable` branch and wrote it to the user's file. Confirmed
through the real binary, by md5 before and after.

**A second, larger consequence: "declares no packages" does not mean
"has nothing to resolve".** A zero-package devbox project still gets a
stdenv (gcc, coreutils, bash — all visible on the captured `PATH`), and
that stdenv comes from the same unpinned base. So the earlier spec
scenario asserting such a project needs no lockfile was wrong on
reproducibility grounds, not merely incomplete: without one, two machines
running `up` a month apart get different toolchains. It is replaced.

**Why a post-check rather than one more precondition.** The base entry's
key is not a constant — measured, a project pinning `nixpkgs.commit` in
`devbox.json` locks under `github:NixOS/nixpkgs/<that commit>` instead of
`github:NixOS/nixpkgs/nixpkgs-unstable`. Predicting the complete key set
means reimplementing devbox's resolution rules, which decision 1 already
rejects by name ("devcroft would own a second implementation of devbox's
semantics, which will drift"). Comparing the file's bytes needs no such
knowledge, and keeps working if devbox changes its scheme.

The check restores the original bytes (or removes a lockfile capture
created) before failing, so a refused `up` leaves the tree as it found
it. Both directions are tested: a complete lockfile — one `devbox
install` produced — survives capture byte-identically, so the guard
cannot be satisfied by always failing.

Worth recording for whoever qualifies the next provider: `devbox add`
alone does **not** produce a complete lockfile. It writes the package's
entry and omits the base one; only `devbox install` writes both. Two of
this change's own tests used `add` and were passing for the wrong
reason.
Computing that key from `devbox.json` needs to account for two accepted
shapes, both exercised live against devbox 0.18.0:

- **Array form** (`"packages": ["ripgrep@latest"]`): the string is the
  lock key verbatim, including devbox's "legacy" bare-name-with-no-`@`
  form (`"ripgrep"` locks under the literal key `"ripgrep"`, no `@latest`
  appended — devbox prints a deprecation warning but accepts it).
- **Object form** (`"packages": {"ripgrep": "latest"}` or
  `{"ripgrep": {"version": "latest", ...}}`): the lock key is
  `"{name}@{version}"` when a version string is present (as a bare value
  or the table's `version` field), or the bare name with no `@` when it
  is absent — matching the array form's legacy behavior exactly.

Exotic package reference shapes this does not attempt to normalize (git
refs, `path:` local packages, per-platform overrides) are out of scope,
consistent with design's own non-goal of not translating `devbox.json`
semantics. Where a declared entry's key cannot be confidently computed,
the precondition SHALL fail closed (report unresolved) rather than skip
it — the same bias `flox.rs`'s `declares_activation_hook` already uses,
for the same reason: a false negative here defeats the precondition
entirely, while a false positive only costs a `devbox install` the user
didn't strictly need.

### 2. The init-hook problem is a qualification question, not a detail

devbox environments can define an initialization hook that runs on
activation. If capture cannot avoid running it, then resolving a devbox
environment executes project code during the trusted host-side phase —
with the host's network and filesystem, before any restriction exists.
That is a direct violation of the two-phase rule, which is
non-negotiable per CLAUDE.md.

This is specced as a requirement (`env-provider`: "Provisioning never
executes project code") rather than left as an implementation note,
because the honest outcome depended on what devbox actually does.

**Measured (task 0.3): devbox passes criterion 4, but only through the
mechanism decision 1 now chooses.** A hook writing a sentinel file did
not run under `devbox shellenv` in any variant; it did run under
`devbox run`, `--pure` included. So the qualification does not come from
devbox being careful — it comes from picking the one entry point that
does not activate. That makes decision 1 load-bearing for correctness
rather than for ergonomics, and a future switch back to `devbox run`
for any reason would silently reintroduce a two-phase violation.

The stakes are higher than a normal edge case because `devbox init`
writes an `init_hook` into **every** new `devbox.json` — having one is
the out-of-the-box state, not an unusual choice. There is no population
of hook-free devbox projects to fall back on.

Task 1.6 therefore asserts the hook does not run, as a test rather than
as a comment: it is the guard on the property that qualified devbox.

Stating the rejection condition up front was the point, and it nearly
fired. A provider change that cannot fail is not being evaluated.

### 3. Global packages are a lockfile-integrity question

devbox maintains a machine-global package set. If activation includes it,
the captured environment is not a function of the committed files, and
two machines produce different sandboxes from the same repo — criterion 2
in substance even though `devbox.lock` exists on paper.

Handled the same way as decision 2: specced as a requirement
(`env-provider`: "Resolution depends only on committed files"), measured
in task 0.4, and a blocker if it could not be excluded.

**Measured: no leak.** A package added with `devbox global add` did not
appear in the project capture — not on `PATH`, not in any variable's
value. devbox's global profile is opt-in at the shell level (it tells
the user to add `eval "$(devbox global shellenv)"` to an rcfile) and
project activation does not consult it. Combined with `--pure` from
decision 1, which strips whatever the operator's shell had already
loaded, the captured environment is a function of the committed files.

Worth keeping the requirement in the spec even though it passed: it
passed because of how devbox scopes its global profile today, which is
devbox's decision to revisit, not devcroft's.

### 4. Nix is devbox's precondition, reported as devbox's

devbox cannot work without Nix, so `up` must check for it. The reporting
choice matters: telling a devbox user "nix is missing" invites the wrong
fix (switching providers). The error names Nix as *devbox's* requirement
and how to install it, keeping the provider the user chose intact.

This also means `doctor`'s devbox check is two probes, not one, and the
`cli` delta says so. It is the first provider whose preconditions include
another provider's tooling — worth naming because it is the pattern
`devenv` will need too.

### 5. Fold the unspecced `doctor` scoping into this change's `cli` delta

`doctor` was recently changed to check only the provider the manifest
declares, rather than probing flox unconditionally. That behavior shipped
without a spec, and the existing `doctor` requirement still describes a
backend check by binary version — which stopped being true when the
process tier moved to a linked library.

A `MODIFIED` requirement must carry full updated content, so this
change's `cli` delta cannot copy the stale text forward without
asserting something false. It states the current contract instead:
capability probed rather than inferred, and providers scoped to what the
project declares. That is not scope creep into unrelated territory — the
devbox check *is* a per-provider check, and it needs the rule it depends
on to exist in a spec.

## Risks / Trade-offs

- **Everything about devbox here is documentation-derived** → Mitigation:
  task group 0 runs before any implementation task and can reject the
  change. No code is written against an unverified claim; the design
  decisions above each name what would falsify them.

- **`devbox run` may run the init hook with no way to suppress it** →
  Mitigation: this is decision 2's rejection condition, not a bug to work
  around. Recording it as a criterion-4 failure is a valid outcome of
  this change, and a more useful one than a provider with a silent
  two-phase violation.

- **devbox's closure may not be as self-contained as flox's or nix's** →
  Mitigation: task 3.4 repeats `own-policy-baseline`'s own measurement —
  a real build inside the sandbox with the host toolchain denied. If it
  needs host libraries, devbox is not closure tier for devcroft's
  purposes regardless of being Nix-backed, and the tier claim changes
  rather than the measurement being explained away.

- **Adding devbox to the devcontainer grows the image and adds a
  dependency the project does not otherwise need** → Mitigation: it is a
  development image, the same argument that kept the `nono` binary after
  `use-nono-library`. The tests self-skip without it, so contributors
  who do not install it lose devbox coverage and nothing else.

- **A third provider makes the "adding a provider touches one file"
  claim harder to keep honest, since each provider adds arms rather than
  reducing them** → Mitigation: success criteria in the proposal make it
  falsifiable — if anything outside `provider::mod` and the new module
  must change shape, that is recorded as the trait not generalizing, not
  absorbed silently.

## Migration Plan

Additive. No existing manifest changes meaning: `devbox` was previously
a validation error, so no manifest in the field names it and none can
change behavior. The `config` delta's third scenario is the regression
test — a manifest not naming devbox compiles byte-identically.

Rollback is removing the dispatch arms and the module; the name returns
to `NOT_YET_SUPPORTED` with its existing message.

## Open Questions

- Whether `devcroft init` should offer to run `devbox init` for a project
  that has none, the way it currently only *advises* `flox init`. The
  advisory form is consistent and requires no decision now; making init
  run provider commands is a broader change than this one.
- Whether the devbox provider should accept a non-default environment via
  a manifest key later. Additive whenever it is wanted, and settling it
  now would not change the specs or the task breakdown.
