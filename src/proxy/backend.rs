//! The `nono-proxy` instance devcroft runs inside its own proxy process
//! (`adopt-nono-proxy`, design D1/D6/D7).
//!
//! **What devcroft keeps** is the process: `proxy::spawn` binds the listeners,
//! records the pid, and `up`/`down`/`rm` own the lifetime;
//! `CompiledPolicy::network_proxy_port` and the `NetworkMode::ProxyOnly` kernel
//! gate are unchanged. What moves here is the accept loop's *decisions* —
//! authentication, host filtering, and (later) credential brokering.
//!
//! **Why a bridge rather than a direct swap** (D7): `nono_proxy::start` binds
//! its own `TcpListener` from `bind_addr`/`bind_port` and accepts neither a
//! pre-bound fd nor a unix socket. devcroft needs both — the fd-passed TCP
//! listener is how `spawn` learns the port before the policy is compiled, and
//! the unix socket is the *only* path for a network-isolated sandbox, whose
//! loopback-only namespace cannot see the host's `127.0.0.1` at all. So
//! nono-proxy binds an ephemeral loopback port of its own and devcroft's two
//! acceptors splice into it. The unix bridge already existed for the same
//! reason; this adds its TCP twin.

use std::io;

/// The configuration devcroft hands `nono-proxy`.
///
/// A pure function on purpose: `tests/proxy_refused_capabilities.rs` asserts
/// the three capabilities devcroft refuses are off without starting a server,
/// which a config built inline inside `start` could not offer. That assertion
/// is the change's D3 — the failure mode is not someone enabling TLS
/// interception, it is an upstream default moving in a minor release and
/// devcroft inheriting a capability its threat model denies having.
pub fn proxy_config(allow: Vec<String>, token: &str) -> nono_proxy::ProxyConfig {
    nono_proxy::ProxyConfig {
        // Ephemeral: the port the *policy* names is the one `spawn` bound and
        // fd-passed, which this instance never sees. Nothing outside this
        // process connects here.
        bind_addr: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        bind_port: 0,

        allowed_hosts: allow,
        // An empty allowlist denies rather than falls back to allow-all —
        // `proxy::server::HostFilter::new_strict`'s rule, preserved. `spawn` is
        // only called when `wants_egress_proxy()` is true, so the list is never
        // empty in practice; a proxy that failed open on one would be a silent
        // footgun for any future caller that forgets to check.
        strict_filter: true,

        // D6. The token stays devcroft's — minted in `proxy::spawn`, delivered
        // as `DEVCROFT_EGRESS_TOKEN` — and only the checking moves here.
        require_auth: true,
        session_token: Some(zeroize::Zeroizing::new(token.to_string())),
        // **Deliberately against the crate's default of `false`.** That default
        // rests on undici not echoing URL-userinfo as `Proxy-Authorization` on
        // CONNECT; measured otherwise (undici 8.10.2 / Node 22.22.3 send it, as
        // does curl 8.7.1). Taking the default would tunnel CONNECT
        // unauthenticated and reopen the open-relay hole `add-egress-proxy`
        // task group 4a closed.
        strict_connect_auth: true,

        // The three devcroft refuses. Compiled into the dependency graph and
        // unreachable here; see `docs/nono-feature-gating-issue.md` for the
        // upstream request that would let them be compiled out instead.
        intercept_ca_dir: None,
        intercept_parent_ca_pems: None,
        preloaded_ca: None,
        routes: Vec::new(),

        ..Default::default()
    }
}

/// Starts the instance and returns the loopback port devcroft's acceptors
/// splice into.
///
/// The runtime and the handle are leaked rather than returned by value: this
/// process exists only to run the proxy and never returns from
/// `egress_proxy_main`, so there is no scope for them to live in and dropping
/// either would stop serving. The handle is handed back as `&'static` because
/// [`drain_into_log`] needs it for the lifetime of the process.
pub fn start(
    allow: Vec<String>,
    token: &str,
) -> io::Result<(u16, &'static nono_proxy::ProxyHandle)> {
    let config = proxy_config(allow, token);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let handle = runtime
        .block_on(nono_proxy::server::start(config))
        .map_err(|e| io::Error::other(format!("starting nono-proxy: {e}")))?;
    let port = handle.port;
    let handle: &'static nono_proxy::ProxyHandle = Box::leak(Box::new(handle));
    Box::leak(Box::new(runtime));
    Ok((port, handle))
}

/// Renders `nono-proxy`'s audit events as the lines devcroft's proxy log has
/// always carried, and never returns.
///
/// **Why this exists rather than letting the crate's own logging stand.** The
/// proxy log is a devcroft interface, not an implementation detail: `logs`
/// surfaces it and `tests/egress_proxy_e2e.rs` asserts on its shape — that an
/// allowed request records `allow` with its port and a refused one records
/// `refuse` naming the host. Swapping the accept loop without this would have
/// kept every kernel-level property and silently emptied the one file a user
/// looks at to see what the proxy decided.
///
/// Polled rather than pushed because `drain_audit_events` is the only
/// interface offered, and buffered upstream to 4096 events — so the interval
/// is a liveness choice, not a correctness one.
pub fn drain_into_log(handle: &'static nono_proxy::ProxyHandle, log: std::path::PathBuf) -> ! {
    use std::io::Write;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let events = handle.drain_audit_events();
        if events.is_empty() {
            continue;
        }
        let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&log) else {
            continue;
        };
        for ev in events {
            let port = ev
                .port
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".to_string());
            let line = match ev.decision {
                nono::undo::NetworkAuditDecision::Allow => {
                    format!("allow host={} port={port}", ev.target)
                }
                other => {
                    // `reason` is upstream text about a client-supplied
                    // hostname; newlines are stripped so one event can never
                    // forge a second log record.
                    let reason = ev
                        .reason
                        .unwrap_or_else(|| format!("{other:?}"))
                        .replace(['\n', '\r'], " ");
                    format!("refuse host={} port={port} reason={reason}", ev.target)
                }
            };
            let _ = writeln!(f, "{line}");
        }
    }
}
