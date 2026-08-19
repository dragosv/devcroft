# Design: own-policy-baseline

## Context

See `proposal.md` — Why. The short version: `extends: "default"` makes
240 rules reach the backend that `policy --render` cannot show, and the
invariant says that must not happen.

Everything below rests on measurements taken against a live nono 0.71.0
in this repo's devcontainer, not on reading nono's source. The commands
are recorded so the numbers can be re-derived when nono changes:

```sh
nono profile show default --json          # the 18 groups `default` includes
nono profile groups <name> --json         # the rules in one group
nono profile validate <file>              # schema check for an emitted profile
```

## Goals / Non-Goals

**Goals:**

- Every rule that reaches the backend is one devcroft chose and can
  print.
- No change in effective access for a project that works today.
- The compatible surface against nono becomes the profile schema, not a
  ruleset — so a nono minor release stops being a compatibility event.

**Non-Goals:**

- Reimplementing nono's command blocklist. It is inert in the mode
  devcroft uses, and adopting it is a policy stance devcroft has not
  taken.
- Owning a general-purpose system-paths database. devcroft needs the
  paths *its* sandboxes need; nono's catalog serves a broader case and
  stays nono's.
- Changing the enforcement mechanism. This change is about what devcroft
  says, not who applies it. `use-nono-library` is the separate question.

## Decision 1: enumerate the baseline, do not extend

**What:** `to_nono_profile` stops emitting `extends` and emits the
baseline rules inline, each already carrying `Origin::Baseline`.

**Why:** the alternative — keeping `extends` and teaching
`policy --render` to shell out to `nono profile show default --json` and
print the inherited rules — was considered seriously, because it repairs
the *visibility* half without owning anything. It was rejected on two
grounds. First, it makes `--render` depend on invoking the backend
binary, so rendering a policy requires a working nono, which `--render`
today does not. Second, and decisively, it renders rules devcroft cannot
justify: 69 that are redundant under the allowlist model and 49 that are
inert in `wrap` mode. Printing them accurately would make the output
*more* misleading, not less — a reader would see a command blocklist
that does not apply.

**Evidence it is sufficient:** a hand-built profile with no `extends`,
carrying the 61 `system_read_linux_core` paths plus devcroft's rules,
was run under `nono wrap`:

| probe | result |
|---|---|
| `sh -c 'echo EXEC_OK'` | `EXEC_OK` |
| `cat f.txt` in project root | file contents |
| `cat ~/.ssh/probe_key` | `Permission denied` |

**What this corrects:** the comment at `src/policy/mod.rs` states that
without `extends` a profile "can't exec anything at all — confirmed
against a live nono 0.71.0: a from-scratch manifest with no `extends`
denies even `/usr/bin/cat` (EPERM)". That observation was real but the
generalization drawn from it was wrong: the profile tested was empty.
The finding is "a profile must grant the linker and system binaries",
not "a profile must extend `default`".

## Decision 2: drop the deny groups' rules, keep devcroft's own denies

**What:** the 69 rules in the eight `required` groups
(`deny_credentials`, `deny_keychains_*`, `deny_browser_data_*`,
`deny_shell_history`, `deny_shell_configs`, `deny_macos_private`) are
not carried over. devcroft's existing five deny entries stay.

**Why:** measured redundant. With devcroft's `deny` list emptied
entirely and no `extends`, `~/.ssh/probe_key` and `~/.bashrc` were both
`Permission denied` — the allowlist model already excludes them.
devcroft grants a project root and a store path; a rule denying
`~/.aws` is denying something that was never reachable.

**Why keep devcroft's five, then:** they are not redundant in the same
way. They are the written form of the "baseline denials always win"
invariant, and their value is as a stated guarantee that survives a
future manifest granting something broad. Whether that reasoning also
argues for re-adding nono's set is `proposal.md`'s first open question,
and is deliberately left open rather than settled here.

