//! Brokered credentials end to end with a real agent (`add-agent-workload`
//! 4b.8): Claude Code, inside a real sandbox, with a dummy key brokered for
//! `api.anthropic.com`, against a local TLS upstream that answers enough of
//! the Messages API for `claude -p` to finish.
//!
//! The route reaches the upstream through `DEVCROFT_TEST_BROKER_UPSTREAM`,
//! which exists in `test-support` builds only (`proxy::server::broker_route`):
//! a route otherwise always targets `https://<host>` on 443.
//!
//! Both paths run where the host allows: network-isolated, then not (the
//! test-only `DEVCROFT_TEST_NO_NETNS` answers as a host without namespaces).
//! The isolated path reaches the endpoint through the
//! relay and the proxy's unix bridge, the other straight over loopback.

#![cfg(feature = "test-support")]

use devcroft::config::{BrokerDecl, parse};
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down};
use devcroft::provider::{ProviderEntry, ProviderError, Resolution, ServiceSupport, Tier};
use devcroft::test_support::up_with_provider;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const SECRET: &str = "sk-ant-api03-real-secret-never-inside-4b8";
const REPLY: &str = "BROKERED-OK-4b8";

fn store_bin(marker: &str, binary: &str) -> Option<PathBuf> {
    std::fs::read_dir("/nix/store")
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().contains(marker))
                && p.join("bin").join(binary).is_file()
        })
        .map(|p| p.join("bin"))
}

struct Row {
    path: String,
    ca: PathBuf,
}

impl ProviderEntry for Row {
    fn resolve(&self, _: &Path) -> Result<Resolution, ProviderError> {
        Ok(Resolution {
            env: BTreeMap::from([
                ("PATH".to_owned(), self.path.clone()),
                // The trust store the environment names, which is the one
                // the route verifies its upstream against (`tls_ca`).
                ("SSL_CERT_FILE".to_owned(), self.ca.display().to_string()),
            ]),
            unset: Vec::new(),
            read_only_grants: vec!["/nix/store".to_owned()],
            activation_script: None,
            services: ServiceSupport::Unsupported,
            ran_activation_hook: false,
        })
    }
    fn fingerprint(&self, _: &Path) -> Result<String, ProviderError> {
        Ok("broker-claude-row".to_owned())
    }
    fn static_name(&self) -> &'static str {
        "nix"
    }
    fn tier(&self) -> Tier {
        Tier::Closure
    }
}

/// One request as the upstream received it.
struct Seen {
    head: String,
    body: String,
}

