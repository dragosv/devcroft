//! `devcroft --help` and `devcroft --version`, which is a smaller subject
//! than it looks.
//!
//! `src/lib.rs` tells anyone reading the crate docs to depend on "the
//! `devcroft` binary and its documented command surface (`devcroft
//! --help`, and the README)". Until the first release audit ran the
//! *packaged* binary, that command answered `unknown command "--help"`,
//! and the fallback pointed a user of a published binary at "the cli
//! spec" — a file that ships in the repository and not in the crate. The
//! first thing anyone does with a freshly installed CLI is ask it for
//! help, so this is the one claim that cannot be allowed to rot quietly.
//!
//! Needs no provider, no sandbox and no kernel feature: it runs the
//! binary and reads what it prints, so unlike most of this suite it can
//! never self-skip.

use std::process::Command;

/// The closed MVP command surface (CLAUDE.md). Listed here rather than
/// spot-checked so that adding a command without documenting it fails
/// this test — the usage text is the only place a user of the published
/// binary can discover what exists.
const COMMANDS: &[&str] = &[
    "init",
    "up",
    "down",
    "rm",
    "status",
    "logs",
    "ps",
    "shell",
    "exec",
    "ssh",
    "proxy",
    "ssh-config",
    "policy",
    "why",
    "doctor",
];

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn help_is_reachable_by_every_spelling_and_lists_the_whole_surface() {
    for spelling in ["--help", "-h", "help"] {
        let (code, stdout, _) = run(&[spelling]);
        assert_eq!(code, 0, "`devcroft {spelling}` must succeed");
        for cmd in COMMANDS {
            assert!(
                stdout.contains(cmd),
                "`devcroft {spelling}` does not mention `{cmd}`"
            );
        }
    }
}

/// Hidden re-exec targets are devcroft talking to itself, not commands
/// anyone types; listing them would invite someone to.
#[test]
fn help_does_not_advertise_the_internal_reexec_modes() {
    let (_, stdout, _) = run(&["--help"]);
    assert!(!stdout.contains("__"), "help text leaks a hidden mode");
}

#[test]
fn version_reports_the_crate_version_on_stdout() {
    for spelling in ["--version", "-V"] {
        let (code, stdout, _) = run(&[spelling]);
        assert_eq!(code, 0);
        assert_eq!(
            stdout.trim(),
            format!("devcroft {}", env!("CARGO_PKG_VERSION"))
        );
    }
}

/// An explicit `help` is a question and gets stdout with exit 0; a bare
/// or malformed invocation is a mistake and gets stderr with the error
/// contract's usage code. A script piping one should be able to tell
/// them apart.
#[test]
fn misuse_goes_to_stderr_with_exit_2_while_help_goes_to_stdout_with_0() {
    let (code, stdout, stderr) = run(&[]);
    assert_eq!(code, 2, "a bare invocation is a usage error");
    assert!(stdout.is_empty(), "usage errors must not print to stdout");
    assert!(stderr.contains("usage:"));

    let (code, stdout, stderr) = run(&["frobnicate"]);
    assert_eq!(code, 2);
    assert!(stdout.is_empty());
    assert!(
        stderr.contains("frobnicate"),
        "the message must name what was not understood, got: {stderr}"
    );
    assert!(
        stderr.contains("usage:"),
        "an unknown command should show what the known ones are"
    );
}

/// The old message sent a user of a published binary to a file that only
/// exists in the repository.
#[test]
fn no_message_here_points_at_something_the_crate_does_not_ship() {
    let (_, stdout, _) = run(&["--help"]);
    let (_, _, stderr) = run(&["frobnicate"]);
    for text in [stdout, stderr] {
        assert!(!text.contains("cli spec"), "points at an unshipped file");
        assert!(!text.contains("openspec"), "points at an unshipped tree");
    }
}
