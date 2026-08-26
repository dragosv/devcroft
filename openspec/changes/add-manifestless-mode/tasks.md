# Tasks — Manifestless Mode

## 0. Sequencing and shape

- [ ] **Do not start before `sandbox-provisioning` lands.** This mode points the
      tool at unreviewed repositories; until activation is confined, it runs
      their code on the host. See the proposal's Risk section.
- [ ] Choose and document the precedence order for ambiguous detection
      (design.md Q4).
- [ ] Decide what `--with` accepts — provider pass-through or common vocabulary
      (Q1). Pass-through is the smaller first move.
- [ ] Decide whether fleet may run in this mode or requires a manifest (Q3).

## 1. Resolution

- [ ] Implement the fixed resolution order, in one place, used by every command
      path.
- [ ] Signature-file detection at the worktree root only.
- [ ] Deterministic precedence on ambiguity; report the choice and the
      alternatives.
- [ ] `--provider` override.
- [ ] Failure naming what was looked for and which flags would supply it.

## 2. Ad-hoc environments

- [ ] `--with` packages, routed to the selected provider.
- [ ] Keep all mode state outside the worktree.
- [ ] Report that ad-hoc environments are not reproducible.
- [ ] Offer to write a manifest from what resolved; write only on explicit
      acceptance.

## 3. Policy

- [ ] Strict default for this mode: worktree writable, other paths denied,
      minimal network.
- [ ] Report the policy in force on every ad-hoc run.
- [ ] Denials name the path and point at declaring it rather than widening the
      default.

## 4. Diagnostics

- [ ] One line per ad-hoc run: what was detected, which provider, which policy.
- [ ] Provider errors attributed to the project, not to devcroft.
- [ ] `doctor` reports what would resolve for the current directory and why.

## 5. Validation

- [ ] A repository with only a flake resolves and runs.
- [ ] A repository with only a devbox config resolves and runs.
- [ ] A repository with two signature files resolves deterministically, both
      ways round.
- [ ] A repository with nothing runs with supplied packages.
- [ ] A repository whose environment configuration is broken produces a message
      that reads as the project's fault, verified by reading it cold.
- [ ] Detection does not escape the worktree — tested from a monorepo
      subdirectory.

## 6. Documentation

- [ ] README opens with an example that does not require installing a provider
      first.
- [ ] The resolution order is documented once and linked from error messages.
