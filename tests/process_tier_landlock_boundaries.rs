//! Four real attacks against a live `process`-tier sandbox, run to find
//! out whether CLAUDE.md's "accident protection, not a security boundary"
//! framing means these specific vectors are actually open. It was written
//! expecting all four to succeed (no PID namespace, cooperative network
//! filtering — both already claimed as known gaps by README.md and
//! design.md Decision 5 before this file existed). On this host —
//! Landlock ABI **V6** specifically, confirmed by `devcroft doctor`'s own
//! `kernel: Landlock V6` line — all four are blocked instead. That's a
//! real, kernel-version-dependent finding, not a foregone conclusion:
//! Landlock's signal-scoping (the mechanism behind the first attack) only
//! exists from ABI V6 onward, and older kernels this project still
//! supports would very plausibly reproduce the originally-assumed gap.
//! README.md's known-gaps section and `docs/decisions.md`'s "Cooperative
//! network filtering" entry are corrected accordingly, citing this file.
//!
//! What's tested, and why each one is a real attempt rather than a
//! synthetic check:
//!
//! 1. **Cross-process signal delivery.** No PID namespace exists at this
//!    tier (confirmed separately: `grep` finds no
//!    `unshare`/`CLONE_NEWPID`/`setns` anywhere in `src/keeper` or
//!    `src/lifecycle`'s `process`-tier path), so a sandboxed process
//!    shares the host's raw PID space and, running as the same uid, has
//!    every DAC precondition met to `kill()` a process with nothing to
//!    do with the sandbox. Landlock V6's signal-scoping LSM hook is the
//!    only thing standing in the way — verified live: it does.
//! 2. **`/proc/<pid>/*` introspection of that same process**, via its
//!    real path rather than directory listing (Landlock mediates each
//!    path independently) — blocked by the same default-deny filesystem
//!    policy that already governs every other ungranted path; `/proc`
//!    just happens to be one more real VFS path nothing grants.
//! 3. **A raw socket, bypassing devcroft's `[network]` policy entirely**,
//!    against `network.default = "deny"` with no allowlist — blocked at
//!    the kernel level (`Permission denied`, not `Connection refused`),
//!    meaning nono's `block: true` is a genuine Landlock network-scope
//!    rule, not a proxy the raw socket simply never talks to.
//! 4. **The same raw socket against an *unrelated* IP while a domain
//!    allowlist is active** (`network.allow = ["example.com"]`) — the
//!    exact shape `docs/decisions.md`'s original "Cooperative network
//!    filtering" entry named as unstopped ("raw sockets, direct IPs...
//!    not stopped by this mechanism"). `policy --render` still shows
//!    `network.block: true` with the allowlist layered on top, and the
//!    connect to an IP with no relationship to the allowed domain is
//!    still denied at the kernel level. What this does *not* rule out:
//!    whether the *allowed* domain's own resolved-IP scope is wider than
//!    intended (a different service on the same allowed IP, or DNS-
//!    rebinding-shaped tricks) — untested here, and the doc corrections
//!    this file drives are careful not to claim more than what was
//!    actually run.
//!
//! See `tests/lifecycle_up.rs` for why this needs `CARGO_BIN_EXE_devcroft`
//! and why each such test lives in its own file/process.

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down, up};
use std::net::TcpListener;
use std::process::{Command, Stdio};

fn tooling_missing() -> bool {
    !devcroft::policy::backend_supported()
        || (Command::new("flox").arg("--version").output().is_err()
            || !devcroft::provider::host_can_build_nix_closures())
}

/// A loopback listener the sandbox has no legitimate reason to reach —
/// bound to an OS-assigned ephemeral port (no fixed-port collisions
/// across concurrent test runs) and actually accepting, so a failed
/// connect can only mean the connect itself was refused, not that
/// nothing was listening yet.
fn spawn_forbidden_listener() -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        listener.set_nonblocking(false).unwrap();
        let _ = listener.accept();
    });
    (port, handle)
}

/// `bash`'s own `/dev/tcp` (not `sh`'s — dash has no such feature) as the
/// raw-socket client — installed into each fixture's flox environment
/// (own-policy-baseline excludes host toolchain access, so this is no
/// longer "already on the sandbox's inherited system `PATH`", the false
/// assumption this comment used to make). Its stderr on failure names the
/// real reason (`Permission denied` for a kernel-level Landlock deny vs
/// `Connection refused` for "nobody was listening"), which is exactly the
/// distinction these tests need to be meaningful — and exactly why a
/// missing `bash` would be a silent false pass: `devcroft exec`'s own
/// "keeper refused to spawn: Permission denied" also contains the
/// substring these tests check for.
/// A raw-socket `/dev/tcp` connect blocked at the kernel level, regardless
/// of which errno the running nono/kernel combination surfaces it as.
/// Verified live against both nono 0.71.0 (`Permission denied`, EACCES)
/// and 0.74.0 (`Operation not permitted`, EPERM) — same enforcement
/// (`socket()` itself is refused, not merely an unreached proxy), an
/// upstream difference in which errno the Landlock network-scope deny
/// surfaces as, not a regression in either version. own-policy-baseline
/// task 6.2's compatibility record for this specific behavior.
fn stderr_is_a_kernel_level_denial(stderr: &str) -> bool {
    stderr.contains("Permission denied") || stderr.contains("Operation not permitted")
}

