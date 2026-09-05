//! `up` (task 4.2): design.md decision 1's supervisor sequence — create
//! the listener, resolve the environment, compile the policy, spawn the
//! keeper under nono with the listener fd inherited, wait for it to come
//! up. Idempotent by default; `--recreate` forces a full teardown and
//! re-resolution.

use std::fmt;
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::config::Manifest;
use crate::policy;
use crate::provider::{ProviderError, ProviderKind, Resolution, ServiceSupport};

use super::hooks;
use super::state::{self, Health, StatePaths};
use super::terminate::GRACE_PERIOD as TERMINATE_GRACE_PERIOD;

const KEEPER_START_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpOutcome {
    /// A healthy keeper already existed; nothing was done (spec: "Up on a
    /// healthy sandbox").
    AlreadyUp,
    /// State existed but the keeper was dead/unresponsive; it was cleared
    /// and a fresh keeper started (spec: "Recovery after host reboot").
    Recovered,
    /// No prior state; started clean.
    Started,
    /// `--recreate`: any existing keeper was torn down and everything was
    /// re-resolved and recompiled from scratch.
    Recreated,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UpOptions {
    pub recreate: bool,
    /// Lifecycle spec's "Hooks run inside the boundary" requirement: a
    /// failing hook fails `up` unless this is set, in which case hooks
    /// (both `post_create` and `post_start`) are not run at all rather
    /// than run-and-ignored.
    pub skip_hooks: bool,
}

#[derive(Debug)]
pub enum UpError {
    State(io::Error),
    /// CLAUDE.md's error contract layer `config` (exit 2): the compiled
    /// policy cannot be enforced as written — today this only ever means
    /// `policy::CapabilitySetError::DenyOverlapsAllow`, a manifest (or
    /// baseline) deny rule Landlock cannot express because a broader
    /// allow already covers it (`use-nono-library` task 2). A config-time
    /// problem, not a provider/backend/keeper one, so it gets its own
    /// layer rather than being folded into `Keeper`.
    Policy(String),
    /// Also CLAUDE.md's `config` layer (exit 2), but for a manifest that
    /// cannot be *realized on this host* rather than one whose policy
    /// cannot be compiled — today, a project path deep enough that the
    /// service supervisor's socket would exceed the OS `sun_path` limit.
    /// Same layer and exit code as [`UpError::Policy`], kept separate
    /// because the cause and the fix are unrelated.
    Config(String),
    Provider(ProviderError),
    /// CLAUDE.md's error contract names `backend` as its own layer
    /// (exit code 4) — unreachable before `add-hardened-tier`, since the
    /// process tier's only backend (nono) failures always land in
    /// `Keeper` (a missing/failing `nono wrap` invocation). The hardened
    /// tier's hard failures belong here instead: `hardened` requested on
    /// a host that cannot provide it (macOS, or Linux without a working
    /// `runsc`) is a `Backend` error, never a silent downgrade to
    /// `process`.
    Backend(String),
    Keeper(String),
    /// CLAUDE.md's error contract names `ssh` as its own layer; key
    /// generation/resolution failures (task 6.1) land here rather than
    /// `Keeper`, even though both currently surface through the same
    /// exit code (keeper/connection, 5) until task group 7 wires up a
    /// dedicated CLI.
    Ssh(String),
}

impl From<io::Error> for UpError {
    fn from(e: io::Error) -> Self {
        UpError::State(e)
    }
}

impl fmt::Display for UpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpError::State(e) => write!(f, "state: {e}"),
            UpError::Policy(msg) | UpError::Config(msg) => write!(f, "config: {msg}"),
            UpError::Provider(e) => write!(f, "provider: {e}"),
            UpError::Backend(msg) => write!(f, "backend: {msg}"),
            UpError::Keeper(msg) => write!(f, "keeper: {msg}"),
            UpError::Ssh(msg) => write!(f, "ssh: {msg}"),
        }
    }
}

impl std::error::Error for UpError {}

pub fn up(
    manifest: &Manifest,
    project_root: &Path,
    opts: &UpOptions,
) -> Result<UpOutcome, UpError> {
    // `env.provider` is already validated and normalized by `config::parse`
    // (the only place a `Manifest` is constructed), so this only ever
    // dispatches to a real implementation. Selecting the provider is the
    // *whole* of what this function does that `up_with_provider` does not —
    // see that function's doc for why the split is drawn exactly here.
    let provider = ProviderKind::from_name(&manifest.env.provider).map_err(UpError::Provider)?;
    up_with_provider(manifest, project_root, opts, &provider)
}

