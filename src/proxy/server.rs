//! The proxy's actual protocol handling: accept a connection, read one
//! HTTP request line (`CONNECT host:port` for HTTPS, or an absolute-URI
//! request for plain HTTP), decide via `nono::HostFilter`, and either
//! refuse with a legible message (design.md E5) or dial out and splice
//! bytes bidirectionally. No TLS interception — a `CONNECT` tunnel is
//! opaque bytes after the `200`, per this change's non-goals.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use base64::Engine;
use nono::HostFilter;

/// Bytes read for the request line plus headers before giving up — a
/// hostile or confused client gets a fast, bounded failure rather than
/// unbounded memory growth. Ordinary requests are a few hundred bytes;
/// this is generous headroom, not a real limit anyone should hit.
const MAX_HEADER_BYTES: usize = 64 * 1024;

/// Runs the accept loop until `listener` is closed. One thread per
/// connection — this is a low-throughput control-plane proxy, not a data
/// path optimized for concurrency, and it matches `keeper::connection`'s
/// own thread-per-session style rather than introducing async machinery
/// for its own sake.
///
/// `token` is the per-session credential every request must present via
/// `Proxy-Authorization` (`add-egress-proxy`'s authentication
/// requirement). Binding to loopback is reachability, not authorisation:
/// every process on the host can reach it, so a proxy that accepted any
/// local caller would lend this sandbox's allowlisted egress to anything
/// local, including another sandbox's unsandboxed operator tooling.
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
/// **Why a hairpin rather than making the whole request path generic over
/// the stream type.** The decision logic — auth before the allowlist, the
/// `407`/`502` split, `HostFilter`, the splice — is written against
/// `TcpStream` and is the security-critical part. Forwarding UDS bytes
/// into the proxy's own TCP port reuses every line of it unchanged, at
/// the cost of one extra loopback hop inside a process that is already on
/// the host and unrestricted. Generalising over a `ClientStream` trait
/// would touch `handle_connection`, `splice`, `write_refusal` and
/// `write_auth_required` to save that hop — a poor trade against the risk
/// of subtly changing who gets authenticated.
///
/// This is what makes a network-isolated sandbox able to use the proxy at
/// all: it has no route to the host's loopback, but a pathname unix
/// socket crosses the namespace boundary.
pub fn bridge_unix_to_tcp(unix: std::os::unix::net::UnixListener, tcp_port: u16) {
    for stream in unix.incoming() {
        let Ok(client) = stream else { continue };
        thread::spawn(move || {
            let Ok(upstream) = TcpStream::connect(("127.0.0.1", tcp_port)) else {
                return;
            };
            splice_unix(client, upstream);
        });
    }
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

pub fn run(listener: TcpListener, allow: Vec<String>, log_path: PathBuf, token: String) {
    // `new_strict`: an empty allowlist denies rather than allows. `spawn`
    // is only ever called when `CompiledPolicy::wants_egress_proxy()` is
    // true, which already implies a non-empty `network.allow`, but a
    // proxy that fails open on an empty list would be a silent footgun
    // for any future caller that forgets to check first.
    let filter = Arc::new(HostFilter::new_strict(&allow));
    let token = Arc::new(token);
    let log = Arc::new(Mutex::new(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&log_path)
            .expect("proxy log path was created by spawn() just before this process started"),
    ));

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let filter = Arc::clone(&filter);
        let token = Arc::clone(&token);
        let log = Arc::clone(&log);
        thread::spawn(move || {
            let _ = handle_connection(stream, &filter, &token, &log);
        });
    }
}

/// One decoded request line: which HTTP verb led to a dial-out target and
/// how much of the initial read is "headers" versus body/tunnel bytes to
/// forward verbatim.
#[derive(Debug)]
struct Target {
    is_connect: bool,
    host: String,
    port: u16,
    /// The exact bytes read before the target was known (request line +
    /// headers, including the trailing blank line) — forwarded verbatim
    /// to the upstream for a plain HTTP request; unused for `CONNECT`,
    /// which sends its own `200` instead of relaying the request line.
    header_bytes: Vec<u8>,
    /// The raw `Proxy-Authorization` header value, if the client sent
    /// one — checked against this process's token before any allowlist
    /// decision. `None` when absent, which is refused exactly like a
    /// present-but-wrong value; there is no unauthenticated path.
    proxy_auth: Option<String>,
}

