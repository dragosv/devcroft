# microVM experiments

This is the experimental implementation of `evaluate-microvm-runtimes`.
`libkrun/` and `hvi/` have their own Cargo workspaces and lockfiles. The main devcroft build and
published crate do not depend on it. No production VM dispatch exists yet.

The [native assessment](results/macos-arm64-evaluation-assessment-v4.json) records
the full 180-case macOS ARM64 comparison, clean reconstruction, final SSH fault
recovery and real CLI/TLS regressions against the five backend criteria. Both
adapters work on this host. Production adoption remains no-go under the current
evidence: hostile preparation confinement, production lifecycle/resource/credential
handling and remaining platform/fault qualification are still open. The libkrun recommendation is only
for the next scoped integration experiment, conditional on the recorded
author-boundary criterion and the owner's acceptance.

## hvi macOS ARM64 comparison

The isolated `hvi/` workspace now passes eight functional rows with the same
kernel, initramfs and disk images already used by libkrun: Stage A, keeper/SSH,
Alpine build/Redis, private adaptable base, Alpine editor, and Nix SSH, workload
and editor. The final reviewed patch run covers 25 clean/crash-recovery boots.
See [hvi confinement and validation](fixtures/hvi-macos-confinement.json) and
`results/hvi-macos-arm64-*.json` for actual events, hashes and raw log paths.

`python3 experiments/microvm/build_hvi.py` prepares the fixed upstream revision,
verifies the exact local patch/effective source diff, builds the separately
locked helper and signs it on macOS. Run any common `*_smoke.py` harness with
`--helper experiments/microvm/hvi/target/debug/devcroft-hvi-spike`. No production
build depends on hvi. The helper uses hvi's native process-wide deny-default
Seatbelt after resource acquisition and before I/O/vCPU threads, with no host
filesystem shares or NIC. Actual retained disk descriptors are checked for
RO/RW modes and unexpected regular files are rejected. Negative probes run on
the first vcpu0 safepoint before its first guest entry; source ordering covers
the other vCPU threads. Preopened descriptors remain usable after confinement.

The pinned hvi API exposes only one fixed guest agent port. The recorded
[local patch](patches/hvi-vsock-fixed-ports.patch) adds preopened Unix listeners
for host-selected guest ports. This helper configures only control 1024,
keeper 1025 and SSH 1026. Each connection validates the guest source port before
handshake, credit, data or teardown, and all outgoing control/data packets retain
the selected port. The extension changes no Seatbelt grants and uses no extra
sidecar or guest multiplexer. Native vsock tests include cross-port injection.

The earlier SSH calibration failed its console-capture gate because hvi writes
guest console output to stdout while libkrun uses stderr. The common harness now
checks both bounded streams, records the stream, and attributes the adapter from
actual pinned helper provenance. The failure and passing reruns remain recorded.
An initial double-lock integration failure was also retained in the raw runs;
the wrapper now leaves disk locking to hvi's own shared-RO/exclusive-RW locks.

hvi's `File::sync_data` flush uses `F_FULLFSYNC` in the recorded Rust 1.95 macOS
implementation; this is source-level alignment, not a power-loss test or matched
performance result. The pinned API exposes no RNG or balloon device, so memory
recovery parity remains open. Native Linux/x86-64, trusted Remote-SSH, actual agent
completion, fault coverage and density remain separate unqualified rows.

[Brig reference](fixtures/brig-reference.json) records the owner-requested design
comparison. Its 4 GiB agent allocations and 1.5x relative regression gate do not
provide our absolute E5 limits. The owner subsequently accepted the devcroft
limits in [the frozen E5 record](fixtures/no-go-thresholds.json) before density
collection: 4096 MiB after build/60 s idle, 600 s cold preparation with downloaded
inputs, 5 s session start in a prepared VM, and 8 GiB physical storage per environment.

## Linux ARM64 nested diagnostics

`libkrun/src/linux.rs` installs a fail-closed host boundary while `/proc/self/task`
contains exactly one thread, before payload/device constructors and `build()`.
It requires Landlock ABI 6 or newer, grants immutable inputs read-only and only
the owned instance/device paths needed for boot, and handles TCP/IPC scopes.
The compiled rust-vmm `seccompiler` policy has 41 syscall names selected from
post-filter traces of successful Stage A, keeper and real workload runs. Every
miss kills the process. Socket family, destination-less sends, thread-only clone
flags and the thread-name `prctl` are constrained. Host `connect`, addressed sends,
exec, fork-like clone, ptrace and io_uring are unavailable. There is no log/bypass
mode. The helper refuses unsupported architectures rather than inventing a list.

Seven separate fixed probes die with SIGSYS; filesystem read/write denials and
working input/private-file/Unix/thread controls pass. The probes fork only in the
single-threaded **selftest before restriction**, reuse already-hashed inputs,
close ambient descriptors and apply the real policy in each child. VMM runs never
fork after restriction. Stage A additionally reads every helper thread's status
from the controller at READY and requires seccomp mode 2 plus `NoNewPrivs=1`.

[Measured inventory](fixtures/libkrun-linux-aarch64-syscalls.json) includes archive
hashes, filtered roots/threads and the final-policy reruns. The labelled
`results/libkrun-linux-arm64-nested-*.json` records common-artifact Stage A,
Alpine and Nix builds/Redis/restart/SIGKILL. The outer machine is this M4 Mac,
Lima/VZ 4 vCPUs/4 GiB, with nested KVM API 12 and Linux Landlock ABI 7. This does
**not** qualify a native Linux host or x86-64, and traced timings are not used for
the native comparison. The [matrix](fixtures/hardware-matrix.json) names missing
hardware rows explicitly.

