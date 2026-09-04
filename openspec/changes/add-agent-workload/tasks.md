## 1. Sandbox identity per project root

Independent of everything below and the smallest real bug here — land it
first so the fan-out fix is not gated on the tooling design.

- [x] 1.1 `up`: compare the current project root against `meta.json`'s
      recorded `project_root` (already stored — no new bookkeeping) and
      refuse to adopt a state dir bound to a different root
- [x] 1.2 Error names both roots, the sandbox name, and `--name` as the
      fix, with a stable exit code per the error contract
- [x] 1.3 Confirm same-root `up` stays idempotent — the check must
      distinguish a different root from a repeated run, not conflate them
- [x] 1.4 `--name` override plumbed through the commands that resolve a
      sandbox; when given, the discovered manifest's declared name need
      not match
- [x] 1.5 An overridden name is used consistently for state dir,
      `status`/`ps`/`logs`, and SSH host naming
- [x] 1.6 Integration test with a **real** `git worktree`: two worktrees
      of one repo, same committed manifest. Without `--name` the second
      `up` fails naming both roots; with distinct `--name` values both
      sandboxes run independently
- [ ] 1.7 README known gaps / status: note the behavior change — two
      worktrees that silently shared a sandbox now fail loudly

> **Landed. Two things the implementation found, both by running it:**
>
> - **Placement was the fix, not the comparison.** The first version put the
>   check after the health decision, where it never fired for the case it
>   exists to catch: a *healthy* sandbox returns `AlreadyUp` early, and
>   adopting a healthy sandbox from the wrong root is precisely the silent
>   failure. Caught by the worktree test, not by review. "Does this state dir
>   belong to me" has to be answered before "should I adopt it".
> - **A test was relying on the bug.** `symlink_escape_cli` hardcoded a
>   sandbox name with no pid while its project root varied by pid, so a
>   leftover state dir from an earlier run belonged to a different root — and
>   was silently adopted. It now fails loudly, which is the change working.
>
> `--name` is an *override*, distinct from the positional `[name]` selector:
> it renames this project's sandbox for one invocation, so the discovered
> manifest deliberately need not agree. Applied to the parsed manifest rather
> than threaded through callers, which makes 1.5 hold by construction — the
> state dir, `status`/`ps`/`logs` and SSH naming all read
> `manifest.sandbox.name`.

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

## 6b. Denial feedback: the agent learns why it was refused

> The technique taken from nono's registry packs (`docs/prior-art.md`).
> An agent that hits a policy denial today gets a bare `EPERM`: it does not
> know it is sandboxed, what it was granted, or that `why` answers exactly
> that. The likely outcome is a wrong inference — "the file does not exist",
> "this needs sudo" — and either a give-up or an absurd request.
>
> **Depends on `fix-silent-policy-degradation`.** Measured: `why` run from
> inside a sandbox does not fail, it inverts — `~/.local/share/devcroft` is
> baseline-denied and not overridable, so `compile_with_provider_grants`
> silently drops the provider grants and reports `DENIED / not granted by any
> rule` for a store path the sandbox is actually granted. Wiring an agent to
> a confidently inverted answer is worse than leaving it with the `EPERM`.

- [ ] 6b.1 Confirm `fix-silent-policy-degradation` has landed and that `why`
      from inside a session returns the same verdict and origin as on the
      host. Everything below is unsafe to ship before that.
- [ ] 6b.2 Decide how the agent reaches `why`, and record the trade:
      **(a)** a hook script that reads the policy artifact directly — no new
      policy surface, answers "what was I granted"; **(b)** grant read+exec
      on `current_exe()`, folded into the grants in `up()` the way the
      resolved shell's store root already is, which buys real per-path `why`
      but lets an agent run *every* devcroft subcommand inside the sandbox.
      (b) needs its own audit of that surface — `rm`, `up --recreate` — and
      is not a free consequence of (a).
- [ ] 6b.3 Implement (a) as the shipped path: a denial-triggered hook that
      fires on the agent's tool-failure event, gates on a denial signature so
      ordinary failures pass through untouched, and injects the sandbox's
      grants plus the instruction to diagnose **before** asking the user for
      permission.
- [ ] 6b.4 Wiring delivery: `devcroft init --agent` writes it once and the
      user commits it. Rejected alternatives, recorded rather than
      re-litigated: silently generating it at `up` (devcroft's requirement
      appearing unasked in the user's tree — the same criticism
      `decouple-service-supervisor` levelled at the process-compose
      coupling), and requiring the user to hand-install it.
- [ ] 6b.5 State the limitation in the change rather than discovering it
      later: this is **agent-specific by necessity**. devcroft cannot rewrite
      the `EPERM` a tool sees — it comes from the kernel through whatever
      binary the agent ran — so an interception point is required, and the
      agent's own hook system is the only one available. Per-agent wiring,
      not a general mechanism.
- [ ] 6b.7 Adopt ArcBox's credential-forwarding shape, which is better than
      this change's current plan and costs nothing to copy: forward only
      `ANTHROPIC_*`/`CLAUDE_*`, for that session only, never written into any
      image or sandbox record, every other host variable left behind
      (`docs/prior-art.md`). Independent evidence for 7.1's premise arrives
      with it: a project with a real microVM boundary states that OAuth
      credentials are *deliberately not copied in*, so subscription auth is
      unsolved there too, not merely unsolved here.
- [ ] 6b.6 Test that the gate is real: an ordinary failure (a genuinely
      missing file) must **not** trigger the sandbox explanation. A hook that
      fires on everything teaches the agent to ignore it.

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
