//! Measures what devcroft's *own* network policy actually enforces on this
//! host, by applying the same `CapabilityPlan` the keeper applies
//! (`bin/devcroft.rs::self_restrict`) and then attempting connections.
//!
//! Written for the 1.0 gate `docs/roadmap.md` names — "Seatbelt is
//! implemented and has never run on a CI host. Domain filtering there is
//! unverified" — and for the open question in `docs/known-gaps.md`, whether
//! macOS `NetworkMode::ProxyOnly` narrows or merely permits.
//!
//! Reading nono's macOS source answers it on paper: `ProxyOnly` emits
//! `(deny network*)` and then a scoped `(allow network-outbound (remote tcp
//! "localhost:PORT"))`. That is what this project has *already* done and
//! recorded as insufficient — "does not ship a security claim it hasn't
//! measured". So this runs it.
//!
//! Four probes per mode, and the last two are the interesting ones:
//!   proxy    connect to the granted proxy port on loopback
//!   other    connect to a *different* loopback port
//!   direct   connect to an off-host IP, no name resolution involved
//!   resolve  resolve a name — macOS grants mDNSResponder in every
//!            restricted mode, where Linux's namespace has no route out
//!
//! `allowall` is the control. Without it a probe that fails for an
//! unrelated reason (no network, a firewall) reads as "enforced", which is
//! the failure mode this whole exercise exists to avoid.

use devcroft::policy::CapabilityPlan;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::time::Duration;

fn plan(mode: &str, proxy: u16, other: u16) -> CapabilityPlan {
    let mut p = CapabilityPlan {
        filesystem_allow: vec![],
        filesystem_read: vec![],
        filesystem_deny: vec![],
        network_block: false,
        network_ports: vec![],
        network_proxy_port: None,
        unix_socket_bind: vec![],
        signal_mode: "isolated".to_string(),
    };
    match mode {
        // The control: what `network.default = "allow"` compiles to.
        "allowall" => {}
        // `network.default = "deny"` with no allowlist.
        "blocked" => p.network_block = true,
        // `network.allow = [...]`, once `up` has started the egress proxy.
        "proxyonly" => p.network_proxy_port = Some(proxy),
        // As above, plus a declared `network.ports` entry — the branch that
        // turns on Seatbelt's blanket bind/inbound allow.
        // A declared `network.ports` entry turns on Seatbelt's blanket
        // bind/inbound allow. The port declared is deliberately *not* one
        // the probes dial, so `other` still tests whether that blanket
        // leaks into outbound.
        "proxyonly_ports" => {
            p.network_proxy_port = Some(proxy);
            p.network_ports = vec![other.wrapping_add(1)];
        }
        // Distinguishes *why* `resolve` fails under `allowall`: macOS
        // resolves through a unix socket at a filesystem path, and the
        // plans above grant no filesystem at all. If this succeeds, the
        // denial is a missing grant and devcroft could choose to make it;
        // if it still fails, name resolution is closed on macOS for a
        // reason no grant fixes.
        "allowall_dns" => {
            p.filesystem_read = vec![
                "/private/var/run/mDNSResponder".to_string(),
                "/var/run/mDNSResponder".to_string(),
                "/etc".to_string(),
                "/private/etc".to_string(),
            ];
        }
        other => panic!("unknown mode {other}"),
    }
    p
}

/// The same modes, but over devcroft's **real** compiled baseline rather
/// than an empty plan — `policy::compile` on a minimal manifest, which is
/// what an actual sandbox runs under. The empty plan grants no filesystem
/// at all, and macOS resolves names through a socket at a filesystem path,
/// so without this the `resolve` row measures a missing grant rather than
/// the network policy.
fn real_plan(mode: &str, proxy: u16, other: u16) -> CapabilityPlan {
    let toml = format!(
        "[sandbox]\nname = \"probe\"\n[env]\nprovider = \"flox\"\n{}",
        match mode {
            "real_blocked" => "[network]\ndefault = \"deny\"\n".to_string(),
            _ => String::new(),
        }
    );
    let (manifest, _warnings) = devcroft::config::parse(&toml).expect("probe manifest");
    let mut p = devcroft::policy::compile(&manifest).to_capability_plan();
    if mode == "real_proxyonly" {
        p.network_block = false;
        p.network_proxy_port = Some(proxy);
    }
    let _ = other;
    p
}

