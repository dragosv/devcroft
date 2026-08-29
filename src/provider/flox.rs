//! The `flox` provider (task 3.2): MVP's only implemented environment
//! provider until add-nix-provider. Activation runs once, host-side,
//! before any sandbox restriction (design.md decision 2) — this module
//! never runs inside the boundary and never activates per session. Shared
//! capture/diff/fingerprint machinery lives in `provider::capture`.

use super::capture;
use super::{Provider, ProviderError, Resolution, ServiceDecl, ServiceSupport};
use crate::paths::resolve_on_path;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

pub struct FloxProvider;

impl Provider for FloxProvider {
    /// Resolve a flox environment: verify `.flox/` exists, run `flox
    /// activate -- env -0` once to capture the post-activation environment,
    /// diff it against the same fixed base activation ran with, and derive
    /// the read-only store grants the compiled policy must carry.
    fn resolve(&self, project_root: &Path) -> Result<Resolution, ProviderError> {
        ensure_environment_present(project_root)?;

        // The hook is the whole reason this branches. Where a project
        // declares `[hook].on-activate`, materializing from the project's
        // own environment would execute it host-side, unconfined, before
        // any boundary exists — the inversion `sandbox-provisioning`
        // exists to close. Materializing from a derived hook-free copy
        // avoids that without changing what gets materialized (P2d).
        let activation_script = activation_hook_script(project_root)?;
        let materialize_from = match activation_script {
            Some(_) => derive_hook_free_env(project_root)?,
            None => project_root.to_path_buf(),
        };

        let baseline = capture::canonical_base_env()?;
        let activated = capture_activated_env(&materialize_from, &baseline)?;

        Ok(Resolution {
            env: capture::changed_env(&baseline, &activated),
            unset: capture::unset_env(&baseline, &activated),
            read_only_grants: capture::store_grants(&activated),
            services: read_service_declarations(project_root)?,
            // False now even when a hook is declared: with P2d the
            // capture above runs against a derived environment that has
            // none, so nothing project-supplied executed on the host.
            // This field means "did this resolution run project code
            // unconfined", and the honest answer is now no.
            ran_activation_hook: false,
            activation_script,
        })
    }
}

