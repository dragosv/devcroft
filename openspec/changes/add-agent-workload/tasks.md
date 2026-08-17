## 1. Sandbox identity per project root

Independent of everything below and the smallest real bug here — land it
first so the fan-out fix is not gated on the tooling design.

- [ ] 1.1 `up`: compare the current project root against `meta.json`'s
      recorded `project_root` (already stored — no new bookkeeping) and
      refuse to adopt a state dir bound to a different root
- [ ] 1.2 Error names both roots, the sandbox name, and `--name` as the
      fix, with a stable exit code per the error contract
- [ ] 1.3 Confirm same-root `up` stays idempotent — the check must
      distinguish a different root from a repeated run, not conflate them
- [ ] 1.4 `--name` override plumbed through the commands that resolve a
      sandbox; when given, the discovered manifest's declared name need
      not match
- [ ] 1.5 An overridden name is used consistently for state dir,
      `status`/`ps`/`logs`, and SSH host naming
- [ ] 1.6 Integration test with a **real** `git worktree`: two worktrees
      of one repo, same committed manifest. Without `--name` the second
      `up` fails naming both roots; with distinct `--name` values both
      sandboxes run independently
- [ ] 1.7 README known gaps / status: note the behavior change — two
      worktrees that silently shared a sandbox now fail loudly

## 2. Config surface

- [ ] 2.1 `[tools]` section in `src/config/`, rejecting unknown keys with
      the full key path like every other section
- [ ] 2.2 Credential request key, distinguishing env-var shape from
      single-file shape
- [ ] 2.3 Reject at parse time: a file-shaped credential naming a
      directory, and any glob or wildcard — a credential is per named
      file (`credentials` delta spec)
- [ ] 2.4 Unit tests for each rejection, asserting layer `config` and
      exit code 2
- [ ] 2.5 Regression test: a manifest with neither section produces a
      byte-identical `policy --render` and captured env to before this
      change — the migration plan's stated invariant

## 3. Tooling layer resolution

- [ ] 3.1 Resolve the declared tooling environment host-side at `up`, in
      the same trusted phase as the project environment; never from
      inside the boundary
- [ ] 3.2 Reject a tooling layer with no lock, or one that would pass a
      host binary through, at layer `provider` exit code 3 — this is the
      `host`-passthrough guard, not a nicety (design.md decision 1)
- [ ] 3.3 Compose the tooling env diff and store grants with the project
      environment at a fixed position, extending the existing
      "Fixed composition order" requirement rather than adding a parallel
      rule
- [ ] 3.4 Tooling grants are read-only and carry their own origin,
      distinct from `provider:` and `manifest:`
- [ ] 3.5 `up` fails naming the path if resolving the tooling layer would
      need write access outside the project root
- [ ] 3.6 Determinism test: repeated resolution of the same two layers
      yields byte-identical env and rendered policy
- [ ] 3.7 Test: rendered policy with a tooling layer differs from without
      it *only* by read-only grants carrying the tooling origin

## 4. Credentials

- [ ] 4.1 Env-var shape: deliver through the backend's credential
      mechanism, adding no filesystem grant
- [ ] 4.2 Assert the secret value never reaches the compiled policy,
      `meta.json`, or the logs — a test, not a code comment
- [ ] 4.3 File shape: grant exactly one file, read-only, using the
      backend's per-file grant. Granting the containing directory is
      explicitly not an acceptable implementation
- [ ] 4.4 Test: sibling files in the credential's directory stay
      unreadable, and the credential file itself is not writable
- [ ] 4.5 Baseline-denied paths stay denied even when named by a
      credential request
- [ ] 4.6 `up` prints exactly one disclosure line per exposed credential,
      including on a repeat `up` against an already-running sandbox
- [ ] 4.7 File credentials appear in `policy --render` with a credential
      origin, distinguishable from an ordinary manifest grant

## 5. CLI and doctor

- [ ] 5.1 `doctor`: report tooling-layer resolvability, distinguishing
      "none declared" from "declared but unresolvable" and naming the fix
- [ ] 5.2 Confirm no new top-level command — `--name` is a flag, tooling
      and credentials are manifest sections

## 6. End-to-end: an agent actually runs

- [ ] 6.1 Integration test, self-skipping like the other real-tooling
      tests: a project declaring an agent CLI in `[tools]` comes up and
      `exec -- <agent> --version` runs it **inside** the sandbox, with
      that agent absent from the project's own environment manifest
- [ ] 6.2 Auth test, both shapes: an env-var key, and a single-file
      credential. Skip honestly rather than faking a token if no
      credential is available in the test environment
- [ ] 6.3 Re-run the probe from proposal.md's Why and record the new
      result in the change — `node` present, credential readable — so the
      before/after is evidence, not assertion

## 7. Docs

- [ ] 7.1 `docs/decisions.md`: amend the "secret injection ... never via
      mounted files or plain env vars" position, naming the property that
      made it unachievable (subscription auth has no env-var form), per
      that file's own convention that a rejection whose premise stops
      holding is revisited rather than defended
- [ ] 7.2 `docs/decisions.md`: record why a host-binary tooling shortcut
      is rejected, naming the criteria it fails, so it is not proposed
      again as "just for agents"
- [ ] 7.3 Document the residual credential exposure plainly: in-sandbox
      code can read an exposed credential; the mitigations are narrowness
      and disclosure, not isolation from the code under edit
- [ ] 7.4 README: the fleet-of-agents claim can now be stated as
      supported — but only to the extent tasks 6.1/6.2 actually passed,
      and tier-qualified as always

## 8. Verification

- [ ] 8.1 `cargo build`, `cargo clippy`, `cargo fmt` clean
- [ ] 8.2 `openspec validate --all` passes with this change included
- [ ] 8.3 **macOS**: verify the tooling layer, both credential shapes,
      and the project-root check on a real macOS host, or report plainly
      that they are unverified there. Desktop agent fan-out is mac-heavy;
      a Linux-only result materially limits this change and must not be
      papered over