/// `up`, with the provider supplied rather than selected from the manifest.
///
/// **This holds `up`'s entire body, not just the part after resolution**,
/// and that is the requirement rather than a convenience: the lifecycle
/// lock, the health/recreate decision, the deny-overlap validation, the
/// mount-isolation probe, listener-before-restriction ordering and hook
/// ordering all live in here. A seam drawn *after* those would let an
/// injected caller skip them and still call itself a test of `up`
/// (`provider-injection-seam`: "the injected path is the enforcement
/// path"). One function with two callers cannot drift; two functions that
/// must be kept in step can, and this project designs that class of bug out
/// rather than watching for it — the same reasoning that makes
/// `resolved_grants` and `to_capability_set` share one resolver.
///
/// **Reachable only through the `test-support` feature.** The function is
/// `pub` because a `pub(crate)` item cannot be re-exported, but its module
/// is crate-private, so in a default build there is no path to it from
/// outside — the ordinary facade pattern. `crate::test_support` re-exports
/// it, and that module does not exist unless the feature is on. The feature
/// carries this rather than `cfg(test)` because the integration suite
/// compiles this crate as an ordinary dependency, where `cfg(test)` is
/// false.
pub fn up_with_provider(
    manifest: &Manifest,
    project_root: &Path,
    opts: &UpOptions,
    provider: &dyn crate::provider::ProviderEntry,
) -> Result<UpOutcome, UpError> {
    let paths = StatePaths::new(&manifest.sandbox.name)?;
    // Held for the rest of this function — see `acquire_lifecycle_lock`'s
    // doc for the concurrent-`up` race this closes. Every `?` and early
    // return below drops `_lock` (and so releases it) on the way out,
    // same as any other RAII guard.
    let _lock = state::acquire_lifecycle_lock(&paths.lifecycle_lock)?;

    // **A state directory belongs to the project root it was created for.**
    //
    // `meta.json` has always recorded `project_root`; nothing ever compared
    // it, so two git worktrees of one repo — which share a committed
    // `devcroft.toml`, and therefore a sandbox *name* — silently shared one
    // sandbox. The second `up` adopted the first's keeper, and an agent
    // working in worktree B ran against worktree A's environment and grants.
    // Detection of an existing bug rather than new bookkeeping
    // (`add-agent-workload` task group 1).
    //
    // **Before the health decision below, and that placement is the fix.**
    // The first version of this sat after it and never fired for the case it
    // exists to catch: a *healthy* sandbox returns `AlreadyUp` early, and
    // adopting a healthy sandbox from the wrong root is precisely the silent
    // failure. Caught by the worktree test, not by review. "Does this state
    // dir belong to me" has to be answered before "should I adopt it".
    // Brokered credentials resolve here — the earliest point at which the
    // manifest is known and nothing has been started (`adopt-nono-proxy` task
    // 3.2). A route whose secret is absent must fail *now*, not at the agent's
    // first request: deferred, it surfaces as an upstream authentication error
    // inside a sandbox, which is the least diagnosable place it could appear
    // and looks like the agent's fault rather than a missing export.
    let brokered = resolve_brokers(&manifest.brokers)?;

    if let Some(meta) = state::read_meta(&paths.meta)?
        && meta.project_root != project_root.to_string_lossy()
    {
        return Err(UpError::Config(format!(
            "sandbox '{name}' already exists for a different project root\n  \
             recorded: {recorded}\n   current: {current}\n\
             two worktrees of one repository share a committed `devcroft.toml`, \
             and so share a sandbox name; give this one its own with \
             `devcroft up --name <other>`",
            name = manifest.sandbox.name,
            recorded = meta.project_root,
            current = project_root.display(),
        )));
    }

    let outcome = if opts.recreate {
        // `terminate_and_wait` reads and identity-verifies each pidfile
        // itself (`is_same_process` — a resurrected unrelated process
        // must never be signaled on the strength of a bare pid number
        // read back off disk), and no-ops cleanly if either is absent or
        // stale, so no separate `health()` check or liveness guard is
        // needed here first.
        state::terminate_and_wait(&paths.pidfile, TERMINATE_GRACE_PERIOD);
        // The egress proxy is a separate process from the keeper (see
        // `crate::proxy`'s module doc for why it must be), so tearing
        // down the keeper above says nothing about it. `--recreate`
        // means "redo everything from the manifest", and the manifest's
        // `network.allow` may have changed — an old proxy is running
        // with whatever allowlist it was spawned with baked into its own
        // env, so reusing it here would silently ignore that change
        // until the next full `down`.
        state::terminate_and_wait(&paths.proxy_pidfile, TERMINATE_GRACE_PERIOD);
        state::clear_runtime_state(&paths)?;
        UpOutcome::Recreated
    } else {
        match state::health(&paths)? {
            Health::Healthy(_) => return Ok(UpOutcome::AlreadyUp),
            Health::Stale(_) => {
                state::clear_runtime_state(&paths)?;
                UpOutcome::Recovered
            }
            Health::None => UpOutcome::Started,
        }
    };

    // Resolved before anything else touches the host: `hardened` on a
    // host that cannot provide it must fail before state-dir creation or
    // provider resolution do any work, never as a silent downgrade to
    // `process` (CLAUDE.md's error contract; add-hardened-tier's
    // "Tier resolution fails loudly" design decision).

    // Host-side, before any restriction applies (design.md decision 2):
    // the resolved environment and its store grants are captured now,
    // once, and folded into the profile/bundle the sandbox will be
    // confined to. This step is identical for both tiers — the two-phase
    // execution model (CLAUDE.md) is backend-generic, not just
    // process-tier.
    let resolution = provider.resolve(project_root).map_err(UpError::Provider)?;

    // Recorded now so `status` (task 4.3) can later tell whether the
    // environment has drifted since this `up`, and which concrete
    // backend it resolved to, without needing the manifest or project
    // root passed back in — the keeper itself is never told its own
    // state dir, so it can't answer either.
    let env_fingerprint = provider
        .fingerprint(project_root)
        .map_err(UpError::Provider)?;

    // ssh spec: "0700 state dir" — set on creation (mode only applies to
    // dirs `create_dir_all`/`DirBuilder` actually create, so an
    // already-existing root from before this existed is left alone, same
    // as `create_dir_all`'s own idempotency).
    //
    // Created here, immediately before the first write into it, rather
    // than at the top of `up`: it used to be created before provider
    // resolution, so every `up` that failed at layer `provider` — a
    // missing `.flox/`, an unlocked flake, a devbox project needing
    // `devbox install` — left an empty state directory behind for a
    // sandbox that never existed. Found by adversarial review, by
    // noticing this repo's own state dir had accumulated 23 of them from
    // test runs. Failures *after* this point still leave a directory,
    // and should: by then it holds real state, and `rm` is the cleanup.
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&paths.root)?;

    // The egress proxy is a separate, permanently unsandboxed process
    // (see `crate::proxy`'s module doc), spawned host-side here rather
    // than inside `up_process` so its port is known in time to be
    // recorded in `Meta` alongside everything else `status` needs and
    // the keeper can't answer. `policy::compile` alone (no keeper-exe or
    // provider-grant folding — neither affects the network fields) is
    // enough to know whether one is wanted at all.
    // Port and token always travel together — a proxy without a
    // recorded token would be indistinguishable from one predating
    // authentication entirely, so the pair is one `Option`, not two, all
    // the way to where each half is consumed.
    let proxy = if policy::compile(manifest).wants_egress_proxy() {
        Some(ensure_egress_proxy(
            &paths,
            &manifest.network.allow,
            &brokered,
        )?)
    } else {
        // A manifest that no longer wants filtering (`network.allow` was
        // emptied, or `network.default` changed to `"allow"`) leaves a
        // proxy from a previous `up` running with nothing left to filter
        // for — same "no orphan" reasoning `clear_runtime_state` already
        // applies to the keeper's own pidfile.
        stop_orphaned_egress_proxy(&paths)?;
        None
    };

    // devcroft's own shell dependency, resolved host-side out of the
    // closure that was just materialized — see `crate::shell`. Resolved
    // here rather than inside `up_process` so the answer is recorded in
    // `Meta` with everything else the client side needs later, and so a
    // sandbox that has no shell at all fails before any state that
    // implies a working one is written.
    let shell =
        crate::shell::resolve(&resolution.env, &resolution.read_only_grants).ok_or_else(|| {
            UpError::Provider(crate::provider::ProviderError::ResolutionFailed(
                "no POSIX shell found in this environment or its closure, and devcroft \
             needs one for `shell`, SSH login sessions and services; add one to the \
             environment manifest (e.g. `flox install bash`)\nsearched: the store \
             entries on the resolved environment's PATH, then their closure \
             requisites"
                    .to_string(),
            ))
        })?;

    // The provider's own grants plus the store path the resolved shell
    // lives in, folded together *here* rather than at compile time so
    // `Meta` and the compiled profile cannot disagree: `policy --render`
    // renders from `Meta`, and a rule reaching the backend that
    // `--render` cannot show is exactly what the policy capability
    // forbids. (The shell grant is redundant while `/nix/store` is
    // granted wholesale; `add-mount-isolation` is what makes it load-
    // bearing, and a grant that appears only then would appear
    // unexplained.)
    let mut provider_grants = resolution.read_only_grants.clone();
    if let Some(grant) = &shell.grant
        && !provider_grants.contains(grant)
    {
        provider_grants.push(grant.clone());
    }

    state::write_meta(
        &paths.meta,
        &state::Meta {
            project_root: project_root.to_string_lossy().into_owned(),
            env_fingerprint,
            read_only_grants: provider_grants.clone(),
            resolved_backend: RESOLVED_BACKEND.to_string(),
            // What the provider declared, recorded so `status` can
            // report a service the supervisor never accepted — see
            // `Meta::declared_services`.
            declared_services: resolution
                .services
                .declared()
                .iter()
                .map(|s| s.name.clone())
                .collect(),
            ran_activation_hook: resolution.ran_activation_hook,
            shell: Some(shell.path.to_string_lossy().into_owned()),
            proxy_port: proxy.as_ref().map(|(port, _)| *port),
            proxy_token: proxy.as_ref().map(|(_, token)| token.clone()),
        },
    )?;

    up_process(
        provider,
        manifest,
        project_root,
        &paths,
        opts,
        outcome,
        &resolution,
        &shell,
        &provider_grants,
        proxy,
    )
}