fn probe(label: &str, f: impl FnOnce() -> std::io::Result<()>) {
    match f() {
        Ok(()) => println!("{label}=ok"),
        Err(e) => println!(
            "{label}=denied({}: {})",
            e.kind(),
            e.to_string().replace('\n', " ")
        ),
    }
}

fn connect(addr: SocketAddr) -> std::io::Result<()> {
    TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map(|_| ())
}

fn child(mode: &str, proxy: u16, other: u16) {
    let cwd = std::env::current_dir().unwrap();
    // The true control: no sandbox at all. Without it, a probe that fails
    // for a reason unrelated to policy — no network, a filesystem grant the
    // plan never made — is indistinguishable from one the policy refused.
    if mode == "none" {
        println!("apply=skipped");
        probes(proxy, other);
        return;
    }
    let built = if mode.starts_with("real_") {
        real_plan(mode, proxy, other)
    } else {
        plan(mode, proxy, other)
    };
    println!(
        "plan: block={} proxy={:?} ports={:?} fs_allow={} fs_read={}",
        built.network_block,
        built.network_proxy_port,
        built.network_ports,
        built.filesystem_allow.len(),
        built.filesystem_read.len()
    );
    let caps = built.to_capability_set(&cwd).unwrap();
    if let Err(e) = nono::Sandbox::apply_auto(&caps) {
        println!("apply=failed({e})");
        std::process::exit(1);
    }
    println!("apply=ok");
    probes(proxy, other);
}

fn probes(proxy: u16, other: u16) {
    probe("proxy", || {
        connect(SocketAddr::from((Ipv4Addr::LOCALHOST, proxy)))
    });
    probe("other", || {
        connect(SocketAddr::from((Ipv4Addr::LOCALHOST, other)))
    });
    // 1.1.1.1 by address: no name lookup, so this isolates connect() from
    // resolution. A denial here is the gate doing its job.
    probe("direct", || connect(SocketAddr::from(([1, 1, 1, 1], 443))));
    // Resolution only — the socket that follows is never opened. Three
    // rungs, because "cannot resolve" has three different causes and they
    // have different consequences for what devcroft should claim:
    //   numeric  no lookup at all — fails only if the resolver library
    //            itself cannot run
    //   local    served from /etc/hosts — no DNS query
    //   remote   a real DNS query
    probe("resolve_numeric", || {
        ("127.0.0.1", 443).to_socket_addrs().map(|mut it| {
            it.next();
        })
    });
    probe("resolve_local", || {
        ("localhost", 443).to_socket_addrs().map(|mut it| {
            it.next();
        })
    });
    probe("resolve_remote", || {
        ("example.com", 443).to_socket_addrs().map(|mut it| {
            it.next();
        })
    });
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 4 {
        child(&args[1], args[2].parse().unwrap(), args[3].parse().unwrap());
        return;
    }

    // Both listeners are bound before any child is restricted, so a denial
    // below is the policy refusing, never a missing peer.
    let l1 = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let l2 = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let proxy = l1.local_addr().unwrap().port();
    let other = l2.local_addr().unwrap().port();
    for l in [l1, l2] {
        std::thread::spawn(move || {
            for s in l.incoming() {
                drop(s);
            }
        });
    }

    let exe = std::env::current_exe().unwrap();
    println!("host: proxy port {proxy}, other port {other}\n");
    for mode in [
        "none",
        "allowall",
        "blocked",
        "proxyonly",
        "proxyonly_ports",
        "real_blocked",
        "real_proxyonly",
        "allowall_dns",
    ] {
        let out = std::process::Command::new(&exe)
            .args([mode, &proxy.to_string(), &other.to_string()])
            .output()
            .unwrap();
        println!("--- {mode} ---");
        std::io::stdout().write_all(&out.stdout).unwrap();
        if !out.stderr.is_empty() {
            std::io::stdout().write_all(&out.stderr).unwrap();
        }
        println!();
    }
}
