//! `add-port-allocation`, end to end against real flox environments,
//! through the CLI: what a user runs, what `up` prints, what `status` and
//! `policy --render` report, and what a client inside each sandbox reaches.
//!
//! Allocation applies only where a sandbox shares the host's loopback,
//! which on a host that can create namespaces means
//! `network.default = "allow"`: every deny sandbox gets its own namespace,
//! `network.allow` or not (`CompiledPolicy::wants_network_isolation`). So A
//! and B use `"allow"`, and C puts the two modes side by side.
//!
//! See `tests/services_e2e.rs` for the project-root and tooling notes this
//! shares; each test builds its own flox project, so they run in parallel.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

/// The provider's own value for the service's port. Allocation must
/// override it; a service still on it means substitution did not happen.
const DECLARED: u16 = 18781;

fn devcroft() -> &'static str {
    env!("CARGO_BIN_EXE_devcroft")
}

fn tooling_missing() -> bool {
    !devcroft::policy::backend_supported()
        || Command::new("flox").arg("--version").output().is_err()
        || !devcroft::provider::host_can_build_nix_closures()
}

/// `devcroft <args>` in `dir`: (exit code, stdout, stderr).
fn run(dir: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    let Output {
        status,
        stdout,
        stderr,
    } = Command::new(devcroft())
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    (
        status.code(),
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
    )
}

/// A flox project with process-compose, python3 and bash, declaring one
/// service `web` whose command is `command` (with `{port}` replaced by
/// `$WEB_PORT` or a literal), its port arriving through `vars.WEB_PORT`.
/// `None` (after printing why) when flox cannot build it here.
fn flox_project(root: &Path, command: &str) -> Option<()> {
    std::fs::create_dir_all(root).unwrap();
    let ok = |o: Output| o.status.success();
    if !ok(Command::new("flox")
        .arg("init")
        .current_dir(root)
        .output()
        .unwrap())
        || !ok(Command::new("flox")
            .args(["install", "process-compose", "python3", "bash"])
            .current_dir(root)
            .output()
            .unwrap())
    {
        eprintln!("skipping: flox could not build the test environment");
        return None;
    }
    let manifest_path = root.join(".flox/env/manifest.toml");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap().replacen(
        "[services]\n",
        &format!("[services]\nweb.command = \"{command}\"\nweb.vars.WEB_PORT = \"{DECLARED}\"\n"),
        1,
    );
    std::fs::write(manifest_path, manifest).unwrap();
    Some(())
}

/// The devcroft manifest every test here uses: an allocation request for
/// `web`'s `WEB_PORT`, under `network.default = default` (`"allow"` shares
/// the host's loopback, `"deny"` gets the sandbox its own namespace).
fn write_devcroft_toml(root: &Path, name: &str, default: &str) {
    write_devcroft_toml_with(root, name, default, "");
}

/// As [`write_devcroft_toml`], with more keys in the `web` entry.
fn write_devcroft_toml_with(root: &Path, name: &str, default: &str, web_extra: &str) {
    std::fs::write(
        root.join("devcroft.toml"),
        format!(
            "[sandbox]\nname = {name:?}\n\n[env]\nprovider = \"flox\"\n\n\
             [network]\ndefault = {default:?}\n\n\
             [network.services.web]\nvar = \"WEB_PORT\"\n{web_extra}"
        ),
    )
    .unwrap();
}

