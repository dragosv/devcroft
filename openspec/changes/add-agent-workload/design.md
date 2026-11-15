# Design: add-agent-workload

## Context

See `proposal.md` — Why, including the live probe showing `node` absent
and `~/.claude` unreadable inside a real sandbox.

Constraints that shape everything below:

- **No non-reproducible mode** (CLAUDE.md framing rules). `host` and
  `none` providers are out of scope by design. Any mechanism that puts a
  host binary inside the sandbox is that rejection reopened under a new
  name — and the provider test explicitly warns that "a provider that
  covers language runtimes but leaves the C toolchain to the host has
  smuggled `host` passthrough back in."
- **Fixed composition order** (`env-provider`): the environment resolves
  before restriction, never after. That requirement says nothing about
  two environments (an earlier version of this design read it as if it
  did); how the layer and the project combine is Decision 4, a
  requirement of its own in `tooling`, and it must be deterministic.
- **devcroft reads no user configuration today** — verified 2026-10-29.
  Decision 1 introduces the first.
- **Provider resolution must not widen the policy.**
- **`meta.json` already records `project_root`** — verified. The
  worktree bug is a missing check over data already on disk, not a
  storage change.
- **`~/.claude` is not in `SENSITIVE_PATHS`** — verified. A user can
  grant it today with an ordinary `filesystem.read`. The design question
  is not "can this be done" but "what is the narrowest correct form",
  since the ordinary form grants a whole directory of unrelated state.

## Goals / Non-Goals

**Goals:**

- An agent runs inside the sandbox without appearing in the project's
  own dependency manifest.
- Both auth shapes work, with the file-based one bounded to a single
  file.
- N worktrees of one repo produce N sandboxes, and the current silent
  sharing becomes a loud failure.

**Non-Goals:**

- The listening-socket gap. An agent editing code needs no listener;
  this change is deliberately independent of that blocker so it is
  implementable now.
- Per-sandbox port allocation, service supervision (`add-flox-services`).
- A project-declared (committed) tooling layer — see Decision 1. A team
  that wants an agent reproducible for everyone already has the answer:
  the project's own environment, option (a).
- Composing the tooling layer's environment beyond `PATH` — see
  Decision 4.
- Any integration code for a specific agent IDE. The SSH surface already
  supports `ProxyCommand`-based connection; nothing here is
  vendor-specific.

## Decisions

### 1. The tooling layer is a declarative environment, declared by the user

Options for getting an agent runtime inside. Three were considered when
this was written; (d) was added on reading what the ecosystem ships, and
(e) only became possible later — the count is left visible rather than
smoothed over, because which options existed *when* is the reason (c) was
chosen:

**(a) Tell users to add it to the project's environment manifest.**
Rejected as *the* answer: it versions a personal tool as a project
dependency, and imposes it on every developer on the team whether they
use an agent or not. It works, though, and is what the eleven agent
samples do (`docs/agents.md`); a team that does want one agent for
everyone should keep doing it.

**(b) Mount the host's agent binary read-only.** Rejected on the
property that fails, not on taste: a host-linked binary is not part of
any closure, so the sandbox's environment stops being reproducible from
its manifests. This is exactly the `host` passthrough the project has
already rejected, and it would fail the six-criterion provider test on
criteria 2 (restorable lockfile) and 5 (completeness).

**(c) A second declarative, locked environment, composed at `up`.**
Chosen. It keeps every existing invariant intact — resolved host-side in
the trusted phase, contributing only a read-only store grant — while
separating "what the project needs" from "what I need in order to work
on the project." What it contributes to a session is its `PATH` and
nothing else (Decision 4).

**(d) Read what the ecosystem already ships.** Not an option for the
runtime, but its absence from this list was the real gap: nono publishes
signed packs, and `nolabs-ai/claude` exists. Read live
(`docs/prior-art.md`): a pack carries a policy profile and Claude Code
plugin wiring, **no binaries** — so it does not answer the runtime problem.
What it does carry is a technique this change should adopt separately —
a denial-triggered feedback channel that tells the agent *why* a tool call
was refused, which devcroft has the expensive half of already (`why`) and
exposes to nothing.

