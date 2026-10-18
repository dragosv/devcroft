//! The egress proxy's token in a network-isolated sandbox: not in the proxy
//! URL, presented by the unix socket's bridge instead
//! (`proxy::server::bridge_unix_to_proxy`; `docs/agents.md` has the five
//! agents that could not use userinfo). The TCP listener still requires it.
//!
//! The request from inside is made with bash's `/dev/tcp`, no HTTP client, so
//! nothing on the client side could be supplying a credential.

#![cfg(feature = "test-support")]

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down};
use devcroft::provider::{ProviderEntry, ProviderError, Resolution, ServiceSupport, Tier};
use devcroft::test_support::up_with_provider;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

fn store_path_with_bash() -> Option<PathBuf> {
    std::fs::read_dir("/nix/store")
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.join("bin/bash").is_file() && p.to_string_lossy().contains("bash"))
}

struct Row {
    store: PathBuf,
}

impl ProviderEntry for Row {
    fn resolve(&self, _: &Path) -> Result<Resolution, ProviderError> {
        Ok(Resolution {
            env: BTreeMap::from([(
                "PATH".to_owned(),
                self.store.join("bin").display().to_string(),
            )]),
            unset: Vec::new(),
            read_only_grants: vec!["/nix/store".to_owned()],
            activation_script: None,
            services: ServiceSupport::Unsupported,
            ran_activation_hook: false,
        })
    }
    fn fingerprint(&self, _: &Path) -> Result<String, ProviderError> {
        Ok("proxy-token-bridge-row".to_owned())
    }
    fn static_name(&self) -> &'static str {
        "nix"
    }
    fn tier(&self) -> Tier {
        Tier::Closure
    }
}

/// The status line of the proxy's answer to an unauthenticated CONNECT.
fn connect_status(stream: &mut std::net::TcpStream) -> String {
    stream
        .write_all(b"CONNECT allowed.invalid:443 HTTP/1.1\r\nHost: allowed.invalid:443\r\n\r\n")
        .unwrap();
    let mut buf = [0u8; 256];
    let n = stream.read(&mut buf).unwrap_or(0);
    String::from_utf8_lossy(&buf[..n])
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn an_isolated_sandbox_needs_no_proxy_credential_and_the_host_listener_still_does() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    let exe = Path::new(env!("CARGO_BIN_EXE_devcroft"));
    if !matches!(devcroft::fleet::netns::probe(exe), Ok(true)) {
        eprintln!("skipping: no network namespaces here, so the sandbox is not isolated");
        return;
    }
    let Some(store) = store_path_with_bash() else {
        eprintln!("skipping: no /nix/store bash with /dev/tcp on this host");
        return;
    };
    // SAFETY: this process runs a single test.
    unsafe { std::env::set_var("DEVCROFT_KEEPER_EXE", exe) };

    let pid = std::process::id();
    let project = PathBuf::from(format!("/tmp/dcproxybridge{pid}"));
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();
    let project = project.canonicalize().unwrap();
    let name = format!("e2eproxybridge{pid}");
    let (manifest, _) = parse(&format!(
        "[sandbox]\nname = {name:?}\n\n[network]\ndefault = \"deny\"\n\
         allow = [\"allowed.invalid\"]\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    let outcome = up_with_provider(&manifest, &project, &UpOptions::default(), &Row { store });
    // From inside: the proxy URL, then a credential-less CONNECT through it.
    let inside = Command::new(exe)
        .current_dir(&project)
        .args([
            "exec",
            &name,
            "--",
            "bash",
            "-c",
            "echo \"URL=$HTTPS_PROXY\"; port=${HTTPS_PROXY##*:}; \
             exec 3<>/dev/tcp/127.0.0.1/$port; \
             printf 'CONNECT allowed.invalid:443 HTTP/1.1\\r\\nHost: allowed.invalid:443\\r\\n\\r\\n' >&3; \
             read -r status <&3; echo \"STATUS=$status\"",
        ])
        .output()
        .unwrap();
    // From the host: the TCP listener, with no token.
    let port = devcroft::lifecycle::read_meta(&paths.meta)
        .ok()
        .flatten()
        .and_then(|m| m.proxy_port);
    let host_status = port
        .and_then(|p| std::net::TcpStream::connect(("127.0.0.1", p)).ok())
        .map(|mut s| connect_status(&mut s));
    let log = std::fs::read_to_string(&paths.log).unwrap_or_default();
    let _ = down(&name);
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project);

    assert_eq!(
        outcome.unwrap_or_else(|e| panic!("up: {e}\nlog:\n{log}")),
        UpOutcome::Started
    );
    let out = String::from_utf8_lossy(&inside.stdout);
    let url = out
        .lines()
        .find_map(|l| l.strip_prefix("URL="))
        .unwrap_or_default();
    assert!(
        url.starts_with("http://127.0.0.1:") && !url.contains('@'),
        "an isolated sandbox's proxy URL carries no credential: {url:?}"
    );
    let status = out
        .lines()
        .find_map(|l| l.strip_prefix("STATUS="))
        .unwrap_or_default();
    // Past authentication: the allowlist admits the name, and only then does
    // resolving it fail. A 407 would be the bridge not presenting the token.
    assert!(
        status.starts_with("HTTP/1.1"),
        "{out}{}",
        String::from_utf8_lossy(&inside.stderr)
    );
    assert!(
        !status.contains(" 407 "),
        "refused for want of a credential: {status}"
    );
    let host_status = host_status.expect("the proxy's TCP listener is reachable from the host");
    assert!(
        host_status.contains(" 407 "),
        "the TCP listener must still require the token: {host_status}"
    );
}