Build with `python3 experiments/microvm/build_linux_helper.py`; the static Linux
ARM64 target uses the recorded Zig linker and its own lockfile. The dedicated
mount-free Lima instance uses [nested-builder.yaml](kernel/nested-builder.yaml).
`nested_probe.py <row> --out <path> --trace` copies inputs and keeps full raw logs;
it stops the builder in `finally` unless `--keep-running` is explicitly selected.
An exclusive controller lock prevents concurrent uploads from truncating shared
inputs. Tests retain calibration failures: missing Linux `/private/tmp`, BusyBox
netcat without Unix `-U`, and redundant large-image hashing under strace. The
final runner uses `/tmp`, OpenBSD netcat and prevalidated single-threaded probe children.

The common control-frame reader now bounds the whole frame deadline, preserves
pipelined frames and peeks chunks rather than issuing one syscall per byte.
Four framing tests, the 26-test host suite and two-boot native macOS regressions
for both adapters pass. Guest policy, kernel and runtime images remain unchanged.

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
and `keeper.sock` → vsock 1025 (keeper framing, relayed to its guest Unix socket),
plus `ssh.sock` → vsock 1026 for SSH/SFTP. All remain inside the private instance directory and exact Seatbelt socket
grants. The default console now creates a real hvc0 terminal; guest logs flow
through the helper's bounded stderr capture. libkrun logs repeated ENOTCONN
shutdown warnings at connection teardown; functional checks pass, but that noise
remains recorded in the raw evidence.

The [SSH/SFTP run](results/libkrun-macos-arm64-ssh.json) reuses the actual
`src/ssh/server.rs` and `src/ssh/sftp.rs`, with russh 0.62.5 and russh-sftp 2.4.0.
Two VMs run simultaneously. Real system OpenSSH clients exercise exec, separated
stdout/stderr, nonzero exit, 128 KiB binary stdin/EOF, an interactive shell, PTY
resize via local SIGWINCH and Ctrl-C. SFTP uploads, renames, downloads and removes
128 KiB, while nono rejects private-file reads and runtime-data writes. SSH
commands also cannot read the private raw bootstrap disk. Both VMs forward their
own HTTP identity fixture on `127.0.0.1:8080`; a live destination on 8081 is
denied. Wrong and sibling client keys fail authentication, and pinned host keys
refuse a sibling endpoint. One VM cold-restarts with its private host key while
the sibling remains available. Owned helpers are reaped and their endpoints
removed after the run. The shared runtime disk remains unchanged.

Real clients exposed an existing SSH authentication bug: comparing `PublicKey`
also compared the local file comment, which the wire identity omits. The server
now compares cryptographic key data; a regression test covers commented keys
without changing rejection of other keys.

The [real workload run](results/libkrun-macos-arm64-workload.json) adds an
immutable Alpine GCC/make/Redis filesystem and a separate private ext4 workspace.
Both VMs compile upstream xxHash 0.8.3, run its `test_sanity`, verify 2,048 small
files against SHA-256 inputs, hash those files with the compiled executable and
export the ARM64 ELF. Both run real Redis 8.4.2 on guest port 6379, with AOF and
`appendfsync always`, also exercised over SSH forwarding.

One VM clean-restarts, then its helper receives SIGKILL during a progressing
Redis SET workload. After both restarts, the previously built binaries and
small-file checksums match without rebuilding; all 32 separately acknowledged
durable keys match. The sibling Redis keeps its run ID and data. The crash boot
logs ext4 journal recovery. The toolchain image remains byte-identical and owned
helpers/endpoints are cleaned up. This tests helper termination while the host
stays running, not host power loss or every storage fault.

Redis exposed missing madvise syscalls and procfs page monitoring in the minimal
kernel. It now enables `ADVISE_SYSCALLS` and `PROC_PAGE_MONITOR`; the workload
policy grants read access to the VM's own procfs, never the host's. Redis's ARM64
copy-on-write safety check explicitly passes without an `ignore-warnings`
bypass. Other system checks retain a slow-clocksource warning/skips. Performance
is not qualified.

The [guest-served editor run](results/libkrun-macos-arm64-editor.json) passes with
code-server 4.145.2 and real headless Chrome in a fresh profile, reached only
through a pinned-key SSH tunnel over vsock. The browser logs in, opens `main.c`
from Explorer, saves new C source, explicitly trusts this disposable fixture
when the terminal requests it, then compiles and runs the program through the
actual xterm UI. The probe uses an xterm paste event without the system clipboard.
Independent SSH reads verify exact source, output and an exported AArch64 ELF.
After a clean cold restart, those files match before reopening the browser;
the second browser sees the persisted source and the binary hash is unchanged.
Both boots pass bidirectional WebSocket activity, shutdown and endpoint cleanup;
the read-only editor image hash is unchanged. Screenshots and raw logs are linked
from the result. This is the Alpine/npm row with 2 vCPUs and 1 GiB, not the Nix
closure or the separately labelled trusted Remote-SSH row.

The browser retains duplicate TypeScript extension registration, missing-asset
and blocked-gallery errors. Installing extensions or using an agent/API is not
qualified. Calibration failures are retained, including an empty runtime APK
closure caught by a real boot and a UI probe that initially selected Chat's
Monaco instead of the file editor. The final probe checks the file editor and
terminal focus, then gates success on independently verified guest files.

The separately declared Nix closure now passes the same functional rows on
libkrun/macOS ARM64. Its locked flake contains GCC 15.3.0, Redis 8.8.1 and
code-server 4.115.0. All 146 NARs were verified and materialized in an empty
store without network before exporting the read-only runtime filesystem:

