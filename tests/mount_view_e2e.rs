//! `add-mount-isolation` task 2.1/2.2: a real toolchain build succeeds
//! inside `fleet::mount::construct_view`'s constructed, `pivot_root`ed
//! view, and the sandbox's own proxy socket stays reachable through it
//! (design.md M3) — formalizing what task 2.1's own live verification
//! already showed by hand for all three closure-tier providers.
//!
//! Uses `devbox-citytime-sample` specifically: zero crates.io
//! dependencies and no `[hook]`/`shell.init_hook` mechanism at all
//! (`provider::devbox` never runs one), so capturing its real activated
//! environment here needs no network and no hook-splitting machinery —
//! the same reason `devbox.rs`'s own tests build fixtures this way.
//! Deliberately does **not** cover the mount-plan-vs-Landlock daemon-
//! socket assertion — that is task 4.1/4.2's job, on the existing
//! `tests/unix_socket_not_mediated.rs`, not a new file.

use std::process::Command;

fn sample_root() -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/samples/devbox-citytime-sample"
    ))
}

/// Capability gate only — matches `tests/fleet_netns.rs`'s own discipline
/// of asking strictly less than what the tests below assert, so a
/// regression in the feature never reads as an unsupported host.
fn mount_namespaces_available() -> bool {
    Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("__mount_probe")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `devbox shellenv --pure`, evaluated the same two-step way
/// `provider::devbox`'s own capture does (`devbox.rs`'s own doc comment:
/// command substitution swallows a failing exit status) — returns the
/// activated `PATH`, or `None` if this host cannot resolve the sample's
/// closure at all (no devbox, no usable nix, no daemon).
fn devbox_activated_path() -> Option<String> {
    if Command::new("devbox").arg("version").output().is_err() {
        return None;
    }
    if !devcroft::provider::host_can_build_nix_closures() {
        return None;
    }

    let shellenv = Command::new("devbox")
        .arg("shellenv")
        .arg("--pure")
        .current_dir(sample_root())
        .output()
        .ok()?;
    if !shellenv.status.success() || shellenv.stdout.iter().all(u8::is_ascii_whitespace) {
        return None;
    }

    let dumped = Command::new("sh")
        .arg("-c")
        .arg(r#"eval "$1" && env -0"#)
        .arg("devcroft-test-capture")
        .arg(std::ffi::OsStr::new(
            std::str::from_utf8(&shellenv.stdout).ok()?,
        ))
        .current_dir(sample_root())
        .output()
        .ok()?;
    if !dumped.status.success() {
        return None;
    }

    String::from_utf8_lossy(&dumped.stdout)
        .split('\0')
        .filter_map(|entry| entry.split_once('='))
        .find(|(k, _)| *k == "PATH")
        .map(|(_, v)| v.to_string())
}

#[test]
fn a_real_devbox_build_succeeds_inside_the_constructed_view() {
    if !mount_namespaces_available() {
        eprintln!("skipping: this host cannot create unprivileged mount namespaces");
        return;
    }
    let Some(path) = devbox_activated_path() else {
        eprintln!("skipping: this host cannot resolve the devbox-citytime-sample closure");
        return;
    };
    let cargo = path
        .split(':')
        .map(|dir| std::path::Path::new(dir).join("cargo"))
        .find(|p| p.is_file());
    let Some(cargo) = cargo else {
        eprintln!("skipping: no `cargo` in the activated devbox environment");
        return;
    };

    let target = sample_root().join("target");
    let _ = std::fs::remove_dir_all(&target);
    // A name distinct from a real `.cargo` (the sample declares none —
    // zero dependencies, by design — so this test's own `CARGO_HOME`
    // must live *inside* the project root for a real build to have
    // anywhere writable within the view, same reasoning design.md
    // records for the pre-existing nix/devbox `$HOME/.cargo` gap).
    // Named and cleaned up distinctly so it never lingers as an
    // unexplained artifact.
    let cargo_home = sample_root().join(".cargo-test-e2e");
    let _ = std::fs::remove_dir_all(&cargo_home);

    let new_root =
        std::env::temp_dir().join(format!("devcroft-mount-view-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&new_root);
    std::fs::create_dir_all(&new_root).unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("__mount_view_probe")
        .arg(sample_root())
        .arg(&new_root)
        .arg("--provider-grant")
        .arg("/nix/store")
        .arg("--")
        .arg(&cargo)
        .arg("build")
        .env("CARGO_HOME", &cargo_home)
        .env("PATH", &path)
        .status()
        .unwrap();

    let _ = std::fs::remove_dir_all(&new_root);
    let _ = std::fs::remove_dir_all(&target);
    let _ = std::fs::remove_dir_all(&cargo_home);

    assert!(
        status.success(),
        "a real `cargo build` must succeed inside the constructed mount view, \
         using exactly the paths the compiled policy grants"
    );
}

/// A standalone connect probe, compiled once with the host's own `rustc`
/// (no provider closure, no dependencies) directly into the sample's
/// project root — so it is reachable from inside the view purely via the
/// ordinary project-root grant, with nothing extra to include.
fn build_connect_probe(target_sock: &std::path::Path) -> std::path::PathBuf {
    let src = sample_root().join("connect_probe.rs");
    std::fs::write(
        &src,
        format!(
            r#"fn main() {{
                match std::os::unix::net::UnixStream::connect({:?}) {{
                    Ok(_) => println!("CONNECTED"),
                    Err(e) => println!("REFUSED: {{e}}"),
                }}
            }}"#,
            target_sock
        ),
    )
    .unwrap();
    let bin = sample_root().join("connect_probe");
    let status = Command::new("rustc")
        .arg("-o")
        .arg(&bin)
        .arg(&src)
        .status()
        .unwrap();
    assert!(status.success(), "compiling the connect probe failed");
    let _ = std::fs::remove_file(&src);
    bin
}

/// design.md M3: the sandbox's own proxy socket stays reachable through
/// the view even though it lives in devcroft's baseline-denied state
/// directory, which nothing else in the view resolves into — and, absent
/// `--proxy-socket`, a real listening socket outside every grant stays
/// exactly as unreachable as any other ungranted path (spec: "Another
/// sandbox's proxy socket").
#[test]
fn the_proxy_socket_is_reachable_only_when_explicitly_included() {
    if !mount_namespaces_available() {
        eprintln!("skipping: this host cannot create unprivileged mount namespaces");
        return;
    }
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skipping: no `rustc` on this host to build the connect probe");
        return;
    }

    let sock_dir = std::env::temp_dir().join(format!("devcroft-proxy-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&sock_dir);
    std::fs::create_dir_all(&sock_dir).unwrap();
    let sock_path = sock_dir.join("proxy.sock");
    let listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    // Exactly one real accept expected: the "without" case never reaches
    // `connect()` at the OS level (the path itself doesn't resolve
    // inside the view), only the "with" case does.
    let accept_thread = std::thread::spawn(move || {
        let _ = listener.accept();
    });

    let probe_bin = build_connect_probe(&sock_path);

    let mut results = std::collections::HashMap::new();
    for (label, include) in [("without", false), ("with", true)] {
        let new_root = std::env::temp_dir().join(format!(
            "devcroft-mount-view-proxy-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&new_root);
        std::fs::create_dir_all(&new_root).unwrap();

        let mut cmd = Command::new(env!("CARGO_BIN_EXE_devcroft"));
        cmd.arg("__mount_view_probe")
            .arg(sample_root())
            .arg(&new_root);
        if include {
            cmd.arg("--proxy-socket").arg(&sock_path);
        }
        cmd.arg("--").arg(&probe_bin);

        let output = cmd.output().unwrap();
        let _ = std::fs::remove_dir_all(&new_root);
        results.insert(
            label,
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        );
    }

    let _ = std::fs::remove_file(&probe_bin);
    // Unblock `accept_thread` before removing the socket: the "without"
    // run above never connects, so the thread is still waiting.
    let _ = std::os::unix::net::UnixStream::connect(&sock_path);
    let _ = accept_thread.join();
    let _ = std::fs::remove_dir_all(&sock_dir);

    assert!(
        results["without"].starts_with("REFUSED"),
        "without --proxy-socket, the socket must be unreachable: got {:?}",
        results["without"]
    );
    assert_eq!(
        results["with"], "CONNECTED",
        "with --proxy-socket, the socket must be reachable"
    );
}
