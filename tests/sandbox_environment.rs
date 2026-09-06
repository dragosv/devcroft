//! **The sibling of `flox_env_capture_is_deterministic`, one layer up.**
//!
//! That test closed the gap at *resolution*: a decoy `PATH` entry on the
//! invoking shell must not leak into the captured activation. This one closes
//! it at *session*: the same decoy must not reach a process running inside the
//! sandbox.
//!
//! Both halves of one gap. The first was found by review and fixed by running
//! activation under `.env_clear()` with a fixed base; the second survived
//! because `spawn_keeper` then let its `Command` inherit, layering the
//! operator's shell back on top of the very result that had been computed
//! without it. Measured before the fix: 180 variables reached a sandbox and
//! 101 were byte-identical to the invoking shell, including live tokens
//! belonging to unrelated tools (`own-sandbox-environment`).
//!
//! This file sets a process-global environment variable, so it needs to be
//! alone in its own process — same reasoning the sibling test documents.

use std::process::Command;

/// A name no provider, shell profile or tool would ever set, so its presence
/// inside can only mean inheritance.
const DECOY: &str = "DEVCROFT_DECOY_MUST_NOT_LEAK";

#[test]
fn the_invoking_shell_does_not_reach_the_sandbox() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    if Command::new("flox").arg("--version").output().is_err()
        || !devcroft::provider::host_can_build_nix_closures()
    {
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
        std::env::set_var(DECOY, "leaked");
    }
    let bin = env!("CARGO_BIN_EXE_devcroft");

    let root = std::env::temp_dir().join(format!("devcroft-sandbox-env-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    if !Command::new("flox")
        .arg("init")
        .current_dir(&root)
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("skipping: flox init failed");
        return;
    }
    if !Command::new("flox")
        .args(["install", "coreutils"])
        .current_dir(&root)
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("skipping: flox install coreutils failed");
        return;
    }

    // A variable the *provider* sets. Its presence is the control: an
    // implementation that emptied the environment wholesale would pass the
    // leak assertion and break every sandbox.
    // Into the `[vars]` table flox already generates — appending a second one
    // is a duplicate key, which devcroft refuses to parse. That mistake is how
    // this test first "passed": `up` failed, the failure was treated as an
    // unsupported host, and the run went green having asserted nothing. The
    // skip guard below is deliberately narrow for the same reason.
    let manifest_path = root.join(".flox/env/manifest.toml");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap();
    assert!(
        manifest.contains("\n[vars]\n"),
        "flox's generated manifest no longer has a [vars] table; this fixture \
         needs updating rather than working around it"
    );
    std::fs::write(
        &manifest_path,
        manifest.replace(
            "\n[vars]\n",
            "\n[vars]\nDEVCROFT_PROVIDER_SET = \"from-the-manifest\"\n",
        ),
    )
    .unwrap();

    let name = format!("sandboxenv{}", std::process::id());
    std::fs::write(
        root.join("devcroft.toml"),
        format!("[sandbox]\nname = \"{name}\"\n"),
    )
    .unwrap();

    let up = Command::new(bin)
        .arg("up")
        .current_dir(&root)
        .output()
        .unwrap();
    // **Not a skip.** A failed `up` here is a failure: every precondition this
    // test needs was checked above, so the only remaining explanations are a
    // devcroft bug or a broken fixture — and treating either as "unsupported
    // host" is how a green run comes to mean nothing. That is not
    // hypothetical: the first version of this file skipped here, on a
    // duplicate-key mistake of its own making, and reported success.
    assert!(
        up.status.success(),
        "`up` failed with flox available and a Nix store reachable: {}",
        String::from_utf8_lossy(&up.stderr)
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
        !env_dump.contains(DECOY),
        "a variable exported only by the invoking shell must not reach the \
         sandbox; env was:\n{env_dump}"
    );
    assert!(
        env_dump.contains("DEVCROFT_PROVIDER_SET=from-the-manifest"),
        "the provider's own contribution must survive intact — without this, an \
         implementation that emptied the environment would pass the assertion \
         above and break every sandbox. env was:\n{env_dump}"
    );
    // The specific case a name-only comparison would have destroyed: `PATH`
    // exists in both the shell and the activation, and the provider rewrote it.
    //
    // Asserted against the environment's own `bin` rather than a `/nix/store`
    // prefix: flox puts a symlink farm under `.flox/run/<system>-<name>-dev`
    // on `PATH`, not a store path, so a store-prefix check would fail for a
    // sandbox whose toolchain is perfectly present.
    let path_line = env_dump
        .lines()
        .find(|l| l.starts_with("PATH="))
        .unwrap_or_default();
    assert!(
        path_line.contains(".flox/run/"),
        "PATH must still lead with the resolved environment's own bin; \
         got: {path_line}"
    );
}