- [Nix workload/crash evidence](results/libkrun-macos-arm64-nix-workload.json):
  two real guest builds, upstream sanity tests, 2,048 small files, two Redis
  services on port 6379, clean restart and helper SIGKILL during 28,920 observed
  SET calls. All 32 acknowledged keys and existing build outputs recover; the
  sibling keeps its run ID/data. Exported AArch64 executables name the native
  `/nix/store` glibc loader.
- [Nix editor evidence](results/libkrun-macos-arm64-nix-editor.json): real Chrome
  edit/save, compile/run through the integrated terminal, independently checked
  source/output/Nix-linked ELF, and cold-restart persistence before reopening.
  This uses the Nix-packaged editor, not the Alpine/npm image.
- [Nix SSH/keeper evidence](results/libkrun-macos-arm64-nix-ssh.json): two VMs,
  OpenSSH/SFTP/interactive PTY/resize/Ctrl-C, real keeper protocol/signal probes,
  allowed/denied access, forwarding, cross-instance key refusal and restart.

All three runs leave the immutable image unchanged, reap their owned helpers
and remove endpoints. They retain application diagnostics, including missing
assets, generic browser errors and inability to retrieve a MAC address from the
NIC-less guest. The bootstrap's first attempt failed because BusyBox `cp` lacks
the official installer's `--preserve` option; GNU coreutils was added to the
builder only. Raw calibration and successful build logs remain under `runs/`.
Versions and image sizes differ deliberately from the earlier Alpine fixtures;
these are not matched performance comparisons. The Nix source pin reuses only
xxHash and small-file inputs from `workload-source.json`; that file's service
description applies to Alpine, while this row's Redis comes from the flake.

The [adaptable-base run](results/libkrun-macos-arm64-adaptable.json) adds a private
writable rootfs and a bounded administrative setup phase. Five clean boots across
three instances demonstrate the actual transition: GCC works on the baseline,
the zlib program fails for missing `zlib.h`, fixed offline APKs install through
PID 1, then the real keeper compiles and runs a zlib compression round trip.
After cold restart the existing executable runs without rebuilding. A new
instance starts from the empty seed and reconstructs the same header, library,
source and executable SHA-256 hashes from the recipe, without copying the first
instance's modified disk. A third fresh baseline fails the missing-dependency
build when setup is omitted. Empty install inputs, missing hashes, foreign URLs,
path escapes, corrupt APKs and symlink inputs fail host recipe validation.

This still uses four disks. The private ext4 disk contains disjoint `rootfs`
and `workspace` subtrees; only the latter is bind-mounted at `/workspace` and
granted read-write access. Guest nono grants read-only access to the rootfs.
Normal sessions cannot append to the installed header or remove the package,
although the filesystem itself remains writable for administrative setup.
The immutable input image remains unchanged. All owned helpers and endpoints
are cleaned up. A mismatched seed is rejected before instance/key/helper creation.

The setup guest withholds keeper/SSH until `SETUP CLOSE`. Its control parser on
the existing private `control.sock` accepts only the compiled-in recipe hash
and fixed apply/close operations; no paths, packages or shell strings come from
the wire. APK input hashes/signatures are checked, install uses `--no-network`
and `--no-scripts`, and there is no guest NIC/TSI. Wrong hashes, path injection,
oversized frames, duplicate application and reopening after close are refused.
The window cannot reopen within that boot. Administrative child processes and
baseline copying have deadlines; control descriptors have close-on-exec.
The exact recipe and separate runtime/setup initramfs identities are retained.

This is one trusted fixture recipe using Alpine APK, not the production package
mechanism or a general privileged shell/API. Setup interruption/recovery, other
adapters/platforms and full fleet/security qualification remain open. Omitting
the whole setup is the runtime negative control; empty/floating recipe inputs
are separately rejected before guest creation.

