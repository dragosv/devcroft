# Implementation log

What was built, in the order it was built, and — more usefully — **what
turned out to be wrong along the way**. Moved here from the README, which
had accumulated 376 lines of this and had stopped being readable as an
introduction to the project.

Kept rather than deleted, deliberately. Most of these entries record a
belief this project held, tested, and had to correct: a `--json` flag that
looked like a fix and wasn't, a tier that could not be stacked with
Landlock, a capability claim refuted by one command. The corrections are
the durable part — they are why several decisions look the way they do,
and re-deriving them would cost more than keeping them.

**This is history, not current state.** For what devcroft does today see
the README; for why a given thing was rejected see
[decisions.md](decisions.md); for what is in flight see `openspec list`.
An entry here describes what was true when it was written.

---

**MVP implementation underway — 23/25 tasks.** A `..` path-traversal gap in
task 2.1's `filesystem.allow`/`read`/`deny` validation, and a task 3.2
reproducibility gap where flox activation inherited whoever's shell ran
`up` (personal `PATH`, ad hoc env vars) instead of a fixed environment,
were both found and closed along the way — see the git history for each
fix. The fd-passing keeper trick
(spike binary, task group 1) is proven on both Linux/Landlock and
macOS/Seatbelt; the config/policy compiler, the environment provider layer
(`flox` resolution, task group 3), the keeper's spawn protocol (control
socket, session registry, pty allocation — task 4.1), the supervisor
(`up`/`down`/`rm`, idempotent with crash recovery and `--recreate` — task
4.2), read-only sandbox introspection (`status`/`logs`/`ps` — task 4.3), and
sessions — task group 5 in full: one-shot `exec` with exit-code
propagation, cwd mapping, and signal forwarding (5.1); interactive `shell`
with a real pty, resize propagation, and a `$SHELL`-then-`/bin/sh` fallback
(5.2); and auto-up (`exec`/`shell` bring a cold sandbox up themselves unless
`--no-up`, 5.3) — are implemented and tested end to end against real `nono`
and `flox`. Task group 6 (SSH endpoint) is now complete except for its
cross-editor validation matrix (6.5, nearly done — OpenSSH and rsync are
validated by real end-to-end tests, VS Code Remote-SSH and Cursor are
validated by real manual connections against a live sandbox, only Zed
remains (no CLI to drive it here) — see
[docs/ssh-validation.md](ssh-validation.md)): an SSH server (russh)
embedded in the
keeper on a second unix socket, mode 0600 in the state dir's mode 0700, bound
host-side and fd-passed the same way the control socket is, with publickey
auth against the devcroft client keypair and a fresh ephemeral host key per
`up` (6.1); `devcroft proxy <name>.devcroft` and `devcroft ssh-config
[--write]` (6.2); and full channel support (6.3/6.4) — exec, pty/shell with
resize and an env allowlist (`TERM`/`LANG`/`LC_*`), exit status, the `sftp`
subsystem (also what modern `scp` speaks by default), and `-L` direct-tcpip
forwarding gated by nothing devcroft-specific — it just lets the sandbox's
own network restriction accept or reject the target, same as every other
syscall the keeper makes. All of it is tested against the real `ssh`/`scp`/
`sftp` CLIs through a real `devcroft proxy` subprocess, not just a russh test
client. Task group 7 (CLI polish & release) is well underway: `devcroft
init` detects an existing flox environment or a bare single-ecosystem
toolchain pin (`rust-toolchain.toml`/`.nvmrc`/`.python-version`) and
generates a minimal manifest without ever overwriting one without
`--force`; its default sandbox name (the directory slug) is disambiguated
with a short path-derived suffix only on a real collision against another
project's already-existing state — e.g. two unrelated projects both named
`api` — so the common case keeps the plain slug and only a genuine clash
gets a suffix; `devcroft doctor` reports backend presence/version-range, kernel
sandboxing capability, the provider binary, `ssh-config` managed-section
state, and (when a manifest is discoverable) which of its aspects would be
degraded on this host, with every `FAIL` naming its fix (7.1). The rest of
the command surface is wired up too, each with the stable 0–5 exit codes
and layer-named errors the cli spec's error contract requires: `up`
(idempotent, `--recreate`), `down`, `rm`, `status`, `logs`, `ps`, `policy
--render`, `why --path`/`--host`, and `ssh` (execs a real system `ssh` with
the right options pre-filled). Destructive operations (`rm`, `up
--recreate`) refuse to run non-interactively without `--yes` (7.2). Two
sandboxes now have end-to-end coverage
running side by side with disjoint state and independently-enforced
policy, and a keeper survives a freeze/resume cycle (`SIGSTOP`/`SIGCONT`
on the keeper pid, the realistic proxy for host suspend/resume available
in this environment) with the next command transparently confirming
health rather than assuming it (7.3).

