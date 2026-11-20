//! The egress proxy's protocol handling.
//!
//! **The HTTP side is `nono-proxy`** (`add-agent-workload` 4b.0, adopted
//! 2026-11-15): it reads `CONNECT host:port` and absolute-URI requests,
//! checks the per-session token, decides per hostname with
//! `nono::HostFilter` (by name first, so a refused name never reaches the
//! resolver), and splices. No TLS interception: `intercept_ca_dir` stays
//! `None`, a non-goal of `add-egress-proxy`. It also serves credential
//! routes, which is why it was adopted (design.md Decision 6).
//!
//! What stays devcroft's is everything around it: the unix-socket bridge
//! a network-isolated sandbox reaches the proxy through, the relay inside
//! that sandbox's namespace, the admission cap on the bridges, and the log
//! format the proxy log has always had (`allow …` / `refuse …`), now
//! written from `nono-proxy`'s audit events.

use std::io::{self, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use base64::Engine;
use nono::undo::{NetworkAuditDecision, NetworkAuditDenialCategory, NetworkAuditEvent};

/// Bytes read for the request head the unix bridge rewrites before giving
/// up — a hostile or confused client gets a fast, bounded failure rather
/// than unbounded memory growth.
const MAX_HEADER_BYTES: usize = 64 * 1024;

/// Runs inside a network-isolated sandbox: accepts on a TCP listener
/// bound in *that* namespace and forwards each connection to the host's
/// proxy over a unix socket.
///
/// This is the sandbox-side half of the pair whose host side is
/// [`bridge_unix_to_tcp`]. It exists because the two ends live in
/// different network namespaces and a unix socket is the only thing that
/// crosses: the sandbox has its own port table and no route to the
/// host's loopback.
///
/// **The listener is bound before the keeper restricts itself**, the same
/// listener-before-restriction ordering the control and SSH sockets use —
/// so no bind permission is needed after restriction. Connecting to the
/// unix socket afterwards needs no grant either, because Landlock does
/// not mediate AF_UNIX at all (`tests/unix_socket_not_mediated.rs`).
///
/// It binds the *same port number* the host proxy uses, which is what
/// keeps `HTTP_PROXY` and the compiled `proxy_only` gate byte-identical
/// whether or not a sandbox is isolated. Safe because a fresh namespace
/// has every port free — the number only has to avoid this sandbox's own
/// `network.ports`, which `up` checks before choosing isolation.
pub fn relay_to_host_proxy(listener: TcpListener, socket_path: PathBuf) {
    for stream in listener.incoming() {
        let Ok(client) = stream else { continue };
        let socket_path = socket_path.clone();
        thread::spawn(move || {
            let Ok(upstream) = std::os::unix::net::UnixStream::connect(&socket_path) else {
                return;
            };
            splice_tcp_to_unix(client, upstream);
        });
    }
}

/// [`splice`] with the stream types the other way round.
fn splice_tcp_to_unix(client: TcpStream, upstream: std::os::unix::net::UnixStream) {
    let (Ok(client_r), Ok(upstream_r)) = (client.try_clone(), upstream.try_clone()) else {
        return;
    };
    let to_upstream = {
        let mut r = client_r;
        let mut w = upstream;
        thread::spawn(move || {
            let _ = io::copy(&mut r, &mut w);
            let _ = w.shutdown(std::net::Shutdown::Write);
        })
    };
    let mut r = upstream_r;
    let mut w = client;
    let _ = io::copy(&mut r, &mut w);
    let _ = w.shutdown(std::net::Shutdown::Write);
    let _ = to_upstream.join();
}

/// Bridges a unix listener into the TCP one above, one hairpin
/// connection per accept.
///
/// **Why a hairpin.** The decision logic (auth before the allowlist, the
/// `407`/`403` split, `HostFilter`, the splice) is `nono-proxy`'s, and it
/// accepts on a TCP listener only. Forwarding UDS bytes into that port
/// reuses all of it unchanged, at the cost of one loopback hop inside a
/// process that is already on the host and unrestricted.
///
/// This is what makes a network-isolated sandbox able to use the proxy at
/// all: it has no route to the host's loopback, but a pathname unix
/// socket crosses the namespace boundary.
pub fn bridge_unix_to_tcp(unix: std::os::unix::net::UnixListener, tcp_port: u16) {
    let open = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for stream in unix.incoming() {
        let Ok(client) = stream else { continue };
        // The same admission the TCP side has: this bridge is how a
        // network-isolated sandbox reaches the proxy, so it is the agent's
        // way to make a host-side process start threads.
        let Some(slot) = Slot::take(&open) else {
            continue;
        };
        thread::spawn(move || {
            let _slot = slot;
            let Ok(upstream) = TcpStream::connect(("127.0.0.1", tcp_port)) else {
                return;
            };
            splice_unix(client, upstream);
        });
    }
}

/// [`bridge_unix_to_tcp`] for the egress proxy's own unix socket, which
/// presents this process's token on the client's behalf.
///
/// **The socket's filesystem permissions are its authentication**, as the
/// SSH socket's are (CLAUDE.md): it lives in the sandbox's 0700 state
/// directory and is bound into that one sandbox's view, so whoever can
/// connect to it is the sandbox itself or its own user, who can read the
/// token from `meta.json` anyway. The token exists for the TCP listener,
/// which any local process can reach, and that check is unchanged: this
/// adds the header a client would have sent, so `nono-proxy` decides
/// exactly as it would for that client. It is `Basic nono:<token>`, the
/// form `nono-proxy` checks (`Basic <token>:` is refused, measured).
///
/// **Why not leave it to the client** (`add-agent-workload`, measured
/// 2026-10-18, eleven coding agents inside one sandbox): five could not use
/// a token in the proxy URL's userinfo. Gemini CLI (since replaced in the
/// matrix by Antigravity), Qwen Code and Cursor's
/// agent sent no `Proxy-Authorization` at all and got 407; OpenCode and Amp,
/// built on Bun, read `<token>@127.0.0.1` as the host name. The userinfo
/// shape assumed "every standard HTTP client" honours it, and that is false
/// for the clients this tool exists to run.
///
/// Only the first request's head is rewritten, which is all the proxy reads:
/// a `CONNECT` is one request, and a plain request's connection is spliced
/// after it.
pub fn bridge_unix_to_proxy(unix: std::os::unix::net::UnixListener, tcp_port: u16, token: String) {
    let header = Arc::new(format!(
        "Proxy-Authorization: Basic {}\r\n",
        base64::engine::general_purpose::STANDARD.encode(format!("nono:{token}"))
    ));
    let open = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for stream in unix.incoming() {
        let Ok(client) = stream else { continue };
        let Some(slot) = Slot::take(&open) else {
            continue;
        };
        let header = Arc::clone(&header);
        thread::spawn(move || {
            let _slot = slot;
            let Ok(mut upstream) = TcpStream::connect(("127.0.0.1", tcp_port)) else {
                return;
            };
            let mut client = client;
            let _ = client.set_read_timeout(Some(HEAD_TIMEOUT));
            let Ok(head) = read_head(&mut client) else {
                return;
            };
            let _ = client.set_read_timeout(None);
            if io::Write::write_all(&mut upstream, &with_proxy_auth(&head, &header)).is_err() {
                return;
            }
            splice_unix(client, upstream);
        });
    }
}

/// Everything up to and including the blank line ending a request head,
/// plus whatever arrived with it; at most [`MAX_HEADER_BYTES`] of head.
fn read_head(client: &mut std::os::unix::net::UnixStream) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        if buf.len() > MAX_HEADER_BYTES {
            break;
        }
        let n = io::Read::read(client, &mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(buf)
}

/// `head` with any `Proxy-Authorization` the client sent replaced by
/// `header`, inserted after the request line. A head with no complete
/// request line is passed through for the proxy to refuse as malformed.
fn with_proxy_auth(head: &[u8], header: &str) -> Vec<u8> {
    let Some(end) = head.windows(4).position(|w| w == b"\r\n\r\n") else {
        return head.to_vec();
    };
    let (headers, rest) = head.split_at(end + 2);
    let mut lines = headers.split_inclusive(|b| *b == b'\n');
    let Some(request_line) = lines.next() else {
        return head.to_vec();
    };
    let mut out = request_line.to_vec();
    out.extend_from_slice(header.as_bytes());
    for line in lines {
        let is_auth = line.len() >= 20 && line[..20].eq_ignore_ascii_case(b"proxy-authorization:");
        if !is_auth {
            out.extend_from_slice(line);
        }
    }
    out.extend_from_slice(rest);
    out
}

/// [`splice`]'s shape for a unix client, kept separate rather than made
/// generic for the same reason [`bridge_unix_to_tcp`] exists.
fn splice_unix(client: std::os::unix::net::UnixStream, upstream: TcpStream) {
    let (Ok(client_r), Ok(upstream_r)) = (client.try_clone(), upstream.try_clone()) else {
        return;
    };
    let to_upstream = {
        let mut r = client_r;
        let mut w = upstream;
        thread::spawn(move || {
            let _ = io::copy(&mut r, &mut w);
            let _ = w.shutdown(std::net::Shutdown::Write);
        })
    };
    let mut r = upstream_r;
    let mut w = client;
    let _ = io::copy(&mut r, &mut w);
    let _ = w.shutdown(std::net::Shutdown::Write);
    let _ = to_upstream.join();
}

/// A running proxy: `nono-proxy` on its own loopback port, inside a runtime
/// this value owns. Dropping it stops the proxy.
pub struct Running {
    /// The port `nono-proxy` bound, which is the port the sandbox is granted.
    pub port: u16,
    /// How many brokered routes had their credential loaded.
    loaded: usize,
    handle: nono_proxy::ProxyHandle,
    _runtime: tokio::runtime::Runtime,
}

impl Running {
    /// How many brokered routes had their credential loaded by `start`.
    pub fn loaded_routes(&self) -> usize {
        self.loaded
    }

    /// Stops accepting and shuts the proxy down.
    pub fn shutdown(&self) {
        self.handle.shutdown();
    }
}

/// The `nono-proxy` configuration devcroft runs: a strict allowlist, the
/// session token devcroft generated, authentication enforced on every
/// path, and nothing devcroft has decided against.
///
/// - **`strict_connect_auth`**: `nono-proxy`'s default lets an
///   unauthenticated `CONNECT` through to an allowed host (measured,
///   `200`), which is the open relay `add-egress-proxy` 4a fixed. With it,
///   `407`.
/// - **`strict_filter`**: an empty allowlist denies everything rather than
///   allowing everything. `spawn` only runs with a non-empty one, but a
///   proxy that fails open on an empty list is a footgun for a future
///   caller.
/// - **No TLS interception** (`intercept_ca_dir: None`), and so no OAuth
///   capture and no `CONNECT` endpoint rules, which need it.
pub fn config(
    allow: Vec<String>,
    token: String,
    routes: Vec<nono_proxy::config::RouteConfig>,
) -> nono_proxy::ProxyConfig {
    nono_proxy::ProxyConfig {
        routes,
        bind_addr: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        bind_port: 0,
        allowed_hosts: allow,
        strict_filter: true,
        require_auth: true,
        strict_connect_auth: true,
        session_token: Some(zeroize::Zeroizing::new(token)),
        max_connections: MAX_CONNECTIONS,
        intercept_ca_dir: None,
        enable_network_audit: true,
        ..Default::default()
    }
}

/// Starts `nono-proxy` with [`config`] and returns once it is listening.
///
/// `nono-proxy` binds its own listener, and `ProxyHandle::add_listener`
/// accepts only one on that same port, so the TCP port cannot be bound
/// before exec the way the unix socket is. That is not a weakening: this
/// process is never restricted, and the port is reported to `up` (over a
/// pipe, `proxy::spawn_at`) before `up` compiles it into the policy.
///
/// Every audit event becomes one line of `log_path`, prefixed
/// `agent=<label> ` for a fleet agent's proxy.
pub fn start(
    allow: Vec<String>,
    log_path: PathBuf,
    token: String,
    label: Option<String>,
    routes: Vec<nono_proxy::config::RouteConfig>,
) -> io::Result<Running> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let log = Arc::new(AuditLog {
        file: Mutex::new(file),
        label,
    });
    let cfg = config(allow, token, routes);
    let handle = runtime
        .block_on(nono_proxy::start(cfg.clone()))
        .map_err(|e| io::Error::other(format!("nono-proxy: {e}")))?;
    // `credential_env_vars` emits a route's placeholder only when its
    // credential loaded (`nono-proxy` 0.82), which is the public way to ask.
    let placeholders = handle.credential_env_vars(&cfg);
    let loaded = cfg
        .routes
        .iter()
        .filter(|r| {
            r.env_var
                .as_ref()
                .is_some_and(|v| placeholders.iter().any(|(k, _)| k == v))
        })
        .count();
    let sink_log = Arc::clone(&log);
    handle.attach_audit_sink(Arc::new(move |event: &NetworkAuditEvent| {
        log_line(&sink_log, &record(event));
        Ok(())
    }));
    Ok(Running {
        port: handle.port,
        loaded,
        handle,
        _runtime: runtime,
    })
}

