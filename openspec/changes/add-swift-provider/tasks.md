# Tasks — Swift Provider

Ordered so the two measurements that can invalidate the design are taken
before any code depends on them, matching `add-mount-isolation`'s task
group 0.

## 0. Measure before building

> Both open questions in design.md are of the form "the lever probably
> exists". Both were assumed once already in this project's history —
> `nix`'s `TMPDIR` and flox's `--mode run` — and both assumptions were
> wrong. Measure, then write.

- [x] 0.1 Determine whether SwiftPM honours **environment variables** for
      cache, scratch and configuration paths, or only the `--cache-path`
      and `--scratch-path` flags (design.md D3, open question 1). devcroft
      injects an environment; it does not wrap commands. If only flags
      exist, decide between a `swift` shim on the sandbox `PATH` and a
      devcroft-owned granted directory, and record which.
- [ ] 0.2 Record what a real `swift build` opens outside the project root,
      per installation layout: Command Line Tools only, and `Xcode.app`.
      The question is specifically whether anything beyond the developer
      directory and the dyld shared cache is needed.
- [ ] 0.3 Establish the dyld shared cache path on supported macOS versions
      and confirm a Swift-built binary runs with that granted and
      `/usr/lib` not. This is D2's load-bearing claim; if the binary still
      fails, the grant set is wrong and the design changes.
- [ ] 0.4 Confirm the two installation layouts produce the same
      `Resolution` shape. `xcode-select -p` returns
      `/Library/Developer/CommandLineTools` on a CLT-only host and
      `/Applications/Xcode.app/Contents/Developer` on an Xcode host; the
      SDK sits at a different relative path in each.
- [x] 0.5 Determine how an unaccepted licence actually presents — exit
      code and stderr of `xcrun` and `swift --version` — so 3.3 can
      distinguish it from a missing toolchain rather than guessing.

## 1. Toolchain resolution

- [ ] 1.1 `src/provider/swift.rs` implementing `Provider::resolve`:
      developer directory, SDK path, toolchain `usr/bin` on `PATH`,
      captured as a diff against the same fixed pre-activation baseline
      the other three providers use.
- [ ] 1.2 **Assert the negative in the code, not only in the spec**: the
      resolver must never open a project file. Structure it so the project
      root is used for nothing but locating the sandbox, and say so in a
      doc comment naming design.md D1.
- [ ] 1.3 Apply the cache/scratch redirection chosen in 0.1, so no SwiftPM
      state is written under the invoking user's home directory.
- [ ] 1.4 Populate `read_only_grants` with the developer directory and the
      dyld shared cache, and with nothing else. No individual dylib paths
      (D2).
- [ ] 1.5 Leave `Resolution`'s activation-script field `None` and comment
      why: the provider has no project script *because it never asks for
      one*, which is a different reason from nix's and devbox's.

## 2. Registration and platform gating

- [ ] 2.1 Accept `swift` in `provider::validate`'s `SUPPORTED`, with no
      alias. Add tests mirroring the devbox ones, plus one asserting
      `swiftpm` and `xcode` are rejected (D6).
- [ ] 2.2 A new `ProviderError` variant for platform mismatch, distinct in
      *type* from `Unknown`, `NotYetSupported`, `OutOfScope` and
      `VersionManager` (D5). Layer `provider`, exit code 3.
- [ ] 2.3 Wire dispatch in `src/provider/mod.rs`, keyed off the provider
      name in the one place the other three are.
- [ ] 2.4 Gate on macOS at validation time, so a Linux user gets the
      platform message from `up`, `status` and `doctor` alike rather than
      only from resolution.

## 3. Preconditions and `doctor`

- [ ] 3.1 A precondition probe that **executes** the toolchain rather than
      testing for a path, in the shape of
      `provider::host_can_build_nix_closures`.
- [ ] 3.2 A `doctor` arm reporting the selected developer directory, which
      installation backs it, and the SDK build version.
- [ ] 3.3 Distinguish the three failure modes measured in 0.5 — no
      toolchain, stale selection, unaccepted licence — each naming the
      command that fixes it.
- [ ] 3.4 `doctor` on Linux reports the provider as unavailable on this
      platform, not as broken.

## 4. Tier surfacing

