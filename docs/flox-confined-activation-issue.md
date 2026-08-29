# Feature request for Flox: separate environment materialization from `hook.on-activate`

**Status:** drafted, not yet filed. Written to be sent upstream to the Flox
project. Filing it is an external action on a third-party repository and is
deliberately left to the project owner — the same posture `use-nono-library`
takes for its own upstream ask.

## The request

Expose a public, versioned way to **materialize a Flox environment without
executing `hook.on-activate`**, and a separate, public way to run that hook in
a restricted context afterwards.

Today, every documented path that yields an activated environment also runs the
project's hook:

| invocation | runs `hook.on-activate`? |
| --- | --- |
| `flox activate -- <command>` | yes |
| `flox activate --mode dev` | yes |
| `flox activate --mode run` | yes |
| `flox activate --no-start-services` | yes |

Measured, not inferred — devcroft tested each of these against a real
environment (`fix-provisioning-hooks`). No combination suppresses the hook, and
there is no documented alternative entry point that returns the environment as
data.

## Why this matters to a consumer

devcroft runs project toolchains inside an OS-level sandbox. To do that it must
first *materialize* the environment — realise the packages a lockfile pins —
and then capture the resulting environment as data.

Materialization needs real authority: on Nix-backed providers it talks to the
`nix-daemon` socket, which is a host-global service shared by everything on the
machine. Running a project's hook needs no such authority; it is ordinary
project code, and devcroft treats it exactly like the project's own source.

Because Flox fuses the two, a consumer's obvious options are all bad:

1. **Give the hook daemon authority.** Project-controlled shell inherits a
   host-global capability over a store shared with every other environment on
   the machine. For devcroft's multi-agent case that means one repository's
   hook can affect every other agent's store.
2. **Refuse Flox environments that have a hook.** Safe, and unpleasant —
   `hook.on-activate` is idiomatic Flox, so this refuses a large share of real
   environments, including the ones `flox init` scaffolds.
3. **Run activation unconfined on the host.** An inversion: the project's code
   runs *before* any boundary exists, which is weaker than what that same code
   gets afterwards.

**What devcroft actually does, and why this request is still worth filing.**
There is a fourth option, and devcroft has taken it: materialize from a
**derived copy of the environment with the `[hook]` table removed**, then run
the hook afterwards inside a sandbox. Measured against real Flox, stripping
`[hook]` produces a byte-identical locked package set and an identical store
path — a hook is not a package input, so removing it cannot change what gets
realised.

That works, and it is not what a consumer should have to do. It means devcroft
parses and rewrites Flox's configuration format to reconstruct a boundary Flox
could simply expose. A schema change, a second place project code can run, or
any divergence between "what `[hook]` contains" and "what runs at activation"
would silently break it, and devcroft would not necessarily notice. The ask is
therefore not "unblock us" — it is "make this a supported contract rather than
an inference maintained against your internals".

## Other providers already do this

This is not a novel ask — it is the only supported provider that cannot do it:

| provider | hook-free materialization | runs the project hook? |
| --- | --- | --- |
| Nix flakes | `nix print-dev-env --json` | no — `shellHook` is returned as an inert string |
| Devbox | `devbox shellenv --pure` | no — `init_hook` is not run in any variant |
| **Flox** | **none** | **always** |

Both alternatives return the environment as structured data and leave the
project's shell to the caller's discretion. Note the near-miss on the Nix side,
because it shows how easy this is to get subtly wrong: plain `nix
print-dev-env` (without `--json`) emits a script ending in `eval
"${shellHook:-}"`, so it *does* run the hook. `--json` is the fix; the
distinction is exactly the one this request is about.

## Illustrative interface

The shape matters more than the spelling. Two separable operations:

```
# 1. Materialize from the lockfile. No project code runs.
flox environment materialize --locked --mode dev --json
    → environment as JSON, hook included as an inert string

# 2. Run the hook, separately and restrictedly, if the caller wants it.
flox environment run-on-activate
    → receives only the pre-hook context
    → writes the resulting environment to a dedicated descriptor
    → holds no nix-daemon connection
```

What a consumer needs from (1) is that it is **public and versioned** — a
documented interface with a stability commitment, not an internal file whose
format may change. devcroft deliberately depends on Flox's documented
`[services]` schema rather than the generated `service-config.yaml` for the same
reason, and would depend on this the same way.

What a consumer needs from (2) is that the hook runs with **no daemon authority
remaining**. If the hook can still reach the daemon after materialization, the
split has not bought anything.

## The flow this would make supported

```
1. trusted resolver
     materializes from the lockfile
     holds daemon authority
     runs no project code

2. restricted provisioning sandbox
     runs the project's hook
     private HOME, scoped caches, allowlisted egress
     no daemon authority
     environment captured as data

3. runtime sandbox
     project code runs under the manifest's own policy
```

Stage 1 and stage 2 are the split being requested. devcroft can already
construct that split by deriving a hook-free environment; what it cannot do is
*rely* on it. A public interface makes stages 1 and 2 Flox's contract instead
of devcroft's reconstruction of one.

## What we are not asking for

- Not asking Flox to sandbox anything. The confinement is the consumer's job;
  what is missing is the seam to confine *at*.
- Not asking for `hook.on-activate` to change, be deprecated, or be discouraged.
  It is a good feature and people rely on it.
- Not asking for a private or undocumented entry point. An unversioned internal
  would leave a consumer depending on something that can move without notice,
  which is the situation this request exists to get out of.
