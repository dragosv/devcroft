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

## 1. Provider implementation

- [ ] 1.1 `src/provider/devbox.rs` with `DevboxProvider`, implementing
      `Provider::resolve` via the mechanism task 0.2 chose, diffed
      against the existing shared canonical baseline — never a new one
- [ ] 1.2 Preconditions, checked before any devbox command runs and each
      failing at layer `provider` with exit code 3: `devbox.json`
      present, `devbox` on PATH, Nix usable, and **every declared package
      resolved for the current system**. That last one is not
      "`devbox.lock` exists" — measured against 0.18.0, a project
      declaring no packages has no lockfile at all and is legitimate,
      and resolutions are recorded per-system with no guarantee the
      current system is among them, so a presence check both rejects
      valid projects and accepts ones that will still resolve at `up`.
      Nix's absence is reported as devbox's own unmet requirement
      (design.md decision 4), never as advice to switch providers
- [ ] 1.3 Derive read-only store grants from the resolved closure —
      **not** by scraping `PATH` for `/nix/store/` prefixes, which is
      what the other two closure providers can do and which would be
      wrong here. Measured (0.5): a devbox project's declared packages
      reach `PATH` through `<project>/.devbox/nix/profile/default`, a
      symlink chain into a `/nix/store/…-profile`, while the bare store
      paths on `PATH` are only the stdenv. Scraping would grant the
      toolchain and miss every package the project declared. Resolve
      through the profile link, or read `HOST_PATH`, and test against a
      project whose declared package is *not* part of stdenv so the
      difference is observable
- [ ] 1.4 `devbox_fingerprint`: content hash of `devbox.json` +
      `devbox.lock`, matching the flox and nix staleness contracts —
      with an **absent** lockfile hashed as a distinct state rather than
      as empty, so a lockfile appearing between two `up`s registers as a
      change instead of being invisible
- [ ] 1.5 `ServiceSupport::Unsupported` — deliberate, not a stub. Leave a
      comment naming why (proposal — Impact: the declarations come from
      plugin process-compose configs, which is the shape
      `add-flox-services` decision 1 rejected) so the next reader does
      not "finish" it by wiring up something unexamined
- [ ] 1.6 Suppress the init hook if task 0.3 found a way to; assert in a
      test that it does not run during resolution

## 2. Dispatch and validation

- [ ] 2.1 `ProviderKind::Devbox`: `from_name`, `static_name`, the
      `Provider` impl arm, and `manifest_fingerprint`
- [ ] 2.2 `validate.rs`: move `devbox` out of `NOT_YET_SUPPORTED` into
      `SUPPORTED`. No aliases (config spec: devbox has exactly one name)
- [ ] 2.3 Verify nothing outside `src/provider/` needed to change. **If
      `Resolution`, `policy::compile`, or `lifecycle::up`'s provider
      handling had to change shape, record it** — that is the "the trait
      generalizes" claim failing, and the proposal's success criteria
      make it reportable rather than absorbable

## 3. Tests

- [ ] 3.1 Unit: fingerprint changes when either file changes and is
      stable otherwise; preconditions fail at layer `provider` with the
      right hint for each of the four missing-thing cases
- [ ] 3.2 Integration, self-skipping without devbox the way the existing
      real-tooling tests do — and gated on **devbox only**, never on
      flox, per the rule that a test gates on what its own assertions
      need
- [ ] 3.3 Capture determinism: the env diff is byte-identical from a
      shell with extra `PATH` entries and extra variables set, in the
      shape `tests/flox_env_capture_is_deterministic.rs` already uses
- [ ] 3.4 **Closure-tier measurement**: a real build inside the sandbox
      with `network.default = "deny"`, asserting the host toolchain
      (`/usr/bin/gcc`) is denied and the build still succeeds. This is
      the test that earns the tier claim; without it, "closure tier" is
      an inference from devbox being Nix-backed
- [ ] 3.5 `policy --render` shows the store grants with origin
      `provider:devbox`, and provider resolution adds no write grant
- [ ] 3.6 A manifest declaring services under `provider = "devbox"` fails
      distinguishably from "supports services, none declared" — the
      `services` spec already requires the two be separable, and this is
      the second provider to exercise it

## 4. CLI surface

- [ ] 4.1 `doctor`: a devbox check, scoped to projects declaring it, and
      reporting Nix as devbox's own precondition. Two probes, not one
- [ ] 4.2 `doctor`: a test that a devbox project reports devbox and stays
      silent about flox and nix, matching the two tests that already pin
      this for the other providers
- [ ] 4.3 `init`: detect `devbox.json`, with precedence flox > devbox >
      flake (cli spec, with the reasoning: a root `flake.nix` in a devbox
      project is often generated *from* `devbox.json`, so ranking the
      flake higher points devcroft at the derived artifact)
- [ ] 4.4 `init`: print `devbox install` when the project declares
      packages that are not yet resolved — **not** a lock subcommand,
      which devbox does not have (`devbox --help` lists `install` and
      `update`; there is no `lock`). Say nothing about resolving when
      the project declares no packages. State in one line which other
      environments were found and remain available

## 5. Samples and docs

- [ ] 5.1 A `samples/devbox-*-sample` project with its own README, in the
      shape the existing samples use — including the `[workspace]` table
      if it is a Rust project, so it is not pulled into this crate's
      workspace
- [ ] 5.2 README Status: devbox implemented, closure tier, with whatever
      task group 0 measured stated plainly — including anything that
      turned out worse than expected
- [ ] 5.3 `docs/decisions.md` §1: devbox moves from the closure-tier
      listing into implemented providers. If group 0 found a criterion
      failure, that is what gets written instead, with the property that
      failed named
- [ ] 5.4 `openspec/config.yaml`: remove devbox from the provider
      roadmap, since the roadmap tracks what is not yet built
- [ ] 5.5 CLAUDE.md: devbox moves out of the "not yet supported" list in
      the framing rules, alongside the existing "Nix flakes are
      implemented, not pending" correction

## 6. Verification

- [ ] 6.1 `cargo build`, `cargo clippy`, `cargo fmt` clean
- [ ] 6.2 `openspec validate --all` passes
- [ ] 6.3 Report which tasks ran against a live devbox and which did not.
      Group 0 exists because this change started with none of that
      evidence; finishing it with the same gap unstated would be worse
      than starting with it
