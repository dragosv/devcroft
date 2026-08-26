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

/// Per-sandbox artifact directory: `<root>/.devcroft/<sandbox-name>/`.
///
/// Keyed on the sandbox name, not the project root alone, and that is a
/// correctness requirement rather than tidiness. Two sandboxes with
/// different names can share one project root — that is exactly the
/// arrangement `add-port-allocation` task 6.1 offers as the alternative
/// to git worktrees. Keyed on the root alone, the second `up` overwrites
/// the first's generated config, both supervisors race for one socket
/// path, and `status`/`ps` for either sandbox reports the other's
/// services. Nothing about that fails loudly; it just silently reports
/// the wrong thing.
pub fn artifact_dir(project_root: &Path, sandbox_name: &str) -> PathBuf {
    project_root.join(ARTIFACT_DIR).join(sandbox_name)
}

pub fn config_path(project_root: &Path, sandbox_name: &str) -> PathBuf {
    artifact_dir(project_root, sandbox_name).join("services.yaml")
}

pub fn log_path(project_root: &Path, sandbox_name: &str) -> PathBuf {
    artifact_dir(project_root, sandbox_name).join("services.log")
}

/// How long process-compose waits for a service to exit on SIGTERM
/// before sending SIGKILL, in seconds.
///
/// **This is what makes teardown a guarantee rather than a request**, and
/// it was missing: devcroft's shutdown handler kills the *registered
/// process group*, which is process-compose's — a service process lives
/// in its own group, so the escalation never reached it. A service that
/// ignores SIGTERM therefore survived `down`, was reparented to init, and
/// kept running, directly against the `services` spec's "no service
/// process started by it remains alive on the host". Found by task 3.6's
/// test, which is exactly the case it was written for; the polite service
/// the earlier tests used dies on SIGTERM and hid it.
///
/// Reaping is delegated to process-compose rather than reimplemented by
/// walking the process tree, following design.md decision 1's rule that
/// the supervisor owns restart policy, daemon handling, and — this —
/// shutdown. Measured against a real process-compose: with this set, a
/// child trapping SIGTERM is gone within the timeout; without it, it
/// survives indefinitely.
///
/// **Must stay strictly below [`keeper::connection::DEFAULT_GRACE_PERIOD`]**,
/// the window devcroft gives before SIGKILLing process-compose itself.
/// If it were longer, devcroft would kill the supervisor before the
/// supervisor got to kill its children — reintroducing the orphan by a
/// different route. Asserted by a test rather than left to a comment.
pub const SHUTDOWN_TIMEOUT_SECS: u64 = 1;

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

        // Every service gets a shutdown timeout, daemon or not — see
        // `SHUTDOWN_TIMEOUT_SECS`. A daemon additionally gets its
        // declared `shutdown.command`, which is how a backgrounding
        // service is stopped at all (its launcher has already exited, so
        // signalling that pid reaps nothing).
        let mut shutdown = serde_json::Map::new();
        shutdown.insert(
            "timeout".to_string(),
            serde_json::Value::Number(SHUTDOWN_TIMEOUT_SECS.into()),
        );
        if svc.is_daemon {
            proc.insert("is_daemon".to_string(), serde_json::Value::Bool(true));
            if let Some(cmd) = &svc.shutdown_command {
                shutdown.insert(
                    "command".to_string(),
                    serde_json::Value::String(cmd.clone()),
                );
            }
        }
        proc.insert("shutdown".to_string(), serde_json::Value::Object(shutdown));

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

pub fn socket_path(project_root: &Path, sandbox_name: &str) -> PathBuf {
    artifact_dir(project_root, sandbox_name).join("services.sock")
}

/// `sockaddr_un.sun_path` is 108 bytes on Linux and 104 on macOS, minus
/// the NUL — a limit that bites here rather than theoretically, because
/// [`socket_path`] is one directory deeper than it used to be
/// (per-sandbox keying) and the project root is chosen by the user, not
/// by devcroft. Over the limit, `process-compose` fails to bind with an
/// error naming neither the path nor the reason, so `up` checks it
/// host-side and fails at layer `config` instead.
pub const MAX_SOCKET_PATH: usize = 103;

/// One service's live state, as reported by process-compose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceState {
    pub name: String,
    pub health: ServiceHealth,
    pub pid: Option<i64>,
}