**Risk:** if a future change grants a home-directory path — a plausible
provider grant — the redundancy argument stops holding and these rules
become load-bearing. The mitigation is that such a grant must not widen
the policy silently anyway (the provider-resolution invariant), so it
would surface as a decision rather than a regression.

## Decision 3: drop the command blocklist rather than reimplement it

**What:** `dangerous_commands`, `dangerous_commands_linux`,
`dangerous_commands_macos` — 49 rules — are not carried over, and
`deny.commands` is not emitted at all.

**Why:** verified inert. Under `extends: "default"`, `rm victim.txt`
and `cp f.txt f2.txt` both succeeded inside `nono wrap`. The mechanism
is enforced by nono's supervisor in `run`/`shell` mode, where nono stays
resident and can intercept exec; `wrap` applies the restriction and
execs away, leaving nothing to intercept.

Reimplementing it would also be wrong on the merits. The list denies
`rm`, `mv`, `cp`, `npm`, `pip`, `rsync`, `xargs` — every one of which a
build runs. devcroft's own framing calls the process tier "accident
protection", and there is an argument that a blocklist serves exactly
that; but the argument has to be made and the list chosen, not inherited
by accident from a profile written for a different product.

**What this obliges:** if the blocklist is later wanted, it is a change
of its own with its own justification, and it must state which
enforcement mode makes it real.

## Decision 4: the keeper-executable grant moves into the compiled policy

**What:** the `filesystem.read` entry for the directory containing the
devcroft binary is compiled as a rule with an origin, not appended to
the profile after compilation.

**Why:** found while measuring — `profile.json` contains it,
`policy --render` does not print it. That is the same invariant
violation as `extends`, reached by a different route, and fixing one
while leaving the other would be incoherent. It needs an origin that
says what it is; `Origin::Baseline` is the closest existing fit, since
it is devcroft's own requirement rather than the user's or the
provider's.

## Decision 5: `doctor` tests a surface, not a version

**What:** the `>=0.71.0, <0.72.0` range widens, and the failure message
names what compatibility means.

**Why:** the narrow window exists because `default`'s contents were an
undocumented dependency. Removing that dependency leaves two things
devcroft actually relies on — the named-profile schema it emits, and
`wrap`'s invocation shape. Both are checkable directly: nono ships
`nono profile validate` and `nono profile schema`, so a test can assert
devcroft's emitted profile validates against the installed nono's own
schema. That turns "which versions work" from a guess maintained by hand
into something the test suite answers.

**Consequence:** `doctor`'s range should be widened to versions the
suite has been run against, and the schema-validation test is what
justifies widening it. Shipping the wider range without the test would
just move the guess.

## Migration

The change is invisible to a working project by construction, and the
way that is checked matters more than the change itself:

1. Add the schema-validation test against the installed nono first, so
   drift is detectable before anything moves.
2. Land the baseline enumeration behind the existing compile path, and
   assert `policy --render` output and `profile.json` contain the same
   rule set — the test that makes the invariant mechanical.
3. Re-run the sample projects end to end at both tiers. `samples/` is
   the regression surface here: `flox-clap-sample` builds Rust,
   `nix-go-sample` builds Go, and a missing linker path shows up as a
   build failure rather than a subtle denial.

## Risks

- **A missing path is a build failure, not a warning.** The failure mode
  of an incomplete baseline is "the toolchain cannot exec", which is
  loud but unhelpful. `why` must be able to attribute it, which is why
  that is a success criterion rather than a nicety.
- **Host diversity.** The measured set comes from one Debian-family
  devcontainer. musl, NixOS, and macOS all differ. This is the strongest
  argument for keeping `extends`, and the honest answer is that it
  trades an inspectable, testable gap for an opaque, untestable one.
- **nono's `default` may improve.** If nono later publishes its group
  catalog as data devcroft can consume and render, Decision 1 should be
  revisited — that would give visibility without ownership, which is
  strictly better than either option available today.
