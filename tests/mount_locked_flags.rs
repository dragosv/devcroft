//! A read-only grant on a `nosuid`/`nodev`/`noexec` mount must not break
//! the view.
//!
//! Inside a user namespace, an inherited mount's `nosuid`, `nodev`,
//! `noexec` and atime flags are locked: a remount may add restrictions
//! but not drop one, and a remount that does not name a flag drops it.
//! `construct_view`'s read-only remount named only `MS_RDONLY`, so every
//! `filesystem.read` of such a path failed `up` with a bare `EPERM`. Found
//! through fleet, whose tests read `/proc`; then measured on `/dev/shm`.
//! `/usr/share`, on a mount without those flags, always worked, which is
//! why nothing had caught it.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn mount_namespaces_available() -> bool {
    Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("__mount_probe")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A project whose manifest reads `read`, and an empty view root.
fn project(tag: &str, read: &str) -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("devcroft-locked-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (project, view) = (base.join("project"), base.join("view"));
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&view).unwrap();
    std::fs::write(
        project.join("devcroft.toml"),
        format!(
            "[sandbox]\nname = \"locked\"\n\n[env]\nprovider = \"flox\"\n\n\
             [filesystem]\nallow = [\".\"]\nread = [{read:?}]\n"
        ),
    )
    .unwrap();
    (project, view)
}

/// Build the view with `read` granted, run `script` in it, return
/// (exit code, combined output).
fn in_view(tag: &str, read: &str, script: &str) -> (i32, String) {
    let (project, view) = project(tag, read);
    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("__mount_view_probe")
        .arg(&project)
        .arg(&view)
        .args([
            "--provider-grant",
            "/usr",
            "--",
            "/usr/bin/sh",
            "-c",
            script,
        ])
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(project.parent().unwrap());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap_or(-1), text)
}

fn mount_options(path: &str) -> Option<String> {
    let info = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    info.lines()
        .map(|l| l.split(' ').collect::<Vec<_>>())
        .find(|f| f.get(4) == Some(&path))
        .map(|f| f[5].to_owned())
}

#[test]
fn a_read_grant_on_proc_builds_the_view() {
    if !mount_namespaces_available() {
        eprintln!("skipping: this host cannot create unprivileged mount namespaces");
        return;
    }
    // /proc is nosuid,nodev,noexec on every Linux host.
    let (code, out) = in_view("proc", "/proc", "echo built");
    assert_eq!(code, 0, "a read grant on /proc broke the view: {out}");
    assert!(out.contains("built"), "{out}");
}

#[test]
fn a_read_grant_on_a_nosuid_tmpfs_is_read_only_and_keeps_its_flags() {
    if !mount_namespaces_available() {
        eprintln!("skipping: this host cannot create unprivileged mount namespaces");
        return;
    }
    let Some(opts) = mount_options("/dev/shm").filter(|o| o.contains("nosuid")) else {
        eprintln!("skipping: this host has no /dev/shm mount with nosuid to test against");
        return;
    };
    assert!(Path::new("/dev/shm").is_dir(), "{opts}");
    let (code, out) = in_view(
        "shm",
        "/dev/shm",
        "grep ' /dev/shm ' /proc/self/mountinfo | head -1 | cut -d' ' -f6; \
         touch /dev/shm/devcroft-locked-probe 2>&1; true",
    );
    assert_eq!(code, 0, "a read grant on /dev/shm broke the view: {out}");
    let first = out.lines().next().unwrap_or_default();
    // Read-only, and nothing the kernel had locked was dropped.
    assert!(first.split(',').any(|f| f == "ro"), "{out}");
    assert!(first.contains("nosuid"), "{out}");
    assert!(
        out.contains("Read-only file system"),
        "the grant was writable: {out}"
    );
    assert!(!Path::new("/dev/shm/devcroft-locked-probe").exists());
}
