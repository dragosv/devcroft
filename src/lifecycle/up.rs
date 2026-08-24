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

use crate::config::{Isolation, Manifest};
use crate::policy;
use crate::provider::{Provider, ProviderError, ProviderKind, Resolution};

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
    let paths = StatePaths::new(&manifest.sandbox.name)?;

    let outcome = if opts.recreate {
        if let Health::Healthy(pid) | Health::Stale(pid) = state::health(&paths)? {
            state::terminate_and_wait(pid, TERMINATE_GRACE_PERIOD);
        }
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
    let resolved_backend = resolve_backend(manifest.sandbox.isolation)?;

    // Host-side, before any restriction applies (design.md decision 2):
    // the resolved environment and its store grants are captured now,
    // once, and folded into the profile/bundle the sandbox will be
    // confined to. `env.provider` is already validated and normalized by
    // config::parse (the only place a Manifest is constructed), so
    // `from_name` here only ever dispatches to a real implementation.
    // This step is identical for both tiers — the two-phase execution
    // model (CLAUDE.md) is backend-generic, not just process-tier.
    let provider = ProviderKind::from_name(&manifest.env.provider).map_err(UpError::Provider)?;
    let resolution = provider.resolve(project_root).map_err(UpError::Provider)?;

    // Recorded now so `status` (task 4.3) can later tell whether the
    // environment has drifted since this `up`, and which concrete
    // backend it resolved to, without needing the manifest or project
    // root passed back in — the keeper itself is never told its own
    // state dir, so it can't answer either.
    let env_fingerprint =
        crate::provider::manifest_fingerprint(&manifest.env.provider, project_root)
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

    state::write_meta(
        &paths.meta,
        &state::Meta {
            project_root: project_root.to_string_lossy().into_owned(),
            env_fingerprint,
            read_only_grants: resolution.read_only_grants.clone(),
            resolved_backend: resolved_backend.clone(),
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
        },
    )?;

    match manifest.sandbox.isolation {
        Isolation::Process => {
            up_process(manifest, project_root, &paths, opts, outcome, &resolution)
        }
        Isolation::Hardened => up_hardened(
            manifest,
            project_root,
            &paths,
            opts,
            outcome,
            &resolution,
            &resolved_backend,
        ),
    }
}

/// Resolves `isolation` to the concrete backend string `status`/`meta.json`
/// record (`"process"`, or `"gvisor/<platform>"`), failing at layer
/// `backend` if the host cannot provide what was asked for. Cheap and
/// side-effect-free: for `hardened` this only probes `/dev/kvm`
/// accessibility (`gvisor::select_platform`) and checks the compile-time
/// target OS, never spawns a process or touches the state dir.
fn resolve_backend(isolation: Isolation) -> Result<String, UpError> {
    match isolation {
        Isolation::Process => Ok("process".to_string()),
        Isolation::Hardened => {
            if !cfg!(target_os = "linux") {
                return Err(UpError::Backend(
                    "the hardened isolation tier is Linux-only".to_string(),
                ));
            }
            let platform = crate::gvisor::select_platform();
            Ok(format!("gvisor/{}", platform.runsc_flag()))
        }
    }
}