**Post-MVP:** `add-nix-provider` is implemented — nix flakes as a second
`env.provider` value alongside flox, same closure tier, same contract
(`Provider` trait, host-side activation capture, store grants, staleness
fingerprinting). `init` and `doctor` both learned about it; see
[samples/nix-flake-sample](../samples/nix-flake-sample/) for a working
example and `openspec/changes/add-nix-provider/` for the full spec. This
also closed a real, pre-existing gap that predated nix entirely: `policy
--render`/`why` never showed *any* provider's store grants before this
(`Origin::Provider` existed since MVP with no caller) — fixed for flox
and nix alike.

**Superseded — the hardened tier described in the next three paragraphs was
removed** (`remove-gvisor-backend`; see Limitations below for why, and the tag
`gvisor-backend-last` to recover the code). The history is kept rather than
deleted because the measurements in it are the durable part: what stacking
Landlock on a container runtime actually costs, and three integration defects
that only real toolchains surfaced.

**`add-hardened-tier`/`add-gvisor-backend` were implemented and
verified live**, end to end, against a real rootless `runsc` (17/17 and
28/28 tasks). The manifest's `[sandbox].isolation` key, the
`SessionBackend` trait `lifecycle::up` dispatches sessions through, and
the `gvisor` module (OCI bundle synthesis from the same `CompiledPolicy`
the process tier compiles, `runsc` command assembly, `doctor`
diagnostics, a pinned `runsc` install in the devcontainer) are all
implemented, unit tested, and covered by real-tooling integration tests
(`tests/gvisor_hardened_e2e.rs`, `tests/hardened_tier_ssh_parity.rs`,
`tests/hardened_services_wiring.rs`) that self-skip wherever `runsc`
isn't functionally usable, the same convention every other real-tooling
test in this suite already follows. One correction along the way, made
before any code shipped against the wrong assumption: an earlier draft
leaned on gVisor's per-sandbox netstack to close the listen-socket gap
below for free, but `runsc` rejects that mode outright under
`--rootless`, and devcroft runs unprivileged everywhere by design — so
the hardened tier shares the host's network namespace exactly like
`process` does, and does **not** close that gap either (see the note
below).

**Getting a real `runsc` running here took two fixes, landed in
sequence, each confirmed against the running container rather than
assumed:** first, `.devcontainer/devcontainer.json` sets
`"runArgs": ["--security-opt", "seccomp=unconfined"]` — the container
runtime's default seccomp profile was blocking `clone(CLONE_NEWUSER)`
for a process without effective `CAP_SYS_ADMIN` (not the more commonly
cited `kernel.unprivileged_userns_clone` sysctl, which doesn't exist on
this kernel at all), diagnosed directly against `/proc/self/status` and
confirmed fixed by a later rebuild: `unshare --user --map-root-user` and
`runsc --rootless --platform systrap do true` both now succeed in this
devcontainer. Second — and this is what actually let a full `up` at
`isolation = "hardened"` complete — the Landlock profile this module
used to apply to itself before exec'ing into `runsc run`, as defense in
depth additive to gVisor's own Sentry confinement, was **removed**. It
turned out to make `--rootless` bootstrap fail unconditionally on every
host, not just this one: `runsc run`'s own chroot setup issues a
`mount()` call to change mount propagation, and that call returns
`EPERM` under *any* active Landlock ruleset regardless of what it
grants — confirmed by elimination (a ruleset granting `/` full
read-write still failed identically), and Landlock cannot mediate
`mount()` in any current ABI, so no grant could have fixed it. This had
never been exercised against a real unprivileged user namespace before
today; see `src/gvisor/runner.rs`'s module doc and
`openspec/changes/add-flox-services/tasks.md` task 6.5 for the full
evidence trail.

