//! **`ssh.forward_agent` does what it says** (`own-sandbox-environment` D5,
//! and `add-mvp-core`'s ssh spec scenario "Agent forwarding is off by
//! default").
//!
//! The key parsed and validated since the MVP and **nothing implemented it**,
//! while `SSH_AUTH_SOCK` reached every sandbox anyway through plain
//! environment inheritance — so the spec's requirement was satisfied by
//! accident at best and not at all at worst. Closing the environment made both
//! halves fixable in one place.
//!
//! The *off* case is the one that is a security property rather than a
//! feature, and it needs nothing installed, so it runs wherever a sandbox can
//! start. The *on* case needs `openssh` in the closure and a live agent on the
//! host, and skips loudly without them.

use std::process::Command;

fn project(name: &str, manifest: &str, with_openssh: bool) -> Option<std::path::PathBuf> {
    let root =
        std::env::temp_dir().join(format!("devcroft-agentfwd-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).ok()?;
    if !Command::new("flox")
        .arg("init")
        .current_dir(&root)
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return None;
    }
    let pkgs: &[&str] = if with_openssh {
        &["coreutils", "openssh"]
    } else {
        &["coreutils"]
    };
    for p in pkgs {
        if !Command::new("flox")
            .args(["install", p])
            .current_dir(&root)
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return None;
        }
    }
    std::fs::write(root.join("devcroft.toml"), manifest).ok()?;
    Some(root)
}

fn usable_host() -> bool {
    devcroft::policy::backend_supported()
        && Command::new("flox").arg("--version").output().is_ok()
        && devcroft::provider::host_can_build_nix_closures()
}

#[test]
fn without_the_key_no_agent_socket_reaches_the_sandbox() {
    if !usable_host() {
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let bin = env!("CARGO_BIN_EXE_devcroft");
    let name = format!("agentoff{}", std::process::id());
    let Some(root) = project("off", &format!("[sandbox]\nname = \"{name}\"\n"), false) else {
        eprintln!("skipping: could not prepare a flox project");
        return;
    };

    assert!(
        Command::new(bin)
            .arg("up")
            .current_dir(&root)
            .output()
            .unwrap()
            .status
            .success(),
        "`up` failed with every precondition already checked"
    );
    let out = Command::new(bin)
        .args(["exec", "--", "env"])
        .current_dir(&root)
        .output()
        .unwrap();
    let env_dump = String::from_utf8_lossy(&out.stdout).into_owned();

    let _ = Command::new(bin).arg("down").current_dir(&root).output();
    let paths = devcroft::lifecycle::StatePaths::new(&name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        !env_dump.contains("SSH_AUTH_SOCK"),
        "without `ssh.forward_agent = true`, no agent socket may reach the \
         sandbox — this used to arrive by inheritance whatever the key said. \
         env was:\n{env_dump}"
    );
}

#[test]
fn with_the_key_the_host_agent_is_reachable() {
    if !usable_host() {
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
        return;
    }
    let Ok(host_sock) = std::env::var("SSH_AUTH_SOCK") else {
        eprintln!("skipping: no SSH agent running on this host (SSH_AUTH_SOCK unset)");
        return;
    };
    // An agent with no identities would make the assertion below vacuous: the
    // command succeeds and lists nothing, which is indistinguishable from a
    // sandbox that reached a different agent.
    let host_keys = Command::new("ssh-add").arg("-l").output();
    if !host_keys.as_ref().is_ok_and(|o| o.status.success()) {
        eprintln!("skipping: the host agent holds no identities, so this would assert nothing");
        return;
    }
    let host_listing = String::from_utf8_lossy(&host_keys.unwrap().stdout).into_owned();

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let bin = env!("CARGO_BIN_EXE_devcroft");
    let name = format!("agenton{}", std::process::id());
    let Some(root) = project(
        "on",
        &format!("[sandbox]\nname = \"{name}\"\n[ssh]\nforward_agent = true\n"),
        true,
    ) else {
        eprintln!("skipping: could not prepare a flox project with openssh");
        return;
    };

    assert!(
        Command::new(bin)
            .arg("up")
            .current_dir(&root)
            .output()
            .unwrap()
            .status
            .success(),
        "`up` failed with every precondition already checked"
    );
    let listed = Command::new(bin)
        .args(["exec", "--", "ssh-add", "-l"])
        .current_dir(&root)
        .output()
        .unwrap();
    let inside = String::from_utf8_lossy(&listed.stdout).into_owned();
    let inside_err = String::from_utf8_lossy(&listed.stderr).into_owned();

    let _ = Command::new(bin).arg("down").current_dir(&root).output();
    let paths = devcroft::lifecycle::StatePaths::new(&name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&root);

    // The same identities the host agent holds, which is what distinguishes
    // "reached the agent" from "the command ran".
    assert_eq!(
        inside.trim(),
        host_listing.trim(),
        "the sandbox must reach the host's own agent. \
         `Operation not permitted` here means the socket grant is missing on \
         the *network* axis, not the filesystem one: macOS classifies the \
         AF_UNIX connect as network activity, so `network.default = \"deny\"` \
         refuses it even with the path granted read-write (measured). \
         stderr: {inside_err}. host socket was {host_sock}"
    );
}
