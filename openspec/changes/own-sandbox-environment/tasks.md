# Tasks — Own the Sandbox Environment

## 0. Establish the baseline before changing it

> The measurement that motivated this change was taken from one host, one
> provider, one shell. Before removing 101 variables from every sandbox, know
> whether that number is representative.

- [~] 0.1 Record what reaches a sandbox on each provider row (flox, nix,
      devbox), not just flox.
      → **Moot, and the reason matters.** This existed to validate the
      *subtraction* approach's numbers, and D1's correction removed subtraction
      entirely. `env_clear` does not compare anything, so no per-provider
      distribution changes what it does. Left recorded rather than deleted:
      the number was load-bearing for a design that no longer exists.
- [~] 0.2 Do the same from a clean shell, separating "what any shell leaks"
      from "what this developer's shell happens to hold".
      → Moot for the same reason. The distinction mattered only for deciding
      what to subtract.
- [x] 0.3 Enumerate what breaks. Run the existing suite with the change applied
      and record every failure **before** fixing any of them.
      → **Nothing breaks: 416 passed, 0 failed**, at the default 12 threads,
      clippy and fmt clean.
      Two things this run cost before it could be believed. The first attempt
      **timed out at ten minutes**, which looked like the change hanging a
      keeper; a re-run at `--test-threads=4` passed, which would have been a
      comfortable and wrong place to stop. The difference was a **pre-existing
      flaky test**, confirmed by running the unmodified tree under `git stash`:
      FAILED / ok / FAILED across three runs. Fixed here rather than routed
      around — see 0.4 — because a suite that fails intermittently teaches
      people to re-run instead of investigate, and this change's whole claim
      rests on a red test meaning something.
      **Three limits on "nothing breaks", stated rather than implied**: ~19
      tests skip on macOS, so Linux-only paths are unverified; this covers
      group 1 alone, with `forward`, `HOME` and `forward_agent` not yet built;
      and the suite tests what it tests — a real project relying on an
      undeclared variable *will* feel this, which is the point of the change.
- [x] 0.4 Fix the flake found by 0.3, since it blocks trusting any result here.
      → `capability_set`'s test fixture keyed its directory on pid plus wall
      clock **nanoseconds**, which is not the same as unique: macOS's
      `SystemTime` granularity is coarser than a nanosecond, so two tests
      starting in one tick got the same directory and the first to finish
      deleted the other's fixture mid-run. An atomic counter is collision-free
      by construction rather than by hoping the clock is fine-grained enough.
      Five consecutive clean runs.

## 1. Do not inherit the ambient environment

- [x] 1.1 `cmd.env_clear()` before `.envs(env)` in `spawn_keeper` (D1).
      → **The whole of group 1, and the design's own correction.** The first
      plan was to capture devcroft's ambient environment and subtract it by
      name and value. That was re-solving a problem devcroft had already solved
      one layer down: every provider runs activation under
      `capture::canonical_base_env()` — the real `HOME` and a canonical `PATH`,
      nothing else — with `.env_clear()`, explicitly so the result depends on
      the manifest and lockfile rather than the operator.
      So `resolution.env` *is* the authoritative environment, and the 101
      leaked variables never came through it. They came from `spawn_keeper`
      letting a `Command` inherit, putting the operator's shell back on top of
      a result computed without it. **Not a missing filter — a discarded
      guarantee.**
- [~] 1.2 ~~Remove matching variables through the `unset` channel.~~
      **Superseded by 1.1.** `unset` remains what it was for: keys a provider's
      activation explicitly removed. Nothing about this change needs it.
- [~] 1.3 ~~Keep an essential set.~~ **Superseded by 1.1.** An essential set
      existed to protect what subtraction would wrongly delete. `env_clear`
      deletes nothing that was wanted: what a session needs comes from the
      provider's activation, which is what the project declared. If something a
      session needs turns out to have no provider supplying it, that is a gap
      to add explicitly and name — not a list of shell variables preserved by
      accident.
