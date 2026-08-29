//! `add-linux-agent-fleet`'s `service-ports` capability, at the level it
//! actually rests on: N agents each get their own network namespace, so
//! N agents can each bind the *same* declared port without colliding.
//!
//! This is the "every agent gets its own Postgres" claim, reduced to the
//! mechanism that makes it true. The spec requires the in-namespace port
//! to be used unchanged in every agent — no allocation, no injected
//! variable, no cooperation from the service — so the test binds one
//! fixed number in several namespaces at once and checks each reaches
//! its own listener.
//!
//! Deliberately independent of fleet's open D5 question (pasta vs
//! slirp4netns for *egress*): nothing here needs a forwarding helper or
//! `/dev/net/tun`, which is the finding that let this ship first.

use std::io::Read;
use std::process::Command;

/// The port every simulated agent binds *within one test*. A real,
/// recognisable service port rather than an arbitrary high one, because
/// using the same number in every namespace is the property under test —
/// and 5432 is the number fleet's own proposal uses when describing the
/// collision.
const SHARED_PORT: u16 = 5432;

/// The two tests below need ports of their **own**, distinct from
/// `SHARED_PORT` and from each other, because cargo runs a binary's
/// tests concurrently in one process and both of these care about what
/// is reachable in the *host's* namespace.
///
/// Found by writing them: the host-reachability test initially reused
/// `SHARED_PORT` and failed, having connected to the host listener that
/// `agents_do_not_take_the_hosts_port` holds open — a true observation
/// ("something answered on 5432") about the wrong process. Sharing a
/// port is only meaningful *between namespaces*, which is one test's
/// subject; between tests it is just interference.
const HOST_HOLD_PORT: u16 = 5433;
const HOST_REACH_PORT: u16 = 5434;

/// Skips rather than fails where per-agent namespaces are unavailable —
/// a container runtime's seccomp profile, an AppArmor policy restricting
/// unprivileged user namespaces, or an exhausted `max_user_namespaces`
/// can each deny this independently, and none of them is a devcroft bug.
/// Probed by running the real thing, matching how `policy::
/// backend_support` gates the Landlock tests.
///
/// **Deliberately asks strictly less than the tests assert.** The bare
/// `__netns_probe` only creates a namespace; it does not bring loopback
/// up or check reachability. An earlier version used the full probe for
/// both, and when `bring_loopback_up` was disabled to check these tests
/// had teeth, all four reported `ok` — the guard had hit the same
/// failure and skipped everything silently. A broken feature looked
/// exactly like an unsupported host. A gate must never depend on the
/// behaviour it gates.
fn namespaces_available() -> bool {
    Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("__netns_probe")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn a_fresh_namespace_gives_a_reachable_loopback() {
    if !namespaces_available() {
        eprintln!("skipping: this host cannot create unprivileged network namespaces");
        return;
    }

    // The probe asserts *reachability*, not that `bind()` returned Ok.
    // That distinction is the whole point: in a namespace whose loopback
    // is still DOWN, `bind` succeeds and the client gets ENETUNREACH, so
    // a bind-only assertion passes against a service that starts,
    // reports healthy, and can never be connected to.
    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("__netns_probe")
        .arg("--reachable")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "a service bound inside a fresh namespace must be reachable from inside it"
    );
}

#[test]
fn several_agents_bind_the_same_port_without_colliding() {
    if !namespaces_available() {
        eprintln!("skipping: this host cannot create unprivileged network namespaces");
        return;
    }

    // Five, matching the fleet task's own wording ("five agents bind the
    // same declared port"). More than two because two could pass by an
    // accident of scheduling; five concurrent binds of one number would
    // not.
    const AGENTS: usize = 5;

    let handles: Vec<_> = (0..AGENTS)
        .map(|i| {
            std::thread::spawn(move || {
                let agent = format!("a{i}");
                let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
                    .arg("__netns_agent_sim")
                    .arg(&agent)
                    .arg(SHARED_PORT.to_string())
                    .output()
                    .unwrap();
                (agent, out)
            })
        })
        .collect();

    for handle in handles {
        let (agent, out) = handle.join().unwrap();
        assert!(
            out.status.success(),
            "agent {agent} failed to bind and reach {SHARED_PORT} in its own namespace: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        // Each agent must reach *its own* listener, not a neighbour's —
        // the identity check is what distinguishes real namespace
        // isolation from five processes taking turns on one host port.
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            stdout.trim(),
            format!("served-by-{agent}"),
            "agent {agent} reached the wrong listener"
        );
    }
}

#[test]
fn agents_do_not_take_the_hosts_port() {
    if !namespaces_available() {
        eprintln!("skipping: this host cannot create unprivileged network namespaces");
        return;
    }

    // Hold the host's own copy of the port for the duration, then run an
    // agent that binds the same number. Both must succeed: if the agent
    // were binding in the host's namespace it would get EADDRINUSE, so
    // this fails closed against the isolation silently not happening.
    let host_listener = match std::net::TcpListener::bind(("127.0.0.1", HOST_HOLD_PORT)) {
        Ok(l) => l,
        Err(_) => {
            eprintln!("skipping: something on this host already holds {HOST_HOLD_PORT}");
            return;
        }
    };

    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("__netns_agent_sim")
        .arg("solo")
        .arg(HOST_HOLD_PORT.to_string())
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "an agent must bind {HOST_HOLD_PORT} even while the host holds it: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    drop(host_listener);
}

/// Guards the claim the whole capability makes to a *developer*: an
/// agent's services are private unless something maps them out. The spec
/// states this as the default ("reachable from inside that agent and
/// from nowhere else"), so it is asserted rather than assumed.
#[test]
fn an_agents_service_is_not_reachable_from_the_host() {
    if !namespaces_available() {
        eprintln!("skipping: this host cannot create unprivileged network namespaces");
        return;
    }

    // Start an agent that holds its listener open, then try to reach it
    // from this (host-namespace) process on the same port.
    let mut child = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("__netns_agent_sim")
        .arg("hidden")
        .arg(HOST_REACH_PORT.to_string())
        .arg("--hold")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    // Wait for it to report readiness rather than sleeping a guessed
    // interval — a fixed sleep would make this test's meaning depend on
    // machine speed.
    let mut ready = [0u8; 5];
    let read = child
        .stdout
        .as_mut()
        .unwrap()
        .read_exact(&mut ready)
        .is_ok();

    let host_reach = std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{HOST_REACH_PORT}").parse().unwrap(),
        std::time::Duration::from_millis(500),
    );

    let _ = child.kill();
    let _ = child.wait();

    assert!(read, "the agent should have signalled readiness");
    assert!(
        host_reach.is_err(),
        "an agent's service must not be reachable from the host without an explicit mapping"
    );
}

/// Not a test — a compile-time reminder of what this file does *not*
/// cover, so a later reader does not mistake its scope.
///
/// Absent here, and correctly so: the optional host-side port mapping
/// (which needs the forwarding helper D5 has not selected), egress of any
/// kind, cgroups, pid/mount namespaces, and the supervisor. This file
/// covers exactly the slice the D5 spike showed was independently
/// deliverable.
#[allow(dead_code)]
fn scope_note() {}