/// The `process` tier's supervisor sequence — today's `up`, unchanged in
/// every particular, just extracted so [`up`] can dispatch to it or to
/// [`up_hardened`] from one shared prefix (state dir, provider
/// resolution, meta).
fn up_process(
    manifest: &Manifest,
    project_root: &Path,
    paths: &StatePaths,
    opts: &UpOptions,
    outcome: UpOutcome,
    resolution: &Resolution,
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
        .with_provider_grants(
            crate::provider::ProviderKind::from_name(&manifest.env.provider)
                .map_err(UpError::Provider)?
                .static_name(),
            &resolution.read_only_grants,
        );
    let plan = compiled.to_capability_plan();
    // Validated host-side, before anything is created — the keeper (task
    // group 4) re-derives the identical `CapabilitySet` from the same
    // plan right before self-restricting, but doing it here too means a
    // `CapabilitySetError::DenyOverlapsAllow` fails `up` at the `config`
    // layer immediately, before a listener exists or a process spawns,
    // rather than surfacing as an opaque keeper-startup failure.
    plan.to_capability_set(project_root)
        .map_err(|e| UpError::Policy(e.to_string()))?;
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
    let services = prepare_services(project_root, &manifest.sandbox.name, resolution, opts)?
        .then_some(manifest.sandbox.name.as_str());

    let keeper_pid = spawn_keeper(
        &exe,
        &listener,
        paths,
        project_root,
        &resolution.env,
        &resolution.unset,
        &plan,
        SshHandoff {
            listener: &ssh_listener,
            host_key_pem: &host_key_pem,
            authorized_key_pem: &authorized_key_pem,
        },
        services,
    )
    .map_err(|e| UpError::Keeper(e.to_string()))?;
    // Both fds must outlive this function for the child to inherit them
    // across exec; ownership passes to the keeper process from here.
    std::mem::forget(listener);
    std::mem::forget(ssh_listener);

    state::write_pidfile(&paths.pidfile, keeper_pid)?;

    wait_until_responsive(paths, KEEPER_START_TIMEOUT)
        .map_err(|e| UpError::Keeper(format!("keeper did not become responsive: {e}")))?;

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
fn prepare_services(
    project_root: &Path,
    sandbox_name: &str,
    resolution: &Resolution,
    opts: &UpOptions,
) -> Result<bool, UpError> {
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
                "{} service(s) are declared but `process-compose` is not in the \
                 resolved environment; add it to the environment manifest \
                 (e.g. `flox install process-compose`)",
                services.len()
            )),
        ));
    }
    let config_path = crate::services::config_path(project_root, sandbox_name);
    if let Some(dir) = config_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&config_path, crate::services::render_config(services))?;
    Ok(true)
}