This does not qualify a complete backend. The original keeper/SSH fixture has
an in-memory workspace; the workload fixture uses ext4. Both are Alpine-based
fixtures; the Nix closure is a separate image/guest row. Complete project policy,
the separately labelled trusted Remote-SSH path,
production credential delivery, remaining failure modes and density tests remain
later stages. The ARM64
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
rtk proxy python3 experiments/microvm/ssh_smoke.py
```

The SSH harness generates fresh Ed25519 client and host keys for each instance,
never using the operator's SSH agent or user config. It pins the known host key
before connecting. The private state disk carries a bounded JSON bootstrap at
offset 8192 (`DCSHKEY1`, big-endian u32 length, at most 16 KiB), separately from
the raw-state counter at offset 4096. Keys are loaded and the disk fd closed
before nono and before threads exist; key material is absent from the shared
initramfs and session environment. These are disposable test identities in a
mode-0700 instance directory, with mode-0600 private files; disks and test keys
are retained for diagnosis. This is an experimental fixture, not the production
credential provisioning/rotation design.

Only guest loopback is enabled. There is still no NIC or TSI. The SSH fixture's
nono policy permits TCP connect to 8080; a live 8081 listener supplies the denial
control. The tiny HTTP identity server is a forwarding fixture, not evidence of
the later Redis/Postgres, editor or application-build workloads.

To reproduce the real build/persistence/crash demonstration after the kernel and
helper builds above:

```sh
rtk proxy python3 experiments/microvm/build_workload.py
rtk proxy python3 experiments/microvm/build_keeper_guest.py --workload
rtk proxy python3 experiments/microvm/workload_smoke.py
```

`build_workload.py` uses the same mount-free Linux builder. Its APK closure is
fixed by committed URLs and SHA-256 identities in
`fixtures/workload-packages-aarch64.json`. Fetching precedes explicitly offline
APK installation into the image. `--freeze-inputs` is an explicit repin, never
the normal reproduction path. Input APKs are retained in `packages.tar`. The
source archive is separately pinned in `fixtures/workload-source.json`.
Application inputs and builder tools are recorded; bit-identical filesystem
output is not claimed.

The workload uses four raw disks: runtime probe, private bootstrap/counter,
read-only toolchain and private ext4 data. Both writable disks have exclusive
locks and full sync mode. Private data cannot alias bootstrap/immutable files
or escape its instance directory. Each VM uses 2 vCPUs and 512 MiB. SHA-256 is
optimized in the helper's dev profile so full verification of the 512 MiB image
stays practical; VMM code retains its diagnostic dev profile. Validation and
boot deadlines are bounded at 90 and 120 seconds. This is functional evidence,
not a density or latency benchmark.

To prepare the separate guest-served code-server fixture (after the workload
build), with Node 20+ and Google Chrome installed on the host:

```sh
rtk proxy python3 experiments/microvm/build_editor.py
rtk proxy env PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm ci --prefix experiments/microvm/editor-browser --ignore-scripts --no-audit --no-fund
rtk proxy python3 experiments/microvm/editor_smoke.py
```

The editor builder starts from the hash-pinned workload image and adds a fixed
Alpine runtime APK closure plus code-server 4.145.2 from its pinned npm archive.
It records two local upstream corrections: nested installs use `npm ci`, and the
published shrinkwrap's missing `tslib` peer is fixed to an explicit version and
integrity. Native modules build with LTO disabled, retaining fortify. After
fetching, the builder extracts the source again and reinstalls from the npm
cache in a new network namespace with no network interfaces enabled. It checks
the lockfile hashes and runs Node/code-server inside the exported filesystem
with chroot. Runtime APKs, npm cache, source/patch identities and installed
builder versions are retained; builder tools are not yet a fully hash-pinned
closure, so this does not establish fleet eligibility or bit-identical images.
`--freeze-inputs` explicitly repins runtime APKs.

The editor harness uses 2 vCPUs and 1 GiB for one VM, a read-only editor image
and a private ext4 workspace. Guest nono permits TCP bind/connect on 9090 only
when that editor fixture exists; the host reaches it through SSH over vsock.
The kernel includes inotify. The test-only Playwright dependency stays under
`editor-browser/`; Chrome uses a fresh temporary profile and no operator login.
The probe drives the actual workbench and terminal UI, captures WebSocket frame
counts/screenshots, verifies written files independently through SSH, then
checks persistence before reopening the editor after a cold restart.

To materialize the separately declared Nix closure on Linux and package its
guest (requires the pinned xxHash archive fetched by `build_workload.py`):

```sh
rtk proxy python3 experiments/microvm/build_nix.py
rtk proxy python3 experiments/microvm/build_keeper_guest.py --nix
rtk proxy python3 experiments/microvm/ssh_smoke.py --guest experiments/microvm/artifacts/stage-b-nix/guest.json --workload experiments/microvm/artifacts/nix/toolchain.img --seed experiments/microvm/artifacts/nix/data.seed.img
rtk proxy python3 experiments/microvm/workload_smoke.py --guest experiments/microvm/artifacts/stage-b-nix/guest.json --workload experiments/microvm/artifacts/nix/toolchain.img --seed experiments/microvm/artifacts/nix/data.seed.img
rtk proxy python3 experiments/microvm/editor_smoke.py --guest experiments/microvm/artifacts/stage-b-nix/guest.json --editor experiments/microvm/artifacts/nix/toolchain.img --seed experiments/microvm/artifacts/nix/data.seed.img
```

`nix/flake.nix` and `nix/flake.lock` declare the exact nixpkgs revision and NAR
identity; every evaluation/build refuses lockfile updates. Nix 2.34.9's official
ARM64 Linux bootstrap archive is separately SHA-256 pinned. The mount-free Lima
builder installs it in its own Linux filesystem; macOS Nix is not used. The
builder runs only this trusted fixture recipe, with Nix build sandboxing disabled
inside the dedicated VM; project compilation runs later under guest nono.

The builder verifies source-store content and trust, exports the runtime closure
to a private NAR cache, then creates an empty destination store and imports it
in a network namespace without connectivity. Destination content hashes must
match every source NAR. Only that private import disables signature checks to
accept the locally built, unsigned top-level environment; the receipt records
this explicitly. It does not disable source trust verification or NAR content
verification. `cache.tar` and `closure.json` are retained with the image and
private ext4 seed. This verifies offline materialization from the captured
closure cache, not rebuilding GCC or code-server from source.

The Nix guest adds `/nix` → `/toolchain/nix`, preserving the binaries' native
store references. `/usr` is a link to the declared environment; it does not
import Alpine toolchain binaries. GCC, Redis and the Nix-packaged code-server
therefore come from the flake. Runtime and editor probes record their actual
versions and image hashes separately from the Alpine rows. Builder tools and
filesystem metadata are recorded, not claimed bit-identical; complete fleet
qualification and the x86-64/hvi rows remain separate work.

To reproduce the adaptable-base row after building the immutable workload input:

```sh
rtk proxy python3 experiments/microvm/build_adaptable.py
rtk proxy python3 experiments/microvm/build_keeper_guest.py --adaptable
rtk proxy python3 experiments/microvm/build_keeper_guest.py --adaptable --setup-window
rtk proxy python3 experiments/microvm/adaptable_smoke.py
```

`fixtures/adaptable-recipe-aarch64.json` fixes the baseline image identity,
native source and all package-manager/dependency APK URLs and SHA-256 hashes.
`--freeze-inputs` explicitly resolves new pins; normal reproduction never
repins them. `packages.tar` retains the archives. The Linux builder adds only
the package manager to the immutable baseline; the native development package
remains uninstalled until guest setup. The seed is an empty 768 MiB ext4 disk;
instances use 2 vCPUs and 512 MiB. The host checks recipe/image/seed consistency
before starting any guest. The setup protocol is an experimental PID 1 contract,
not a change to the keeper or the public devcroft CLI.

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
rtk proxy python3 -m unittest discover -s experiments/microvm -p test_adaptable_recipe.py -v
rtk proxy cargo test --manifest-path experiments/microvm/guest-keeper/Cargo.toml --locked
```