- [ ] 4.1 Record the guarantee tier in the sandbox metadata the compiled
      policy is built from, so `status` and the `up` notice read one fact
      (policy delta, "Guarantee tier is carried in the compiled policy").
- [ ] 4.2 One notice at `up` naming the tier **and what does not hold** —
      no shared store, host-linked runtime — in the shape of the existing
      degraded-capability warning (D4).
- [ ] 4.3 `status` shows the tier and the SDK build version, and does not
      repeat the notice.
- [ ] 4.4 A test that the notice fires exactly once for `swift` and not at
      all for `flox`, `nix` or `devbox`.

## 5. Policy

- [ ] 5.1 Compile the provider grants with a `provider:swift` origin and
      confirm `policy --render` shows them.
- [ ] 5.2 Guard against grants naming absent paths, surfacing rather than
      silently emitting them. General, not Swift-specific (D2).
- [ ] 5.3 A test comparing a rendered flox policy against a rendered Swift
      policy, asserting the host paths appear in one and not the other and
      are attributable to the provider origin. This is the artifact tier
      becoming a measurable difference rather than a documented one.

## 6. Sample and tests

- [ ] 6.1 `samples/swift-clt-sample` — a SwiftPM project with an explicit
      note that it depends on nothing beyond the standard library, for the
      same reason `devbox-citytime-sample` does: there is no host-side
      phase in which to fetch anything.
- [ ] 6.2 Move the manifest-execution probe into the sample so the
      `MANIFEST-SIDE-EFFECT-RAN` and `PROBE …` results in proposal.md and
      design.md are **measured on the reader's host**, not asserted
      (`nix-probe-sample`'s precedent).
- [ ] 6.3 An e2e test that `up` in a project whose `Package.swift` has a
      side effect resolves without the side effect occurring. This is D1's
      regression test and the most important test in the change.
- [ ] 6.4 An e2e test that a binary built inside the sandbox runs inside
      the sandbox, with no grant beyond the project root, the
      `provider:swift` paths and the baseline.
- [ ] 6.5 An e2e test that a full build writes nothing under the invoking
      user's home directory.
- [ ] 6.6 Every test in this group self-skips on a host without a usable
      toolchain, guarding on the **capability** probe from 3.1 and never
      on the binary's presence. Verify with
      `cargo test -- --nocapture 2>&1 | grep skipping`.
- [ ] 6.7 A test that `devcroft --help` is unchanged, satisfied by the
      existing `tests/cli_help_and_version.rs` continuing to pass
      unmodified.

## 7. Documentation

- [ ] 7.1 `docs/decisions.md` §1: a Swift entry recording the verdict per
      criterion, the criterion-3 failure, and the criterion-3-versus-4
      tension. Written so a maintainer can reject the provider on it.
- [ ] 7.2 Add the SwiftPM row to §1's criterion-4 table — the one with an
      empty "does not run the hook" column, and a note that unlike flox
      there is nothing to strip.
- [ ] 7.3 `docs/known-gaps.md`: no shared store, so N Swift sandboxes cost
      N builds; and the SDK is whatever the host installed.
- [ ] 7.4 `docs/implementation-log.md`: the two measurements that were
      surprising — SwiftPM's manifest sandbox confines writes and network
      but not reads or exec, and the linked system dylibs do not exist as
      files.
- [ ] 7.5 README status/limitations: Swift listed as artifact tier,
      macOS-only, with the shared-store caveat. Do not restate the
      reasoning there; link `docs/decisions.md`.
- [ ] 7.6 `docs/threat-model.md`: state that code signing is out of scope
      and why, so a user does not discover it by having a build fail.

## 8. Close-out

- [ ] 8.1 `cargo clippy` and `cargo doc --no-deps` warning-free.
- [ ] 8.2 `openspec validate add-swift-provider --type change` passes.
- [ ] 8.3 If `Cargo.lock` changed, regenerate `THIRD-PARTY-LICENSES.md`.
      It should not have — this change adds no dependency, and a diff here
      means something was pulled in that the design did not intend.
- [ ] 8.4 Confirm `Cargo.toml`'s `include` allowlist still covers what the
      published crate needs, and that `samples/swift-clt-sample` is not
      swept in.
