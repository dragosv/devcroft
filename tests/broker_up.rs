//! Brokered credentials through `up` (`add-agent-workload` 4b.1, 4b.2, 4b.4,
//! 4b.5): the sandbox holds a placeholder and an endpoint, the proxy holds the
//! value, and nothing devcroft writes or starts holds it anywhere else.
//!
//! Inside a real, network-isolated sandbox, so the endpoint is reached the
//! way an agent reaches it: through the relay and the proxy's unix bridge.
//! The upstream is `broker.invalid`, which resolves nowhere, so a request the
//! proxy accepts ends in its own upstream error, and one it refuses ends in
//! its refusal: the two are told apart by status, and nothing leaves.
//!
//! The substitution itself, against a TLS upstream, is `proxy::server`'s
//! `a_brokered_request_reaches_the_upstream_with_the_real_value`.

#![cfg(feature = "test-support")]

use devcroft::config::{BrokerDecl, parse};
use devcroft::lifecycle::{StatePaths, UpError, UpOptions, UpOutcome, down};
use devcroft::provider::{ProviderEntry, ProviderError, Resolution, ServiceSupport, Tier};
use devcroft::test_support::up_with_provider;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const SECRET: &str = "real-secret-never-inside-4b2";

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
        Ok("broker-up-row".to_owned())
    }
    fn static_name(&self) -> &'static str {
        "nix"
    }
    fn tier(&self) -> Tier {
        Tier::Closure
    }
}

fn broker(env: &str) -> BrokerDecl {
    BrokerDecl {
        host: "broker.invalid".into(),
        header: "x-api-key".into(),
        env: env.into(),
        endpoint_env: format!("{env}_ENDPOINT"),
        placeholder: None,
    }
}

/// Every file under `dir` whose bytes contain `needle`.
fn files_containing(dir: &Path, needle: &[u8]) -> Vec<PathBuf> {
    let mut hits = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return hits;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            hits.extend(files_containing(&path, needle));
        } else if kind.is_file()
            && std::fs::read(&path)
                .is_ok_and(|bytes| bytes.windows(needle.len()).any(|w| w == needle))
        {
            hits.push(path);
        }
    }
    hits
}

fn environ_holds(pid: &str, needle: &str) -> Option<bool> {
    let bytes = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    Some(bytes.windows(needle.len()).any(|w| w == needle.as_bytes()))
}

fn pid_in(pidfile: &Path) -> Option<String> {
    std::fs::read_to_string(pidfile)
        .ok()?
        .split_whitespace()
        .next()
        .map(str::to_owned)
}

