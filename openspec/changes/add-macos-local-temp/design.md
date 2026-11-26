## Context

Measured 2026-11-24, macOS arm64, `flox-services-sample`'s environment, two
sandboxes A and C:

| Probe | Result |
|---|---|
| `TMPDIR`, `TMP`, `TEMP` in a session | unset |
| A writes `/tmp/x`, C reads it | reads it |
| the host's `/tmp/y` from A | read, and `rm` **deletes it** |
| `mktemp` (the runtime's) | `EPERM` under `/var/folders/.../T` |
| the same with `TMPDIR` set | works; Python's `tempfile` too |
| `/var/tmp` | `EPERM` |
| `ls /Users/<name>` | `EPERM` |
| `getpwuid(getuid())` | the host's name and `/Users/<name>` |
| `bind` of a unix socket in `/tmp`, `network.default = "deny"` | `EPERM` (Seatbelt's network axis, not the path) |

`src/policy/mod.rs` records why `/tmp` was granted: devenv computes
`/tmp/devenv-<hash>` itself and ignores `TMPDIR`, as process-compose's default
log and `mysql_install_db` do.

## Measurement gate

Measured on macOS 15.8.3 arm64 with the prototype in
`spikes/macos-local-temp/`. Its Seatbelt profile denies host temp data,
grants the scratch project and its temp directory, and permits metadata on
the `/private/tmp` parent as the backend does for ancestors of grants.
Without the library the host sentinel was refused through both temp spellings,
and a file under `TMPDIR` was writable.

- **devenv 2.4.0, sample lock 2.2.2.** The actual evaluated preamble uses
  `mkdir -p /tmp/devenv-<hash>` and `ln -snf` to link it from `.devenv/run`.
  GNU coreutils uses `mkdir`, `chdir` and `symlinkat`. The kernel can follow
  the resulting physical symlink in a single sandbox.
- **Python 3.13.13, Node 24.21.0 and Go 1.26.1.** File creation, reopening
  and removal worked via `TMPDIR`, `/tmp` and `/private/tmp`. Python uses
  `open`, `stat` and `unlink`. Node also reaches `mkdtemp` and
  `realpath$DARWIN_EXTSN` through libuv. The wrapper must copy the generated
  suffix back into Node's original template buffer. Go's `open` preserved
  mode 0600. Its Unix socket round trip exercised `bind` and `connect`.
- **process-compose 1.122.0.** With `TMPDIR=/tmp`, the default log used
  `open` and was written to the private directory. The supervised shell
  inherited the interposer and wrote a marker via a literal path.
- **PostgreSQL 18.6.** `initdb` completed with a literal data directory under
  the temp path in the probe profile, using `mkdir`, `stat`, `fopen` and
  `chmod` among other calls. Its bootstrap child needs the existing exec
  redirect's `/bin/sh` substitution to keep the temp library. The probe
  profile permits IPC, so this establishes path compatibility only.
- **Git 2.55.0 and Apple SSH.** Git created a repo at a literal path using
  `chdir`, `stat` and `mkdir`. Apple SSH refused a literal `-F` path with
  the library supplied and read the same config at its physical path.
  `mysql_install_db` was unavailable on the test host. **MariaDB 11.4.12**
  (2026-11-26, in a real nix sandbox, with the implementation): install,
  server and a client over `/tmp/my.sock` all work in the sandbox's
  directory. `mariadb-install-db` needs `--force` there, because `resolveip`
  cannot look the host name up inside a macOS sandbox.

### Two sandboxes sharing a devenv checkout fail

The prototype successfully ran the evaluated hook and wrote through
`.devenv/run` in sandbox A. Sandbox B used the same checkout and hook with
its own temp grant. Its hook also succeeded and replaced
`.devenv/run -> <temp-A>/devenv-<hash>` with
`.devenv/run -> <temp-B>/devenv-<hash>`. A's next open of
`.devenv/run/probe` returned `EPERM`. A and B each deny the other's physical
directory. The file path entering the interposer is project-relative, so the
D3 mapping leaves it to the kernel to follow the shared symlink.

Keeping the symlink's target literal would make the kernel follow host
`/tmp`, which is refused. Mapping the target at symlink creation stores a
physical target that belongs to one sandbox. This triggered the D3 stop
criterion. The owner approved the provider-level adjustment below.

### D3a. Per-sandbox devenv paths on macOS

Pass typed `devenv.dotfile` and `devenv.runtime` overrides to the three
devenv capture commands. The sandbox's home holds the dotfile directory
because devenv derives its persistent state directory from it. Private
scratch holds the runtime. The generated hook links `$DEVENV_DOTFILE/run` to
`$DEVENV_RUNTIME`, using a different link and target for each sandbox. Both
options were verified against the sample's locked input. Provisioning and
sessions receive these same paths. Linux and standalone provider captures
keep their usual devenv configuration.

Code that hardcodes the checkout's `.devenv/run` must use the provider's
variables on macOS. Document this migration explicitly. The checkout can
still contain its ordinary `.devenv/run` for devenv commands run on the host.
Verify the new paths with two sandboxes of the same checkout, with both
hook invocations completed before reopening the first sandbox's file.

### Cost

Each benchmark alternated the existing exec redirect and the exec redirect
plus the prototype. Tracing was off. A clean, offline
`cargo build` of `devbox-citytime-sample` took a median of 0.1990 s with
the existing library and 0.2015 s with the prototype across nine runs per
arm (+1.3%). The file loop alternated a workspace path and a temp path
for create, write, stat and unlink. Its 4,000 iterations took a median
of 0.3788 s and 0.3775 s across seven runs per arm (−0.3%). A separate
40,000-iteration run was 3.7653 s and 3.7992 s (+0.9%). This measures the
prototype's prefix checks and wrappers, before the full D3 call set and
lexical normalization.

### Current dependencies on the host temp grant

- The `devbox-citytime-sample`, `flox-clap-sample`, `nix-go-sample` and
  `nix-probe-sample` explicitly grant `/tmp`. The nix probe also puts Go's
  build temp directory and caches there. These manifests would still grant
  host temp under D2. The flox rustup sample sets `TMPDIR` to a directory
  under its project and creates it in the activation hook.
- devenv's preamble, process-compose logging and provider service
  initialization use literal paths as scratch. The devenv preamble also
  writes the shared project symlink, which makes the physical mapping
  observable across sessions of two sandboxes.
- `tests/common/mod.rs` and the devenv, devbox, provisioning and service
  tests place projects under `/tmp` to keep Unix socket paths short. They
  need access to the granted project subtree. The Unix-socket network probes
  explicitly arrange filesystem access to their socket directory. The
  devbox GCC build test explicitly grants `/tmp`, and the macOS policy
  golden records the baseline grant. No other use of host-shared temp data was
  found in the tracked tests.

## Decisions

### D1. Where
`/private/tmp/dc-<uid>/<8 hex of the state path>`, 0700, not under the project:
`.devcroft/<name>/` makes long paths, and a unix socket has 103 bytes. Created
host-side with every component opened `O_NOFOLLOW` and checked to be the
caller's, since `/private/tmp` is world-writable (`runtime::owned_dir`'s rule).
*Alternative:* a disk image attached at that path (`hdiutil`, no privilege). A
real mount, with a size limit, but still at one path of the host's namespace,
so it does not put anything at `/tmp`; slower to bring up. Not needed.

### D2. The grant moves, last
`baseline` grants that directory, and the host's `/tmp` no longer, on macOS. The
second half waits for D3: without it, devenv's preamble and every other literal
user would be refused. Implemented together, since D3 held. A manifest
that names `/tmp` gets the host's, and then literal paths are not redirected.

### D3. Literal paths, through the library (owner, 2026-11-24)
**The mapping.** `/tmp` and `/private/tmp`, and what is under them, map to the
directory. Not mapped: a path inside another grant (a project kept in
`/private/tmp`, the directory itself), and everything when the manifest grants
`/tmp`. The keeper passes the directory and those exceptions as it passes the
namespace, in variables it sets on its children only. The comparison is
lexical on a path with `.` and `..` resolved; a relative path is not mapped,
since a process whose working directory is there is already inside.

**The calls.** Those that take a path: `open`, `openat`, `creat`, the `stat`,
`access`, `chmod`, `chown`, `utimes` and `xattr` families, `mkdir`, `rmdir`,
`unlink`, `rename`, `link`, `symlink`, `readlink`, `opendir`, `chdir`,
`truncate`, `realpath`, `mkstemp` family, `bind`, `connect`, and the exec and
spawn calls, `posix_spawn` file actions among them. Both names of a two-path
call are mapped. `bind` and `connect` fail with `ENAMETOOLONG` where the mapped
path does not fit, never with the unmapped one.

**What comes back.** Not rewritten: `getcwd`, `realpath` and `F_GETPATH` answer
the real directory, so a program that compares them with `/tmp` sees a
difference, as it does with `/private/tmp` today. `/tmp` itself stats as the
directory.

**What the library is not.** A boundary. Calls that are not covered, raw system
calls, and processes that lost it (Apple-signed, hardened) are refused by the
policy. `go run` and `go test` programs of mise's Go have it through `-exec`.

**Measured before it is built** (0.1, 0.2): the calls the samples' tools make,
what can be interposed, and the cost of the check on every call. If devenv's
preamble, process-compose or the Python, Node and Go cases do not work, or an
ordinary build is measurably slower (proposed: more than 5%), the owner is told
before the code is written.
*Rejected:* `TMPDIR` alone with `/tmp` refused (breaks devenv); `TMPDIR` alone
with `/tmp` shared (isolates nothing), kept only as the first step.

### D4. The user database
`getpwuid`, `getpwuid_r`, `getpwnam`, `getpwnam_r` answer, for the sandbox's own
uid or name, `[identity] user`, the sandbox's `HOME` and its shell. `id -un`
then prints `devcroft`, as on Linux, and `up`'s macOS line says the name is
cooperative: the uid is still the host user's.

### D5. Lifetimes
`HOME`: kept across `down` and `up`, removed by `rm`, as today. Temp: emptied at
`down` and before `up` starts a keeper, removed by `rm`. Linux's tmpfs already
ends with the keeper.

## Risks / Trade-offs

- [A program that loses the library and uses a literal `/tmp`] → refused and
  published; `TMPDIR` still works for it.
- [Sharing through `/tmp` on purpose, with the host or between sandboxes] →
  breaks; `filesystem.allow` is the remedy, and `up` cannot tell.
- [A mapped call missing from the list] → an `EPERM` on a path that exists,
  confusing; hence the corpus in 0.1 and tests per call.

## Decided (owner, 2026-11-24)

- A `TMPDIR` the environment or `[env.vars]` sets wins, where it names a path the
  sandbox may write; else the sandbox's own.
- The provisioning process shares the directory with sessions, as it shares
  `HOME`: devenv's `/tmp/devenv-<hash>` has to outlive it.
- D3's cost budget: an ordinary build no more than 5% slower.