/// The four states the `services` spec requires be distinguishable,
/// plus the two process-compose reports that map onto none of them —
/// see [`ServiceState::from_json`] for the live measurements.
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
    /// Accepted by process-compose but not started yet — waiting on a
    /// `depends_on` gate. Distinct from [`ServiceHealth::NotStarted`],
    /// which means the supervisor has no record of it whatsoever.
    Pending,
    /// Will never run: a dependency it gates on failed. Not a failure
    /// *of this service* — the dependency reports that — but decidedly
    /// not healthy either.
    Skipped,
}

impl ServiceHealth {
    pub fn label(self) -> String {
        match self {
            ServiceHealth::Running => "running".to_string(),
            ServiceHealth::Failed { exit_code } => format!("failed (exit {exit_code})"),
            ServiceHealth::Exited => "exited".to_string(),
            ServiceHealth::NotStarted => "not started".to_string(),
            ServiceHealth::Pending => "pending".to_string(),
            ServiceHealth::Skipped => "skipped (dependency failed)".to_string(),
        }
    }

    pub fn is_failure(self) -> bool {
        matches!(self, ServiceHealth::Failed { .. })
    }

    /// Whether this state should stop a sandbox reading as "all services
    /// fine". Broader than [`ServiceHealth::is_failure`], which counts
    /// only services that themselves failed: a skipped or never-started
    /// service is not a failure to attribute, but it is not health
    /// either, and the `services` spec forbids presenting it as such.
    pub fn is_healthy(self) -> bool {
        matches!(
            self,
            ServiceHealth::Running | ServiceHealth::Exited | ServiceHealth::Pending
        )
    }
}

impl ServiceState {
    /// Maps one process-compose process entry onto [`ServiceHealth`].
    ///
    /// For a service that has actually *run*, the mapping is driven by
    /// `exit_code`, **not** by `status`: confirmed live against
    /// deliberately failing services, process-compose reports
    /// `status: "Completed"` for a clean exit *and* for `exit 7` alike.
    /// Trusting `status` there would render a crashed database as
    /// "Completed", which reads as healthy — precisely the silent
    /// failure the `services` spec forbids.
    ///
    /// But `exit_code` is meaningless for a service that has *not* run,
    /// and two such states report one that actively misleads. Measured
    /// against a real process-compose 1.120.0, not reasoned about:
    ///
    /// | `status`    | `is_running` | `exit_code` | correct reading |
    /// |-------------|--------------|-------------|-----------------|
    /// | `Pending`   | `false`      | `0`         | waiting on `depends_on` |
    /// | `Skipped`   | `false`      | `1`         | dependency failed, will never run |
    /// | `Running`   | `true`       | `0`         | running |
    /// | `Completed` | `false`      | `0`         | exited cleanly |
    /// | `Completed` | `false`      | `7`         | failed |
    ///
    /// Read by `exit_code` alone, the first row is "exited" (a service
    /// still queuing to start looks like one that already finished —
    /// what `status` immediately after `up` would show) and the second
    /// is "failed (exit 1)", an exit code no process ever produced. So
    /// `status` is consulted *first*, and only for the two states where
    /// no run has happened; everywhere else `exit_code` stays the
    /// authority.
    fn from_json(v: &serde_json::Value) -> Option<Self> {
        let name = v.get("name")?.as_str()?.to_string();
        let status = v.get("status").and_then(serde_json::Value::as_str);
        let is_running = v.get("is_running").and_then(serde_json::Value::as_bool);
        let exit_code = v.get("exit_code").and_then(serde_json::Value::as_i64);
        let health = match (status, is_running, exit_code) {
            (Some("Pending"), _, _) => ServiceHealth::Pending,
            (Some("Skipped"), _, _) => ServiceHealth::Skipped,
            (_, Some(true), _) => ServiceHealth::Running,
            (_, _, Some(code)) if code != 0 => ServiceHealth::Failed { exit_code: code },
            (_, Some(false), _) => ServiceHealth::Exited,
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
/// Why a query did not produce state.
///
/// Split out because collapsing every failure into one "no state" answer
/// is what let a dead supervisor read as a healthy sandbox: three
/// declared services plus a `process-compose` that died at startup
/// produced exactly the same `None` as a sandbox declaring no services
/// at all, and `status` printed nothing either way. The `services`
/// spec's "SHALL NOT be omitted from service listings" needs these two
/// to be distinguishable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreachable {
    /// No socket at the expected path. Expected and benign on its own —
    /// a sandbox declaring no services never creates one.
    NoSocket,
    /// The socket is there but did not yield usable state. Always worth
    /// reporting: something created it and then stopped answering.
    Unusable(String),
}

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unreachable::NoSocket => write!(f, "no supervisor socket"),
            Unreachable::Unusable(why) => write!(f, "{why}"),
        }
    }
}