/// Starts the egress proxy if none from a previous `up` is still alive,
/// or reuses the live one's already-recorded port and token. Reuse only
/// ever applies across a `Health::Stale` recovery — `--recreate` already
/// tears the old proxy down before this runs (see the top of [`up`]), so
/// a live pidfile here can only mean "the keeper died independently of
/// the proxy," never "the manifest changed and this is stale." A proxy
/// found alive but with no recorded token (only possible from a
/// `meta.json` written before authentication existed) is **not**
/// eligible for reuse — it predates a token entirely, so there is
/// nothing correct to authenticate against, and it is replaced rather
/// than trusted.
/// Resolves every `[[broker]]`'s secret, or fails naming the route.
///
/// Layer `provider`, matching the error contract: this is an environment
/// precondition devcroft could not satisfy, the same class as a provider whose
/// tooling is missing — not a malformed manifest (`config`) and not a runtime
/// fault (`keeper`).
///
/// The value is carried as `Zeroizing` from here on, so a failure between this
/// point and the proxy handing it upstream does not leave it in a dropped
/// buffer.
type ResolvedBroker = (crate::config::Broker, zeroize::Zeroizing<String>);

fn resolve_brokers(brokers: &[crate::config::Broker]) -> Result<Vec<ResolvedBroker>, UpError> {
    let mut out = Vec::with_capacity(brokers.len());
    for b in brokers {
        match crate::proxy::secret::resolve(&b.secret) {
            Ok(value) => out.push((b.clone(), zeroize::Zeroizing::new(value))),
            Err(e) => {
                return Err(UpError::Provider(
                    crate::provider::ProviderError::ResolutionFailed(format!(
                        "broker `{}` cannot be brokered: {e}\n  \
                     the credential stays on the host and never enters the sandbox, so `up` \
                     refuses rather than starting a sandbox that would fail at its first request",
                        b.provider
                    )),
                ));
            }
        }
    }
    Ok(out)
}

fn ensure_egress_proxy(
    paths: &StatePaths,
    allow: &[String],
    brokers: &[ResolvedBroker],
) -> Result<(u16, String), UpError> {
    // `is_same_process`, not `is_process_alive`: a resurrected unrelated
    // process at a reused pid would otherwise pass as "our proxy is
    // still running", skip spawning a real one, and leave egress
    // completely unfiltered — a silent security downgrade, and arguably
    // worse than the sibling bug in `terminate_and_wait` (signaling the
    // wrong process is at least noisy).
    if let Some((pid, start_time)) = state::read_pidfile(&paths.proxy_pidfile)?
        && state::is_same_process(pid, start_time)
        && let Some(meta) = state::read_meta(&paths.meta)?
        && let (Some(port), Some(token)) = (meta.proxy_port, meta.proxy_token)
    {
        return Ok((port, token));
    }
    let exe = keeper_exe()?;
    let (pid, port, token) = crate::proxy::spawn(&exe, paths, allow, brokers)
        .map_err(|e| UpError::Keeper(format!("starting egress proxy: {e}")))?;
    state::write_pidfile(&paths.proxy_pidfile, pid)?;
    Ok((port, token))
}

fn stop_orphaned_egress_proxy(paths: &StatePaths) -> io::Result<()> {
    state::terminate_and_wait(&paths.proxy_pidfile, TERMINATE_GRACE_PERIOD);
    let _ = std::fs::remove_file(&paths.proxy_pidfile);
    Ok(())
}

/// Resolves `isolation` to the concrete backend string `status`/`meta.json`
/// The concrete backend recorded in `meta.json` and shown by `status`.
///
/// A constant since `remove-gvisor-backend`: there is one backend, so
/// there is nothing to resolve and nothing that can fail. Kept as a named
/// value rather than inlined because `meta.json` records it and `status`
/// prints it, and a future second backend (the criteria are in that
/// change's design.md, G5's trait is still here) reintroduces the choice
/// at exactly this point.
pub const RESOLVED_BACKEND: &str = "process";

