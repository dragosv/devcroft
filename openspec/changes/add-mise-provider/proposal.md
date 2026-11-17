# Change: add-mise-provider

## Why

The owner explicitly requested mise on 2026-11-16. This satisfies the
demonstrated-demand condition in `docs/decisions.md` §1 and reopens the removed
sketch against today's policy and provisioning boundaries, rather than
restoring its obsolete host-access assumptions.

Revised 2026-11-17 after an adversarial review of the first draft. The review
found contradictions the measurement gate could not have resolved (a Linux
`/usr/bin` grant against the draft's own "no broad host toolchain" rule, a
derived configuration that silently moved the project root, an isolation list
that left out where mise installs), and it found that every hard problem in the
service half was independent of the provider half. Services therefore moved to
`add-mise-daemons`, which depends on this change.

## What Changes

- Accept `env.provider = "mise"` as an **artifact-tier** provider on qualified
  native Linux/macOS profiles. The tier and the host-runtime dependence stay
  visible in `up`, `status` and rendered policy.
- Require a `mise.lock` entry with a URL **and** a checksum for every selected
  tool on the current platform. Admit only backends that record both and whose
  installation executes nothing it downloaded; refuse the rest by tool and
  backend before any subprocess. Group 0 decides the admitted list.
- Host-side mise sees a devcroft-written derived configuration holding
  `[tools]` and nothing else, under an environment built from an allowlist
  (isolated `HOME` and every mise directory, including the one installs land
  in). The project's `[env]` is interpreted by devcroft and run in declaration
  order in confined provisioning, anchored at the real project root.
- Classify every top-level table of the project's configuration: consumed,
  refused by key, or stated as not carried. Refuse every other project-level
  mise configuration file by path.
- Materialize into immutable per-recipe objects under
  `~/.local/share/devcroft-artifacts/mise/`. A sandbox's recorded state is the
  reference to its object; an object nothing references is removed at `rm` and
  `up --recreate`.
- Supply devcroft's shell from the locked environment or, failing that, from
  the narrowest host grant that works: the shell's own file plus its loader and
  libraries on Linux, the shell's directory on macOS. **No `/usr/bin` grant on
  either platform.**
- Refuse `[daemons]` by name until `add-mise-daemons` lands; never read them as
  an empty service set.
- Add provider-aware `init`, `doctor`, fingerprints, samples and qualification
  evidence, including a real build.

## Capabilities

### New Capabilities

None: this extends the existing provider contract.

### Modified Capabilities

- `config`: accepts mise without changing the default or other provider values.
- `env-provider`: locked resolution, tools-only host capture, confined `[env]`,
  immutable objects, input identity and staleness.
- `policy`: attributed artifact and host-runtime grants, the narrow shell
  grant, and refusal of a manifest write grant that overlaps the artifact store.
- `cli`: discovery, diagnostics, visible guarantee and actionable refusals.

The main spec tree is empty, so these are provider-scoped **ADDED**
requirements, following the archived provider changes. They contradict one
scenario in the still-unarchived `add-mvp-core` (`specs/env-provider/spec.md`,
"Only declarative providers": `provider = "mise"` fails as not yet supported).
Task 1.1 amends that scenario in place rather than leaving two active changes
that disagree.

## Impact

Expected implementation touches `src/provider/{mod,validate,mise}.rs`,
`src/provider/capture.rs`, `src/policy/capability_set.rs`, `src/shell.rs`,
`src/lifecycle/{up,state,status}.rs`, `src/bin/devcroft.rs`, tests and samples.

Whether reading binary headers needs a new dependency is decided in task 0.4.
The first draft said "no new Rust dependency" while also requiring static
inspection of ELF and Mach-O files; that sentence is withdrawn so it does not
constrain the answer.

`mise run` is not available inside a sandbox, because mise itself is not in
it. A project's `[tasks]` are therefore not runnable by name there. This is a
stated limitation (`doctor`, the sample README, `docs/known-gaps.md`), not
something the change works around.

Group 0 in [tasks.md](tasks.md) is a measurement gate with stop criteria, not a
claim that mise already passes. Existing providers and policies are unchanged.
`add-pixi-provider` is an independent sibling; the artifact store, the
allowlisted subprocess environment and the narrow shell grant are the same
contract in both and are extracted when the second one lands, not before.

The `new/` material constrains future composition: native support here is not a
VM, companion or fleet support claim, and this change selects no VMM or kernel.

See [design.md](design.md) for evidence, decisions and what the review changed.