**(e) Run the vendor's own installer in `[hooks] post_create`.** Not
available when this list was written, and now the cheapest thing on it.
`own-sandbox-environment` gave every sandbox a writable `HOME` of its own
(`<project>/.devcroft/<name>/home`), which is precisely what an official
install script wants and what the old `HOME` — the host user's, denied by
the baseline — refused. So the agent installs the way its vendor documents,
into a directory `rm` already owns the lifetime of, and devcroft
reimplements no part of that path.

It does **not** replace (c), and the difference is the tier, not the
convenience: an installer fetches a host-linked artifact, so what it
produces is `artifact` tier at best and unpinned in practice. A hook also
runs *inside* the boundary by the two-phase rule, so it needs its own
`network.allow` entry for the vendor's domain and cannot reach for host
tooling. (c) remains the answer for a developer who wants their agent
pinned, and the same in every project they work on; (a) for a team that
wants one agent for everyone; (e) for one developer who wants it working
this afternoon in one sandbox, and it should be documented as exactly
that rather than sold as equivalent.

**Where it is declared: the user's configuration, not the project
(reversed 2026-10-30, owner's decision).** The first version of this
design chose the project, committed:

> The agent case genuinely wants a user-level layer — my machine, my
> agent, not my team's. But a user-level layer is per-machine by
> construction, and per-machine variance is the property devcroft exists
> to eliminate.

Two things undid that. First, a committed `[tools]` is option (a) in a
different file: the team's repository still names one developer's agent,
and the only thing gained is that a colleague running `flox activate`
without devcroft does not get it. The case that motivates the change —
my laptop, my agent, the team's project — was the one rejected. Second,
the premise was too broad. devcroft's reproducibility claim has always
been about **the project's environment**, and it already tolerates
operator tooling outside the closure: the editor's server is downloaded
into the sandbox, unpinned (`docs/ssh-validation.md`), and nobody reads
that as breaking the project's guarantee. An agent is the same kind of
thing. It is the operator, not a dependency.

So the layer is declared in a user configuration file,
`$XDG_CONFIG_HOME/devcroft/config.toml` (`~/.config/devcroft/config.toml`
when unset, on macOS too), as `[tools] provider = "…"` and `path = "…"`,
naming a directory that holds a flox, devenv, nix flake or devbox
environment of the user's own. `up --tools <path>` names one for a single
invocation and takes precedence over the file. It is never read from
`devcroft.toml`.

The variance is admitted, not hidden, and bounded on three sides:

- **It must still be locked.** A layer with no lockfile is refused at
  `up`, as before. A user layer is per-machine, but on that machine it is
  reproducible.
- **It is recorded.** `Meta` holds the layer's path and a fingerprint of
  its lock; `policy --render` renders from `Meta`, so the policy shown is
  the policy applied, whatever the configuration file says now.
- **It is said.** `up` and `status` name the layer, so "this sandbox has
  more on `PATH` than the project declares" is never discovered by
  surprise.

This is the first user configuration devcroft reads. The policy's inputs
become manifest, provider grants, baseline and the layer, and that is
worth stating plainly rather than letting it arrive as a side effect.

### 2. Credentials split by auth shape, because they are not one problem

The roadmap's stated position — "secret injection delegated to the
backend's credential proxy, never via mounted files or plain env vars" —
was written before this was examined closely, and it only covers half
the ground.

- **Key-shaped auth** fits it exactly: the backend already has a
  credential mechanism that delivers a secret as an environment variable
  with no filesystem grant. Use it.
- **Subscription/OAuth auth is file-based.** Verified:
  `~/.claude/.credentials.json` exists. There is no env var that
  substitutes for it, so "never via mounted files" cannot be satisfied
  for the case that motivates this change. Refusing to support it would
  mean supporting API-key users only — precisely the users who do *not*
  need this, since a key is already injectable today.

**Two things in this decision are unmeasured, and either could overturn
its file half (noted 2026-10-29; no host here has a subscription
login):**

- **A read-only file may not survive a token refresh.** OAuth logins
  refresh their access token and rewrite the credentials file. With a
  read-only grant, an agent may work until the first expiry and then
  fail, in a way that reads as a login problem rather than a sandbox one.
- **The "no env-var form" premise may no longer hold for Claude Code.**
  `claude setup-token` issues a long-lived token for
  `CLAUDE_CODE_OAUTH_TOKEN`, which would make a subscription key-shaped,
  delivered like any other key with no file grant at all. If that holds,
  task 7.1's amendment of "never via mounted files" loses its premise
  for the agent that motivated it.

Both are measured before the file shape is built (task 4.0).

**Where a credential is requested: the user's configuration, with the
layer.** The same reasoning as Decision 1, and more forcefully: a
committed "grant my token file into the sandbox" is a statement about
every developer's machine. The request lives in the user configuration
file beside `[tools]`. The project-level `[env] forward` that already
ships is unchanged and out of scope: it names a variable the user's own
shell must hold, and devcroft never stores the value.

So file-based credentials are supported, bounded to the narrowest form
the backend can express: **a single file, read-only.** The backend
supports per-file grants, so granting the containing directory is an
implementation shortcut, not a necessity — the spec forbids it.

**Residual risk, stated rather than mitigated away:** any process in the
sandbox can read an exposed credential, including the project code the
agent is editing. That is unavoidable when the agent must run in the
same boundary as the code under edit. Narrowness (one file), read-only,
and mandatory disclosure at `up` are the mitigations; isolation from the
code under edit is not among them, and the docs must not imply it is.

### 3. Project-root binding: fail, do not auto-suffix

Two worktrees sharing a sandbox is a correctness bug, not an ergonomics
gap. The fix could be automatic — derive a suffix from the root hash so
each worktree silently gets its own sandbox — but that makes the
user-facing handle unpredictable, and `devcroft exec myproj` ambiguous
across checkouts.

Failing loudly and naming `--name` is chosen because it preserves the
property that a sandbox name means one thing, and because fan-out is
usually driven by a script (an IDE recipe, a shell loop) that can supply
a name trivially. The ergonomic cost lands on interactive users creating
a second worktree by hand, who get one clear error once.

The check is nearly free: `meta.json` already stores `project_root`.

### 4. The layer contributes `PATH` entries only, after the project's

With two environments, "which `node` wins" becomes a real question, and
the first version of this design answered it by composing two env diffs
at a fixed position. That was the wrong unit. A nix, devenv or flox
environment's diff carries `CC`, `NIX_CFLAGS_COMPILE`, `NIX_LDFLAGS`,
`PKG_CONFIG_PATH`, `PYTHONPATH` and more; laid over the project's, the
agent's layer would change the project's build.

So the layer contributes **its `PATH` entries and nothing else**:

```
 PATH in a session:
 ┌──────────────────┬──────────────────┬────────────────────┐
 │ project          │ tooling layer    │ devcroft's shell   │   first match wins
 └──────────────────┴──────────────────┴────────────────────┘
 every other variable: the project's alone, then [env.vars] as before
 grants: the layer's store root, read-only, origin tools:<provider>
```

- **The project comes first.** A `node` in the layer never shadows the
  project's `node`, so a failure never looks like the project's fault
  because of something the user added for themselves.
- **The layer's hooks never run.** Taking only `PATH`, it is resolved
  through each provider's existing hook-free route (`print-dev-env
  --json`, `shellenv --pure`, `devenv build shell`, the derived flox
  copy), so `fix-provisioning-hooks`' whole problem does not arise for
  it, and its activation script is never carried into the sandbox.
- **Shadowing is shown, not warned.** A name provided by both is not a
  degraded capability, so it does not earn an `up` warning under the
  "exactly one warning" rule. `status` lists the layer's binaries that
  the project's `PATH` shadows. This closes the Open Question the first
  version left.
- **Deterministic by construction.** The layer's entries are taken in
  the order its provider reports them and placed at one position; no
  resolution timing is involved.

**Measured (2026-10-29): the layer needs to contribute PATH entries, not
an env diff.** Composing a second environment's diff would let the
agent's layer change the project's build (`CC`, `NIX_CFLAGS_COMPILE`,
`PKG_CONFIG_PATH`, …), so the question was whether `PATH` alone is
enough. It is, for all eleven agents in `docs/agents.md`. Spike: a devenv
project holding only `jq`, from the same `devenv.lock` as the agent
samples, and a separate `buildEnv` of the eleven agents plus bash,
coreutils, findutils, grep, sed and git, from the same nixpkgs revision.
That is the shape of a devenv profile, and it has no environment of its
own to compose. Each session got `<layer>/bin` appended *after* the
project's `PATH` and nothing else from the layer. All eleven printed
their version, and all eleven reached their own vendor's API through
the proxy and were refused there for a dummy key, the same result as the
matrix, with the agent no longer in the project's environment. No grant
was added: the provider's store grant is the whole `/nix/store` root
(`capture::store_grants`), so a closure-tier layer is already readable.
That also means a layer adds no rule to `policy --render` unless one is
added deliberately, for inspectability.

What this does not show: a layer from a provider whose environment is
not store-backed (artifact tier), a layer whose own tools rely on its
env diff (none of the eleven did, since nixpkgs wraps them with absolute
runtime paths), and macOS.

The grant is rendered anyway. Where the project's provider already
grants the store root, the layer's identical rule adds no access, but it
does make the layer visible in `policy --render` under its own origin;
for a project whose provider grants no store (`swift`), it is the access
the layer needs.

### 5. Only the closure tier serves a layer; it is recorded, and goes stale

- **Closure-tier providers only** — flox, nix, devbox, devenv —
  regardless of the project's own provider. A `swift` project can carry
  a devenv layer; a `swift` layer is refused. An artifact-tier layer is
  host-linked by definition, needs grants of its own
  (`own-policy-baseline`), and is the host passthrough of option (b) one
  step removed.
- **Recorded in `Meta`.** The layer's path and its lock's fingerprint,
  the way the project environment's are. `policy --render` renders the
  layer's grant from `Meta`, never from the configuration file as it
  stands.
- **Stale like the project.** A layer whose lock changed since `up`, or a
  configuration file now naming a different layer, is reported stale by
  `status` and applied by `up --recreate`, exactly as for the project
  environment: the environment resolves once, at `up`.
- **Fleet resolves it once.** A fleet's agents share the user, so they
  share the layer, resolved once per fleet. The minimal root's
  `/workspace` rewrite does not touch it: its `PATH` entries are store
  paths, independent of where any clone lives.

### 6. Brokering: the agent speaks plain HTTP to the proxy, which adds the key and TLS

The credentials spec requires that a brokered credential never be in the
sandbox: the sandbox holds a placeholder, and the proxy substitutes the real
value on requests to the declared upstream. Building that needs the proxy to
*see* the header, and a `CONNECT` tunnel to `:443` carries TLS it cannot
read, by design (TLS interception is an explicit non-goal). So the agent
must send its API traffic in clear, and only as far as the proxy.

**Measured 2026-11-15, all eleven agents in `docs/agents.md`**, same
versions, each with a dummy key, outside any sandbox. Its proxy variables
pointed at a logging listener, and its endpoint was `http://broker.invalid`,
a name that resolves nowhere, so an agent bypassing the proxy failed at DNS
instead of leaving.

- **Every agent's endpoint can be moved** (`docs/agents.md`, "Whose
  endpoint can be moved", names the setting per agent; Codex's and
  Crush's are not the variable their SDK's convention suggests).
