//! The `devenv` provider (add-devenv-provider): the fourth closure-tier
//! environment provider, and the first whose hook-free capture route and
//! whose project hook are *both* things the provider itself exposes —
//! flox needed `flox::derive_hook_free_env` because no flox mode
//! suppresses `[hook].on-activate`, where devenv separates the two
//! upstream.
//!
//! Two entry points, both measured not to run project code (design.md —
//! Measured (group 0), devenv 2.2.2):
//!
//! - `devenv build shell` builds the environment and returns the store
//!   path of a `declare -x` dump. 0 hook executions.
//! - `devenv eval enterShell` returns the hook's text as JSON. 0 hook
//!   executions. devcroft runs it *inside* the sandbox
//!   (`Resolution::activation_script`), never here.
//!
//! `devenv direnv-export` runs the hook once and `devenv shell -- <cmd>`
//! runs it twice, so neither is usable for capture however convenient
//! their output is.

use super::capture;
use super::{Provider, ProviderError, Resolution};
use crate::paths::resolve_on_path;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

pub struct DevenvProvider;

impl Provider for DevenvProvider {
    /// Resolve a devenv environment: verify `devenv.nix`, then `devenv`,
    /// then a usable Nix, then `devenv.lock`, then capture through the
    /// hook-free route, capture the hook as data, and verify capture
    /// resolved nothing.
    ///
    /// Preconditions run in the order that makes a failure name the most
    /// specific fix available, the ordering `nix.rs` established.
    fn resolve(&self, project_root: &Path) -> Result<Resolution, ProviderError> {
        ensure_project_present(project_root)?;
        let devenv_bin = resolve_on_path("devenv").ok_or(ProviderError::MissingBinary {
            provider: "devenv",
            hint: "devcroft doctor",
        })?;
        ensure_nix_usable()?;
        ensure_lock_present(project_root)?;

        // The lockfile as it was *before* capture, compared again after.
        // Measured (task 0.4): `devenv build shell` on a project with no
        // lockfile writes one rather than refusing, which is why
        // `ensure_lock_present` runs first and why this check exists at
        // all — between them, "nothing resolves at `up`" is enforced
        // rather than hoped for.
        let lock_before = read_lock(project_root);

        let baseline = capture::canonical_base_env()?;
        let activated = capture_activated_env(&devenv_bin, project_root, &baseline)?;
        let activation_script = capture_enter_shell(&devenv_bin, project_root, &baseline)?;
        let services = capture_processes(&devenv_bin, project_root, &baseline)?;
        restore_lock_if_capture_resolved(project_root, &lock_before)?;

        Ok(Resolution {
            env: capture::changed_env(&baseline, &activated),
            unset: capture::unset_env(&baseline, &activated),
            read_only_grants: capture::store_grants(&activated),
            // Read from `devenv eval processes`, which runs no project
            // code (`add-devenv-services`). The question this used to
            // defer — whether devenv's declarations are a contract or
            // only generated output — was measured and answered: they
            // come from a documented option in the project's own
            // manifest, which is the bar flox met.
            services,
            // Structurally false, and false for a reason no other
            // provider has: devenv *has* a project hook and devcroft
            // captures it, but neither capture route executes it —
            // measured with a sentinel, asserted by
            // `build_shell_does_not_run_enter_shell`. nix and devbox
            // report false because they have no captured hook at all.
            ran_activation_hook: false,
            activation_script,
        })
    }
}

/// `up` fails at layer `provider` with the `devenv init` hint (spec:
/// "Missing environment, not missing feature") rather than letting devenv
/// produce its own, less specific error about a project it cannot find.
fn ensure_project_present(project_root: &Path) -> Result<(), ProviderError> {
    if project_root.join("devenv.nix").is_file() {
        Ok(())
    } else {
        Err(ProviderError::NoEnvironment {
            provider: "devenv",
            hint: "devenv init",
        })
    }
}

/// devenv is a frontend over Nix and cannot materialize anything without
/// it. Reported as **devenv's own** unmet requirement (cli spec: "Where a
/// provider is a frontend over another tool"), never as advice to switch
/// providers, and probed as a capability rather than by the binary's
/// presence — `nix --version` succeeds against a store this host cannot
/// reach.
fn ensure_nix_usable() -> Result<(), ProviderError> {
    let nix = resolve_on_path("nix").ok_or(ProviderError::MissingBinary {
        provider: "nix",
        hint: "devcroft doctor",
    })?;

    let usable = Command::new(&nix)
        .arg("eval")
        .arg("--expr")
        .arg("1")
        .output()
        .is_ok_and(|o| o.status.success());

    if usable {
        Ok(())
    } else {
        Err(ProviderError::ResolutionFailed(
            "devenv requires a usable Nix, but `nix eval` failed on this host; devenv is a \
             frontend over Nix and cannot resolve packages without it (see `devcroft doctor`)"
                .to_string(),
        ))
    }
}

/// A `devenv.nix` without `devenv.lock` has nothing pinning its inputs.
///
/// Checked **before** capture rather than being left to
/// [`restore_lock_if_capture_resolved`], because the two failures are not
/// the same and neither is the message: a project that was never locked
/// needs `devenv update`, where one whose capture rewrote the lockfile
/// has a provider resolving at `up`. Measured (task 0.4): `devenv build
/// shell` writes a missing lockfile rather than refusing, so without this
/// the first `up` of an unlocked project would resolve inputs and mutate
/// the tree before anything could object.
fn ensure_lock_present(project_root: &Path) -> Result<(), ProviderError> {
    if project_root.join("devenv.lock").is_file() {
        Ok(())
    } else {
        Err(ProviderError::MissingLock {
            provider: "devenv",
            hint: "devenv update",
        })
    }
}

fn read_lock(project_root: &Path) -> Option<Vec<u8>> {
    std::fs::read(project_root.join("devenv.lock")).ok()
}

/// Enforces "nothing resolves at `up`" by byte comparison rather than by
/// predicting which inputs devenv needs — the rule `add-devbox-provider`
/// recorded: predicting the key set means reimplementing the provider's
/// resolution rules, which drifts silently when they change.
///
/// Restores the original on mismatch so a refused `up` leaves the working
/// tree as it found it.
fn restore_lock_if_capture_resolved(
    project_root: &Path,
    before: &Option<Vec<u8>>,
) -> Result<(), ProviderError> {
    let after = read_lock(project_root);
    if &after == before {
        return Ok(());
    }

    let lock_path = project_root.join("devenv.lock");
    match before {
        Some(bytes) => std::fs::write(&lock_path, bytes),
        // Unreachable while `ensure_lock_present` runs first, and handled
        // anyway: a capture that creates a lockfile where the project had
        // none has still resolved at `up`.
        None => std::fs::remove_file(&lock_path),
    }
    .map_err(|e| {
        ProviderError::ResolutionFailed(format!(
            "devenv resolved during `up` and rewrote {}, and restoring it failed: {e}",
            lock_path.display()
        ))
    })?;

    Err(ProviderError::ResolutionFailed(format!(
        "devenv resolved inputs while capturing the environment and rewrote {} \
         (devcroft restored it); provisioning must not resolve revisions or contact a \
         package index — run `devenv update` and commit the result",
        lock_path.display()
    )))
}