/// Cap on the supervisor's response. Generous next to a real listing
/// (a few hundred bytes per service) and small next to memory pressure,
/// so a peer that streams forever is cut off rather than followed.
const MAX_RESPONSE: u64 = 4 * 1024 * 1024;

/// How long the whole exchange may take. The per-read timeout below
/// bounds each individual `read`, which a peer dripping one byte at a
/// time resets forever; this bounds the sum.
const QUERY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// Asks the supervisor for per-service state over the unix socket it
/// already listens on.
///
/// Speaks HTTP directly rather than shelling out to `process-compose
/// list`: the binary lives in the *sandbox's* environment, so a host-side
/// command like `status` would otherwise have to resolve it or spawn a
/// session just to read state. It also sidesteps the CLI writing
/// warn/debug lines to stdout ahead of its JSON.
///
/// **This reads a socket the sandbox controls.** At the `process` tier
/// that is accident protection, consistent with the tier's framing; at
/// `hardened` it is a real trust inversion, since `--host-uds=create`
/// exists precisely so the host can reach inward. So the peer is treated
/// as untrusted input rather than as devcroft's own supervisor: the path
/// must be a socket owned by this user (not a regular file or a FIFO
/// swapped in underneath), the response is capped at [`MAX_RESPONSE`],
/// and the exchange is bounded by [`QUERY_DEADLINE`] as a whole and not
/// only per-read.
pub fn query(socket: &Path) -> Result<Vec<ServiceState>, Unreachable> {
    use std::io::{Read, Write};
    use std::os::unix::fs::FileTypeExt;
    use std::os::unix::net::UnixStream;

    let meta = match std::fs::symlink_metadata(socket) {
        Ok(m) => m,
        Err(_) => return Err(Unreachable::NoSocket),
    };
    if !meta.file_type().is_socket() {
        return Err(Unreachable::Unusable(format!(
            "{} exists but is not a socket",
            socket.display()
        )));
    }
    // Ownership, checked rather than assumed: the project root is
    // writable by whatever runs in the sandbox, so the path this
    // resolves to is not devcroft's to trust by construction.
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: getuid is always successful and takes no arguments.
        let uid = unsafe { libc::getuid() };
        if meta.uid() != uid {
            return Err(Unreachable::Unusable(format!(
                "{} is owned by uid {}, not {uid}",
                socket.display(),
                meta.uid()
            )));
        }
    }

    let started = std::time::Instant::now();
    let mut stream =
        UnixStream::connect(socket).map_err(|e| Unreachable::Unusable(format!("connect: {e}")))?;
    stream
        .set_read_timeout(Some(QUERY_DEADLINE))
        .and_then(|()| stream.set_write_timeout(Some(QUERY_DEADLINE)))
        .map_err(|e| Unreachable::Unusable(format!("socket timeout: {e}")))?;
    // `Connection: close` so the server ends the body by closing, which
    // makes the read terminate without parsing chunked encoding.
    stream
        .write_all(b"GET /processes HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .map_err(|e| Unreachable::Unusable(format!("write: {e}")))?;

    let mut raw = Vec::new();
    // `take` bounds the total, so a peer that never closes cannot grow
    // this without limit; the deadline check below bounds the time.
    if let Err(e) = stream.take(MAX_RESPONSE).read_to_end(&mut raw) {
        return Err(Unreachable::Unusable(format!("read: {e}")));
    }
    if started.elapsed() > QUERY_DEADLINE {
        return Err(Unreachable::Unusable(
            "supervisor did not answer within the deadline".to_string(),
        ));
    }
    if raw.len() as u64 >= MAX_RESPONSE {
        return Err(Unreachable::Unusable(format!(
            "response exceeded {MAX_RESPONSE} bytes"
        )));
    }

    let text = String::from_utf8_lossy(&raw);
    let Some(body) = text.split_once("\r\n\r\n").map(|(_, b)| b) else {
        return Err(Unreachable::Unusable(
            "no HTTP body in response".to_string(),
        ));
    };
    // Start at the first `{`: chunked framing (or any preamble) would
    // otherwise break the parse.
    let Some(start) = body.find('{') else {
        return Err(Unreachable::Unusable(
            "no JSON in response body".to_string(),
        ));
    };
    let parsed = serde_json::from_str::<serde_json::Value>(&body[start..])
        .map_err(|e| Unreachable::Unusable(format!("malformed JSON: {e}")))?;
    let Some(items) = parsed.get("data").and_then(serde_json::Value::as_array) else {
        return Err(Unreachable::Unusable(
            "response JSON has no `data` array".to_string(),
        ));
    };

    let mut states: Vec<ServiceState> = items.iter().filter_map(ServiceState::from_json).collect();
    states.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(states)
}