- **Nine send an absolute-URI request** to the proxy:
  `POST http://broker.invalid/v1/messages` with the key header in clear.
  These are Claude Code, Codex (after its WebSocket attempt), Antigravity,
  OpenCode, Copilot, Aider, Goose, Amp and Crush.
- **Two open `CONNECT broker.invalid:80`** even for an `http://` endpoint
  (Qwen Code, Cursor). Codex does the same for its WebSocket. Inside the
  tunnel each sent **plain HTTP/1.1**, key header included: there is no TLS
  on a tunnel to an `http://` endpoint, so the proxy can read it once it
  accepts the tunnel.
- Every agent's other traffic (telemetry, update checks, model catalogues)
  still went to `:443` as ordinary `CONNECT`s to other hosts, and none
  carried the key.

**Decision (the shape for devcroft's own proxy; `nono-proxy` uses a
loopback prefix instead, see the investigation below).** For a host with a
brokered credential, the proxy accepts both
forms (absolute-URI, and `CONNECT <host>:80`, read as HTTP/1.1). It
substitutes the placeholder in the declared header and sends the request on
to `https://<host>` over TLS of its own. The endpoint the agent is given is
the **real upstream host with an `http://` scheme** (for example
`ANTHROPIC_BASE_URL=http://api.anthropic.com`). That has three consequences:

