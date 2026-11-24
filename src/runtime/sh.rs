//! The runtime's shell (`embed-brush-runtime`): `brush_core`, in a process of
//! its own started from the `devcroft` binary through a link named `sh`.
//!
//! Not brush's own entry point, which parses the process's arguments itself,
//! loads a configuration file and evaluates startup files. Nothing is read
//! here before the command: no profile, no rc file, no `ENV` or `BASH_ENV`.
//! Posix mode is always on, since `sh` is the only name that reaches this.
//!
//! What this shell does not do (traps on signals, `$!`, `wait PID`) is in
//! `docs/known-gaps.md`; the remedy is a shell in the environment, which
//! always comes first (`super::layer`).

use std::path::PathBuf;

use brush_builtins::ShellBuilderExt as _;

/// The name that reaches this shell.
pub const NAME: &str = "sh";

/// The pinned upstream, part of the generation.
pub const UPSTREAM: &str = "brush";
/// `brush-core`'s version, as `Cargo.toml` pins it.
pub const VERSION: &str = "0.5.0";

/// One invocation's arguments.
#[derive(Debug, Default, PartialEq, Eq)]
struct Invocation {
    /// `-c`.
    command: Option<String>,
    /// `-s`.
    from_stdin: bool,
    /// `-i`.
    interactive: bool,
    /// `-l`, or an `argv[0]` starting with `-`.
    login: bool,
    /// `set` options turned on and off, by their `-o` names.
    on: Vec<String>,
    off: Vec<String>,
    /// What follows the options.
    operands: Vec<String>,
}

/// The `-o` names of the single-letter `set` options POSIX gives `sh`.
fn option_name(letter: char) -> Option<&'static str> {
    Some(match letter {
        'a' => "allexport",
        'b' => "notify",
        'C' => "noclobber",
        'e' => "errexit",
        'f' => "noglob",
        'h' => "hashall",
        'm' => "monitor",
        'n' => "noexec",
        'u' => "nounset",
        'v' => "verbose",
        'x' => "xtrace",
        _ => return None,
    })
}

fn parse(argv0: &str, args: impl IntoIterator<Item = String>) -> Result<Invocation, String> {
    let mut inv = Invocation {
        login: argv0.starts_with('-'),
        ..Invocation::default()
    };
    let mut args = args.into_iter();
    let mut wants_command = false;
    while let Some(arg) = args.next() {
        if arg == "--" || arg == "-" {
            inv.operands.extend(args);
            break;
        }
        if arg == "--login" {
            inv.login = true;
            continue;
        }
        let (on, letters) = match (arg.strip_prefix('-'), arg.strip_prefix('+')) {
            (Some(l), _) if !l.is_empty() && !l.starts_with('-') => (true, l),
            (_, Some(l)) if !l.is_empty() => (false, l),
            (Some(l), _) if l.starts_with('-') => return Err(format!("{arg}: invalid option")),
            _ => {
                inv.operands.push(arg);
                inv.operands.extend(args);
                break;
            }
        };
        for letter in letters.chars() {
            let list = if on { &mut inv.on } else { &mut inv.off };
            match letter {
                'c' if on => wants_command = true,
                's' if on => inv.from_stdin = true,
                'i' if on => inv.interactive = true,
                'l' if on => inv.login = true,
                'o' => list.push(args.next().ok_or("-o: option requires an argument")?),
                other => list.push(
                    option_name(other)
                        .ok_or_else(|| format!("-{other}: invalid option"))?
                        .to_owned(),
                ),
            }
        }
    }
    if wants_command {
        if inv.operands.is_empty() {
            return Err("-c: option requires an argument".to_owned());
        }
        inv.command = Some(inv.operands.remove(0));
    }
    Ok(inv)
}

/// Run the shell for this process's arguments and exit with its status.
pub fn main() -> ! {
    let mut args = std::env::args_os().map(|a| a.to_string_lossy().into_owned());
    let argv0 = args.next().unwrap_or_else(|| NAME.to_owned());
    let code = match parse(&argv0, args) {
        // Not the current-thread runtime: a substitution whose builtins write
        // more than a pipe holds then waits on itself forever (measured).
        Ok(inv) => match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(run(&argv0, inv)),
            Err(e) => {
                eprintln!("{NAME}: {e}");
                1
            }
        },
        Err(e) => {
            eprintln!("{NAME}: {e}");
            2
        }
    };
    std::process::exit(code.into());
}