fn read_request(s: &mut impl Read) -> Option<Seen> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match s.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return None,
        }
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let lower = head.to_ascii_lowercase();
    let mut body = Vec::new();
    if let Some(len) = lower
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
    {
        body.resize(len, 0);
        s.read_exact(&mut body).ok()?;
    } else if lower.contains("transfer-encoding: chunked") {
        while !body.ends_with(b"0\r\n\r\n") {
            match s.read(&mut byte) {
                Ok(1) => body.push(byte[0]),
                _ => break,
            }
        }
    }
    Some(Seen {
        head,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn sse() -> String {
    let events = [
        (
            "message_start",
            r#"{"type":"message_start","message":{"id":"msg_4b8","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}}"#.to_owned(),
        ),
        (
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#.to_owned(),
        ),
        (
            "content_block_delta",
            format!(
                r#"{{"type":"content_block_delta","index":0,"delta":{{"type":"text_delta","text":"{REPLY}"}}}}"#
            ),
        ),
        (
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#.to_owned(),
        ),
        (
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":1}}"#.to_owned(),
        ),
        ("message_stop", r#"{"type":"message_stop"}"#.to_owned()),
    ];
    events
        .iter()
        .map(|(e, d)| format!("event: {e}\ndata: {d}\n\n"))
        .collect()
}

fn respond(seen: &Seen) -> String {
    let path = seen.head.split_whitespace().nth(1).unwrap_or("");
    let (kind, body) = if path.starts_with("/v1/messages/count_tokens") {
        ("application/json", r#"{"input_tokens":1}"#.to_owned())
    } else if path.starts_with("/v1/messages") && seen.body.contains("\"stream\":true") {
        ("text/event-stream", sse())
    } else if path.starts_with("/v1/messages") {
        (
            "application/json",
            format!(
                r#"{{"id":"msg_4b8","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{{"type":"text","text":"{REPLY}"}}],"stop_reason":"end_turn","stop_sequence":null,"usage":{{"input_tokens":1,"output_tokens":1}}}}"#
            ),
        )
    } else {
        ("application/json", "{}".to_owned())
    };
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )
}

/// A TLS upstream on loopback for `localhost`, under a fresh CA written to
/// `ca_path`; every request head it receives is sent on the channel.
fn tls_upstream(ca_path: &Path) -> (u16, mpsc::Receiver<String>) {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::new(ca_params, ca_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf = rcgen::CertificateParams::new(vec!["localhost".to_string()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();
    std::fs::write(ca_path, ca.pem()).unwrap();
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
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let (config, tx) = (config.clone(), tx.clone());
            std::thread::spawn(move || {
                let Ok(conn) = rustls::ServerConnection::new(config) else {
                    return;
                };
                let mut tls = rustls::StreamOwned::new(conn, stream);
                let Some(seen) = read_request(&mut tls) else {
                    return;
                };
                let _ = tls.write_all(respond(&seen).as_bytes());
                tls.conn.send_close_notify();
                let _ = tls.flush();
                let _ = tx.send(seen.head);
            });
        }
    });
    (port, rx)
}

#[test]
fn claude_code_reaches_its_api_with_the_real_key_and_never_holds_it() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    if !devcroft::provider::host_can_build_nix_closures() {
        eprintln!("skipping: no usable Nix store on this host");
        return;
    }
    let Some(claude) = store_bin("-claude-code-", "claude") else {
        eprintln!("skipping: no Claude Code in /nix/store (nixpkgs#claude-code, unfree)");
        return;
    };
    let Some(bash) = store_bin("-bash-", "bash") else {
        eprintln!("skipping: no bash in /nix/store");
        return;
    };
    let exe = Path::new(env!("CARGO_BIN_EXE_devcroft"));
    // Probed before the lever below is ever set. Sequential, not two tests:
    // every variable `up` reads here is process-wide.
    if matches!(devcroft::fleet::netns::probe(exe), Ok(true)) {
        run_once(exe, &claude, &bash, true);
    } else {
        eprintln!("broker_claude: no network namespaces here, so only the non-isolated path ran");
    }
    // SAFETY: no other test in this binary.
    unsafe { std::env::set_var("DEVCROFT_TEST_NO_NETNS", "1") };
    run_once(exe, &claude, &bash, false);
    unsafe { std::env::remove_var("DEVCROFT_TEST_NO_NETNS") };
}

/// One `up`, one `claude -p`, one `down`. `isolated` is what the sandbox is
/// expected to be, checked from inside it rather than assumed.
fn run_once(exe: &Path, claude: &Path, bash: &Path, isolated: bool) {
    let mode = if isolated { "isolated" } else { "not isolated" };
    eprintln!("broker_claude: running {mode}");
    let pid = std::process::id();
    let tag = if isolated { "i" } else { "n" };
    let project = PathBuf::from(format!("/tmp/dcbrokerclaude{pid}{tag}"));
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();
    let project = project.canonicalize().unwrap();
    let ca = project
        .join("..")
        .join(format!("dcbrokerclaude{pid}{tag}-ca.pem"));
    let (upstream, heads) = tls_upstream(&ca);
    let ca = ca.canonicalize().unwrap();

    // SAFETY: names no other test in this binary reads or writes.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", exe);
        std::env::set_var("ANTHROPIC_API_KEY", SECRET);
        std::env::set_var(
            "DEVCROFT_TEST_BROKER_UPSTREAM",
            format!("https://localhost:{upstream}"),
        );
    }

    let name = format!("e2ebrokerclaude{pid}{tag}");
    let text = format!(
        "[sandbox]\nname = {name:?}\n\n[network]\ndefault = \"deny\"\n\
         allow = [\"api.anthropic.com\"]\n\n[env.vars]\n\
         CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC = \"1\"\nDISABLE_AUTOUPDATER = \"1\"\n"
    );
    std::fs::write(project.join("devcroft.toml"), &text).unwrap();
    let (manifest, _) = parse(&text).unwrap();
    let paths = StatePaths::new(&name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_file(&paths.lifecycle_lock);

    // A shaped placeholder, as a client that checks a key's form would need.
    let brokers = [BrokerDecl {
        host: "api.anthropic.com".into(),
        header: "x-api-key".into(),
        env: "ANTHROPIC_API_KEY".into(),
        endpoint_env: "ANTHROPIC_BASE_URL".into(),
        placeholder: Some("sk-ant-api03-{}".into()),
    }];
    let opts = UpOptions {
        brokers: &brokers,
        ..UpOptions::default()
    };
    let row = Row {
        path: format!("{}:{}", claude.display(), bash.display()),
        ca: ca.clone(),
    };
    let outcome = up_with_provider(&manifest, &project, &opts, &row);

    let session_env = Command::new(exe)
        .current_dir(&project)
        .args(["exec", &name, "--", "bash", "-c", "export -p"])
        .stdin(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();

    let mut child = Command::new(exe)
        .current_dir(&project)
        .args([
            "exec",
            &name,
            "--",
            "claude",
            "-p",
            "Reply with the word you are given.",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let mut out = String::new();
    let mut err = String::new();
    let _ = child.stdout.take().unwrap().read_to_string(&mut out);
    let _ = child.stderr.take().unwrap().read_to_string(&mut err);

    let received: Vec<String> = heads.try_iter().collect();
    let meta = devcroft::lifecycle::read_meta(&paths.meta).ok().flatten();
    let proxy_log = std::fs::read_to_string(&paths.proxy_log).unwrap_or_default();
    let log = std::fs::read_to_string(&paths.log).unwrap_or_default();
    let _ = down(&name);
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_file(&paths.lifecycle_lock);
    let _ = std::fs::remove_dir_all(&project);
    let _ = std::fs::remove_file(&ca);

    assert_eq!(
        outcome.unwrap_or_else(|e| panic!("up: {e}\nlog:\n{log}")),
        UpOutcome::Started
    );
    let token = meta
        .and_then(|m| m.proxy_token)
        .expect("a proxy token in meta.json");
    let status = status.unwrap_or_else(|| panic!("claude timed out\nstdout:{out}\nstderr:{err}"));
    let context = format!(
        "{mode}: status: {status}\nstdout:\n{out}\nstderr:\n{err}\nupstream saw {} request(s):\n{}\n\
         proxy log:\n{proxy_log}",
        received.len(),
        received.join("\n---\n")
    );
    // The isolated sandbox reaches the proxy through the unix bridge, which
    // presents the token itself; only the loopback path carries it as
    // userinfo (`up`, `proxy::client_env`).
    let proxy_url = session_env
        .lines()
        .find_map(|l| l.strip_prefix("declare -x HTTPS_PROXY="))
        .map(|v| v.trim_matches('"'))
        .unwrap_or_else(|| panic!("{mode}: no HTTPS_PROXY in the session:\n{session_env}"));
    assert_eq!(
        !proxy_url.contains('@'),
        isolated,
        "{mode}: HTTPS_PROXY={proxy_url}"
    );
    assert!(
        !session_env.contains(SECRET),
        "{mode}: the value is in the session's env"
    );
    assert!(status.success(), "{mode}: {context}");
    assert!(out.contains(REPLY), "the upstream's reply: {context}");
    let messages: Vec<&String> = received
        .iter()
        .filter(|h| h.starts_with("POST /v1/messages"))
        .collect();
    assert!(!messages.is_empty(), "no Messages API call: {context}");
    for head in &received {
        let lower = head.to_ascii_lowercase();
        assert!(
            lower.contains(&format!("x-api-key: {}", SECRET.to_ascii_lowercase())),
            "a request without the real key: {head}"
        );
        assert!(
            !head.contains(&token),
            "the placeholder reached the upstream: {head}"
        );
    }
    assert!(!proxy_log.contains(SECRET), "the value is in the proxy log");
}
