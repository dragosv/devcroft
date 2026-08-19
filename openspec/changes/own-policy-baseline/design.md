# Design: own-policy-baseline

## Context

See `proposal.md` — Why, including the note on why this file was
rewritten.

Everything below was measured against a live nono 0.71.0 in this repo's
devcontainer. The commands are recorded so the findings can be
re-derived rather than trusted, which matters here because the first
version of this design drew a wrong conclusion from a right measurement:

```sh
nono profile show <file> --json      # what a profile RESOLVES to, incl. injected groups
nono profile diff <a> <b>            # what one profile field actually changes
nono why -p <file> --path P --op read # per-path verdict WITH its source
nono profile groups <name> --json    # the rules in one group
nono profile validate <file>
```

`nono profile show` on the *file* is the instrument that settles what a
profile actually means. `nono profile groups` alone counts rules without
telling you whether your profile controls them — which is how the first
version concluded that 240 rules were attributable to `extends`.

## The measurement that reframes everything

| profile | resolved groups |
|---|---|
| `extends: "default"` | 18 |
| no `extends`, no `groups` key | **18** |

`nono profile diff` between them:

```
signal_mode:
  + Isolated
```

nono injects its full group set into every profile. `extends: "default"`
buys one setting. Any design premised on "stop extending `default` and
the inherited rules go away" is premised on something false.

## Goals / Non-Goals

**Goals:**

- Every rule reaching the backend is accounted for in `policy --render`,
  whether devcroft chose it or the backend enforces it unconditionally.
- devcroft stops granting host toolchain access it does not want and
  never asked for.
- Nothing devcroft depends on is inherited implicitly — `signal_mode`
  included.

**Non-Goals:**

- Removing the backend's mandatory deny groups. They cannot be excluded,
  and they are the ones worth keeping.
- Reimplementing nono's group catalog. devcroft grants what its own
  sandboxes need; the general catalog stays nono's.
- Making `policy --render` work without a backend installed. The first
  version treated that as a benefit of ownership; since ownership is
  partial, rendering the backend-enforced half needs the backend.

## Decision 1: exclude groups, do not stop extending

**What:** the compiled profile keeps `extends`, and adds
`groups.exclude` naming the groups devcroft replaces or does not want.

**Why:** `extends` was never the lever. Excluding is, and it works:
with `system_read_linux_core` excluded, `nono why --path /usr/bin/env
--op read` returns `DENIED — path_not_granted`, where the same query
against the unmodified profile returns `ALLOWED — Source:
group:system_read_linux_core`.

**Boundary:** the eight `required` groups refuse exclusion —
`Cannot exclude required groups: 'deny_credentials'` — and the profile
fails validation rather than silently ignoring the request. Good
behavior on nono's part, and it settles what devcroft can own: the
grants, not the mandatory denials.

## Decision 2: exclude the system-read groups because devcroft is closure-tier

**What:** `system_read_linux_core` and `system_read_macos` are excluded,
and replaced by explicit grants for what devcroft's own processes need.

**Why:** this is the decision with an argument behind it rather than a
measurement. devcroft has no `host` provider by design; a project's
toolchain comes from its closure. A nix-supplied `bash` resolves:

```
ld-linux-aarch64.so.1 => /nix/store/…-glibc-2.42-51/lib/
libc.so.6             => /nix/store/…-glibc-2.42-51/lib/
```

Nothing from `/lib`, `/lib64`, or `/usr/lib`. Meanwhile the group grants
read on `/usr/bin`, `/lib`, `/usr/share` and 58 more — host binaries
project code can exec. `docs/decisions.md` rejects providers that leave
the C toolchain to the host as smuggling `host` passthrough back in;
the baseline has been doing that underneath every provider, which is a
gap in the thesis rather than in any one provider.

**What makes this risky:** the keeper is a host-linked Rust binary, and
its needs are not the project's. Hooks are project code that may
reasonably expect host `sh`. Neither is settled by the argument above —
both are settled by task group 2 actually running the samples. If the
exclusion cannot survive them, Decision 2 is the part that gets dropped,
and the change still has Decisions 3–5 worth landing.

## Decision 3: drop the command blocklist, keep the mandatory denials

**What:** `dangerous_commands`, `dangerous_commands_linux`,
`dangerous_commands_macos` are excluded (49 rules). The eight required
deny groups stay, because they cannot go.

**Why:** verified inert in the mode devcroft uses. Under
`extends: "default"`, `rm victim.txt` and `cp f.txt f2.txt` both
succeeded inside `nono wrap` — `deny.commands` needs nono's resident
supervisor, which `run`/`shell` provide and `wrap` does not. Carrying a
blocklist that does nothing is worse than not carrying it: anyone
reading the emitted profile would conclude `npm` and `rm` are blocked.