/// The `process` tier's supervisor sequence — today's `up`, unchanged in
/// every particular, just extracted so [`up`] can dispatch to it or to
/// [`up_hardened`] from one shared prefix (state dir, provider
/// resolution, meta).
// `shell` and `provider_grants` are passed in rather than derived here,
// even though both could be: they are computed once in [`up`] so that
// `Meta` and the compiled profile cannot disagree about the shell's
// grant, and recomputing either here would reintroduce exactly the
// divergence that guarantee exists to prevent. Same allow, and same
// reason, as `spawn_keeper` below.
#[allow(clippy::too_many_arguments)]
fn up_process(
    // Passed down rather than re-derived from `manifest.env.provider`: this
    // is the third of the provider entry points `up` uses (rule
    // attribution), and a seam that covered resolution and fingerprinting
    // but let this one fall back to a name lookup would attribute an
    // injected row's grants to whatever the manifest happened to say.
    provider: &dyn crate::provider::ProviderEntry,
    manifest: &Manifest,
    project_root: &Path,
    paths: &StatePaths,
    opts: &UpOptions,
    outcome: UpOutcome,
    resolution: &Resolution,
    shell: &crate::shell::ResolvedShell,
    provider_grants: &[String],
    proxy: Option<(u16, String)>,
) -> Result<UpOutcome, UpError> {
    // The keeper binary itself must be readable+executable inside the
    // boundary it's about to apply to itself — no baseline group can know
    // where *this build* of devcroft lives. Compiled as a rule with an
    // origin (`with_keeper_exe_grant`) rather than appended after the
    // fact, so it shows up in `policy --render`/`why` like every other
    // grant (own-policy-baseline task 6.1).
    let exe = keeper_exe()?;
    let exe_dir = exe
        .parent()
        .ok_or_else(|| io::Error::other("devcroft executable path has no parent directory"))?;
    let compiled = policy::compile(manifest)
        .with_keeper_exe_grant(exe_dir.to_string_lossy().into_owned())
        .with_provider_grants(provider.static_name(), provider_grants);
    // `proxy_port` was already spawned/reused and recorded in `Meta` by
    // the caller (`up`), before this function ever ran — see its own
    // module-level comment for why the proxy has to exist independently
    // of anything computed here.
    let compiled = match proxy {
        Some((port, _)) => compiled.with_proxy_port(port),
        None => compiled,
    };
    // The service supervisor binds its own control socket *inside* the
    // sandbox, and on macOS that needs an explicit grant: Seatbelt treats
    // AF_UNIX `bind` as network activity, so `network.default = "deny"`
    // refuses it even though the socket sits in the granted project root
    // (`add-macos-unix-socket-scoping`; measured — every declared service
    // died with `bind: operation not permitted` and the supervisor never
    // started). Folded in here rather than in `prepare_services` below,
    // because the compiled policy and its plan are built before that runs
    // and the keeper restricts itself from the plan.
    //
    // The condition mirrors `prepare_services`' own gate deliberately: a
    // sandbox that will not start services must not carry a grant for a
    // socket nothing will create.
    let compiled = if !opts.skip_hooks && !resolution.services.declared().is_empty() {
        compiled.with_unix_socket_bind(
            crate::services::socket_path(project_root, &manifest.sandbox.name)
                .to_string_lossy()
                .into_owned(),
            policy::Origin::Baseline,
        )
    } else {
        compiled
    };
    let plan = compiled.to_capability_plan();
    // Validated host-side, before anything is created — the keeper (task
    // group 4) re-derives the identical `CapabilitySet` from the same
    // plan right before self-restricting, but doing it here too means a
    // `CapabilitySetError::DenyOverlapsAllow` fails `up` at the `config`
    // layer immediately, before a listener exists or a process spawns,
    // rather than surfacing as an opaque keeper-startup failure.
    plan.to_capability_set(project_root)
        .map_err(|e| UpError::Policy(e.to_string()))?;

    // The same resolved grants Landlock's own `CapabilitySet` above was
    // just built from — `resolved_grants` and `to_capability_set` share
    // one resolver (`policy/capability_set.rs`) precisely so the mount
    // view built from this can never diverge from what Landlock grants.
    let mount_grants = plan
        .resolved_grants(project_root)
        .map_err(|e| UpError::Policy(e.to_string()))?;

    // Mount isolation fails closed on Linux (design.md M4: "does not
    // fall back to the host's namespace") but degrades like network
    // isolation on every other platform — the two are not the same
    // claim, and treating them alike was a regression found by
    // adversarial review, not a deliberate scope decision recorded
    // anywhere. M4's own reasoning is specifically about a Linux host
    // that *should* have unprivileged user namespaces but doesn't, for
    // a host-specific reason (an old kernel, a restrictive container) —
    // the same "should normally work" framing `netns`'s own degrade
    // already uses below. macOS is a structurally different case: there
    // is no Linux namespace primitive to fail closed *about*, Seatbelt
    // already provided a real, working boundary before this change
    // existed, and failing every macOS `up` outright would be removing
    // that boundary entirely rather than tightening it. Probed and
    // decided *here*, before any listener exists or state is written,
    // for the identical reason the deny-overlap check above runs before
    // anything is created.
    let isolate_filesystem = match (
        cfg!(target_os = "linux"),
        matches!(crate::fleet::mount::probe(&exe), Ok(true)),
    ) {
        (_, true) => true,
        (true, false) => {
            return Err(UpError::Backend(
                "this host cannot create unprivileged mount namespaces, which every \
                 sandbox's filesystem view now requires (see `devcroft doctor`)"
                    .to_string(),
            ));
        }
        (false, false) => {
            // What the fallback actually costs depends on this sandbox's
            // own network mode, so the warning says which case it is
            // rather than asserting the worse one unconditionally. The
            // unix-socket half was measured on macOS 15.7.4 against a
            // real nix daemon socket (add-macos-unix-socket-scoping task
            // 0): Seatbelt classifies unix-socket connect() as
            // network-outbound, so a deny-default sandbox refuses it
            // (EPERM) with no mount view involved. Claiming it "remains
            // reachable" there was simply false, and this is the first
            // thing a macOS user sees.
            let deny_default = plan.network_block || plan.network_proxy_port.is_some();
            let residual = if deny_default {
                "this sandbox's `network.default = \"deny\"` still denies \
                 connect() to any unix socket it was not granted, on the network \
                 axis instead — but nothing narrows what the sandbox can *see*"
            } else {
                "and because this sandbox does not set `network.default = \
                 \"deny\"`, a world-accessible unix socket outside the compiled \
                 policy remains reachable"
            };
            eprintln!(
                "devcroft: warning: mount isolation is unavailable on this platform \
                 (fallback: this sandbox is confined by Seatbelt alone, the same \
                 boundary every sandbox had before add-mount-isolation; {residual} \
                 — see `devcroft doctor` and docs/known-gaps.md)"
            );
            false
        }
    };

    // Kept for inspection/debugging parity with what `down` has always
    // promised ("down must keep compiled policy") — no longer a nono
    // profile nono-cli consumes (use-nono-library task 4.4), just
    // devcroft's own compiled-policy dump.
    std::fs::write(
        &paths.profile,
        serde_json::to_string_pretty(&plan).expect("CapabilityPlan serialization is infallible"),
    )?;

    // Listener created BEFORE restriction (CLAUDE.md's listener-before-
    // restriction ordering, proven by the task 1.1/1.2 spike): its fd
    // survives the exec below, which is the only reason the socket stays
    // reachable once the keeper can no longer widen its own boundary.
    let _ = std::fs::remove_file(&paths.socket); // a stale file would fail bind()
    let listener = UnixListener::bind(&paths.socket)?;
    // 0600, for the same belt-and-suspenders reason the ssh socket below
    // has always had it. This used to be left at whatever umask produced
    // (0755 here), protected only by the 0700 root — which is real, but
    // is the *only* thing protecting it, and `up`'s own state-dir
    // creation above notes that a root existing from before that mode
    // was set keeps its old permissions. Asymmetric in the wrong
    // direction, too: this is the spawn protocol, so it is the more
    // sensitive of the two sockets, not the less. Found by adversarial
    // review; no spec required it, which was itself the gap.
    std::fs::set_permissions(&paths.socket, std::fs::Permissions::from_mode(0o600))?;
    clear_cloexec(listener.as_raw_fd())?;

    // The ssh server's socket (ssh spec, task 6.1): same listener-before-
    // restriction reasoning as the control socket above, plus its own
    // mode-0600 requirement (belt-and-suspenders alongside the 0700 root
    // — either alone already blocks every other user).
    let _ = std::fs::remove_file(&paths.ssh_socket);
    let ssh_listener = UnixListener::bind(&paths.ssh_socket)?;
    std::fs::set_permissions(&paths.ssh_socket, std::fs::Permissions::from_mode(0o600))?;
    clear_cloexec(ssh_listener.as_raw_fd())?;

    // Both key materials are generated/resolved host-side because the
    // keeper cannot read either back off disk itself once sandboxed —
    // everything under `policy::DEVCROFT_DATA_DIR` (this whole state
    // dir included) is baseline-denied, even to the keeper's own
    // process. `spawn_keeper` hands both down as env vars instead (see
    // `ssh::start_from_env`).
    let (client_private_path, client_public_path) = state::client_key_paths()?;
    let client_key = crate::ssh::ensure_client_keypair(&client_private_path, &client_public_path)
        .map_err(|e| UpError::Ssh(e.to_string()))?;
    let host_key = crate::ssh::generate_host_key(&paths.ssh_host_key)
        .map_err(|e| UpError::Ssh(e.to_string()))?;
    let host_key_pem = host_key
        .to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .map_err(|e| UpError::Ssh(e.to_string()))?;
    let authorized_key_pem = client_key
        .public_key()
        .to_openssh()
        .map_err(|e| UpError::Ssh(e.to_string()))?;

    // `Some(name)` means "start services, using this sandbox's artifact
    // subdirectory"; `None` means there are none to start.
    let services = prepare_services(
        project_root,
        &manifest.sandbox.name,
        resolution,
        &shell.path,
        opts,
    )?
    .then_some(manifest.sandbox.name.as_str());

    // The port-collision fix (README's own "Why"): a sandbox that
    // declares ports or services gets its own network namespace, so its
    // declared ports have a private table instead of the host's shared
    // one.
    //
    // This used to require zero egress, on the reasoning that an
    // isolated namespace has no route to the host-bound proxy and no
    // forwarding helper exists. That was measured and turned out wrong:
    // a *pathname unix socket crosses a network namespace*, so the proxy
    // gained a unix listener and the keeper relays to it from inside the
    // namespace. Both properties now hold at once, which is what an agent
    // actually needs — its own Postgres and its own filtered egress.
    //
    // Probed rather than assumed, and only when actually wanted — the
    // probe forks a real child (`fleet::netns::probe`), which is not
    // free, so a sandbox that doesn't qualify never pays for it. An
    // unsupported host degrades rather than fails `up`: the sandbox
    // still comes up, just back on the host's shared port table, which
    // is where every sandbox already was before this existed. Degrading
    // silently would violate CLAUDE.md's "Degraded capabilities are
    // surfaced, never silent" invariant, so this warns once.
    let wants_isolation = compiled.wants_network_isolation(services.is_some())
        // The relay binds the proxy's own port number inside the
        // namespace; if this manifest also declared that number, the
        // relay would fail to bind and egress would vanish. Isolation is
        // the half that gets dropped, since the collision it prevents is
        // less costly than the egress it would break.
        && match &proxy {
            Some((port, _)) if compiled.proxy_port_collides_with_declared_ports(*port) => {
                eprintln!(
                    "devcroft: warning: network isolation is skipped for this sandbox:                      the egress proxy was assigned port {port}, which this manifest also                      declares in `network.ports` (fallback: shared host port table;                      re-run `up` to draw a different proxy port)"
                );
                false
            }
            _ => true,
        };
    let isolate_network = wants_isolation
        && match crate::fleet::netns::probe(&exe) {
            Ok(true) => true,
            Ok(false) | Err(_) => {
                eprintln!(
                    "devcroft: warning: network isolation is degraded on this host: \
                     unprivileged network namespaces are unavailable (fallback: this \
                     sandbox's ports share the host's port table, and another sandbox \
                     binding the same port will collide)"
                );
                false
            }
        };

    // Isolation moves a declared port out of the host's reach, and a user
    // running a dev server has no way to discover that except by the
    // browser failing to connect. Measured: with `default = "deny"` and
    // `ports = [18440]`, a server bound inside the sandbox answers `200`
    // through `devcroft ssh -L` and nothing at all on the host's own
    // `127.0.0.1:18440`, where before isolation it answered directly.
    //
    // CLAUDE.md's "degraded capabilities are surfaced, never silent"
    // invariant is about capabilities the *host* cannot enforce, so this
    // is not literally that case — nothing is degraded, the port works
    // exactly as granted. But the principle is the same one, and the
    // failure it prevents is identical: a user who is not told reads a
    // manifest granting a port, sees it bind, and cannot reach it.
    if isolate_network && !compiled.network_ports.is_empty() {
        let ports: Vec<String> = compiled
            .network_ports
            .iter()
            .map(|p| p.value.to_string())
            .collect();
        eprintln!(
            "devcroft: note: this sandbox has its own network namespace, so its \
             declared port(s) {} are reachable from inside it and through \
             `devcroft ssh -L <local>:127.0.0.1:<port> {}`, but not directly on \
             the host's own loopback",
            ports.join(", "),
            manifest.sandbox.name,
        );
    }

    // design.md's Open Question 3, resolved: `NetworkMode::ProxyOnly`'s
    // kernel gate only ever permits a literal `connect()` to this port —
    // it does not redirect other destinations — so a client that never
    // looks at these variables gets denied at the kernel layer, not
    // silently mediated. Both cases (most tooling only honors one) and
    // both schemes point at the same endpoint: the proxy has no TLS
    // interception, so `HTTPS_PROXY` names a plain-`http` CONNECT
    // endpoint, same as every other forward proxy's convention.
    let mut env = resolution.env.clone();
    if let Some((port, token)) = &proxy {
        // Userinfo in the proxy URL, not a bespoke header or env var:
        // every standard HTTP client already turns `user@host` in a
        // proxy URL into `Proxy-Authorization: Basic <base64(user:)>`
        // unprompted — this is the mechanism that lets `curl`/`git`/
        // `npm`/`pip` authenticate to this sandbox's proxy without any
        // devcroft-specific proxy support (`add-egress-proxy`'s
        // authentication requirement; `proxy::server::authorized` is the
        // matching check).
        let endpoint = format!("http://{token}@127.0.0.1:{port}");
        for key in ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy"] {
            env.insert(key.to_string(), endpoint.clone());
        }
        // Without this, a well-behaved client honoring the variables
        // above would route its own loopback traffic (e.g. a test
        // hitting a `network.ports`-granted dev server) through the
        // proxy too — which then denies it, since `localhost` is not
        // something anyone would think to add to `network.allow`. Standard
        // proxy convention, not a devcroft invention.
        for key in ["NO_PROXY", "no_proxy"] {
            env.insert(key.to_string(), "localhost,127.0.0.1,::1".to_string());
        }

        // Brokered routes (`brokered-credentials`). The builder is given the
        // manifest's routes, the port and the *session* token — never the
        // resolved secret — so "the credential never enters the sandbox" is a
        // property of the signature rather than of this call site.
        //
        // `127.0.0.1:{port}` is correct in both topologies, which is why there
        // is no isolated/unisolated branch: an unisolated sandbox reaches the
        // host proxy on that port directly, and an isolated one has the
        // in-namespace relay bound to *the same port number*
        // (`DEVCROFT_PROXY_RELAY_PORT`). `NO_PROXY` above already covers
        // loopback, so the SDK dials this endpoint straight rather than
        // tunnelling its own base URL through the forward proxy.
        env.extend(crate::proxy::backend::broker_env(
            &manifest.brokers,
            *port,
            token,
        ));
    }

    // The relay is only needed when the sandbox is *both* isolated (no
    // route to host loopback) and has a proxy to reach. Either alone
    // needs nothing: an unisolated sandbox reaches the proxy's TCP port
    // directly, and an isolated one with no proxy has no egress to
    // relay. Also exactly the condition under which the mount view needs
    // the proxy socket (M3): that socket is only ever dialled by path at
    // all when the relay is what's doing the dialling — an unisolated
    // sandbox's `HTTPS_PROXY` points at the proxy's TCP port, never at
    // this path, so there is nothing for the view to grant in that case.
    let relay = isolate_network
        .then(|| {
            proxy
                .as_ref()
                .map(|(port, _)| (*port, paths.proxy_socket.clone()))
        })
        .flatten();

    let keeper_pid = spawn_keeper(
        &exe,
        &listener,
        paths,
        project_root,
        &env,
        // **The secret must not arrive by inheritance**, and a map cannot
        // express that: `.envs()` in `spawn_keeper` can only add or override.
        // `env:NAME` requires the user to have the credential exported, and a
        // provider's activated environment carries devcroft's own ambient
        // variables through — so without this every brokered credential also
        // sat in plain sight inside the sandbox, on a path unrelated to the
        // route, defeating brokering with its own precondition.
        //
        // Found by `tests/broker_credential_injection.rs`, not by review. The
        // structural guarantee in `backend::broker_env` — that it cannot leak
        // the secret because it is never given it — was necessary and not
        // sufficient.
        &resolution
            .unset
            .iter()
            .cloned()
            .chain(
                manifest
                    .brokers
                    .iter()
                    .filter_map(|b| b.source_var().map(str::to_string)),
            )
            .collect::<Vec<_>>(),
        &plan,
        SshHandoff {
            listener: &ssh_listener,
            host_key_pem: &host_key_pem,
            authorized_key_pem: &authorized_key_pem,
        },
        services,
        &shell.path,
        isolate_filesystem,
        isolate_network,
        relay.clone(),
        &mount_grants,
        relay.as_ref().map(|(_, sock)| sock.as_path()),
    )
    .map_err(|e| UpError::Keeper(e.to_string()))?;
    // Both fds must outlive this function for the child to inherit them
    // across exec; ownership passes to the keeper process from here.
    std::mem::forget(listener);
    std::mem::forget(ssh_listener);

    state::write_pidfile(&paths.pidfile, keeper_pid)?;

    wait_until_responsive(paths, KEEPER_START_TIMEOUT)
        .map_err(|e| UpError::Keeper(format!("keeper did not become responsive: {e}")))?;

    // The provider's own activation script, run **inside** the boundary
    // (`sandbox-provisioning` P2d). Ordered before devcroft's own hooks
    // deliberately: this is what prepares the project's environment, so a
    // `post_create` that depends on it — the common case, since that is
    // what environment setup is for — would otherwise run first and fail.
    //
    // `--skip-hooks` suppresses it for the same reason it suppresses
    // everything else: that flag's promise is that nothing
    // project-supplied runs, and this is project-supplied.
    if !opts.skip_hooks
        && let Some(script) = &resolution.activation_script
    {
        hooks::run_activation_script(paths, project_root, script)
            .map_err(|e| UpError::Keeper(e.to_string()))?;
    }

    // Lifecycle spec: `post_create` runs once, as the first session after
    // the *first* successful `up` or after `--recreate` — exactly the
    // outcomes below, since `Recovered` means state already existed (so
    // `post_create` already ran back when it was `Started`) and `AlreadyUp`
    // already returned above without spawning anything. `post_start` runs
    // on every keeper start regardless, so it always runs here too.
    if !opts.skip_hooks {
        let run_post_create = matches!(outcome, UpOutcome::Started | UpOutcome::Recreated);
        hooks::run(paths, project_root, &manifest.hooks, run_post_create)
            .map_err(|e| UpError::Keeper(e.to_string()))?;
    }

    Ok(outcome)
}