- [x] 1.4 Confirm the provider's own contribution is untouched: a variable
      activation *set*, one it *modified*, and one it *unset* must each behave
      as before.
      → `tests/sandbox_environment.rs`, the sibling of
      `flox_env_capture_is_deterministic` one layer up: that test closed this
      gap at *resolution*, this one closes it at *session*. Same decoy idea.
      Asserts the decoy absent, a `[vars]`-set variable present with the
      provider's value, and `PATH` still leading with the environment's own
      `bin`. Teeth-checked by removing `env_clear`.
      **The `unset` case is not covered and should not be faked**: only `HOME`
      and `PATH` are in `canonical_base_env`, so an activation would have to
      remove one of those to exercise it. Left to `Resolution::unset`'s own
      tests, which is where that mechanism lives.
      Two mistakes this file made before it was honest. It appended a second
      `[vars]` table to flox's manifest, which devcroft refuses as a duplicate
      key — and then **reported success**, because a failed `up` was being
      treated as an unsupported host. The guard is now narrow: with flox
      present and a Nix store reachable, a failed `up` is a failure. And its
      `PATH` assertion looked for a `/nix/store` prefix, which flox does not
      put there — it puts a symlink farm under `.flox/run/` — so the assertion
      was wrong rather than the code.
- [x] 1.5 **Audit what devcroft's own internals leave in a session.** Found by
      doing it: `DEVCROFT_SSH_HOST_KEY` was readable inside every sandbox,
      containing `BEGIN OPENSSH PRIVATE KEY`.
      The keeper needs both keys — it cannot read them off disk once
      restricted, which is why they travel as environment variables — but
      *sessions inherit the keeper's environment*, so a control-plane private
      key was being handed to the workload the sandbox exists to confine.
      Direct exploitation is limited: the control socket lives in a 0700
      baseline-denied directory, so nothing inside can bind a fake server. That
      is a separate control this one should not lean on.
      Fixed with an ordering constraint worth keeping: `remove_var` is sound
      only while the process is single-threaded, so `keeper_main` reads and
      removes both in its prologue — before the first `thread::spawn` — and
      passes them by value. `ssh::start_from_env` became
      `ssh::start(listener, backend, host_key, authorized_key)`; removing them
      inside the ssh function would have been too late.
      `DEVCROFT_CAPABILITY_PLAN` removed too — not a secret, since it is the
      sandbox's own policy, but this change's rule is that what the sandbox
      sees is decided rather than left over.
      Verified: no private key material inside, and the SSH e2e suite
      (`ssh_server_authenticates_the_real_client_key_and_binds_no_tcp`,
      pty channel, rsync) passes.

## 2. `[env] forward`

- [x] 2.1 Manifest key, names only, schema-checked.
      → `[env] forward = ["NAME", …]`. A name appearing in **both**
      `[env.vars]` and `forward` is **refused**, not resolved by precedence:
      the two say opposite things about where a value comes from, and which
      wins is not something the reader of a committed manifest should have to
      guess.
- [x] 2.1b **`[env.vars]` was a dead key and is now implemented.** Found while
      building `forward`: it parsed, was schema-exempted, and warned about `$`
      interpolation — and **nothing in `lifecycle/` ever applied it**. A user
      could write `[env.vars] RUST_LOG = "debug"`, get a clean validation, and
      have it reach nothing. The same shape of defect as `ssh.forward_agent`,
      and the third dead manifest key this change has turned up.
      Applied *after* the provider, so a project can override what activation
      set — the only ordering that makes the key useful.
      The split between the two keys is the security-relevant one: a manifest
      is committed, so `vars` are configuration and never secrets, and anything
      whose value lives on the host goes through `forward`.