Reimplementing it is separately wrong: the list denies `rm`, `mv`, `cp`,
`npm`, `pip`, `rsync`, `xargs` — every one of which a build runs.

## Decision 4: declare `signal_mode` explicitly

**What:** the compiled profile sets `signal_mode` rather than receiving
it from `extends: "default"`.

**Why:** it is the *only* thing `extends: "default"` contributes, which
means it is currently the single most easily lost property in the whole
policy — and it was invisible until `nono profile diff` surfaced it. A
protection that depends on a profile field nobody knew was load-bearing
is exactly what "deterministic and inspectable" is supposed to prevent.

## Decision 5: render what devcroft does not own

**What:** `policy --render` reports the backend-enforced groups
alongside devcroft's own rules, marked as backend-enforced rather than
as devcroft's baseline.

**Why:** ownership turned out to be partial, so completeness cannot come
from owning everything. It can come from reporting: nono attributes
every path to its source through `nono why`, distinguishing
`group:deny_shell_configs` from `profile`, and `nono profile show`
resolves a profile to its full group set. Rendering from that is
reporting available information, not reimplementing policy.

**Cost, stated plainly:** `--render` gains a dependency on the backend
binary for the part of the policy the backend owns. The first version of
this design rejected exactly that, on the grounds that ownership was the
better path. Ownership is not available for these rules, so the
objection no longer has an alternative to prefer.

**Naming:** `Origin::Baseline` means "devcroft chose this for its own
reasons". These rules are not that — devcroft neither chose them nor can
remove them. `proposal.md` leaves the fourth-origin question open rather
than overloading an existing variant here.

## Decision 6: host-linked providers declare their grants; the tier name stops carrying the guarantee

**What:** a provider whose runtime links against host libraries declares
those paths as provider grants, compiled with `provider:<name>` origin.
The baseline supplies none.

**Why:** Decision 2 removes host library access, and `docs/decisions.md`
defines the artifact tier as "identical downloaded artifacts, host-linked
runtime […] behavior still depends on host libraries". Those two
statements cannot both hold silently. Something has to give, and the
options were to reject the tier outright or to make its requirement
explicit.

Explicit wins because it converts a documentation claim into a policy
fact. Today the difference between a closure and a host-linked runtime
lives in a tier name shown in `status`; afterwards it lives in
`policy --render`, as `provider:mise` grants on `/lib` and `/usr/lib`
that a flox or nix project simply does not have. A user comparing two
sandboxes can see the weaker guarantee instead of being told about it.

**What this costs, stated plainly:** host passthrough returns for such
providers. That is the thing `docs/decisions.md` rejects for `host` and
`none` — and the distinction being drawn is that there it was the whole
product contradicting itself, whereas here it is a declared, attributed,
renderable grant that the existing "provider resolution must not widen
the policy" rule already governs. If that distinction turns out to be
too fine in practice, the fallback is rejecting the tier, and this
decision is where that argument gets reopened.

**Consequence for the roadmap:** `add-mise-provider` is removed. Not
because mise fails the six criteria — it passes them, which is why
`docs/decisions.md` qualified it — but because the change was written
before this constraint existed and would have to be rewritten around it.
`pixi` and `hermit` remain qualified-but-unscheduled under the same
constraint. The criteria in §1 are unchanged; what changes is that
meeting them is no longer sufficient to inherit host access.

## Migration

1. Land the render comparison first, as a failing test: `policy --render`
   against `nono profile show` on the emitted profile. It fails today for
   two independent reasons (injected groups, keeper-exe grant), and it is
   what makes the invariant mechanical rather than aspirational.
2. Answer Decision 2's open question by running the samples with the
   exclusion in place. This is the gate; everything downstream depends on
   the answer, and no amount of design settles it.
3. Land the rest — `signal_mode`, the blocklist exclusion, the doctor
   range, the keeper-exe grant — each of which stands whether or not
   Decision 2 survives.

## Risks

- **Decision 2 may not survive contact with real projects.** Explicitly
  scoped so the change degrades to Decisions 3–5 rather than failing
  whole.
- **The injected group set is a nono implementation detail.** The
  profile guide states "a profile with no `groups.include` has no deny
  rules", which contradicts the measured behavior. Depending on the
  injection is depending on something undocumented — worth an upstream
  question, and worth a test that detects it changing.
- **Excluding a group is a wider blast radius than granting a path.**
  A missing grant fails one operation; an excluded group can fail every
  process at once. The samples are the regression surface for that
  reason.