/// Host-side, trusted-phase preparation for declared services, shared by
/// both tiers — the `services` change's task 3.2 requires exactly one
/// path here ("do not add a tier-specific path"), and this is it.
/// Returns whether the keeper should start services at all.
///
/// Runs before any restriction is applied, so nothing project-supplied
/// executes to produce the config; `--skip-hooks` suppresses it for the
/// same reason it suppresses hooks — one flag that guarantees nothing
/// project-supplied runs.
/// The `services` spec's "Services requested from a provider that cannot
/// supply them fail loudly", in the only shape that can actually happen.
///
/// The literal reading — a manifest asking for services under `nix` —
/// is unreachable: declarations come from the *provider's* own manifest,
/// and `devcroft.toml` has no `[services]` section of its own, so a nix
/// project has no way to ask. Task 2.4 was left open for exactly that
/// reason rather than shipping a check that could never fire.
///
/// What *is* reachable, and what users actually hit: a project carrying a
/// flox environment whose `[services]` are declared, with `devcroft.toml`
/// saying `provider = "nix"`. Those services were silently ignored — the
/// sandbox came up reporting no services at all, indistinguishable from a
/// project that declares none, which is the failure mode the whole
/// `services` spec is written against.
///
/// Deliberately narrow: it only fires when another provider devcroft
/// *supports* has real declarations sitting there. It is a "you asked for
/// this and it will not happen" check, not an invitation to scan the
/// project for anything service-shaped.
fn ensure_no_services_declared_for_another_provider(
    project_root: &Path,
    opts: &UpOptions,
) -> Result<(), UpError> {
    // `--skip-hooks` promises nothing project-supplied runs; refusing to
    // start because of services that would not have started anyway would
    // make the escape hatch useless for debugging.
    if opts.skip_hooks {
        return Ok(());
    }
    let declared = crate::provider::services_declared_by_flox(project_root);
    if declared.is_empty() {
        return Ok(());
    }
    Err(UpError::Provider(
        crate::provider::ProviderError::ResolutionFailed(format!(
            "this project's flox environment declares {} service(s) ({}), but \
             `env.provider` is `nix`, which has no service concept — they would \
             be silently ignored; set `provider = \"flox\"` to run them, or \
             remove them from the flox manifest",
            declared.len(),
            declared.join(", ")
        )),
    ))
}