fn handle_connection(
    mut client: TcpStream,
    filter: &HostFilter,
    token: &str,
    log: &Mutex<std::fs::File>,
) -> io::Result<()> {
    let peer = client
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    let target = match read_target(&mut client) {
        Ok(Some(t)) => t,
        Ok(None) => return Ok(()), // client closed before sending a full request
        Err(e) => {
            log_line(
                log,
                &format!("refuse peer={peer} reason=malformed-request detail={e}"),
            );
            let _ = write_refusal(&mut client, true, "malformed request");
            return Ok(());
        }
    };

    // Checked before the allowlist decision, deliberately: the question
    // "whose allowlist applies" has no answer for an unauthenticated
    // caller, so there is nothing yet to decide by hostname (design.md
    // E6, the "authenticated, not merely local" requirement).
    if !authorized(target.proxy_auth.as_deref(), token) {
        log_line(log, &format!("refuse peer={peer} reason=unauthenticated"));
        let _ = write_auth_required(&mut client, target.is_connect);
        return Ok(());
    }

    // Resolved *before* the filter decision, but a resolution failure
    // does not short-circuit into its own refusal here: `check_host` only
    // needs `resolved_ips` for its link-local step, so an empty list
    // (from a lookup failure) still lets it decide correctly by name
    // alone. Deciding on the name first — rather than refusing early on
    // DNS failure — means a host that was never going to be allowed
    // reads as "not in network.allow", not as "could not resolve", and
    // never gets a DNS query issued on its behalf in the first place is
    // not achievable here (the lookup already ran), but at least its
    // *failure* doesn't leak a different-shaped refusal than a denied
    // host that resolves fine would get.
    let resolved: Vec<IpAddr> = (target.host.as_str(), target.port)
        .to_socket_addrs()
        .map(|addrs| addrs.map(|a| a.ip()).collect())
        .unwrap_or_default();

    let decision = filter.check_host(&target.host, &resolved);
    if !decision.is_allowed() {
        // `FilterResult::reason()` already sanitizes control characters
        // out of the (untrusted, client-supplied) hostname before this
        // is printed anywhere — see its own doc comment.
        let reason = decision.reason();
        log_line(
            log,
            &format!(
                "refuse peer={peer} host={} port={} reason={reason}",
                target.host, target.port
            ),
        );
        let _ = write_refusal(
            &mut client,
            target.is_connect,
            &format!("devcroft: egress denied for {}: {reason}", target.host),
        );
        return Ok(());
    }

    // Dial the *same* resolved addresses just checked, rather than
    // resolving again — resolving twice would let a second lookup return
    // a different (unfiltered) address between the decision and the
    // connection, the same rebind concern `HostFilter`'s own link-local
    // check exists to close one layer down.
    let mut upstream = None;
    for ip in &resolved {
        if let Ok(s) = TcpStream::connect((*ip, target.port)) {
            upstream = Some(s);
            break;
        }
    }
    let Some(mut upstream) = upstream else {
        log_line(
            log,
            &format!(
                "refuse peer={peer} host={} port={} reason=connect-failed",
                target.host, target.port
            ),
        );
        let _ = write_refusal(
            &mut client,
            target.is_connect,
            &format!("could not connect to {}", target.host),
        );
        return Ok(());
    };

    log_line(
        log,
        &format!(
            "allow peer={peer} host={} port={}",
            target.host, target.port
        ),
    );

    if target.is_connect {
        client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    } else {
        upstream.write_all(&target.header_bytes)?;
    }
    splice(client, upstream);
    Ok(())
}

