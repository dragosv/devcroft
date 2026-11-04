//! `add-agent-interaction` task group 2: a plain sandbox — not a fleet
//! agent — needing attention, through the real binary against a real devenv
//! sandbox. `fleet_commands.rs` covers the same report for fleet agents.
//!
//! The agent's side is `devcroft agent attention`, run **inside** the
//! sandbox by its absolute path: the keeper's own binary is already granted
//! read+exec in every sandbox (`with_keeper_exe_grant`), so this needs
//! nothing the policy does not already hold. It is not on the sandbox's
//! `PATH`. The binary, not its directory: the last test here holds that.
//!
//! Guards on the capability, never on the binary (CLAUDE.md).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn devcroft_bin() -> &'static str {
    env!("CARGO_BIN_EXE_devcroft")
}

fn capable_host() -> bool {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return false;
    }
    let devenv = Command::new("devenv")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !devenv || !devcroft::provider::host_can_build_nix_closures() {
        eprintln!("skipping: devenv or a usable nix store not on this host");
        return false;
    }
    true
}

struct Sandbox {
    root: PathBuf,
    name: String,
}

impl Sandbox {
    /// A devenv project with a shell and nothing else, under the default
    /// deny-by-default policy: no `[filesystem]` grant, no network.
    fn new(tag: &str) -> Self {
        unsafe {
            std::env::set_var("DEVCROFT_KEEPER_EXE", devcroft_bin());
        }
        // Short, under `/tmp`: the reason `devenv_provider_e2e` gives.
        let root = PathBuf::from(format!("/tmp/dca-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/devenv-claude-sample");
        std::fs::copy(sample.join("devenv.lock"), root.join("devenv.lock")).unwrap();
        std::fs::write(
            root.join("devenv.yaml"),
            "inputs:\n  nixpkgs:\n    url: github:cachix/devenv-nixpkgs/rolling\n",
        )
        .unwrap();
        std::fs::write(
            root.join("devenv.nix"),
            "{ pkgs, ... }: { packages = [ pkgs.bash pkgs.coreutils ]; }\n",
        )
        .unwrap();
        let name = format!("dcattn{tag}{}", std::process::id());
        std::fs::write(
            root.join("devcroft.toml"),
            format!("[sandbox]\nname = {name:?}\n\n[env]\nprovider = \"devenv\"\n"),
        )
        .unwrap();
        Sandbox { root, name }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(devcroft_bin())
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "devcroft {args:?} failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// `devcroft agent <args>`, run inside the sandbox as its agent would.
    fn agent(&self, args: &[&str]) {
        let mut argv = vec!["exec", "--", devcroft_bin(), "agent"];
        argv.extend_from_slice(args);
        self.ok(&argv);
    }

    /// This sandbox's `ps` row, split on tabs.
    fn ps_row(&self) -> Vec<String> {
        row(&ps(), &self.name)
    }

    fn status(&self) -> String {
        self.ok(&["status"])
    }

    fn keeper_pid(&self) -> libc::pid_t {
        let paths = devcroft::lifecycle::StatePaths::new(&self.name).unwrap();
        match devcroft::lifecycle::health(&paths).unwrap() {
            devcroft::lifecycle::Health::Healthy(pid) => pid,
            other => panic!("expected a healthy keeper, got {other:?}"),
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = self.run(&["rm", "--yes"]);
        if let Ok(paths) = devcroft::lifecycle::StatePaths::new(&self.name) {
            let _ = std::fs::remove_dir_all(paths.root);
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// One `ps`, the whole listing.
fn ps() -> String {
    let out = Command::new(devcroft_bin())
        .arg("ps")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn row(listing: &str, name: &str) -> Vec<String> {
    listing
        .lines()
        .find(|l| l.split('\t').next() == Some(name))
        .unwrap_or_else(|| panic!("no `ps` row for {name}:\n{listing}"))
        .split('\t')
        .map(str::to_owned)
        .collect()
}

/// Tasks 2.1, 2.3 and 1.3–1.5: an agent raises attention under the default
/// policy and exits; the sandbox goes on reporting it, healthy *and* needing
/// attention, in `ps` and with its message in `status`; the agent clears it
/// itself. The policy is the same before and after: raising it needed no
/// grant.
#[test]
fn an_agent_raises_attention_from_inside_and_it_outlives_the_agent() {
    if !capable_host() {
        return;
    }
    let s = Sandbox::new("raise");
    s.ok(&["up"]);
    let policy = s.ok(&["policy", "--render"]);
    assert_eq!(s.ps_row()[3], "-", "nothing raised yet");
    assert!(!s.status().contains("attention"), "{}", s.status());

    // An escape sequence in the message: shown, not obeyed.
    s.agent(&["attention", "which database?\x1b[2J"]);

    let row = s.ps_row();
    assert!(row[1].starts_with("healthy"), "{row:?}");
    assert_eq!(row[3], "attention", "{row:?}");
    let status = s.status();
    assert!(status.contains("keeper: healthy"), "{status}");
    assert!(
        status.contains("attention: which database?\\u{1b}[2J (reported "),
        "the message, verbatim and escaped:\n{status}"
    );
    assert_eq!(s.ok(&["policy", "--render"]), policy);

    s.agent(&["attention", "--clear"]);
    assert_eq!(s.ps_row()[3], "-");
    assert!(!s.status().contains("attention"));
}

/// Task 2.2: among several sandboxes, the one needing attention is found
/// from the listing alone, without asking any sandbox individually.
#[test]
fn one_ps_names_the_one_sandbox_that_needs_attention() {
    if !capable_host() {
        return;
    }
    let (quiet, waiting) = (Sandbox::new("quiet"), Sandbox::new("wait"));
    quiet.ok(&["up"]);
    waiting.ok(&["up"]);
    waiting.agent(&["attention", "need a review"]);

    let listing = ps();
    assert_eq!(row(&listing, &waiting.name)[3], "attention", "{listing}");
    assert_eq!(row(&listing, &quiet.name)[3], "-", "{listing}");
}

/// Task 2.4: `down` keeps it, as it keeps every other record; `rm` and
/// `up --recreate` each leave a sandbox that does not start needing it.
#[test]
fn teardown_clears_attention_and_down_keeps_it() {
    if !capable_host() {
        return;
    }
    let s = Sandbox::new("tear");
    s.ok(&["up"]);
    s.agent(&["attention", "still here?"]);

    s.ok(&["down"]);
    let row = s.ps_row();
    assert_eq!(
        (row[1].as_str(), row[3].as_str()),
        ("not running", "attention")
    );
    s.ok(&["up"]);
    assert_eq!(s.ps_row()[3], "attention", "a restart is not a teardown");

    s.ok(&["up", "--recreate", "--yes"]);
    assert_eq!(s.ps_row()[3], "-", "a recreated sandbox starts without it");

    s.agent(&["attention", "again"]);
    s.ok(&["rm", "--yes"]);
    s.ok(&["up"]);
    assert_eq!(s.ps_row()[3], "-", "nor does one made after `rm`");
}

/// Task 2.5: a keeper that died is reported as the failure it is — never
/// as needing attention — and attention raised before it died is still
/// reported beside the failure, neither standing in for the other.
#[test]
fn a_dead_keeper_is_a_failure_not_attention() {
    if !capable_host() {
        return;
    }
    let s = Sandbox::new("dead");
    s.ok(&["up"]);
    unsafe {
        libc::kill(s.keeper_pid(), libc::SIGKILL);
    }
    std::thread::sleep(std::time::Duration::from_millis(300));

    let row = s.ps_row();
    assert_eq!(
        (row[1].as_str(), row[3].as_str()),
        ("stale", "-"),
        "{row:?}"
    );
    let status = s.status();
    assert!(status.contains("keeper: stale"), "{status}");
    assert!(!status.contains("attention"), "{status}");

    // Raised, then the keeper died: both facts, side by side.
    s.ok(&["up"]);
    s.agent(&["attention", "blocked"]);
    unsafe {
        libc::kill(s.keeper_pid(), libc::SIGKILL);
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    let row = s.ps_row();
    assert_eq!(
        (row[1].as_str(), row[3].as_str()),
        ("stale", "attention"),
        "{row:?}"
    );
}

/// What makes the agent's side need no grant is the keeper's own grant, and
/// that grant is the binary alone. It was its directory, which put every
/// file beside devcroft — all of `~/.cargo/bin` after a `cargo install` —
/// in every sandbox. `spike` sits beside `devcroft` in the build directory,
/// so it stands in for those.
#[test]
fn the_keeper_binary_is_granted_and_its_neighbours_are_not() {
    if !capable_host() {
        return;
    }
    let s = Sandbox::new("exe");
    s.ok(&["up"]);
    let spike = env!("CARGO_BIN_EXE_spike");
    let dir = Path::new(devcroft_bin()).parent().unwrap();
    assert_eq!(
        Path::new(spike).parent().unwrap(),
        dir,
        "spike is a neighbour"
    );

    s.ok(&["exec", "--", devcroft_bin(), "--version"]);
    let out = s.run(&["exec", "--", spike]);
    assert!(!out.status.success(), "a neighbour ran: {out:?}");
    let listed = s.run(&["exec", "--", "sh", "-c", &format!("ls {}", dir.display())]);
    assert!(
        !listed.status.success(),
        "the directory is readable: {listed:?}"
    );
}