/// The `hardened` tier's supervisor sequence (add-gvisor-backend): no
/// nono, no fd-inheritance-across-self-restriction dance — there is
/// nothing to self-restrict, since the host-side control process is not
/// the trust boundary at this tier (the backend's own sandboxing is).
/// Builds the OCI bundle from the same `CompiledPolicy` [`up_process`]
/// projects into a nono profile, starts the sandbox detached, then
/// starts a host-side control server (`__hardened_keeper`) dispatching
/// sessions through `runsc exec` instead of a local fork/exec — see the
/// `ssh` delta spec's "Sessions via native exec, dispatched host-side"
/// requirement.
#[cfg(target_os = "linux")]
fn up_hardened(
    manifest: &Manifest,
    project_root: &Path,
    paths: &StatePaths,
    opts: &UpOptions,
    outcome: UpOutcome,
    resolution: &Resolution,
    resolved_backend: &str,
) -> Result<UpOutcome, UpError> {
    let runsc = crate::gvisor::runsc_command::resolve().ok_or_else(|| {
        UpError::Backend(
            "runsc not found on PATH; install it and re-run, or `devcroft doctor` for details"
                .to_string(),
        )
    })?;

    // "Provider grants map onto mounts or fail loudly" (add-gvisor-backend):
    // every read-only grant must be an absolute, existing host path before
    // any sandbox starts — never silently widened or dropped.
    for grant in &resolution.read_only_grants {
        let path = Path::new(grant);
        if !path.is_absolute() || !path.exists() {
            return Err(UpError::Backend(format!(
                "provider grant `{grant}` cannot be represented as a bundle mount: \
                 not an absolute, existing path"
            )));
        }
    }

    // Before anything starts: the config is a host-side artifact written
    // into the project root, and the project root is a rw bind mount at
    // the *identical* path inside the sandbox (`oci_spec::build`), so the
    // file named here is the file process-compose opens inside the
    // boundary. Ordered ahead of `runsc run` for the same reason
    // `up_process` puts it ahead of `spawn_keeper` — a missing
    // `process-compose` must fail before a sandbox exists, not after.
    // `Some(name)` means "start services, using this sandbox's artifact
    // subdirectory"; `None` means there are none to start.
    let services = prepare_services(project_root, &manifest.sandbox.name, resolution, opts)?
        .then_some(manifest.sandbox.name.as_str());

    let compiled = policy::compile(manifest);
    let network = crate::gvisor::oci_spec::NetworkMode::from_compiled_policy(&compiled);
    let spec = crate::gvisor::oci_spec::build(
        &compiled,
        &crate::gvisor::oci_spec::BundleInputs {
            project_root,
            bundle_dir: &paths.gvisor_bundle,
            read_only_grants: &resolution.read_only_grants,
            env: &resolution.env,
        },
    );
    crate::gvisor::runner::materialize_bundle(&paths.gvisor_bundle, &spec)
        .map_err(|e| UpError::Backend(format!("materializing OCI bundle: {e}")))?;

    // `resolved_backend` is "gvisor/<platform>" (see `resolve_backend`);
    // re-parsed rather than re-probed, so `run` uses exactly the
    // platform `status`/`meta.json` already committed to recording for
    // this `up`, not a second, potentially different KVM-accessibility
    // check moments later.
    let platform = match resolved_backend.split_once('/') {
        Some((_, "kvm")) => crate::gvisor::Platform::Kvm,
        _ => crate::gvisor::Platform::Systrap,
    };

    let container = crate::gvisor::runsc_command::Container {
        id: &manifest.sandbox.name,
        state_root: &paths.gvisor_runsc_state,
    };
    crate::gvisor::runner::run(
        &runsc,
        &container,
        &paths.gvisor_bundle,
        platform,
        network,
        services.is_some(),
    )
    .map_err(|e| UpError::Backend(format!("runsc run: {e}")))?;

    // From here on, mirrors `up_process`'s own listener-creation and key-
    // handoff steps exactly — the socket layout and ssh key material are
    // tier-agnostic (`ssh` delta spec: "still only a 0600 unix socket in
    // a 0700 dir, still never binding TCP").
    let _ = std::fs::remove_file(&paths.socket);
    let listener = UnixListener::bind(&paths.socket)?;
    // Same 0600 as the process tier's control socket — see the comment
    // there. This tier needs it at least as much: `--host-uds=create`
    // exists precisely to let the host reach inward.
    std::fs::set_permissions(&paths.socket, std::fs::Permissions::from_mode(0o600))?;
    clear_cloexec(listener.as_raw_fd())?;

    let _ = std::fs::remove_file(&paths.ssh_socket);
    let ssh_listener = UnixListener::bind(&paths.ssh_socket)?;
    std::fs::set_permissions(&paths.ssh_socket, std::fs::Permissions::from_mode(0o600))?;
    clear_cloexec(ssh_listener.as_raw_fd())?;

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

    let exe = keeper_exe()?;
    let control_pid = spawn_hardened_keeper(
        &exe,
        &listener,
        paths,
        project_root,
        &runsc,
        &manifest.sandbox.name,
        SshHandoff {
            listener: &ssh_listener,
            host_key_pem: &host_key_pem,
            authorized_key_pem: &authorized_key_pem,
        },
        services,
    )
    .map_err(|e| UpError::Keeper(e.to_string()))?;
    std::mem::forget(listener);
    std::mem::forget(ssh_listener);

    state::write_pidfile(&paths.pidfile, control_pid)?;

    wait_until_responsive(paths, KEEPER_START_TIMEOUT)
        .map_err(|e| UpError::Keeper(format!("control server did not become responsive: {e}")))?;

    if !opts.skip_hooks {
        let run_post_create = matches!(outcome, UpOutcome::Started | UpOutcome::Recreated);
        hooks::run(paths, project_root, &manifest.hooks, run_post_create)
            .map_err(|e| UpError::Keeper(e.to_string()))?;
    }

    Ok(outcome)
}

