## 0. Qualify devbox against a real install, before writing any code

> **Complete. The change survives, and it nearly did not.** This group
> existed because the proposal and design were written entirely against
> devbox's documentation, with nothing verified. Measuring devbox 0.18.0
> corrected four drafted requirements, falsified one rationale, and
> **inverted decision 1** — the drafted capture mechanism turned out to
> run project code during the trusted phase, a two-phase violation, and
> the mechanism the draft rejected as fragile is the one that qualifies.
>
> devbox passes all six criteria, but criterion 4 passes only because
> `shellenv` is used instead of `run`. That is a property of devcroft's
> choice, not of devbox's caution, so it is guarded by a test (1.6)
> rather than by a comment.

- [x] 0.1 Install devbox in `.devcontainer/Dockerfile`, pinned by version
      the way `NONO_VERSION`/`runsc` already are. Record the version
      every measurement below was taken against — a later devbox may
      behave differently and the findings must say what they apply to.
      **Done: `DEVBOX_VERSION=0.18.0`** (released 2026-08-16), official
      GitHub release tarball verified against the published
      `checksums.txt`, not the `get.jetify.com` installer script — that
      script resolves the latest release itself, which is the drift the
      pin exists to prevent. `DEVBOX_NO_TELEMETRY=1` set for the same
      reason `NONO_NO_UPDATE_CHECK` is.
      Same caveat the flox and runsc blocks carry: **not exercised
      through a container rebuild** (no docker socket from inside a
      running devcontainer). Verified instead by performing the
      identical steps live — download, checksum, extract — into
      `~/.local/bin`, which is what every measurement below runs
      against. Confirmed working end to end on `linux_arm64`:
      `devbox init`, `devbox add ripgrep` (resolved through
      `devbox-search`, fetched into `/nix/store` via this image's
      existing nix 2.31.5), and `devbox run -- rg --version` executing
      the binary from the closure.

