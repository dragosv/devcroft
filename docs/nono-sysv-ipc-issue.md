# Feature request for nono: System V IPC is denied on macOS with no way to allow it

Draft for https://github.com/nolabs-ai/nono. Written against **nono
0.74.0**, measured on macOS 15.7.4 / aarch64-darwin. Not yet filed —
filing is the maintainer's to do.

## The request

`IpcMode::Full` should have a counterpart that also permits the System V
IPC family, or `CapabilitySet` should carry a separate capability for it.
Today there is no value of any public API that lets a sandboxed process
touch a System V shared memory segment it did not itself create.

The generated Seatbelt profile emits, unconditionally:

```lisp
(allow ipc-posix-shm-read-data)
(allow ipc-posix-shm-write-data)
(allow ipc-posix-shm-write-create)
```

and, under `IpcMode::Full`:

```lisp
(allow ipc-posix-sem*)
```

(`src/sandbox/macos.rs`, `generate_profile`.) The profile opens with
`(deny default)`, so `ipc-sysv-shm`, `ipc-sysv-sem` and `ipc-sysv-msg`
are denied in every configuration.

This is consistent with the documentation — `IpcMode::Full` says *"Full
POSIX IPC"*, and System V is not POSIX — so this is a scope request
rather than a bug report about a broken promise. But the consequence is
that a whole class of ordinary software cannot run.

## What it costs a consumer

**PostgreSQL cannot start.** It is not an exotic dependency, and it does
not use System V shared memory for its main shared memory — since 9.3
that is `mmap`'d anonymous memory. It creates one **56-byte** System V
segment as a postmaster interlock, to detect a second postmaster on the
same data directory. That one segment is enough:

```
FATAL:  could not create shared memory segment: Operation not permitted
DETAIL:  Failed system call was shmget(key=73998130, size=56, 03600).
```

Anything else using `shmget`/`semget`/`msgget` is in the same position.

## The non-obvious part, which is why this may have gone unnoticed

**`ipc-sysv-shm` gates access to an *existing* segment, not the creation
of a fresh one.** Measured inside a sandbox, same user, same flags:

| call | outside | inside `(deny default)` |
|---|---|---|
| `shmget(K, 56, IPC_CREAT\|IPC_EXCL\|0600)`, **fresh** `K` | ok | **ok** |
| `shmget(K, 56, IPC_CREAT\|IPC_EXCL\|0600)`, existing `K` | `EEXIST` | **`EPERM`** |
| `shmget(K, 56, IPC_CREAT\|0600)`, existing `K` | ok | **`EPERM`** |
| `shmget(K, 56, 0600)` — lookup only | ok | **`EPERM`** |

So a test that creates a segment with a fresh key and asserts success
**passes** with the operation denied. We wrote four such probes before
noticing, and each one cleared Seatbelt of involvement. If a regression
test is added for this, it has to create the segment outside the sandbox
and look it up inside, or it will pass either way.

Isolated to the single rule, with `sandbox-exec`:

| profile | existing-segment lookup |
|---|---|
| `(allow default)` | ok |
| `(deny default)` + the POSIX allows above | `EPERM` |
| same + `(allow ipc-sysv-shm)` | ok |

End to end, on a real PostgreSQL, one rule of difference:

```
(version 1) (allow default) (deny ipc-sysv-shm)
  -> FATAL: could not create shared memory segment: Operation not permitted

(version 1) (allow default)
  -> database system is ready to accept connections
```

## The platforms already disagree

Landlock does not mediate System V IPC, so on Linux the same
`CapabilitySet` permits what macOS denies. A consumer writing one
capability set for both platforms gets a working PostgreSQL on Linux and
a failing one on macOS, with nothing in the API to explain the
difference. That asymmetry seems worth closing independently of whether
the default changes.

## Illustrative interface

We are not asking for a specific shape. Two that would both work:

```rust
// (a) a third IpcMode, keeping the least-privilege default
pub enum IpcMode {
    SharedMemoryOnly,   // unchanged default
    Full,               // unchanged: POSIX shm + sem
    FullWithSystemV,    // adds (allow ipc-sysv*)
}

// (b) an independent capability, since SysV is orthogonal to the
//     POSIX sem question
caps.set_sysv_ipc(true);
```

(b) is probably better: a caller who needs PostgreSQL does not
necessarily want POSIX semaphores, and today they would have to take both.

A wildcard `(allow ipc-sysv*)` would mirror the existing
`(allow ipc-posix-sem*)` and the reasoning already recorded beside it —
that Seatbelt's operation taxonomy is not fully documented and
enumerating individual operations risks missing one.

## What we are not asking for

- **Not a change to the default.** `SharedMemoryOnly` staying the default
  is right; System V IPC is a global namespace with no per-sandbox
  scoping, and granting it by default would be a widening nobody asked
  for. We want it reachable, not automatic.
- **Not per-key or per-segment filtering.** Seatbelt does not appear to
  offer it, and we have no use for it. All-or-nothing for the sandbox is
  fine.
- **Not a Linux change.** Landlock permits this already; we are not
  asking for new restriction there to match macOS.
