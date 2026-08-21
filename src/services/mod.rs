//! Provider-declared services (add-flox-services): turning the
//! declarations a provider resolved at `up` into something that actually
//! runs inside the sandbox, supervised for the sandbox's lifetime.
//!
//! devcroft does not supervise each service itself. It generates a
//! process-compose config it owns and runs process-compose as a single
//! supervised child, which is what makes restart policy, service
//! dependencies and daemon handling work without reimplementing any of
//! them (design.md decision 1).
//!
//! Two things about that decision are load-bearing and non-obvious:
//!
//! - The config devcroft generates is **its own artifact**, built from
//!   the provider's *documented* declaration schema. It is deliberately
//!   not flox's own generated `service-config.yaml`, which is an
//!   undocumented internal file whose process-compose binary belongs to
//!   flox's closure rather than the environment's.
//! - `process-compose` must therefore be a real member of the project's
//!   environment. It is never located by scanning `/nix/store`, which
//!   would pick an arbitrary path with nothing tying it to this
//!   environment.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::provider::ServiceDecl;

/// Where the generated config and process-compose's own runtime files
/// live, relative to the project root.
///
/// The project root rather than the state dir, and that is forced rather
/// than chosen: `<state>/<name>/` sits under `policy::DEVCROFT_DATA_DIR`,
/// which is baseline-denied to the sandbox — the same reason ssh key
/// material is handed to the keeper directly instead of read back off
/// disk. `/tmp` does not work either: confirmed live that nono's
/// baseline grants it write but not read, so a config written there can
/// be created and never opened. The project root is the only location
/// the sandbox can both write and read, and process-compose must read
/// this file.
pub const ARTIFACT_DIR: &str = ".devcroft";

pub fn config_path(project_root: &Path) -> PathBuf {
    project_root.join(ARTIFACT_DIR).join("services.yaml")
}

pub fn log_path(project_root: &Path) -> PathBuf {
    project_root.join(ARTIFACT_DIR).join("services.log")
}

/// Renders `services` as a process-compose configuration.
///
/// Emitted as JSON rather than YAML on purpose: process-compose parses
/// its config with a YAML parser, and YAML is a superset of JSON, so a
/// JSON document in a `.yaml` file is accepted unchanged — verified live
/// against process-compose running inside a real sandbox. That avoids a
/// YAML serializer dependency for output this small, and keeps the
/// generation deterministic (serde_json over a `BTreeMap` preserves key
/// order) so the same declarations always produce a byte-identical file.
pub fn render_config(services: &[ServiceDecl]) -> String {
    let mut processes = serde_json::Map::new();

    for svc in services {
        let mut proc = serde_json::Map::new();
        proc.insert(
            "command".to_string(),
            serde_json::Value::String(svc.command.clone()),
        );

        if !svc.vars.is_empty() {
            // process-compose takes `KEY=value` strings. Sorted, because
            // `vars` is a BTreeMap and determinism is part of the
            // contract.
            let env: Vec<serde_json::Value> = svc
                .vars
                .iter()
                .map(|(k, v)| serde_json::Value::String(format!("{k}={v}")))
                .collect();
            proc.insert("environment".to_string(), serde_json::Value::Array(env));
        }

        // Restart policy stated explicitly, never left to
        // process-compose's default (design.md decision 3: a crashed
        // service stays dead and is reported). Relying on the default
        // would let an upstream change silently reverse that decision,
        // and a flapping database is worse than a visibly dead one for
        // the agent-fleet case this exists to serve.
        let mut availability = serde_json::Map::new();
        availability.insert(
            "restart".to_string(),
            serde_json::Value::String("no".to_string()),
        );
        proc.insert(
            "availability".to_string(),
            serde_json::Value::Object(availability),
        );

        if svc.is_daemon {
            proc.insert("is_daemon".to_string(), serde_json::Value::Bool(true));
            if let Some(cmd) = &svc.shutdown_command {
                let mut shutdown = serde_json::Map::new();
                shutdown.insert(
                    "command".to_string(),
                    serde_json::Value::String(cmd.clone()),
                );
                proc.insert("shutdown".to_string(), serde_json::Value::Object(shutdown));
            }
        }

        processes.insert(svc.name.clone(), serde_json::Value::Object(proc));
    }

    let doc = serde_json::json!({
        "version": "0.5",
        // process-compose's own default is an absolute `/usr/bin/bash`
        // (confirmed by log line "Global shell command: bash -c" against a
        // real 1.116.0 binary) — a host path own-policy-baseline's
        // GROUPS_EXCLUDE makes unreachable regardless of what the
        // project's provider closure supplies. Bare `sh`, PATH-resolved
        // inside the sandbox exactly like every command a service itself
        // runs, matching the same fix `exec.rs`'s shell fallback and
        // `ssh::server::LOGIN_SHELL` needed for the identical reason.
        "shell": {"shell_command": "sh", "shell_argument": "-c"},
        "processes": serde_json::Value::Object(processes),
    });
    serde_json::to_string_pretty(&doc).expect("process-compose config serialization is infallible")
}

