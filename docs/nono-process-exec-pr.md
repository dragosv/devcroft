# PR text for nono: `ProcessExecMode`

Two pieces, in the order nono's contribution policy requires them
(`AGENTS.md`, "Coding Agent Contribution Policy": an issue must exist, the
intent and approach must be disclosed in its discussion, and only then a
PR). Branch: <https://github.com/dragosv/nono/tree/macos-process-exec-mode>,
commit `588bc19`, against `main` at `c11a976` (v0.77.0). Strip this
preamble when pasting.

---

## 1. Comment on #1865 — post first

> Same root fact as this issue, seen from the other side, and a proposed
> fix for both directions.
>
> On macOS `generate_profile` emits `(allow process-exec*)` unconditionally
> (since the initial commit) while `file-read*` is derived from the
> capability set; Seatbelt checks the two independently, which is how a
> binary ends up "exec permitted, own image unreadable" — the state this
> issue's TLS symptom comes from. Linux never enters that state:
> `AccessMode::Read` maps to `ReadFile | ReadDir | Execute` and nothing else
> carries `Execute`, so an ungranted path cannot be executed at all.
>
> **Intent.** Add an opt-in `ProcessExecMode` on `CapabilitySet`,
> `Unrestricted` (default, today's rule, byte-identical profile) or
> `ReadGranted` (`(allow process-exec* (subpath|literal "..."))` per
> Read/ReadWrite grant, both spellings, in place of the unconditional
> rule; no-op on Linux since that is already its semantics). Under
> `ReadGranted` the incoherent state cannot arise: an ungranted binary
> fails at `execve` with `EPERM` — option 2 of this issue's "expected".
> nono-cli's `system_read_macos` group already grants `/bin`, `/usr/bin`,
> `/opt`, `/Applications` and the rest, so a default-profile user opting in
> would exec exactly what they exec today; only paths outside every read
> grant change, which is the point.
>
> **Approach.** Three files in `crates/nono`: the enum beside
> `ProcessInfoMode` with builder/mutable setter/getter, a `match` where the
> rule is emitted, the re-export. Tests: profile text for both modes, a
> write-only grant getting no exec rule, a file grant becoming `literal`,
> and a forked live test — a copy of `/usr/bin/true` inside the one granted
> directory runs, `/bin/sh -c 'exit 7'` is refused at `execve` with
> `EPERM`, and the same grants under `Unrestricted` run `/bin/sh` (the
> control). 828 library tests pass; `clippy -D warnings -D
> clippy::unwrap_used` is clean.
>
> **Two things measured rather than argued**, because they shape the
> design. (1) This cannot be a macOS `Sandbox::restrict_execute` stacked
> after `apply()` like the Linux one: `sandbox_init()` from an already
> sandboxed process returns `EPERM`, in either order — Seatbelt does not
> stack, so the scope has to be in the one profile. (2) Scoped exec works
> in a `(deny default)` profile: a Mach-O under the allowed subpath runs,
> `/bin/echo` and `/usr/bin/true` fail at `execve`.
>
> **Limits / not proposed.** No change to the default — making `ReadGranted`
> the default would be the principled end state (it is what Linux does) but
> is a behaviour change for every macOS consumer and is yours to call, not
> this PR's. Paths reachable only via a `$PATH` metadata grant or a runtime
> extension token get no exec rule under `ReadGranted` (restrictive side).
> Not a fix for this issue's option 1 (auto-granting read on the exec
> target) — that needs the target, which the library does not know and
> the CLI does.
>
> Disclosure: I am filing this as the maintainer of a consumer (devcroft,
> which links `nono` for its process tier and excludes the `system_read_*`
> groups); the implementation was written by an AI agent (Claude) under my
> direction and reviewed by me. Happy to reshape the API — a `bool`
> setter, a different name — if you would rather; the semantics are the
> part I care about. Implementation, ready to open as a PR if you're open
> to it: https://github.com/dragosv/nono/tree/macos-process-exec-mode

---

## 2. PR — title and body

**Title** (conventional commit, `pr-lint.yml`):
`feat(sandbox): opt-in ProcessExecMode scoping process-exec to read grants on macOS`

**Body:**

## Linked Issue

Refs #1865 — implements the `execve`-time refusal half (option 2 of its "expected"), opt-in. Not "Closes": option 1, auto-granting read on the exec target, is CLI-side and not attempted here, and whether `ReadGranted` should become the default is left to maintainers. Intent and approach were disclosed in that issue's discussion before this PR.

## Summary

On macOS `generate_profile` emits `(allow process-exec*)` unconditionally while `file-read*` is derived from the capability set. Seatbelt checks the two independently, so a binary at a path nothing granted executes with its own image unreadable — #1865's symptom. Linux never has this state: the Landlock backend maps `AccessMode::Read` to `ReadFile | ReadDir | Execute` and nothing else carries `Execute`.

This adds an opt-in `ProcessExecMode` on `CapabilitySet`:

