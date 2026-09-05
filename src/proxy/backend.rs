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

/// One brokered route as it crosses the exec boundary into the proxy process.
///
/// **The secret is not in here.** It travels as its own environment variable
/// (`Broker::secret_var`) and the route names it by `env://` reference, so the
/// value never sits inside a JSON blob that gets logged, truncated, or printed
/// in a spawn error.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BrokerRoute {
    pub prefix: String,
    pub upstream: String,
    pub header: String,
    /// The environment variable in *this* process holding the secret.
    pub secret_var: String,
}

/// The host of an absolute `http(s)` URL, without port or path. Shared with
/// `config::validate` so the manifest check and the bypass detection can never
/// disagree about what a route's upstream *is*.
pub fn upstream_host(upstream: &str) -> Option<String> {
    let rest = upstream
        .strip_prefix("https://")
        .or_else(|| upstream.strip_prefix("http://"))?;
    let host = rest
        .split('/')
        .next()?
        .split('@')
        .next_back()?
        .split(':')
        .next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// The configuration devcroft hands `nono-proxy`.
///
/// A pure function on purpose: `tests/proxy_refused_capabilities.rs` asserts
/// the three capabilities devcroft refuses are off without starting a server,
/// which a config built inline inside `start` could not offer. That assertion
/// is the change's D3 — the failure mode is not someone enabling TLS
/// interception, it is an upstream default moving in a minor release and
/// devcroft inheriting a capability its threat model denies having.
pub fn proxy_config(
    allow: Vec<String>,
    token: &str,
    routes: Vec<BrokerRoute>,
) -> nono_proxy::ProxyConfig {
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

        // Brokered credentials (`brokered-credentials`). `credential_key` is an
        // `env://` reference the crate resolves from this process's own
        // environment — the value was resolved host-side at `up`, in the
        // trusted phase, and put there by `proxy::spawn`.
        //
        // `credential_format` is left `None` on purpose: the crate builds
        // `Bearer {}` for an `Authorization` header and the bare secret for any
        // other, which is exactly right for `x-api-key`. Setting it would mean
        // devcroft carrying a second per-provider fact it does not need.
        routes: routes
            .into_iter()
            .map(|r| nono_proxy::config::RouteConfig {
                prefix: r.prefix,
                upstream: r.upstream,
                credential_key: Some(format!("env://{}", r.secret_var)),
                inject_header: r.header,
                ..Default::default()
            })
            .collect(),

        // The three devcroft refuses. Compiled into the dependency graph and
        // unreachable here; see `docs/nono-feature-gating-issue.md` for the
        // upstream request that would let them be compiled out instead.
        intercept_ca_dir: None,
        intercept_parent_ca_pems: None,
        preloaded_ca: None,

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
    routes: Vec<BrokerRoute>,
) -> io::Result<(u16, &'static nono_proxy::ProxyHandle)> {
    let config = proxy_config(allow, token, routes);
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
/// `brokered` maps an upstream host to the route prefix that brokers it, so a
/// `Connect` to one can be reported as the bypass it is — see the `bypass`
/// arm below.
pub fn drain_into_log(
    handle: &'static nono_proxy::ProxyHandle,
    log: std::path::PathBuf,
    brokered: std::collections::BTreeMap<String, String>,
) -> ! {
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
            // A `Connect` to a host this manifest brokers is, by construction,
            // a client that ignored `{PREFIX}_BASE_URL` and dialled the real
            // upstream — the correct path is a plaintext request to the local
            // route, which arrives as `Reverse`.
            //
            // **Reported, not prevented, and that is a limitation rather than a
            // choice.** The crate's reverse path checks the *same* host filter
            // as CONNECT does (`reverse.rs`), so denying the direct route would
            // deny the brokered one with it. Preventing this needs TLS
            // interception, which is an explicit non-goal. Without this line
            // the bypass surfaces only as a `401` from the upstream, which
            // reads as a bad key rather than as an unused route.
            let bypassed = matches!(ev.mode, nono::undo::NetworkAuditMode::Connect)
                .then(|| brokered.get(&ev.target.to_ascii_lowercase()))
                .flatten();
            if let Some(prefix) = bypassed {
                let _ = writeln!(
                    f,
                    "bypass host={} port={port} route={prefix} \
                     reason=brokered-route-not-used \
                     detail=the client dialled the upstream directly instead of \
                     ${}_BASE_URL, so no credential was injected",
                    ev.target,
                    prefix.to_uppercase()
                );
            }

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