/// A root short enough for the supervisor socket, canonical.
fn short_root(tag: &str) -> PathBuf {
    let root = PathBuf::from(format!("/tmp/dcpa-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

/// The allocated port `status` reports for `web`, if any.
fn reported_port(dir: &Path, name: &str) -> Option<u16> {
    let (_, out, _) = run(dir, &["status", name]);
    out.lines()
        .find_map(|l| l.strip_prefix("port WEB_PORT="))
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
}

/// GET `path` from 127.0.0.1:`port` by a client *inside* sandbox `name`.
fn fetch_inside(dir: &Path, name: &str, port: u16, path: &str) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let code = format!(
        "import urllib.request; print(urllib.request.urlopen('http://127.0.0.1:{port}{path}', \
         timeout=3).read().decode().strip())"
    );
    while Instant::now() < deadline {
        let (status, out, _) = run(dir, &["exec", name, "--", "python3", "-c", &code]);
        if status == Some(0) {
            return Some(out.trim().to_owned());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    None
}

/// Removes a sandbox's state and its project, whatever happens.
struct Sandboxes(Vec<(PathBuf, String)>, Vec<PathBuf>);

impl Drop for Sandboxes {
    fn drop(&mut self) {
        for (dir, name) in &self.0 {
            let _ = run(dir, &["rm", name, "--yes"]);
        }
        for root in &self.1 {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

/// A: allocation reaches the service and the sessions, is reported, is
/// rendered, sticks across a restart, is announced when it has to move,
/// and is shown as merely recorded while the sandbox is down.
#[test]
fn an_allocated_port_is_used_reported_rendered_sticky_and_announced() {
    if tooling_missing() {
        eprintln!("skipping: no usable flox here");
        return;
    }
    let root = short_root("a");
    if flox_project(&root, "python3 -m http.server $WEB_PORT --bind 127.0.0.1").is_none() {
        return;
    }
    let name = format!("dcpa{}", std::process::id());
    write_devcroft_toml(&root, &name, "allow");
    let _cleanup = Sandboxes(vec![(root.clone(), name.clone())], vec![root.clone()]);
    std::fs::write(root.join("marker"), &name).unwrap();

    let (code, _, err) = run(&root, &["up"]);
    assert_eq!(code, Some(0), "{err}");
    let port = reported_port(&root, &name).expect("status reports the allocated port");
    assert_ne!(port, DECLARED, "the provider's value was not overridden");

    // The service listens on it (substitution into the generated config),
    // and sessions can read it (the environment).
    assert_eq!(
        fetch_inside(&root, &name, port, "/marker").as_deref(),
        Some(name.as_str())
    );
    let (_, env, _) = run(&root, &["exec", &name, "--", "sh", "-c", "echo $WEB_PORT"]);
    assert_eq!(env.trim(), port.to_string());
    // The provider's manifest on disk is untouched.
    let flox = std::fs::read_to_string(root.join(".flox/env/manifest.toml")).unwrap();
    assert!(
        flox.contains(&format!("WEB_PORT = \"{DECLARED}\"")),
        "{flox}"
    );

    // Rendered, with its request and its origin.
    let (_, render, _) = run(&root, &["policy", "--render"]);
    assert!(
        render.contains("web.WEB_PORT") && render.contains(&format!("{port} (allocated)")),
        "{render}"
    );

    // Sticky across a restart.
    run(&root, &["down", &name]);
    let (_, stopped, _) = run(&root, &["status", &name]);
    assert!(stopped.contains("nothing is listening"), "{stopped}");
    let (code, _, err) = run(&root, &["up"]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(reported_port(&root, &name), Some(port));
    assert!(
        !err.contains("allocation changed"),
        "an unchanged allocation is not announced: {err}"
    );

    // Taken while down: a new port, announced with both numbers.
    run(&root, &["down", &name]);
    let holder = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    let (code, _, err) = run(&root, &["up"]);
    assert_eq!(code, Some(0), "a lost port is not a failure: {err}");
    let moved = reported_port(&root, &name).unwrap();
    assert_ne!(moved, port);
    assert!(
        err.contains("allocation changed")
            && err.contains(&port.to_string())
            && err.contains(&moved.to_string()),
        "{err}"
    );
    drop(holder);
    assert_eq!(
        fetch_inside(&root, &name, moved, "/marker").as_deref(),
        Some(name.as_str())
    );
}

/// B, 6.1 and 6.2: two sandboxes of one repository run the same service at
/// once. From two `git worktree` checkouts, with explicit distinct names
/// (two worktrees of one committed manifest otherwise share a sandbox name,
/// which `add-agent-workload` addresses), and from one project root under
/// two names. Each gets its own port, and a client inside each reaches its
/// own service on it.
#[test]
fn two_sandboxes_of_one_repository_each_get_their_own_port() {
    if tooling_missing() {
        eprintln!("skipping: no usable flox here");
        return;
    }
    let base = short_root("b");
    let repo = base.join("r");
    if flox_project(&repo, "python3 -m http.server $WEB_PORT --bind 127.0.0.1").is_none() {
        return;
    }
    write_devcroft_toml(&repo, "unused", "allow");
    let git = |dir: &Path, args: &[&str]| {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .unwrap()
                .status
                .success(),
            "{args:?}"
        );
    };
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["add", "."]);
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "--quiet",
            "-m",
            "init",
        ],
    );
    let (w1, w2) = (base.join("w1"), base.join("w2"));
    git(&repo, &["worktree", "add", "--quiet", w1.to_str().unwrap()]);
    git(&repo, &["worktree", "add", "--quiet", w2.to_str().unwrap()]);

    let pid = std::process::id();
    let pairs = [
        (w1.clone(), format!("dcpaw1{pid}")),
        (w2.clone(), format!("dcpaw2{pid}")),
        (repo.clone(), format!("dcpar1{pid}")),
        (repo.clone(), format!("dcpar2{pid}")),
    ];
    let _cleanup = Sandboxes(pairs.to_vec(), vec![base.clone()]);

    let mut ports = Vec::new();
    for (dir, name) in &pairs {
        let (code, _, err) = run(dir, &["up", "--name", name]);
        assert_eq!(code, Some(0), "{name}: {err}");
        // Through `status <name>`, from the directory whose manifest names
        // another sandbox: it resolves a `--name` sandbox of this project.
        ports.push(reported_port(dir, name).expect("status reports each one's own port"));
    }
    let mut distinct = ports.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 4, "{ports:?}");

    // The worktrees serve their own directories, so each client inside a
    // worktree's sandbox reaching its own port gets its own marker.
    for (i, (dir, name)) in pairs.iter().enumerate().take(2) {
        std::fs::write(dir.join("marker"), name).unwrap();
        assert_eq!(
            fetch_inside(dir, name, ports[i], "/marker").as_deref(),
            Some(name.as_str())
        );
    }
    // One project root, two names: separate generated configs and
    // supervisor sockets, and each service on its own port.
    let (a, b) = (&pairs[2].1, &pairs[3].1);
    assert_ne!(
        devcroft::services::config_path(&repo, a),
        devcroft::services::config_path(&repo, b)
    );
    for (i, name) in [(2, a), (3, b)] {
        let config = std::fs::read_to_string(devcroft::services::config_path(&repo, name)).unwrap();
        assert!(
            config.contains(&ports[i].to_string()),
            "{name}'s config: {config}"
        );
        assert!(fetch_inside(&repo, name, ports[i], "/").is_some());
    }
}

/// C, 4.4 and fleet's 5.7: a service that hardcodes its port, with an
/// allocation requested for it. Where the sandbox shares the host's
/// loopback (`"allow"`), `up` refuses, naming the service, since it would
/// never listen on the allocated port. With its own namespace (`"deny"`),
/// nothing is allocated and the same service runs unchanged, with no
/// warning, both in `up` and in a fleet agent.
///
/// **Not literally one manifest**, and why: on a host that can create
/// namespaces, `up` shares the loopback only under `"allow"`, which fleet
/// refuses by design (an agent's route-less namespace is its egress
/// boundary). So the two manifests differ in `network.default` alone, which
/// is exactly the property that decides whether allocation applies. A fleet
/// that had copied `up`'s refusal fails the `"deny"` half; an `up` that had
/// dropped it passes the `"allow"` half.
#[test]
fn a_hardcoded_port_is_refused_where_allocated_and_runs_unchanged_with_a_namespace() {
    if tooling_missing() {
        eprintln!("skipping: no usable flox here");
        return;
    }
    let root = short_root("c");
    if flox_project(&root, "python3 -m http.server 18782 --bind 127.0.0.1").is_none() {
        return;
    }
    let name = format!("dcpac{}", std::process::id());
    let _cleanup = Sandboxes(vec![(root.clone(), name.clone())], vec![root.clone()]);

    // Shared loopback: refused, naming the service and its literal port.
    // `port` is the literal the service binds, which is what a namespace
    // uses unchanged below.
    write_devcroft_toml_with(&root, &name, "allow", "port = 18782\n");
    let (code, _, err) = run(&root, &["up"]);
    assert_eq!(code, Some(2), "{err}");
    assert!(
        err.contains("network.services.web.var") && err.contains("18782"),
        "{err}"
    );

    // Own namespace: nothing allocated, the declared port used unchanged,
    // no refusal and no warning. Linux only: off Linux no sandbox gets a
    // namespace, so `"deny"` shares the loopback and is refused the same way.
    #[cfg(target_os = "linux")]
    hardcoded_port_runs_unchanged_with_a_namespace(&root, &name);
}

#[cfg(target_os = "linux")]
fn hardcoded_port_runs_unchanged_with_a_namespace(root: &Path, name: &str) {
    let (root, name) = (root.to_path_buf(), name.to_string());
    write_devcroft_toml_with(&root, &name, "deny", "port = 18782\n");
    let (code, _, err) = run(&root, &["up"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(!err.contains("warning"), "{err}");
    assert_eq!(
        reported_port(&root, &name),
        None,
        "nothing is allocated with a namespace"
    );
    assert!(fetch_inside(&root, &name, 18782, "/").is_some());
    run(&root, &["rm", &name, "--yes"]);

    // And in a fleet agent. Needs a delegated cgroup this process sits in.
    let Some(cgroup) = std::env::var_os("DEVCROFT_TEST_CGROUP_ROOT").map(PathBuf::from) else {
        eprintln!("skipping the fleet half: DEVCROFT_TEST_CGROUP_ROOT is not set");
        return;
    };
    if !devcroft::fleet::cgroup::own_cgroup().is_ok_and(|own| own.starts_with(&cgroup)) {
        eprintln!("skipping the fleet half: run `sudo cgroup-delegate enter $$` first");
        return;
    }
    for args in [
        &["init", "--quiet"][..],
        &["add", "."],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "--quiet",
            "-m",
            "init",
        ],
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let (code, out, err) = run(
        &root,
        &[
            "fleet",
            "up",
            "--agents",
            "1",
            "--cgroup-root",
            cgroup.to_str().unwrap(),
        ],
    );
    let _ = run(&root, &["fleet", "rm", "--all", "--yes"]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert!(out.contains("services ready"), "{out}{err}");
    assert!(!err.contains("warning"), "no warning in fleet: {err}");
}

/// D: a `var`-only request in a sandbox with its own namespace. Nothing is
/// allocated, and the service's port is the one the provider declares for
/// the variable, granted unchanged. Without that, the same manifest that
/// works where the loopback is shared would leave the service unable to
/// bind here. Linux only: namespaces are the precondition.
#[cfg(target_os = "linux")]
#[test]
fn a_var_only_request_with_a_namespace_uses_the_providers_port() {
    if tooling_missing() {
        eprintln!("skipping: no usable flox here");
        return;
    }
    let root = short_root("d");
    if flox_project(&root, "python3 -m http.server $WEB_PORT --bind 127.0.0.1").is_none() {
        return;
    }
    let name = format!("dcpad{}", std::process::id());
    write_devcroft_toml(&root, &name, "deny");
    let _cleanup = Sandboxes(vec![(root.clone(), name.clone())], vec![root.clone()]);
    std::fs::write(root.join("marker"), &name).unwrap();

    let (code, _, err) = run(&root, &["up"]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(
        reported_port(&root, &name),
        None,
        "nothing is allocated with a namespace"
    );
    assert_eq!(
        fetch_inside(&root, &name, DECLARED, "/marker").as_deref(),
        Some(name.as_str())
    );
    let (_, render, _) = run(&root, &["policy", "--render"]);
    assert!(
        render.contains(&DECLARED.to_string()) && render.contains("manifest:network.services"),
        "{render}"
    );
}
