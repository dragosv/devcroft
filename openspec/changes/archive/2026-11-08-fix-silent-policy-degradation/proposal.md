## Why

**`devcroft why` and `policy --render` answer wrongly, not partially, when
`meta.json` cannot be read — and the place they cannot read it is inside
devcroft's own sandbox.**

Measured, same project and same sandbox, only the readability of
`~/.local/share/devcroft/<name>/meta.json` differing:

```
readable:    ALLOWED   allowed by rule provider:flox
unreadable:  DENIED    denied: not granted by any rule      exit=0
```

`policy --render` loses the store grant entirely in the second case. Neither
prints a warning; both exit 0.

The cause is one line. `compile_with_provider_grants` (`bin/devcroft.rs`)
compiles from the manifest, then folds in three things it can only learn from
`Meta` — the provider's `read_only_grants`, the proxy port, and the services
socket — behind `let Ok(Some(meta)) = read_meta(..) else { return compiled }`.
`read_meta` returns `Err` on `EPERM` (`state.rs:362`), and that arm is
indistinguishable from "no sandbox exists", which is a legitimate case the
fallthrough was written for.

**Why the sandbox is the case that matters.** `DEVCROFT_DATA_DIR` is
`filesystem_deny` with origin `Baseline`, deliberately not overridable by any
manifest (`policy/mod.rs:252`). So *every* invocation from inside a sandbox
takes the degraded path, by design and permanently. This is not an edge case
reachable by a misconfiguration; it is the guaranteed behaviour of the only
context where an agent could ask.

**Why now.** `add-agent-workload` needs `why` to be the thing an agent runs
when it hits a policy denial (`docs/prior-art.md`, the technique taken from
nono's registry packs). An answer that is confidently inverted is worse than
no answer: the agent acts on it, concludes its toolchain is ungranted, and
either gives up or asks for something absurd. That change cannot build on
this until it is fixed — but the bug is not an agent bug and should not wait
behind agent design decisions.

## What Changes

- **NEW** `policy-view-fidelity`: a rendered or explained policy is either
  complete or says it is not. The three `Meta`-derived contributions become a
  named, checkable input rather than a silent best-effort.
- `why` and `policy --render` distinguish **"no sandbox exists"** (the
  manifest-only answer is correct and complete) from **"a sandbox exists but
  its record is unreadable"** (the answer is incomplete and must say so).
- `up` writes the compiled policy, origins included, to
  `.devcroft/<name>/policy.json` — inside the project root, the one place the
  sandbox both reads and writes (`services::artifact_dir`, already gitignored
  by `init`). `why` and `policy --render` prefer it when the state dir is
  unreadable, which makes the in-sandbox answer complete rather than merely
  honest about being incomplete.
- **Not in this change**: any agent wiring, any grant of devcroft's own
  binary inside the sandbox. Both are `add-agent-workload`'s, and both depend
  on this.

## Capabilities

### New Capabilities

- `policy-view-fidelity`: what `why` and `policy --render` may claim about a
  policy they could not fully reconstruct, where the in-sandbox copy lives,
  and the invariant that the two views never disagree with what the backend
  was given.

### Modified Capabilities

- (none — `openspec/specs/` holds no synced specs. The `policy` capability
  this corrects lives in the unarchived `add-mvp-core`; its "Policy is
  deterministic and inspectable" requirement is not being changed, it is
  being made true in a context where it currently is not.)

## Impact

- **Affected code**: `bin/devcroft.rs`'s `compile_with_provider_grants`,
  `cli_why`, `cli_policy`; `lifecycle::up` gains one artifact write;
  `services::artifact_dir` gains a second consumer.
- **This is the policy invariant, not a cosmetic one.** "Nothing goes to the
  backend that cannot be shown via `policy --render`" is a stated
  architecture invariant. Today the backend is given the store grant and
  `--render` does not show it — from inside the sandbox, always. The
  invariant is already violated; this change is what makes it hold.
- **A second copy of the policy is a divergence risk**, and the mitigation is
  that it is written by `up` from the same `CompiledPolicy` the
  `CapabilityPlan` is derived from, in the same function, or it is not
  written at all. Design decides whether that is one artifact or two.
- **No behaviour change on the host.** Every host-side invocation already
  reads `Meta` successfully and keeps the answer it has today.

## Non-Goals

- **Not making the data dir readable from inside.** The baseline deny is
  load-bearing — it is what keeps a sandbox out of other sandboxes' keys,
  sockets and state. The fix moves the *data the sandbox is entitled to*
  outward, never the boundary inward.
- **Not exposing anything new.** The compiled policy is already fully
  visible to that sandbox's user via `policy --render` on the host, and
  describes only that sandbox's own grants. Writing it where the sandbox can
  read it discloses nothing the sandbox could not already infer by trying.
- **Not a `why --self` mode.** Whether the in-sandbox query gets its own
  flag, or the existing command simply works, is a design question, not a
  premise.