The fifteen interface tests exercise the compiled helper: patch provenance, input corruption, input
aliasing, writable-scope overlap, unknown network fields, private directory
permissions, resource bounds, oversized records and distinction between input
validation and runtime qualification, occupied endpoints/dangling symlinks,
paired workload/data disks, private-data scope/aliasing, and exclusive locks.
The interface tests require a built helper and do not silently skip. The guest
workspace runs 28 tests: the existing keeper/SSH tests, the authentication
regression and bounded bootstrap/framing/truncation tests. Confinement probes run
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

The first full density attempt (`runs/20261110-native-density-v1`) is excluded
from comparison: relative observer waits accumulated macOS timer coalescing,
leaving 233–234 samples per 60-second idle window. Thresholds are unchanged.
Absolute monotonic deadlines passed four complete 60-second windows on both
adapters with 240 samples each. Raw clocks and interrupted samples are retained;
see `results/macos-arm64-density-cadence-calibration.json`.

### Real agent startup and guest TLS

The owner selected both Codex and Claude Code. `agents/flake.nix` and its lock
reuse the fixed nixpkgs revision in a separate closure: Codex 0.147.0 and Claude
Code 2.1.228, 129 NAR paths, verified and reconstructed in an empty offline
store. `build_agents.py` leaves the baseline Nix image unchanged and removes only
verified reconstruction intermediates from the dedicated builder. Build/closure
receipts are in `fixtures/agents-aarch64-{build,closure}.json`.

`build_keeper_guest.py --agents` packages a separate guest. It gives Bun read-only
access to the exact guest `/dev/urandom` device, enables only loopback TCP 443,
and mounts immutable hosts aliases for the two API names before the keeper
applies nono. An explicitly opted-in SSH reverse listener maps this port to one
fixed vendor:443 through the trusted OpenSSH controller. The three VMM vsock
ports, no-NIC configuration and host Seatbelt boundary are unchanged. Ordinary
keepers pass `None` and refuse reverse forwarding, covered by a protocol test.

`agent_probe.py` measures five fresh SSH/CLI sessions in one prepared 2-vCPU,
1024-MiB VM per adapter. Codex `turn.started` and Claude `system/init` events are
timestamped before the host reads the actual vendor's dummy-key 401. All twenty
startup samples passed the original five-second limit. These are initialization
and transport checks, not completed authenticated agent tasks or concurrent
agent-density measurements. Evidence: `results/{libkrun,hvi}-macos-arm64-agent-start.json`;
the frozen method is `fixtures/agent-start-protocol.json`.

TLS is verified inside the guest against the Nix CA bundle. Both real CLIs also
reach a live local endpoint using the correct vendor SNI and a self-signed
certificate. Codex sends an unknown-CA alert. Claude finishes the cryptographic
handshake but closes without application bytes and reconnects; the controller
stops this negative probe after five seconds. Adding only the public test CA to
that child process makes the exact same endpoint receive application bytes,
providing the positive control. No real credentials are used. Wrong/dynamic/
wildcard reverse ports are refused and OpenSSH cancellation removes the guest
listener. Earlier failed/ineffective calibrations remain labelled in
`results/macos-arm64-agent-calibrations.json`.

### Native observer reliability and capacity controls

`measure_macos.c watch <controller-pid>` now owns its 250-ms absolute Mach
deadlines, runs in an independent session and writes complete native snapshots.
Only the small observer requests user-initiated QoS; VMM priorities are unchanged.
Python records independent phase markers and supervises instance liveness. Raw
monotonic/wall clocks, per-PID usage-call time, descendant-enumeration time and
output-flush time remain available. Seven snapshots persisted while the owned
controller process group was stopped for two seconds; this is not a host-suspend
qualification. `native_observer_probe.py` reproduces that control.

The v2 collection is retained with 54 recorded cases, including cadence failures
at four VMs. The first native eight-VM calibration also hit the host storage
reserve. Only regenerable Cargo caches inside experiment targets were removed;
four native/guest executable hashes were unchanged. After recovering about
6 GiB, both eight-VM fixtures passed their build and two complete idle windows
with 240 samples each. Evidence: `results/macos-arm64-native-sampler-calibration.json`
and `macos-arm64-compilation-cache-cleanup.json`. A fresh comparison uses the
revised collector on all configurations; earlier attempts stay excluded.

Before each cohort, headroom now covers the original 2-GiB host reserve plus
configured guest RAM and the private disks' logical capacities. Density and
materialization runners share an exclusive builder lease. This changes the
measurement safeguards, not the owner's 4096-MiB/600-s/5-s/8-GiB limits.

`summarize_density.py` preserves missing and failed rows, reports cohort
minimum/median/maximum, and never treats the SSH marker as actual agent startup.
`storage_overlap.py` separately accounts for 111 common NAR paths in the baseline
and agent closures without assuming APFS deduplication between raw images. The
Linux hvi ordering audit remains a source finding, not a runtime exploit claim:
`results/hvi-linux-ordering-audit.json`.

