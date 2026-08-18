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
        "processes": serde_json::Value::Object(processes),
    });
    serde_json::to_string_pretty(&doc).expect("process-compose config serialization is infallible")
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
