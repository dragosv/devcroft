//! `add-mount-isolation` adversarial-review findings: `fleet::mount::
//! construct_view`'s handling of a `/tmp` grant had two real bugs, both
//! found by external review and reproduced live before being fixed —
//! this file is what keeps them fixed.
//!
//! 1. **Ordering.** `/tmp` used to be mounted *after* the generic grants
//!    loop. A project root physically under `/tmp` on the host
//!    (`mktemp`-style worktrees, ephemeral CI directories) got its own
//!    bind mount created first, then the private `/tmp` tmpfs mounted
//!    directly on top of it — hiding it. `up` failed with a bare
//!    `ENOENT` for any such project that also granted `/tmp`.
//! 2. **Mode.** The private `/tmp` tmpfs was always writable regardless
//!    of whether the manifest granted it via `filesystem.allow`
//!    (`ReadWrite`) or `filesystem.read` (`Read`) — `policy --render`
//!    could show `Read` while the constructed view stayed writable.
//!
//! The fix for both is `construct_view`'s three-phase `/tmp` handling:
//! mount it early (writable, so nested grants can create their own
//! mount points under it), run the grants loop, then finalize its mode
//! with a *non-recursive* remount — recursive would have silently
//! dragged a nested grant's own, possibly more permissive mode (a
//! project root granted `ReadWrite` even while `/tmp` itself is
//! `Read`) down to read-only too.

use std::process::Command;

fn mount_namespaces_available() -> bool {
    Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("__mount_probe")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A standalone, dependency-free write probe compiled with the host's own
/// `rustc` — these tests are about mount/remount semantics, not toolchain
/// resolution, so nothing here needs a provider or `/nix/store` grant at
/// all.
fn build_write_probe(dir: &std::path::Path) -> std::path::PathBuf {
    let src = dir.join("write_probe.rs");
    std::fs::write(
        &src,
        r#"fn main() {
            let target = std::env::args().nth(1).unwrap();
            match std::fs::write(&target, b"x") {
                Ok(_) => println!("WROTE"),
                Err(e) => println!("REFUSED: {e}"),
            }
        }"#,
    )
    .unwrap();
    let bin = dir.join("write_probe");
    let status = Command::new("rustc")
        .arg("-o")
        .arg(&bin)
        .arg(&src)
        .status()
        .unwrap();
    assert!(status.success(), "compiling the write probe failed");
    bin
}

struct ScratchProject {
    root: std::path::PathBuf,
}

impl ScratchProject {
    /// Deliberately created *under* `/tmp` itself — that placement is
    /// what finding 1 needs to reproduce, and every other sample fixture
    /// in this repo lives outside `/tmp`, which is exactly why the bug
    /// stayed uncaught.
    fn new(name: &str, manifest_filesystem: &str) -> Self {
        let root = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("devcroft.toml"),
            format!(
                "[sandbox]\nname = \"{name}\"\n[env]\nprovider = \"flox\"\n{manifest_filesystem}\n"
            ),
        )
        .unwrap();
        ScratchProject { root }
    }
}

impl Drop for ScratchProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn run_probe(project_root: &std::path::Path, args: &[&std::ffi::OsStr]) -> std::process::Output {
    // Unique per call, not just per test: a test that calls this twice
    // (the combined nested/top-level case) needs two distinct scratch
    // roots, not one reused (and raced) between them.
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let call_id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let new_root = std::env::temp_dir().join(format!(
        "devcroft-tmp-ordering-view-{}-{}",
        std::process::id(),
        call_id
    ));
    let _ = std::fs::remove_dir_all(&new_root);
    std::fs::create_dir_all(&new_root).unwrap();

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_devcroft"));
    cmd.arg("__mount_view_probe")
        .arg(project_root)
        .arg(&new_root);
    cmd.arg("--");
    cmd.args(args);
    let out = cmd.output().unwrap();
    let _ = std::fs::remove_dir_all(&new_root);
    out
}