#[test]
fn the_sandbox_holds_the_placeholder_and_nothing_else_holds_the_value() {
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
    // SAFETY: names no other test in this binary reads or writes.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", exe);
        std::env::set_var("DCTEST_BROKERED_KEY", SECRET);
    }

    let pid = std::process::id();
    let project = PathBuf::from(format!("/tmp/dcbroker{pid}"));
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();
    let project = project.canonicalize().unwrap();
    let name = format!("e2ebroker{pid}");
    let text = format!(
        "[sandbox]\nname = {name:?}\n\n[network]\ndefault = \"deny\"\n\
         allow = [\"broker.invalid\"]\n"
    );
    // On disk too, for `policy --render` below to find.
    std::fs::write(project.join("devcroft.toml"), &text).unwrap();
    let (manifest, _) = parse(&text).unwrap();
    let paths = StatePaths::new(&name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_file(&paths.lifecycle_lock);

    let brokers = [broker("DCTEST_BROKERED_KEY")];
    let opts = UpOptions {
        brokers: &brokers,
        ..UpOptions::default()
    };
    let outcome = up_with_provider(&manifest, &project, &opts, &Row { store });

    // From inside: what the sandbox was given, then one request to the
    // endpoint with the placeholder and one with a wrong key, raw over
    // `/dev/tcp` so no client could be adding anything.
    let script = "echo \"KEY=$DCTEST_BROKERED_KEY\"; echo \"URL=$DCTEST_BROKERED_KEY_ENDPOINT\"; \
         rest=${DCTEST_BROKERED_KEY_ENDPOINT#http://127.0.0.1:}; port=${rest%%/*}; \
         for key in \"$DCTEST_BROKERED_KEY\" wrong-key; do \
           exec 3<>/dev/tcp/127.0.0.1/$port; \
           printf 'GET /broker0/v1/x HTTP/1.1\\r\\nHost: 127.0.0.1\\r\\nx-api-key: %s\\r\\nConnection: close\\r\\n\\r\\n' \"$key\" >&3; \
           read -r status <&3; echo \"STATUS=$status\"; exec 3<&-; \
         done; \
         case \"$(declare -x)\" in *real-secret-never-inside*) echo ENV=leaked;; *) echo ENV=clean;; esac";
    let inside = Command::new(exe)
        .current_dir(&project)
        .args(["exec", &name, "--", "bash", "-c", script])
        .output()
        .unwrap();

    // 4b.6: the rendered policy names the brokered host, origin
    // `credential:broker`, from the artifact `up` wrote.
    let rendered = Command::new(exe)
        .current_dir(&project)
        .args(["policy", "--render", &name])
        .output()
        .unwrap();

    let meta = devcroft::lifecycle::read_meta(&paths.meta).ok().flatten();
    let keeper_environ = pid_in(&paths.pidfile).and_then(|p| environ_holds(&p, SECRET));
    let proxy_environ = pid_in(&paths.proxy_pidfile).and_then(|p| environ_holds(&p, SECRET));
    let state_hits = files_containing(&paths.root, SECRET.as_bytes());
    let project_hits = files_containing(&project, SECRET.as_bytes());
    let log = std::fs::read_to_string(&paths.log).unwrap_or_default();
    let _ = down(&name);
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_file(&paths.lifecycle_lock);
    let _ = std::fs::remove_dir_all(&project);

    assert_eq!(
        outcome.unwrap_or_else(|e| panic!("up: {e}\nlog:\n{log}")),
        UpOutcome::Started
    );
    let meta = meta.expect("meta.json after up");
    let token = meta.proxy_token.clone().expect("a proxy token");
    let port = meta.proxy_port.expect("a proxy port");
    assert_eq!(
        meta.brokers, brokers,
        "Meta records the broker, as declared"
    );

    let out = String::from_utf8_lossy(&inside.stdout);
    let err = String::from_utf8_lossy(&inside.stderr);
    let field = |key: &str| -> Vec<String> {
        out.lines()
            .filter_map(|l| l.strip_prefix(key))
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(field("KEY="), vec![token.clone()], "{out}{err}");
    assert_eq!(
        field("URL="),
        vec![format!("http://127.0.0.1:{port}/broker0")],
        "{out}{err}"
    );
    let statuses = field("STATUS=");
    assert_eq!(statuses.len(), 2, "{out}{err}");
    // The placeholder passes the route's check, so the proxy goes on to the
    // upstream, which resolves nowhere: an upstream error, not a refusal.
    assert!(
        !statuses[0].contains(" 401")
            && !statuses[0].contains(" 407")
            && !statuses[0].contains(" 200"),
        "the placeholder must be accepted and fail only upstream: {statuses:?}"
    );
    // A wrong key is refused by the route before any upstream contact.
    assert!(
        statuses[1].contains(" 401")
            || statuses[1].contains(" 403")
            || statuses[1].contains(" 407"),
        "a wrong key must be refused at the proxy: {statuses:?}"
    );
    assert_ne!(statuses[0], statuses[1], "{statuses:?}");
    // A builtin, not `env | grep`: the row's PATH holds bash alone.
    assert_eq!(
        field("ENV="),
        vec!["clean".to_owned()],
        "the value is in the sandbox's env: {out}"
    );

    let rendered = String::from_utf8_lossy(&rendered.stdout);
    let line = rendered
        .lines()
        .skip_while(|l| *l != "credentials.broker:")
        .nth(1)
        .unwrap_or_else(|| panic!("no credentials.broker section:\n{rendered}"));
    assert!(line.contains("broker.invalid"), "{line}");
    assert!(line.trim_end().ends_with("credential:broker"), "{line}");
    assert!(!rendered.contains(SECRET), "the value is rendered");

    assert!(state_hits.is_empty(), "the value is in {state_hits:?}");
    assert!(project_hits.is_empty(), "the value is in {project_hits:?}");
    assert_eq!(
        keeper_environ,
        Some(false),
        "keeper environ (None: unreadable)"
    );
    assert_eq!(
        proxy_environ,
        Some(false),
        "proxy environ (None: unreadable)"
    );
}

/// 4b.5: each refusal is at layer `config`, before any state exists.
#[test]
fn a_broker_is_refused_rather_than_downgraded() {
    let Some(store) = store_path_with_bash() else {
        eprintln!("skipping: no /nix/store bash on this host");
        return;
    };
    let pid = std::process::id();
    let project = PathBuf::from(format!("/tmp/dcbrokerrefuse{pid}"));
    std::fs::create_dir_all(&project).unwrap();
    let name = format!("e2ebrokerrefuse{pid}");
    let refused = |manifest_tail: &str, env: &str| -> String {
        let (manifest, _) =
            parse(&format!("[sandbox]\nname = {name:?}\n\n{manifest_tail}")).unwrap();
        let brokers = [broker(env)];
        let opts = UpOptions {
            brokers: &brokers,
            ..UpOptions::default()
        };
        match up_with_provider(
            &manifest,
            &project,
            &opts,
            &Row {
                store: store.clone(),
            },
        ) {
            Err(e @ UpError::Config(_)) => e.to_string(),
            other => panic!("{manifest_tail:?}: expected a config refusal, got {other:?}"),
        }
    };
    // SAFETY: names no other test in this binary reads or writes.
    unsafe { std::env::set_var("DCTEST_REFUSED_KEY", "x") };

    let deny = "[network]\ndefault = \"deny\"\nallow = [\"other.invalid\"]\n";
    let err = refused(deny, "DCTEST_REFUSED_KEY");
    assert!(
        err.contains("network.allow") && err.contains("broker.invalid"),
        "{err}"
    );

    let err = refused("[network]\ndefault = \"allow\"\n", "DCTEST_REFUSED_KEY");
    assert!(err.contains("network.default"), "{err}");

    let allowed = "[network]\ndefault = \"deny\"\nallow = [\"broker.invalid\"]\n";
    let err = refused(
        &format!("{allowed}\n[env.vars]\nDCTEST_REFUSED_KEY = \"committed\"\n"),
        "DCTEST_REFUSED_KEY",
    );
    assert!(err.contains("[env.vars]"), "{err}");

    let err = refused(allowed, "DCTEST_UNSET_KEY_4B5");
    assert!(err.contains("not set in this shell"), "{err}");

    let paths = StatePaths::new(&name).unwrap();
    assert!(!paths.root.exists(), "a refusal left state behind");
    let _ = std::fs::remove_dir_all(&project);
}
