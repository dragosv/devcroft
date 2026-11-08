# Remove the gVisor Backend

## Why

The hardened tier was built, verified against real tooling, and works. It is
being removed anyway, for reasons that are architectural rather than about
effort.

**It cannot compose with the sandboxing core.** Applying the sandbox library
around `runsc` is structurally impossible, not merely awkward: `runsc` requires
`mount()`, and Landlock cannot mediate `mount()` at any ABI version, so the
combination fails with `EPERM` under any ruleset. That was measured, not
predicted. Composing the other way — the library applied inside a gVisor
sandbox — would require gVisor to implement the Landlock syscalls, and would in
any case only restrict paths inside a root filesystem devcroft already
constructs entirely, so the marginal value is near zero.

**It cannot support fleet.** Under rootless operation `runsc` shares the host's
network namespace and rejects the sandboxed-network mode. Concurrent instances
therefore collide on service ports exactly as they would with no tier at all.
The direction the project is heading — many environments on one host, each with
its own services — is the one direction this tier cannot go.

**It occupies a squeezed middle.** Below it, the process tier is cheaper and
sufficient for the use case devcroft actually serves: code the user controls,
run in many instances, where the threat is accident and interference. Above it,
a VM is stronger and is already required for macOS. A tier that is more
complicated than the first and weaker than the second has to earn its place, and
it does not: every new capability has to be designed twice, and the honest
answer to "that is not a real boundary" can be "run it in a VM".

## What Changes

- **REMOVED** the gVisor session backend, its OCI bundle synthesis, its pinned
  runtime in the development container, and its integration tests.
- **MODIFIED** `session-backend`: one backend remains. The tier selection that
  chose between them is removed; the isolation axis collapses.
- Lessons from building and removing it are recorded in `design.md` and
  summarised in the README, rather than disappearing with the code.

## Impact

- Affected specs: `session-backend`
- Simplifies `add-backend-capabilities`: with a single backend, the capability
  matrix stops tracking divergence between backends and starts tracking the gap
  between what the sandbox library offers and what devcroft has adopted. That is
  a more useful thing for it to do.
- Resolves the two-axis confusion: with one backend, the isolation model
  collapses cleanly into single-environment versus fleet.
- The upper bound on isolation is now fixed at what the process tier provides.
  See `docs/threat-model.md`: use case B — code written to escape — is not
  served, and this change closes the path to serving it.

## Non-Goals

- Ruling out a second backend permanently. The criteria for what a future
  backend must satisfy are in `design.md`.
- Removing the multi-backend abstraction itself. See G5.