/// Finding 1: a project root under `/tmp`, with `/tmp` also granted,
/// must still be constructible and reachable — not shadowed by `/tmp`'s
/// own private tmpfs.
#[test]
fn a_project_root_nested_under_tmp_is_reachable_when_tmp_is_also_granted() {
    if !mount_namespaces_available() {
        eprintln!("skipping: this host cannot create unprivileged mount namespaces");
        return;
    }

    let project = ScratchProject::new(
        "tmp-ordering-nested",
        "[filesystem]\nallow = [\".\", \"/tmp\"]\n",
    );
    std::fs::write(project.root.join("marker"), b"present").unwrap();
    let probe = build_write_probe(&project.root);

    // Writing inside the project root proves two things at once: the
    // path resolved at all (finding 1) and it did so with the project
    // root's own ReadWrite grant, not /tmp's.
    let target = project.root.join("written-by-probe");
    let out = run_probe(&project.root, &[probe.as_os_str(), target.as_os_str()]);

    assert!(
        out.status.success(),
        "constructing/running inside a project root nested under /tmp must succeed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "WROTE");
}

/// Finding 2, the `ReadWrite` control: `/tmp` granted via
/// `filesystem.allow` must stay writable — confirms the fix didn't
/// overcorrect into refusing everything.
#[test]
fn tmp_granted_read_write_is_writable_at_the_mount_level() {
    if !mount_namespaces_available() {
        eprintln!("skipping: this host cannot create unprivileged mount namespaces");
        return;
    }

    let project = ScratchProject::new(
        "tmp-ordering-rw",
        "[filesystem]\nallow = [\".\", \"/tmp\"]\n",
    );
    let probe = build_write_probe(&project.root);
    let target = std::path::Path::new("/tmp/devcroft-tmp-ordering-rw-marker");

    let out = run_probe(&project.root, &[probe.as_os_str(), target.as_os_str()]);

    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "WROTE",
        "filesystem.allow = [\"/tmp\"] must remain writable at the mount level"
    );
}

/// Finding 2, the actual bug: `/tmp` granted via `filesystem.read` must
/// be genuinely read-only at the mount level, not just in the rendered
/// policy.
#[test]
fn tmp_granted_read_only_is_read_only_at_the_mount_level() {
    if !mount_namespaces_available() {
        eprintln!("skipping: this host cannot create unprivileged mount namespaces");
        return;
    }

    let project = ScratchProject::new(
        "tmp-ordering-ro",
        "[filesystem]\nallow = [\".\"]\nread = [\"/tmp\"]\n",
    );
    let probe = build_write_probe(&project.root);
    let target = std::path::Path::new("/tmp/devcroft-tmp-ordering-ro-marker");

    let out = run_probe(&project.root, &[probe.as_os_str(), target.as_os_str()]);

    assert!(
        out.status.success(),
        "constructing the view must succeed even though /tmp is read-only: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .starts_with("REFUSED"),
        "filesystem.read = [\"/tmp\"] must be read-only at the mount level, not just in \
         policy --render: got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// The combined, trickiest case: a project root nested under `/tmp` with
/// its own `ReadWrite` grant must stay writable even when `/tmp` itself
/// is granted read-only — the non-recursive remount is what this test
/// exists to pin. A recursive remount (the naive fix for finding 1)
/// would silently drag the nested project root's own mount read-only
/// too, which is finding 2's failure mode in a different disguise.
#[test]
fn a_nested_project_root_stays_writable_when_tmp_itself_is_read_only() {
    if !mount_namespaces_available() {
        eprintln!("skipping: this host cannot create unprivileged mount namespaces");
        return;
    }

    let project = ScratchProject::new(
        "tmp-ordering-combo",
        "[filesystem]\nallow = [\".\"]\nread = [\"/tmp\"]\n",
    );
    let probe = build_write_probe(&project.root);

    // Inside the project root: must succeed (its own ReadWrite grant).
    let inside_target = project.root.join("written-inside");
    let out_inside = run_probe(
        &project.root,
        &[probe.as_os_str(), inside_target.as_os_str()],
    );
    assert!(out_inside.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out_inside.stdout).trim(),
        "WROTE",
        "a project root's own ReadWrite grant must survive /tmp's own read-only remount, \
         even though the project root is physically nested under /tmp"
    );

    // Directly under /tmp, outside the project root: must be refused.
    let outside_target = std::path::Path::new("/tmp/devcroft-tmp-ordering-combo-marker");
    let out_outside = run_probe(
        &project.root,
        &[probe.as_os_str(), outside_target.as_os_str()],
    );
    assert!(out_outside.status.success());
    assert!(
        String::from_utf8_lossy(&out_outside.stdout)
            .trim()
            .starts_with("REFUSED"),
        "/tmp's own top-level read-only mode must still hold outside the nested project root"
    );
}