- the allowlist decides on the same name it does today, so brokering grants
  nothing new;
- an agent that bypassed the proxy would hold only a placeholder, and in a
  `"deny"` sandbox it has no route anyway;
- no new listener is needed.

**Why this is not TLS interception.** The agent never negotiates TLS with
anyone, so nothing is decrypted, re-signed or trusted. The clear leg is
sandbox → proxy, which on Linux crosses the unix-socket bridge whose 0700
directory is already the token's boundary (`add-egress-proxy`). On macOS it
is the host's loopback. That is a real difference, and a placeholder is
what keeps a loopback observer from learning anything.

**Shape of the request.** A brokered credential names:

- the upstream host;
- the header that carries it;
- the variable the sandbox gets;
- optionally the placeholder, since a client may check a key's shape
  before sending it (Copilot rejects a classic `ghp_` token locally; spec
  scenario "A placeholder must survive client-side validation").

The proxy replaces the placeholder *inside* the header's value, so
`Bearer <placeholder>` works without knowing the scheme. A request to the
host whose header lacks the placeholder is forwarded unchanged, never given
the key.

**One request per connection.** A brokered connection carries one request
and is closed after the response (`Connection: close`). An HTTP/1.1
keep-alive connection would otherwise carry later requests the proxy never
parsed, and only the first would be rewritten. Every agent measured reopens
cleanly. Responses, streamed (SSE) or not, are copied back untouched.

