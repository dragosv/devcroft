# Change: own-policy-baseline

Status: proposed. Blocks: `use-nono-library` (which is only coherent
once devcroft owns its baseline). Touches an invariant, not a feature.

## Why

Every profile devcroft compiles is emitted with `extends: "default"`
(`src/policy/mod.rs`), so the rules that actually reach the backend are
devcroft's own plus whatever nono's built-in `default` profile contains.
Measured against a live nono 0.71.0: **240 rules across 18 policy
groups**, none of which devcroft can enumerate, reason about, or show.

That contradicts a stated invariant:

> Policy is deterministic and inspectable. [...] Nothing goes to the
> backend that cannot be shown via `policy --render`.

Today a typical sandbox renders **8 rules** and ships **248**. The
ratio is not a rounding error; it is the majority of the policy.

Three consequences, all observed rather than predicted:

**The inherited rules do not match devcroft's model.** nono's `default`
is deny-list-first, designed for an agent running inside a real `$HOME`
with broad grants — `deny_credentials`, `deny_keychains_*`,
`deny_browser_data_*`, `deny_shell_history`, `deny_shell_configs`, 69
rules across eight groups marked `required`. devcroft is allowlist-first
and project-scoped. Verified by emptying devcroft's `deny` list
entirely: `~/.ssh` and `~/.bashrc` remained `Permission denied`, because
nothing outside the granted paths is reachable in the first place. Those
69 rules are defense-in-depth for a threat model devcroft does not have.

**49 of the inherited rules are inert.** `dangerous_commands` and its
platform variants deny `rm`, `mv`, `cp`, `chmod`, `npm`, `pip`, `rsync`,
`xargs`, `kill`, `sudo` and 16 more. `deny.commands` is enforced by
nono's resident supervisor in `run`/`shell` mode; devcroft uses `wrap`,
where nono applies the restriction and execs away. Verified live under
`extends: "default"`: `rm` and `cp` both succeeded. So devcroft carries a
blocklist that does nothing — while anyone reading the emitted
`profile.json` would reasonably conclude those commands are blocked.

**The version pin is the symptom.** `doctor` requires
`>=0.71.0, <0.72.0` (`src/bin/devcroft.rs`) and fails outside it. nono
is at 0.74.0, so `devcroft doctor` currently rejects the current
release. The window is that narrow precisely because the contents of
`default` are an undocumented dependency: a minor release may change
rules devcroft ships without devcroft knowing.

## What Changes

- **devcroft emits a self-contained profile.** No `extends`. The
  baseline it needs is enumerated in devcroft's own source, with each
  entry carrying `Origin::Baseline` as the origin model already
  provides.
- **`policy --render` shows the whole compiled policy**, because there
  is no longer an unrendered remainder. The invariant becomes true
  rather than aspirational.
- **The system-access set is stated, not inherited.** Proven sufficient
  live: a profile with no `extends`, carrying the 61 paths of
  `system_read_linux_core` inline, exec'd normally, read the project,
  and denied `~/.ssh`. The existing source comment claiming a profile
  without `extends` "can't exec anything at all" is true only of an
  *empty* profile and is corrected here.
- **The inherited command blocklist is dropped rather than
  reimplemented.** It is inert in `wrap` mode, and adopting it would
  mean devcroft asserting a policy stance — "a dev sandbox may not run
  `npm`" — that it has never chosen and that its own audience
  contradicts.
- **The `doctor` version range widens and states what it is checking.**
  Once devcroft does not depend on the contents of `default`, the
  compatible surface is the profile schema and the `wrap` invocation,
  not a specific ruleset — so the range reflects tested versions rather
  than a single point release.
- **The keeper-executable grant becomes renderable.** Found while
  measuring: `profile.json` carries a `filesystem.read` entry for the
  directory holding the devcroft binary that `policy --render` does not
  print — the same invariant, violated independently of `extends`.

## Capabilities

### Modified Capabilities

- `policy`: the compiled profile is self-contained; the baseline is
  devcroft's own enumerated rule set with `Origin::Baseline`;
  `policy --render` is complete by construction.
- `cli`: `doctor`'s backend check states the compatibility surface it
  tests and accepts the range devcroft actually works against.

## Impact

- Affected specs: modified `policy`, `cli`.
- Affected code: `src/policy/mod.rs` (baseline rules, `to_nono_profile`,
  `--render` completeness), `src/policy/why.rs` (`why` must be able to
  attribute a denial to a baseline rule now that they are devcroft's),
  `src/lifecycle/up.rs` (the keeper-exe grant, moved into the compiled
  policy rather than appended after it), `src/bin/devcroft.rs`
  (`doctor_backend`).
- Platform-split: the baseline is per-OS. Linux needs ~61 path entries,
  macOS ~35, drawn from the same two groups nono splits them into.
- No behavior change intended for a working project: the point is that
  the same sandbox comes up with the same effective access, described
  fully instead of partially.

## Success Criteria

- A compiled profile contains no `extends` key, and `policy --render`
  output enumerates every rule present in `profile.json` — asserted by a
  test that diffs the two rather than by inspection.
- An existing sample project (`samples/flox-clap-sample`) comes up,
  execs, builds, and tears down identically before and after the change.
- `~/.ssh`, `~/.aws`, and the devcroft data dir remain denied, with the
  deny list carrying only devcroft's own entries.
- `devcroft doctor` passes against nono 0.74.0.
- `why` can attribute a denial caused by a baseline path to
  `baseline`, naming the rule — impossible today for inherited rules.

## Open Questions

- **Whether to keep the redundant deny rules.** They are provably
  unnecessary under the allowlist model, but the invariant says
  "baseline denials always win", and an explicit `deny` on `~/.ssh`
  survives a future mistake that widens an allow. Cheap insurance
  against a class of error, or dead weight of exactly the kind this
  change removes elsewhere — not settled.
- **How the per-distro path set is kept honest.** `/lib/x86_64-linux-gnu`
  vs `/lib/aarch64-linux-gnu` is already handled by listing both, but a
  musl or NixOS host has a different linker layout. Whether that is a
  `doctor` check, a documented limitation, or a provider-supplied grant
  is open.
- **Whether the baseline should be data rather than code.** A checked-in
  JSON the tests validate is more inspectable than a Rust constant, but
  introduces a file that can drift from the code that consumes it.