fn prepare_services(
    project_root: &Path,
    sandbox_name: &str,
    resolution: &Resolution,
    shell: &Path,
    opts: &UpOptions,
) -> Result<bool, UpError> {
    if let ServiceSupport::Unsupported = resolution.services {
        ensure_no_services_declared_for_another_provider(project_root, opts)?;
    }
    let services = resolution.services.declared();
    if opts.skip_hooks || services.is_empty() {
        return Ok(false);
    }
    // Checked here, host-side, because the failure downstream is opaque:
    // over the `sun_path` limit, process-compose fails to bind with an
    // error naming neither the path nor the length. See
    // `services::MAX_SOCKET_PATH` — the per-sandbox subdirectory this
    // path now carries makes it one level deeper than it used to be.
    let socket = crate::services::socket_path(project_root, sandbox_name);
    let socket_len = socket.as_os_str().as_encoded_bytes().len();
    if socket_len > crate::services::MAX_SOCKET_PATH {
        return Err(UpError::Config(format!(
            "the service supervisor's socket path is {socket_len} bytes, over the \
             {} the OS allows for a unix socket: {}\n\
             move the project closer to the filesystem root, or shorten \
             `[sandbox].name`",
            crate::services::MAX_SOCKET_PATH,
            socket.display()
        )));
    }
    // `process-compose` must come from the project's own environment,
    // never the host's PATH and never a scanned store path — see
    // `services::resolve_in_env`. Failing here, at layer `provider`,
    // beats starting a sandbox whose declared services silently never
    // come up.
    if crate::services::resolve_in_env(&resolution.env).is_none() {
        return Err(UpError::Provider(
            crate::provider::ProviderError::ResolutionFailed(format!(
                "{} service(s) are declared but `{binary}` is not in the \
                 resolved environment; add it to the environment manifest \
                 (e.g. `flox install {binary}`)",
                services.len(),
                binary = crate::services::supervisor().binary()
            )),
        ));
    }
    let config_path = crate::services::config_path(project_root, sandbox_name);
    if let Some(dir) = config_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(
        &config_path,
        crate::services::supervisor().render_config(services, shell),
    )?;
    Ok(true)
}

