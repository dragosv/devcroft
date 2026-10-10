//! What a provider's activation script exports reaches what the keeper
//! starts afterwards (`keeper::session::EnvOverlay`).
//!
//! The script runs in its own shell, which exits, so for most of the
//! project's life everything it exported was lost: `LOCALE_ARCHIVE` and
//! `MANPATH` from devenv's `enterShell`, a flox hook's `CARGO_HOME`. The
//! design said running the hook in the sandbox "restores them"; measured,
//! it did not.
//!
//! Its own test binary, with one test: the overlay is process-global by
//! design (a keeper runs one activation), so sharing a process with other
//! tests would let them see each other's.

#![cfg(unix)]

use devcroft::keeper::{LocalSessionBackend, SessionBackend, SpawnRequest};
use devcroft::lifecycle::hooks::{ActivationCapture, run_in_keeper};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

#[test]
fn what_activation_exports_reaches_later_hooks_and_sessions_and_its_status_holds() {
    let dir = std::env::temp_dir().join(format!(
        "devcroft-activation-exports-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let before: BTreeMap<String, String> = [
        ("PATH".to_owned(), std::env::var("PATH").unwrap()),
        ("GOES_AWAY".to_owned(), "x".to_owned()),
    ]
    .into();
    let capture = ActivationCapture {
        exe: Path::new(env!("CARGO_BIN_EXE_devcroft")).to_path_buf(),
        out: dir.join(".devcroft-activation-env"),
        before,
    };

    // A failing script still fails its hook, and records nothing: the
    // wrapper keeps the script's own exit status.
    let failing = [("activation", "export NEVER=1; false".to_owned())];
    let err = run_in_keeper(
        &LocalSessionBackend,
        "/bin/sh",
        &dir,
        &failing,
        Some(&capture),
    );
    assert!(
        err.is_err(),
        "a failing activation script must fail its hook"
    );

    // The script runs with the keeper's real environment here, so it unsets
    // what `before` says it started with.
    let hooks = [
        (
            "activation",
            "export FROM_HOOK='with spaces'; export PATH=\"/hook/bin:$PATH\"; unset GOES_AWAY"
                .to_owned(),
        ),
        // A later hook runs in what activation prepared.
        (
            "post_create",
            "echo \"$FROM_HOOK\" > seen-by-post-create".to_owned(),
        ),
    ];
    run_in_keeper(
        &LocalSessionBackend,
        "/bin/sh",
        &dir,
        &hooks,
        Some(&capture),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("seen-by-post-create")).unwrap(),
        "with spaces\n"
    );
    assert!(!capture.out.exists(), "the dump is removed once read");

    // And a session started afterwards has it, with the request's own
    // `env` still winning where it names the same variable.
    let mut spawned = LocalSessionBackend
        .spawn(&SpawnRequest {
            cmd: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "echo \"$FROM_HOOK|${PATH%%:*}|${GOES_AWAY-unset}|$OVERRIDE|${NEVER-unset}\""
                    .into(),
            ],
            cwd: dir.to_string_lossy().into_owned(),
            env: [("OVERRIDE".to_owned(), "request".to_owned())].into(),
            pty: None,
        })
        .unwrap();
    let mut out = String::new();
    spawned.stdout.read_to_string(&mut out).unwrap();
    assert_eq!(out.trim(), "with spaces|/hook/bin|unset|request|unset");
    let _ = std::fs::remove_dir_all(&dir);
}