- [x] 2.2 A forwarded variable the host does not set **warns and continues**
      (D3) — deliberately unlike a brokered route's missing credential, which
      fails `up`. Record the contrast where the code is, since two adjacent
      features answering the same question differently is exactly what gets
      "fixed" into consistency later.
      → Warned with the variable named, and *not* invented as an empty value —
      asserted both ways, since a silently-empty variable is worse than an
      absent one for anything that tests presence.
- [x] 2.3 Forwarded variables survive the subtraction, whatever their value.
      → Trivially true under D1's correction: `env_clear` subtracts nothing, so
      a forwarded value is simply inserted afterwards. Asserted anyway, because
      the property matters regardless of which mechanism provides it.
      Teeth-checked: stubbing out the forward loop and stubbing out
      `env.vars` each fail the test independently.

## 3. `HOME`

- [ ] 3.1 `HOME=<project>/.devcroft/<name>/home`, created at `up`.
- [ ] 3.2 Confirm the artifact directory's existing lifetime is right for this:
      ignored by `init`, removed by `rm`, surviving `down`.
- [ ] 3.3 Check what assumes the old `HOME`. `policy::compile` writes
      credential denials in `~/...` shorthand and `normalize_path_for_policy`
      rewrites `$HOME`-relative paths — if either now resolves against the
      sandbox-local home, the baseline denials it produces are nonsense.
      **This is the most likely place this change breaks something quietly.**

## 4. `ssh.forward_agent`

- [ ] 4.1 **Measure first**: with `SSH_AUTH_SOCK` present and the socket path
      granted, is the host agent actually reachable from inside? macOS treats
      unix-socket `connect` as a network operation, so `network.default = "deny"`
      may refuse it regardless. The answer decides whether 4.2 is implementable.
- [ ] 4.2 If reachable: implement the key — variable plus socket grant, in one
      place. If not: remove the key and the `add-mvp-core` scenario together,
      and say so in `docs/known-gaps.md`.
- [ ] 4.3 Either way, assert the *off* case, which is the one that is a security
      property rather than a feature.

## 5. Make the removal diagnosable

- [ ] 5.1 Report the removed set somewhere a user hits at the moment they need
      it (design Open Question 1 — `status` risks burying it).
- [ ] 5.2 The report names the manifest key that restores a variable. A
      diagnosis without a remedy just relocates the confusion.

## 6. Tests

- [ ] 6.1 A shell variable no provider sets does not reach the sandbox.
- [ ] 6.2 The control: a variable the provider *does* set arrives with the
      provider's value. Without this, an implementation that removed everything
      would pass 6.1.
- [ ] 6.3 `PATH` still contains the closure — the specific case a name-only
      comparison would have broken.
- [ ] 6.4 A `forward`ed variable arrives; an unset one warns without failing.
- [ ] 6.5 A session writes under `$HOME`, the write succeeds, and nothing
      appears in the host user's home.
- [ ] 6.6 Teeth-check the subtraction: disable it and confirm 6.1 fails.

## 7. Say what changed

- [ ] 7.1 `docs/known-gaps.md`: the gap this closes, stated as it was —
      **no environment filtering existed at all**, so every exported credential
      was inside every sandbox while the baseline denied the directories those
      same credentials live in. A closed gap is still worth publishing when it
      was open in a shipped version.
- [ ] 7.2 README/migration: a project may need one `forward` line. The failure
      is a tool not finding a variable, far from its cause.
- [ ] 7.3 `docs/decisions.md`: the secret-injection position no longer needs the
      retraction `add-agent-workload` task 7.1 drafted. `forward` is the honest
      simple answer and brokering is the strong one; neither is "never via env
      vars", and the entry should say what is true rather than what was hoped.
- [ ] 7.4 Record in `add-agent-workload` that `[hooks] post_create` running an
      agent's **official installer** is now possible, since `HOME` is writable —
      a much cheaper answer to its tooling problem than devcroft shipping
      runtimes, and one that keeps the vendor's own install path rather than
      devcroft reimplementing it.