/// Reads up to the blank line ending the request headers and parses
/// either `CONNECT host:port HTTP/x.y` or an absolute-URI request line
/// (`METHOD http://host[:port]/path HTTP/x.y`). Origin-form requests
/// (`GET /path HTTP/1.1` with a bare `Host:` header) are also accepted,
/// since that is what most HTTP libraries actually send even when
/// configured with a proxy — only `CONNECT`'s target is unambiguous by
/// construction; absolute-URI is optional in the RFC even through a
/// proxy, so refusing origin-form here would break real clients for a
/// protocol purity gain nobody asked for.
///
/// Returns `Ok(None)` on a clean EOF before any bytes arrived (nothing to
/// refuse — the peer just went away), `Err` for anything malformed.
fn read_target(client: &mut TcpStream) -> io::Result<Option<Target>> {
    let mut reader = BufReader::new(client.try_clone()?);
    // Cumulative across every line, on top of `read_capped_line`'s own
    // per-line `Take` bound — the per-line bound alone caps how much an
    // unterminated line can buffer before erroring, but says nothing
    // about a client sending unbounded *many* short, valid lines.
    let mut budget = MAX_HEADER_BYTES;

    let mut request_line = String::new();
    let n = read_capped_line(&mut reader, &mut request_line, &mut budget)?;
    if n == 0 {
        return Ok(None);
    }
    let mut parts = request_line.trim_end().splitn(3, ' ');
    let method = parts.next().unwrap_or_default();
    let target_str = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request target"))?;

    let mut header_bytes = request_line.clone().into_bytes();
    let mut host_header: Option<String> = None;
    let mut proxy_auth: Option<String> = None;
    loop {
        let mut line = String::new();
        let n = read_capped_line(&mut reader, &mut line, &mut budget)?;
        header_bytes.extend_from_slice(line.as_bytes());
        if n == 0 || line.trim_end() == "" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim();
            if name.eq_ignore_ascii_case("host") {
                host_header = Some(value.trim().to_string());
            } else if name.eq_ignore_ascii_case("proxy-authorization") {
                proxy_auth = Some(value.trim().to_string());
            }
        }
    }

    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = split_host_port(target_str, 443)?;
        return Ok(Some(Target {
            is_connect: true,
            host,
            port,
            header_bytes,
            proxy_auth,
        }));
    }

    let authority = if let Some(rest) = target_str
        .strip_prefix("http://")
        .or_else(|| target_str.strip_prefix("https://"))
    {
        rest.split(['/', '?']).next().unwrap_or(rest).to_string()
    } else if let Some(host_header) = host_header {
        host_header
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "origin-form request with no Host header",
        ));
    };
    let (host, port) = split_host_port(&authority, 80)?;
    Ok(Some(Target {
        is_connect: false,
        host,
        port,
        header_bytes,
        proxy_auth,
    }))
}

/// Checks a `Proxy-Authorization` header against this process's token.
///
/// Only the `Basic` scheme is accepted, because it is the one every
/// standard HTTP client already generates unprompted from userinfo in a
/// proxy URL (`http://<token>@127.0.0.1:<port>`) — the whole point of
/// this shape is that `curl`, `git`, `npm`, and friends need no proxy-auth
/// support devcroft invented, only support they already have. The
/// password half is ignored: the token is the username, and there is
/// nothing else to check once it matches.
fn authorized(header: Option<&str>, token: &str) -> bool {
    let Some(header) = header else { return false };
    let Some(encoded) = header.strip_prefix("Basic ") else {
        return false;
    };
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded.trim()) else {
        return false;
    };
    let Ok(decoded) = String::from_utf8(decoded) else {
        return false;
    };
    let presented = decoded.split(':').next().unwrap_or(&decoded);
    constant_time_eq(presented.as_bytes(), token.as_bytes())
}

/// Equal-time comparison so a failed match does not leak how many
/// leading bytes were right through response timing — cheap to get right
/// here and the kind of detail nobody notices is missing until it is
/// exploited.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// `"host:port"` or bare `"host"` (defaulting to `default_port`) —
/// bracketed IPv6 (`"[::1]:8080"`) included, since Host headers and
/// CONNECT targets both permit it.
fn split_host_port(authority: &str, default_port: u16) -> io::Result<(String, u16)> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "invalid host:port");
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, rest) = rest.split_once(']').ok_or_else(bad)?;
        let port = match rest.strip_prefix(':') {
            Some(p) => p.parse().map_err(|_| bad())?,
            None => default_port,
        };
        return Ok((host.to_string(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => {
            Ok((host.to_string(), port.parse().map_err(|_| bad())?))
        }
        _ => Ok((authority.to_string(), default_port)),
    }
}

fn read_capped_line(
    reader: &mut impl BufRead,
    out: &mut String,
    budget: &mut usize,
) -> io::Result<usize> {
    if *budget == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "request headers exceed the size limit",
        ));
    }
    let n = reader
        .take(*budget as u64)
        .read_line(out)
        .map_err(|e| io::Error::new(e.kind(), format!("reading request: {e}")))?;
    if n == *budget && !out.ends_with('\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "request line exceeds the header size limit",
        ));
    }
    *budget -= n;
    Ok(n)
}