/// Read `[services]` out of the flox manifest, host-side, during the
/// trusted provisioning phase.
///
/// Deliberately *only* reads declarations here — the commands themselves
/// are project code and are executed inside the sandbox after
/// restriction, by the keeper. devcroft never runs `flox services
/// start`: that would require the flox binary and its internals to be
/// executable inside the compiled profile, which is exactly what the
/// "environment resolves once, at `up`" invariant rejects for
/// per-session activation, for the same reason (the profile would have
/// to grant flox internals permanently).
///
/// `manifest.toml`'s `[services]` is flox's **documented** schema, and
/// that is why it is the dependency here rather than the
/// `service-config.yaml` flox generates for its own internal
/// process-compose invocation. That file was investigated and rejected:
/// it appears in no published flox documentation, its contents are
/// tailored to flox's own lifecycle (it carries a `flox_never_exit`
/// keep-alive), and the process-compose binary it needs belongs to
/// flox's closure rather than the environment's — zero of the
/// environment's 29 requisites, with `flox-1.14.0` as the referrer. It is
/// reachable from a sandbox today only because devcroft grants
/// `/nix/store` broadly, so consuming it would work by accident and
/// break the day those grants are tightened. See add-flox-services'
/// design.md decision 1 for the full comparison.
fn read_service_declarations(project_root: &Path) -> Result<ServiceSupport, ProviderError> {
    let manifest_path = project_root.join(".flox/env/manifest.toml");
    let text = match std::fs::read_to_string(&manifest_path) {
        Ok(t) => t,
        // No manifest to read is not "no services declared" — but
        // `ensure_environment_present` already established `.flox/`
        // exists, so this is an unreadable environment, not an absent one.
        Err(e) => {
            return Err(ProviderError::ResolutionFailed(format!(
                "reading {}: {e}",
                manifest_path.display()
            )));
        }
    };

    // `toml::Table`, not `toml::Value`: in toml 1.x parsing a whole
    // document as `Value` rejects flox's real manifest outright
    // ("unexpected content, expected nothing"), which is why
    // `config::parse` already uses `Table` for devcroft.toml. Caught by
    // the existing against-real-flox test, not by reasoning.
    let parsed = text.parse::<toml::Table>().map_err(|e| {
        ProviderError::ResolutionFailed(format!("parsing {}: {e}", manifest_path.display()))
    })?;

    let Some(table) = parsed.get("services") else {
        // flox supports services; this environment declares none.
        return Ok(ServiceSupport::Declared(Vec::new()));
    };
    let Some(table) = table.as_table() else {
        return Err(ProviderError::ResolutionFailed(
            "`[services]` in the flox manifest is not a table".to_string(),
        ));
    };

    let mut declared = Vec::new();
    for (name, value) in table {
        // A shape flox no longer produces must fail loudly rather than
        // yielding a silently empty list — the schema-drift risk
        // design.md names.
        let command = value
            .get("command")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| {
                ProviderError::ResolutionFailed(format!(
                    "service `{name}` in the flox manifest has no string `command`"
                ))
            })?;

        // `vars` is not optional decoration: flox's own documented
        // example passes a service's port through it
        // (`command = "… -p \"$PGPORT\""` with `vars.PGPORT`), so a
        // reader that drops it starts the service on the wrong port —
        // silently, since the command string still looks right.
        let mut vars = BTreeMap::new();
        if let Some(table) = value.get("vars") {
            let table = table.as_table().ok_or_else(|| {
                ProviderError::ResolutionFailed(format!("service `{name}`: `vars` is not a table"))
            })?;
            for (k, v) in table {
                // TOML allows non-strings here; process-compose
                // environment entries are strings. Reject rather than
                // stringify, so a manifest meaning `PORT = 5432` is not
                // silently reinterpreted.
                let v = v.as_str().ok_or_else(|| {
                    ProviderError::ResolutionFailed(format!(
                        "service `{name}`: var `{k}` must be a string"
                    ))
                })?;
                vars.insert(k.clone(), v.to_string());
            }
        }

        let is_daemon = match value.get("is-daemon") {
            None => false,
            Some(v) => v.as_bool().ok_or_else(|| {
                ProviderError::ResolutionFailed(format!(
                    "service `{name}`: `is-daemon` must be a boolean"
                ))
            })?,
        };

        // Nested as `shutdown.command` in the manifest.
        let shutdown_command = value
            .get("shutdown")
            .and_then(|s| s.get("command"))
            .map(|c| {
                c.as_str().map(str::to_string).ok_or_else(|| {
                    ProviderError::ResolutionFailed(format!(
                        "service `{name}`: `shutdown.command` must be a string"
                    ))
                })
            })
            .transpose()?;

        // A backgrounding service with no shutdown command cannot be
        // stopped — killing the launcher that already exited does
        // nothing. Caught here, at resolution, rather than discovered at
        // `down` when the process survives teardown.
        if is_daemon && shutdown_command.is_none() {
            return Err(ProviderError::ResolutionFailed(format!(
                "service `{name}` sets `is-daemon` but declares no \
                 `shutdown.command`; it could not be stopped at teardown"
            )));
        }

        declared.push(ServiceDecl {
            name: name.clone(),
            command: command.to_string(),
            vars,
            is_daemon,
            shutdown_command,
        });
    }
    // BTreeMap iteration order from toml's table is already sorted by
    // key, which keeps resolution deterministic.
    Ok(ServiceSupport::Declared(declared))
}