/// The `nono-proxy` route for one brokered credential, reading its value
/// from `credential_var` in this process's own environment
/// (`nono-proxy`'s `env://` source). The caller sets that variable from
/// stdin before any thread starts, so the environment block the kernel
/// shows for this process never holds it.
///
/// The header's value is `nono-proxy`'s default: `Bearer <value>` for
/// `Authorization`, the bare value for any other header, which covers every
/// agent in `docs/agents.md`.
pub fn broker_route(
    route: &crate::proxy::BrokerRoute,
    credential_var: &str,
) -> nono_proxy::config::RouteConfig {
    let upstream = format!("https://{}", route.host);
    // **Test builds only.** An end-to-end test needs a route to reach a TLS
    // upstream on loopback, which `https://<host>` on 443 cannot be without
    // root (`add-agent-workload` 4b.8, owner's decision 2026-11-19). Never
    // compiled into a published build: a variable that redirects a brokered
    // credential to any URL is exfiltration by environment.
    #[cfg(feature = "test-support")]
    let upstream = std::env::var("DEVCROFT_TEST_BROKER_UPSTREAM").unwrap_or(upstream);
    nono_proxy::config::RouteConfig {
        prefix: route.prefix.clone(),
        upstream,
        credential_key: Some(format!("env://{credential_var}")),
        inject_header: route.header.clone(),
        env_var: Some(route.env.clone()),
        phantom_format: route.placeholder.clone(),
        tls_ca: route.tls_ca.clone(),
        ..Default::default()
    }
}