**What brokering does not protect.**

- **Cursor** trades the key for a session token on its first call. That
  token is returned into the sandbox, so brokering keeps the long-lived key
  out but not access. `docs/agents.md` states it.
- **Codex** gets the key on its WebSocket upgrade request, which is the
  same HTTP/1.1 request head, so it is covered. Rewriting the WebSocket
  frames after the upgrade is not needed, since they carry no credential.
  This is not yet measured against a broker that accepts the upgrade.
- A **subscription credential** (a file) cannot be brokered this way,
  since its refresh flow writes the file. The spec's "cannot be brokered"
  scenario refuses it rather than downgrading.

**Whose proxy does it: `nono-proxy`, investigated 2026-11-15.** The owner
asked for a serious look before deciding (task 4b.0). `nono-proxy` 0.82.0
(the current release; devcroft is on nono 0.77) was built into a harness
and run against the same agents.

*What it does, measured.*

- **Injection works, on its own request form, not the one above.** A route
  is a path prefix on the proxy's own port:
  `ANTHROPIC_BASE_URL=http://127.0.0.1:<port>/anthropic`. The agent holds a
  placeholder (the proxy's session token, optionally shaped by
  `phantom_format`, e.g. `ghp_{}`), and the proxy swaps it for the real
  value. A wrong or missing placeholder got `401` from the proxy and never
  reached the upstream. Two requests on one kept-alive connection were both
  rewritten.
