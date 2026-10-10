//! `fix-silent-policy-degradation`: `why` and `policy --render` either
//! answer from the whole policy or say they cannot.
//!
//! Measured before the fix, from inside a real sandbox: `why --path
//! /nix/store --op read` printed `DENIED`, exit 0, where the host printed
//! `ALLOWED provider:nix`. Inside, the state dir does not exist (the mount
//! view never contains it), and "no record" read as "no sandbox", so the
//! answer came from the manifest alone, without the provider's store.
//!
//! These drive the real binary with a constructed `HOME`, so the four
//! cases need no provider: no sandbox, an unreadable record, a malformed
//! one, and inside the sandbox with its own copy of the policy. The live
//! version, from a real session, is `tests/policy_view_fidelity_live.rs`.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const NAME: &str = "fidelity";

/// A project, a home for the user, and the sandbox's state dir under it.
struct Fixture {
    base: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let base =
            std::env::temp_dir().join(format!("devcroft-fidelity-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("project")).unwrap();
        std::fs::create_dir_all(base.join("home")).unwrap();
        std::fs::write(
            base.join("project/devcroft.toml"),
            format!("[sandbox]\nname = \"{NAME}\"\n\n[env]\nprovider = \"nix\"\n"),
        )
        .unwrap();
        Fixture { base }
    }

    fn project(&self) -> PathBuf {
        self.base.join("project")
    }

    fn meta(&self) -> PathBuf {
        self.base
            .join("home/.local/share/devcroft")
            .join(NAME)
            .join("meta.json")
    }

    /// A record of a sandbox whose provider granted `/opt/store-of-x`.
    fn write_meta(&self, text: Option<&str>) {
        std::fs::create_dir_all(self.meta().parent().unwrap()).unwrap();
        let default = format!(
            r#"{{"project_root": {:?}, "env_fingerprint": "f", "read_only_grants": ["/opt/store-of-x"]}}"#,
            self.project().display().to_string()
        );
        std::fs::write(self.meta(), text.unwrap_or(&default)).unwrap();
    }

    fn why(&self, home: &Path) -> Output {
        Command::new(env!("CARGO_BIN_EXE_devcroft"))
            .current_dir(self.project())
            .env("HOME", home)
            .args(["why", "--path", "/opt/store-of-x", "--op", "read"])
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(dir) = self.meta().parent() {
            let _ = std::fs::set_permissions(self.meta(), std::fs::Permissions::from_mode(0o600));
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn a_readable_record_and_no_sandbox_answer_as_before() {
    let f = Fixture::new("plain");
    // No sandbox: the manifest is the whole policy, and the answer is the
    // same bare verdict it always was, exit 0.
    let none = f.why(&f.base.join("home"));
    assert_eq!(none.status.code(), Some(0), "{none:?}");
    assert!(stdout(&none).starts_with("DENIED\n"), "{}", stdout(&none));
    assert!(!stdout(&none).contains("UNKNOWN"));

    // A readable record folds in the provider's grant.
    f.write_meta(None);
    let some = f.why(&f.base.join("home"));
    assert_eq!(some.status.code(), Some(0), "{some:?}");
    assert_eq!(stdout(&some), "ALLOWED\nallowed by rule provider:nix\n");
}

#[test]
fn an_unreadable_record_says_so_instead_of_inverting_the_verdict() {
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipping: root reads a mode-000 file, so nothing is unreadable");
        return;
    }
    let f = Fixture::new("unreadable");
    f.write_meta(None);
    std::fs::set_permissions(f.meta(), std::fs::Permissions::from_mode(0o000)).unwrap();
    let out = f.why(&f.base.join("home"));
    // Before: `DENIED`, exit 0, for a path the provider grants.
    assert!(!stdout(&out).contains("DENIED"), "{}", stdout(&out));
    assert!(stdout(&out).starts_with("UNKNOWN\n"), "{}", stdout(&out));
    assert!(
        stdout(&out).contains("could not be read"),
        "{}",
        stdout(&out)
    );
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn a_malformed_record_is_told_apart_from_an_unreadable_one() {
    let f = Fixture::new("malformed");
    f.write_meta(Some("not json"));
    let out = f.why(&f.base.join("home"));
    assert!(stdout(&out).starts_with("UNKNOWN\n"), "{}", stdout(&out));
    assert!(stdout(&out).contains("not valid"), "{}", stdout(&out));
    assert_eq!(out.status.code(), Some(1));

    // The spec states this one for `policy --render`: still rendered, but
    // marked incomplete for the right reason, and not exit 0.
    let render = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .current_dir(f.project())
        .env("HOME", f.base.join("home"))
        .args(["policy", "--render"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&render.stderr);
    assert!(
        stderr.contains("incomplete") && stderr.contains("not valid"),
        "{stderr}"
    );
    assert!(!stderr.contains("could not be read"), "{stderr}");
    assert_eq!(render.status.code(), Some(1));
}

#[test]
fn inside_the_sandbox_its_own_copy_answers_and_a_leftover_one_does_not() {
    let f = Fixture::new("inside");
    // What `up` writes: the compiled policy, provider grant included.
    let (manifest, _) = devcroft::config::parse(&format!(
        "[sandbox]\nname = \"{NAME}\"\n\n[env]\nprovider = \"nix\"\n"
    ))
    .unwrap();
    let policy = devcroft::policy::compile(&manifest)
        .with_provider_grants("nix", &["/opt/store-of-x".to_owned()]);
    let artifact = devcroft::services::policy_artifact(&f.project(), NAME);
    std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    std::fs::write(&artifact, serde_json::to_string(&policy).unwrap()).unwrap();

    // Inside: `HOME` is the sandbox's own, and there is no state dir.
    let home = artifact.parent().unwrap().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let inside = f.why(&home);
    assert_eq!(inside.status.code(), Some(0), "{inside:?}");
    assert_eq!(stdout(&inside), "ALLOWED\nallowed by rule provider:nix\n");

    // On the host, with no sandbox recorded, the same leftover artifact is
    // not read: "no record" there means no sandbox.
    let host = f.why(&f.base.join("home"));
    assert!(stdout(&host).starts_with("DENIED\n"), "{}", stdout(&host));
    assert_eq!(host.status.code(), Some(0));
}