/// One log line for an audit event, in the shape the proxy log has always
/// had: `allow host=<h> port=<p>` or `refuse host=<h> port=<p> reason=<r>`.
///
/// **An authentication failure never uses `host=`.** `nono-proxy`'s event
/// for one names the host the caller asked for, which devcroft's own proxy
/// never logged, and `lifecycle::attribution` reads every `refuse` line
/// with `host=` and `port=` as the *allowlist* refusing that host. A token
/// problem is not a policy decision, so its target is logged as
/// `requested=<h>:<p>` instead.
fn record(event: &NetworkAuditEvent) -> String {
    let verdict = match event.decision {
        NetworkAuditDecision::Allow => "allow",
        _ => "refuse",
    };
    let auth_failed = matches!(
        event.denial_category,
        Some(NetworkAuditDenialCategory::AuthenticationFailed)
    );
    let mut line = if auth_failed {
        let mut l = format!("{verdict} requested={}", event.target);
        if let Some(port) = event.port {
            l.push_str(&format!(":{port}"));
        }
        l
    } else {
        let mut l = format!("{verdict} host={}", event.target);
        if let Some(port) = event.port {
            l.push_str(&format!(" port={port}"));
        }
        l
    };
    // A plain-HTTP allow is `nono-proxy`'s L7 event, which carries method
    // and status but no port (measured, 0.82: `forward::forward_request`
    // logs through `log_l7_request`, which has no port parameter). Log what
    // it does carry; never the path, which can hold a token.
    if event.port.is_none() {
        if let Some(method) = &event.method {
            line.push_str(&format!(" method={method}"));
        }
        if let Some(status) = event.status {
            line.push_str(&format!(" status={status}"));
        }
    }
    if let Some(route) = &event.route_id {
        line.push_str(&format!(" route={route}"));
    }
    if !matches!(event.decision, NetworkAuditDecision::Allow) {
        let reason = match event.denial_category {
            Some(NetworkAuditDenialCategory::AuthenticationFailed) => "unauthenticated".to_owned(),
            _ => event
                .reason
                .clone()
                .unwrap_or_else(|| "denied".to_owned())
                .replace(['\r', '\n'], " "),
        };
        line.push_str(&format!(" reason={reason}"));
    }
    line
}

