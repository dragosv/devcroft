# Design: use-nono-library

## Context

See `proposal.md`. This file records what was measured, so a later
reader can tell which parts are findings and which are judgement — and
re-derive the findings when the upstream numbers move.

Measured against `nono` 0.74.0 and this repo at commit `f507f07`:

```sh
curl https://crates.io/api/v1/crates/nono/0.74.0/dependencies   # optional flags
cargo tree --edges normal                                       # both trees
curl https://api.github.com/repos/nolabs-ai/nono/contents/crates/nono/src
curl https://api.github.com/repos/nolabs-ai/nono/contents/crates/nono-cli/src
```

| | value |
|---|---|
| `nono` resolved tree | 189 crates |
| devcroft resolved tree | 158 crates |
| shared | 48 |
| net new for devcroft | 141 |
| `nono` normal deps declared `optional = true` | 1 (`keyring`) |
| probe build, cold, no `cmake` present | 15.7s wall |

**The actual library API**, read from `nono` 0.71.0's own source
(`~/.cargo/registry/src/.../nono-0.71.0/src/`, not just docs) since that's
what everything below depends on being right:

- `CapabilitySet::new().allow_path(path, AccessMode::Read|ReadWrite)?...`
  — the builder `CompiledPolicy::to_capability_set` targets. Read-only vs
  read-write is `AccessMode`, matching `filesystem_read`/`filesystem_allow`'s
  existing split exactly.
- `CapabilitySet::block_network()` / `.set_network_mode(NetworkMode::{Blocked,AllowAll,ProxyOnly{port,bind_ports}})`
  — no domain-level primitive at this layer (see the Non-Goals entry on
  `network.allow`).
- `CapabilitySet::allow_tcp_bind(port)` / `allow_localhost_port(port)` —
  `network.ports`' target.
- `CapabilitySet::set_signal_mode(SignalMode::Isolated)` — matches
  own-policy-baseline's `SIGNAL_MODE` constant exactly; the two changes
  agree on the value without coordinating.
- `Sandbox::apply_auto(&caps) -> Result<SeccompNetFallback>` (Linux) /
  `Result<()>` (macOS) — irreversible, applies to the calling process.
  `Sandbox::is_supported()` / `Sandbox::support_info() -> SupportInfo
  { is_supported, platform, details }` — `doctor`'s new target.

## Goals / Non-Goals

**Goals:**

- Pin the backend in `Cargo.lock` rather than in a hand-maintained
  version range.
- Remove the intermediate process between `up` and the keeper.
- Stop requiring an external binary at runtime for the process tier.

**Non-Goals:**

- The hardened tier. gVisor is a separate backend reached through
  `SessionBackend`; nothing here touches it.
- Reimplementing any part of nono. If something needed is missing from
  the library, the answer is an upstream request, not a local copy —
  see Decision 3.
- Owning policy content. That is `own-policy-baseline`, and it is a
  prerequisite rather than a part of this change.
- **Making `network.allow` (domain-level filtering) actually work.**
  Verified live, against real `nono wrap` invocations, that it doesn't
  today: `curl` to a domain named in `allow_domain` gets the identical
  kernel-level `Permission denied` as an unrelated one. `nono run
  --allow-domain` is the mode that provides a working filter — a resident
  credential/MITM proxy nono-cli runs internally, almost certainly backed
  by the separate `nono-proxy` crate (checked: 1223 transitive crates,
  mandatory AWS SDK + `rcgen` TLS-CA generation — a full credential-
  injection proxy, not a lightweight filter). devcroft has only ever used
  `wrap` (no resident supervisor), so this was already broken before this
  change existed. `CompiledPolicy::to_capability_set` compiles
  `network.allow` to `NetworkMode::Blocked` — the literal behavior devcroft
  already has today — rather than silently regressing (nothing to
  regress) or quietly building a proxy as a side effect of an unrelated
  migration. Fixing the domain allowlist for real is a follow-up change of
  its own.

## Decision 1: self-restriction replaces the wrap hop

