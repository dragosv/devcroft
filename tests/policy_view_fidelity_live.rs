//! `fix-silent-policy-degradation` 4.3, the one that matters: from inside a
//! real session, `why` for the provider's store returns `ALLOWED` with
//! origin `provider:nix`, the host's answer. Before, inside a sandbox the
//! state dir does not exist, and it answered `DENIED`, exit 0.
//!
//! A real sandbox on a copy of `samples/nix-flake-sample` under its own
//! name, so it cannot collide with one a developer has up. Guarded on the
//! capability, never on a binary being on `PATH`: `nix --version` passes on
//! a host whose store cannot build anything.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn devcroft(project: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .current_dir(project)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap()
}

struct Sandbox {
    project: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = devcroft(&self.project, &["rm", "--yes"]);
        let _ = std::fs::remove_dir_all(&self.project);
    }
}

#[test]
fn why_inside_a_real_session_answers_as_the_host_does() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    if Command::new("nix").arg("--version").output().is_err()
        || !devcroft::provider::host_can_build_nix_closures()
    {
        eprintln!("skipping: no nix, or no reachable Nix store");
        return;
    }
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/nix-flake-sample");
    let name = format!("fidelitylive{}", std::process::id());
    let project = std::env::temp_dir().join(&name);
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();
    for file in ["flake.nix", "flake.lock", "Cargo.toml", "Cargo.lock"] {
        std::fs::copy(sample.join(file), project.join(file)).unwrap();
    }
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::copy(sample.join("src/main.rs"), project.join("src/main.rs")).unwrap();
    std::fs::write(
        project.join("devcroft.toml"),
        format!("[sandbox]\nname = \"{name}\"\n\n[env]\nprovider = \"nix\"\n"),
    )
    .unwrap();
    let sandbox = Sandbox { project };
    // A git repository that never ran `init`: `up` now writes into
    // `.devcroft/` for every sandbox, and must not leave the tree dirty
    // (design.md D6, task 3.2).
    let git = |args: &[&str]| {
        Command::new("git")
            .current_dir(&sandbox.project)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap()
    };
    git(&["init", "--quiet"]);
    git(&["add", "."]);
    git(&["commit", "--quiet", "-m", "init"]);

    let up = devcroft(&sandbox.project, &["up"]);
    assert!(up.status.success(), "{up:?}");
    let status = git(&["status", "--porcelain"]);
    assert_eq!(
        String::from_utf8_lossy(&status.stdout),
        "",
        "`up` left the working tree dirty"
    );
    let host = devcroft(
        &sandbox.project,
        &["why", "--path", "/nix/store", "--op", "read"],
    );
    assert_eq!(
        String::from_utf8_lossy(&host.stdout),
        "ALLOWED\nallowed by rule provider:nix\n",
        "{host:?}"
    );

    // The same command, unchanged, from inside a session (design.md D4: no
    // flag that only an informed caller would pass). Through the shell
    // devcroft resolved, by absolute path: the closure need not put one on
    // `PATH`.
    let meta =
        devcroft::lifecycle::read_meta(&devcroft::lifecycle::StatePaths::new(&name).unwrap().meta)
            .unwrap()
            .unwrap();
    let shell = meta.shell.expect("up records the resolved shell");
    let in_session = |command: &str| {
        let script = format!(
            "cd {} && {} {command}",
            sandbox.project.display(),
            env!("CARGO_BIN_EXE_devcroft")
        );
        devcroft(&sandbox.project, &["exec", "--", &shell, "-c", &script])
    };
    let inside = in_session("why --path /nix/store --op read");
    assert_eq!(
        String::from_utf8_lossy(&inside.stdout),
        "ALLOWED\nallowed by rule provider:nix\n",
        "inside the sandbox: {inside:?}"
    );
    assert_eq!(inside.status.code(), Some(0));

    // And the two renderings are the same policy (task 4.4). Before the
    // host preferred `up`'s own artifact, they were not: the host's
    // reconstruction from the record lacked the keeper binary's grant,
    // which the backend does get, so it was the host that was incomplete.
    let host_render = devcroft(&sandbox.project, &["policy", "--render"]);
    let inside_render = in_session("policy --render");
    assert!(host_render.status.success() && inside_render.status.success());
    assert_eq!(
        String::from_utf8_lossy(&inside_render.stdout),
        String::from_utf8_lossy(&host_render.stdout),
        "the in-sandbox rendering differs from the host's"
    );
    assert!(String::from_utf8_lossy(&host_render.stdout).contains("provider:nix"));
}
