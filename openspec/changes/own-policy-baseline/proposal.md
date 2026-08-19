# Change: own-policy-baseline

Status: proposed. Blocks: `use-nono-library`. Touches an invariant, not
a feature.

> **Rewritten after measuring.** The first version of this proposal
> claimed the unrendered rules come from `extends: "default"` and that
> dropping `extends` removes them. That is false, and the corrected
> mechanism is in Why below. The wrong version is preserved in git
> history rather than quietly replaced, because the error is instructive:
> it was reasoned from a rule *count* without checking what the profile
> field actually controls.

## Why

Every profile devcroft compiles reaches the backend carrying **240 rules
across 18 policy groups** that `policy --render` cannot show. A typical
sandbox renders 8 rules and ships 248. That contradicts a stated
invariant:

> Policy is deterministic and inspectable. [...] Nothing goes to the
> backend that cannot be shown via `policy --render`.

**Where the rules actually come from.** Not from `extends: "default"`.
nono injects the full 18-group set into *every* profile, including one
that declares `extends: null` and no `groups` at all — confirmed with
`nono profile show <file> --json`, which resolves such a profile to the
identical 18 groups. `nono profile diff` between a profile with and
without `extends: "default"` reports exactly one difference:

```
signal_mode:
  + Isolated
```

So `extends: "default"` contributes a single setting. Removing it would
not remove one inherited rule — it would silently drop signal isolation,
a protection devcroft relies on and has never declared.

**The lever that does work is `groups.exclude`.** Excluding
`system_read_linux_core` makes `/usr/bin/env` resolve to `DENIED —
path_not_granted`, verified through `nono why --path --op`. The eight
`required` deny groups refuse exclusion (`Cannot exclude required
groups: 'deny_credentials'`) and remain enforced regardless — which is
the right outcome: those are the credential, keychain, browser-data and
shell-config denials, and devcroft should not be able to turn them off.

**Why excluding the system-read group is right on the merits, not just
tidy.** devcroft is closure-tier by design: `host` and `none` providers
are out of scope, so a project's toolchain comes from the provider's
store. A nix-provided `bash` resolves its interpreter and libc to
`/nix/store/…-glibc-…/lib/`, touching none of the 61 host paths that
group grants. Meanwhile that group grants read on `/usr/bin`, `/lib`,
and `/usr/share` — host toolchain access that project code can exec.
The six-criterion provider test in `docs/decisions.md` rejects a
provider that "leaves the C toolchain to the host [as having] smuggled
`host` passthrough back in under another name". The baseline is
currently doing exactly that, underneath every provider.

**Two smaller findings, independent of the above.** `doctor` pins
`>=0.71.0, <0.72.0` and so rejects the published 0.74.0. And
`profile.json` carries a `filesystem.read` grant for the directory
holding the devcroft binary that `policy --render` never prints — the
same invariant, violated by a route that has nothing to do with groups.

## What Changes

- **devcroft excludes the groups it does not want** via `groups.exclude`
  — the system-read groups it replaces with explicit closure-appropriate
  grants, and the command blocklist it does not use.
- **The system access devcroft actually needs is granted explicitly**,
  with `Origin::Baseline`, and is expected to be far smaller than the
  group it replaces. How much smaller is a question for measurement, not
  for this proposal to assert — the first version asserted "~61 entries"
  and was wrong to.
- **`signal_mode` is declared explicitly** rather than inherited from
  `extends: "default"`, so it appears in the compiled policy and cannot
  be lost by a change to the profile's inheritance.
- **The rules devcroft does not own are rendered anyway.** The eight
  required deny groups stay, and `policy --render` shows them with an
  origin identifying them as backend-enforced. nono attributes every
  path to its source (`group:deny_shell_configs`, `group:…`, `profile`)
  through `nono why`, so this is reporting available information rather
  than reimplementing it.
- **The `doctor` version range widens and states what it tests.**
- **The keeper-executable grant becomes renderable.**

## Capabilities

### Modified Capabilities

- `policy`: the compiled profile states which backend groups it
  excludes; devcroft's own system grants carry `Origin::Baseline`;
  `policy --render` accounts for every rule that reaches the backend,
  including those the backend enforces unconditionally.
- `cli`: `doctor`'s backend check states the compatibility surface it
  tests and accepts the range devcroft works against.

## Impact

- Affected specs: modified `policy`, `cli`.
- Affected code: `src/policy/mod.rs` (group exclusions, explicit
  grants, `signal_mode`, render completeness), `src/policy/why.rs`
  (attributing a denial to a backend-enforced group), `src/lifecycle/up.rs`
  (the keeper-exe grant), `src/bin/devcroft.rs` (`doctor_backend`).
- **Behavior does change**, unlike what the first version claimed:
  excluding the system-read groups removes host toolchain access from
  inside sandboxes. That is the point, and it is the main risk.

## Success Criteria

- `policy --render` accounts for every rule reaching the backend,
  including backend-enforced groups, verified by comparing the render
  against `nono profile show` on the emitted profile rather than by
  inspection.
- The sample projects build end to end with the system-read groups
  excluded — `flox-clap-sample` (Rust), `nix-go-sample` (Go),
  `gvisor-kotlin-sample` (Kotlin/Gradle, hardened tier).
- A sandbox cannot exec a host binary that the provider's closure does
  not supply, and `why` explains the denial.
- `~/.ssh` and cloud credentials stay denied, attributed to the
  backend-enforced group that denies them.
- `signal_mode` appears in the compiled policy and in `--render`.
- `devcroft doctor` passes against nono 0.74.0.

## Open Questions

- **Whether excluding the system-read groups is survivable at all.** The
  devcroft keeper is a host-linked binary; project code is closure-linked.
  Those have different needs and the exclusion affects both. This is the
  question that decides whether the change ships, and it is settled by
  task group 2, not by argument.
- **Whether `hooks` change the answer.** Hooks are project code but are
  written by people who may reasonably expect `sh` and `coreutils` from
  the host. A closure that supplies them is the correct answer; whether
  every real project's closure does is unknown.
- **What `--render` should call the backend-enforced rules.** They are
  not `baseline` in devcroft's existing sense — devcroft neither chose
  nor can remove them. A fourth origin may be more honest than
  overloading an existing one.
