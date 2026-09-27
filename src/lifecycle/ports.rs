//! Port allocation (`add-port-allocation`): a free loopback port per
//! sandbox for each `network.services.<name>.var` request, so several
//! sandboxes from one committed manifest can run the same service where
//! they share the host's loopback.
//!
//! **Only there.** A sandbox with its own network namespace (an isolated
//! `up`, every fleet agent) keeps the declared port unchanged: allocating
//! inside a private port table would trade a predictable number for an
//! unpredictable one and fix nothing (`port-allocation`: *Allocation
//! applies where a collision is possible*). Callers decide that; this
//! module only allocates when asked.
//!
//! **Drawn from outside the ephemeral range** (design.md's open question,
//! settled). Binding `:0` hands out an ephemeral port, which the kernel may
//! give to an unrelated outbound connection while the sandbox is down, so a
//! recorded port would be lost most often on exactly the busy hosts
//! allocation exists for. Ports below the ephemeral range are handed out by
//! nobody but listeners. `:0` remains the fallback when nothing there is
//! free.
//!
//! **Sticky, and never silently changed.** A recorded port is reclaimed if
//! it can still be bound; if it cannot, a new one is chosen and the change
//! is announced, naming both, since a connection string written against
//! the old one is now wrong.

use std::collections::BTreeMap;
use std::io;
use std::net::TcpListener;

use super::state::Allocation;
use crate::config::ServicePort;
use crate::provider::ServiceDecl;

/// Where allocation looks first: from here up to the ephemeral range.
const LOW: u16 = 10000;
/// Random draws before falling back to the kernel's choice.
const TRIES: usize = 64;

/// The manifest's allocation requests, as (service, variable).
pub fn requests(services: &BTreeMap<String, ServicePort>) -> Vec<(String, String)> {
    services
        .iter()
        .filter_map(|(name, s)| Some((name.clone(), s.var.clone()?)))
        .collect()
}

/// What allocation did: the ports, and an announcement for each recorded
/// port that could not be reclaimed.
#[derive(Debug, Default)]
pub struct Allocated {
    pub allocations: Vec<Allocation>,
    pub changed: Vec<String>,
}

/// Allocate a port for each request, reclaiming `previous` where it can
/// still be bound, and never choosing one in `reserved` (ports other
/// sandboxes have recorded).
pub fn allocate(
    requests: &[(String, String)],
    previous: &[Allocation],
    reserved: &[u16],
) -> io::Result<Allocated> {
    let mut out = Allocated::default();
    for (service, var) in requests {
        let recorded = previous
            .iter()
            .find(|a| &a.service == service && &a.var == var)
            .map(|a| a.port);
        let port = match recorded {
            Some(port) if bindable(port) => port,
            _ => {
                let taken: Vec<u16> = reserved
                    .iter()
                    .copied()
                    .chain(out.allocations.iter().map(|a| a.port))
                    .collect();
                let port = choose(&taken)?;
                if let Some(old) = recorded {
                    out.changed.push(format!(
                        "{service}'s {var} was {old}, which is no longer free; it is now {port}"
                    ));
                }
                port
            }
        };
        out.allocations.push(Allocation {
            service: service.clone(),
            var: var.clone(),
            port,
        });
    }
    Ok(out)
}

/// Whether a request's service can receive its allocation: its command must
/// read the variable. Scoped to the one service the request names (every
/// other service legitimately never mentions it), and decided by looking
/// for the variable, never by parsing a port out of shell. `Err` names the
/// service. A request for an undeclared service is
/// `services::check_declared_ports`'s to refuse.
pub fn check_substitutable(
    requests: &[(String, String)],
    declared: &[ServiceDecl],
) -> Result<(), String> {
    for (service, var) in requests {
        let Some(decl) = declared.iter().find(|d| &d.name == service) else {
            continue;
        };
        if !references(&decl.command, var) {
            return Err(format!(
                "network.services.{service}.var: `{service}` never reads ${var}, so an \
                 allocated port would be granted and nothing would listen on it; its \
                 command is `{}`. Make the command use ${var} (with the port in the \
                 service's vars), or drop the allocation and keep the fixed port",
                decl.command
            ));
        }
    }
    Ok(())
}

/// The declared services with each allocation substituted into its
/// service's `vars`, overriding the provider's value; every other variable
/// as the provider set it. The provider's manifest is not touched.
pub fn substitute(declared: &[ServiceDecl], allocations: &[Allocation]) -> Vec<ServiceDecl> {
    let mut services = declared.to_vec();
    for a in allocations {
        if let Some(decl) = services.iter_mut().find(|d| d.name == a.service) {
            decl.vars.insert(a.var.clone(), a.port.to_string());
        }
    }
    services
}

/// `$VAR` not followed by more of a name, or `${VAR` followed by `}` or an
/// expansion operator (`${VAR:-8000}`).
fn references(command: &str, var: &str) -> bool {
    let is_name = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let bare = format!("${var}");
    let braced = format!("${{{var}");
    command
        .match_indices(&bare)
        .any(|(i, _)| !command[i + bare.len()..].starts_with(is_name))
        || command.match_indices(&braced).any(|(i, _)| {
            matches!(
                command[i + braced.len()..].chars().next(),
                Some('}' | ':' | '-' | '=')
            )
        })
}