/// The proxy's audit log, and whose egress it records.
struct AuditLog {
    file: Mutex<std::fs::File>,
    label: Option<String>,
}

fn log_line(log: &AuditLog, line: &str) {
    // One write per record, appended to a file whose fd is `O_APPEND`: the
    // proxy's runtime threads share the file, and a multi-write record can
    // interleave with another writer's (`keeper::connection::log_record`).
    let record = match &log.label {
        Some(label) => format!("agent={label} {line}\n"),
        None => format!("{line}\n"),
    };
    if let Ok(mut f) = log.file.lock() {
        let _ = f.write_all(record.as_bytes());
    }
}

/// How many connections a proxy (or its unix bridge) serves at once.
///
/// The proxy runs outside the agent, in its own leaf, so what an agent makes
/// it do is not counted against the agent's limits: measured by adversarial
/// review, 30 connections opened to an agent's proxy put 93 tasks in its
/// helpers' leaf against the agent's own `pids.max` of 64. A thread per
/// connection (two while splicing) is fine within a cap; the cap is what
/// keeps "outside the sandbox" from meaning "outside the budget". The
/// helpers' leaf has its own limits too (`fleet::supervisor`).
pub const MAX_CONNECTIONS: usize = 128;

/// One admitted connection; released when dropped.
struct Slot(Arc<std::sync::atomic::AtomicUsize>);