fn attempt_raw_connect(devcroft_bin: &str, sandbox_name: &str, port: u16) -> std::process::Output {
    Command::new(devcroft_bin)
        .arg("exec")
        .arg(sandbox_name)
        .arg("--")
        .arg("bash")
        .arg("-c")
        .arg(format!(
            "exec 3<>/dev/tcp/127.0.0.1/{port} && echo CONNECTED"
        ))
        .output()
        .unwrap()
}

#[test]
fn process_tier_blocks_cross_process_signals_and_proc_reads() {
    if tooling_missing() {
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
        return;
    }

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    // The victim: an ordinary host process with no relationship to the
    // sandbox that's about to attack it — not its parent, not its child,
    // not aware it exists, started before the sandbox even comes up.
    let mut victim = Command::new("sleep")
        .arg("300")
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn victim process");
    let victim_pid = victim.id();

    let project_root = std::env::temp_dir().join(format!(
        "devcroft-landlock-boundaries-signal-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();
    // Resolved rather than as spelled: the sandbox's policy is compiled
    // from this path, and macOS matches paths as written — `temp_dir()`
    // is `/var/folders/…`, a symlink, so a cwd named that way is denied
    // under a grant built from its target. No-op on Linux, where
    // Landlock works on inodes. See `docs/known-gaps.md`.
    let project_root = project_root.canonicalize().unwrap_or(project_root);
    let init = Command::new("flox")
        .arg("init")
        .current_dir(&project_root)
        .output()
        .unwrap();
    if !init.status.success() {
        eprintln!(
            "skipping: flox init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        let _ = victim.kill();
        let _ = victim.wait();
        return;
    }
    let install = Command::new("flox")
        .args(["install", "bash", "coreutils"])
        .current_dir(&project_root)
        .output()
        .unwrap();
    if !install.status.success() {
        eprintln!(
            "skipping: flox install bash coreutils failed: {}",
            String::from_utf8_lossy(&install.stderr)
        );
        let _ = victim.kill();
        let _ = victim.wait();
        return;
    }

    let sandbox_name = format!("e2elandlock1{}", std::process::id());
    let (manifest, _) = parse(&format!("[sandbox]\nname = {sandbox_name:?}\n")).unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    // Attack 1: kill() across the (namespace-shared) PID space.
    let out = Command::new(devcroft_bin)
        .arg("exec")
        .arg(&sandbox_name)
        .arg("--")
        .arg("sh")
        .arg("-c")
        .arg(format!("kill -TERM {victim_pid}"))
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "expected Landlock V6 signal-scoping to block a sandboxed `kill` against a process \
         outside the sandbox; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Operation not permitted"),
        "expected an EPERM-shaped denial naming the real reason, got stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        victim.try_wait().unwrap().is_none(),
        "victim must still be alive — the sandboxed kill must not have reached it"
    );

    // Attack 2: read the same victim's /proc entry directly (no
    // directory listing involved — Landlock mediates the path itself).
    //
    // Only where there is a `/proc` to mediate. On macOS the read fails
    // too, but with `No such file or directory`, which asserts nothing
    // about the policy — the second assertion below is specifically that
    // the *reason* is a denial and not an absence, so a host without
    // procfs cannot answer the question this attack asks. Skipping the
    // attack rather than the whole test keeps attack 1 (signal scoping,
    // which Seatbelt does enforce) measured here.
    if std::path::Path::new("/proc/self/cmdline").exists() {
        let out = Command::new(devcroft_bin)
            .arg("exec")
            .arg(&sandbox_name)
            .arg("--")
            .arg("sh")
            .arg("-c")
            .arg(format!("cat /proc/{victim_pid}/cmdline"))
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "expected /proc/<pid>/cmdline for a process outside the sandbox to be denied by \
             the default-deny filesystem policy; stdout={}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("Permission denied"),
            "expected a permission-denied-shaped failure, got stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    } else {
        eprintln!(
            "skipping attack 2: no procfs on this host, so an unreadable /proc entry \
                   would prove nothing"
        );
    }

    down(&sandbox_name).unwrap();
    let _ = victim.kill();
    let _ = victim.wait();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

#[test]
fn process_tier_blocks_raw_socket_bypass_of_deny_all_network() {
    if tooling_missing() {
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
        return;
    }

    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    let project_root = std::env::temp_dir().join(format!(
        "devcroft-landlock-boundaries-netdeny-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();
    // Resolved for the same reason as above (macOS path spellings).
    let project_root = project_root.canonicalize().unwrap_or(project_root);
    let init = Command::new("flox")
        .arg("init")
        .current_dir(&project_root)
        .output()
        .unwrap();
    if !init.status.success() {
        eprintln!(
            "skipping: flox init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        return;
    }

    let install = Command::new("flox")
        .args(["install", "bash"])
        .current_dir(&project_root)
        .output()
        .unwrap();
    if !install.status.success() {
        eprintln!(
            "skipping: flox install bash failed: {}",
            String::from_utf8_lossy(&install.stderr)
        );
        return;
    }

    // `network.default` defaults to `deny` with an empty allowlist
    // (config::Network's Default impl) — no `[network]` section needed.
    let sandbox_name = format!("e2elandlock2{}", std::process::id());
    let (manifest, _) = parse(&format!("[sandbox]\nname = {sandbox_name:?}\n")).unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    let (port, listener_handle) = spawn_forbidden_listener();
    let out = attempt_raw_connect(devcroft_bin, &sandbox_name, port);
    assert!(
        !out.status.success(),
        "expected a raw socket to be denied under network.default = deny; stdout={}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        stderr_is_a_kernel_level_denial(&String::from_utf8_lossy(&out.stderr)),
        "expected a kernel-level (Landlock network-scope) denial, not merely an unreached \
         proxy, got stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    down(&sandbox_name).unwrap();
    // Unblocks the listener thread so it doesn't outlive the test: connect
    // once from here, outside the sandbox, so `accept()` returns.
    let _ = std::net::TcpStream::connect(("127.0.0.1", port));
    let _ = listener_handle.join();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

#[test]
fn process_tier_blocks_raw_socket_bypass_of_a_domain_allowlist() {
    if tooling_missing() {
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
        return;
    }

    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    let project_root = std::env::temp_dir().join(format!(
        "devcroft-landlock-boundaries-netallow-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();
    // Resolved for the same reason as above (macOS path spellings).
    let project_root = project_root.canonicalize().unwrap_or(project_root);
    let init = Command::new("flox")
        .arg("init")
        .current_dir(&project_root)
        .output()
        .unwrap();
    if !init.status.success() {
        eprintln!(
            "skipping: flox init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        return;
    }

    let install = Command::new("flox")
        .args(["install", "bash"])
        .current_dir(&project_root)
        .output()
        .unwrap();
    if !install.status.success() {
        eprintln!(
            "skipping: flox install bash failed: {}",
            String::from_utf8_lossy(&install.stderr)
        );
        return;
    }

    // A domain that has nothing to do with the forbidden listener below —
    // this is the exact shape docs/decisions.md's original "Cooperative
    // network filtering" entry described as unstopped.
    let sandbox_name = format!("e2elandlock3{}", std::process::id());
    let (manifest, _) = parse(&format!(
        "[sandbox]\nname = {sandbox_name:?}\n[network]\ndefault = \"deny\"\nallow = [\"example.com\"]\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    // Confirm the allowlist actually took effect before attacking it —
    // otherwise a passing test here would prove nothing. `policy
    // --render` re-parses the manifest from disk (it's not sandbox
    // runtime state), so it needs both a devcroft.toml on disk and to
    // run from within the project directory — `up` above only took a
    // parsed `Manifest` in memory, same as every other test in this
    // suite, so the file needs writing here specifically for this check.
    std::fs::write(
        project_root.join("devcroft.toml"),
        format!(
            "[sandbox]\nname = {sandbox_name:?}\n[network]\ndefault = \"deny\"\nallow = [\"example.com\"]\n"
        ),
    )
    .unwrap();
    let render = Command::new(devcroft_bin)
        .arg("policy")
        .arg("--render")
        .current_dir(&project_root)
        .output()
        .unwrap();
    let render_stdout = String::from_utf8_lossy(&render.stdout);
    assert!(
        render_stdout.contains("example.com"),
        "expected the manifest's network allowlist to be reflected in the compiled policy, \
         got {render_stdout}"
    );

    let (port, listener_handle) = spawn_forbidden_listener();
    let out = attempt_raw_connect(devcroft_bin, &sandbox_name, port);
    assert!(
        !out.status.success(),
        "expected a raw socket to an IP unrelated to the allowlisted domain to be denied \
         even with network.allow set; stdout={}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        stderr_is_a_kernel_level_denial(&String::from_utf8_lossy(&out.stderr)),
        "expected a kernel-level denial, not a proxy the raw socket simply bypassed, \
         got stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    down(&sandbox_name).unwrap();
    let _ = std::net::TcpStream::connect(("127.0.0.1", port));
    let _ = listener_handle.join();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}