/// Whether `port` is free, asked a few times: a socket this process just
/// closed stays bound while any child forked in that moment still holds its
/// inherited copy, until that child execs (CLOEXEC closes it only then).
/// Measured: a bind, drop, rebind loop failed 20 times in 5000 with another
/// thread spawning processes, and 0 times without. One failed probe would
/// otherwise count as a lost port, and a recorded allocation would move for
/// nothing. A port that is really taken stays taken across the retries.
fn bindable(port: u16) -> bool {
    for attempt in 0..4 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return true;
        }
        if attempt < 3 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    false
}

fn choose(taken: &[u16]) -> io::Result<u16> {
    let high = ephemeral_low();
    if high > LOW + 1 {
        for _ in 0..TRIES {
            let port = rand::random_range(LOW..high);
            if !taken.contains(&port) && bindable(port) {
                return Ok(port);
            }
        }
    }
    // Nothing free below the ephemeral range: the kernel's choice, which is
    // free now but may not stay reserved while the sandbox is down.
    loop {
        let port = TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port();
        if !taken.contains(&port) {
            return Ok(port);
        }
    }
}

/// The bottom of the kernel's ephemeral range: Linux's
/// `ip_local_port_range`, or the IANA dynamic range's start elsewhere.
fn ephemeral_low() -> u16 {
    std::fs::read_to_string("/proc/sys/net/ipv4/ip_local_port_range")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse().ok())
        .unwrap_or(49152)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(name: &str, command: &str) -> ServiceDecl {
        ServiceDecl {
            name: name.into(),
            command: command.into(),
            vars: BTreeMap::from([("PGPORT".into(), "5432".into()), ("LOG".into(), "1".into())]),
            is_daemon: false,
            working_dir: None,
            depends_on: Vec::new(),
            restart: crate::provider::RestartPolicy::Never,
            shutdown: crate::provider::Shutdown::Default,
            readiness: None,
        }
    }

    #[test]
    fn a_variable_reference_is_found_in_either_form_and_only_as_a_whole_name() {
        assert!(references("postgres -p $PGPORT", "PGPORT"));
        assert!(references("postgres -p \"${PGPORT}\"", "PGPORT"));
        assert!(references("serve --port=${PGPORT:-5432}", "PGPORT"));
        assert!(!references("postgres -p 5432", "PGPORT"));
        assert!(!references("echo $PGPORT_OLD", "PGPORT"));
        assert!(!references("echo ${PGPORTX}", "PGPORT"));
    }

    #[test]
    fn only_the_named_service_must_read_the_variable() {
        let declared = [
            decl("db", "postgres -p $PGPORT"),
            decl("worker", "run-worker"),
        ];
        let requests = [("db".to_string(), "PGPORT".to_string())];
        assert!(check_substitutable(&requests, &declared).is_ok());

        let hardcoded = [decl("db", "postgres -p 5432")];
        let err = check_substitutable(&requests, &hardcoded).unwrap_err();
        assert!(
            err.contains("network.services.db.var") && err.contains("5432"),
            "{err}"
        );
    }

    #[test]
    fn substitution_overrides_the_one_variable_and_nothing_else() {
        let declared = [decl("db", "postgres -p $PGPORT"), decl("other", "x")];
        let out = substitute(
            &declared,
            &[Allocation {
                service: "db".into(),
                var: "PGPORT".into(),
                port: 23456,
            }],
        );
        assert_eq!(out[0].vars["PGPORT"], "23456");
        assert_eq!(out[0].vars["LOG"], "1");
        assert_eq!(
            out[1].vars["PGPORT"], "5432",
            "another service's value is untouched"
        );
        assert_eq!(
            declared[0].vars["PGPORT"], "5432",
            "the declaration itself is untouched"
        );
    }

    #[test]
    fn a_recorded_port_is_reclaimed_or_its_loss_announced() {
        let requests = [("db".to_string(), "PGPORT".to_string())];
        let first = allocate(&requests, &[], &[]).unwrap();
        let port = first.allocations[0].port;
        assert!(
            port >= LOW && port < ephemeral_low(),
            "{port} is outside the chosen range"
        );
        assert!(first.changed.is_empty());

        // Still free: reclaimed, nothing announced.
        let again = allocate(&requests, &first.allocations, &[]).unwrap();
        assert_eq!(again.allocations[0].port, port);
        assert!(again.changed.is_empty());

        // Taken by something else: a new port, and the change named.
        // Retried for the same reason `bindable` is: the probe just before
        // may have been inherited by a child another test is forking.
        let holder = (0..20)
            .find_map(|_| {
                TcpListener::bind(("127.0.0.1", port)).ok().or_else(|| {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    None
                })
            })
            .expect("the port frees within 200 ms");
        let moved = allocate(&requests, &first.allocations, &[]).unwrap();
        assert_ne!(moved.allocations[0].port, port);
        let note = &moved.changed[0];
        assert!(note.contains(&port.to_string()), "{note}");
        assert!(
            note.contains(&moved.allocations[0].port.to_string()),
            "{note}"
        );
        drop(holder);
    }

    #[test]
    fn two_requests_and_other_sandboxes_records_get_distinct_ports() {
        let requests = [
            ("a".to_string(), "P".to_string()),
            ("b".to_string(), "P".to_string()),
        ];
        let reserved: Vec<u16> = (LOW..LOW + 5).collect();
        let out = allocate(&requests, &[], &reserved).unwrap();
        assert_ne!(out.allocations[0].port, out.allocations[1].port);
        assert!(out.allocations.iter().all(|a| !reserved.contains(&a.port)));
    }
}
