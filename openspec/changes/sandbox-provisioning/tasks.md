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
- [x] Measure what `flox activate` actually needs when confined: run it under a
      deny-by-default profile against a real project and add grants until it
      works. Record the minimal set. Candidates to check: the provider's config
      and data directories, `TMPDIR`, the terminal.
      **Note what is deliberately absent from that list:** the nix daemon
      socket. It used to appear here as an ordinary candidate grant, which is
      exactly the conflation P2a rejects — measuring "does it work once I add
      the daemon" would qualify a profile that hands host-global
      materialization authority to project shell. Measure the *resolver's*
      needs separately (below) and keep the hook's profile without it.
      **Measured for the hook-free activation (design.md P6, 2026-10-09):**
      `flox activate` on the derived copy works with no daemon, no network
      and an empty substituted home, giving the same `PATH` and `FLOX_ENV`.
      Minimal set: the project and its derived copy read, the derived
      copy's `.flox/log` and `run/` written, `/nix/store` read, a
      substituted home **with the XDG variables set** (with `HOME` alone flox
      wrote its env registry into the real home). The hook itself already
      runs inside the runtime sandbox (P2d), so it was not part of this.
- [ ] Measure what a **trusted resolver** needs, separately from the hook: what
      materialization requires when no project code is running. This is the
      half that legitimately holds daemon authority, and the point of measuring
      it apart is to know exactly how much authority the split is protecting.
      **First answer (P6):** the daemon, plus the network unless the store
      already holds everything (a never-installed devbox copy, confined,
      failed on `cache.nixos.org`). `devbox install` runs no `init_hook`.
      For nix the trusted half is all of resolution: evaluating a local
      flake writes the project's source into the store. The trusted half's
      own minimal filesystem grants are not yet measured.
