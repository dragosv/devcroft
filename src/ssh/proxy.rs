//! `devcroft proxy <host>` (ssh spec's "ProxyCommand bridging"
//! requirement, task 6.2): a dumb byte-for-byte bridge between this
//! process's own stdio and the target sandbox's ssh socket. It does not
//! participate in the SSH protocol at all — the real ssh client on the
//! other end of stdio (invoked via `ProxyCommand devcroft proxy %n`, per
//! design.md decision 3's `ssh-config` block) is the one actually
//! speaking SSH to the keeper's embedded server; this is exactly what
//! `ssh -W` or `nc -U` would do in its place.

use std::fmt;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::thread;

use crate::lifecycle::{Health, StatePaths, health};

#[derive(Debug)]
pub enum ProxyError {
    /// No healthy keeper for this sandbox (the ssh spec's "exiting
    /// non-zero with a clear error when the sandbox does not exist or is
    /// not up" — the two cases aren't distinguishable from here, since
    /// there is no separate registry of "known sandboxes" beyond state
    /// dirs, same as `exec`/`shell`'s identical `NotRunning`).
    NotRunning,
    /// No fleet agent is listening at this name: stopped, crashed, or never
    /// started.
    AgentNotRunning {
        id: String,
        fleet: String,
    },
    Connect(io::Error),
}

impl fmt::Display for ProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProxyError::NotRunning => {
                write!(f, "sandbox is not up; run `devcroft up` first")
            }
            ProxyError::AgentNotRunning { id, fleet } => write!(
                f,
                "agent {id} of fleet '{fleet}' is not running; see `devcroft fleet ls`"
            ),
            ProxyError::Connect(e) => write!(f, "connecting to keeper: {e}"),
        }
    }
}

impl std::error::Error for ProxyError {}

/// Parses the sandbox name out of a `<name>.devcroft` host argument (the
/// `%n` real ssh clients substitute into `ProxyCommand devcroft proxy
/// %n`). Anything else is a usage error — this command is only ever
/// meaningfully invoked with that shape, per the `ssh-config` block this
/// crate itself emits.
///
/// Deliberately does not itself validate the extracted name against the
/// config spec's slug constraint (`../../victim.devcroft` strips to
/// `../../victim` and passes this function fine) — `proxy`'s call to
/// `StatePaths::new` right after is where that is enforced, the same
/// choke point every other caller of a sandbox name goes through. Found
/// missing by adversarial review; fixed at the choke point rather than
/// here, so any future parser feeding a name into `StatePaths` gets it
/// for free too.
pub fn sandbox_name_from_host(host: &str) -> Result<&str, String> {
    host.strip_suffix(".devcroft")
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("'{host}' is not a <name>.devcroft host"))
}

/// A fleet agent's `<id>.<fleet>.devcroft` host (the proposal's
/// `ssh a17.myrepo.devcroft`): the agent ID and the fleet's sandbox name,
/// or `None` for a host with no dot before `.devcroft`, which names a
/// sandbox. Unambiguous because a sandbox name cannot contain a dot.
///
/// A dotted host that is not exactly `a<N>.<valid name>` is an error, not a
/// sandbox name: `../victim.devcroft` has a dot too, and neither half may
/// reach a path unvalidated.
pub fn fleet_agent_from_host(host: &str) -> Result<Option<(&str, &str)>, String> {
    let name = sandbox_name_from_host(host)?;
    let Some((id, fleet)) = name.split_once('.') else {
        return Ok(None);
    };
    let is_id = id.strip_prefix('a').is_some_and(|n| {
        !n.starts_with('0') && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
    });
    if !is_id || !crate::config::is_valid_name(fleet) {
        return Err(format!(
            "'{host}' is neither <name>.devcroft nor <agent>.<name>.devcroft"
        ));
    }
    Ok(Some((id, fleet)))
}