/// The environment, through the route that does not run `enterShell`.
///
/// Two steps, because `devenv build shell` returns a *path* to the dump
/// rather than the dump: build, then read what it names.
fn capture_activated_env(
    devenv_bin: &Path,
    project_root: &Path,
    base: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, ProviderError> {
    let built = Command::new(devenv_bin)
        .arg("build")
        .arg("shell")
        .current_dir(project_root)
        .env_clear()
        .envs(base)
        .output()
        .map_err(|e| {
            ProviderError::ResolutionFailed(format!("running `devenv build shell`: {e}"))
        })?;

    if !built.status.success() {
        return Err(ProviderError::ResolutionFailed(format!(
            "`devenv build shell` exited with {}: {}",
            built.status,
            String::from_utf8_lossy(&built.stderr).trim()
        )));
    }

    let artifact = shell_artifact_path(&built.stdout)?;
    let dump = std::fs::read_to_string(&artifact).map_err(|e| {
        ProviderError::ResolutionFailed(format!(
            "reading the environment `devenv build shell` produced at {}: {e}",
            artifact.display()
        ))
    })?;

    let mut env = parse_declare_dump(&dump)?;
    strip_builder_variables(&mut env, base);
    Ok(env)
}

/// `devenv build shell` prints `{"shell": "/nix/store/…"}`. Parsed as
/// JSON rather than scraped, so a second output key or a changed shape
/// fails here instead of producing a path that happens to look right.
fn shell_artifact_path(stdout: &[u8]) -> Result<std::path::PathBuf, ProviderError> {
    #[derive(serde::Deserialize)]
    struct Built {
        shell: String,
    }

    let built: Built = serde_json::from_slice(stdout).map_err(|e| {
        ProviderError::ResolutionFailed(format!(
            "parsing `devenv build shell` output: {e} (got: {})",
            String::from_utf8_lossy(stdout).trim()
        ))
    })?;
    Ok(std::path::PathBuf::from(built.shell))
}

/// Parse the `declare -x NAME="value"` dump.
///
/// **Fails on anything it does not recognize**, rather than skipping it
/// (design.md decision 2). The artifact opens with devenv's own warning
/// that it is "an internal implementation detail for pkgs.mkShell", and
/// the price of consuming it is that a change to its shape must break
/// loudly: a partial environment that looks like a whole one is exactly
/// the failure the internal-artifact risk was accepted to avoid. So
/// everything before the first `declare -x` is the known banner, and
/// everything after it must be a `declare -x` line or blank.
fn parse_declare_dump(dump: &str) -> Result<BTreeMap<String, String>, ProviderError> {
    const PREFIX: &str = "declare -x ";
    let mut env = BTreeMap::new();
    let mut seen_declaration = false;

    for (index, line) in dump.lines().enumerate() {
        if !seen_declaration && !line.starts_with(PREFIX) {
            continue; // devenv's banner, above the first declaration.
        }
        if line.trim().is_empty() {
            continue;
        }
        let Some(rest) = line.strip_prefix(PREFIX) else {
            return Err(ProviderError::ResolutionFailed(format!(
                "`devenv build shell` produced a line devcroft does not recognize at line {}: \
                 {line:?}; devcroft refuses a partial environment rather than capturing one \
                 that looks complete (the artifact is documented as an internal implementation \
                 detail, so its format changing is expected to break here)",
                index + 1
            )));
        };
        seen_declaration = true;

        match rest.split_once('=') {
            // An exported-but-unset name has no representation in a
            // process environment, the same case `nix.rs` skips for a
            // non-string `exported` entry.
            None => continue,
            Some((name, value)) => {
                env.insert(name.to_string(), unquote(value));
            }
        }
    }

    if !seen_declaration {
        return Err(ProviderError::ResolutionFailed(
            "`devenv build shell` produced no environment; devcroft cannot tell an empty \
             activation from a broken one, so it refuses rather than building a sandbox with \
             none of the project's tooling in it"
                .to_string(),
        ));
    }
    Ok(env)
}

/// Undo bash's own quoting of a `declare -x` value: `"…"` for the common
/// case and `$'…'` for values carrying newlines (`shellHook`,
/// `buildPhase` — both of which are then dropped as builder variables,
/// but must parse before they can be dropped).
fn unquote(value: &str) -> String {
    if let Some(inner) = value.strip_prefix("$'").and_then(|v| v.strip_suffix('\'')) {
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        }
        return out;
    }

    let inner = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(escaped @ ('"' | '\\' | '$' | '`')) => out.push(escaped),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            },
            other => out.push(other),
        }
    }
    out
}

/// Names the Nix *builder* sets that must never reach a session
/// (design.md decision 8).
///
/// `devenv build shell` returns the environment of the derivation that
/// builds the dev shell, which is a superset of the one a developer gets.
/// These are derivation attributes and build-sandbox plumbing: they
/// describe how the shell was *made* and mean nothing once it runs.
///
/// **Why not devenv's own list.** `enterShell` opens by unsetting most of
/// these by name. devcroft cannot inherit that: the hook runs in its own
/// shell inside the sandbox, so its `unset` never reaches what the keeper
/// injects into every session — and the list omits the three names in
/// [`BUILDER_SENTINELS`] entirely.
///
/// Kept deliberately: `NIX_CFLAGS_COMPILE` and friends (toolchain
/// configuration a build needs), every `DEVENV_*` (the captured hook
/// reads `DEVENV_STATE` and `DEVENV_PROFILE`), and `SHELL`, which points
/// at a store bash and is a better answer than the host login shell the
/// hook-running route reports.
const BUILDER_VARIABLES: &[&str] = &[
    // stdenv's derivation attributes.
    "__structuredAttrs",
    "buildInputs",
    "buildPhase",
    "builder",
    "depsBuildBuild",
    "depsBuildBuildPropagated",
    "depsBuildTarget",
    "depsBuildTargetPropagated",
    "depsHostHost",
    "depsHostHostPropagated",
    "depsTargetTarget",
    "depsTargetTargetPropagated",
    "doCheck",
    "doInstallCheck",
    "dontAddDisableDepTrack",
    "name",
    "nativeBuildInputs",
    "out",
    "outputs",
    "patches",
    "phases",
    "preferLocalBuild",
    "propagatedBuildInputs",
    "propagatedNativeBuildInputs",
    "shell",
    "shellHook",
    "stdenv",
    "strictDeps",
    // The build sandbox's own plumbing.
    "GZIP_NO_TIMESTAMPS",
    "HOST_PATH",
    "NIX_BUILD_CORES",
    "NIX_BUILD_TOP",
    "NIX_ENFORCE_PURITY",
    "NIX_LOG_FD",
    "TERM",
    "TZ",
    // Position of the *build*, not of the session.
    "OLDPWD",
    "PWD",
    "SHLVL",
];

