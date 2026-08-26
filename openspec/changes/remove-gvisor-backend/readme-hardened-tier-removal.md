# README draft — hardened tier removal

Drop-in replacement for the tier table section. Trim to taste; the detail lives
in `openspec/changes/remove-gvisor-backend/design.md`.

> **Not yet applied.** This is the draft for task 3's README item, kept beside
> the change rather than merged into `README.md`, which still describes the
> hardened tier as implemented and verified. Applying it is that task, not this
> file's existence.
>
> One line below needs the G2 correction before it ships: "concurrent
> environments collided on service ports exactly as they would with no tier at
> all" is true only when egress is granted. A deny-default hardened sandbox got
> its own network namespace from devcroft's own OCI spec, so those did not
> collide. See `design.md`, the note under G2.

---

## Isolation

devcroft has one isolation tier: kernel-enforced path and network policy applied
in-process, with namespaces and resource limits around it. It is accident
protection — it contains an agent that misbehaves, deletes the wrong directory,
or fights another agent for a port. It is not a boundary against code written to
escape; the host kernel's full syscall surface remains reachable. If you need
that, run devcroft inside a VM. That is the supported answer, and it is already
how the macOS path works.

### We built a hardened tier and removed it

An earlier version had a second tier backed by gVisor. It worked — full
environment startup, exec, SSH, and services, verified against real tooling.
It was removed anyway, for three reasons worth stating:

**Landlock cannot confine anything that builds its own filesystem view.**
`runsc` needs `mount()`; Landlock has no hook for `mount()` at any ABI version,
so the two fail together with `EPERM` under any ruleset, however permissive.
Composing the tiers was structurally impossible, not merely fiddly.

**Rootless operation shared the host's network namespace.** The sandboxed
network mode is rejected under `--rootless`, so concurrent environments collided
on service ports exactly as they would with no tier at all. The direction this
project is going — many environments on one host, each with its own services —
is the one direction that tier could not go.

**The middle was squeezed.** Below it, the process tier is cheaper and matches
what devcroft is for. Above it, a VM is stronger and already required elsewhere.
A tier more complicated than the first and weaker than the second has to earn
its place, and every new feature would have had to be designed twice.

What we kept: the backend abstraction, so a future backend is an addition rather
than a rewrite; a written set of criteria any candidate has to meet; and three
integration defects that only appeared when real toolchains ran — mount
destinations needing to exist in the bundle beforehand, root path handling in
the generated bundle, and an argument separator the runtime rejects. None were
in any documentation. Budget for that class of defect in any sandbox
integration.
