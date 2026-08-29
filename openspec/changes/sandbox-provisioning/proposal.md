# Sandbox Provider Resolution

**Depends on:** `add-egress-proxy`. Provisioning needs the network — package
installs are the common case — so confining it without a domain allowlist leaves
unreviewed code with open egress. See `design.md`, open question 2.

That dependency has now shipped, and it handed this change a requirement along
with the mechanism: **"Network policy is declared per context" moved here**
(`specs/network/spec.md`) when `add-egress-proxy` landed. It could not be
satisfied there — a per-context policy needs two contexts, and provisioning is
the second one, which this change is what creates.

## Why

`devcroft up` on a flox or devbox project runs that project's code on the host,
outside any sandbox. Provider resolution happens host-side before a boundary
exists — that is how the toolchain gets materialized — and a flox manifest's
`[hook].on-activate` is arbitrary shell that `flox activate` executes. Today
this is detected and warned about, because it cannot be prevented.

Warning is the right answer for a developer opening their own repository. It is
the wrong answer for anything beyond that. Running several agents against
repositories nobody has read — external contributions, dependency updates,
anything the agent itself fetched — means `up` is arbitrary code execution on
the host, and the sandbox around the agent is irrelevant because the code
already ran before it existed.

**What this change does and does not deliver.** It closes an inversion: today
`up` runs project code on the host, *outside* any boundary, which is weaker than
what the agent itself gets. This change moves provisioning to the same
process-tier boundary as everything else. It does **not** make devcroft safe for
running code written to escape — the process tier is accident protection and the
full host kernel surface stays reachable. See
[docs/threat-model.md](../../../docs/threat-model.md), "Two use cases". Wording
in this change and in `up`'s output must not imply otherwise.

The hook is not an abuse of the format. It is where people put everything Nix
does not do: `npm ci`, `poetry install`, generating a `.env`, running
migrations, `git config`, writing into `~/.cache`. Expecting hooks to touch only
Nix-related paths is not a realistic assumption to build on.

The nix provider does not have *this* problem — `print-dev-env --json`
returns the dev shell's environment as data and never evaluates the
`shellHook`. This change gives the other providers a comparable property by a
different route: not by avoiding execution, but by confining it.

Worth being exact about what "as data" does and does not cover, since a
reviewer flagged the looser reading of this sentence as an overclaim:
producing that JSON means **evaluating `flake.nix`**, which the repository
controls. What nix avoids is running the project's *shell*; it does not avoid
interpreting the project's Nix. The difference that makes this acceptable is
the evaluator — pure, no ambient shell, no arbitrary syscalls — not an absence
of repository-controlled input. The `env-provider` delta spec states the same
boundary normatively; neither document should be read as claiming nothing from
an unreviewed repository is interpreted host-side.

## What Changes

- **MODIFIED** `env-provider`: provider resolution runs inside a sandbox
  (the *provisioning sandbox*) rather than on the host. The captured environment
  is read out as data.
- **MODIFIED** `policy`: a second policy profile, distinct from the runtime one,
  governs provisioning. It is declared in the manifest and rendered by
  `policy --render` like any other.
- The host-side execution warning is replaced by an accurate statement of what
  provisioning is permitted to reach.

## Impact

- Affected specs: `env-provider`, `policy`, `network` (the per-context
  requirement inherited from `add-egress-proxy`)
- Affects the flox and devbox providers. The nix provider gains a provisioning
  profile for consistency but its resolution path is unchanged.
- Linux first. On macOS the same structure applies with the platform's sandbox
  backend, at whatever fidelity that backend allows; the degradation is reported
  rather than silent.

## Non-Goals

- Making provisioning hermetic. A hook that runs `npm ci` needs the network and
  a writable cache. The goal is a stated boundary, not the absence of side
  effects.
- Preventing a hook from modifying the project's own working tree. That is
  what hooks are for.
- Treating read-write `/nix` access, or a connection to a host-global
  package-manager daemon, as a narrow filesystem grant. Materialization
  authority is modelled as its own capability (design.md P2a) and is never
  handed to project-controlled activation code (P2b). A provider that cannot
  separate the two fails closed rather than receiving either as a fallback —
  which today means Flox environments with `hook.on-activate` (P2c). The
  upstream request that would unblock them is drafted at
  [docs/flox-confined-activation-issue.md](../../../docs/flox-confined-activation-issue.md).