- `Unrestricted` (default) — today's rule; the generated profile is byte-identical.
- `ReadGranted` — `(allow process-exec* (subpath|literal "..."))` per Read/ReadWrite grant, both original and resolved spellings (same as `file-read*`), in place of the unconditional rule. No-op on Linux, where this is already the semantics.

Under `ReadGranted` an ungranted binary fails at `execve` with `EPERM` instead of starting and failing downstream in dyld / Security.framework.

Design constraint, measured: this cannot be a macOS `Sandbox::restrict_execute` stacked after `apply()` — `sandbox_init()` from an already sandboxed process returns `EPERM` in either order. Seatbelt does not stack profiles, so the scope has to be part of the single profile applied.

No change to the default. nono-cli's `system_read_macos` group grants `/bin`, `/usr/bin`, `/opt`, `/Applications` etc., so a default-profile user opting in execs exactly what they exec today. Paths reachable only through a `$PATH` metadata grant or a runtime extension token get no exec rule under `ReadGranted` (restrictive side, documented on the enum).

## Agent Disclosure

- The code in this PR was written by an AI agent (Claude, via Claude Code) under the direction of, and reviewed by, the PR author, who is the maintainer of a consumer of this library.
- Consulted: `AGENTS.md` (Coding Standards, Security Considerations, Coding Agent Contribution Policy), `neps/README.md` ("a small feature addition does not need a NEP" — this adds one opt-in enum and changes no default, no boundary, no existing API), `.github/PULL_REQUEST_TEMPLATE.md`, `crates/nono/src/sandbox/macos.rs` (`generate_profile`, `path_filters_for_cap`, the platform-rules ordering comment citing #970), `crates/nono/src/sandbox/linux.rs` (`AccessMode::Read` → `Execute` mapping; `restrict_execute`), `crates/nono/src/capability.rs` (`ProcessInfoMode` as the pattern), `crates/nono-cli/data/policy.json` (`system_read_macos`).
- Attribution. Existing project code reused unchanged: `path_filters_for_cap` (emits the per-grant filters; this PR calls it for `process-exec*` exactly as the `file-read*` loop does). Adapted: the fork + exit-code live-test pattern from `linux.rs::test_restrict_execute_does_not_break_rename_into_new_subdir`, reshaped into `exec_under` for macOS. Newly written: `ProcessExecMode`, its setters/getter and re-export, the `match` in `generate_profile`, the profile-text tests, `dyld_read_caps`, and the live test.
- This contribution complies with the repository's coding standards (no `unwrap`/`expect` outside `#[cfg(test)]`, `NonoError` propagation via `?`, `// SAFETY:` on the one new `unsafe` block, paths canonicalized by the existing `FsCapability` constructors) and security requirements (fail secure: the new mode only removes an allow; the default is unchanged).

## Test Plan

- `cargo test -p nono --lib` — 828 passed (4 new profile/capability tests plus the live one).
- `cargo clippy -p nono --all-targets -- -D warnings -D clippy::unwrap_used` — clean.
- `cargo doc -p nono --no-deps` — no new warnings (one doc link to the Linux-only `restrict_execute` deliberately left as plain text so macOS doc builds stay clean).
- `cargo build -p nono-cli` — builds; the CLI is untouched and its behaviour unchanged (default mode).
- Live, macOS 15.7.4 / aarch64: `test_apply_process_exec_read_granted_refuses_ungranted_binary` forks three children — granted copy of `/usr/bin/true` runs (exit 0); `/bin/sh -c 'exit 7'` refused at `execve` with `EPERM` (exit 5); control under `Unrestricted` runs `/bin/sh` (exit 7).
- Consumer measurement: devcroft's end-to-end "host gcc is refused inside a devbox closure sandbox" test now passes on macOS with exec scoped to the read set, and fails without it.

## Checklist

- [x] An issue exists and is linked above
- [x] All commits are signed-off, using DCO
- [x] All new code follows the project's coding standards (CLAUDE.md) and is covered by tests
- [x] Public-facing changes are paired with documentation updates — rustdoc on `ProcessExecMode` and both setters; no `docs/` page documents the library's `*Mode` enums beyond the CLI manifest, which this does not touch
- [x] Not a major feature or security-boundary change (opt-in, default unchanged, no library/CLI boundary moved) — no NEP; happy to write one if you disagree

## Agent Compliance Check

- [x] I am not prohibited from contributing under this policy
- [x] An issue already exists (#1865)
- [x] I described my intent and approach in the issue discussion (comment on #1865, before this PR)
- [x] I reviewed repository coding and security rules for the affected area
- [x] I provided required attribution for reused or adapted code (above)
- [x] I did not use forbidden patterns such as unwrap/expect (test module carries the crate's existing `#[allow(clippy::unwrap_used)]`)
- [x] I used NonoError where required (no new error paths; `generate_profile`'s existing `Result` is propagated)
- [x] I validated and canonicalized all relevant paths (grants enter through the existing `FsCapability` constructors; no new path input)
- [x] This PR matches the approved or disclosed issue scope