- [x] Qualify the hook-free paths for the two eligible providers —
      `nix print-dev-env --json` and `devbox shellenv --pure` — as running
      inside the provisioning worker with no daemon connection.
      **devbox qualifies; nix does not (P6).** `devbox shellenv --pure` with
      the daemon unreachable, no network and a substituted home was
      byte-identical to the unconfined run (apart from `HOME` and devbox's
      hash over it) and wrote nothing. `nix print-dev-env --json` fails even
      fully warm: the flake's source is copied into the store on every
      evaluation, and a read-only store fails on that copy's lock.
      **devenv, the provider this item did not name, measured too:** it is
      nix's case. Evaluation needs the daemon, and a confined `build shell`
      that succeeded did so only from devenv's evaluation cache; with
      `--no-eval-cache` it failed on the hidden socket. It also needs Nix's
      fetcher cache from the real home. No project shell runs either way.
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
- [x] Correct the flox context variables when running the hook. **Not yet
      done — the mechanism works without it, which is exactly why it is worth
      keeping on the list rather than assuming it is fine.** Measured, the
      ones that point at the derived directory are `FLOX_ENV`,
      `FLOX_ENV_PROJECT`, `FLOX_ENV_DIRS`, `FLOX_ENV_DESCRIPTION` and
      `FLOX_PROMPT_ENVIRONMENTS`. `FLOX_ENV_PROJECT` is the one that matters —
      hooks use it to find the project root, and uncorrected a hook would
      resolve paths into devcroft's scratch directory.
      **Done (2026-10-10), and the list above was wrong for current flox.**
      Re-measured by diffing `flox activate -- env` in the project against
      the derived copy: `FLOX_ENV_DESCRIPTION` and `FLOX_PROMPT_ENVIRONMENTS`
      do not differ, while two the list missed do: `FLOX_ENV_CACHE` (where
      flox's own idiom keeps a hook's virtualenv; it sat in a directory keyed
      by the environment's fingerprint, so every manifest edit discarded it)
      and `_FLOX_ACTIVE_ENVIRONMENTS`. `flox::restore_project_context` points
      `FLOX_ENV_PROJECT`, `FLOX_ENV_CACHE` and `_FLOX_ACTIVE_ENVIRONMENTS` back
      at the project. `FLOX_ENV` and the `run/` paths are left on the derived
      copy deliberately: both links resolve to the same store path, and the
      project's own `run/` may never have been built. The in-sandbox hook test
      (`tests/flox_derived_env.rs`) now records both variables from inside
      the sandbox under real flox; without the fix it reads
      `.devcroft/flox-env-…` for both.
- [x] Check whether `[profile]` scripts run on the same path and need the same
      treatment. **Unmeasured** — do not assume either way.
      **Measured (2026-10-10, flox in this devcontainer):** they do not. A
      scratch environment with `[hook].on-activate` and `[profile]` `common`
      and `bash` scripts, each touching a marker, under devcroft's own call
      (`flox activate -- env -0`): the hook ran, neither profile script did.
      Profile scripts initialise interactive shells, and command mode starts
      none, so they are not a host-side execution path.
      **The other side of that is a silent drop:** devcroft keeps `[profile]`
      in the derived copy and never runs it anywhere, so a project relying
      on it gets nothing in a session and is not told. That breaks the
      "degraded capabilities are surfaced" rule and is tracked below.
- [x] Say so when a flox environment declares `[profile]`: devcroft never runs
      it (sessions are not flox interactive shells), and the project should
      hear that once at `up` rather than find its aliases missing.
      **Done:** `flox::profile_warning`, printed once by `up` and once per
      `fleet up` (not per agent), naming the declared scripts, why, and the
      fallback (`[vars]`, `[hook].on-activate`; aliases and functions have no
      equivalent). Unit-tested; checked by hand with the real binary.
- [x] Detect a derived environment whose lock has drifted from the project's,
      and re-derive rather than materializing something the project did not
      declare.
      **Done (2026-10-10), and the drift was real, not hypothetical.**
      Measured on `flox-clap-sample`: devcroft copied the lock verbatim, but
      flox embeds the manifest in its lock, and the derived manifest had
      lost its hook, so flox re-locked the copy on activation (the embedded
      manifest rewritten). Re-locking re-resolves; the 20 pins happened to
      survive because flox keeps existing pins for unchanged entries, which
      is flox's habit, not something devcroft checked. Now: the derived lock
      is the project's with `hook` gone from its embedded manifest
      (`lock_without_hook`), which flox then leaves untouched (measured); a
      reused copy whose pins drifted is derived again; and after activation
      the copy must pin what the project pins (outputs, derivation, locked
      URL per package and system) or `up` fails at the provider layer
      naming the packages (`lock_drift`). The real-flox closure test now
      compares the derived lock's **bytes**: a re-lock yields the same JSON
      value, so the first, value-based version passed with the old verbatim
      copy. `flox list` locks a project without running its hook
      (measured), which is how the test locks it.
      **One refinement the full suite forced:** a project whose own lock is
      behind its manifest (`flox init`, then an edit) makes every copy
      re-lock, and the first version refused that, with a remedy naming
      `flox lock`, which flox 1.14 does not have. flox re-locks such a
      project itself, so that is the project's state, not drift devcroft
      introduced: it is now a warning naming what is unlocked, with `flox
      list` as the remedy. Drift is refused only against a lock whose
      packages match the manifest's `[install]` (`stale_project_lock`).
      **Not tested end to end:** the refusal itself. Real flox keeps pins on
      a re-lock, so genuine drift cannot be provoked; `lock_drift` is
      unit-tested and the branch is a two-way match on it.
- [ ] Implement the fail-closed diagnostic for what remains genuinely refused:
      activation code that needs materialization authority *while running as
      project code*. The error must distinguish this from "this provider
      cannot be confined", since the fix is to declare the dependency in
      `[install]` rather than to wait for anything.
- [x] Repeat for `devbox`.
      Done with the measurement above.
- [ ] Confirm the environment can be written to a descriptor the supervisor
      holds, across the boundary, without a shell round trip.
- [x] **If the minimal grant set turns out to be most of `$HOME`, stop and
      reconsider** — the change would then be confinement in name only.
      **It did not:** an empty substituted home is enough for both devbox
      and flox (P6).

## 1. Provisioning policy

- [x] Add the provisioning profile to the manifest schema.
      `[provisioning.network] allow = [...]` (2026-10-12), unknown keys named by
      full path. Only the network: the filesystem is the runtime one, and the
      home is already the sandbox's own.
- [x] Define the default profile from the spike's findings.
      The runtime filesystem, and **no network, nothing inherited** from the
      runtime allowlist (`CompiledPolicy::for_provisioning`). The network
      spec's "Provisioning inherits nothing by default".
- [ ] Extend `policy --render` and `why` to cover both profiles with origin
      attribution.
      **`--render` done (2026-10-12):** a provisioning section with its allowlist
      and origin `manifest:provisioning.network.allow`, shown even when empty.
      `why` does not yet answer for the provisioning context.
- [ ] Report degradation where the backend cannot enforce a declared aspect.

## 2. Provisioning sandbox

- [x] Build the provisioning sandbox from the compiled provisioning profile,
      reusing the existing sandbox construction path.
      **Built (2026-10-12) as a separate process**, the mechanism the owner
      chose: `up` runs `__provision` (`lifecycle::up::run_provisioning`) through
      the same mount and network confinement as the keeper, under the
      provisioning plan, with a proxy of its own holding the provisioning
      allowlist, stopped when it exits. The keeper then starts under the runtime
      plan and does not run the script again. `provisioning_network.rs`: the
      hook reaches only the provisioning host, a session only the runtime one,
      and the hook's exports reach the session; with the hook back in the keeper
      the hook cannot reach its host. **Enforcement is the proxy each process
      can reach**, not the plan's domain list: a mutant swapping only the plan's
      network changed nothing, which is why the test's mutant is the old path.
- [x] Substituted home directory, with declared paths bound in.
      The provisioning process gets the sandbox's own home (`.devcroft/<name>/
      home`), persistent across runs, so a cache written there survives; no
      declared extra paths yet.
- [x] Capture the environment across the boundary as data.
      What the script exported is diffed **in the provisioning process**,
      against what it started from, and handed to the keeper as JSON
      (`hooks::run_activation`, `adopt_activation`). A keeper-side diff was the
      first version and was wrong: the two differ in the proxy variables, so
      every session would have been handed the provisioning proxy.
- [ ] Apply the existing baseline diff, store grants and staleness
      fingerprinting to the captured environment, unchanged.

## 3. Failure attribution

- [ ] Distinguish policy denial from provider failure in `up`'s output.
- [ ] Name the denied path or interface.
- [ ] Test both paths: a hook denied a path it needs, and a hook that is simply
      broken.

## 4. Providers

- [x] Route flox through the provisioning sandbox.
      A flox hook (and devenv's `enterShell`) runs in the provisioning process.
      **Not fleet:** a fleet agent's keeper still runs the hook itself, under
      the agent's runtime policy; fleet builds agents without `up`.
- [ ] Route devbox through it, keeping the existing lockfile byte-comparison.
- [ ] Give nix a provisioning profile for inspectability; its resolution path is
      unchanged.
- [ ] Replace the host-execution warning with an accurate statement of
      provisioning's reach.

## 5. Validation

- [x] A project whose hook runs a package install still provisions correctly.
      Shown for the part that differs, reaching a registry: the hook reaches a
      provisioning-allowed host (a local server standing in) that the runtime
      denies. A real `npm ci` was not run.
- [ ] A hook writing to an undeclared home path does not affect the real home.
- [ ] A hook attempting to read credentials outside the profile is denied.
- [x] Every existing sample still provisions.
      The full suite with the provisioning process in place (2026-10-12),
      devenv on `PATH` and `--features test-support`: 653 passed, 0 failed,
      bringing every provider's samples up (flox with a hook, devenv's
      `enterShell`, devbox, nix). Skipped: Swift (macOS-only) and rsync.
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
