# microVM experiments

This is the experimental implementation of `evaluate-microvm-runtimes`.
`libkrun/` has its own Cargo workspace and lockfile. The main devcroft build and
published crate do not depend on it. No production VM dispatch exists yet.

## Current result

The pinned libkrun **2.0**, plus the recorded local half-close patch, boots a
Linux ARM64 Stage-A guest on macOS with
Seatbelt applied **before** payload loading, devices or `VmmBuilder::build()`.
Only raw block devices, console, plain vsock, RNG and balloon are configured.
There is no NIC, TSI, virtiofs share or host project execution.

The latest two-boot run **passed** READY, ping, half-close with 130,816 bytes of
pending input and matching responses, oversized-frame rejection, backend
read-only enforcement, private raw state persistence and normal shutdown.
Its seven Seatbelt checks passed. The newly built kernel exposes Landlock ABI 6;
a guest child process demonstrated allowed read/write and denied read/write
outside its path grant. See [current evidence](results/libkrun-macos-arm64-landlock.json).
That earlier run used direct Landlock syscalls.

The [keeper integration run](results/libkrun-macos-arm64-keeper.json) adds the
actual `src/keeper` implementation, cross-compiled with nono **0.77.0**, and a
hash-pinned Alpine static BusyBox. Both cold boots passed separate stdout/stderr,
exit status, 128 KiB binary stdin with EOF, request environment/cwd, PTY and resize,
INT/TERM/HUP, session-query/disconnect cleanup and malformed/oversized keeper
frames. Commands launched by the keeper demonstrated allowed workspace writes,
denied private-file reads, denied runtime-data writes and denied TCP bind, with
working controls before restriction. `nono` applies before the first keeper/relay
thread and is inherited by its sessions. The kernel now includes Unix98 PTYs.

The fixed host channels are `control.sock` → vsock 1024 (PID 1 administration)
and `keeper.sock` → vsock 1025 (keeper framing, relayed to its guest Unix socket).
Both remain inside the private instance directory and exact Seatbelt socket
grants. The default console now creates a real hvc0 terminal; guest logs flow
through the helper's bounded stderr capture. libkrun logs repeated ENOTCONN
shutdown warnings at connection teardown; functional checks pass, but that noise
remains recorded in the raw evidence.

This does not qualify a complete backend. The keeper fixture has an in-memory
workspace, synthetic private sentinel and restricted TCP; it is not the complete
project policy or a Nix closure. SSH/SFTP, services, editor, credential delivery,
crash recovery and density tests remain later stages. The ARM64
kernel and configuration are now built and recorded; the x86-64 kernel row remains
not-run. Linux host confinement is not implemented, so the helper refuses boot
on Linux. The [original failed evidence](results/libkrun-macos-arm64-stage-a.json)
is retained: it used an unpatched library and a diagnostic kernel without Landlock.

## Build and run on macOS ARM64

Requires installed Rust, Python 3.11+, Zig, Lima and macOS signing tools. Downloaded
artifacts and build output stay under this experiment; project dependencies are
not installed on the host. Use the pinned lockfile:

```sh
rtk proxy python3 experiments/microvm/build_helper.py
rtk proxy python3 experiments/microvm/build_kernel.py
rtk proxy python3 experiments/microvm/build_guest.py
rtk proxy python3 experiments/microvm/smoke.py
```

To include keeper/nono (requires Rust target `aarch64-unknown-linux-musl`):

```sh
rtk proxy rustup target add aarch64-unknown-linux-musl
rtk proxy python3 experiments/microvm/build_keeper_guest.py
rtk proxy python3 experiments/microvm/smoke.py --guest experiments/microvm/artifacts/stage-b/guest.json --keeper
```

The guest has a separate workspace and lockfile. The build uses installed Zig
for both Rust linking and native dependencies, and disables Rust's duplicate
self-contained CRT. The guest receipt records all reused keeper source hashes,
guest entry point, lockfile, compiler version and BusyBox input/output identities.
Only the keeper environment-name filter moved out of host lifecycle code;
native session behavior and the public activation constants remain unchanged.

`build_helper.py` fetches the exact Git revision into ignored artifacts, applies
the reviewed patch and verifies the complete effective diff before building and
signing. The helper embeds the patch hash in its provenance. `--from CHECKOUT`
can seed this from an existing repository at the pin. The source tree is not a
modified global Cargo cache.