/// `up` fails at layer `provider` with the `flox init` hint (spec: "Missing
/// environment, not missing feature") rather than letting `flox activate`
/// produce its own, less specific error.
/// Build a **derived, hook-free copy** of this project's flox environment
/// and return its directory, so materialization can run without executing
/// the project's `[hook].on-activate` (`sandbox-provisioning` P2d).
///
/// This is the workaround for a flox interface gap, and it works because
/// of one measured property: **a hook is not a package input.** Stripping
/// the `[hook]` table leaves the resolved closure byte-identical —
/// verified live, same store path, same locked package set — so this is a
/// *split* of materialization from activation, not a different
/// environment. If that ever stopped holding, this would be silently
/// materializing something the project did not declare, which is why
/// `tests/flox_derived_env.rs` asserts the closure identity rather than
/// merely that activation succeeds.
///
/// **The project's own `.flox/` is read, never written.** The derived copy
/// lives under the project's `.devcroft/` artifact directory, for the same
/// reason the service artifacts do (`services::ARTIFACT_DIR`): the
/// sandbox has to *read* it at runtime — flox puts its `run/` symlinks
/// there and `PATH` points into them — and devcroft's own state directory
/// is baseline-denied to the sandbox. Anywhere outside the granted project
/// root would leave the sandbox unable to reach its own toolchain.
///
/// Keyed by the environment's fingerprint, which makes the derived copy
/// content-addressed: a manifest or lock change produces a different
/// directory rather than a stale one being reused, and two concurrent
/// resolutions of the same environment converge on identical content.
fn derive_hook_free_env(project_root: &Path) -> Result<std::path::PathBuf, ProviderError> {
    let fingerprint = manifest_fingerprint(project_root)?;
    let derived = project_root
        .join(crate::services::ARTIFACT_DIR)
        .join(format!("{}{fingerprint}", super::DERIVED_ENV_PREFIX));
    let env_dir = derived.join(".flox/env");

    // Already derived for this exact manifest+lock: reuse it. The
    // fingerprint is what makes this safe — a changed environment gets a
    // different path rather than hitting this branch.
    if env_dir.join("manifest.toml").is_file() {
        return Ok(derived);
    }

    std::fs::create_dir_all(&env_dir).map_err(|e| {
        ProviderError::ResolutionFailed(format!("creating {}: {e}", env_dir.display()))
    })?;

    let source = project_root.join(".flox");
    let manifest_text = std::fs::read_to_string(source.join("env/manifest.toml"))
        .map_err(|e| ProviderError::ResolutionFailed(format!("reading the flox manifest: {e}")))?;

    std::fs::write(env_dir.join("manifest.toml"), strip_hook(&manifest_text)?).map_err(|e| {
        ProviderError::ResolutionFailed(format!("writing the derived manifest: {e}"))
    })?;

    // The lock and `env.json` are copied verbatim when present. The lock
    // is what pins the closure — copying it is what makes the derived
    // environment resolve to the same packages rather than re-resolving.
    // `env.json` carries the environment's *name*, which flox surfaces as
    // `FLOX_ENV_DESCRIPTION`; without it the derived environment would
    // report itself under the scratch directory's name.
    for optional in ["env/manifest.lock", "env.json"] {
        let from = source.join(optional);
        if from.is_file() {
            let to = derived.join(".flox").join(optional);
            if let Some(parent) = to.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::copy(&from, &to).map_err(|e| {
                ProviderError::ResolutionFailed(format!("copying {}: {e}", from.display()))
            })?;
        }
    }

    Ok(derived)
}

/// Remove the `[hook]` table from a flox manifest, leaving everything
/// else — `[install]`, `[vars]`, `[services]`, `[profile]` — untouched.
///
/// Operates on the parsed TOML rather than on text. A regex over the raw
/// manifest would be shorter and wrong in a way that matters here:
/// `flox init`'s stock manifest ships a commented-out `[hook]` block, and
/// a hook's body is a multi-line string that can itself contain anything,
/// including lines that look like TOML table headers. Getting this wrong
/// silently changes what gets materialized, which is the one thing this
/// function must not do.
fn strip_hook(manifest_text: &str) -> Result<String, ProviderError> {
    let mut parsed: toml::Table = manifest_text
        .parse()
        .map_err(|e| ProviderError::ResolutionFailed(format!("parsing the flox manifest: {e}")))?;
    parsed.remove("hook");
    toml::to_string(&parsed).map_err(|e| {
        ProviderError::ResolutionFailed(format!("serialising the derived manifest: {e}"))
    })
}

