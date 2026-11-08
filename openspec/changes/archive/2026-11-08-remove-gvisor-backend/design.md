# Design — Removing the gVisor Backend

The point of this document is that the tier was built, measured, and removed.
The measurements are the part worth keeping.

## Findings that survive the removal

### G1 — Landlock and gVisor cannot be stacked, in that order

`runsc` needs `mount()`. Landlock has no hook for `mount()` at any ABI version,
so a Landlock ruleset denies it and `runsc` fails with `EPERM` regardless of how
permissive the rest of the policy is. This is not a configuration problem and no
grant fixes it.

Generalisation worth carrying: **Landlock cannot confine anything that builds its
own filesystem view.** Any future component that mounts — a container runtime, a
mount-namespace sandbox, an overlay tool — has to run *outside* the ruleset, not
inside it. The ordering constraint in `add-linux-agent-fleet` (D2) comes from
the same fact.

### G2 — Rootless `runsc` shares the host network namespace

The sandboxed-network mode is rejected under `--rootless`, so instances share
the host's port space. This is why the tier could never support fleet: the
feature that makes concurrent environments work is one the tier structurally
lacks.

Generalisation: **check the isolation a backend gives under the privilege level
you will actually run it at**, not the isolation it advertises. Rootless
operation removed the property the tier was chosen for.

> **Correction, from verifying this finding against the code (task 0).** The
> first sentence is right and the second overstates it, in exactly the way
> `add-port-allocation`'s proposal already caught once and corrected in place.
> Port separation at this tier never came from `runsc`'s netstack; it came from
> the network namespace **devcroft itself requests in the OCI spec**.
> `oci_spec::build` pushes a `network` namespace entry with no `path` — a fresh,
> isolated netns — whenever the policy resolves to `NetworkMode::None`
> (`oci_spec.rs`, asserted by
> `deny_all_policy_produces_network_none_and_a_fresh_netns`).
>
> So N hardened sandboxes with a **deny-default** network do *not* collide on
> ports; each has its own loopback. The host's port space is shared only under
> `NetworkMode::Host`, which is what a granted egress allowlist resolves to.
>
> The finding still holds where it matters — a tier whose port isolation
> disappears the moment egress is granted cannot carry fleet, and the
> generalisation about privilege level is untouched. But "the tier structurally
> lacks it" is not accurate, and the removal should not rest on a reason the
> repo has already corrected elsewhere. G1 and the squeezed middle are the
> reasons that carry their own weight.

### G3 — Three integration defects found only by running real tooling

Recorded because they are the recoverable value of the work:

- mount destination directories must exist in the bundle's root filesystem
  before the runtime starts, rather than being created implicitly;
- `root.path` handling in the generated bundle did not accept the form initially
  produced;
- `runsc exec` does not accept the argument separator that the equivalent
  Docker-style invocation does.

None were discoverable from documentation. All three cost hours. Any future
backend integration should budget for the same class of defect and plan to find
them by running real toolchains, not by reading specifications.

> **Verified against the code (task 0), all three accurate.** In the repo's own
> terms: `runner::materialize_bundle` creates every mount's destination inside
> `rootfs/` (`materialize_bundle_writes_config_json_and_pre_creates_every_mount_point`);
> `oci_spec::build` emits `root.path` as `bundle_dir/rootfs`, **absolute**,
> because gVisor's rootless gofer compares the opened destination against
> `/proc/self/fd/<n>`'s always-absolute `readlink` target as a symlink-escape
> guard, which a relative path can never satisfy; and
> `runsc_command::exec_args` carries argv with **no** `--` separator
> (`exec_args_carry_the_argv_directly_with_no_separator`), since `runsc exec`'s
> usage is `exec [options] <container-id> <command> [args...]` — the literal
> `--` became the command's own argv[0].

### G4 — Bypassing the composition core costs less than it first appears

The initial framing was that a backend not routed through the sandbox library
cannot receive any of that library's capabilities, and that the gap grows with
every release. On inspection that overstates it: most of the library's modules —
host filtering, supervisor IPC, attestation, audit, snapshots — run in the
supervisor, outside any sandbox, and work identically regardless of backend.

What genuinely cannot transfer is the capability-set-to-Landlock application
path and the ABI-level scoping that goes with it. That is a real but narrow gap.

**Recorded so the removal is not justified by a stronger claim than the facts
support.** The reasons that hold are G1, G2, and the squeezed-middle argument —
not "the backend gets nothing".

### G5 — The abstraction was worth building; keep it

The session backend trait was introduced to prove the design generalised, in the
same way a third environment provider was. That it now has one implementation
does not make it wrong — it makes the cost of a second implementation known
rather than guessed, which is what the exercise was for.

Keep the trait. Removing it would make any future backend a re-architecture
rather than an addition, and would discard the one durable artifact of this
work.

## Criteria for a future backend

Written now, while the reasons are fresh, so the next candidate is judged
against them rather than against enthusiasm:

1. **Composes with, or runs beneath, the sandboxing core.** A substrate the
   library runs *inside* is fine — this is how the macOS VM path works, and it
   is a different thing from a backend that replaces the library.
2. **Supports what fleet needs**: independent network namespaces, resource
   control, an independent port space per instance. A backend that cannot do
   these is a single-environment backend, and must declare that.
3. **Runs real toolchains.** Compilers, language runtimes, package managers,
   process supervisors — demonstrated, not assumed.
4. **Is a security boundary its own authors will call one.** Preview projects
   that decline to make that claim do not clear this bar.
5. **Is reachable from Rust as a library**, not only as a subprocess with exit
   codes. The move from invoking the sandbox CLI to consuming it as a crate
   was a real gain; a new backend should not undo it.

## Rejected alternatives

**Freeze rather than remove.** Retaining the code marked unsupported preserves
the option and an answer to the "not a real boundary" objection. Rejected
because the cost is not maintenance of the existing code — it is that every new
capability needs a decision, a tested refusal and a documentation line for a
tier nobody runs. The capability matrix makes saying no cheap, not free.

**Compose the other way — library inside gVisor.** Requires gVisor to implement
the Landlock syscalls, which is unverified, and would restrict paths inside a
root filesystem devcroft already fully controls. Near-zero marginal value even
if it works.

**Replace it with a different strong-isolation backend.** Evaluated: a library
OS whose own authors describe it as pre-stable and not production-ready, and a
cross-platform sandbox SDK that sits at the same architectural layer as the
session backend trait rather than beneath it, is TypeScript-first, and states
that its profiles should not currently be treated as security boundaries.
Neither clears the criteria above. The VM path remains the answer for anyone
needing a stronger boundary, and it is already required on macOS.

## Consequence to state plainly

The isolation ceiling is now the process tier, permanently as far as this
roadmap goes. Anyone needing to run code written to escape should run devcroft
inside a VM — which is a supported answer, not a deflection, since the macOS
path works that way already.
