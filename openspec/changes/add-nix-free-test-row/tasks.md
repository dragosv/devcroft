# Tasks — Nix-Free Test Row

## 0. What is already measured, and what it leaves open

> Recorded rather than repeated: these four results came out of
> `add-test-runtime-fixture`'s group 5 investigation and are what let this
> change start from a narrower question. Do not re-derive them; do not
> assume anything past them either.

- [x] 0.1 A row that declares its own directory as a grant brings `up` to
      `Started` on macOS, with the shell recorded outside the store. The
      mechanism works — this is the payoff of generalizing `shell::resolve`.
- [x] 0.2 Copied macOS platform binaries **hang**: a copied `/bin/sh` never
      returns, and neither does a copied `/bin/echo`, so it is not specific
      to shells. Signatures survive the copy intact (`codesign -dv` reports
      an identical `CodeDirectory`), so signing is not the cause. Copying a
      host shell is therefore not an option on macOS, on top of already
      being one the row contract forbids.
- [x] 0.3 Freshly compiled binaries run fine from a scratch directory
      (`clang`-built C, printed and exited 0), so building the row's shell
      from source is viable where copying is not.
- [x] 0.4 `process-compose` has no store requirement — one in an ordinary
      directory resolves, because `services::resolve_in_env` only walks the
      resolved environment's `PATH`. Services are a shipping question for
      this row, not a blocker.

## 1. Prove the row end-to-end, which 0.1 did not

> design.md Open Question 3: `up` reaching `Started` was measured with a
> shell that could not actually execute. Until a session runs, "the row
> works" is proven only up to `up`.

- [ ] 1.1 Build the row with a shell that works on this platform, and run a
      real session in it (`exec` a command, assert its output). This is the
      first thing that must pass; everything below assumes it.
- [ ] 1.2 Verify the negative that makes 1.1 meaningful: the row's shell is
      neither under `/nix/store` nor on the ambient host `PATH`. A row that
      passed 1.1 by quietly finding a store shell would prove nothing.
- [ ] 1.3 Add the liveness check the spec requires — execute the row's shell
      once at setup with a bounded wait, and report the row unavailable
      rather than blocking if it does not answer. The failure mode this
      guards against is a hang, so a check that itself hangs is no check.

## 2. macOS: build the shell from source

- [ ] 2.1 Pick the shell (dash is the candidate: small, POSIX, dependency-
      free) and record why in design.md — including how long a from-source
      build takes, since that cost lands on every fixture setup.
- [ ] 2.2 Detect the toolchain and report the row unavailable, by name, when
      it is missing. A developer without the Xcode command-line tools should
      be told that, not handed a build error.
- [ ] 2.3 Decide whether the built shell is cached across test runs. Building
      it per fixture is simple and possibly too slow; caching it is faster
      and introduces a staleness question. Measure before choosing.

## 3. Linux: static BusyBox

- [ ] 3.1 Resolve design.md Open Question 2 — fetch at setup versus vendor in
      the repo — and record the reasoning. Both need a per-architecture hash
      pin; vendoring additionally needs a decision about `Cargo.toml`'s
      anchored `include` allowlist so the binary never ships in the published
      crate.
- [ ] 3.2 Pin by hash per architecture, and fail the row loudly on a
      mismatch. An unpinned or silently-updated binary in the test path is a
      supply-chain hole in a project that generates and audits
      `THIRD-PARTY-LICENSES.md`.
- [ ] 3.3 Add the licence attribution BusyBox requires (GPL-2.0) wherever the
      binary is obtained from, and confirm it does not contaminate the
      published crate's own licensing — it is a test artifact, never linked.
- [ ] 3.4 Verify 1.1 and 1.2 on Linux, not by analogy with macOS. **Neither
      half of this change has been run on Linux**, and the two platforms
      share no artifact.

## 4. Services, if the row is to have them

- [ ] 4.1 Decide whether this row ships `process-compose`. It is not needed
      for the lifecycle/exec/SSH band, which is most of the neutral surface,
      and it costs a second pinned binary per platform.
- [ ] 4.2 If it does: `capabilities().services` reports it, so a neutral
      services test gates on the capability and not on the row's name.

## 5. Wire it in, and say what it is not

- [ ] 5.1 Register the row in `tests/common/mod.rs`'s `ROWS`, selectable as
      `DEVCROFT_TEST_PROVIDER=test` and never by default.
- [ ] 5.2 Assert the row does not become the default: a test that fails if
      the default selection resolves to it. The spec requirement exists
      because this is the change that creates the temptation.
- [x] 5.3 Verify it satisfies the shared realism check already in
      `tests/matrix_lifecycle.rs`
      (`every_row_resolves_its_shell_out_of_the_closure`) — which today
      asserts a `/nix/store` prefix and will need to become "inside the row's
      declared grants" to accommodate a row that is correct and store-free.
      **That relaxation must be made carefully**: it is the check that stops
      a row reaching for `/bin/sh`, and widening it wrongly re-opens exactly
      what it guards.
      → **Done ahead of the row, because it is the one part of this change
      that needs no new binary.** It now asserts the shell is inside one of
      the grants `up` actually recorded for that sandbox, which is *stronger*
      than the store prefix rather than weaker: it compares against what the
      provider declared, so a shell resolved from anywhere undeclared fails.
      Verified green on nix, flox and devbox, and teeth-checked by planting
      `/usr/bin/dash` — the exact regression it guards — and confirming the
      failure names both the shell and the grants.
- [ ] 5.4 Document in the row's own source what it is not evidence for — no
      dynamic loader on Linux, minimal environment on both — so a future
      reader does not promote a green board from this row into a claim about
      closures.

## 6. Unblock the migration this exists for

- [ ] 6.1 Confirm on a daemon-less host that `DEVCROFT_TEST_PROVIDER=test`
      runs the neutral surface green. That is the condition
      `add-test-runtime-fixture` group 4 was waiting on.
- [ ] 6.2 Hand back to that change's group 4, and record there that the
      blocker is cleared — including whether the devcontainer's ~80
      currently-skipping tests now run rather than skip, which is the number
      that motivated all of this.