`cold_prepare.py` prepares a separate agent image from downloaded immutable
inputs in a fresh, network-isolated builder namespace and verifies it in a real
guest. It refuses to run during density collection or calibration. Its frozen
scope is `fixtures/cold-preparation-protocol.json`; cold preparation has not
passed until its timed clean verification result exists.

`finish_native.py --density-run runs/<collector> --out runs/<continuation>`
waits for that collector's final report and exclusive lease release, then
publishes the density summary, runs clean preparation, and exercises SSH-client
loss, helper pause/resume, keeper death and helper termination for each adapter.
The fault probes retain a live sibling and verify existing builds/acknowledged
Redis keys after recovery. Session cleanup checks the guest PID's start time;
a failed SSH connection is never counted as a reaped process. Physical host
suspend and PID 1 death stay explicitly not-run. Independent phase failures
remain visible and do not prevent collecting the other adapter's evidence.
The continuation's `report.json` identifies its current phase; a waiting/running
controller is not a completed evaluation or production approval.

The complete native comparison is now retained in
`results/macos-arm64-density.json`: all 180 Alpine/Nix launch/build/idle cases
passed, and every measured runtime-memory/storage row meets the original
limits. Clean offline agent-closure preparation took 68.24 seconds, or 117.69
seconds including clean guest verification, against the accepted 600-second
limit (`results/macos-arm64-cold-preparation.json`). These measurements used the
original guest artifacts; later guest changes do not inherit the full matrix.

The first fault runs found the same SSH lifecycle failure on both adapters:
after abrupt client transport loss, the guest command stayed alive. The failed
`results/{libkrun,hvi}-macos-arm64-faults.json` records are retained. Helper
pause/resume, keeper death/cold recovery and helper SIGTERM/cold recovery passed,
including existing builds, 32 acknowledged Redis keys, live sibling identity
and helper/endpoint cleanup. That finding prompted the follow-up below.

The follow-up keeper fixes SSH loss by dropping a per-session cancellation guard
on whole-connection loss as well as channel close. The worker owning the child
sends TERM, retains the unreaped group leader through the two-second grace
period, then sends KILL before reaping; delayed cleanup never signals a PID
after it was reaped. Stdin EOF remains a half-close. Both final native fault
rows pass all four scenarios; initial failed reports remain unchanged. Their
follow-ups are `results/{libkrun,hvi}-macos-arm64-faults-ssh-fix.json`.

Local validation passes 442 library tests and warnings-denied all-target Clippy.
New tests exercise pipe/PTY raw transport loss, pipe/PTY channel close with TERM
ignored, a fresh session on the surviving connection, and 128-KiB buffered output
before exit status after stdin EOF. A separate native macOS direct-russh-client
burst echo diagnostic timed out on both the original and corrected servers; it
remains an open duplex/backpressure finding, rather than being counted as a new
regression or a passed Linux guest fixture. Raw before/after logs are retained
under `runs/20261111-ssh-lifecycle-validation`.

`build_keeper_guest.py --out <directory>` and `fault_probe.py --guest <receipt>`
allow validating new guests without overwriting the original density artifacts.
The 180-case comparison and cold-preparation timing therefore continue to
identify their original binary hashes; they do not silently qualify a later
keeper build.

`storage_infrastructure.py` records global preparation/compiler/evidence costs
separately from deployed VM storage. The stopped kernel/materializer builder's
disk currently has 13.49 GiB allocated. Charging that builder to one environment
already exceeds the original 8-GiB limit before runtime or downloads are added.
It also contains kernel-building and multiple-fixture infrastructure; a minimal
per-project materializer has not been measured. This prevents treating the
successful ~2.05-GiB deployed-runtime subtotal as a complete E5 storage pass.

Final post-fix regressions retain fresh OpenSSH/keeper/SFTP/PTY/forwarding runs
and five Codex/Claude sessions each on both adapters. All twenty initialization
samples are below 1.52 seconds, with vendor TLS/401 and live certificate controls:
`results/{libkrun,hvi}-macos-arm64-agent-start-ssh-fix.json`. These use the final
keeper hash, while the original density/cold-preparation receipts retain theirs.

TLS calibration found that a newly generated certificate's default `notBefore`
could be later than the guest's wall clock. The local test now records both
clocks, refuses skew above one hour, explicitly covers both in its short validity
interval and independently verifies hostname, server purpose and dates with the
public test CA at both clocks. It requires OpenSSL supporting explicit validity
dates and records its actual path/version. Expiry alerts no longer count as a
CA-rejection witness. The real CLIs keep TLS validation enabled; neither clock
nor vendor trust is changed. Guest clock synchronization remains a product
qualification item, especially for suspend/resume.

### Controller separation and private preparation follow-up

The current harness stores client keys, host pins, launch records and SSH master
sockets in a separate short mode-0700 controller directory. The VMM runtime
directory contains disks and the three permitted endpoints. Launch validation
requires a private, singly linked record at its absolute canonical path outside
the writable instance subtree. Symlink/hardlink and public-permission variants
are rejected before boot. Future density accounting includes both directories.

`controller_probe.py` passes on both native macOS ARM64 adapters: six protected
targets refuse read/write access, a live controller socket refuses a new
connection after confinement, and allowed runtime/file/thread/Unix controls
continue to work. Two real builds and Redis services survive a cold restart.
The complete OpenSSH and actual CLI/TLS regressions also pass with the new
layout. Evidence: `results/{libkrun,hvi}-macos-arm64-controller-*.json` and
`{libkrun,hvi}-macos-arm64-controller-separation.json`.