pub fn socket_path(project_root: &Path) -> PathBuf {
    project_root.join(ARTIFACT_DIR).join("services.sock")
}

/// One service's live state, as reported by process-compose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceState {
    pub name: String,
    pub health: ServiceHealth,
    pub pid: Option<i64>,
}

/// The four states the `services` spec requires be distinguishable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceHealth {
    Running,
    /// Exited non-zero. **Not** distinguishable from a clean exit by
    /// process-compose's `status` field alone — see [`ServiceState::from_json`].
    Failed {
        exit_code: i64,
    },
    /// Ran and exited cleanly. Normal for one-shot work, suspicious for
    /// something meant to stay up.
    Exited,
    /// Declared but not present in process-compose's view at all.
    NotStarted,
}

impl ServiceHealth {
    pub fn label(self) -> String {
        match self {
            ServiceHealth::Running => "running".to_string(),
            ServiceHealth::Failed { exit_code } => format!("failed (exit {exit_code})"),
            ServiceHealth::Exited => "exited".to_string(),
            ServiceHealth::NotStarted => "not started".to_string(),
        }
    }

    pub fn is_failure(self) -> bool {
        matches!(self, ServiceHealth::Failed { .. })
    }
}

impl ServiceState {
    /// Maps one process-compose process entry onto [`ServiceHealth`].
    ///
    /// The mapping is driven by `exit_code`, **not** by `status`, and
    /// that is the whole point: confirmed live against deliberately
    /// failing services, process-compose reports `status: "Completed"`
    /// for a clean exit *and* for `exit 7` alike. Trusting `status`
    /// would render a crashed database as "Completed", which reads as
    /// healthy — precisely the silent failure the `services` spec
    /// forbids.
    fn from_json(v: &serde_json::Value) -> Option<Self> {
        let name = v.get("name")?.as_str()?.to_string();
        let is_running = v.get("is_running").and_then(serde_json::Value::as_bool);
        let exit_code = v.get("exit_code").and_then(serde_json::Value::as_i64);
        let health = match (is_running, exit_code) {
            (Some(true), _) => ServiceHealth::Running,
            (_, Some(code)) if code != 0 => ServiceHealth::Failed { exit_code: code },
            (Some(false), _) => ServiceHealth::Exited,
            _ => ServiceHealth::NotStarted,
        };
        Some(ServiceState {
            name,
            health,
            pid: v
                .get("pid")
                .and_then(serde_json::Value::as_i64)
                .filter(|p| *p > 0),
        })
    }
}

/// Asks process-compose for per-service state over the unix socket it
/// already listens on.
///
/// Speaks HTTP directly rather than shelling out to `process-compose
/// list`: the binary lives in the *sandbox's* environment, so a host-side
/// command like `status` would otherwise have to resolve it or spawn a
/// session just to read state. The socket, by contrast, is an ordinary
/// file in the project root that unrestricted host-side code can open.
/// It also sidesteps the CLI writing warn/debug lines to stdout ahead of
/// its JSON.
///
/// Returns `Ok(None)` when the socket does not exist or cannot be
/// reached — a sandbox with no services, or one whose services never
/// started. Callers must not treat that as an error.
pub fn query(socket: &Path) -> std::io::Result<Option<Vec<ServiceState>>> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    if !socket.exists() {
        return Ok(None);
    }
    let Ok(mut stream) = UnixStream::connect(socket) else {
        return Ok(None);
    };
    stream.set_read_timeout(Some(std::time::Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(3)))?;
    // `Connection: close` so the server ends the body by closing, which
    // makes `read_to_end` terminate without parsing chunked encoding.
    stream.write_all(b"GET /processes HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;

    let mut raw = Vec::new();
    if stream.read_to_end(&mut raw).is_err() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&raw);
    let Some(body) = text.split_once("\r\n\r\n").map(|(_, b)| b) else {
        return Ok(None);
    };
    // Start at the first `{`: chunked framing (or any preamble) would
    // otherwise break the parse.
    let Some(start) = body.find('{') else {
        return Ok(None);
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body[start..]) else {
        return Ok(None);
    };
    let Some(items) = parsed.get("data").and_then(serde_json::Value::as_array) else {
        return Ok(None);
    };

    let mut states: Vec<ServiceState> = items.iter().filter_map(ServiceState::from_json).collect();
    states.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Some(states))
}