impl Slot {
    fn take(open: &Arc<std::sync::atomic::AtomicUsize>) -> Option<Slot> {
        use std::sync::atomic::Ordering;
        let prev = open.fetch_add(1, Ordering::SeqCst);
        if prev >= MAX_CONNECTIONS {
            open.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Slot(Arc::clone(open)))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// How long a client has to send its request head. Without one, a
/// connection that never speaks holds its slot (and its thread) forever.
const HEAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn basic_auth_header(token: &str) -> String {
        format!(
            "Proxy-Authorization: Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("nono:{token}"))
        )
    }

    fn exchange(port: u16, request: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        s.write_all(request.as_bytes()).unwrap();
        let mut buf = [0u8; 4096];
        let n = s.read(&mut buf).unwrap_or(0);
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }

    fn temp_log(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "devcroft-proxy-test-{tag}-{}.log",
            std::process::id()
        ));
        std::fs::write(&p, "").unwrap();
        p
    }

    /// Admission stops at the cap and frees a slot when a connection ends.
    #[test]
    fn connections_past_the_cap_are_not_admitted_until_one_ends() {
        let open = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let slots: Vec<Slot> = (0..MAX_CONNECTIONS)
            .map(|_| Slot::take(&open).unwrap())
            .collect();
        assert!(Slot::take(&open).is_none());
        drop(slots.into_iter().next());
        assert!(Slot::take(&open).is_some());
    }

    #[test]
    fn the_unix_bridge_presents_the_token_in_place_of_the_clients() {
        let header = "Proxy-Authorization: Basic dDo=\r\n";
        let head = b"CONNECT api.example:443 HTTP/1.1\r\nHost: api.example:443\r\n\
            proxy-authorization: Basic d3Jvbmc6\r\n\r\nEARLY";
        assert_eq!(
            String::from_utf8(with_proxy_auth(head, header)).unwrap(),
            "CONNECT api.example:443 HTTP/1.1\r\nProxy-Authorization: Basic dDo=\r\n\
             Host: api.example:443\r\n\r\nEARLY"
        );
        assert_eq!(
            String::from_utf8(with_proxy_auth(b"GET http://a/ HTTP/1.1\r\n\r\n", header)).unwrap(),
            "GET http://a/ HTTP/1.1\r\nProxy-Authorization: Basic dDo=\r\n\r\n"
        );
        assert_eq!(with_proxy_auth(b"GET http://a/", header), b"GET http://a/");
    }

    /// The configuration is what devcroft decided, not `nono-proxy`'s
    /// defaults, which differ on two of these (`strict_connect_auth`,
    /// `strict_filter`).
    #[test]
    fn the_config_enforces_auth_and_a_strict_allowlist_with_no_interception() {
        let c = config(vec!["example.com".into()], "t".into(), Vec::new());
        assert!(c.require_auth && c.strict_connect_auth && c.strict_filter);
        assert!(c.intercept_ca_dir.is_none());
        assert!(c.routes.is_empty() && c.oauth_capture.is_empty());
        assert_eq!(c.max_connections, MAX_CONNECTIONS);
        assert!(c.bind_addr.is_loopback());
    }

    /// End to end: an allowlisted host is tunnelled to, a host off the list
    /// is refused naming it, and both decisions are in the log.
    #[test]
    fn end_to_end_connect_allows_and_denies_by_host() {
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        let upstream_thread = thread::spawn(move || {
            let (mut s, _) = upstream.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            s.write_all(b"HELLO-FROM-UPSTREAM").unwrap();
        });
        let log_path = temp_log(&format!("e2e-{upstream_port}"));
        let proxy = start(
            vec!["127.0.0.1".into()],
            log_path.clone(),
            "test-token-e2e".into(),
            None,
            Vec::new(),
        )
        .unwrap();

        let mut client = TcpStream::connect(("127.0.0.1", proxy.port)).unwrap();
        client
            .write_all(
                format!(
                    "CONNECT 127.0.0.1:{upstream_port} HTTP/1.1\r\n{}\r\n\r\n",
                    basic_auth_header("test-token-e2e")
                )
                .as_bytes(),
            )
            .unwrap();
        let mut response = [0u8; 4096];
        let n = client.read(&mut response).unwrap();
        let response = String::from_utf8_lossy(&response[..n]);
        assert!(
            response.starts_with("HTTP/1.1 200"),
            "expected a 200 tunnel, got: {response}"
        );
        client.write_all(b"ping").unwrap();
        let mut tail = [0u8; 4096];
        let n = client.read(&mut tail).unwrap();
        assert_eq!(&tail[..n], b"HELLO-FROM-UPSTREAM");
        upstream_thread.join().unwrap();

        let refused = exchange(
            proxy.port,
            &format!(
                "CONNECT evil.example.com:443 HTTP/1.1\r\n{}\r\n\r\n",
                basic_auth_header("test-token-e2e")
            ),
        );
        assert!(
            refused.starts_with("HTTP/1.1 403"),
            "expected a 403 refusal, got: {refused}"
        );
        assert!(
            refused.contains("evil.example.com"),
            "refusal must name the denied host, got: {refused}"
        );

        std::thread::sleep(std::time::Duration::from_millis(200));
        let log = std::fs::read_to_string(&log_path).unwrap();
        assert!(
            log.contains("allow") && log.contains(&upstream_port.to_string()),
            "{log}"
        );
        assert!(
            log.contains("refuse") && log.contains("evil.example.com"),
            "{log}"
        );
        proxy.shutdown();
        let _ = std::fs::remove_file(&log_path);
    }

    /// The auth boundary: a caller with no credential, or the wrong one, is
    /// refused on an *allowlisted* host, so the refusal is the token's and
    /// not the allowlist's (`add-egress-proxy` 4a). `nono-proxy`'s default
    /// would have tunnelled the first one; `strict_connect_auth` is why it
    /// does not.
    #[test]
    fn an_unauthenticated_caller_is_refused_even_for_an_allowed_host() {
        let log_path = temp_log("auth");
        let proxy = start(
            vec!["example.com".into()],
            log_path.clone(),
            "right-token".into(),
            None,
            Vec::new(),
        )
        .unwrap();
        for request in [
            "CONNECT example.com:443 HTTP/1.1\r\n\r\n".to_owned(),
            format!(
                "CONNECT example.com:443 HTTP/1.1\r\n{}\r\n\r\n",
                basic_auth_header("wrong-token")
            ),
            "GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\n".to_owned(),
        ] {
            let response = exchange(proxy.port, &request);
            assert!(
                response.starts_with("HTTP/1.1 407"),
                "expected 407 for {request:?}, got: {response}"
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        let log = std::fs::read_to_string(&log_path).unwrap();
        assert!(
            !log.contains("allow"),
            "an unauthenticated request must never be allowed: {log}"
        );
        // Logged, but never as a host the allowlist refused, which is how
        // `lifecycle::attribution` would read a `host=` on a `refuse` line.
        assert!(log.contains("reason=unauthenticated"), "{log}");
        assert!(
            log.lines().all(|l| !l.contains(" host=")),
            "an auth failure must not read as an allowlist refusal: {log}"
        );
        proxy.shutdown();
        let _ = std::fs::remove_file(&log_path);
    }

    /// A TLS upstream on loopback, trusting a fresh test CA: returns its
    /// port, the CA's PEM path, and a channel carrying each request head it
    /// received. Answers every request `200 ok`.
    fn tls_upstream(tag: &str) -> (u16, std::path::PathBuf, std::sync::mpsc::Receiver<String>) {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca = ca_params.self_signed(&ca_key).unwrap();
        let ca_issuer = rcgen::Issuer::new(ca_params, ca_key);
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let leaf = rcgen::CertificateParams::new(vec!["localhost".to_string()])
            .unwrap()
            .signed_by(&leaf_key, &ca_issuer)
            .unwrap();
        let ca_path = std::env::temp_dir().join(format!(
            "devcroft-broker-ca-{tag}-{}.pem",
            std::process::id()
        ));
        std::fs::write(&ca_path, ca.pem()).unwrap();

        let config = std::sync::Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![leaf.der().clone()],
                    rustls::pki_types::PrivateKeyDer::try_from(leaf_key.serialize_der()).unwrap(),
                )
                .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let Ok(conn) = rustls::ServerConnection::new(config.clone()) else {
                    return;
                };
                let mut tls = rustls::StreamOwned::new(conn, stream);
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match tls.read(&mut byte) {
                        Ok(1) => head.push(byte[0]),
                        _ => break,
                    }
                }
                let _ = tx.send(String::from_utf8_lossy(&head).into_owned());
                let _ = tls.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                );
                tls.conn.send_close_notify();
                let _ = tls.flush();
            }
        });
        (port, ca_path, rx)
    }

    /// A proxy with one brokered route to a [`tls_upstream`], the value
    /// read from `var` as `egress_proxy_main` arranges it.
    fn brokered_proxy(
        tag: &str,
        upstream_port: u16,
        ca: &std::path::Path,
        placeholder: Option<&str>,
        also_allow: &[&str],
    ) -> (Running, String, PathBuf) {
        let var = format!("DEVCROFT_TEST_BROKERED_{}", tag.to_ascii_uppercase());
        // SAFETY: a name no other test reads or writes.
        unsafe { std::env::set_var(&var, "the-real-secret") };
        let mut route = broker_route(
            &crate::proxy::BrokerRoute {
                prefix: "broker0".into(),
                host: "localhost".into(),
                header: "x-api-key".into(),
                env: "ANTHROPIC_API_KEY".into(),
                placeholder: placeholder.map(str::to_owned),
                tls_ca: Some(ca.display().to_string()),
                value: String::new(),
            },
            &var,
        );
        // `broker_route` reaches `https://<host>` on 443; the test upstream
        // is on a port of its own.
        route.upstream = format!("https://localhost:{upstream_port}");
        let token = format!("token-{tag}");
        let log_path = temp_log(&format!("broker-{tag}"));
        let allow = std::iter::once("localhost")
            .chain(also_allow.iter().copied())
            .map(str::to_owned)
            .collect();
        let proxy = start(allow, log_path.clone(), token.clone(), None, vec![route]).unwrap();
        (proxy, token, log_path)
    }

    fn post(port: u16, key: &str) -> String {
        exchange(
            port,
            &format!(
                "POST /broker0/v1/messages HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
                 x-api-key: {key}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            ),
        )
    }

    /// The substitution itself (4b.3): the sandbox's placeholder goes in,
    /// the upstream receives the real value, and the placeholder never
    /// reaches it. The value never appears in the proxy's log.
    #[test]
    fn a_brokered_request_reaches_the_upstream_with_the_real_value() {
        let (upstream, ca, heads) = tls_upstream("sub");
        let (proxy, token, log_path) = brokered_proxy("sub", upstream, &ca, None, &[]);
        assert_eq!(proxy.loaded_routes(), 1);

        let phantom = crate::proxy::broker_phantom(&token, None).unwrap();
        let response = post(proxy.port, &phantom);
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let head = heads
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the upstream received nothing");
        let lower = head.to_ascii_lowercase();
        assert!(lower.contains("x-api-key: the-real-secret"), "{head}");
        assert!(
            !head.contains(&token),
            "the placeholder reached the upstream: {head}"
        );
        assert!(head.starts_with("POST /v1/messages "), "{head}");

        std::thread::sleep(std::time::Duration::from_millis(200));
        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        assert!(!log.contains("the-real-secret"), "{log}");
        proxy.shutdown();
        let _ = std::fs::remove_file(&log_path);
        let _ = std::fs::remove_file(&ca);
    }

    /// A shaped placeholder (`ghp_{}`-style) is what the route accepts, and
    /// the bare token no longer is; neither wrong form reaches the upstream.
    #[test]
    fn a_wrong_or_missing_placeholder_never_reaches_the_upstream() {
        let (upstream, ca, heads) = tls_upstream("wrong");
        let (proxy, token, log_path) =
            brokered_proxy("wrong", upstream, &ca, Some("sk-ant-{}"), &[]);

        for key in ["not-the-placeholder", token.as_str()] {
            let response = post(proxy.port, key);
            assert!(!response.starts_with("HTTP/1.1 200"), "{key}: {response}");
        }
        assert!(
            heads
                .recv_timeout(std::time::Duration::from_millis(500))
                .is_err(),
            "a request without the placeholder reached the upstream"
        );

        let shaped = crate::proxy::broker_phantom(&token, Some("sk-ant-{}")).unwrap();
        assert!(shaped.starts_with("sk-ant-"), "{shaped}");
        let response = post(proxy.port, &shaped);
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let head = heads
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(
            head.to_ascii_lowercase()
                .contains("x-api-key: the-real-secret"),
            "{head}"
        );
        proxy.shutdown();
        let _ = std::fs::remove_file(&log_path);
        let _ = std::fs::remove_file(&ca);
    }

    /// A plain upstream on loopback: sends each request head it reads, and
    /// answers `200 ok`. Reached through the proxy as another allowed host.
    fn plain_upstream() -> (u16, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { return };
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match s.read(&mut byte) {
                        Ok(1) => head.push(byte[0]),
                        _ => break,
                    }
                }
                let _ = tx.send(String::from_utf8_lossy(&head).into_owned());
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                );
            }
        });
        (port, rx)
    }

    /// 4b.7: the value is the route's alone. The placeholder sent to another
    /// allowed host, as an absolute-URI request or inside a `CONNECT`
    /// tunnel, arrives as the placeholder; and the route's own host is not
    /// reachable around the route, which is what `up` discloses.
    #[test]
    fn a_request_to_another_allowed_host_never_gets_the_value() {
        let (upstream, ca, heads) = tls_upstream("other");
        let (other, other_heads) = plain_upstream();
        let (proxy, token, log_path) = brokered_proxy("other", upstream, &ca, None, &["127.0.0.1"]);
        assert_eq!(proxy.loaded_routes(), 1);
        let phantom = crate::proxy::broker_phantom(&token, None).unwrap();
        let auth = basic_auth_header(&token);
        let recv = |rx: &std::sync::mpsc::Receiver<String>| {
            rx.recv_timeout(std::time::Duration::from_secs(5))
                .expect("the other host received nothing")
        };

        let response = exchange(
            proxy.port,
            &format!(
                "GET http://127.0.0.1:{other}/v1/messages HTTP/1.1\r\nHost: 127.0.0.1:{other}\r\n\
                 {auth}\r\nx-api-key: {phantom}\r\nConnection: close\r\n\r\n"
            ),
        );
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let head = recv(&other_heads);
        assert!(!head.contains("the-real-secret"), "absolute-URI: {head}");
        assert!(head.contains(&phantom), "forwarded unchanged: {head}");

        let mut tunnel = TcpStream::connect(("127.0.0.1", proxy.port)).unwrap();
        tunnel
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        tunnel
            .write_all(format!("CONNECT 127.0.0.1:{other} HTTP/1.1\r\n{auth}\r\n\r\n").as_bytes())
            .unwrap();
        let mut buf = [0u8; 4096];
        let n = tunnel.read(&mut buf).unwrap();
        assert!(
            buf[..n].starts_with(b"HTTP/1.1 200"),
            "{:?}",
            String::from_utf8_lossy(&buf[..n])
        );
        tunnel
            .write_all(
                format!(
                    "GET /v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nx-api-key: {phantom}\r\n\r\n"
                )
                .as_bytes(),
            )
            .unwrap();
        let head = recv(&other_heads);
        assert!(!head.contains("the-real-secret"), "CONNECT: {head}");
        assert!(head.contains(&phantom), "{head}");

        // Around the route, to its own host: refused, and nothing arrives.
        let refused = exchange(
            proxy.port,
            &format!("CONNECT localhost:{upstream} HTTP/1.1\r\n{auth}\r\n\r\n"),
        );
        assert!(!refused.starts_with("HTTP/1.1 200"), "{refused}");
        assert!(
            heads
                .recv_timeout(std::time::Duration::from_millis(500))
                .is_err(),
            "the route's host was reached around the route"
        );

        std::thread::sleep(std::time::Duration::from_millis(200));
        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        assert!(!log.contains("the-real-secret"), "{log}");
        proxy.shutdown();
        let _ = std::fs::remove_file(&log_path);
        let _ = std::fs::remove_file(&ca);
    }

    /// A fleet agent's proxy labels every line with whose egress it is.
    #[test]
    fn a_labelled_proxy_prefixes_every_line() {
        let log_path = temp_log("label");
        let proxy = start(
            vec!["example.com".into()],
            log_path.clone(),
            "t".into(),
            Some("agent-7".into()),
            Vec::new(),
        )
        .unwrap();
        let _ = exchange(
            proxy.port,
            &format!(
                "CONNECT denied.example:443 HTTP/1.1\r\n{}\r\n\r\n",
                basic_auth_header("t")
            ),
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
        let log = std::fs::read_to_string(&log_path).unwrap();
        assert!(!log.is_empty(), "the refusal was not logged");
        assert!(
            log.lines().all(|l| l.starts_with("agent=agent-7 ")),
            "{log}"
        );
        proxy.shutdown();
        let _ = std::fs::remove_file(&log_path);
    }
}
