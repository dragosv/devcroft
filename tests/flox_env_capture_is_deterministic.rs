//! Regression test for a real reproducibility gap found via review:
//! `provider::flox::FloxProvider::resolve` used to run `flox activate`
//! inheriting the *entire* environment of whoever ran `up` — so a
//! personal `PATH` (nvm, rustup, ad hoc `~/bin`, ...) on the invoking
//! shell could leak into the captured activation diff, meaning the exact
//! same manifest could resolve differently depending on who ran `up` and
//! from where. Fixed by running `flox activate` with `.env_clear()` and a
//! fixed `HOME`+`PATH`, diffed against that same fixed base rather than
//! `std::env::vars()`.
//!
//! This mutates the test *process's own* `PATH` env var to inject a decoy
//! directory, so it needs to be alone in its own process — same reasoning
//! as every other test file in this suite that sets a process-global env
//! var (see e.g. tests/lifecycle_up.rs's `DEVCROFT_KEEPER_EXE`).

use devcroft::provider::{FloxProvider, Provider};
use std::process::Command;

#[test]
fn a_decoy_path_entry_on_the_invoking_shell_does_not_leak_into_the_activation() {
    if Command::new("flox").arg("--version").output().is_err() {
        eprintln!("skipping: flox not on PATH");
        return;
    }

    let project_root =
        std::env::temp_dir().join(format!("devcroft-flox-determinism-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();
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

    // A directory that looks exactly like a real operator's personal tool
    // dir: a marker env var an activation-time leak could plausibly carry,
    // plus a fake binary that must never appear reachable afterward.
    let decoy_dir =
        std::env::temp_dir().join(format!("devcroft-flox-decoy-{}", std::process::id()));
    std::fs::create_dir_all(&decoy_dir).unwrap();
    std::fs::write(
        decoy_dir.join("not-a-real-tool"),
        "#!/bin/sh\necho leaked\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(decoy_dir.join("not-a-real-tool"))
            .unwrap()
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(decoy_dir.join("not-a-real-tool"), perms).unwrap();
    }

    let real_path = std::env::var("PATH").unwrap();
    let contaminated_path = format!("{}:{real_path}", decoy_dir.display());
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("PATH", &contaminated_path);
        std::env::set_var("DEVCROFT_LEAK_MARKER", "should-not-survive");
    }

    let resolution = FloxProvider.resolve(&project_root).unwrap();

    let path_in_diff = resolution.env.get("PATH").cloned().unwrap_or_default();
    assert!(
        !path_in_diff.contains(&decoy_dir.display().to_string()),
        "the decoy PATH entry from the invoking shell leaked into the \
         activation diff's PATH: {path_in_diff:?}"
    );
    assert!(
        !resolution.env.contains_key("DEVCROFT_LEAK_MARKER"),
        "an arbitrary env var from the invoking shell leaked into the \
         activation diff: {:?}",
        resolution.env
    );

    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::remove_dir_all(&decoy_dir);
}