/// Locates `process-compose` through the *resolved environment's* `PATH`,
/// not this process's own.
///
/// The distinction matters: the binary has to exist inside the sandbox,
/// which sees the provider's environment, not `up`'s ambient one. A host
/// that happens to have process-compose installed must not make a
/// project look ready when its own environment does not provide it.
pub fn resolve_in_env(env: &BTreeMap<String, String>) -> Option<PathBuf> {
    let path = env.get("PATH")?;
    for dir in path.split(':').filter(|d| !d.is_empty()) {
        let candidate = Path::new(dir).join("process-compose");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(name: &str, command: &str) -> ServiceDecl {
        ServiceDecl {
            name: name.to_string(),
            command: command.to_string(),
            vars: BTreeMap::new(),
            is_daemon: false,
            shutdown_command: None,
        }
    }

    #[test]
    fn config_is_valid_json_and_names_each_service() {
        let cfg = render_config(&[decl("api", "serve"), decl("worker", "work")]);
        let parsed: serde_json::Value = serde_json::from_str(&cfg).unwrap();
        assert_eq!(parsed["processes"]["api"]["command"], "serve");
        assert_eq!(parsed["processes"]["worker"]["command"], "work");
    }

    #[test]
    fn restart_is_explicitly_disabled_not_left_to_the_default() {
        // design.md decision 3. Asserted directly so an upstream default
        // change cannot silently reintroduce restarts.
        let cfg = render_config(&[decl("api", "serve")]);
        let parsed: serde_json::Value = serde_json::from_str(&cfg).unwrap();
        assert_eq!(parsed["processes"]["api"]["availability"]["restart"], "no");
    }

    #[test]
    fn vars_become_environment_entries() {
        let mut svc = decl("db", "postgres");
        svc.vars.insert("PGPORT".to_string(), "5433".to_string());
        svc.vars
            .insert("PGDATA".to_string(), "./pgdata".to_string());

        let parsed: serde_json::Value = serde_json::from_str(&render_config(&[svc])).unwrap();
        let env = parsed["processes"]["db"]["environment"].as_array().unwrap();
        // Sorted, because determinism is part of the contract.
        assert_eq!(env[0], "PGDATA=./pgdata");
        assert_eq!(env[1], "PGPORT=5433");
    }

    #[test]
    fn a_daemon_carries_its_shutdown_command() {
        let mut svc = decl("db", "pg_ctl start");
        svc.is_daemon = true;
        svc.shutdown_command = Some("pg_ctl stop".to_string());

        let parsed: serde_json::Value = serde_json::from_str(&render_config(&[svc])).unwrap();
        assert_eq!(parsed["processes"]["db"]["is_daemon"], true);
        assert_eq!(
            parsed["processes"]["db"]["shutdown"]["command"],
            "pg_ctl stop"
        );
    }

    #[test]
    fn no_services_still_renders_a_valid_empty_config() {
        let parsed: serde_json::Value = serde_json::from_str(&render_config(&[])).unwrap();
        assert!(parsed["processes"].as_object().unwrap().is_empty());
    }

    #[test]
    fn generation_is_deterministic() {
        let mut svc = decl("db", "postgres");
        svc.vars.insert("B".to_string(), "2".to_string());
        svc.vars.insert("A".to_string(), "1".to_string());
        assert_eq!(
            render_config(std::slice::from_ref(&svc)),
            render_config(&[svc])
        );
    }

    /// The mapping that matters most, and the one a reasonable reading of
    /// process-compose's output gets wrong: `status` is `"Completed"` for
    /// a clean exit *and* for a crash. Only `exit_code` separates them.
    #[test]
    fn a_crashed_service_is_failed_even_though_status_says_completed() {
        let crashed = serde_json::json!({
            "name": "db", "status": "Completed", "exit_code": 7,
            "is_running": false, "pid": 0
        });
        let state = ServiceState::from_json(&crashed).unwrap();
        assert_eq!(state.health, ServiceHealth::Failed { exit_code: 7 });
        assert!(state.health.is_failure());
        assert!(state.health.label().contains("exit 7"));
    }

    #[test]
    fn a_clean_exit_is_not_a_failure() {
        let done = serde_json::json!({
            "name": "migrate", "status": "Completed", "exit_code": 0,
            "is_running": false, "pid": 0
        });
        let state = ServiceState::from_json(&done).unwrap();
        assert_eq!(state.health, ServiceHealth::Exited);
        assert!(!state.health.is_failure());
    }

    #[test]
    fn a_running_service_reports_running_and_its_pid() {
        let running = serde_json::json!({
            "name": "api", "status": "Running", "exit_code": 0,
            "is_running": true, "pid": 4242
        });
        let state = ServiceState::from_json(&running).unwrap();
        assert_eq!(state.health, ServiceHealth::Running);
        assert_eq!(state.pid, Some(4242));
    }

    #[test]
    fn query_on_a_missing_socket_is_not_an_error() {
        // A sandbox with no services has no socket; callers must not see
        // that as a failure.
        let missing = Path::new("/nonexistent/devcroft/services.sock");
        assert_eq!(query(missing).unwrap(), None);
    }

    #[test]
    fn resolve_in_env_uses_the_provided_path_not_the_hosts() {
        // An empty PATH must find nothing even on a host that has
        // process-compose installed — the binary has to come from the
        // environment the sandbox will actually see.
        let mut env = BTreeMap::new();
        env.insert("PATH".to_string(), String::new());
        assert!(resolve_in_env(&env).is_none());
        assert!(resolve_in_env(&BTreeMap::new()).is_none());
    }
}
