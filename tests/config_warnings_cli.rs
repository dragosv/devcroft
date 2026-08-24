//! The `config` spec's two warning scenarios, through the real binary.
//!
//! These existed as spec requirements, and as a `Warning` type with a
//! `Display` impl, but nothing ever printed them: every `config::parse`
//! call site in the CLI bound `_warnings` and dropped them. Found by
//! adversarial review rather than by a failing test — precisely because
//! no test covered them, which is what this file fixes.
//!
//! Deliberately needs **no external tooling at all**. The warnings are
//! about the manifest, so they are printed before provider resolution
//! even begins; asserting on an `up` that then fails at layer `provider`
//! keeps this file portable, where gating it on flox/devbox/nix would
//! have made the regression invisible again on hosts without them.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn scratch_project(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "devcroft-config-warnings-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(cwd: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// Removes the state directory `up` creates before it fails, so these
/// tests do not leave one behind per run.
fn cleanup(dir: &std::path::Path, sandbox: &str) {
    if let Ok(paths) = devcroft::lifecycle::StatePaths::new(sandbox) {
        let _ = std::fs::remove_dir_all(paths.root);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Spec: "validation succeeds but a warning is printed at `up`, once".
#[test]
fn up_warns_once_about_each_sensitive_path_grant() {
    let dir = scratch_project("sensitive");
    std::fs::write(
        dir.join("devcroft.toml"),
        "[sandbox]\nname = \"cfgwarnsensitive\"\n\
         [filesystem]\nallow = [\"~/.ssh\", \"~/.aws\"]\n",
    )
    .unwrap();

    let out = run(&dir, &["up"]);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        stderr.contains("~/.ssh") && stderr.contains("credential directory"),
        "granting ~/.ssh must warn, got: {stderr}"
    );
    assert!(
        stderr.contains("~/.aws"),
        "each sensitive grant is named, not just the first, got: {stderr}"
    );
    assert_eq!(
        stderr.matches("~/.ssh").count(),
        1,
        "the spec says once, got: {stderr}"
    );

    cleanup(&dir, "cfgwarnsensitive");
}

/// Spec: an `[env] vars` value containing `$` "prints a one-time warning
/// that interpolation is not supported".
#[test]
fn up_warns_that_env_vars_are_not_interpolated() {
    let dir = scratch_project("interpolation");
    std::fs::write(
        dir.join("devcroft.toml"),
        "[sandbox]\nname = \"cfgwarninterp\"\n\
         [env]\nvars = { GREETING = \"hello $USER\" }\n",
    )
    .unwrap();

    let out = run(&dir, &["up"]);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        stderr.contains("not interpolated"),
        "a `$` in an env var value must warn, got: {stderr}"
    );

    cleanup(&dir, "cfgwarninterp");
}

/// The scoping half of the spec's "at `up`, once": a manifest with
/// nothing notable must not produce warning noise.
#[test]
fn up_is_silent_for_a_manifest_with_nothing_to_warn_about() {
    let dir = scratch_project("quiet");
    std::fs::write(
        dir.join("devcroft.toml"),
        "[sandbox]\nname = \"cfgwarnquiet\"\n",
    )
    .unwrap();

    let out = run(&dir, &["up"]);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !stderr.contains("warning:"),
        "an unremarkable manifest must warn about nothing, got: {stderr}"
    );

    cleanup(&dir, "cfgwarnquiet");
}

/// `status` re-reads the same manifest; re-nagging on every command is
/// what "at `up`, once" rules out.
#[test]
fn status_does_not_repeat_the_warnings() {
    let dir = scratch_project("norepeat");
    std::fs::write(
        dir.join("devcroft.toml"),
        "[sandbox]\nname = \"cfgwarnnorepeat\"\n\
         [filesystem]\nallow = [\"~/.ssh\"]\n",
    )
    .unwrap();

    let out = run(&dir, &["status"]);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !stderr.contains("credential directory"),
        "status must not repeat up's warnings, got: {stderr}"
    );

    cleanup(&dir, "cfgwarnnorepeat");
}