/// The ssh server's fd and key material `spawn_keeper` hands the keeper,
/// bundled to keep that call under clippy's argument-count lint —  see
/// its call site for why the key material can't just be a file the
/// keeper reads back itself.
struct SshHandoff<'a> {
    listener: &'a UnixListener,
    host_key_pem: &'a str,
    authorized_key_pem: &'a str,
}

/// Execs the keeper binary directly — no `nono wrap` prefix
/// (`use-nono-library`; lifecycle spec: "The keeper restricts itself with
/// no intermediate process"). The keeper applies `plan` to itself as its
/// first action in `__keeper` mode, right after reconstructing the
/// listener fds and before anything else runs — see `keeper_main`'s own
/// doc comment for why that ordering is non-negotiable.
#[allow(clippy::too_many_arguments)]
fn spawn_keeper(
    exe: &Path,
    listener: &UnixListener,
    paths: &StatePaths,
    project_root: &Path,
    env: &std::collections::BTreeMap<String, String>,
    unset: &[String],
    plan: &policy::CapabilityPlan,
    ssh: SshHandoff,
    services: Option<&str>,
    shell: &Path,
    isolate_filesystem: bool,
    isolate_network: bool,
    relay: Option<(u16, PathBuf)>,
    mount_grants: &[policy::ResolvedGrant],
    proxy_socket_for_view: Option<&Path>,
) -> io::Result<libc::pid_t> {
    // The pivot-root scratch directory (`StatePaths::mount_root`'s own
    // doc) must exist and be empty before `pre_exec` runs — created here,
    // host-side, rather than inside the forked child, so a failure to
    // create it surfaces as an ordinary `io::Error` from this function
    // rather than as an opaque `pre_exec`/`spawn` failure. Skipped
    // entirely when mount isolation is degraded (macOS today, per
    // `up_process`'s own platform split): nothing will ever pivot into
    // it, so creating it would just be a stray empty directory.
    if isolate_filesystem {
        let _ = std::fs::remove_dir_all(&paths.mount_root);
        std::fs::create_dir_all(&paths.mount_root)?;
    }
    let mount_root = paths.mount_root.clone();
    let mount_grants = mount_grants.to_vec();
    let proxy_socket_for_view = proxy_socket_for_view.map(Path::to_path_buf);
    let project_root_owned = project_root.to_path_buf();

    // Truncate the previous run's log, then reopen with `O_APPEND` — the
    // keeper is not this file's only writer. `hooks::run` appends hook
    // output to it from the `up` side (lifecycle spec: "hook output SHALL
    // appear in `logs`"), concurrently with the keeper's own spawn/exit
    // records. Without `O_APPEND` the keeper's fd carries its own offset
    // and overwrites whatever the hook appended in between, silently
    // eating exactly the output the spec requires to be there.
    std::fs::File::create(&paths.log)?;
    let log = std::fs::OpenOptions::new().append(true).open(&paths.log)?;

    let mut cmd = Command::new(exe);
    cmd.arg("__keeper")
        .arg(listener.as_raw_fd().to_string())
        .arg(ssh.listener.as_raw_fd().to_string())
        .current_dir(project_root)
        .envs(env);
    for key in unset {
        // provider::Resolution's "unset" gap: without this, a key
        // activation explicitly removed would still leak into the keeper
        // from *this* process's own ambient environment (whoever's shell
        // ran `up`) — `.envs(env)` above can only add/override, never
        // remove, so a plain map has no way to represent "unset" at all.
        cmd.env_remove(key);
    }
    cmd
        // The keeper's very first action (task group 4): deserialize this
        // and self-restrict, before reading anything else from `env`.
        // Same trust boundary the SSH key material below already crosses
        // this way — the keeper cannot read this back off disk itself
        // once sandboxed (nothing under `policy::DEVCROFT_DATA_DIR` is
        // reachable to it, and by the time it *could* read a file, it
        // would already need to be restricted to know it's safe to).
        .env(
            "DEVCROFT_CAPABILITY_PLAN",
            serde_json::to_string(plan).expect("CapabilityPlan serialization is infallible"),
        )
        // ssh spec's key handoff (task 6.1): the keeper can't read either
        // key back off disk itself (see the call site's comment), so both
        // travel as env vars, same trust boundary the resolved provider
        // environment above already crosses this way.
        .env("DEVCROFT_SSH_HOST_KEY", ssh.host_key_pem)
        .env("DEVCROFT_SSH_AUTHORIZED_KEY", ssh.authorized_key_pem)
        // The absolute shell `up` resolved out of this sandbox's closure
        // (`crate::shell`). The keeper starts SSH login sessions with it;
        // a bare `sh` would PATH-resolve to the host's, which the
        // compiled policy denies, and the failure surfaces only as
        // `shell request failed on channel 0`.
        .env("DEVCROFT_SHELL", shell)
        // Services are started by the *keeper*, not by `up`: `up` exits,
        // and a session whose client disconnects is escalated after
        // `connection::DEFAULT_GRACE_PERIOD`, so anything `up` started
        // over the control socket would die seconds later. The keeper
        // owns their lifetime, and its own startup is the moment — which
        // also puts services before hooks, the ordering add-flox-services'
        // design.md decision 4 settled on independently.
        .env(
            "DEVCROFT_START_SERVICES",
            if services.is_some() { "1" } else { "0" },
        )
        // Which sandbox's artifact subdirectory to use. Service paths are
        // keyed on the sandbox name as well as the root, so that two
        // sandboxes sharing one project root do not overwrite each
        // other's config or fight over one supervisor socket.
        .env("DEVCROFT_SANDBOX_NAME", services.unwrap_or_default())
        // Absolute, not relative-to-cwd: the hardened tier's control
        // server runs host-side and dispatches through `runsc exec
        // --cwd`, which needs an absolute path. Same value here keeps
        // one code path in `start_services_if_requested`.
        .env("DEVCROFT_SERVICES_ROOT", project_root)
        // Set together or not at all — `keeper_main` reads them as a
        // pair, so a half-configured relay is not representable.
        .envs(
            relay
                .as_ref()
                .map(|(port, sock)| {
                    [
                        ("DEVCROFT_PROXY_RELAY_PORT".to_string(), port.to_string()),
                        (
                            "DEVCROFT_PROXY_SOCKET".to_string(),
                            sock.to_string_lossy().into_owned(),
                        ),
                    ]
                })
                .unwrap_or_default(),
        )
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    // SAFETY: setsid() only touches this (freshly forked, single-
    // threaded) child's own session/process-group state. Detaching from
    // the supervisor's controlling terminal is what lets the keeper
    // outlive `up`'s own process and the invoking shell. The mount/
    // network namespace calls below are the same raw `unshare`/`mount`/
    // `ioctl` primitives `tests/fleet_netns.rs` and `tests/fleet_mount.rs`
    // already exercise live, safe in a freshly forked, single-threaded,
    // not-yet-exec'd child for the same reason `setsid` is.
    //
    // **Order is load-bearing, not just "the order these were added
    // in" (unlike the netns-only version this replaced).** `fleet::mount::
    // enter_mount_namespace_with_network` must run before
    // `make_propagation_private` (there is nothing to make private
    // until the namespace exists) and before `construct_view` (which
    // needs both the namespace and private propagation — design.md M1).
    // `bring_loopback_up` needs the network namespace but nothing
    // mount-related, so it can run any time after the first call;
    // kept together with the netns half of this closure for locality.
    // `construct_view` runs last: it `pivot_root`s, after which this
    // process's view of the filesystem is the sandbox's own — nothing
    // after it may assume the host's paths still resolve.
    //
    // **`construct_view` ends by `chdir("/")`, and this closure chdirs
    // back to `project_root` immediately after — not a redundant step.**
    // `Command`'s own `current_dir(project_root)` above runs *before*
    // any `pre_exec` closure (std's own child setup order: cwd, then
    // stdio, then the caller's closures), so without this, the keeper
    // would start in `/` — still the mount view's `/`, but not the
    // project root the rest of `up` assumes it starts in. Bind-mounted
    // at the identical absolute path it has on the host (`construct_view`
    // never remaps paths), so the same string that worked before
    // `pivot_root` still resolves correctly after it.
    //
    // Spiked separately before wiring the network half in originally:
    // `unshare(CLONE_NEWUSER)` changes what this process's own uid/gid
    // read back as *from inside* the new namespace, but does not change
    // the credentials the kernel actually checks file access against —
    // confirmed live, not assumed, and still true with mount isolation
    // layered on top (`fleet::mount::enter_mount_namespace`'s own doc
    // makes the identical point for its uid/gid mapping).
    unsafe {
        cmd.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            // `isolate_network` can only be `true` here when
            // `isolate_filesystem` is too — network namespaces are as
            // Linux-only as mount namespaces (`fleet::netns`'s own
            // `#[cfg(not(target_os = "linux"))]` stub), and `up_process`
            // only ever sets it once mount isolation's own platform
            // check already passed. So this is not two independent
            // conditions to reconcile: when mount isolation is degraded
            // (macOS today), nothing here runs at all, and the keeper
            // execs under plain Landlock/Seatbelt — the exact boundary
            // every sandbox had before `add-mount-isolation` existed.
            if isolate_filesystem {
                crate::fleet::mount::enter_mount_namespace_with_network(isolate_network)?;
                if isolate_network {
                    crate::fleet::netns::bring_loopback_up()?;
                }
                crate::fleet::mount::make_propagation_private()?;
                crate::fleet::mount::construct_view(
                    &mount_root,
                    &mount_grants,
                    proxy_socket_for_view.as_deref(),
                )?;
                std::env::set_current_dir(&project_root_owned)?;
            }
            Ok(())
        });
    }

    let child = cmd.spawn()?;
    let pid = child.id() as libc::pid_t;
    // Detach: `up` is a one-shot command, not the keeper's supervisor for
    // its whole lifetime. Not calling `.wait()` lets the keeper outlive
    // this process; once `up` exits, the OS reparents it to init, which
    // reaps it same as any other orphaned daemon.
    std::mem::forget(child);
    Ok(pid)
}

/// The binary to re-exec as the keeper. Normally `current_exe()`, but that
/// resolves to whatever process is *currently running* — inside a `cargo
/// test` unit test that's the libtest harness binary, not `devcroft`,
/// which would otherwise get `__keeper <fd>` handed to it as bogus test
/// filter arguments. `DEVCROFT_KEEPER_EXE` lets the integration test
/// (`tests/lifecycle_up.rs`, via `CARGO_BIN_EXE_devcroft`) point this at
/// the real built binary instead; production code never sets it.
fn keeper_exe() -> io::Result<PathBuf> {
    if let Ok(path) = std::env::var("DEVCROFT_KEEPER_EXE") {
        return Ok(PathBuf::from(path));
    }
    std::env::current_exe()
}

fn wait_until_responsive(paths: &StatePaths, timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if UnixStream::connect(&paths.socket).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "timed out waiting for the keeper's control socket to accept connections",
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `pub(crate)`: `crate::proxy::spawn` needs the identical clear (its
/// listener crosses the same exec boundary the control/SSH sockets do
/// here, just into a different child), and duplicating a five-line
/// `fcntl` dance is worse than one more crate-visible function.
pub(crate) fn clear_cloexec(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
