# Feature request for nono: a typed capability for System V IPC on macOS

Draft for <https://github.com/nolabs-ai/nono>, in the fields of its
`feature_request.yml` template. Verified against **nono 0.77.0** (the
`ipc-sysv*` situation is unchanged from 0.74.0, where it was first
measured); measurements on macOS 15.7.4 / aarch64-darwin. Not yet filed —
filing is the maintainer's to do, in their own name; strip this preamble.

**An earlier version of this draft claimed no public API could grant
this. That was false** — `CapabilitySet::platform_rule` can, and
devcroft uses it. The request is now for the typed form, with the raw
rule declared as the workaround it is. Filing the earlier version would
have earned a one-line answer.

---

## What problem are you trying to solve?

PostgreSQL cannot start in a `nono` sandbox on macOS. It does not use
System V shared memory for its main shared memory (that has been
`mmap`'d anonymous memory since 9.3) — it creates one **56-byte** System
V segment as a postmaster interlock, and that one segment is enough:

```
FATAL:  could not create shared memory segment: Operation not permitted
DETAIL:  Failed system call was shmget(key=73998130, size=56, 03600).
```

Anything else using `shmget`/`semget`/`msgget` is in the same position.

The generated Seatbelt profile emits, unconditionally,
`(allow ipc-posix-shm-read-data)`, `(allow ipc-posix-shm-write-data)`,
`(allow ipc-posix-shm-write-create)`, and under `IpcMode::Full`
`(allow ipc-posix-sem*)` (`src/sandbox/macos.rs`, `generate_profile`).
The profile opens with `(deny default)`, so `ipc-sysv-shm`,
`ipc-sysv-sem` and `ipc-sysv-msg` are denied in every `IpcMode`. That is
consistent with the documentation — `IpcMode::Full` says *"Full POSIX
IPC"*, and System V is not POSIX — so this is a scope request, not a bug
report about a broken promise.

Two things make it worth a typed capability rather than leaving it to
raw rules:

**The platforms disagree silently.** Landlock does not mediate System V
IPC, so one `CapabilitySet` gets a working PostgreSQL on Linux and a
failing one on macOS, with nothing in the API to say why.

**The denial is easy to misdiagnose, and we did.** `ipc-sysv-shm` gates
access to an *existing* segment, not creation of a fresh one. Measured
inside a sandbox, same user, same flags:

| call | outside | inside `(deny default)` |
|---|---|---|
| `shmget(K, 56, IPC_CREAT\|IPC_EXCL\|0600)`, **fresh** `K` | ok | **ok** |
| `shmget(K, 56, IPC_CREAT\|IPC_EXCL\|0600)`, existing `K` | `EEXIST` | **`EPERM`** |
| `shmget(K, 56, IPC_CREAT\|0600)`, existing `K` | ok | **`EPERM`** |
| `shmget(K, 56, 0600)` — lookup only | ok | **`EPERM`** |

A test that creates a segment with a fresh key and asserts success
**passes** with the operation denied; we wrote four such probes before
noticing. A regression test has to create the segment outside the
sandbox and look it up inside. Isolated to the single rule with
`sandbox-exec`: `(allow default)` → ok; `(deny default)` + the POSIX
allows → `EPERM`; same + `(allow ipc-sysv-shm)` → ok. End to end on a
real PostgreSQL, `(version 1) (allow default) (deny ipc-sysv-shm)` is
the FATAL above and `(version 1) (allow default)` is "ready to accept
connections".

## What would you like to see?

A typed way to grant the System V family, so it appears in the
capability model — inspectable, serializable, diffable — rather than as
an opaque string. Two shapes that would both work; (b) seems better,
since a caller who needs PostgreSQL does not necessarily want POSIX
semaphores and would otherwise have to take both:

```rust
// (a) a third IpcMode, keeping the least-privilege default
pub enum IpcMode {
    SharedMemoryOnly,   // unchanged default
    Full,               // unchanged: POSIX shm + sem
    FullWithSystemV,    // adds (allow ipc-sysv*)
}

// (b) an independent capability, orthogonal to the POSIX sem question
caps.set_sysv_ipc(true);
```

A wildcard `(allow ipc-sysv*)` would mirror `(allow ipc-posix-sem*)` and
the reasoning recorded beside it — Seatbelt's operation taxonomy is not
fully documented and enumerating operations risks missing one.

Not asked for: a change to the default (`SharedMemoryOnly` staying the
default is right — System V IPC is a global namespace with no per-sandbox
scoping); per-key or per-segment filtering (Seatbelt does not appear to
offer it and we have no use for it); any Linux change.

## What have you tried instead?

`CapabilitySet::platform_rule("(allow ipc-sysv-shm)")`. It works — the
library appends platform rules after its own, and this one has nothing
to override anyway. It is what a consumer will do until a typed form
exists, and it is a fine escape hatch. What it cannot do is show up as a
capability: a policy renderer sees a string, a serialized set carries
SBPL text, and a Linux consumer of the same set has no way to learn that
the string is load-bearing on one platform and inert on the other.

## How is this blocking you?

It would be a nice improvement but I have a workaround.

## Additional context

Consumer is devcroft (<https://github.com/dragosv/devcroft>), which links
`nono` as a library for its process tier and renders every rule it hands
the backend with an origin; a raw rule is the one thing that renders as
"raw rule" rather than as a policy decision. Found while bringing up a
devbox `postgresql` service in a macOS sandbox; reproduced through a
second provider (flox), so it is the profile, not a provider's
arrangement. Worth knowing for anyone reproducing: macOS's per-process
System V segment table is small (`kern.sysv.shmseg` = 8; `shmmni` = 32),
and a day of failed runs leaks enough segments that the *next* failure is
`ENOSPC`, whose own hint points away from permissions. `ipcrm` first.