/// The project's `[hook].on-activate` script, read as **data**.
///
/// Never executed here. It travels out through `Resolution` so that `up`
/// can run it *inside* the sandbox after restriction — the two-phase
/// execution invariant, applied to a provider's activation code for the
/// first time.
///
/// **Fails rather than guessing on an unreadable or malformed manifest**,
/// and the direction matters. This replaced a `-> bool` predicate whose
/// documented posture was to err toward "there might be a hook", because
/// a false negative defeated the warning it fed. The same asymmetry is
/// now sharper: a false negative here would route the project down the
/// *undeived* path and execute its hook on the host — the exact thing
/// this mechanism exists to prevent. Since a manifest that cannot be
/// parsed also cannot have its `[hook]` table stripped, there is no safe
/// way to proceed, so it is an error.
///
/// Does not match on raw text. `flox init`'s stock manifest ships a
/// `[hook]` section whose `on-activate` is commented out; a substring
/// search would treat every freshly created environment as hooked.
fn activation_hook_script(project_root: &Path) -> Result<Option<String>, ProviderError> {
    let path = project_root.join(".flox/env/manifest.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| ProviderError::ResolutionFailed(format!("reading {}: {e}", path.display())))?;
    let parsed = text.parse::<toml::Table>().map_err(|e| {
        ProviderError::ResolutionFailed(format!(
            "parsing {}: {e}\n\
             devcroft must parse this manifest to separate materialization from \
             `[hook].on-activate`; it will not fall back to running the hook on the host",
            path.display()
        ))
    })?;
    Ok(parsed
        .get("hook")
        .and_then(|h| h.as_table())
        .and_then(|h| h.get("on-activate"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty()))
}

fn ensure_environment_present(project_root: &Path) -> Result<(), ProviderError> {
    if project_root.join(".flox").is_dir() {
        Ok(())
    } else {
        Err(ProviderError::NoEnvironment {
            provider: "flox",
            hint: "flox init",
        })
    }
}

fn capture_activated_env(
    project_root: &Path,
    base: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, ProviderError> {
    let flox_bin = resolve_on_path("flox").ok_or(ProviderError::MissingBinary {
        provider: "flox",
        hint: "devcroft doctor",
    })?;

    let output = Command::new(flox_bin)
        .arg("activate")
        .arg("--")
        .arg("env")
        .arg("-0")
        .current_dir(project_root)
        .env_clear()
        .envs(base)
        .output()
        .map_err(|e| ProviderError::ResolutionFailed(format!("running `flox activate`: {e}")))?;

    if !output.status.success() {
        return Err(ProviderError::ResolutionFailed(format!(
            "`flox activate` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(capture::parse_env_dump(&output.stdout))
}

/// The names of services this project's flox environment declares, or an
/// empty list when there is no flox environment, it declares none, or it
/// cannot be read.
///
/// Exists for one caller: `up`'s check that a project is not silently
/// dropping declared services because `env.provider` names a provider
/// with no service concept (`lifecycle::up::
/// ensure_no_services_declared_for_another_provider`). Best-effort by
/// construction — an unreadable or malformed flox manifest is *not* this
/// function's error to raise, since the project did not ask devcroft to
/// use flox at all; the honest answer there is "nothing to warn about",
/// not a failure about a provider the manifest does not name.
pub fn declared_service_names(project_root: &Path) -> Vec<String> {
    match read_service_declarations(project_root) {
        Ok(ServiceSupport::Declared(services)) => services.into_iter().map(|s| s.name).collect(),
        _ => Vec::new(),
    }
}

/// Content fingerprint of a flox environment's `manifest.toml` + lockfile,
/// for staleness detection (spec: "Stale environment after manifest
/// change"). `provider::is_stale` (dispatched by provider name) compares
/// this against the fingerprint recorded at the last `up`.
pub fn manifest_fingerprint(project_root: &Path) -> Result<String, ProviderError> {
    ensure_environment_present(project_root)?;
    let manifest_path = project_root.join(".flox/env/manifest.toml");
    let manifest = std::fs::read(&manifest_path).map_err(|e| {
        ProviderError::ResolutionFailed(format!("reading {}: {e}", manifest_path.display()))
    })?;
    // The lockfile does not exist until the first activation; its absence
    // is itself part of what makes a fingerprint change once one appears
    // — which is why this goes through `optional_file_part` rather than
    // `unwrap_or_default()`, the latter having collapsed "absent" and
    // "present but empty" into the same hash.
    let lock = capture::optional_file_part(&project_root.join(".flox/env/manifest.lock"));

    Ok(capture::fingerprint(&[&manifest, &lock]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tempdir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("devcroft-flox-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn flox_env(root: &Path, manifest: &str, lock: Option<&str>) {
        let env_dir = root.join(".flox/env");
        fs::create_dir_all(&env_dir).unwrap();
        fs::write(env_dir.join("manifest.toml"), manifest).unwrap();
        if let Some(lock) = lock {
            fs::write(env_dir.join("manifest.lock"), lock).unwrap();
        }
    }

    #[test]
    fn resolve_fails_with_no_environment_when_flox_dir_missing() {
        let root = tempdir("no-env");
        let err = FloxProvider.resolve(&root).unwrap_err();
        assert_eq!(
            err,
            ProviderError::NoEnvironment {
                provider: "flox",
                hint: "flox init"
            }
        );
    }

    #[test]
    fn manifest_fingerprint_changes_when_manifest_changes() {
        let root = tempdir("fingerprint-manifest");
        flox_env(&root, "version = 1\n", Some("locked-a"));
        let before = manifest_fingerprint(&root).unwrap();

        flox_env(&root, "version = 2\n", Some("locked-a"));
        let after = manifest_fingerprint(&root).unwrap();

        assert_ne!(before, after);
    }

    #[test]
    fn manifest_fingerprint_is_stable_for_unchanged_content() {
        let root = tempdir("fingerprint-stable");
        flox_env(&root, "version = 1\n", Some("locked-a"));

        let first = manifest_fingerprint(&root).unwrap();
        let second = manifest_fingerprint(&root).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn manifest_fingerprint_fails_without_flox_environment() {
        let root = tempdir("fingerprint-missing");
        let err = manifest_fingerprint(&root).unwrap_err();
        assert_eq!(
            err,
            ProviderError::NoEnvironment {
                provider: "flox",
                hint: "flox init"
            }
        );
    }

    #[test]
    fn resolve_against_real_flox_environment_if_available() {
        // Best-effort integration check: only runs meaningfully where
        // `flox` is installed and a real environment can be initialized
        // (the devcontainer provides both — see task 3.2's Dockerfile
        // change). Skips quietly otherwise so the suite stays portable.
        if Command::new("flox").arg("--version").output().is_err() {
            return;
        }
        let root = tempdir("real-resolve");
        let init = Command::new("flox")
            .arg("init")
            .current_dir(&root)
            .output()
            .unwrap();
        if !init.status.success() {
            return;
        }

        let resolution = FloxProvider.resolve(&root).unwrap();
        assert!(resolution.env.contains_key("PATH"));
        assert!(!resolution.read_only_grants.is_empty());
        // A freshly initialized flox environment declares no services,
        // but flox *supports* them — the two must stay distinguishable.
        assert_eq!(resolution.services, ServiceSupport::Declared(Vec::new()));
    }

    #[test]
    fn services_are_read_from_the_flox_manifest() {
        let root = tempdir("services-decl");
        flox_env(
            &root,
            r#"
            version = 1
            [services]
            redis.command = "redis-server --port 6379"
            db.command = "postgres -D ./pgdata"
            "#,
            None,
        );

        let ServiceSupport::Declared(services) = read_service_declarations(&root).unwrap() else {
            panic!("flox must report itself as supporting services");
        };
        // Sorted by key, so resolution stays deterministic.
        assert_eq!(services.len(), 2);
        assert_eq!(services[0].name, "db");
        assert_eq!(services[0].command, "postgres -D ./pgdata");
        assert_eq!(services[1].name, "redis");
    }

    #[test]
    fn vars_is_daemon_and_shutdown_are_all_read() {
        // The whole documented schema, not just `command`. Modeled on
        // flox's own documented example, where the port arrives through
        // `vars` — dropping it would start the service on the wrong port
        // while the command string still looked correct.
        let root = tempdir("services-full");
        flox_env(
            &root,
            r#"
            version = 1
            [services.database]
            command = "exec postgres -D \"$PGDATA\" -p \"$PGPORT\""
            vars.PGPORT = "5433"
            vars.PGDATA = "./pgdata"
            is-daemon = true
            shutdown.command = "pg_ctl stop -D ./pgdata"
            "#,
            None,
        );

        let ServiceSupport::Declared(services) = read_service_declarations(&root).unwrap() else {
            panic!("expected declared services");
        };
        assert_eq!(services.len(), 1);
        let db = &services[0];
        assert_eq!(db.vars.get("PGPORT").map(String::as_str), Some("5433"));
        assert_eq!(db.vars.get("PGDATA").map(String::as_str), Some("./pgdata"));
        assert!(db.is_daemon);
        assert_eq!(
            db.shutdown_command.as_deref(),
            Some("pg_ctl stop -D ./pgdata")
        );
    }

    #[test]
    fn a_daemon_without_a_shutdown_command_is_rejected() {
        // Such a service is unstoppable: the launcher exits immediately
        // by design, so killing it at teardown reaps nothing. Better to
        // fail at resolution than to discover it when `down` leaves a
        // database running.
        let root = tempdir("services-daemon-nostop");
        flox_env(
            &root,
            "version = 1\n[services.db]\ncommand = \"start-db\"\nis-daemon = true\n",
            None,
        );
        let err = read_service_declarations(&root).unwrap_err();
        match err {
            ProviderError::ResolutionFailed(msg) => {
                assert!(msg.contains("db") && msg.contains("shutdown"), "{msg}");
            }
            other => panic!("expected ResolutionFailed, got {other:?}"),
        }
    }

    #[test]
    fn a_non_string_var_is_rejected_rather_than_stringified() {
        // `PGPORT = 5432` (an integer) must not be silently reinterpreted
        // as the string "5432" — the manifest means something devcroft
        // cannot faithfully represent, so it says so.
        let root = tempdir("services-badvar");
        flox_env(
            &root,
            "version = 1\n[services.db]\ncommand = \"x\"\nvars.PGPORT = 5432\n",
            None,
        );
        assert!(read_service_declarations(&root).is_err());
    }

    #[test]
    fn no_services_section_is_supported_but_empty() {
        let root = tempdir("services-none");
        flox_env(&root, "version = 1\n", None);
        assert_eq!(
            read_service_declarations(&root).unwrap(),
            ServiceSupport::Declared(Vec::new()),
            "absent [services] means none declared, not unsupported"
        );
    }

    #[test]
    fn a_service_without_a_command_fails_loudly() {
        // The schema-drift guard: a shape flox no longer produces must
        // fail rather than silently resolving to an empty service list,
        // which would look exactly like "no services declared".
        let root = tempdir("services-drift");
        flox_env(
            &root,
            "version = 1\n[services]\nweird = { port = 5432 }\n",
            None,
        );
        let err = read_service_declarations(&root).unwrap_err();
        match err {
            ProviderError::ResolutionFailed(msg) => {
                assert!(msg.contains("weird"), "error must name the service: {msg}");
            }
            other => panic!("expected ResolutionFailed, got {other:?}"),
        }
    }
}