/// The descriptors this process was given beyond the standard three, which
/// `>&3` must reach.
fn inherited_fds() -> impl Iterator<Item = i32> {
    (3..=9).filter(|fd| {
        // SAFETY: F_GETFD only asks about a descriptor.
        let flags = unsafe { libc::fcntl(*fd, libc::F_GETFD) };
        flags >= 0 && flags & libc::FD_CLOEXEC == 0
    })
}

async fn run(argv0: &str, inv: Invocation) -> u8 {
    use std::io::IsTerminal;
    // Before anything here opens a descriptor of its own.
    // Standard input too, as a file: brush's own handle for it reads ahead,
    // so a `read` would take the rest of a script that comes from there.
    let fds = [0]
        .into_iter()
        .chain(inherited_fds())
        .filter_map(|fd| brush_core::sys::fd::try_get_file_for_open_fd(fd).map(|f| (fd, f)))
        .collect();
    let script = inv.command.is_none() && !inv.from_stdin && !inv.operands.is_empty();
    let interactive = inv.interactive
        || (inv.command.is_none()
            && !script
            && std::io::stdin().is_terminal()
            && std::io::stderr().is_terminal());
    // `sh -c cmd name args`, `sh -s args`, `sh script args`.
    let (name, rest) = match &inv.command {
        Some(_) if !inv.operands.is_empty() => (inv.operands[0].clone(), &inv.operands[1..]),
        _ if script => (argv0.to_owned(), &inv.operands[..0]),
        _ => (argv0.to_owned(), &inv.operands[..]),
    };
    let built = brush_core::Shell::builder()
        .enable_options(inv.on.clone())
        .disable_options(inv.off.clone())
        .login(inv.login)
        .interactive(interactive)
        .command_string_mode(inv.command.is_some())
        .read_commands_from_stdin(inv.command.is_none() && !script)
        .no_editing(true)
        .profile(brush_core::ProfileLoadBehavior::Skip)
        .rc(brush_core::RcLoadBehavior::Skip)
        .posix(true)
        .fds(fds)
        .shell_name(name)
        .shell_args(rest.to_vec())
        .maybe_working_dir(std::env::current_dir().ok())
        .default_builtins(brush_builtins::BuiltinSet::BashMode)
        .build()
        .await;
    let mut shell = match built {
        Ok(shell) => shell,
        Err(e) => {
            eprintln!("{NAME}: {e:#}");
            return 1;
        }
    };
    if let Some(command) = inv.command {
        let result = shell.run_dash_c_command(command).await;
        return finish(&shell, result);
    }
    if script {
        let path = PathBuf::from(&inv.operands[0]);
        if let Err(e) = std::fs::File::open(&path) {
            eprintln!("{NAME}: {}: {e}", path.display());
            return if e.kind() == std::io::ErrorKind::NotFound {
                127
            } else {
                126
            };
        }
        let result = shell.run_script(&path, inv.operands[1..].iter()).await;
        return finish(&shell, result);
    }
    // A terminal or a pipe: one complete command at a time, with no editing
    // and no question to the terminal that a bare pty would never answer.
    let shell = std::sync::Arc::new(tokio::sync::Mutex::new(shell));
    let mut input = CommandInput { interactive };
    let options = brush_interactive::InteractiveOptions::default();
    let result = match brush_interactive::InteractiveShell::new(&shell, &mut input, &options) {
        Ok(mut session) => session.run_interactively().await,
        Err(e) => Err(e),
    };
    if let Err(e) = result {
        eprintln!("{NAME}: {e}");
        return 1;
    }
    shell.lock().await.last_exit_status()
}

/// Standard input, up to the end of a complete command. Read a byte at a
/// time, so a `read` in the command finds the line after it.
struct CommandInput {
    interactive: bool,
}