- **Real agents:** Claude Code, Codex, Qwen Code, OpenCode and Antigravity
  each delivered the real value to the upstream, in `x-api-key`,
  `Authorization: Bearer` and `x-goog-api-key`. Codex's WebSocket upgrade
  got a clean `501` and it fell back to `POST`, which worked.
- **Cursor does not work.** It ignores `NO_PROXY`, so it sends
  `CONNECT 127.0.0.1:<port>` to the proxy, and `nono-proxy` refuses a
  tunnel to itself (`403`). Brokering for Cursor was already weak, since
  the session token it exchanges stays in the sandbox.
- **It refuses the form this decision first chose.** An absolute-URI
  request, or a `CONNECT`, addressed to a route's upstream host gets
  `403`, by design: a route's host is reachable only through its prefix.
  So adopting it means the loopback-prefix form, and "no new listener"
  still holds, since the prefix lives on the proxy's own port.
- **Credential sources** are more than devcroft has: `env://`, `file://`,
  the OS keyring, `op://`, and an OAuth2 client-credentials exchange.
- **The name is decided before DNS**, as in devcroft's proxy, so a refused
  name is not leaked to the resolver.

*What has to be set, because its defaults are not devcroft's.*

- **`strict_connect_auth = true`.** By default an unauthenticated `CONNECT`
  to an allowed host succeeds (measured: `200`). That is the open-relay
  defect devcroft fixed in `add-egress-proxy` 4a. With the flag it is
  `407`. The bridge must then present `Basic nono:<token>` or `Bearer`,
  since its current `Basic <token>:` form is refused (measured). That is
  a one-line change.
- **A route forwards whether or not its host is on the allowlist.**
  devcroft has to require the allowlist entry itself (task 4b.1).
- **TLS interception stays off** (`intercept_ca_dir = None`), by decision.
  That also keeps off `oauth_capture`, WebSocket tunnelling and endpoint
  rules on `CONNECT` hosts, all of which need it.

*What it costs.*

| | crates (normal deps, host target) |
|---|---|
| devcroft today (nono 0.77) | 335 |
| nono 0.82, nothing else | 280 |
| nono 0.82 + a TLS client of devcroft's own (`rustls`) | 283 |
| nono 0.82 + `nono-proxy` 0.82 | 375 |

- `nono-proxy` requires the matching nono, so adopting it includes the
  0.77 → 0.82 upgrade. devcroft **compiles unchanged** against 0.82. The
  boundary tests were not run on it, and `Cargo.toml` says an upgrade is
  scheduled work with the full suite.
- nono 0.82 dropped its HTTP client, so `rustls` is no longer in the tree
  for free. The earlier claim here that the own-proxy route "adds no
  crate" is true only on 0.77. After the upgrade it is 3.
- The 95 crates `nono-proxy` adds include the AWS SDK (21), SPIFFE with
  `tonic`/`prost`, `rcgen` and `x509-parser`. None of them is behind a
  feature. devcroft would link them and use none.

*How mature it is.*

- First published 2026-02-23, 93 releases, roughly one every two weeks.
- About 48,000 lines and 892 tests in 0.82.0, up from 33,000 lines and 575
  tests in 0.77.0 two months earlier.
- `clippy::unwrap_used` is denied, and there are 4 `unsafe` sites.
- Between 0.77 and 0.82 the surface devcroft would use (`ProxyConfig`,
  `RouteConfig`, `start`, `ProxyHandle`) changed by addition only: 27
  items added, none removed or renamed.

It is well tested and widely featured, and it is still growing fast.