The fresh private materializer now passes independently of the original
development builder: **186.38 seconds** including clean guest verification and
**6.59 GiB** retained for one complete environment. See
`results/macos-arm64-minimal-private-preparation.json`. The total includes the
entire stopped builder namespace, downloaded base/NAR/bootstrap/APK inputs,
exported images and receipts/logs/sources, common kernel/helper/guest inputs and
private guest/controller files. It does not amortize the builder across an
invented fleet size. The original 13.49-GiB observation remains retained.

`prepare_minimal_inputs.py` publishes a fixed trusted input bundle with receipts.
`minimal_prepare.py` starts a fresh builder without host filesystem shares with its own user and Nix
store, installs fixed tools and reconstructs without external interfaces inside
the materialization namespace. It re-evaluates the locked definition and checks
all 129 NAR hashes against the publication. A 512-MiB compressed-cache tmpfs and
2-GiB destination-store tmpfs fit within its declared 4-GiB guest allocation;
guest memory is captured before unmount. The builder stops before verification
in the confined runtime VM. Verification also compiles/tests upstream xxHash,
checks 2,048 files, writes/verifies 32 Redis keys and exercises both real CLIs.

This is downloaded-input closure materialization from a trusted publication,
not a full source rebuild or independently re-established vendor-signature
trust. The temporary Lima builder runs a fixed trusted recipe; production
confinement for hostile preparation code remains separate. Original density
measurements retain their original helper/guest hashes and do not qualify these
later artifacts automatically.

Retention evidence is in `results/macos-arm64-preparation-retention.json`.
Exported duplicates and unreferenced build dependencies were reclaimed only
after protecting the fixed closures/source/Nix roots. Remote traces were
removed only after comparing each file with the host archive. Guest fstrim did
not substantially reduce physical host allocation, which remains recorded.
Regenerable Cargo intermediates were then reclaimed under build-directory locks;
the five protected runtime executable hashes were unchanged. Runtime images and
all host diagnostic archives remain retained.

The revised assessment is `results/macos-arm64-evaluation-assessment-v2.json`.
It records these two closed experimental gaps while preserving the original
assessment and its failed full-builder storage observation. Production adoption
remains unapproved; trusted preparation, missing native platforms and the
remaining SSH/fault controls still have explicit limits.


### Consumed-input SSH window prototype

Both native macOS ARM64 adapters pass binary OpenSSH echo at 128 KiB and
16 MiB, with exact hashes and nonzero exit status, using the same fixed guest.
A nonreading command throttles input while another channel on the same SSH
master remains usable. Killing the master/proxy transport reaps the original
command identity; a new connection works and owned helpers/endpoints are
removed. Evidence: `results/{libkrun,hvi}-macos-arm64-ssh-flow-control.json`.
These are functional samples, not agent-start or density measurements.

The earlier bounded stdin queue rejected valid fast input. The replacement
returns receive-window credit only after writing to the child's stdin. Tiny
frames coalesce into bounded chunks. A channel write half respects the remote
stdout window, and connection/channel loss interrupts that wait and releases
output pumps. Control credits bypass the application-output backlog.

This is an isolated fork in `guest-keeper/src/ssh_server.rs`, derived from the
recorded production server, plus vendored russh 0.62.5. The root dependency and
production SSH source are unchanged. `fixtures/ssh-consumption-windows.json`
records the upstream crate checksum, 84 vendored input hashes and reconstructible
patches. `freeze_ssh_sources.py` deliberately refreshes these pins; guest builds
verify them before compilation. The extra input worker per active session and
patch maintenance need qualification before any production port.

The trusted host ProxyCommand now moves both socket directions independently;
the previous single `nc` proxy stalled in the native large-duplex calibration.
Readers drain the multiplex master's stdio to actual EOF rather than stopping
at the mux client's earlier exit notification. No real credentials are used.

Failures remain in `results/macos-arm64-ssh-flow-calibrations.json`. The pinned
russh library passes 154 unit tests, the keeper passes warnings-denied all-target
Clippy, and the explicit native OpenSSH 16-MiB test passes. At that checkpoint,
the direct-russh unpaced multiplex diagnostic still intermittently stalled;
the full native keeper suite was not claimed clean. A common macOS PTY test also
returned empty output once. Upstream strict library Clippy reports two unchanged
saturating-arithmetic style findings. The original 180-case density and private
preparation reports retain their older binary/source hashes.


### SSH duplex and queued-credit follow-up (2026-11-13)

The isolated russh fork now queues sealed output behind a cancellation-safe
partial-write cursor. Its client and server continue reading packets while the
socket is blocked. Application producers stop at a 256-KiB soft threshold;
control replies retain a bounded reserve, with an 8-MiB cap on queued buffer
capacities. Staged plaintext also counts toward application backpressure.
Cipher state, packet order, half-close and final disconnect output are retained.

Window adjustments now add only the newly received credit, capped by the
protocol window, preserving credit already reserved by messages awaiting the
session loop. Previously, replacing that credit with the absolute protocol
window could grant queued bytes twice and stall unrelated channels behind
pending data. The regression sends unpaced input directly through russh at
128 KiB and 16 MiB, without echo-based application pacing.

The follow-up passes all 36 keeper tests including native OpenSSH, all 157 russh
library tests, and five integration tests for backpressure, channel streams and
strict rekeying. The unpaced duplex and nonreading-stdin multiplex regressions
each pass ten consecutive runs. Keeper all-target Clippy passes with warnings
denied. Russh library Clippy passes with only the pre-existing
`manual_saturating_arithmetic` lint allowed; strict mode still reports its two
unchanged upstream findings. Evidence and source hashes are recorded in
`results/macos-arm64-ssh-wire-validation.json`; earlier failures remain retained.

