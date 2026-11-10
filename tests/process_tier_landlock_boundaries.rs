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
//!
//! **Neutral surface** (`test-runtime-fixture`): all four are claims about
//! the backend, whatever the provider. They used to `flox init` + `flox
//! install bash coreutils` purely to have a `bash` and a `cat`; they now
//! run on whichever row is selected that has the probes they need
//! (`capabilities().boundary_probes`: bash, `cat`, and a `kill` that names
//! the errno). The Nix-free row has none of the three, and on it the attacks
//! fail without a reason, which is no evidence of a refusal.

#[path = "common/host_pid.rs"]
mod host_pid;

mod common;

use common::{ProviderFixture, for_each_row};
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down};
use std::net::TcpListener;
use std::process::{Command, Stdio};

fn supported() -> bool {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return false;
    }
    // SAFETY: each test here sets the same value before any thread reads it.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    true
}

/// Append to the row's own manifest, keeping its provider and name.
fn append_manifest(fx: &dyn ProviderFixture, extra: &str) {
    let path = fx.project_root().join("devcroft.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{text}\n{extra}")).unwrap();
}

fn bring_up(fx: &dyn ProviderFixture) -> StatePaths {
    let paths = StatePaths::new(fx.sandbox_name()).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_file(&paths.lifecycle_lock);
    assert_eq!(
        fx.bring_up(&UpOptions::default())
            .unwrap_or_else(|e| panic!("row {}: up failed: {e}", fx.name())),
        UpOutcome::Started
    );
    paths
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
/// raw-socket client, from the row's own environment: own-policy-baseline
/// excludes host toolchain access, and every row with `boundary_probes`
/// installs bash. Its stderr on failure names the real reason (`Permission denied` for a kernel-level Landlock deny vs
/// `Connection refused` for "nobody was listening"), which is exactly the
/// distinction these tests need to be meaningful — and exactly why a
/// missing `bash` would be a silent false pass: `devcroft exec`'s own
/// "keeper refused to spawn: Permission denied" also contains the
/// substring these tests check for, hence `assert_bash_runs`.
///
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

/// The control the old comment above warned was missing: `bash` runs in
/// this sandbox. Without it, a missing `bash` makes `exec` fail with
/// "keeper refused to spawn: Permission denied", which the denial check
/// would accept as the boundary working.
fn assert_bash_runs(devcroft_bin: &str, fx: &dyn ProviderFixture) {
    let out = Command::new(devcroft_bin)
        .args([
            "exec",
            fx.sandbox_name(),
            "--",
            "bash",
            "-c",
            "echo bash-ready",
        ])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "bash-ready",
        "row {}: bash must run in the sandbox before its /dev/tcp is evidence of anything: {out:?}",
        fx.name()
    );
}

