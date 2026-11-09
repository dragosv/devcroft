//! A POSIX shell for the Nix-free test row (`add-nix-free-test-row`).
//!
//! **One line, and that is the point.** The row needs a real shell from
//! neither the Nix store nor the host's `/bin` — the store because the row
//! exists to work without one, and `/bin` because a copied macOS platform
//! binary lands in an unkillable `UE` state when executed (measured; see
//! that change's design.md N2).
//!
//! Taking `brush` as a dev-dependency dissolves the problems every other
//! source had: one artifact on both platforms instead of BusyBox-on-Linux
//! and dash-on-macOS, pinned by `Cargo.lock` instead of a hand-maintained
//! per-architecture hash, MIT instead of GPL-2.0, and no C toolchain at
//! fixture-setup time.
//!
//! **Why `examples/` and not `src/bin/`.** `src/bin/` targets are
//! auto-discovered — CLAUDE.md records that excluding `spike.rs` from the
//! packaging allowlist is load-bearing for exactly that reason — and they
//! cannot use dev-dependencies, so a wrapper there would drag `brush` into
//! the *shipped* tree. An example can use dev-dependencies and is never
//! installed. Measured: adding this changes devcroft's shipped dependency
//! tree by zero crates.

/// Dispatch on `argv[0]`, busybox-style, so one binary can be the row's
/// whole userland.
///
/// The row symlinks `pwd`, `sleep` and `echo` at this same file. Without
/// that, a row backed only by a shell cannot run `exec -- pwd`: a shell
/// builtin does not satisfy `Command::new("pwd")`, which is what devcroft's
/// keeper actually calls. The utilities are `uutils` reimplementations, and
/// cost 4 crates on top of `brush` because the shared `uucore` layer is
/// already in the tree.
fn main() {
    let arg0 = std::env::args_os().next().unwrap_or_default();
    let name = std::path::Path::new(&arg0)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    // `uumain` returns the exit code directly here (the crates wrap their
    // own `UResult` before returning).
    let code = match name.as_str() {
        "pwd" => uu_pwd::uumain(std::env::args_os()),
        "sleep" => uu_sleep::uumain(std::env::args_os()),
        "echo" => uu_echo::uumain(std::env::args_os()),
        // Anything else — `sh` included — is the shell.
        _ => {
            with_plain_line_input();
            return brush_shell::entry::run();
        }
    };
    std::process::exit(code);
}

/// On a terminal, re-exec with `--input-backend=basic` unless a backend was
/// already chosen.
///
/// brush's default on a terminal is `reedline`, which asks the terminal for
/// the cursor position (`ESC[6n`) at its first prompt and aborts when no
/// reply comes. A pty with no emulator behind it — `script` in
/// `tests/shell_up.rs`, an editor's remote terminal before it draws — never
/// replies, so the session died with "the cursor position could not be
/// read". bash's readline asks nothing, so a real provider's shell never
/// hit this; the line editor is this row's choice, not devcroft's behaviour,
/// and the session is still a pty either way.
///
/// A re-exec rather than an argument because `brush_shell::entry::run`
/// reads `std::env::args()` itself and takes none.
fn with_plain_line_input() {
    use std::io::IsTerminal;
    use std::os::unix::process::CommandExt;

    let args: Vec<_> = std::env::args_os().collect();
    let chosen = args
        .iter()
        .any(|a| a.to_string_lossy().starts_with("--input-backend"));
    if chosen || !std::io::stdin().is_terminal() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut cmd = std::process::Command::new(exe);
    // `argv[0]` kept, so a login shell (`-sh`) stays one.
    cmd.arg0(&args[0])
        .arg("--input-backend=basic")
        .args(&args[1..]);
    // Only returns on failure; the shell then runs with its default.
    let _ = cmd.exec();
}