/// `407`, not `502` — an allowlist refusal and a missing/wrong
/// credential are different facts, and collapsing them would make a
/// misconfigured (or absent) proxy token indistinguishable from a
/// correctly-denied host, which is exactly the confusion `write_refusal`
/// exists to avoid one layer up. `Proxy-Authenticate` is included for
/// clients that surface it, though devcroft's own env wiring never
/// depends on the client prompting — it already sends the credential
/// unprompted via userinfo.
fn write_auth_required(client: &mut TcpStream, is_connect: bool) -> io::Result<()> {
    let detail = "devcroft: proxy authentication required";
    if is_connect {
        client.write_all(
            format!(
                "HTTP/1.1 407 Proxy Authentication Required\r\n\
                 Proxy-Authenticate: Basic realm=\"devcroft\"\r\n\
                 Connection: close\r\n\r\n{detail}\r\n"
            )
            .as_bytes(),
        )
    } else {
        client.write_all(
            format!(
                "HTTP/1.1 407 Proxy Authentication Required\r\n\
                 Proxy-Authenticate: Basic realm=\"devcroft\"\r\n\
                 Content-Type: text/plain\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{detail}",
                detail.len()
            )
            .as_bytes(),
        )
    }
}

fn write_refusal(client: &mut TcpStream, is_connect: bool, detail: &str) -> io::Result<()> {
    if is_connect {
        // No sensible body for a `CONNECT` refusal — the client is about
        // to speak TLS on this stream, not read an HTTP body from it.
        // The status line and the log record are where the reason lives.
        client.write_all(
            format!("HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n{detail}\r\n").as_bytes(),
        )
    } else {
        client.write_all(
            format!(
                "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{detail}",
                detail.len()
            )
            .as_bytes(),
        )
    }
}

/// Copies bytes in both directions until either side closes, then shuts
/// the other down so its own copy thread unblocks — a half-open splice
/// would otherwise leak a thread per connection for the lifetime of the
/// proxy.
fn splice(client: TcpStream, upstream: TcpStream) {
    let (client_r, upstream_r) = match (client.try_clone(), upstream.try_clone()) {
        (Ok(c), Ok(u)) => (c, u),
        _ => return,
    };

    // client -> upstream, on its own thread.
    let to_upstream = {
        let mut r = client_r;
        let mut w = upstream;
        thread::spawn(move || {
            let _ = io::copy(&mut r, &mut w);
            let _ = w.shutdown(std::net::Shutdown::Write);
        })
    };
    // upstream -> client, on this thread. Once either direction's source
    // hits EOF/error, shutting down the *other* stream's write half lets
    // that stream's own peer observe EOF too, rather than hanging until
    // some other timeout closes it.
    let mut r = upstream_r;
    let mut w = client;
    let _ = io::copy(&mut r, &mut w);
    let _ = w.shutdown(std::net::Shutdown::Write);
    let _ = to_upstream.join();
}