impl CommandInput {
    fn line() -> std::io::Result<String> {
        let mut line = Vec::new();
        loop {
            let mut byte = 0u8;
            // SAFETY: one byte into a live variable, from standard input.
            match unsafe { libc::read(0, (&raw mut byte).cast(), 1) } {
                0 => break,
                1 => {
                    line.push(byte);
                    if byte == b'\n' {
                        break;
                    }
                }
                _ => {
                    let e = std::io::Error::last_os_error();
                    if e.kind() != std::io::ErrorKind::Interrupted {
                        return Err(e);
                    }
                }
            }
        }
        Ok(String::from_utf8_lossy(&line).into_owned())
    }

    fn prompt(&self, text: &str) {
        if self.interactive {
            eprint!("{text}");
        }
    }
}

impl brush_interactive::InputBackend for CommandInput {
    fn read_line(
        &mut self,
        shell: &brush_interactive::ShellRef<impl brush_core::ShellExtensions>,
        prompt: brush_interactive::InteractivePrompt,
    ) -> Result<brush_interactive::ReadResult, brush_interactive::ShellError> {
        use brush_interactive::{ReadResult, ShellError};
        self.prompt(&prompt.prompt);
        let mut input = Self::line().map_err(ShellError::InputError)?;
        if input.is_empty() {
            return Ok(ReadResult::Eof);
        }
        loop {
            // A line ending in `\\` goes on, which the parser does not say.
            let body = input.strip_suffix('\n').unwrap_or(&input);
            let escapes = body.len() - body.trim_end_matches('\\').len();
            let complete = escapes % 2 == 0
                && tokio::task::block_in_place(|| {
                    !is_incomplete(&shell.blocking_lock().parse_string(input.clone()))
                });
            if complete {
                return Ok(ReadResult::Input(input));
            }
            self.prompt(&prompt.continuation_prompt);
            let more = Self::line().map_err(ShellError::InputError)?;
            if more.is_empty() {
                // Ended inside a command: the parser says what is missing.
                return Ok(ReadResult::Input(input));
            }
            input.push_str(&more);
        }
    }
}

/// Whether a parse failed only because the input stopped.
fn is_incomplete<T>(parsed: &Result<T, brush_core::parser::ParseError>) -> bool {
    use brush_core::parser::ParseError;
    match parsed {
        Err(ParseError::ParsingAtEndOfInput) => true,
        Err(ParseError::Tokenizing { inner, .. }) => inner.is_incomplete(),
        _ => false,
    }
}

fn finish<SE: brush_core::ShellExtensions, T>(
    shell: &brush_core::Shell<SE>,
    result: Result<T, brush_core::Error>,
) -> u8 {
    match result {
        Ok(_) => shell.last_exit_status(),
        Err(e) => {
            eprintln!("{NAME}: {e:#}");
            match shell.last_exit_status() {
                0 => 1,
                status => status,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Invocation, String> {
        parse("sh", args.iter().map(|a| (*a).to_owned()))
    }

    #[test]
    fn arguments_are_a_posix_shells() {
        let inv = p(&["-c", "cmd", "name", "a", "b"]).unwrap();
        assert_eq!(inv.command.as_deref(), Some("cmd"));
        assert_eq!(inv.operands, ["name", "a", "b"]);
        let inv = p(&["-euc", "cmd"]).unwrap();
        assert_eq!(inv.on, ["errexit", "nounset"]);
        assert_eq!(inv.command.as_deref(), Some("cmd"));
        assert_eq!(
            p(&["-c", "--", "-x"]).unwrap().command.as_deref(),
            Some("-x")
        );
        let inv = p(&["-o", "pipefail", "+e", "script", "-c"]).unwrap();
        assert_eq!(
            (inv.on, inv.off),
            (vec!["pipefail".to_owned()], vec!["errexit".to_owned()])
        );
        assert_eq!(inv.operands, ["script", "-c"]);
        assert!(inv.command.is_none());
        assert!(p(&["-s", "a"]).unwrap().from_stdin);
        assert!(parse("-sh", []).unwrap().login);
        assert!(p(&["-c"]).is_err());
        assert!(p(&["-Z"]).is_err());
        assert!(p(&["--bogus"]).is_err());
        assert!(p(&["-o"]).is_err());
    }
}