**What:** the keeper, after inheriting its listener fds, calls the
library's apply-to-self entry point. `spawn_keeper` drops the `nono
wrap -p <profile> --` prefix and spawns the keeper binary directly.

**Why:** it is what the architecture already describes. The stated
invariant is that the sockets are created first and the keeper applies
the profile *to itself*, and the only reason a foreign process is in the
middle today is that self-application was only available through a
binary. The library's `Sandbox::apply_auto(&caps)` applies to the
current process and is irreversible — the same semantics, minus the
exec.

**What this removes:** the fd numbers currently travel as argv through
`nono wrap` into the keeper, and the SSH key material travels as
environment variables across the same boundary. Neither has to cross a
foreign process afterwards. That does not make the current arrangement
wrong — it works and is tested — but it does mean one fewer place where
an upstream change could break it.

**What must be proven, not assumed:** that restriction still happens
after the listeners exist and before anything project-supplied runs. The
ordering is the whole safety argument, and moving where restriction
happens is exactly the change most likely to get it wrong. It is the
first success criterion for that reason.

**What actually went wrong, twice, and how it was found:** removing
nono-cli's group catalog (Decision 5) turned out to have been silently
covering for two gaps in the keeper's own baseline grants, both masked by
the still-active `system_write_linux` group under own-policy-baseline
and both surfacing as an identical, unhelpful "keeper refused to spawn:
Permission denied" with nothing pointing at the real cause:

1. `/dev/pts` needs read+write, not read-only, for `devcroft shell`'s pty
   allocation.
2. `/dev/null` needs read+write too — every session's `Stdio::null()`
   redirection opens it for *writing* (stdout/stderr), not just reading.

Neither was found by inspection. Both were found by writing a minimal
standalone reproduction outside this crate — `CapabilitySet::new()` with
exactly the grants in question, `Sandbox::apply_auto`, then the exact
sequence devcroft's own code runs (`openpty`, fork, `setsid`,
`TIOCSCTTY`, `dup2`, `execv`) — narrowing the failure one layer at a
time: raw `fork`+`execv` succeeded, which ruled out the pty mechanics
themselves; `std::process::Command::spawn()` with the identical
`pre_exec` failed, which pointed at `Command`'s own `Stdio::null()`
handling specifically. `KEEPER_SYSTEM_READWRITE` (`src/policy/mod.rs`)
now carries both, separate from the read-only `KEEPER_SYSTEM_READ` list.
This is exactly the category of gap task group 4's own success criteria
anticipated ("proven, not assumed") — it just took a live pty session to
prove it wrong the first two times.

## Decision 2: the policy stays a rendered artifact

**What:** `policy::compile` continues to produce devcroft's annotated
representation with origins. Conversion to the library's capability set
is a projection from it, exactly as `to_nono_profile` is today.

**Why:** the origin model is devcroft's, not the backend's, and
`policy --render` plus `why` are built on it. Handing the library a
capability set changes the projection target and nothing else. A design
that built the library's types directly and rendered *from those* would
lose the origins, which is the one thing rendering exists to show.

**Side benefit worth stating:** rendering stops depending on a backend
binary being installed at all, on both counts. `own-policy-baseline`
made `policy --render` account for everything reaching the backend,
including the 13 non-excluded groups, but doing so still shelled out to
`nono profile groups` live — so `render_backend_enforced`,
`Origin::BackendEnforced`, and `why`'s backend-group attribution existed
specifically to compensate for what `nono-cli`'s implicit group
injection added *outside* devcroft's own compiled policy. Decision 5
below removes that injection from the process tier entirely, which makes
the compensating machinery dead code, not merely backend-independent —
see Decision 5 for why it's deleted rather than left unused.

## Decision 3: missing library capability is an upstream request

**What:** if the library lacks something devcroft needs, the response is
an upstream issue or PR, not a local reimplementation.

**Why:** the failure mode this guards against is specific and likely.
The library deliberately excludes the profile and group machinery; a
devcroft that starts filling gaps locally ends up maintaining a partial
copy of `nono-cli` with none of its testing. `own-policy-baseline`
already draws the line in the right place — devcroft owns the ~96 paths
its own sandboxes need, and does not own a general system-paths catalog.
This decision keeps that line from eroding.

**The known request:** gate the trust module behind a Cargo feature, or
publish enforcement separately from verification. That single change
retires this proposal's only unresolved objection, which makes it the
highest-value thing to ask for.

## Decision 4: the trust dependency is accepted

**Settled by the project owner**, on the terms this section already laid
out: nothing calls the Sigstore/TUF paths, the keeper's network
restriction is enforced by the sandbox rather than by what's absent from
the binary, and the risk is contained supply-chain/audit surface rather
than a functional one. The argument each way, preserved for the record:

*For accepting it:* nothing calls the Sigstore paths. Unused code in a
linked library is not a capability the process exercises, and devcroft
already links substantial crypto through russh. The keeper's network
restriction is enforced by the sandbox, not by what is absent from the
binary — a linked HTTP client behind an active network denial cannot
reach anything.

*Against:* the argument above proves the risk is contained, not that it
is zero, and it applies to the process tier whose own framing is
"accident protection, not a security boundary". Adding 141 crates —
including a TUF client and a second TLS stack — to the process whose
defining property is having no network is the kind of thing that should
be decided explicitly rather than justified after the fact. Supply-chain
surface is also a real cost independent of runtime behavior.

The honest position is that this is a judgement about acceptable
dependency surface, not a technical unknown, and it belonged to whoever
owns the project's dependency policy. Decision: accept it. The upstream
ask (Decision 3's feature-gate request) remains worth filing regardless —
it would retire this surface for free — but does not block this change.

## Decision 5: the backend-enforced group catalog is dropped, not replicated

**What:** the process tier's compiled policy no longer includes any
equivalent of nono-cli's 13 non-excluded groups (`deny_shell_configs`,
`deny_shell_history`, `deny_browser_data_macos`/`_linux`,
`deny_keychains_macos`/`_linux`, `deny_macos_private`, `user_tools`,
`homebrew_macos`/`_linux`, `system_write_macos`/`_linux`) — roughly 100
paths across browser cookies, shell history, keychains, and dotfiles.
`own-policy-baseline`'s `SENSITIVE_PATHS` baseline (`~/.ssh`, `~/.aws`,
`~/.config/gcloud`, `~/.kube`) plus `DEVCROFT_DATA_DIR` is what the
process tier denies going forward — unchanged from what devcroft already
owned before this change, not a new grant.

**Why this is safe to drop rather than replicate:** measured, this data
exists nowhere in the `nono` library — `deny_credentials` and its
siblings are pure `nono-cli` concepts (its own `policy.json` catalog),
invisible to code that only depends on the `nono` crate. Self-restricting
via `Sandbox::apply_auto(&caps)` — the entire mechanism this change is
built on — cannot carry a group it does not know exists. Replicating the
catalog would mean embedding and maintaining a synced copy of ~100
third-party paths, which is exactly what Decision 3 rules out
("reimplementing... is not the answer, upstream request is") and what
own-policy-baseline's own design already declined to do for the same
reason ("devcroft owns the ~96 paths its own sandboxes need, and does not
own a general system-paths catalog").

**Why this doesn't weaken the stated guarantee:** the process tier's
framing has never been "protects the host from the sandbox" for
concerns outside devcroft's own credential surface — CLAUDE.md's own
words are "accident protection, not a security boundary." Browser
cookies, shell history, and keychain data are real concerns for
`nono-cli`'s target use case (wrapping an arbitrary, possibly untrusted
AI agent with broad host access), not for devcroft's: a project's own
code, running against a curated provider closure, with devcroft's own
credential paths already denied. Confirmed with the project owner rather
than assumed.

**What this obsoletes:** `render_backend_enforced`, `Origin::BackendEnforced`,
and `why`'s backend-group attribution fallback (all added by
`own-policy-baseline` specifically to make nono-cli's implicit group
injection inspectable) have nothing left to report once the process tier
stops injecting those groups at all — deleted, not left as unreachable
code, per the project's own "don't leave half-finished/backwards-compat
shims" convention. `policy --render`'s output shrinks back to exactly
what devcroft's own `CompiledPolicy` carries, which is now also *all*
that's enforced — render and reality match by construction, the same
property own-policy-baseline established for the manifest+baseline half
and this change now extends to the whole policy.

## Risks

- **Restriction ordering regression.** The highest-consequence risk, and
  the reason this is not a mechanical refactor. Mitigated only by tests
  that assert the socket is reachable from outside after restriction —
  which exist, and must be run against the new arrangement rather than
  assumed to still apply.
- **Feature unification.** Linking `nono` pulls `rustls` with its
  default features into the shared dependency graph. devcroft cannot
  turn off default features of a transitive dependency, so the crypto
  provider is decided by nono, not devcroft. Worth checking before
  committing, since it can affect the whole binary and not just the
  backend path.
- **Losing an escape hatch.** An external binary can be swapped,
  version-matched, or patched by a user without rebuilding devcroft. A
  linked library cannot. For a tool whose users are exactly the kind of
  people who patch their sandboxing layer, that is a real if minor loss.