#[test]
fn process_tier_blocks_cross_process_signals_and_proc_reads() {
    if !supported() {
        return;
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    for_each_row("landlocksig", |fx| {
        if !fx.capabilities().boundary_probes {
            eprintln!(
                "row {}: skipped (no `kill` that names the errno, no `cat`)",
                fx.name()
            );
            return;
        }
        // The victim: an ordinary host process with no relationship to the
        // sandbox that's about to attack it — not its parent, not its
        // child, not aware it exists, started before the sandbox comes up.
        // Reaped on every exit, a failed assertion included.
        struct Reap(std::process::Child);
        impl Drop for Reap {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut victim = Reap(
            Command::new("sleep")
                .arg("300")
                .stdout(Stdio::null())
                .spawn()
                .expect("spawn victim process"),
        );
        let victim_pid = victim.0.id();

        let paths = bring_up(fx);

        // Two regimes, and each has its own correct answer. With a PID
        // namespace of its own (`add-up-pid-namespace`), the victim has no
        // number inside the sandbox at all: `kill` finds no such process
        // and `/proc` has no entry, which is the stronger property. Sharing
        // the host's (a host that masks `/proc`, or macOS), the victim is
        // addressable and Landlock V6's signal scoping and the default-deny
        // `/proc` refuse it. Either way the victim must survive.
        let own_namespace = host_pid::has_own_pid_namespace(&paths.pidfile);
        let (signal_refusal, read_refusal) = if own_namespace {
            ("No such process", "No such file or directory")
        } else {
            ("Operation not permitted", "Permission denied")
        };

        // Attack 1: kill() across the PID space.
        let out = Command::new(devcroft_bin)
            .args(["exec", fx.sandbox_name(), "--", "sh", "-c"])
            .arg(format!("kill -TERM {victim_pid}"))
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "row {}: expected a sandboxed `kill` against a process outside the sandbox to \
             fail; stdout={} stderr={}",
            fx.name(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(signal_refusal),
            "row {}: expected {signal_refusal:?} (own PID namespace: {own_namespace}), got \
             stderr={}",
            fx.name(),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            victim.0.try_wait().unwrap().is_none(),
            "victim must still be alive — the sandboxed kill must not have reached it"
        );

        // Attack 2: read the same victim's /proc entry directly (no
        // directory listing involved — Landlock mediates the path itself).
        //
        // Linux-only, because the *attack* is: there is no `/proc` on
        // macOS for a sandbox to read, so the read fails `ENOENT` rather
        // than being refused, and asserting a denial there would be
        // asserting the absence of a filesystem rather than the presence
        // of a boundary. Attack 1 above holds on both — Seatbelt's
        // `(allow signal (target same-sandbox))` is the macOS counterpart
        // to Landlock V6 signal scoping.
        #[cfg(target_os = "linux")]
        {
            let out = Command::new(devcroft_bin)
                .args(["exec", fx.sandbox_name(), "--", "sh", "-c"])
                .arg(format!("cat /proc/{victim_pid}/cmdline"))
                .output()
                .unwrap();
            assert!(
                !out.status.success(),
                "row {}: expected /proc/<pid>/cmdline for a process outside the sandbox to \
                 be refused; stdout={}",
                fx.name(),
                String::from_utf8_lossy(&out.stdout)
            );
            assert!(
                String::from_utf8_lossy(&out.stderr).contains(read_refusal),
                "row {}: expected {read_refusal:?} (own PID namespace: {own_namespace}), got \
                 stderr={}",
                fx.name(),
                String::from_utf8_lossy(&out.stderr)
            );
        }

        down(fx.sandbox_name()).unwrap();
    });
}

#[test]
fn process_tier_blocks_raw_socket_bypass_of_deny_all_network() {
    if !supported() {
        return;
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    for_each_row("landlocknetdeny", |fx| {
        if !fx.capabilities().boundary_probes {
            eprintln!("row {}: skipped (no bash for /dev/tcp)", fx.name());
            return;
        }
        // `network.default` defaults to `deny` with an empty allowlist
        // (config::Network's Default impl), which is what every row writes.
        bring_up(fx);
        assert_bash_runs(devcroft_bin, fx);

        let (port, listener_handle) = spawn_forbidden_listener();
        let out = attempt_raw_connect(devcroft_bin, fx.sandbox_name(), port);
        assert!(
            !out.status.success(),
            "row {}: expected a raw socket to be denied under network.default = deny; \
             stdout={}",
            fx.name(),
            String::from_utf8_lossy(&out.stdout)
        );
        assert!(
            stderr_is_a_kernel_level_denial(&String::from_utf8_lossy(&out.stderr)),
            "row {}: expected a kernel-level (Landlock network-scope) denial, not merely an \
             unreached proxy, got stderr={}",
            fx.name(),
            String::from_utf8_lossy(&out.stderr)
        );

        down(fx.sandbox_name()).unwrap();
        // Unblocks the listener thread so it doesn't outlive the test:
        // connect once from here, outside the sandbox, so `accept()`
        // returns.
        let _ = std::net::TcpStream::connect(("127.0.0.1", port));
        let _ = listener_handle.join();
    });
}

#[test]
fn process_tier_blocks_raw_socket_bypass_of_a_domain_allowlist() {
    if !supported() {
        return;
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    for_each_row("landlocknetallow", |fx| {
        // `policy --render` below is the CLI resolving the row's provider.
        if !(fx.capabilities().boundary_probes && fx.capabilities().cli_drivable) {
            eprintln!(
                "row {}: skipped (needs bash for /dev/tcp and a CLI-drivable row)",
                fx.name()
            );
            return;
        }
        // A domain that has nothing to do with the forbidden listener
        // below — the exact shape docs/decisions.md's original "Cooperative
        // network filtering" entry described as unstopped.
        append_manifest(
            fx,
            "[network]\ndefault = \"deny\"\nallow = [\"example.com\"]\n",
        );
        bring_up(fx);
        assert_bash_runs(devcroft_bin, fx);

        // Confirm the allowlist actually took effect before attacking it —
        // otherwise a passing test here would prove nothing.
        let render = Command::new(devcroft_bin)
            .args(["policy", "--render"])
            .current_dir(fx.project_root())
            .output()
            .unwrap();
        let render_stdout = String::from_utf8_lossy(&render.stdout);
        assert!(
            render_stdout.contains("example.com"),
            "row {}: expected the manifest's network allowlist in the compiled policy, got \
             {render_stdout}",
            fx.name()
        );

        let (port, listener_handle) = spawn_forbidden_listener();
        let out = attempt_raw_connect(devcroft_bin, fx.sandbox_name(), port);
        assert!(
            !out.status.success(),
            "row {}: expected a raw socket to an IP unrelated to the allowlisted domain to be \
             denied even with network.allow set; stdout={}",
            fx.name(),
            String::from_utf8_lossy(&out.stdout)
        );
        assert!(
            stderr_is_a_kernel_level_denial(&String::from_utf8_lossy(&out.stderr)),
            "row {}: expected a kernel-level denial, not a proxy the raw socket simply \
             bypassed, got stderr={}",
            fx.name(),
            String::from_utf8_lossy(&out.stderr)
        );

        down(fx.sandbox_name()).unwrap();
        let _ = std::net::TcpStream::connect(("127.0.0.1", port));
        let _ = listener_handle.join();
    });
}