*What it would change in devcroft.* Its `CONNECT` filtering overlaps
devcroft's own proxy entirely (same `nono::HostFilter`). Running both is
two proxies per sandbox, so adoption means **replacing**
`src/proxy/server.rs` (1,105 lines), not adding beside it. That changes
observable behaviour: an allowlist refusal becomes `403` where devcroft
answers `502` naming the host, which `tests/loopback_any_port.rs`,
`tests/egress_proxy_e2e.rs` and `docs/agents.md` assert. The spawn, the
unix-socket bridge, the namespace relay and the provisioning proxy stay
devcroft's.

*Recommendation, revised.* **Adopt `nono-proxy`, as a replacement, in
three steps that can each be stopped:**

1. upgrade nono to 0.82 with the full suite;
2. swap the proxy server behind the existing spawn and bridge, with
   `strict_connect_auth`, keeping every egress test (their expected status
   changes, their meaning does not);
3. add brokered routes (4b.1 onward).

The earlier recommendation, devcroft's own proxy, rested on two things the
measurement removed: `rustls` being free, and `nono-proxy` being unable to
serve the agents. It serves ten of eleven, and what it brings (credential
sources, a tested phantom scheme, upgrade handling, rate limits) is work
devcroft would otherwise write and maintain alone. The price is 95 crates,
most of them unused, and a second crate whose releases devcroft must track
in lockstep with nono.

Not measured: a streamed (SSE) response through a route, macOS, the suite
on nono 0.82, and the six agents not re-run through it (Copilot, Aider,
Goose, Amp, Crush were measured only against a plain loopback listener,
which is the same request form).

## Risks / Trade-offs

- **A user layer is per-machine** → Mitigation: it must be locked, is
  recorded in `Meta`, and is named by `up` and `status`; the project's
  environment, which is what the guarantee covers, is untouched apart
  from `PATH` entries appended after its own. Two developers' sandboxes
  of one project differ only in what is on `PATH` after the project's
  tools.

- **devcroft gains a user configuration file** → Mitigation: it holds
  the layer and credential requests and nothing that changes how a
  project's own manifest is read. Every other key stays in
  `devcroft.toml`, so "what does this project do" is still answered by
  the repository.

- **Supporting file-based credentials weakens a stated roadmap
  position** → Mitigation: the position is amended explicitly in
  `docs/decisions.md` with the property that made it unachievable
  (subscription auth has no env-var form), following that file's own
  convention that a rejection whose premise stops holding is revisited
  rather than defended.

- **A narrow file grant still exposes a long-lived token to project
  code** → Mitigation: bounded and disclosed, never implicit. Users who
  cannot accept it should not grant it; the docs say so plainly instead
  of implying safety.

- **BREAKING: a second worktree that worked today now fails** →
  Mitigation: it "worked" by silently serving the wrong code. The error
  names both roots and the fix. Worth a README note, since anyone who
  had two worktrees up was affected without knowing.

- **macOS is unmeasured** → Mitigation: an explicit verification task
  rather than an assumption. Desktop agent fan-out is mac-heavy, so a
  Linux-only result would materially limit this change's value.

## Migration Plan

Additive except for the project-root check. With no user configuration
file and no `--tools`, every sandbox behaves exactly as before, and
`policy --render` output is unchanged — which is the regression test.

The project-root check is the one behavior change on existing setups. It
fails closed, and the failure is actionable, so no data migration is
needed; sandboxes recorded before this change already carry the
`project_root` the check reads.

Rollback is removal of the check and of the user configuration file's
reader; no persisted state becomes unreadable, though a `Meta` recording
a layer would then carry a field nothing reads.

## Open Questions

- ~~Whether a binary shadowed by the tooling layer should be reported at
  `up` or silently lose.~~ Neither: shown by `status` (Decision 4).
- Turning a configured layer off for one invocation. See the proposal;
  deferred to use.
- Which credential shapes beyond the two here are worth naming (for
  example, a helper command that prints a token to stdout). Deferrable:
  additive, and the file/env split covers every agent examined.
