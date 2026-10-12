//! `sandbox-provisioning` P2 and the network spec's "declared per context":
//! the provider's activation script runs under the **provisioning**
//! policy, in its own process with its own proxy, and sessions under the
//! **runtime** one. Before, the keeper ran the script under the runtime
//! policy, so a hook that needed a registry got it only from the runtime
//! allowlist, and then every session kept reaching it.
//!
//! Driven through the provider seam (`test_support::up_with_provider`) with
//! a row whose closure is a store shell plus the host's `/usr`, so the hook
//! and the session have `curl` without a real provider. Two local servers,
//! on 127.0.0.3 (provisioning's) and 127.0.0.4 (runtime's), so no internet
//! is needed; `NO_PROXY` does not exempt them.

#![cfg(feature = "test-support")]

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down};
use devcroft::provider::{ProviderEntry, ProviderError, Resolution, ServiceSupport, Tier};
use devcroft::test_support::up_with_provider;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn store_path_with_a_shell() -> Option<PathBuf> {
    std::fs::read_dir("/nix/store")
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.join("bin/sh").is_file() && p.to_string_lossy().contains("bash"))
}

/// A host-side HTTP server answering `200 ok` on `addr`, for the test's life.
fn serve_ok(addr: &str) -> Option<u16> {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind((addr, 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        }
    });
    Some(port)
}

struct Row {
    store: PathBuf,
    hook: String,
}

impl ProviderEntry for Row {
    fn resolve(&self, _: &Path) -> Result<Resolution, ProviderError> {
        Ok(Resolution {
            env: BTreeMap::from([(
                "PATH".to_owned(),
                format!("{}:/usr/bin", self.store.join("bin").display()),
            )]),
            unset: Vec::new(),
            // The store root, as a real provider grants it: the shell's own
            // store path alone is not enough to exec it, since its dynamic
            // loader lives in another (an exec without it is ENOENT).
            read_only_grants: vec!["/nix/store".to_owned(), "/usr".to_owned()],
            activation_script: Some(self.hook.clone()),
            services: ServiceSupport::Unsupported,
            ran_activation_hook: false,
        })
    }
    fn fingerprint(&self, _: &Path) -> Result<String, ProviderError> {
        Ok("provisioning-network-row".to_owned())
    }
    fn static_name(&self) -> &'static str {
        "nix"
    }
    fn tier(&self) -> Tier {
        Tier::Closure
    }
}

#[test]
fn the_hook_reaches_provisionings_hosts_and_sessions_reach_runtimes() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    if !Path::new("/usr/bin/curl").exists() {
        eprintln!("skipping: no /usr/bin/curl");
        return;
    }
    let Some(store) = store_path_with_a_shell() else {
        eprintln!("skipping: no /nix/store path with a shell on this host");
        return;
    };
    let (Some(p3), Some(p4)) = (serve_ok("127.0.0.3"), serve_ok("127.0.0.4")) else {
        eprintln!("skipping: no 127.0.0.3/127.0.0.4 loopback aliases");
        return;
    };
    // SAFETY: this process runs a single test.
    unsafe { std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft")) };

    let project = PathBuf::from(format!("/tmp/dcprov{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();
    let project = project.canonicalize().unwrap();
    let name = format!("e2eprov{}", std::process::id());
    let (manifest, _) = parse(&format!(
        "[sandbox]\nname = {name:?}\n\n[network]\ndefault = \"deny\"\nallow = [\"127.0.0.4\"]\n\n\
         [provisioning.network]\nallow = [\"127.0.0.3\"]\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    let code = |url: String| {
        format!("/usr/bin/curl -s -o /dev/null --max-time 5 -w '%{{http_code}}' {url}")
    };
    let (to3, to4) = (
        format!("http://127.0.0.3:{p3}/"),
        format!("http://127.0.0.4:{p4}/"),
    );
    let row = Row {
        store,
        hook: format!(
            "{} > hook-reached-3; {} > hook-reached-4; export FROM_THE_HOOK=yes",
            code(to3.clone()),
            code(to4.clone())
        ),
    };
    let outcome = up_with_provider(&manifest, &project, &UpOptions::default(), &row);
    let session = |cmd: &str| {
        Command::new(env!("CARGO_BIN_EXE_devcroft"))
            .current_dir(&project)
            .args(["exec", &name, "--", "/usr/bin/env", "sh", "-c", cmd])
            .output()
            .unwrap()
    };
    let out = session(&format!(
        "{}; echo; {}; echo; echo \"${{FROM_THE_HOOK-unset}}\"",
        code(to3),
        code(to4)
    ));
    let log = std::fs::read_to_string(&paths.log).unwrap_or_default();
    let read = |f: &str| std::fs::read_to_string(project.join(f)).unwrap_or_default();
    let (hook3, hook4) = (read("hook-reached-3"), read("hook-reached-4"));
    let _ = down(&name);
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project);

    assert_eq!(
        outcome.unwrap_or_else(|e| panic!("up: {e}\nlog:\n{log}")),
        UpOutcome::Started
    );
    // The hook, under the provisioning policy: provisioning's host, and not
    // the runtime's, which provisioning does not inherit.
    assert_eq!(
        hook3, "200",
        "the hook could not reach its provisioning host\n{log}"
    );
    assert_ne!(hook4, "200", "the hook reached a runtime-only host\n{log}");
    // A session, under the runtime policy: the reverse.
    let seen = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = seen.lines().collect();
    assert_eq!(lines.len(), 3, "{out:?}");
    assert_ne!(
        lines[0], "200",
        "a session reached the provisioning-only host"
    );
    assert_eq!(
        lines[1], "200",
        "a session could not reach its runtime host: {out:?}"
    );
    // And what the hook exported still reaches the session, now across two
    // processes.
    assert_eq!(lines[2], "yes", "{out:?}");
}