/// Names whose *value* decides whether they are the builder's or the
/// project's, paired with the marker that identifies the builder's.
///
/// These cannot go in [`BUILDER_VARIABLES`], and the reason is a bug this
/// filter had before the list was split: `SSL_CERT_FILE` is set to
/// `/no-cert-file.crt` by the build sandbox — but a project declaring
/// `pkgs.cacert` gets a *real* bundle under the same name, and dropping
/// that would break TLS for exactly the projects that took care to
/// configure it. Same shape for `HOME`, and for the temp directories,
/// which devenv's own hook rewrites conditionally for the same reason
/// ("only reset those that still point to the Nix build dir").
///
/// So each is dropped only when it still carries the builder's marker.
const BUILDER_SENTINELS: &[(&str, &str)] = &[
    ("HOME", "/homeless-shelter"),
    ("SSL_CERT_FILE", "/no-cert-file.crt"),
    ("NIX_SSL_CERT_FILE", "/no-cert-file.crt"),
    ("TMP", "/nix/var/nix/builds/"),
    ("TMPDIR", "/nix/var/nix/builds/"),
    ("TEMP", "/nix/var/nix/builds/"),
    ("TEMPDIR", "/nix/var/nix/builds/"),
];

/// Apply [`BUILDER_VARIABLES`] and [`BUILDER_SENTINELS`] to a captured
/// environment.
///
/// A name the baseline also carries is reset to the **baseline's** value
/// rather than removed: dropping it outright would put it in
/// [`capture::unset_env`]'s output, which tells the keeper to actively
/// unset it — so a filtered `HOME` would leave every session with no
/// `HOME` at all, which is worse than the wrong one this exists to
/// remove.
fn strip_builder_variables(env: &mut BTreeMap<String, String>, base: &BTreeMap<String, String>) {
    let doomed: Vec<String> = BUILDER_VARIABLES
        .iter()
        .map(|n| (*n).to_string())
        .chain(BUILDER_SENTINELS.iter().filter_map(|(name, marker)| {
            env.get(*name)
                .filter(|value| value.starts_with(marker))
                .map(|_| (*name).to_string())
        }))
        .collect();

    for name in doomed {
        match base.get(&name) {
            Some(baseline_value) => {
                env.insert(name, baseline_value.clone());
            }
            None => {
                env.remove(&name);
            }
        }
    }
}