> **Finding from 0.1, and it moves 0.3 from an edge case to the default
> case:** `devbox init` generates a `shell.init_hook` into every new
> `devbox.json` — the stock template runs an `echo`. So the criterion-4
> question in 0.3 is not "what happens to projects that define a hook",
> it is "what happens to devbox projects", since having one is the
> out-of-the-box state. Whatever 0.3 measures applies to essentially the
> whole population, and a mitigation that depends on projects not using
> init hooks is not available.
>
> **Three more findings from 0.1, each of which corrected a spec that
> had already been written against the documentation.** Recording them
> together because they share a cause: the first draft assumed devbox's
> lockfile and CLI mirror nix's, and they do not.
>
> 1. **A fresh devbox project has no lockfile.** `devbox init` writes
>    only `devbox.json`; `devbox.lock` appears at the first
>    `devbox add`. So a zero-package project — legitimate, if minimal —
>    has no lock at all, and the drafted "require `devbox.lock`"
>    precondition would have rejected it for a reason untrue of it.
> 2. **Resolutions are recorded per system, and the set is partial.**
>    `ripgrep@latest` locked to `aarch64-darwin`, `aarch64-linux`,
>    `x86_64-linux` — and not `x86_64-darwin`. So a committed lockfile
>    can be present and complete for another platform while leaving the
>    running one unresolved: a file-presence check passes and the
>    package still resolves at `up`, which is the one thing the
>    precondition exists to prevent. Both 1 and 2 are why the
>    `env-provider` requirement is now phrased as "nothing resolves at
>    `up`" rather than "a lockfile exists".
> 3. **There is no `devbox lock` subcommand.** `devbox --help` lists
>    `install` and `update`; the lockfile is a side effect of installing.
>    The drafted `init` hint pointed at a command that does not exist.
>
> One drafted *rationale* was also false, separately from the
> requirements: the `cli` spec justified ranking devbox above a bare
> flake by claiming a root `flake.nix` in a devbox project is generated
> from `devbox.json`. devbox writes its generated flake to
> `.devbox/gen/flake/flake.nix` and never to the project root, so a root
> flake beside a `devbox.json` was written deliberately. The ordering is
> kept as a deterministic tiebreak; the false reasoning is removed
> rather than replaced with a better-sounding one.
- [x] 0.2 **Decide the capture mechanism by running both**
      (design.md decision 1). **Measurement inverted the drafted
      decision.** `devbox run` — the drafted choice — runs the project's
      init hook, `--pure` included, which is a two-phase violation and
      disqualifies it on a ground the draft never considered.
      `devbox shellenv` does not run the hook in any variant; even
      `--init-hook` merely appends one `. .devbox/gen/scripts/.hooks.sh`
      line to the emitted text rather than executing anything.
      The draft's *other* claim held though: shellenv output is not a
      clean assignment list — it carries multi-line values that are
      themselves shell (nixpkgs' `mkShell` `$out` snippet) and ends with
      an `if ! type refresh …; alias …; fi` block plus `hash -r`, so
      line-parsing it would silently produce a wrong environment.
      **Chosen: `sh -c 'eval "$(devbox shellenv --pure)"; env -0 > tmp'`**
      from devcroft's canonical baseline — devbox's own shell code sets
      the environment up, `env -0` reads it back machine-readably, and
      no parser and no hook are involved anywhere.
      `--pure` is mandatory: without it the capture carried this
      operator's `CLAUDECODE`, `AI_AGENT`, and a `BROWSER` pointing into
      a VS Code server install. With it, a decoy `PATH` prepend and
      decoy variables did not survive, and two runs from differently
      polluted shells produced identical captures
- [x] 0.3 **Determine whether activation runs a project-defined init
      hook.** **Passes criterion 4 — but only via 0.2's mechanism.** A
      hook appending to a sentinel file: did not run under
      `devbox shellenv`, `--pure`, or `--init-hook`; **did** run under
      `devbox run` and `devbox run --pure`. So the qualification comes
      from choosing the one entry point that does not activate, not from
      devbox being careful — which makes decision 1 load-bearing for
      correctness. Task 1.6 asserts this as a test rather than trusting
      it
- [x] 0.4 **Determine whether machine-global devbox packages leak into
      the capture.** **No leak.** `devbox global add hello` landed in
      `~/.local/share/devbox/global/default/devbox.json`, and the
      project capture showed it nowhere — not on `PATH`, not in any
      variable's value. devbox's global profile is opt-in at the shell
      level (it instructs the user to add `eval "$(devbox global
      shellenv)"` to an rcfile) and project activation does not consult
      it
- [x] 0.5 **Confirm the resolved environment is a self-contained
      closure.** **It is.** The captured `PATH` has 24 entries: 20
      `/nix/store` paths covering the full stdenv — `gcc-wrapper`,
      `gcc`, `glibc`, `binutils`, `coreutils`, `bash`, `gnumake`,
      `findutils`, `gnused`, `gnugrep`, `gawk`, `gnutar`, `patch`, and
      more — three project-root `.devbox` paths, and `/usr/bin`.
      **`/usr/bin` is at position 22, after every store entry**, so
      `cc`/`gcc`/coreutils all resolve into the store first and the host
      copy is only a trailing fallback devcroft's policy will deny and
      `PATH` lookup will skip. Closure tier is a measured claim about
      devbox, not an inference from it being Nix-backed
- [x] 0.6 Write the findings from 0.2–0.5 into design.md as decided
      values, replacing the conditional language. Done: decision 1
      rewritten (with the inversion stated rather than quietly
      corrected), new decision 1a added for the profile-symlink finding
      below, decisions 2 and 3 now carry their measured outcomes

> ## Finding that is not about devbox, and is more urgent than it
>
> Asking 0.3's question for the first time exposed the same defect in
> **both providers devcroft already ships**. Measured, not inferred:
>
> - **flox**: `flox activate -- env -0` — the exact command in
>   `flox.rs` — runs `[hook].on-activate`. A hook appending to a
>   sentinel file ran during resolution.
> - **nix**: `nix develop --no-update-lock-file --command sh -c 'env -0'`
>   — the exact command in `nix.rs` — runs the devShell's `shellHook`.
>   Same sentinel, same result.
>
> Both execute arbitrary project-supplied shell **host-side, before any
> restriction, with the invoking user's full network and filesystem
> access**. That contradicts an invariant CLAUDE.md states outright:
> "Hooks are project code and never get provisioning privileges; a hook
> that needs the network needs an allowlist entry." It is also the exact
> violation this change rejected `devbox run` for — so devbox, via
> `shellenv --pure`, would ship as the *only* provider that does not
> have it.
>
> **The fix is concrete and already demonstrated here.** The pattern is
> identical across all three providers: the "run a command inside the
> activated shell" entry point runs hooks, and the "emit the environment
> as text" entry point does not. Measured for nix: `nix print-dev-env`
> did **not** run the shellHook — it emits it as data (the `shellHook`
> variable) for the caller to decide about. `nix.rs` explicitly rejected
> `print-dev-env` as "a bash script to be sourced, not a clean env
> dump", which is word-for-word the reasoning this change's own draft
> used for devbox before measurement inverted it. So nix can be fixed by
> the same eval-then-`env -0` hybrid decision 1 settled on. Whether flox
> has a non-activating equivalent is unmeasured.
>
> **Deliberately not fixed in this change.** It is a defect in
> `add-mvp-core` (flox) and `add-nix-provider` (nix), it is
> security-relevant, and folding a two-provider fix into a
> third-provider proposal would bury it. It needs its own change, and
> should be prioritized above this one.
>
> **Finding from 0.5, and it changes task 1.3:** a devbox project's
> declared packages are **not** on `PATH` as store paths. They arrive
> through `<project>/.devbox/nix/profile/default`, a symlink chain
> (`default-1-link` → `/nix/store/…-profile`) rooted inside the project
> root; the underlying store paths appear in `HOST_PATH`. Deriving
> grants by scraping `PATH` for `/nix/store/` prefixes — which is what
> the other two closure providers can do — would grant the stdenv
> closure and **miss every package the project actually declared**. See
> design.md decision 1a.

> ## Corrections from adversarial review, after the change first shipped
>
> Reviewing the delivered implementation adversarially — trying to break
> it rather than confirm it — found that **task 1.2's precondition did
> not actually deliver the spec sentence it was written for**, plus one
> false clause in the evidence for 1b. Recorded here rather than folded
> silently into the tasks above, since the whole point of group 0 was
> that measurement outranks reasoning:
>
> - **`up` rewrote the user's `devbox.lock`** — the exact thing
>   `env-provider` says resolution SHALL NOT do. A project whose every
>   *declared* package was locked still passed, because `devbox.lock`
>   also carries devbox's own base nixpkgs entry, which no per-package
>   check can see. Confirmed through the real binary by md5 before/after.
>   Fixed by comparing the lockfile's bytes across capture and restoring
>   + failing on any change (design.md decision 1c) — a byte comparison
>   rather than a bigger precondition, because the base entry's key is
>   not a constant (a project pinning `nixpkgs.commit` locks under a
>   different key) and predicting it would mean reimplementing devbox's
>   resolution rules.
> - **"Declares no packages" turned out not to mean "nothing to
>   resolve".** A zero-package devbox project still gets its stdenv from
>   that same unpinned base, so it is not reproducible without a
>   lockfile. The `env-provider` scenario asserting otherwise, the `cli`
>   scenario mirroring it, `init`'s advice, and three tests were all
>   wrong in the same way and are corrected.
> - **One leg of 1b's evidence was false.** It cited the
>   `cache.nixos.org` fetch as proof of the violation; a cold-store
>   measurement shows the *permitted* case fetches too (13 MiB) while
>   leaving the lockfile untouched. Withdrawn — the lockfile write is the
>   only discriminator.
> - **`devbox add` does not produce a complete lockfile** (only `devbox
>   install` does), which is why two of this change's own tests were
>   passing for the wrong reason.

## 1. Provider implementation

> **Complete.** `src/provider/devbox.rs`. Two more corrections surfaced
> by live measurement while implementing, on top of the ones task group 0
> already made — recorded in design.md decisions 1a and 1b, and in the
> `env-provider` delta spec, rather than silently absorbed:
>
> - **1.3's premise was wrong.** `capture::store_grants` scans `PATH` for
>   the first entry containing `/nix/store` and returns only the root
>   prefix (`/nix/store` itself), never an enumerated path — so reusing
>   it unchanged, unmodified, already returns the correct coarse grant.
>   devbox's own stdenv wrapper puts literal `/nix/store/...` entries on
>   `PATH` regardless of declared packages (measured for both a
>   ripgrep-declaring project and an empty one), so the scrape always
>   finds a match, and the coarse root it returns covers the profile
>   symlink's target too, since a directory-prefix grant is not an
>   enumeration. No profile-symlink resolution needed; task simplified
>   to a straight reuse, verified with a non-stdenv marker package
>   (`resolve_against_a_real_project_grants_the_store_root`).
> - **1.2's per-system check was wrong, and would have rejected working
>   projects.** Measured with the exact chosen capture command: a
>   `devbox.lock` entry resolved only for a *different* system succeeds
>   here too, from its pinned commit reference, without touching the
>   lockfile. What actually contacts a package index and rewrites the
>   lockfile is a declared package with **no key at all** in
>   `devbox.lock` — confirmed directly, `nixpkgs-unstable` fetched from
>   `cache.nixos.org`, lockfile mutated on disk. The precondition checks
>   exactly that: every declared package's lock key present, regardless
>   of which systems its entry covers.
- [x] 1.1 `src/provider/devbox.rs` with `DevboxProvider`, implementing
      `Provider::resolve` via the mechanism task 0.2 chose (`shellenv
      --pure`, evaluated in a controlled shell, full binary path — the
      canonical baseline `PATH` has no reason to contain wherever this
      host installs devbox), diffed against the existing shared canonical
      baseline
- [x] 1.2 Preconditions, checked before any devbox command runs and each
      failing at layer `provider` with exit code 3: `devbox.json`
      present, `devbox` on PATH, Nix usable (reported by naming `nix`
      itself, not `devbox`, as devbox's own unmet requirement — never
      advice to switch providers), and every declared package has a key
      in `devbox.lock` — corrected from "resolved for the current
      system" by measurement; see the note above and design.md
      decision 1b
- [x] 1.3 Derive read-only store grants — reuses `capture::store_grants`
      **unchanged**, corrected from the profile-symlink-resolution plan
      by measurement; see the note above and design.md decision 1a
- [x] 1.4 `devbox_fingerprint`: content hash of `devbox.json` +
      `devbox.lock`, matching the flox and nix staleness contracts —
      with an **absent** lockfile marked distinct from a present-but-empty
      one (a leading marker byte, since `flox.rs`/`nix.rs`'s own
      `unwrap_or_default()` pattern would collide the two), so a lockfile
      appearing between two `up`s registers as a change
- [x] 1.5 `ServiceSupport::Unsupported` — deliberate, not a stub, with a
      comment naming why (proposal — Impact: the declarations come from
      plugin process-compose configs, which is the shape
      `add-flox-services` decision 1 rejected)
- [x] 1.6 Suppress the init hook: `shellenv --pure` never runs it (task
      0.3's finding). Asserted live by
      `devbox_shellenv_does_not_run_the_init_hook`, which writes a real
      `shell.init_hook` and confirms its sentinel file does not exist
      after `resolve()`

## 2. Dispatch and validation

- [x] 2.1 `ProviderKind::Devbox`: `from_name`, `static_name`, the
      `Provider` impl arm, and `manifest_fingerprint`
- [x] 2.2 `validate.rs`: moved `devbox` out of `NOT_YET_SUPPORTED` into
      `SUPPORTED`. No aliases (config spec: devbox has exactly one name)
- [x] 2.3 Verified, and **restated after adversarial review caught this
      record being stale**: it was written before task group 4 and
      claimed the provider files were the only ones that changed shape,
      which stopped being true the moment `doctor`/`init` landed.
      Accurate list: `src/provider/mod.rs` (dispatch arms),
      `src/provider/validate.rs` (one name moved lists), the new
      `src/provider/devbox.rs`, and `src/bin/devcroft.rs` (a `doctor`
      probe and `init` detection). The last of those is expected — the
      proposal's own Impact section lists it — and is *not* what the
      success criterion is about. What the criterion actually names
      stayed untouched: `Resolution`, `policy::compile`, and
      `lifecycle::up`'s provider handling. The trait generalizes, as
      claimed; the earlier wording just overstated it by omission

## 3. Tests

- [x] 3.1 Unit (`src/provider/devbox.rs`): fingerprint changes when
      either file changes and is stable otherwise, and distinguishes an
      absent lockfile from a present-but-empty one; preconditions fail
      with the right variant for each missing-thing case tested
      (`NoEnvironment`, `MissingLock`, and a package-not-locked
      `ResolutionFailed` naming the package). The fourth case —
      `devbox`/`nix` absent from `PATH` — follows `nix.rs`/`flox.rs`'s own
      precedent of not testing `MissingBinary` directly (no dedicated
      test exists for it there either); a comment there already notes
      what would report it
- [x] 3.2 `tests/devbox_provider_e2e.rs`: integration, self-skipping
      without devbox (and the nix it depends on) the way the existing
      real-tooling tests do — gated on devbox only, never on flox
- [x] 3.3 `tests/devbox_env_capture_is_deterministic.rs`: capture
      determinism — the env diff is byte-identical from a shell with
      extra `PATH` entries and extra variables set, in the shape
      `tests/nix_env_capture_is_deterministic.rs` already uses
- [x] 3.4 **Closure-tier measurement**
      (`a_real_build_succeeds_from_the_devbox_closure_with_the_host_toolchain_denied`):
      a real build inside the sandbox, host `/usr/bin/gcc` denied,
      devbox's own resolved `gcc` compiling and running a real program.
      Two corrections found while writing it, neither about devbox
      itself: `command -v /usr/bin/gcc` is the wrong assertion — a path
      can pass a bare existence check under Landlock while exec is still
      denied, since stat and exec are mediated separately, so the test
      must actually invoke the binary (matches
      `process_tier_landlock_boundaries.rs`'s own pattern); and `/tmp` is
      *not* part of what devcroft grants a closure-tier project by
      default (already documented — `samples/nix-go-sample`'s manifest
      carries the identical note for `go build`'s scratch directory), so
      the fixture manifest declares `[filesystem] allow = [".", "/tmp"]`
      the same way a real project would
- [x] 3.5 `policy_render_shows_the_devbox_store_grant_after_up`: shows the
      store grant with origin `provider:devbox`; provider resolution adds
      no write grant (`Resolution::read_only_grants` is the only field
      devbox populates beyond `env`/`unset`)
- [ ] 3.6 **Blocked — not devbox-specific, and not implemented anywhere
      yet.** Investigated rather than skipped: the mechanism this task
      assumes ("a manifest declaring services... fails distinguishably")
      does not exist in the codebase for *any* provider. `devcroft.toml`
      has no `[services]` section of its own — service declarations are
      entirely provider-side (`resolution.services`) — and
      `lifecycle::up::prepare_services` treats `ServiceSupport::Unsupported`
      identically to "supported, zero declared": both produce an empty
      slice and `up` proceeds silently. The `add-flox-services` delta
      spec this task cites (env-provider: "Services requested from a
      provider that cannot supply them fail loudly") is real but
      unimplemented — that change is at 30/45 tasks, not archived. So
      there is nothing distinguishable to test yet, for nix or devbox.
      Writing a devbox-only detection (e.g. sniffing for a
      `process-compose.yaml`) would invent cross-cutting behavior this
      change's own design explicitly deferred (Non-Goals: "devbox
      services... a separate change's decision to make, not this one's").
      Belongs to whatever change finishes `add-flox-services`'s
      loud-failure requirement, which should then cover every provider
      at once — including devbox, at that point trivially, since
      `ServiceSupport::Unsupported` already reports correctly and is
      covered by every `resolve()` test in `devbox.rs`

## 4. CLI surface

- [x] 4.1 `doctor`: `doctor_devbox_provider`, scoped to projects
      declaring `provider = "devbox"` via `doctor_provider`'s dispatch
      (which previously mapped everything except `"nix"` to the flox
      probe — devbox needed its own arm, not just a new function).
      Reports Nix as devbox's own precondition (two probes: devbox on
      PATH, then Nix usable) — a missing Nix names `nix` itself, never
      `devbox`, so the message reads as "you also need nix" rather than
      "switch providers"
- [x] 4.2 `doctor_on_a_devbox_project_reports_devbox_and_stays_silent_about_flox_and_nix`
      in `tests/init_and_doctor_cli.rs`, matching the two tests that
      already pin this for flox/nix
- [x] 4.3 `init`: detects `devbox.json`, precedence flox > devbox > flake.
      The cli spec's own text already corrects the reasoning this task
      description repeats (a root `flake.nix` is *not* usually generated
      from `devbox.json` — devbox writes its generated flake under
      `.devbox/gen/flake/`, never the project root); only the ordering
      itself is a real requirement, kept as a deterministic tiebreak
- [x] 4.4 `init`: prints `devbox install` when `devbox.json` declares
      packages with no `devbox.lock` present (advisory precision, not
      `up`'s exact precondition — matches nix's own `init` advice, which
      similarly only checks `flake.lock` presence). Says nothing about
      resolving when the project declares none. States in one line which
      other environments were found and remain available, both
      directions (flox/flake noted when devbox wins; devbox noted when
      flox wins)

## 5. Samples and docs

- [x] 5.1 `samples/devbox-citytime-sample/`: a `citytime` CLI, the same
      concept `flox-clap-sample`/`nix-flake-sample` use for direct
      comparison across providers — but std-only, no clap/chrono,
      because devbox's provider has no host-side hook devcroft will ever
      execute (unlike the other two, whose real crates.io deps are
      fetched via a hook this provider deliberately never runs — see
      design.md decision 2), so a dependency-bearing version would need
      vendoring or an explicit network allowlist, an unrelated question
      this sample sidesteps. Verified live end to end: `up`, `cargo
      build` (needs `[filesystem] allow = [".", "/tmp"]` — same
      `own-policy-baseline` requirement `nix-go-sample` already
      documents for Go), the built binary, host `/usr/bin/gcc` denied,
      `devcroft ssh`, `policy --render` showing `provider:devbox`, `down`
- [x] 5.2 README Status: a full paragraph on `add-devbox-provider`,
      placed after `use-nono-library`/services-reporting (chronological
      order the rest of Status follows), naming both corrections from
      task groups 1–3 (the per-system precondition and the store-grant
      simplification) as measured findings rather than absorbing them
      silently. Samples table gained a row
- [x] 5.3 `docs/decisions.md` §1: **already current, no change needed.**
      Checked rather than assumed: this section already speaks of devbox
      as a settled, qualified provider ("three providers with three
      different activation mechanisms (flox, nix flakes, devbox)", "what
      qualifying devbox actually cost") — task group 0's live
      measurement already informed this text before any provider code
      existed. Group 0 found no criterion failure, so there was nothing
      to move
- [x] 5.4 `openspec/config.yaml`: removed the "Provider roadmap, in
      order" list entirely — both entries on it (nix flakes, devbox) are
      now implemented, so there is no remaining roadmap to state at the
      closure tier
- [x] 5.5 CLAUDE.md: devbox moved out of the "not yet supported" list in
      the framing rules (alongside the existing "Nix flakes are
      implemented, not pending" correction, now naming devbox too); the
      two-phase invariant's provider table updated from "proposed" to
      "fixed, and implemented"; the Repository state section's sample
      list and implemented-changes list both gained devbox, the latter
      naming task 3.6's deliberate deferral rather than claiming a false
      "tasks.md and all"

## 6. Verification

- [x] 6.1 `cargo build`, `cargo clippy --all-targets`, `cargo fmt` all
      clean (the only clippy warning anywhere is `spike.rs`'s
      preexisting `zombie_processes` one, unrelated to this change).
      Full `cargo test` — every test file in the suite, not just the new
      ones — passes with devbox and nix on `PATH`: 227 lib tests, plus
      every integration file, zero failures
- [x] 6.2 `openspec validate --all`: 11 passed, 0 failed
- [x] 6.3 **Ran against a live devbox 0.18.0 for essentially everything —
      this change has no gap task group 0 didn't already close.** Every
      test in `src/provider/devbox.rs`, `tests/devbox_provider_e2e.rs`,
      and `tests/devbox_env_capture_is_deterministic.rs` that exercises
      `DevboxProvider::resolve` or the `devcroft` CLI against a real
      project self-skips without a working `devbox` + `nix` on `PATH`
      (verified: none silently no-op when tooling is present — all ran
      and passed against 0.18.0 in this devcontainer). The samples
      README's every command block (`up`, `cargo build`, the built
      binary, `ssh`, `policy --render`, `down`) was run for real, not
      copied from the other samples' pattern. What did *not* run against
      a live devbox: the pure-parsing unit tests
      (`declared_package_keys_*`, `package_key_*`, and the
      `ensure_everything_locked_*` tests that write a synthetic
      `devbox.lock` by hand rather than a real one) — deliberately, since
      their whole point is to pin the parsing contract independent of
      whether devbox happens to be installed wherever this suite runs
      next.