Fresh native OpenSSH VM proofs use the separately built, pinned guest in
`artifacts/stage-b-nix-wire-v2`. Their results are
`results/{libkrun,hvi}-macos-arm64-ssh-wire-progress.json`. The probe now retains
its post-transport-loss process-reaping and new-connection checks in the result
row itself, alongside exact transfer hashes and owned-resource cleanup.
These close the direct-client diagnostic for this experimental fork. Production
porting, physical host suspend, PID 1 death, and a repeated density/preparation
matrix for this new binary remain outside this validation.

### Controlled PID 1 death and recovery (2026-11-13)

`build_keeper_guest.py --nix --pid1-exit-probe --out <separate-directory>` builds
an explicit fault fixture. Its compile-time-only branch accepts exactly
`TEST PID1 EXIT 86` on the administrative control channel and calls `_exit(86)`
without synchronizing or shutting down the guest first. Ordinary builds omit
the branch. The builder refuses to overwrite an existing guest with this fixture.

Run `fault_probe.py --adapter <libkrun|hvi> --guest <fault-receipt>
--control-guest <ordinary-receipt> --pid1-exit --out <new-run>` to add init death
to the existing four fault cases. The two receipts must match init source,
keeper, shell and runtime hashes, with distinct initramfs hashes and opposite
hook settings. The ordinary sibling must refuse the exact exit command and
remain responsive; the enabled fixture must refuse three non-fixed commands.

Both native macOS ARM64 adapters pass all five cases, across ten total boots.
The console records the actual `Attempted to kill init!` panic with exit code
`0x00005600`. Helpers terminate without a host kill; owned endpoints are removed,
existing compiled outputs and 32 acknowledged Redis keys recover without
rebuilding, and the live sibling retains its Redis identity/data. The paired
builds pass C warnings-as-errors; 38 Python tests cover opt-in guards and existing
framing, recipe and launch validation. Evidence:
`results/{libkrun,hvi}-macos-arm64-faults-pid1.json` and
`results/macos-arm64-pid1-fault-validation.json`.

The test also exposes a lifecycle distinction. At the pinned libkrun HVF backend,
PSCI reset and poweroff both become `Shutdown`; the PID 1 panic returns helper
status zero, just like clean shutdown. hvi exposes `SystemReset`, which this
helper reports as an error with status one. A controller cannot classify clean
shutdown from a zero helper status alone; it needs correlated shutdown intent,
acknowledgement and process identity. This remains a production integration gate.

The revised assessment is `results/macos-arm64-evaluation-assessment-v3.json`.
It closes the explicitly scoped duplex and PID 1 gaps while retaining previous
results and their exact artifact generations. Physical host suspend, other native
platforms and renewed density/preparation/CLI-start qualification for the latest
keeper remain open. Production adoption remains unapproved.

### Interrupted adaptable setup and atomic publication (2026-11-13)

The unfinished follow-up recovered from another agent reproduced two setup
failures on both native adapters. Interrupting baseline creation before its
publication left the private base unbootable. Interrupting the package install
before its recipe marker exposed a partial package/header/marker signature to
ordinary sessions. Those captures remain in
`results/{libkrun,hvi}-macos-arm64-setup-interruption-before.json`. Their harness
status records successful reproduction; `atomic: false` is the failed setup gate.

The corrected setup removes unpublished staging trees on startup, copies the
baseline into `rootfs.next`, installs the pinned offline recipe there, and syncs
its completed contents and marker. `renameat2(RENAME_EXCHANGE)` publishes the
prepared tree in one exchange. The old baseline remains under `rootfs.next`
until it can be discarded, including on the next boot after an interruption.

Build separate runtime, setup and fault guests with `python3 experiments/microvm/build_keeper_guest.py
--adaptable --out <directory>`, adding `--setup-window` for setup and both
`--setup-window --setup-fault-probe` for the disposable fault fixture. The fault
builder refuses to overwrite an existing guest. Its five one-time, synchronized
stalls are absent from ordinary runtime/setup binaries.

`python3 experiments/microvm/setup_interrupt_probe.py --mode atomic --runtime-guest <directory>
--setup-guest <directory> --fault-guest <directory> --out <new-run>` exercises
baseline copy, recipe staging, package installation, marker creation and the
directory exchange. Select hvi with
`--helper experiments/microvm/hvi/target/debug/devcroft-hvi-spike`.
The harness verifies fixture roles and pinned recipe/input/seed identities,
holds the exclusive native-runner lease, snapshots its sources, and removes
owned helpers, endpoints, private disks and controller directories.

Fresh reviewed runs pass all five interruptions on each adapter, across 26
boots. The observed signature is baseline at the first four points and applied
after the exchange. Four recovered dependency/build hashes match a fresh
uninterrupted reconstruction on each adapter. Earlier five-boot ordinary
adaptation/reconstruction regressions are also retained: their kernel, helper,
boot artifacts, inputs and seed match these builds exactly, so they are reused
without being counted as new repetitions. All 57 Python tests pass.

Evidence is in `results/{libkrun,hvi}-macos-arm64-setup-interruption-atomic.json`,
`results/{libkrun,hvi}-macos-arm64-adaptable-atomic.json`, and
`results/macos-arm64-setup-interruption-validation.json`. The five fixed stalls
sync before SIGKILL and the host page cache survives. They do not establish
arbitrary mid-write timing or physical host power-loss durability. Classification
checks the fixed package/header/marker signature and four final artifact hashes,
rather than whole-filesystem equivalence. Peak storage from the extra staging
copy still needs measurement. The updated assessment is
`results/macos-arm64-evaluation-assessment-v4.json`; production adoption remains
unapproved.