With both fixes in, and two more real bugs caught by the same live run
and fixed alongside them — `oci_spec::build`'s bundle never pre-created
each mount's destination directory inside `rootfs/` (gVisor's gofer
requires one to exist before it will bind onto it), and `root.path` was
a relative `"rootfs"` where gVisor's own symlink-escape guard requires
an absolute path — **a full `up` at `isolation = "hardened"` now
completes end to end**: `exec` and the SSH round trip both work (a third
bug, `runsc_command::exec_args` inserting a `--` separator `runsc exec`
doesn't expect or want, was found and fixed by this same run), and a
project declaring `[services]` gets a real `process-compose` running
inside the sandbox via `runsc exec`, with `ps`/`status`/`logs` showing
it and `down` reaping it cleanly. Every one of this tier's claims that
was previously "implemented but unverified" is now verified live, not
just reasoned about.

**`own-policy-baseline` is implemented.** Every profile devcroft compiled
used to carry 240 rules across 18 backend policy groups that `policy
--render` could not show — a typical sandbox rendered 8 rules and shipped
248. The unrendered majority came from nono injecting its full group set
into any profile, `extends: "default"` or not (confirmed with `nono
profile diff`: `extends` contributes exactly one setting, `signal_mode`).
Fixed at the root: the compiled profile now names, via `groups.exclude`,
every group it declines — `system_read_linux_core`/`system_read_macos`
(broad host `/usr/bin`, `/lib`, `/usr/share` access that contradicted
devcroft's own closure-tier thesis) and the inert `dangerous_commands*`
blocklist (verified live that `rm`/`cp` both succeed under it — `wrap`
has no resident supervisor to enforce a command blocklist, so emitting it
would claim a protection that isn't real). `signal_mode` is now set
explicitly rather than inherited. What still reaches the backend outside
devcroft's own rules — the eight required deny groups plus five narrow
optional ones (`/tmp`, `/dev` writes, a handful of `~/.local`/Homebrew
paths) this change deliberately leaves alone — is rendered too, sourced
live from `nono profile groups <name> --json` and attributed to
`backend:<group>` rather than devcroft's own `baseline`, so `policy
--render` now accounts for literally everything reaching the backend, a
claim verified by a test that resolves a real compiled profile through
nono and asserts nothing comes back unaccounted for.

The result is real, not cosmetic: `/usr/bin/gcc` and `/bin/ls` are now
denied inside every process-tier sandbox, verified live against
`samples/flox-clap-sample` (a full `cargo build` still succeeds, entirely
from the flox closure) and `samples/nix-go-sample` (`go build` too, once
`/tmp` — needed for Go's build scratch dir — was added to the sample's
own manifest, the same declaration any project needs now that the
baseline no longer grants it implicitly). Two independent, pre-existing
bugs were found and fixed along the way, both host-toolchain-passthrough
masking the same class of gap this change targets: `devcroft shell`'s and
the SSH server's `$SHELL`-then-fallback logic used to fall back to an
absolute `/bin/sh`, a host path no provider closure can ever satisfy —
now a bare `sh`, resolved by `PATH` inside the sandbox like every other
command, so a project that installs a shell into its closure gets a
working `devcroft shell`. And a generated `process-compose` services
config relied on its own undeclared `/usr/bin/bash` default, fixed by
naming `sh` explicitly (`shell_command` in the generated config) for the
same reason. `doctor`'s backend check now also exercises the actual
interface — schema validation and a live check that `groups.exclude`
still resolves the way the compiled policy assumes — rather than asserting
a version number alone, and the tested range widened to `>=0.71.0,
<0.75.0`, verified against both ends live.

**`use-nono-library` is implemented.** The process tier no longer execs a
`nono` binary at all — `nono` moved from a runtime `PATH` dependency to a
linked library, and the keeper applies the compiled policy to *itself*
directly (`nono::Sandbox::apply_auto`) right after inheriting its
listener fds, closing the fd-passing hop through a foreign process the
architecture's own listener-before-restriction invariant always described
as temporary. `nono` is no longer required on `PATH` for `up`/`exec`/
`down` to work; `doctor`'s backend check now reports kernel/platform
support (`Sandbox::support_info()`) instead of a binary version. Verified
live: a full `cargo build` under `flox-clap-sample`, with the built
binary running, `/usr/bin/gcc` and `~/.ssh` denied throughout, and no
`nono` process anywhere in the sandbox's process tree.

This is a real, security-relevant scope narrowing, not a side effect:
own-policy-baseline's rendering of nono-cli's ~100-path group catalog
(browser cookies, keychains, shell history, dotfiles beyond devcroft's
own baseline) is gone along with it — that catalog is a pure `nono-cli`
concept, invisible to the raw library. The process tier's credential/
privacy protection is devcroft's own `SENSITIVE_PATHS` (`~/.ssh`,
`~/.aws`, `~/.config/gcloud`, `~/.kube`) and `DEVCROFT_DATA_DIR`, exactly
as before, and always the load-bearing part — nono-cli's broader catalog
targets a different threat model (wrapping an arbitrary, possibly
untrusted AI agent with broad host access) than devcroft's (a project's
own code, running against a curated provider closure). Confirmed with the
project owner rather than assumed; see `openspec/changes/use-nono-library/design.md`
Decision 5 for the full reasoning.

`network.allow` (domain-level filtering) was unaffected by *this* change
in the sense that mattered at the time: it was already non-functional
under devcroft's `nono wrap`-based invocation (`wrap` has no resident
supervisor, and domain filtering needs one — verified live that a `curl`
to an *allowed* domain got the identical kernel-level denial as an
unrelated one), and still compiled to a plain network block under the
library. **Superseded by `add-egress-proxy`, which fixed it for real**:
`network.allow` now compiles to `NetworkMode::ProxyOnly` (a Landlock
`NetPort`/Seatbelt rule permitting `connect()` to a resident proxy's port
and nothing else), and that proxy makes the actual per-hostname decision.
Verified live end to end (`tests/egress_proxy_e2e.rs`): a real `up`, a
real `curl` inside the sandbox, an allowed loopback host reachable
through the proxy, a nearby-but-not-allowlisted one refused with a `502`
naming it.

**Service reporting was rebuilt after a review found it silent in four
different ways.** All four shared one shape: a service problem that
showed up as *nothing at all*. `status` learned service state only by
asking `process-compose`, so anything the supervisor could not answer for
vanished rather than being reported — against the `services` spec's own
"SHALL NOT be omitted from service listings".

- **A dead supervisor looked like a healthy sandbox.** Three declared
  services plus a `process-compose` that died at startup produced output
  byte-identical to a project declaring no services; the only trace was a
  line in the keeper log. `up` now records the declared service names,
  and `status`/`ps` reconcile the live answer against them, so an
  unreachable supervisor is named and its services still listed.
- **Two of the four service states were wrong, measured live against
  process-compose 1.120.0 rather than assumed.** A service still waiting
  on a `depends_on` gate reports `is_running: false, exit_code: 0` — read
  by exit code alone, `status` called it "exited", so a service that
  hadn't started yet looked like one that had already finished. A service
  *skipped* because its dependency failed reports `exit_code: 1` that no
  process ever produced, rendering as "failed (exit 1)", an invented
  failure. Both now report as themselves. `exit_code` remains
  authoritative for services that actually ran, because a real crash
  reports `status: "Completed"` exactly like a clean exit does.
- **Service artifacts were keyed on the project root alone.** Two
  sandboxes with different names sharing one root overwrote each other's
  generated config, raced for one supervisor socket, and each reported
  the other's services. They now live in `.devcroft/<sandbox-name>/`, and
  `rm` cleans up the directory it created. Since the path grew a level,
  `up` also checks it against the OS socket-path limit and fails at layer
  `config` rather than letting the supervisor fail to bind for an
  unstated reason.
- **`.devcroft/` was gitignored nowhere**, so devcroft's own generated
  files left `git status` dirty in every worktree — worst in exactly the
  fan-out flow it targets. `init` now adds the entry.

One hardening fix came with them: `status`/`ps` read a socket the
*sandbox* controls, which is accident protection at the process tier and was
a real trust inversion at the since-removed hardened tier, where
`--host-uds=create` existed precisely to let the host reach inward. The
hardening stands on its own merits regardless. That read now verifies the path
is a socket owned by the invoking user, caps the response size, and
bounds the whole exchange rather than only each individual read.

**`add-flox-services` is implemented.** A flox environment's documented
`[services]` declarations are read host-side at `up`, started **inside**
the sandbox after restriction (they are project code, so they get no
provisioning privileges), supervised by the keeper for the sandbox's
lifetime, and reaped at `down`. `status`/`ps`/`logs` report them per
service; no new top-level command was added, and a test pins that closed
surface so `devcroft services` cannot quietly appear later.

devcroft generates its own process-compose config from the published
schema rather than shelling out to `flox services` (which would need the
flox binary executable inside the profile — what the "environment
resolves once" invariant already rejects) or consuming flox's
undocumented generated one. The stated cost: `flox services status` run
by hand shows nothing, because flox did not start these processes.
`devcroft doctor` says so explicitly in any project that declares
services, rather than leaving an empty list to be misread.

**Finishing it found a real defect in teardown, and the first fix for it
was wrong.** The `services` spec promises no service process survives
`down`, verified by observing process absence rather than by trusting a
stop command. It did not hold: the shutdown handler killed the registered
*process group*, which is process-compose's, while process-compose puts
each service in a group of its own — so a service that ignores SIGTERM
outlived `down`, got reparented to init, and kept running. Every earlier
services test used a process that dies politely, which hid it completely.

The first fix set process-compose's per-process `shutdown.timeout`, which
reads exactly like the missing escalation, and an initial probe seemed to
confirm it. The probe was an artifact — process-compose was a background
job of a shell that exited moments later, and that is what cleaned up the
group. Re-measured in isolation with `setsid` and ten seconds to act,
process-compose 1.116.0 does not escalate at all: it hangs after logging
"Caught terminated" with the stubborn child still alive. The working fix
keeps the guarantee where the spec puts it — devcroft's: the keeper asks
the supervisor for its service pids before signalling anything, and
includes each service's own process group in both sweeps. (The hardened tier
had a different mechanism — teardown destroyed the sandbox, taking everything
in it — which is moot now that the tier is gone.)

**`add-devbox-provider` is implemented.** devbox is a third closure-tier
`env.provider`, resolved by capturing `devbox shellenv --pure` (never
`devbox run`, which — measured, not assumed — runs a project's
`shell.init_hook`; `shellenv` never does, in any variant) and reusing the
same fixed-baseline diff, store-grant, and staleness machinery flox and
nix already share. This is the second provider proposed purely to
confirm the `Provider` trait generalizes to a substrate the first two
don't share (devbox has its own resolver and its own lockfile format, no
flake underneath), and it does: only `src/provider/mod.rs` (dispatch
arms) and `src/provider/validate.rs` (one name moved lists) changed
shape beyond the new module. See
[samples/devbox-citytime-sample](../samples/devbox-citytime-sample/) for a
working example.

Two corrections were found live while implementing, not while designing —
both narrowed what the change originally assumed devbox needed:

- **The lockfile precondition checks key presence, not per-system
  coverage.** A draft precondition required a declared package's
  `devbox.lock` entry to cover the system `up` runs on, reasoning that an
  entry resolved only for another platform leaves the current one
  unresolved. Measured against a real capture: it doesn't — devbox
  resolves any system from the entry's *pinned commit reference*, which
  is system-independent, without touching the lockfile. What actually
  contacts a package index and rewrites the lockfile — confirmed
  directly, `nixpkgs-unstable` fetched from `cache.nixos.org`, lockfile
  mutated on disk — is a declared package with **no key at all** in
  `devbox.lock`. The precondition checks exactly that.
- **Store grants need no profile-symlink resolution.** A devbox project's
  declared packages reach `PATH` through a `.devbox/nix/profile/default`
  symlink chain rather than as bare store paths, which looked like it
  would require deriving grants by resolving that chain instead of
  reusing the other two providers' scrape-`PATH`-for-`/nix/store`
  mechanism. It doesn't: that mechanism already returns only the coarse
  `/nix/store` root, never an enumerated path, and devbox's own stdenv
  wrapper puts real `/nix/store/...` entries on `PATH` regardless of
  declared packages — so the existing mechanism, reused completely
  unchanged, already grants everything the symlink resolves to. Verified
  with a package outside devbox's stdenv (ripgrep), so the claim is
  falsifiable rather than assumed.

Both are recorded in `openspec/changes/add-devbox-provider/design.md`
decisions 1a and 1b, corrected in place rather than left standing next to
their own contradiction.

**Then an adversarial review of the shipped result found the precondition
did not deliver the rule it was written for**, and that is worth stating
plainly because the change had already been committed as complete.
`up` rewrote the user's `devbox.lock` during provisioning — precisely
what the `env-provider` spec says resolution SHALL NOT do. A project
whose every *declared* package was locked still slipped through, because
`devbox.lock` also carries devbox's own base nixpkgs entry, which no
per-package check can see; `up` resolved that entry against the floating
`nixpkgs-unstable` branch and wrote it to disk. Now enforced by comparing
the lockfile's bytes across capture and restoring + failing on any
change — a byte comparison rather than a larger precondition, since the
base entry's key is not a constant (a project pinning `nixpkgs.commit`
locks under a different one) and predicting it would mean reimplementing
devbox's resolution rules.

That carried a second correction with it: **"declares no packages" does
not mean "nothing to resolve"**. A zero-package devbox project still gets
its stdenv from that same unpinned base, so it is reproducible only once
`devbox install` has written a lockfile. The spec scenario asserting such
a project needs none, `init`'s matching advice, and three tests were all
wrong in the same way, and are corrected together. Two of this change's
own tests had also been passing for the wrong reason, because `devbox
add` — unlike `devbox install` — does not write a complete lockfile.
---

**A published gap turned out to be a wrong diagnosis.** The README once
claimed there was no way to express "no outbound access, but I can still
run my dev server" — a `network` policy that blocked all connections was
assumed to also block `bind()`/`listen()`. False: nono's profile schema
has always carried an `open_port` field; devcroft simply never emitted it.
`[network].ports` now does — `default = "deny"` plus `ports = [3000]`
binds `127.0.0.1:3000` while egress stays filtered and ungranted ports
stay denied, verified end to end in `tests/network_ports_listen.rs`. Worth
recording how long the wrong claim survived unchecked: it was repeated
across the docs and treated as an architectural constraint, and one `nono
profile schema` invocation refuted it.

---

**Isolation and egress stopped being mutually exclusive, and a recorded
blocker turned out to be about the wrong thing.** `wants_network_isolation`
originally refused any sandbox with `network.allow`, on the reasoning that
an isolated namespace has no route to the host-bound egress proxy and
bridging one needs a forwarding helper (pasta/slirp4netns) blocked on
`/dev/net/tun` — which `add-linux-agent-fleet`'s D5 spike had measured as
absent here.

That reasoning was about **IP routing**, which devcroft never needed. It
needs TCP streams reaching a proxy. A *pathname* unix socket crosses a
network namespace, because it is a filesystem object governed by the mount
namespace rather than by netns. So the proxy gained a unix listener and the
keeper relays to it from inside the namespace, and a sandbox now gets its
own port table *and* filtered egress — the combination an agent actually
needs, and the one shape devcroft could not produce.

Worth keeping because the mistake is reusable: a blocker had been recorded
accurately (the TUN device really is absent) against a requirement that was
never the actual one.

**The same property is a hole in the boundary.** Landlock's network rules
cover TCP only, so AF_UNIX connect falls through to plain filesystem
permissions — a sandbox reaches any world-accessible unix socket, including
`/nix/var/nix/daemon-socket/socket` with `/nix` ungranted, which is the
package-manager authority `sandbox-provisioning` says agents must not hold.
`sandbox-provisioning`'s design.md had asserted the opposite. Recorded as a
gap that a test asserts *because it is open*
(`tests/unix_socket_not_mediated.rs`), so closing it fails loudly rather
than leaving another stale claim. The fix is a mount namespace, not seccomp
— measured — and is specified as `add-mount-isolation`.

**And the isolation work broke something no test could see.** Giving a
sandbox its own namespace also took its declared ports off the host's
loopback, so "run a dev server, open localhost:3000" stopped working for
exactly the `default = "deny"` shape the docs recommend. Every port test
devcroft had was written from *inside* the sandbox — binding via `exec`,
counting processes from the host — and all stayed green while the property
a user cares about disappeared. A test that only looks from inside the
boundary cannot see a change in what the boundary exposes.

**A correction that did not reach the artifacts people act on.** The D5/D9
findings above were written into fleet's `design.md` and left out of its
`tasks.md`, where a blocking gate ("no proxy work starts until this
resolves") stood unqualified for another commit. design.md is read to
understand; tasks.md is read to decide what to do next. Correcting only the
first is how a stale blocker survives being disproved.

**A release review, and the number it changed.** Auditing everything for a
first publish moved the version from `0.1.0` to `0.0.1`. Not a
downgrade of confidence in the code — the same code either way — but a
correction of what the number *claims*. `src/lib.rs` already told readers
the modules are internals with no stability guarantee, and `0.1.x` says
the opposite to cargo, which resolves `0.1.1` for a `0.1.0` dependant.
`0.0.z` is the one range treated as incompatible with itself, so the
assertion in the doc and the constraint in the resolver finally agree.
The second reason is the roadmap's own: the next milestone is titled "the
boundary is what the documentation says", which concedes today's is not,
and `tests/unix_socket_not_mediated.rs` asserts the hole. The more
confident number belongs on the more finished thing.

**The audit found one real blocker, of a kind the previous audit was
structured not to see.** `src/bin/` targets are auto-discovered, and the
`include` allowlist matched `spike.rs` — so `cargo install devcroft` would
have put a second binary, named `spike`, on every user's PATH. The earlier
packaging audit had been thorough about the package's *contents* (265
files and 2.0 MB cut to 50 and 781 KB) and asked nothing about what those
contents would *install*, which is a different question with a different
failure mode. Fixed by excluding the source, since removing the file
removes the target.

**And it retired a gap that had already been closed elsewhere.**
`docs/ssh-validation.md` still described "no outbound access, but my dev
server can still listen" as inexpressible, the finding that blocked VS
Code. `network.ports` has since emitted nono's `open_port` alongside
`block: true`, so that combination works and `tests/network_ports_listen.rs`
asserts it. The correction is narrower than it first looked, which is why
the section was rewritten rather than deleted: `network.ports` grants
*named* ports, and VS Code's supervisor picks its port at runtime, so the
general gap closed while this specific consumer's did not. Two documents
had drifted in opposite directions — one claiming an editor works, one
claiming the mechanism it needs does not exist — and neither was right.

**Then the release audit's own leftovers found a real defect, and the
route to it is the point.** Three unit tests had been failing on this
devcontainer and were written off as environmental — the `nix-daemon` is
not running here, and no `sudo` exists to start it. Two of them failed in
a shape that does not match "the host is broken": they expected a refusal
and got `Ok`. Chasing that found the devbox provider swallowing devbox's
exit status entirely.

The capture was one shell command,
`sh -c 'eval "$(devbox shellenv --pure)" && env -0'`, followed by a check
of the shell's status. **Command substitution does not propagate status.**
A devbox that failed printed nothing, `eval ""` succeeded, `env -0`
succeeded, and the shell exited 0 — so the status check was unreachable
for the only failure it was written to catch, and `resolve` returned `Ok`
with an environment containing nothing but `PWD`. `up` would have built a
sandbox with none of the project's tooling in it and reported success:
precisely the "never let it pass silently" rule this repository states for
providers, broken by a shell idiom rather than by a decision. Now two
steps — devbox runs as its own process so its status is seen, and only its
output is eval'd — plus a refusal for the case that check still cannot
see, a devbox that exits 0 having printed nothing.

**The tests' guards were the same mistake one level up.** They checked
`flox --version`, `flox init`, `nix flake --help`, `devbox version` — all
of which succeed with an unreachable store, none of which is the
capability the tests need. So a host that could build nothing reported
~80 e2e failures that read as devcroft regressions, which is how the
provider defect above stayed hidden inside noise for a whole release
audit. `provider::host_can_build_nix_closures()` now probes the daemon
socket and is shared by every one of them; a missing socket counts as
usable, since a single-user store has none and the safe direction for a
skip guard is to run the test.

Two of those guards were worse than merely wrong.
`tests/unix_socket_not_mediated.rs` asserts an open security gap and told
its reader that a failure means "the gap has closed and those docs need
correcting" — but a socket file with no daemon behind it produces exactly
that failure, so a dead daemon would have reported a boundary hole as
fixed while it stood wide open. It now connects from outside the sandbox
first, which turns the assertion into the implication it always meant:
reachable out here, therefore it must not be reachable in there. And
`tests/symlink_escape_cli.rs` guarded on `flox init`, which writes a
manifest without building anything, so `up` failed at layer `provider`
before reaching the policy check and the test reported the escape as
un-refused — a false accusation against code that was working.

The general lesson is about where a test's *evidence* comes from. All
three of these had a guard, and all three guards tested the presence of a
tool rather than the property the test's conclusion depends on. A green
suite here is still not a suite that ran: `cargo test -- --nocapture |
grep skipping` is what says which.