/// What `status`/`ps` should show for a sandbox's services, reconciling
/// what the supervisor reports against what the provider actually
/// declared at `up`.
///
/// The reconciliation is the point. Querying alone can only report what
/// process-compose knows about, so a declared service the supervisor
/// never accepted was reported by *absence* — and a supervisor that died
/// outright made every service disappear at once while the sandbox went
/// on looking healthy. Both are the silent-failure mode the `services`
/// spec forbids.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ServicesReport {
    pub states: Vec<ServiceState>,
    /// Set when services were declared but the supervisor could not be
    /// reached — the diagnostic that would otherwise exist only in the
    /// keeper's log.
    pub supervisor_error: Option<String>,
}

impl ServicesReport {
    pub fn is_empty(&self) -> bool {
        self.states.is_empty() && self.supervisor_error.is_none()
    }
}

/// Builds the report for one sandbox. `declared` is what the provider
/// resolved at `up` (recorded in `meta.json`); `queried` is this call's
/// live answer.
pub fn reconcile(
    declared: &[String],
    queried: Result<Vec<ServiceState>, Unreachable>,
) -> ServicesReport {
    match queried {
        Ok(mut states) => {
            // A declared service the supervisor has no record of is
            // `NotStarted` — the fourth state, which until now nothing
            // could actually produce, since only process-compose's own
            // listing was ever consulted and it cannot report a service
            // it never accepted.
            for name in declared {
                if !states.iter().any(|s| &s.name == name) {
                    states.push(ServiceState {
                        name: name.clone(),
                        health: ServiceHealth::NotStarted,
                        pid: None,
                    });
                }
            }
            states.sort_by(|a, b| a.name.cmp(&b.name));
            ServicesReport {
                states,
                supervisor_error: None,
            }
        }
        // Nothing declared and no socket: an ordinary sandbox without
        // services. The only case that legitimately reports nothing.
        Err(Unreachable::NoSocket) if declared.is_empty() => ServicesReport::default(),
        Err(why) => ServicesReport {
            states: declared
                .iter()
                .map(|name| ServiceState {
                    name: name.clone(),
                    health: ServiceHealth::NotStarted,
                    pid: None,
                })
                .collect(),
            supervisor_error: Some(format!("supervisor unreachable: {why}")),
        },
    }
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
    /// The invariant `SHUTDOWN_TIMEOUT_SECS`'s doc comment states, as a
    /// test rather than a comment: process-compose must reap its own
    /// children *before* devcroft reaps process-compose, or a service
    /// that ignores SIGTERM is orphaned instead of killed.
    #[test]
    fn the_service_shutdown_timeout_stays_below_the_keeper_grace_period() {
        assert!(
            std::time::Duration::from_secs(SHUTDOWN_TIMEOUT_SECS)
                < crate::keeper::DEFAULT_GRACE_PERIOD,
            "process-compose must get its children killed before devcroft kills it"
        );
    }

    /// Every service carries a shutdown timeout, not only daemons — the
    /// bug was that ordinary services got no `shutdown` block at all.
    #[test]
    fn every_service_gets_a_shutdown_timeout() {
        let svc = ServiceDecl {
            name: "web".into(),
            command: "true".into(),
            vars: Default::default(),
            is_daemon: false,
            shutdown_command: None,
        };
        let rendered = render_config(std::slice::from_ref(&svc));
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(
            parsed["processes"]["web"]["shutdown"]["timeout"],
            serde_json::json!(SHUTDOWN_TIMEOUT_SECS),
            "an ordinary service must still get a shutdown timeout, got: {rendered}"
        );
    }

    /// ...and a daemon keeps its declared shutdown command alongside it,
    /// rather than the timeout displacing it.
    #[test]
    fn a_daemon_keeps_both_its_shutdown_command_and_the_timeout() {
        let svc = ServiceDecl {
            name: "db".into(),
            command: "start-db".into(),
            vars: Default::default(),
            is_daemon: true,
            shutdown_command: Some("stop-db".into()),
        };
        let rendered = render_config(std::slice::from_ref(&svc));
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(parsed["processes"]["db"]["shutdown"]["command"], "stop-db");
        assert_eq!(
            parsed["processes"]["db"]["shutdown"]["timeout"],
            serde_json::json!(SHUTDOWN_TIMEOUT_SECS)
        );
    }

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
    fn query_on_a_missing_socket_reports_no_socket() {
        let missing = Path::new("/nonexistent/devcroft/services.sock");
        assert_eq!(query(missing), Err(Unreachable::NoSocket));
    }

    /// A regular file where the socket should be is *not* the benign
    /// "no services" case — something put it there. The project root is
    /// sandbox-writable, so this is checked rather than assumed.
    #[test]
    fn query_refuses_a_path_that_is_not_a_socket() {
        let dir = std::env::temp_dir().join(format!("devcroft-sockcheck-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let impostor = dir.join("services.sock");
        std::fs::write(&impostor, b"not a socket").unwrap();

        match query(&impostor) {
            Err(Unreachable::Unusable(why)) => assert!(why.contains("not a socket")),
            other => panic!("expected Unusable, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Measured against process-compose 1.120.0: a service gated behind
    /// `depends_on` reports `is_running: false, exit_code: 0` — which
    /// read by exit code alone is indistinguishable from a clean exit.
    /// `status` is what separates them.
    #[test]
    fn a_pending_service_is_not_reported_as_exited() {
        let pending = serde_json::json!({
            "name": "slowstart", "status": "Pending", "exit_code": 0,
            "is_running": false, "pid": 0
        });
        let state = ServiceState::from_json(&pending).unwrap();
        assert_eq!(state.health, ServiceHealth::Pending);
        assert!(!state.health.is_failure());
    }

    /// Also measured live: a service skipped because its dependency
    /// failed carries `exit_code: 1` that no process ever produced.
    /// Reporting "failed (exit 1)" would invent a failure.
    #[test]
    fn a_skipped_service_does_not_borrow_a_synthetic_exit_code() {
        let skipped = serde_json::json!({
            "name": "migrate", "status": "Skipped", "exit_code": 1,
            "is_running": false, "pid": 0
        });
        let state = ServiceState::from_json(&skipped).unwrap();
        assert_eq!(state.health, ServiceHealth::Skipped);
        assert!(!state.health.is_failure());
        // Not a failure to attribute, but not health either.
        assert!(!state.health.is_healthy());
    }

    /// The gap that let a dead supervisor look like a healthy sandbox.
    #[test]
    fn a_dead_supervisor_is_reported_not_silently_empty() {
        let report = reconcile(
            &["db".to_string(), "api".to_string()],
            Err(Unreachable::Unusable("connect: refused".to_string())),
        );
        assert!(report.supervisor_error.is_some());
        assert_eq!(report.states.len(), 2);
        assert!(
            report
                .states
                .iter()
                .all(|s| s.health == ServiceHealth::NotStarted)
        );
    }

    /// The benign case must stay silent: no services declared, no
    /// socket, nothing to report.
    #[test]
    fn no_declared_services_and_no_socket_reports_nothing() {
        let report = reconcile(&[], Err(Unreachable::NoSocket));
        assert!(report.is_empty());
    }

    /// `NotStarted` was previously unreachable: only process-compose's
    /// own listing was consulted, and it cannot report a service it
    /// never accepted. Reconciling against the declared set is what
    /// makes the fourth state produceable.
    #[test]
    fn a_declared_service_the_supervisor_never_saw_is_not_started() {
        let running = ServiceState {
            name: "db".to_string(),
            health: ServiceHealth::Running,
            pid: Some(42),
        };
        let report = reconcile(&["db".to_string(), "ghost".to_string()], Ok(vec![running]));

        assert_eq!(report.supervisor_error, None);
        let ghost = report.states.iter().find(|s| s.name == "ghost").unwrap();
        assert_eq!(ghost.health, ServiceHealth::NotStarted);
    }

    #[test]
    fn artifacts_are_keyed_on_the_sandbox_name_not_just_the_root() {
        let root = Path::new("/proj");
        assert_ne!(socket_path(root, "alpha"), socket_path(root, "beta"));
        assert_ne!(config_path(root, "alpha"), config_path(root, "beta"));
        assert_ne!(log_path(root, "alpha"), log_path(root, "beta"));
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