/// `enterShell`, as data.
///
/// `devenv eval enterShell` returns it as JSON and does not run it
/// (measured, task 0.3). The `…-devenv-enterShell` derivation in the
/// closure holds the same text and is not used: it means locating a store
/// path by name pattern where a documented command exists.
///
/// What comes back is **not only the project's `enterShell` block** —
/// devenv wraps it in a generated preamble (temp-directory fixups,
/// `MANPATH`, a profile symlink, the builder-variable `unset`). devcroft
/// runs the whole of it inside the sandbox: the preamble is part of what
/// makes a devenv shell a devenv shell, and two of the variables the
/// hook-free capture lacks (`IN_NIX_SHELL`, `MANPATH`) come from it.
fn capture_enter_shell(
    devenv_bin: &Path,
    project_root: &Path,
    base: &BTreeMap<String, String>,
) -> Result<Option<String>, ProviderError> {
    #[derive(serde::Deserialize)]
    struct Evaluated {
        #[serde(rename = "enterShell")]
        enter_shell: String,
    }

    let output = Command::new(devenv_bin)
        .arg("eval")
        .arg("enterShell")
        .current_dir(project_root)
        .env_clear()
        .envs(base)
        .output()
        .map_err(|e| {
            ProviderError::ResolutionFailed(format!("running `devenv eval enterShell`: {e}"))
        })?;

    if !output.status.success() {
        return Err(ProviderError::ResolutionFailed(format!(
            "`devenv eval enterShell` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let evaluated: Evaluated = serde_json::from_slice(&output.stdout).map_err(|e| {
        // Never fall back to a route that runs the hook: that would
        // reintroduce host-side execution of project code, which is the
        // property this provider qualified on.
        ProviderError::ResolutionFailed(format!("parsing `devenv eval enterShell` output: {e}"))
    })?;

    Ok(match evaluated.enter_shell.trim().is_empty() {
        true => None,
        false => Some(evaluated.enter_shell),
    })
}

/// The processes a devenv project declares, read as data.
///
/// `devenv eval processes` returns every process the evaluation
/// produces, normalized, and runs `enterShell` zero times (measured,
/// task 0.1). That is the whole qualification: a project's services are
/// discoverable without executing any of its code, which is what devbox
/// cannot offer — `devbox services ls` runs `shell.init_hook`.
///
/// **What comes back is broader than what the project typed.** A devenv
/// integration (`services.redis.enable = true`) contributes a process
/// too, whose `exec` is a store path. Taken deliberately: devenv does
/// not mark which is which, and filtering would drop exactly the
/// services a user enabling an integration expects to be supervised
/// (design.md decision 4a).
fn capture_processes(
    devenv_bin: &Path,
    project_root: &Path,
    base: &BTreeMap<String, String>,
) -> Result<super::ServiceSupport, ProviderError> {
    let output = Command::new(devenv_bin)
        .arg("eval")
        .arg("processes")
        .current_dir(project_root)
        .env_clear()
        .envs(base)
        .output()
        .map_err(|e| {
            ProviderError::ResolutionFailed(format!("running `devenv eval processes`: {e}"))
        })?;

    if !output.status.success() {
        return Err(ProviderError::ResolutionFailed(format!(
            "`devenv eval processes` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let parsed: DeclaredProcesses = serde_json::from_slice(&output.stdout).map_err(|e| {
        // Loud rather than empty: a shape devcroft cannot read is not the
        // same as a project with no services, and reporting it as the
        // latter would start a sandbox missing everything it declared.
        ProviderError::ResolutionFailed(format!("parsing `devenv eval processes` output: {e}"))
    })?;

    let names: BTreeSet<&str> = parsed.processes.keys().map(String::as_str).collect();
    let mut declared = Vec::with_capacity(parsed.processes.len());
    for (name, process) in &parsed.processes {
        declared.push(translate_process(name, process, &names)?);
    }
    // `Declared`, never `Unsupported`, even when empty: "this provider
    // has no service concept" and "this provider has one and the project
    // declared nothing" are different facts, and collapsing them is what
    // would let a manifest asking for services silently start nothing.
    Ok(super::ServiceSupport::Declared(declared))
}

#[derive(serde::Deserialize)]
struct DeclaredProcesses {
    processes: BTreeMap<String, DevenvProcess>,
}

/// One process as `devenv eval processes` returns it.
///
/// Every field devenv emits is named here, including the ones devcroft
/// refuses — that is the point. A field devcroft does not mention would
/// be dropped by serde without anyone noticing, which is precisely the
/// silence `fix-lossy-service-translation` exists to remove.
#[derive(serde::Deserialize)]
struct DevenvProcess {
    exec: String,
    #[serde(default)]
    env: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    before: Vec<String>,
    #[serde(default)]
    after: Vec<String>,
    #[serde(default)]
    restart: Option<DevenvRestart>,
    #[serde(default)]
    shutdown: Option<DevenvShutdown>,
    #[serde(default)]
    start: Option<DevenvStart>,
    #[serde(rename = "supervisionMode", default)]
    supervision_mode: Option<String>,
    // Refused, not carried — see `refuse_unsupported`.
    #[serde(default)]
    ready: Option<serde_json::Value>,
    #[serde(default)]
    listen: Vec<serde_json::Value>,
    #[serde(default)]
    ports: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    watch: Option<DevenvWatch>,
    #[serde(default)]
    proxy: Option<DevenvProxy>,
    #[serde(default)]
    linux: Option<DevenvLinux>,
    #[serde(rename = "process-compose", default)]
    process_compose: BTreeMap<String, serde_json::Value>,
}

#[derive(serde::Deserialize)]
struct DevenvRestart {
    #[serde(default)]
    on: Option<String>,
    #[serde(default)]
    max: Option<u32>,
}

#[derive(serde::Deserialize)]
struct DevenvShutdown {
    #[serde(default)]
    signal: Option<i32>,
    #[serde(default)]
    grace: Option<u64>,
}

#[derive(serde::Deserialize)]
struct DevenvStart {
    #[serde(default)]
    enable: Option<bool>,
}

#[derive(serde::Deserialize)]
struct DevenvWatch {
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    extensions: Vec<String>,
}

#[derive(serde::Deserialize)]
struct DevenvProxy {
    #[serde(default)]
    hostname: Option<String>,
}

#[derive(serde::Deserialize)]
struct DevenvLinux {
    #[serde(default)]
    capabilities: Vec<String>,
}

/// The prefix devenv's task graph uses for a process node.
const PROCESS_TASK_PREFIX: &str = "devenv:processes:";

fn translate_process(
    name: &str,
    process: &DevenvProcess,
    declared_names: &BTreeSet<&str>,
) -> Result<super::ServiceDecl, ProviderError> {
    refuse_unsupported(name, process)?;

    let mut vars = BTreeMap::new();
    for (key, value) in &process.env {
        // devenv's `env` values are typed in Nix and arrive as JSON.
        // Numbers and booleans are legitimate and become their obvious
        // string form; anything structured has no representation in a
        // process environment and is refused rather than stringified
        // into something the service would misread.
        let rendered = match value {
            serde_json::Value::String(v) => v.clone(),
            serde_json::Value::Number(v) => v.to_string(),
            serde_json::Value::Bool(v) => v.to_string(),
            other => {
                return Err(unsupported(
                    name,
                    &format!("env.{key}"),
                    &format!("its value is {other}, which has no process-environment form"),
                ));
            }
        };
        vars.insert(key.clone(), rendered);
    }

    Ok(super::ServiceDecl {
        name: name.to_string(),
        command: process.exec.clone(),
        vars,
        // devenv has no is-daemon concept; the field is flox's and stays
        // flox's rather than being given an invented devenv meaning.
        is_daemon: false,
        working_dir: process.cwd.clone(),
        depends_on: ordering_dependencies(name, process, declared_names)?,
        restart: restart_policy(name, process)?,
        shutdown: match &process.shutdown {
            Some(DevenvShutdown {
                signal: Some(signal),
                grace,
            }) => super::Shutdown::Signal {
                signal: *signal,
                grace: grace.unwrap_or(0),
            },
            _ => super::Shutdown::Default,
        },
    })
}

/// devenv's `before`/`after`, reduced to service dependencies — or
/// refused (design.md decision 2a).
///
/// **These are edges in devenv's *task* graph, not process references**,
/// and devenv validates neither (measured, task 0.4). Only one form
/// means the same thing on both sides: `devenv:processes:<name>` naming
/// another process this project declares. devenv builds exactly that
/// edge, and devcroft's supervisor can reproduce it.
///
/// Every other form is refused, and the bare-name case is the one that
/// matters. `after = [ "web" ]` is the spelling a user reaches for, it
/// produces **no edge at all** in devenv — measured, not assumed — and
/// honouring it would make devcroft order a service the provider does
/// not. The two would then disagree about what the same project does,
/// with devcroft the more featureful of the pair, which is the wrong
/// direction for a tool whose job is to run the project's own
/// environment.
fn ordering_dependencies(
    name: &str,
    process: &DevenvProcess,
    declared_names: &BTreeSet<&str>,
) -> Result<Vec<String>, ProviderError> {
    // `before` is the mirror of `after`, and reversing it would mean
    // rewriting another service's dependencies from this one's
    // declaration — possible, but it makes the translation order-
    // dependent for no measured need. Refused until a project wants it.
    if let Some(entry) = process.before.first() {
        return Err(unsupported(
            name,
            "before",
            &format!(
                "devcroft carries ordering as a dependency on the service that must start                  first; declare `after = [ \"{PROCESS_TASK_PREFIX}{name}\" ]` on `{entry}`                  instead"
            ),
        ));
    }

    let mut deps = Vec::new();
    for entry in &process.after {
        let Some(target) = entry.strip_prefix(PROCESS_TASK_PREFIX) else {
            return Err(unsupported(
                name,
                "after",
                &match entry.contains(':') {
                    // A real task node devcroft never runs.
                    true => format!(
                        "`{entry}` names a devenv task rather than one of this project's                          processes, and devcroft does not run devenv's task graph"
                    ),
                    // The spelling that looks right and does nothing.
                    false => format!(
                        "`{entry}` is a bare name, which devenv itself creates no ordering                          from — write `{PROCESS_TASK_PREFIX}{entry}` if you meant the                          process by that name"
                    ),
                },
            ));
        };
        if !declared_names.contains(target) {
            return Err(unsupported(
                name,
                "after",
                &format!("`{entry}` names a process this project does not declare"),
            ));
        }
        deps.push(target.to_string());
    }
    Ok(deps)
}

/// devenv's `restart`, which every process carries — its default is
/// `on_failure` with `max = 5` (measured, task 0.4).
///
/// Honoured rather than overridden by devcroft's own `Never`. See
/// `RestartPolicy`'s doc comment: `add-flox-services` chose `Never` in
/// the absence of a declaration, which is still what a provider
/// declaring nothing gets; a provider that declares one gets what it
/// declared (design.md decision 2b).
fn restart_policy(
    name: &str,
    process: &DevenvProcess,
) -> Result<super::RestartPolicy, ProviderError> {
    let Some(restart) = &process.restart else {
        return Ok(super::RestartPolicy::Never);
    };
    let max = restart.max.unwrap_or(0);
    match restart.on.as_deref() {
        None | Some("no") | Some("never") => Ok(super::RestartPolicy::Never),
        Some("on_failure") | Some("on-failure") => Ok(super::RestartPolicy::OnFailure { max }),
        Some("always") => Ok(super::RestartPolicy::Always { max }),
        Some(other) => Err(unsupported(
            name,
            "restart.on",
            &format!("`{other}` is not a restart policy devcroft can express"),
        )),
    }
}

/// Everything devenv declares that devcroft will not carry, refused by
/// name (design.md decision 3).
///
/// Stricter than a first version needs, deliberately. A project that
/// declares a readiness probe and gets a service without one has been
/// lied to: the service reports healthy on a condition nobody checked.
/// A refusal is recoverable — remove the field, or wait for support. A
/// silent drop is not, because nothing surfaces it.
fn refuse_unsupported(name: &str, process: &DevenvProcess) -> Result<(), ProviderError> {
    if process.ready.is_some() {
        return Err(unsupported(
            name,
            "ready",
            "devcroft does not yet translate readiness probes, and will not report a service              healthy on a condition it never checked",
        ));
    }
    if !process.listen.is_empty() {
        return Err(unsupported(
            name,
            "listen",
            "socket activation overlaps devcroft's own port policy rather than sitting beside              it (`network.ports`)",
        ));
    }
    if !process.ports.is_empty() {
        return Err(unsupported(
            name,
            "ports",
            "port declarations overlap devcroft's own port policy rather than sitting beside              it (`network.ports`)",
        ));
    }
    if let Some(watch) = &process.watch
        && (!watch.paths.is_empty() || !watch.extensions.is_empty())
    {
        return Err(unsupported(
            name,
            "watch",
            "devcroft's keeper supervises services; it does not restart them on file changes",
        ));
    }
    if process.proxy.as_ref().is_some_and(|p| p.hostname.is_some()) {
        return Err(unsupported(
            name,
            "proxy",
            "devcroft has no HTTP proxy for services to be published through",
        ));
    }
    if process
        .linux
        .as_ref()
        .is_some_and(|l| !l.capabilities.is_empty())
    {
        return Err(unsupported(
            name,
            "linux.capabilities",
            "a sandbox grants no Linux capabilities to project code, which is the boundary              rather than an omission",
        ));
    }
    if process
        .start
        .as_ref()
        .is_some_and(|s| s.enable == Some(false))
    {
        return Err(unsupported(
            name,
            "start.enable",
            "devcroft starts every declared service when the sandbox comes up and has no              concept of one that is declared but not started",
        ));
    }
    // Measured (task 0.3): the option is read-only upstream, so this is
    // insurance against devenv emitting a new value rather than a
    // restriction on projects — and the message says so, since a user
    // told to change a read-only option would be sent looking for
    // something that does not exist.
    if let Some(mode) = &process.supervision_mode
        && mode != "native"
    {
        return Err(unsupported(
            name,
            "supervisionMode",
            &format!(
                "devcroft supervises services itself and can only carry `native`; `{mode}`                  came from devenv rather than from this project, since the option is                  read-only upstream"
            ),
        ));
    }
    if !process.process_compose.is_empty() {
        // Would work today — devcroft's supervisor *is* process-compose.
        // Refused because `ServiceDecl` is supervisor-neutral by design
        // and a passthrough would make every future supervisor answer
        // "what do I do with the process-compose block", which is the
        // coupling `decouple-service-supervisor` removed.
        return Err(unsupported(
            name,
            "process-compose",
            "devcroft generates its own supervisor configuration and carries no              supervisor-specific block; express a dependency as              `after = [ \"devenv:processes:<name>\" ]` instead",
        ));
    }
    Ok(())
}

fn unsupported(service: &str, field: &str, why: &str) -> ProviderError {
    ProviderError::ResolutionFailed(format!(
        "devenv process `{service}` declares `{field}`, which devcroft cannot carry: {why}.          devcroft refuses rather than starting a service that differs from what the project          declared"
    ))
}

/// Content fingerprint for staleness: `devenv.nix` + `devenv.yaml` +
/// `devenv.lock`.
///
/// Three files, not two. `devenv.yaml` carries the environment's inputs,
/// so it can change what resolves with `devenv.nix` untouched — omitting
/// it would report a changed environment as fresh, the failure staleness
/// detection exists to prevent.
pub fn devenv_fingerprint(project_root: &Path) -> Result<String, ProviderError> {
    ensure_project_present(project_root)?;
    let nix_path = project_root.join("devenv.nix");
    let nix = std::fs::read(&nix_path).map_err(|e| {
        ProviderError::ResolutionFailed(format!("reading {}: {e}", nix_path.display()))
    })?;
    let yaml = capture::optional_file_part(&project_root.join("devenv.yaml"));
    let lock = capture::optional_file_part(&project_root.join("devenv.lock"));

    Ok(capture::fingerprint(&[&nix, &yaml, &lock]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ServiceDecl, ServiceSupport};
    use std::fs;
    use std::path::PathBuf;

    fn tempdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-devenv-test-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Writes the three declaration files. `lock` is `Option` because
    /// half the preconditions under test are about its absence.
    fn write_devenv_project(root: &Path, devenv_nix: &str, lock: Option<&str>) {
        fs::write(root.join("devenv.nix"), devenv_nix).unwrap();
        fs::write(
            root.join("devenv.yaml"),
            "inputs:\n  nixpkgs:\n    url: github:cachix/devenv-nixpkgs/rolling\n",
        )
        .unwrap();
        if let Some(lock) = lock {
            fs::write(root.join("devenv.lock"), lock).unwrap();
        }
    }

    /// Whether this host can actually *build* a devenv closure, not merely
    /// whether the binary exists — `devenv version` succeeds against an
    /// unreachable store, and every test that then resolved an
    /// environment failed in a way that reads as a devcroft regression
    /// (CLAUDE.md's standing rule; `ensure_nix_usable`'s own doc comment
    /// names it).
    fn devenv_can_resolve() -> Option<PathBuf> {
        let devenv = resolve_on_path("devenv").filter(|devenv| {
            Command::new(devenv)
                .arg("version")
                .output()
                .is_ok_and(|o| o.status.success())
        })?;
        resolve_on_path("nix")?;
        crate::provider::host_can_build_nix_closures().then_some(devenv)
    }

    /// Materializes `devenv.lock` the way a real project would. Returns
    /// false when inputs cannot be fetched at all (no network), so callers
    /// skip rather than fail.
    fn devenv_update(devenv: &Path, root: &Path) -> bool {
        Command::new(devenv)
            .arg("update")
            .current_dir(root)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[test]
    fn resolve_fails_with_no_environment_when_devenv_nix_missing() {
        let root = tempdir("no-env");
        let err = DevenvProvider.resolve(&root).unwrap_err();
        assert_eq!(
            err,
            ProviderError::NoEnvironment {
                provider: "devenv",
                hint: "devenv init"
            }
        );
    }

    /// The precondition group 0 turned from symmetry with devbox into a
    /// correctness requirement: `devenv build shell` on an unlocked
    /// project *writes* the lockfile rather than refusing, so without this
    /// the first `up` would resolve inputs at `up` and mutate the tree.
    #[test]
    fn ensure_lock_present_fails_with_the_command_that_writes_it() {
        let root = tempdir("no-lock");
        write_devenv_project(&root, "{ pkgs, ... }: { }", None);
        let err = ensure_lock_present(&root).unwrap_err();
        assert_eq!(
            err,
            ProviderError::MissingLock {
                provider: "devenv",
                hint: "devenv update"
            }
        );
    }

    #[test]
    fn ensure_lock_present_accepts_a_locked_project() {
        let root = tempdir("has-lock");
        write_devenv_project(&root, "{ pkgs, ... }: { }", Some("{}"));
        assert!(ensure_lock_present(&root).is_ok());
    }

    #[test]
    fn parse_declare_dump_reads_values_past_devenvs_banner() {
        let dump = "------------------------------\n WARNING: internal detail.\n------------------------------\n\ndeclare -x CC=\"clang\"\ndeclare -x DEVCROFT_PROBE=\"measured\"\n";
        let env = parse_declare_dump(dump).unwrap();
        assert_eq!(env.get("CC").map(String::as_str), Some("clang"));
        assert_eq!(
            env.get("DEVCROFT_PROBE").map(String::as_str),
            Some("measured")
        );
        assert_eq!(env.len(), 2);
    }

    /// design.md decision 2's first obligation. The artifact documents
    /// itself as an internal implementation detail, and the price of
    /// consuming it is that a change in its shape stops `up` instead of
    /// producing a partial environment that looks complete.
    #[test]
    fn parse_declare_dump_fails_on_unrecognized_content() {
        let dump = "declare -x CC=\"clang\"\nsome_function () { :; }\n";
        let err = parse_declare_dump(dump).unwrap_err();
        match err {
            ProviderError::ResolutionFailed(msg) => {
                assert!(msg.contains("does not recognize"), "got: {msg}");
                assert!(msg.contains("some_function"), "got: {msg}");
            }
            other => panic!("expected ResolutionFailed naming the line, got {other:?}"),
        }
    }

    /// A dump with no declarations at all is a broken capture, not an
    /// environment that happens to add nothing — the distinction
    /// `devbox.rs` had to learn the hard way when a failing `shellenv`
    /// produced `Ok` with an empty environment.
    #[test]
    fn parse_declare_dump_refuses_a_dump_with_no_declarations() {
        let err = parse_declare_dump("--------\n WARNING\n--------\n").unwrap_err();
        match err {
            ProviderError::ResolutionFailed(msg) => {
                assert!(msg.contains("no environment"), "got: {msg}")
            }
            other => panic!("expected ResolutionFailed about an empty capture, got {other:?}"),
        }
    }

    #[test]
    fn unquote_handles_bashs_two_quoting_forms() {
        assert_eq!(unquote(r#""plain""#), "plain");
        assert_eq!(
            unquote(r#""with \"quotes\" in it""#),
            r#"with "quotes" in it"#
        );
        assert_eq!(
            unquote(r#""a \$dollar and a \\backslash""#),
            r"a $dollar and a \backslash"
        );
        assert_eq!(unquote(r"$'first\nsecond'"), "first\nsecond");
    }

    /// design.md decision 8. The three that matter are not untidiness:
    /// a session with `HOME=/homeless-shelter` has no home, one with
    /// `SSL_CERT_FILE=/no-cert-file.crt` has no TLS, and one with `TMPDIR`
    /// pointing into a Nix build directory has no temp directory — each
    /// failing in a way that reads as a devcroft bug.
    #[test]
    fn builder_variables_do_not_survive_capture() {
        let base = BTreeMap::from([
            ("HOME".to_string(), "/home/real".to_string()),
            ("PATH".to_string(), "/usr/bin".to_string()),
        ]);
        let mut env = BTreeMap::from([
            ("HOME".to_string(), "/homeless-shelter".to_string()),
            ("SSL_CERT_FILE".to_string(), "/no-cert-file.crt".to_string()),
            (
                "TMPDIR".to_string(),
                "/nix/var/nix/builds/nix-1".to_string(),
            ),
            ("out".to_string(), "/nix/store/x-devenv-shell".to_string()),
            ("shellHook".to_string(), "unset stdenv".to_string()),
            ("DEVENV_PROFILE".to_string(), "/nix/store/y".to_string()),
            ("CC".to_string(), "clang".to_string()),
        ]);

        strip_builder_variables(&mut env, &base);

        assert!(!env.contains_key("SSL_CERT_FILE"));
        assert!(!env.contains_key("TMPDIR"));
        assert!(!env.contains_key("out"));
        assert!(!env.contains_key("shellHook"));
        // Kept: the captured hook reads DEVENV_*, and CC is the point.
        assert_eq!(
            env.get("DEVENV_PROFILE").map(String::as_str),
            Some("/nix/store/y")
        );
        assert_eq!(env.get("CC").map(String::as_str), Some("clang"));
    }

    /// `HOME` is filtered by being reset to the baseline's value rather
    /// than removed. Removing it would put it in `capture::unset_env`'s
    /// output — which tells the keeper to actively unset it — leaving
    /// every session with no `HOME` at all, which is worse than the wrong
    /// one this filter exists to remove.
    #[test]
    fn a_filtered_baseline_key_is_neutralized_rather_than_unset() {
        let base = BTreeMap::from([
            ("HOME".to_string(), "/home/real".to_string()),
            ("PATH".to_string(), "/usr/bin".to_string()),
        ]);
        let mut env = base.clone();
        env.insert("HOME".to_string(), "/homeless-shelter".to_string());

        strip_builder_variables(&mut env, &base);

        assert_eq!(env.get("HOME").map(String::as_str), Some("/home/real"));
        assert!(!capture::changed_env(&base, &env).contains_key("HOME"));
        assert!(!capture::unset_env(&base, &env).contains(&"HOME".to_string()));
    }

    /// The half a name-only filter got wrong: `SSL_CERT_FILE` is the
    /// builder's sentinel *or* a real bundle from a project that declared
    /// `pkgs.cacert`, under the same name. Dropping it unconditionally
    /// would break TLS for exactly the projects that configured it.
    #[test]
    fn a_real_certificate_bundle_survives_while_the_builders_sentinel_does_not() {
        let base = BTreeMap::from([("PATH".to_string(), "/usr/bin".to_string())]);

        let mut real = BTreeMap::from([(
            "SSL_CERT_FILE".to_string(),
            "/nix/store/abc-cacert/etc/ssl/certs/ca-bundle.crt".to_string(),
        )]);
        strip_builder_variables(&mut real, &base);
        assert_eq!(
            real.get("SSL_CERT_FILE").map(String::as_str),
            Some("/nix/store/abc-cacert/etc/ssl/certs/ca-bundle.crt"),
            "a project's own certificate bundle must survive the filter"
        );

        let mut sentinel =
            BTreeMap::from([("SSL_CERT_FILE".to_string(), "/no-cert-file.crt".to_string())]);
        strip_builder_variables(&mut sentinel, &base);
        assert!(!sentinel.contains_key("SSL_CERT_FILE"));
    }

    /// Same rule for the temp directories, which is also what devenv's
    /// own hook does ("only reset those that still point to the Nix build
    /// dir; leave any user/CI-supplied value intact").
    #[test]
    fn a_project_supplied_tmpdir_survives_while_the_build_dir_does_not() {
        let base = BTreeMap::from([("PATH".to_string(), "/usr/bin".to_string())]);

        let mut supplied =
            BTreeMap::from([("TMPDIR".to_string(), "/workspace/scratch".to_string())]);
        strip_builder_variables(&mut supplied, &base);
        assert_eq!(
            supplied.get("TMPDIR").map(String::as_str),
            Some("/workspace/scratch")
        );

        let mut build_dir = BTreeMap::from([(
            "TMPDIR".to_string(),
            "/nix/var/nix/builds/nix-1-2".to_string(),
        )]);
        strip_builder_variables(&mut build_dir, &base);
        assert!(!build_dir.contains_key("TMPDIR"));
    }

    #[test]
    fn shell_artifact_path_reads_the_documented_key() {
        let path = shell_artifact_path(br#"{"shell": "/nix/store/x-devenv-shell"}"#).unwrap();
        assert_eq!(path, PathBuf::from("/nix/store/x-devenv-shell"));
    }

    #[test]
    fn shell_artifact_path_fails_on_an_unexpected_shape() {
        let err = shell_artifact_path(b"/nix/store/x-devenv-shell\n").unwrap_err();
        match err {
            ProviderError::ResolutionFailed(msg) => {
                assert!(msg.contains("devenv build shell"), "got: {msg}")
            }
            other => panic!("expected ResolutionFailed, got {other:?}"),
        }
    }

    #[test]
    fn fingerprint_changes_when_any_of_the_three_files_changes() {
        let root = tempdir("fingerprint-three");
        write_devenv_project(&root, "{ pkgs, ... }: { }", Some("{}"));
        let base = devenv_fingerprint(&root).unwrap();

        fs::write(root.join("devenv.nix"), "{ pkgs, ... }: { packages = []; }").unwrap();
        let after_nix = devenv_fingerprint(&root).unwrap();
        assert_ne!(base, after_nix, "devenv.nix must affect the fingerprint");

        fs::write(root.join("devenv.yaml"), "inputs: {}\n").unwrap();
        let after_yaml = devenv_fingerprint(&root).unwrap();
        assert_ne!(
            after_nix, after_yaml,
            "devenv.yaml carries the inputs: it can change what resolves with devenv.nix \
             untouched, so omitting it would report a changed environment as fresh"
        );

        fs::write(root.join("devenv.lock"), r#"{"nodes":{}}"#).unwrap();
        let after_lock = devenv_fingerprint(&root).unwrap();
        assert_ne!(after_yaml, after_lock, "devenv.lock must affect it");
    }

    #[test]
    fn fingerprint_ignores_unrelated_files() {
        let root = tempdir("fingerprint-unrelated");
        write_devenv_project(&root, "{ pkgs, ... }: { }", Some("{}"));
        let before = devenv_fingerprint(&root).unwrap();
        fs::write(root.join("README.md"), "unrelated").unwrap();
        assert_eq!(before, devenv_fingerprint(&root).unwrap());
    }

    #[test]
    fn fingerprint_is_stable_for_unchanged_content() {
        let root = tempdir("fingerprint-stable");
        write_devenv_project(&root, "{ pkgs, ... }: { }", Some("{}"));
        assert_eq!(
            devenv_fingerprint(&root).unwrap(),
            devenv_fingerprint(&root).unwrap()
        );
    }

    fn process_json(extra: &str) -> DevenvProcess {
        let body = match extra.is_empty() {
            true => r#"{"exec": "sleep 60"}"#.to_string(),
            false => format!(r#"{{"exec": "sleep 60", {extra}}}"#),
        };
        serde_json::from_str(&body).expect("fixture parses")
    }

    fn translate(name: &str, extra: &str, siblings: &[&str]) -> Result<ServiceDecl, ProviderError> {
        let declared: BTreeSet<&str> = siblings.iter().copied().chain([name]).collect();
        translate_process(name, &process_json(extra), &declared)
    }

    fn refusal(name: &str, extra: &str) -> String {
        match translate(name, extra, &[]) {
            Err(ProviderError::ResolutionFailed(msg)) => msg,
            other => panic!("expected a refusal for {extra}, got {other:?}"),
        }
    }

    #[test]
    fn a_plain_process_becomes_a_service() {
        let svc = translate(
            "web",
            r#""env": {"PORT": 8080, "DEBUG": true, "NAME": "x"}"#,
            &[],
        )
        .unwrap();
        assert_eq!(svc.name, "web");
        assert_eq!(svc.command, "sleep 60");
        assert_eq!(svc.vars.get("PORT").map(String::as_str), Some("8080"));
        assert_eq!(svc.vars.get("DEBUG").map(String::as_str), Some("true"));
        assert_eq!(svc.vars.get("NAME").map(String::as_str), Some("x"));
        // devenv has no is-daemon concept, so the field stays flox's.
        assert!(!svc.is_daemon);
    }

    /// A structured `env` value has no process-environment form.
    /// Stringifying it would hand the service something it would
    /// misread, which is worse than refusing.
    #[test]
    fn a_structured_env_value_is_refused_rather_than_stringified() {
        let msg = refusal("web", r#""env": {"OPTS": ["a", "b"]}"#);
        assert!(msg.contains("env.OPTS"), "got: {msg}");
    }

    #[test]
    fn restart_and_shutdown_are_carried() {
        let svc = translate(
            "web",
            r#""restart": {"on": "on_failure", "max": 5}, "shutdown": {"signal": 15, "grace": 5}"#,
            &[],
        )
        .unwrap();
        assert_eq!(
            svc.restart,
            crate::provider::RestartPolicy::OnFailure { max: 5 }
        );
        assert_eq!(
            svc.shutdown,
            crate::provider::Shutdown::Signal {
                signal: 15,
                grace: 5
            }
        );
    }

    /// `add-flox-services` chose `Never` in the absence of a declaration,
    /// and that is still what a provider declaring nothing gets — the
    /// case design.md decision 2b keeps intact.
    #[test]
    fn a_process_declaring_no_restart_policy_keeps_devcrofts_default() {
        assert_eq!(
            translate("web", "", &[]).unwrap().restart,
            crate::provider::RestartPolicy::Never
        );
    }

    /// design.md decision 2a. The one form that means the same thing on
    /// both sides.
    #[test]
    fn a_qualified_process_reference_becomes_a_dependency() {
        let svc = translate("worker", r#""after": ["devenv:processes:web"]"#, &["web"]).unwrap();
        assert_eq!(svc.depends_on, vec!["web".to_string()]);
    }

    /// **The refusal that matters.** A bare name is the spelling a user
    /// reaches for, and devenv itself creates no edge from it (measured,
    /// task 0.4) — so honouring it would make devcroft order a service
    /// the provider does not, and the two would disagree about the same
    /// project.
    #[test]
    fn a_bare_name_is_refused_and_the_message_says_devenv_ignores_it() {
        let msg = refusal("worker", r#""after": ["web"]"#);
        assert!(msg.contains("bare name"), "got: {msg}");
        assert!(
            msg.contains("devenv:processes:web"),
            "the message must show the spelling that works, got: {msg}"
        );
    }

    #[test]
    fn a_task_reference_devcroft_never_runs_is_refused() {
        let msg = refusal("worker", r#""after": ["devenv:enterShell"]"#);
        assert!(msg.contains("task graph"), "got: {msg}");
    }

    /// devenv does not validate these (measured, 0.4), so devcroft must,
    /// or a sandbox comes up ordered against a service that never exists.
    #[test]
    fn a_dependency_on_an_undeclared_process_is_refused() {
        let msg = match translate("worker", r#""after": ["devenv:processes:ghost"]"#, &[]) {
            Err(ProviderError::ResolutionFailed(msg)) => msg,
            other => panic!("expected a refusal, got {other:?}"),
        };
        assert!(msg.contains("does not declare"), "got: {msg}");
    }

    #[test]
    fn before_is_refused_with_the_equivalent_after_spelling() {
        let msg = refusal("web", r#""before": ["devenv:processes:worker"]"#);
        assert!(msg.contains("after"), "got: {msg}");
    }

    /// design.md decision 3: every one of these fails by name rather than
    /// being dropped. A refusal that says only "cannot translate" is not
    /// the requirement — the user has to know which field to remove.
    #[test]
    fn every_unsupported_field_is_refused_by_name() {
        for (field, extra) in [
            ("ready", r#""ready": {"http": {"path": "/"}}"#),
            ("listen", r#""listen": ["tcp://127.0.0.1:8080"]"#),
            ("ports", r#""ports": {"http": 8080}"#),
            ("watch", r#""watch": {"paths": ["src"], "extensions": []}"#),
            ("proxy", r#""proxy": {"hostname": "app.local"}"#),
            (
                "linux.capabilities",
                r#""linux": {"capabilities": ["CAP_NET_BIND_SERVICE"]}"#,
            ),
            ("start.enable", r#""start": {"enable": false}"#),
            ("supervisionMode", r#""supervisionMode": "systemd""#),
            (
                "process-compose",
                r#""process-compose": {"availability": {"restart": "always"}}"#,
            ),
        ] {
            let msg = refusal("web", extra);
            assert!(
                msg.contains(field),
                "the refusal for {field} must name it, got: {msg}"
            );
            assert!(
                msg.contains("web"),
                "it must also name the process, got: {msg}"
            );
        }
    }

    /// Measured (task 0.3): the option is read-only upstream, so a user
    /// told to change it would be sent looking for a setting that does
    /// not exist.
    #[test]
    fn the_supervision_mode_refusal_says_it_came_from_devenv() {
        let msg = refusal("web", r#""supervisionMode": "systemd""#);
        assert!(msg.contains("read-only"), "got: {msg}");
    }

    /// Would work today, since devcroft's supervisor *is* process-compose
    /// — refused to keep `ServiceDecl` supervisor-neutral, and the
    /// message has to offer the way that does work.
    #[test]
    fn the_process_compose_refusal_points_at_the_spelling_that_works() {
        let msg = refusal("web", r#""process-compose": {"depends_on": {"db": {}}}"#);
        assert!(msg.contains("devenv:processes:"), "got: {msg}");
    }

    #[test]
    fn defaults_devenv_always_emits_are_carried_without_refusal() {
        // Exactly what `devenv eval processes` returns for a process that
        // declares nothing but `exec` — measured, task 0.4. If any of
        // these tripped a refusal, every devenv project would fail.
        let svc = translate(
            "web",
            r#""env": {}, "cwd": null, "ready": null, "before": [], "after": [],
               "restart": {"max": 5, "on": "on_failure", "window": null},
               "shutdown": {"signal": 15, "grace": 5}, "listen": [], "ports": {},
               "watch": {"paths": [], "extensions": [], "ignore": []},
               "proxy": {"hostname": null, "https": {"enable": false}},
               "linux": {"capabilities": []}, "start": {"enable": true},
               "supervisionMode": "native", "process-compose": {}"#,
            &[],
        )
        .unwrap();
        assert_eq!(
            svc.restart,
            crate::provider::RestartPolicy::OnFailure { max: 5 }
        );
        assert!(svc.depends_on.is_empty());
        assert_eq!(svc.working_dir, None);
    }

    /// The property devenv qualified on (criterion 4), asserted live with
    /// the sentinel method the proposal was written with rather than by
    /// reading devenv's output: an `enterShell` with an observable side
    /// effect outside the project root leaves it untouched across a full
    /// resolution.
    #[test]
    fn build_shell_does_not_run_enter_shell() {
        let Some(devenv) = devenv_can_resolve() else {
            eprintln!("skipping: this host cannot build nix closures");
            return;
        };
        let root = tempdir("hook-does-not-run");
        let sentinel =
            std::env::temp_dir().join(format!("devcroft-devenv-sentinel-{}", std::process::id()));
        let _ = fs::remove_file(&sentinel);
        write_devenv_project(
            &root,
            &format!(
                "{{ pkgs, ... }}: {{\n  processes.web.exec = \"sleep 60\";\n  \
                 enterShell = ''\n    echo ran >> {}\n  '';\n}}\n",
                sentinel.display()
            ),
            None,
        );
        if !devenv_update(&devenv, &root) {
            eprintln!("skipping: devenv update could not fetch inputs");
            return;
        }

        let resolution = DevenvProvider.resolve(&root).unwrap();

        assert!(!resolution.ran_activation_hook);
        assert!(
            !sentinel.exists(),
            "enterShell ran during resolution; the sentinel outside the project root was written"
        );
        // Captured as data for the sandbox to run, which is the other
        // half: a provider that simply never produced the hook would pass
        // the assertion above too.
        let script = resolution
            .activation_script
            .expect("enterShell must be captured for the sandbox to run");
        assert!(
            script.contains(&sentinel.display().to_string()),
            "the captured script must contain the project's own enterShell"
        );
        // The services half of the same guarantee: the declarations are
        // read from the same project, through `devenv eval processes`,
        // and the sentinel above proves reading them ran no project code
        // either (`add-devenv-services` task 4.1).
        let ServiceSupport::Declared(services) = resolution.services else {
            panic!("devenv declares services; it must never report Unsupported");
        };
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "web");
        assert_eq!(services[0].command, "sleep 60");
    }

    /// Group 0's measurement, as a test: the captured environment must
    /// carry a shell `crate::shell` can resolve out of the closure. A
    /// capture that passes every other check and has no resolvable shell
    /// produces a working `exec` and a broken login session.
    #[test]
    fn the_captured_environment_yields_a_shell_from_the_closure() {
        let Some(devenv) = devenv_can_resolve() else {
            eprintln!("skipping: this host cannot build nix closures");
            return;
        };
        let root = tempdir("closure-shell");
        write_devenv_project(&root, "{ pkgs, ... }: { }", None);
        if !devenv_update(&devenv, &root) {
            eprintln!("skipping: devenv update could not fetch inputs");
            return;
        }

        let resolution = DevenvProvider.resolve(&root).unwrap();
        let shell = crate::shell::resolve(&resolution.env, &resolution.read_only_grants)
            .expect("a devenv closure must yield a shell devcroft can resolve");

        assert!(
            shell.path.is_absolute(),
            "resolved shell must be absolute, got {}",
            shell.path.display()
        );
    }

    /// The working-tree property (design.md decision 6): capture may write
    /// under `.devenv/`, devenv's own cache and GC-root store, and nowhere
    /// else. The declaration files are byte-identical afterwards.
    #[test]
    fn capture_writes_nothing_outside_devenvs_own_directory() {
        let Some(devenv) = devenv_can_resolve() else {
            eprintln!("skipping: this host cannot build nix closures");
            return;
        };
        let root = tempdir("working-tree");
        write_devenv_project(&root, "{ pkgs, ... }: { }", None);
        if !devenv_update(&devenv, &root) {
            eprintln!("skipping: devenv update could not fetch inputs");
            return;
        }
        let before: Vec<(PathBuf, Vec<u8>)> = ["devenv.nix", "devenv.yaml", "devenv.lock"]
            .iter()
            .map(|name| {
                let path = root.join(name);
                let bytes = fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect();
        let marker = root.join("untouched.txt");
        fs::write(&marker, "before").unwrap();

        DevenvProvider.resolve(&root).unwrap();

        for (path, bytes) in before {
            assert_eq!(
                fs::read(&path).unwrap(),
                bytes,
                "{} changed across capture",
                path.display()
            );
        }
        assert_eq!(fs::read_to_string(&marker).unwrap(), "before");
    }
}