`build_kernel.py` uses a dedicated pinned Alpine builder image with 4 vCPUs,
4 GiB RAM and no host mounts. Source and build files live in a 3 GiB guest tmpfs;
the source archive is SHA-256 pinned against kernel.org. Build time is bounded to
15 minutes. It copies out Image/config/toolchain metadata, then stops the builder.
The source and output identities are fixed in the build receipt; installing tools
from Alpine repositories is not a bit-reproducible toolchain guarantee. Builder
disks remain in its dedicated `LIMA_HOME` unless explicitly removed.

`fetch_kernel.py` remains available for the older diagnostic fixture, using its
committed OCI digests. It does not produce the default Landlock kernel. No kernel
binary is committed or included in devcroft's package.

The smoke process must run outside an enclosing Seatbelt sandbox, since macOS
refuses a second `sandbox_init`. When running from a sandboxed coding agent,
request escalation for the smoke command. The helper still installs its own
deny-default profile; there is no `--no-sandbox` switch.

`smoke.py` creates a mode-0700 temporary instance with private state, bounds logs
to 1 MiB per stream, applies readiness/control/shutdown deadlines and reaps its
owned process group on failure. It retains the disk for diagnosis and writes
launch records, logs and results under `runs/`. These local diagnostic instances
contain no imported projects or credentials. The helper strips inherited FDs
above stdio and locks the state disk against concurrent use. Launch artifacts
must be trusted experiment-owned inputs, outside the writable instance directory;
hashes identify bytes and are not authorization for arbitrary untrusted kernels.

## Checks

```sh
rtk proxy cargo fmt --manifest-path experiments/microvm/libkrun/Cargo.toml --check
rtk proxy cargo clippy --manifest-path experiments/microvm/libkrun/Cargo.toml --locked --offline -- -D warnings
rtk proxy python3 -m unittest discover -s experiments/microvm -p test_stage_a.py -v
rtk proxy cargo test --manifest-path experiments/microvm/guest-keeper/Cargo.toml --locked
```

The ten interface tests exercise the compiled helper: patch provenance, input corruption, input
aliasing, writable-scope overlap, unknown network fields, private directory
permissions, resource bounds, oversized records and distinction between input
validation and runtime qualification, plus occupied endpoints/dangling symlinks.
The interface tests require a built helper and do not silently skip. The guest
workspace runs the existing 21 keeper tests on the host. Confinement probes run
in separate processes inside `smoke.py`
because Seatbelt entry is irreversible and process-wide.

The initial keeper evidence records the then-existing macOS compilation failure
from host lifecycle references to Linux-only `fleet::init`. Commit `ad92513`
fixed that independently. Revalidation on 2026-11-08 passed all 438 library tests
(including the 21 keeper tests) and native `cargo clippy --all-targets` with
warnings denied. Socket tests ran outside the coding agent's enclosing sandbox.
The original experiment receipt remains a historical record of its run.

## Transport finding

At libkrun `6151710`, `src/devices/src/virtio/vsock/unix_proxy/unix.rs` maps a
zero-byte host `recv()` to `RecvPkt::Close`. In `unix_proxy/mod.rs`, `recv_pkt()`
then sets `ProxyStatus::Closed` without emitting the corresponding guest-facing
shutdown packet. The observed guest read does not return EOF before its timeout.
The local [patch](patches/libkrun-vsock-host-half-close.patch) distinguishes EOF
from transport failure, emits `VSOCK_OP_SHUTDOWN` with SEND only, stops polling the
exhausted host input and retains the guest-to-host response direction. It is
tested with data pending across the credit window, EOF response, reconnect and
guest shutdown. This is a devcroft experiment patch, not an upstream-merged fix;
its hash and source revision are part of every new report.

Pins, enabled features, verified APIs and patch inventory live in
[upstream.json](fixtures/upstream.json). Code inspection confirms that macOS
`start_threaded()` enters the vCPU run loop during `build()`; `run()` is already
too late for applying host confinement.

The experimental code is Apache-2.0. libkrun and hvi are separate Apache-2.0
upstreams. The downloaded Linux kernel has its own GPL obligations and provenance;
this directory does not distribute it. The experiment's additional dependencies
are recorded in its own lockfile and are not part of devcroft's release graph.