/// Unreachable in practice: [`resolve_backend`] already fails at layer
/// `backend` before `up` ever dispatches here on a non-Linux host. This
/// stub exists only so the crate still compiles on macOS, where
/// `crate::gvisor::runner` (this function's real implementation) is not
/// even built.
#[cfg(not(target_os = "linux"))]
fn up_hardened(
    _manifest: &Manifest,
    _project_root: &Path,
    _paths: &StatePaths,
    _opts: &UpOptions,
    _outcome: UpOutcome,
    _resolution: &Resolution,
    _resolved_backend: &str,
) -> Result<UpOutcome, UpError> {
    unreachable!("resolve_backend already rejects the hardened tier on non-Linux hosts")
}

/// Spawns the hardened tier's host-side control server
/// (`__hardened_keeper`) detached, the same "re-exec this binary,
/// `setsid`, forget the child" pattern [`spawn_keeper`] uses — but with
/// no `nono wrap` prefix (nothing to self-restrict) and no resolved
/// provider environment to inject (that's already baked into the OCI
/// bundle's `process.env`; sessions get it from the sandbox, not from
/// this process's own environment).
#[cfg(target_os = "linux")]
#[allow(clippy::too_many_arguments)]
fn spawn_hardened_keeper(
    exe: &Path,
    listener: &UnixListener,
    paths: &StatePaths,
    project_root: &Path,
    runsc: &Path,
    container_id: &str,
    ssh: SshHandoff,
    services: Option<&str>,
) -> io::Result<libc::pid_t> {
    let log = std::fs::File::create(&paths.log)?;

    let mut cmd = Command::new(exe);
    cmd.arg("__hardened_keeper")
        .arg(listener.as_raw_fd().to_string())
        .arg(ssh.listener.as_raw_fd().to_string())
        .arg(container_id)
        .arg(runsc)
        .arg(&paths.gvisor_runsc_state)
        // Same cwd contract `spawn_keeper` gives the process tier: the
        // control server's own cwd is the project root, which is where
        // `ssh::server` takes each session's starting directory from.
        .current_dir(project_root)
        .env("DEVCROFT_SSH_HOST_KEY", ssh.host_key_pem)
        .env("DEVCROFT_SSH_AUTHORIZED_KEY", ssh.authorized_key_pem)
        // Same handoff as the process tier (see `spawn_keeper`): the
        // supervisor cannot own service lifetime, so the control server
        // starts them at its own startup — here through `RunscExecBackend`
        // rather than a local fork/exec, which is the whole point of
        // routing service startup through the `SessionBackend` seam.
        .env(
            "DEVCROFT_START_SERVICES",
            if services.is_some() { "1" } else { "0" },
        )
        // Which sandbox's artifact subdirectory to use. Service paths are
        // keyed on the sandbox name as well as the root, so that two
        // sandboxes sharing one project root do not overwrite each
        // other's config or fight over one supervisor socket.
        .env("DEVCROFT_SANDBOX_NAME", services.unwrap_or_default())
        // The sandbox sees the project root at the identical path, so one
        // absolute value is correct on both sides of the boundary — and
        // `runsc exec --cwd` needs an absolute path, which the process
        // tier's implicit "." would not give it.
        .env("DEVCROFT_SERVICES_ROOT", project_root)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    // SAFETY: setsid() only touches this (freshly forked, single-
    // threaded) child's own session/process-group state — identical
    // reasoning to `spawn_keeper`'s own pre_exec below.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = cmd.spawn()?;
    let pid = child.id() as libc::pid_t;
    std::mem::forget(child);
    Ok(pid)
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
) -> io::Result<libc::pid_t> {
    let log = std::fs::File::create(&paths.log)?;

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
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    // SAFETY: setsid() only touches this (freshly forked, single-
    // threaded) child's own session/process-group state. Detaching from
    // the supervisor's controlling terminal is what lets the keeper
    // outlive `up`'s own process and the invoking shell.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
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

fn clear_cloexec(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