fn log_line(log: &Mutex<std::fs::File>, line: &str) {
    // One write per record, appended to a file whose fd is `O_APPEND` —
    // the same discipline `keeper::connection::log_record` and
    // `hooks::run_one` established this session, for the same reason:
    // multiple threads (here) or processes (there) share the file, and a
    // multi-write record can interleave with another writer's.
    if let Ok(mut f) = log.lock() {
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener as StdListener;

    /// The `Proxy-Authorization` *value* a real HTTP client would send
    /// from `http://<token>@host:port` userinfo — `Basic` over
    /// `<token>:`, matching what `authorized()` expects.
    fn basic_auth_value(token: &str) -> String {
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{token}:"))
        )
    }

    /// The full wire-format header line, for tests that write raw bytes
    /// to a socket.
    fn basic_auth_header(token: &str) -> String {
        format!("Proxy-Authorization: {}", basic_auth_value(token))
    }

    #[test]
    fn authorized_accepts_the_matching_token_via_userinfo_shape() {
        assert!(authorized(Some(&basic_auth_value("secret")), "secret"));
    }

    #[test]
    fn authorized_rejects_a_wrong_token() {
        assert!(!authorized(Some(&basic_auth_value("wrong")), "secret"));
    }

    #[test]
    fn authorized_rejects_a_missing_header() {
        assert!(!authorized(None, "secret"));
    }

    #[test]
    fn authorized_rejects_malformed_base64() {
        assert!(!authorized(Some("Basic not-valid-base64!!!"), "secret"));
    }

    #[test]
    fn authorized_rejects_non_basic_scheme() {
        assert!(!authorized(Some("Bearer secret"), "secret"));
    }

    fn write_and_read_target(request: &'static [u8]) -> io::Result<Option<Target>> {
        let listener = StdListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let writer = thread::spawn(move || {
            let mut s = TcpStream::connect(addr).unwrap();
            s.write_all(request).unwrap();
            // Keep the connection open until the reader is done, then
            // drop — half-closing the write side isn't necessary here
            // since `read_target` only ever needs the header terminator.
            let mut buf = [0u8; 1];
            let _ = s.read(&mut buf);
        });
        let (mut server_side, _) = listener.accept().unwrap();
        let result = read_target(&mut server_side);
        drop(server_side);
        let _ = writer.join();
        result
    }

    #[test]
    fn connect_target_parses_host_and_port() {
        let t = write_and_read_target(b"CONNECT example.com:443 HTTP/1.1\r\n\r\n")
            .unwrap()
            .unwrap();
        assert!(t.is_connect);
        assert_eq!(t.host, "example.com");
        assert_eq!(t.port, 443);
    }

    #[test]
    fn absolute_uri_target_parses_host_default_port_80() {
        let t = write_and_read_target(
            b"GET http://example.com/foo HTTP/1.1\r\nHost: example.com\r\n\r\n",
        )
        .unwrap()
        .unwrap();
        assert!(!t.is_connect);
        assert_eq!(t.host, "example.com");
        assert_eq!(t.port, 80);
    }

    #[test]
    fn origin_form_falls_back_to_host_header() {
        let t = write_and_read_target(b"GET /foo HTTP/1.1\r\nHost: example.com:8080\r\n\r\n")
            .unwrap()
            .unwrap();
        assert!(!t.is_connect);
        assert_eq!(t.host, "example.com");
        assert_eq!(t.port, 8080);
    }

    #[test]
    fn origin_form_without_host_header_is_rejected() {
        let err = write_and_read_target(b"GET /foo HTTP/1.1\r\n\r\n").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn bracketed_ipv6_host_is_parsed() {
        let (host, port) = split_host_port("[::1]:9000", 443).unwrap();
        assert_eq!(host, "::1");
        assert_eq!(port, 9000);
    }

    #[test]
    fn clean_eof_before_any_bytes_is_not_an_error() {
        let listener = StdListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let writer = thread::spawn(move || {
            let _ = TcpStream::connect(addr).unwrap(); // connect, then drop immediately
        });
        let (mut server_side, _) = listener.accept().unwrap();
        let result = read_target(&mut server_side).unwrap();
        assert!(result.is_none());
        let _ = writer.join();
    }

    /// Full accept-loop, not just the request-line parser: a real
    /// `CONNECT` to an allowlisted mock upstream gets tunneled and its
    /// response comes back through the proxy byte-for-byte; a `CONNECT`
    /// to a host not on the list is refused with a `502` naming the host.
    /// This is what turns "the parser is right" into "the proxy actually
    /// works", per this change's own validation task (5.1/5.4).
    #[test]
    fn end_to_end_connect_allows_and_denies_by_host() {
        // The mock upstream: accepts one connection, echoes one fixed
        // response, then goes away.
        let upstream = StdListener::bind("127.0.0.1:0").unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        let upstream_thread = thread::spawn(move || {
            let (mut s, _) = upstream.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf); // drain whatever the tunnel forwards
            s.write_all(b"HELLO-FROM-UPSTREAM").unwrap();
        });

        let log_path = std::env::temp_dir().join(format!(
            "devcroft-proxy-test-{}-{}.log",
            std::process::id(),
            upstream_port
        ));
        std::fs::write(&log_path, "").unwrap();

        let proxy_listener = StdListener::bind("127.0.0.1:0").unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        let allow = vec!["127.0.0.1".to_string()];
        let log_path_for_run = log_path.clone();
        let token = "test-token-e2e".to_string();
        thread::spawn(move || run(proxy_listener, allow, log_path_for_run, token));

        // Allowed: CONNECT to the mock upstream's own address, which is
        // on the allowlist.
        let mut client = TcpStream::connect(proxy_addr).unwrap();
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
        client.write_all(b"ping").unwrap(); // forwarded to upstream, drained above
        let mut tail = [0u8; 4096];
        let n = client.read(&mut tail).unwrap();
        assert_eq!(&tail[..n], b"HELLO-FROM-UPSTREAM");
        upstream_thread.join().unwrap();

        // Denied: a host nowhere near the allowlist. Still authenticated —
        // this test is about the allowlist decision, not auth.
        let mut client2 = TcpStream::connect(proxy_addr).unwrap();
        client2
            .write_all(
                format!(
                    "CONNECT evil.example.com:443 HTTP/1.1\r\n{}\r\n\r\n",
                    basic_auth_header("test-token-e2e")
                )
                .as_bytes(),
            )
            .unwrap();
        let mut response2 = [0u8; 4096];
        let n2 = client2.read(&mut response2).unwrap();
        let response2 = String::from_utf8_lossy(&response2[..n2]);
        assert!(
            response2.starts_with("HTTP/1.1 502"),
            "expected a 502 refusal, got: {response2}"
        );
        assert!(
            response2.contains("evil.example.com"),
            "refusal must name the denied host, got: {response2}"
        );

        let log = std::fs::read_to_string(&log_path).unwrap();
        assert!(log.contains("allow") && log.contains(&upstream_port.to_string()));
        assert!(log.contains("refuse") && log.contains("evil.example.com"));
        let _ = std::fs::remove_file(&log_path);
    }

    /// The auth boundary itself, end to end: a caller with no credential
    /// — the shape of "any other local process on the host" — is refused
    /// on an *allowlisted* host, which is exactly the scenario that
    /// distinguishes an authenticated proxy from one that merely happens
    /// to bind loopback (`add-egress-proxy`'s corrected task: "two
    /// sandboxes with different allowlists... this holds because each
    /// proxy authenticates its own client, not merely because the two
    /// run as separate processes").
    #[test]
    fn an_unauthenticated_caller_is_refused_even_for_an_allowed_host() {
        let log_path = std::env::temp_dir().join(format!(
            "devcroft-proxy-test-auth-{}.log",
            std::process::id()
        ));
        std::fs::write(&log_path, "").unwrap();

        let proxy_listener = StdListener::bind("127.0.0.1:0").unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        let allow = vec!["example.com".to_string()];
        let log_path_for_run = log_path.clone();
        thread::spawn(move || {
            run(
                proxy_listener,
                allow,
                log_path_for_run,
                "right-token".into(),
            )
        });

        // No Proxy-Authorization header at all, against a host that
        // *would* be allowed if this client were authenticated — proving
        // the refusal comes from auth, not from the allowlist.
        let mut unauthed = TcpStream::connect(proxy_addr).unwrap();
        unauthed
            .write_all(b"CONNECT example.com:443 HTTP/1.1\r\n\r\n")
            .unwrap();
        let mut response = [0u8; 4096];
        let n = unauthed.read(&mut response).unwrap();
        let response = String::from_utf8_lossy(&response[..n]);
        assert!(
            response.starts_with("HTTP/1.1 407"),
            "expected 407 Proxy Authentication Required, got: {response}"
        );

        // Wrong token, same allowed host: still refused.
        let mut wrong = TcpStream::connect(proxy_addr).unwrap();
        wrong
            .write_all(
                format!(
                    "CONNECT example.com:443 HTTP/1.1\r\n{}\r\n\r\n",
                    basic_auth_header("wrong-token")
                )
                .as_bytes(),
            )
            .unwrap();
        let mut response2 = [0u8; 4096];
        let n2 = wrong.read(&mut response2).unwrap();
        let response2 = String::from_utf8_lossy(&response2[..n2]);
        assert!(
            response2.starts_with("HTTP/1.1 407"),
            "expected 407 for a wrong token, got: {response2}"
        );

        let log = std::fs::read_to_string(&log_path).unwrap();
        assert_eq!(
            log.matches("reason=unauthenticated").count(),
            2,
            "both refusals should log the auth reason, not an allowlist one: {log}"
        );
        assert!(
            !log.contains("example.com HTTP") && !log.contains("host=example.com"),
            "an unauthenticated request must never reach the allowlist decision: {log}"
        );
        let _ = std::fs::remove_file(&log_path);
    }
}