/// Connects to `sandbox_name`'s ssh socket and bridges it to this
/// process's own stdin/stdout until either side closes. Returns once the
/// bridge has fully drained in both directions.
pub fn proxy(sandbox_name: &str) -> Result<(), ProxyError> {
    let paths = StatePaths::new(sandbox_name).map_err(ProxyError::Connect)?;
    match health(&paths).map_err(ProxyError::Connect)? {
        Health::Healthy(_) => {}
        Health::Stale(_) | Health::None => return Err(ProxyError::NotRunning),
    }
    bridge(UnixStream::connect(&paths.ssh_socket).map_err(ProxyError::Connect)?)
}

/// [`proxy`] for one agent of a fleet: its own SSH socket, which its keeper
/// serves on a listener the supervisor bound before the agent was
/// restricted. No health probe beyond connecting: a stopped agent's socket
/// is removed, and a crashed one's refuses, so either way the connect is
/// the answer. Never starts anything, unlike `proxy`'s auto-`up`: which
/// agents exist is `fleet up`'s decision.
#[cfg(target_os = "linux")]
pub fn proxy_fleet_agent(fleet: &str, id: &str) -> Result<(), ProxyError> {
    let state = crate::lifecycle::fleet_state_dir(fleet).map_err(ProxyError::Connect)?;
    let not_running = || ProxyError::AgentNotRunning {
        id: id.to_owned(),
        fleet: fleet.to_owned(),
    };
    let socket = crate::fleet::supervisor::agent_ssh_socket(&state, id).ok_or_else(not_running)?;
    bridge(UnixStream::connect(&socket).map_err(|_| not_running())?)
}

fn bridge(socket: UnixStream) -> Result<(), ProxyError> {
    let mut socket_read = socket;
    let mut socket_write = socket_read.try_clone().map_err(ProxyError::Connect)?;

    // stdin -> socket, on its own thread so both directions can run
    // concurrently; this thread's own exit (stdin EOF, or a write error
    // once the far end hangs up) doesn't need to be observed beyond the
    // final `join` below — by then there's nothing left to relay either
    // way.
    let to_socket = thread::spawn(move || {
        relay(&mut io::stdin(), &mut socket_write);
        let _ = socket_write.shutdown(std::net::Shutdown::Write);
    });

    // socket -> stdout, on this thread: returns once the keeper's ssh
    // server closes its end (session ended) or errors.
    relay(&mut socket_read, &mut io::stdout());

    let _ = to_socket.join();
    Ok(())
}

/// A manual read/write loop — deliberately *not* `std::io::copy`, which
/// hung indefinitely here in testing (`tests/proxy_up.rs` reproduced it
/// reliably): std's specialized fast path for this exact reader/writer
/// shape apparently doesn't push bytes through a unix-domain-socket <->
/// piped-stdio bridge the way the plain generic loop does. Flushing after
/// every chunk matters for the same reason `io::copy` alone wasn't
/// enough — the far end (a real ssh client during the handshake) is
/// waiting on exactly these bytes to arrive before it sends anything
/// back.
fn relay(from: &mut impl Read, to: &mut impl Write) {
    let mut buf = [0u8; 8192];
    loop {
        let n = match from.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        if to.write_all(&buf[..n]).is_err() || to.flush().is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_name_from_host_strips_the_devcroft_suffix() {
        assert_eq!(sandbox_name_from_host("myproj.devcroft"), Ok("myproj"));
    }

    #[test]
    fn a_fleet_agent_host_names_its_agent_and_fleet() {
        assert_eq!(
            fleet_agent_from_host("a17.myrepo.devcroft"),
            Ok(Some(("a17", "myrepo")))
        );
        assert_eq!(fleet_agent_from_host("myrepo.devcroft"), Ok(None));
        for host in [
            "../victim.devcroft",
            "a1.../x.devcroft",
            "a01.myrepo.devcroft",
            "b1.myrepo.devcroft",
            "a1.my.repo.devcroft",
            "a1.MyRepo.devcroft",
        ] {
            assert!(fleet_agent_from_host(host).is_err(), "{host}");
        }
    }

    #[test]
    fn sandbox_name_from_host_rejects_other_hosts() {
        assert!(sandbox_name_from_host("myproj").is_err());
        assert!(sandbox_name_from_host("myproj.example.com").is_err());
        assert!(sandbox_name_from_host(".devcroft").is_err());
        assert!(sandbox_name_from_host("").is_err());
    }
}
