# Tasks — Sandbox Provider Resolution

## 0. Spike first — this sizes the whole change

- [x] **Determine whether Flox can separate materialization from
      `hook.on-activate` at all** (design.md open question 1).
      **Answered: not by flox, but yes by devcroft.** Flox exposes no public
      materialize path, pre-hook context, or separate hook runner — confirmed
      across `--mode dev`, `--mode run` and `--no-start-services`. The question
      was framed as "can flox do this?", and the useful reframing was "can the
      split be constructed without flox's help?" — which it can, by deriving a
      hook-free copy of the environment (P2d). The original framing would have
      concluded "refuse flox" from a true premise.
- [ ] Measure what `flox activate` actually needs when confined: run it under a
      deny-by-default profile against a real project and add grants until it
      works. Record the minimal set. Candidates to check: the provider's config
      and data directories, `TMPDIR`, the terminal.
      **Note what is deliberately absent from that list:** the nix daemon
      socket. It used to appear here as an ordinary candidate grant, which is
      exactly the conflation P2a rejects — measuring "does it work once I add
      the daemon" would qualify a profile that hands host-global
      materialization authority to project shell. Measure the *resolver's*
      needs separately (below) and keep the hook's profile without it.
- [ ] Measure what a **trusted resolver** needs, separately from the hook: what
      materialization requires when no project code is running. This is the
      half that legitimately holds daemon authority, and the point of measuring
      it apart is to know exactly how much authority the split is protecting.
- [ ] Qualify the hook-free paths for the two eligible providers —
      `nix print-dev-env --json` and `devbox shellenv --pure` — as running
      inside the provisioning worker with no daemon connection.
- [x] **Establish that flox can be split by devcroft, without upstream.**
      Measured live (design.md P2d): materializing from a derived copy of the
      environment with `[hook]` removed yields a byte-identical locked package
      set and an identical store path, and the hook does not run. The hook then
      works when run inside that already-materialized environment. So the
      earlier plan — refuse flox pending upstream — is not needed.
- [x] Implement the derived hook-free environment
      (`flox::derive_hook_free_env`): copy the project's flox environment
      (manifest, lock, `env.json`), strip the `[hook]` table, and materialize
      from that. The project's `.flox/` is read, never written.
      **One correction to the plan as written:** "outside the project" is
      wrong and would have broken the sandbox. flox's `PATH` points at its
      own `run/` symlinks inside the environment directory, so the sandbox
      must *read* that directory at runtime — and anywhere outside the granted
      project root is unreachable to it. It therefore lives under the
      project's `.devcroft/` artifact directory, the same reasoning (and the
      same gitignore) as the service artifacts. Content-addressed by the
      environment fingerprint, so a manifest change derives afresh rather than
      reusing a stale copy.
- [x] Run the captured script inside the sandbox
      (`hooks::run_activation_script`, called from `up_process` before
      devcroft's own hooks — a `post_create` that depends on the environment
      the script sets up would otherwise run first and fail).
- [ ] Correct the flox context variables when running the hook. **Not yet
      done — the mechanism works without it, which is exactly why it is worth
      keeping on the list rather than assuming it is fine.** Measured, the
      ones that point at the derived directory are `FLOX_ENV`,
      `FLOX_ENV_PROJECT`, `FLOX_ENV_DIRS`, `FLOX_ENV_DESCRIPTION` and
      `FLOX_PROMPT_ENVIRONMENTS`. `FLOX_ENV_PROJECT` is the one that matters —
      hooks use it to find the project root, and uncorrected a hook would
      resolve paths into devcroft's scratch directory.
- [ ] Check whether `[profile]` scripts run on the same path and need the same
      treatment. **Unmeasured** — do not assume either way.
- [ ] Detect a derived environment whose lock has drifted from the project's,
      and re-derive rather than materializing something the project did not
      declare.
- [ ] Implement the fail-closed diagnostic for what remains genuinely refused:
      activation code that needs materialization authority *while running as
      project code*. The error must distinguish this from "this provider
      cannot be confined", since the fix is to declare the dependency in
      `[install]` rather than to wait for anything.
- [ ] Repeat for `devbox`.
- [ ] Confirm the environment can be written to a descriptor the supervisor
      holds, across the boundary, without a shell round trip.
- [ ] **If the minimal grant set turns out to be most of `$HOME`, stop and
      reconsider** — the change would then be confinement in name only.

## 1. Provisioning policy

- [ ] Add the provisioning profile to the manifest schema.
- [ ] Define the default profile from the spike's findings.
- [ ] Extend `policy --render` and `why` to cover both profiles with origin
      attribution.
- [ ] Report degradation where the backend cannot enforce a declared aspect.

## 2. Provisioning sandbox

- [ ] Build the provisioning sandbox from the compiled provisioning profile,
      reusing the existing sandbox construction path.
- [ ] Substituted home directory, with declared paths bound in.
- [ ] Capture the environment across the boundary as data.
- [ ] Apply the existing baseline diff, store grants and staleness
      fingerprinting to the captured environment, unchanged.

## 3. Failure attribution

- [ ] Distinguish policy denial from provider failure in `up`'s output.
- [ ] Name the denied path or interface.
- [ ] Test both paths: a hook denied a path it needs, and a hook that is simply
      broken.

## 4. Providers

- [ ] Route flox through the provisioning sandbox.
- [ ] Route devbox through it, keeping the existing lockfile byte-comparison.
- [ ] Give nix a provisioning profile for inspectability; its resolution path is
      unchanged.
- [ ] Replace the host-execution warning with an accurate statement of
      provisioning's reach.

## 5. Validation

- [ ] A project whose hook runs a package install still provisions correctly.
- [ ] A hook writing to an undeclared home path does not affect the real home.
- [ ] A hook attempting to read credentials outside the profile is denied.
- [ ] Every existing sample still provisions.
- [ ] Document the behavioural difference from running activation by hand.

## 6. Deferred decisions (see design.md)

- [ ] Network during provisioning: a real provisioning allowlist, from
      `add-egress-proxy`. Not a binary on/off — see design.md open question 2.
      **No longer deferred-and-unowned: `add-egress-proxy` has shipped, and
      its "Network policy is declared per context" requirement moved into
      this change** (`specs/network/spec.md`). The mechanism exists —
      `CompiledPolicy::network_proxy_port` is per-compilation, so a
      provisioning compilation gets its own proxy and allowlist without new
      machinery. What is missing is only the second context, which is this
      change's own subject.
- [ ] Cache sharing across agents: read-only share plus per-agent overlay.
      **Decide before fleet work**, not after.
- [ ] macOS fidelity statement.
