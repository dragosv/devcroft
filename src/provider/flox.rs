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

        let baseline = capture::canonical_base_env()?;
        let activated = capture_activated_env(project_root, &baseline)?;

        Ok(Resolution {
            env: capture::changed_env(&baseline, &activated),
            unset: capture::unset_env(&baseline, &activated),
            read_only_grants: capture::store_grants(&activated),
            services: read_service_declarations(project_root)?,
            // Not "does flox support hooks" but "did this capture run
            // one" — see `declares_activation_hook`.
            ran_activation_hook: declares_activation_hook(project_root),
        })
    }
}

/// Whether this environment's manifest defines `[hook].on-activate`,
/// which `flox activate` runs during capture.
///
/// **There is no way to avoid running it**, measured against flox 1.14.0
/// rather than assumed: neither the default `flox activate -- <cmd>`,
/// nor `--mode run`, nor `--mode dev`, nor `--no-start-services`
/// suppresses it. flox's own help notes that the `<cmd>` form "does not
/// run any profile scripts", which is accurate and describes
/// `[profile]` — a different manifest section from `[hook]`.
///
/// So detection exists to let `up` *report* what already happened
/// (`fix-provisioning-hooks`), not to prevent it. Refusing such a
/// project was considered and rejected: `on-activate` is how flox
/// environments do setup, the user's own `flox activate` runs it too,
/// and a rule that rejects the common case of the default provider is a
/// rule that gets disabled.
///
/// Errs toward reporting. A manifest that cannot be read or parsed
/// counts as "might have one", because a false negative defeats the
/// warning entirely while a false positive is merely noise. It does
/// **not** match on the raw text: `flox init`'s stock manifest ships a
/// `[hook]` section whose `on-activate` is commented out, so a
/// substring search would warn on every freshly created environment and
/// teach users to ignore the warning.
fn declares_activation_hook(project_root: &Path) -> bool {
    let path = project_root.join(".flox/env/manifest.toml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        // No manifest at all is not this function's error to raise —
        // `ensure_environment_present` already ran, so reaching here
        // means something unusual. Report rather than assume safety.
        return true;
    };
    let Ok(parsed) = text.parse::<toml::Table>() else {
        return true;
    };
    parsed
        .get("hook")
        .and_then(|h| h.as_table())
        .and_then(|h| h.get("on-activate"))
        .is_some_and(|v| v.as_str().is_some_and(|s| !s.trim().is_empty()))
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
    // is itself part of what makes a fingerprint change once one appears.
    let lock = std::fs::read(project_root.join(".flox/env/manifest.lock")).unwrap_or_default();

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
