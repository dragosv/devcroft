//! The `devcroft` binary. Its only real entrypoint today is the hidden
//! `__keeper` mode `lifecycle::up` re-execs into via `nono wrap` (see
//! `up.rs`'s module docs) — the user-facing command surface the `cli`
//! spec describes (`init`, `up`, `down`, ...) is built incrementally as
//! each capability lands; task group 7 is where it gets polished with
//! real argument parsing and the error contract's exit codes.

use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::net::UnixListener;
use std::sync::Arc;

use devcroft::keeper::{Keeper, LocalSessionBackend, Registry, SessionBackend};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("__keeper") => {
            let fd: RawFd = args
                .get(2)
                .and_then(|s| s.parse().ok())
                .expect("__keeper requires a control-socket fd argument");
            let ssh_fd: RawFd = args
                .get(3)
                .and_then(|s| s.parse().ok())
                .expect("__keeper requires an ssh-socket fd argument");
            keeper_main(fd, ssh_fd);
        }
        // Hidden, like `__keeper`: `doctor`'s listening-socket probe
        // re-execs into this so the irreversible restriction lands in a
        // throwaway child rather than in `doctor` itself.
        Some("__bind_probe") => std::process::exit(bind_probe_main(&args[2..])),
        // Hidden, same reasoning as `__bind_probe`: entering a network
        // namespace is irreversible for the process that does it, so the
        // probe runs in a child nobody minds losing.
        Some("__netns_probe") => std::process::exit(netns_probe_main(&args[2..])),
        // Hidden: does Landlock mediate connect() to a pathname unix
        // socket? Decides whether an isolated sandbox reaching the host
        // egress proxy over a UDS needs an explicit filesystem grant.
        Some("__uds_probe") => std::process::exit(uds_probe_main(&args[2..])),
        // Hidden: does devcroft's default CapabilitySet (IpcMode left at
        // its library default) actually deny connect() to an *abstract*
        // unix socket — the AF_UNIX half a mount view cannot close, since
        // an abstract socket has no filesystem path to remove
        // (`add-backend-capabilities` task 1.5). Evidence for the
        // `abstract-unix-sockets` matrix entry.
        Some("__abstract_socket_probe") => {
            std::process::exit(abstract_socket_probe_main(&args[2..]))
        }
        // Hidden: simulates one fleet agent binding a service port
        // inside its own namespace, for `tests/fleet_netns.rs`. Lives in
        // the real binary rather than the test so it enters a namespace
        // the same way production will — a test-only reimplementation
        // could drift from what devcroft actually does.
        Some("__netns_agent_sim") => std::process::exit(netns_agent_sim_main(&args[2..])),
        // Hidden, same reasoning as `__netns_probe`: entering a mount
        // namespace is irreversible for the process that does it.
        Some("__mount_probe") => std::process::exit(mount_probe_main(&args[2..])),
        // Hidden: proves a mount made inside a fresh, private-propagation
        // mount namespace does not leak to the host's own namespace, for
        // `tests/fleet_mount.rs`. Lives in the real binary rather than the
        // test for the same reason `__netns_agent_sim` does — a test-only
        // reimplementation could drift from what devcroft actually does.
        Some("__mount_isolation_sim") => std::process::exit(mount_isolation_sim_main(&args[2..])),
        // Hidden: live-verifies `fleet::mount::construct_view` against a
        // real project's compiled policy — not a unit test, since the
        // whole point is proving a real toolchain still works after
        // pivot_root, which nothing short of actually running it can show.
        Some("__mount_view_probe") => std::process::exit(mount_view_probe_main(&args[2..])),
        // Hidden: `crate::proxy::spawn` re-execs into this to run the
        // egress proxy (add-egress-proxy) as its own permanently
        // unsandboxed process — never `__keeper`'s. Not the user-facing
        // `proxy` command above, which is the unrelated SSH
        // `ProxyCommand` handler; the two share nothing but a word.
        Some("__egress_proxy") => {
            let fd: RawFd = args
                .get(2)
                .and_then(|s| s.parse().ok())
                .expect("__egress_proxy requires a listener fd argument");
            let unix_fd: RawFd = args
                .get(3)
                .and_then(|s| s.parse().ok())
                .expect("__egress_proxy requires a unix listener fd argument");
            egress_proxy_main(fd, unix_fd);
        }
        Some("exec") => std::process::exit(cli_exec(&args[2..])),
        Some("shell") => std::process::exit(cli_shell(&args[2..])),
        Some("proxy") => std::process::exit(cli_proxy(&args[2..])),
        Some("ssh-config") => std::process::exit(cli_ssh_config(&args[2..])),
        Some("init") => std::process::exit(cli_init(&args[2..])),
        Some("doctor") => std::process::exit(cli_doctor()),
        Some("up") => std::process::exit(cli_up(&args[2..])),
        Some("down") => std::process::exit(cli_down(&args[2..])),
        Some("rm") => std::process::exit(cli_rm(&args[2..])),
        Some("status") => std::process::exit(cli_status(&args[2..])),
        Some("logs") => std::process::exit(cli_logs(&args[2..])),
        Some("ps") => std::process::exit(cli_ps()),
        Some("ssh") => std::process::exit(cli_ssh(&args[2..])),
        Some("policy") => std::process::exit(cli_policy(&args[2..])),
        Some("why") => std::process::exit(cli_why(&args[2..])),
        Some("help") | Some("--help") | Some("-h") => {
            println!("{USAGE}");
            std::process::exit(0);
        }
        Some("--version") | Some("-V") => {
            println!("devcroft {}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }
        None => {
            // Usage on stderr and exit 2, where an explicit `help` gets
            // stdout and 0: one is a user asking a question and the other
            // is a malformed invocation, and a script that pipes this
            // should be able to tell them apart. Exit 2 is the error
            // contract's usage code (CLAUDE.md).
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
        Some(other) => {
            eprintln!("devcroft: unknown command {other:?}\n\n{USAGE}");
            std::process::exit(2);
        }
    }
}

/// The top-level usage text.
///
/// It exists because `src/lib.rs` tells readers to depend on "the
/// `devcroft` binary and its documented command surface (`devcroft
/// --help`, and the README)" — and until the first release audit ran the
/// packaged binary, `devcroft --help` answered `unknown command "--help"`.
/// The old fallback also pointed a user of a published binary at "the cli
/// spec", which ships in the repository and not in the crate.
///
/// Hidden `__`-prefixed modes are deliberately absent: they are re-exec
/// targets for devcroft's own internals, not commands anyone types.
const USAGE: &str = "\
devcroft — isolated, reproducible development environments, each reachable over SSH

usage: devcroft <command> [args...]

sandboxes
  init [--force]              write a devcroft.toml for this project
  up [name] [--recreate]      build the environment, apply the policy, start the sandbox
  down [name]                 stop a sandbox, keeping its state
  rm [name] [--yes]           stop a sandbox and delete its state

running things
  exec [name] -- <cmd>        run one command inside a sandbox
  shell [name]                open an interactive shell inside a sandbox

inspecting
  status [name]               whether a sandbox is up, and since when
  logs [name] [--tail N]      the keeper's log
  ps                          every sandbox on this host
  policy --render [name]      the compiled profile, every rule with its origin
  why --path P --op <mode>    whether one operation is allowed, and which rule decides
  why --host <domain>         the same question for an outbound host
  doctor                      check this host for what devcroft needs

ssh
  ssh [name]                  connect over the sandbox's own SSH server
  ssh-config [--write]        emit (or install) the ~/.ssh/config block
  proxy <name>.devcroft       ProxyCommand handler; not typed directly

  help                        this text
  --version                   print the version

Misusing a command prints that command's own usage line, which carries
the flags this summary leaves out. Full documentation:
https://github.com/dragosv/devcroft";

/// `devcroft exec [--no-up] [name] -- <cmd> [args...]` (task 5.1, plus
/// auto-up from task 5.3). Returns the process exit code directly,
/// matching the exec spec's "returns the command's exit code as its
/// own" — the child's exit status is passed through as-is and
/// deliberately does not follow CLAUDE.md's 0-5 layered contract, which
/// is for devcroft's own failures, not what it's asked to run.
fn cli_exec(args: &[String]) -> i32 {
    const USAGE: &str = "devcroft exec: usage: devcroft exec [--no-up] [name] -- <cmd> [args...]";

    let Some(sep) = args.iter().position(|a| a == "--") else {
        eprintln!("{USAGE}");
        return 2;
    };
    let (name_args, rest) = args.split_at(sep);
    let command_args = &rest[1..];
    // `--no-up` only counts ahead of `--`: the exec spec's own command
    // could legitimately want to pass that literal string to whatever
    // it's running (`exec -- mytool --no-up`), so only the devcroft-level
    // arguments before the separator are ever inspected for it.
    let no_up = name_args.iter().any(|a| a == "--no-up");
    let name_args: Vec<&String> = name_args.iter().filter(|a| *a != "--no-up").collect();
    if name_args.len() > 1 {
        eprintln!("{USAGE}");
        return 2;
    }
    let Some((cmd, cmd_rest)) = command_args.split_first() else {
        eprintln!("devcroft exec: no command given after `--`");
        return 2;
    };

    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("devcroft exec: cannot determine current directory: {e}");
            return 1;
        }
    };

    let sandbox_name = match name_args.first() {
        Some(name) => (*name).clone(),
        None => match resolve_sandbox_name(&cwd) {
            Ok(name) => name,
            Err(msg) => {
                eprintln!("devcroft exec: {msg}");
                return 2;
            }
        },
    };

    if !no_up && let Err(msg) = maybe_auto_up(&sandbox_name, &cwd) {
        eprintln!("devcroft exec: {msg}");
        return 3; // environment/provider layer, per CLAUDE.md's error contract
    }

    // No path remapping between host and sandbox (design.md decision 5:
    // MVP is access-restricted, not namespace-isolated) — the real host
    // cwd is already the right path inside the sandbox too, which is
    // exactly the exec spec's "Working directory mapping" scenario.
    let req = devcroft::exec::ExecRequest {
        cmd: cmd.clone(),
        args: cmd_rest.to_vec(),
        cwd: cwd.to_string_lossy().into_owned(),
    };

    match devcroft::exec::exec(&sandbox_name, &req) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("devcroft exec: {e}");
            5 // keeper/connection layer, per CLAUDE.md's error contract
        }
    }
}

/// `devcroft shell [--no-up] [name]` (task 5.2, plus auto-up from task
/// 5.3).
fn cli_shell(args: &[String]) -> i32 {
    const USAGE: &str = "devcroft shell: usage: devcroft shell [--no-up] [name]";
    let no_up = args.iter().any(|a| a == "--no-up");
    let name_args: Vec<&String> = args.iter().filter(|a| *a != "--no-up").collect();
    if name_args.len() > 1 {
        eprintln!("{USAGE}");
        return 2;
    }

    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("devcroft shell: cannot determine current directory: {e}");
            return 1;
        }
    };

    let sandbox_name = match name_args.first() {
        Some(name) => (*name).clone(),
        None => match resolve_sandbox_name(&cwd) {
            Ok(name) => name,
            Err(msg) => {
                eprintln!("devcroft shell: {msg}");
                return 2;
            }
        },
    };

    if !no_up && let Err(msg) = maybe_auto_up(&sandbox_name, &cwd) {
        eprintln!("devcroft shell: {msg}");
        return 3; // environment/provider layer, per CLAUDE.md's error contract
    }

    let req = devcroft::exec::ShellRequest {
        cwd: cwd.to_string_lossy().into_owned(),
    };

    match devcroft::exec::shell(&sandbox_name, &req) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("devcroft shell: {e}");
            5 // keeper/connection layer, per CLAUDE.md's error contract
        }
    }
}

/// `devcroft proxy [--no-up] <name>.devcroft` (ssh spec's "ProxyCommand
/// bridging" requirement, task 6.2): parses the sandbox name out of the
/// host argument, auto-ups the same way `exec`/`shell` do (unless
/// `--no-up`), then bridges this process's stdio to that sandbox's ssh
/// socket. Invoked by a real ssh client via the `ssh-config` block's
/// `ProxyCommand`, never directly by a user.
fn cli_proxy(args: &[String]) -> i32 {
    const USAGE: &str = "devcroft proxy: usage: devcroft proxy [--no-up] <name>.devcroft";
    let no_up = args.iter().any(|a| a == "--no-up");
    let host_args: Vec<&String> = args.iter().filter(|a| *a != "--no-up").collect();
    let &[host] = host_args.as_slice() else {
        eprintln!("{USAGE}");
        return 2;
    };

    let sandbox_name = match devcroft::ssh::sandbox_name_from_host(host) {
        Ok(name) => name.to_string(),
        Err(msg) => {
            eprintln!("devcroft proxy: {msg}");
            return 2;
        }
    };

    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("devcroft proxy: cannot determine current directory: {e}");
            return 1;
        }
    };

    // ssh spec's key-management requirement: any ssh-related command
    // ensures the client keypair exists before proceeding, so the real
    // ssh client on the other end of stdio finds its `IdentityFile` in
    // place by the time it gets to authentication.
    if let Err(e) = ensure_client_keypair() {
        eprintln!("devcroft proxy: {e}");
        return 3;
    }

    if !no_up && let Err(msg) = maybe_auto_up(&sandbox_name, &cwd) {
        eprintln!("devcroft proxy: {msg}");
        return 3; // environment/provider layer, per CLAUDE.md's error contract
    }

    match devcroft::ssh::proxy(&sandbox_name) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("devcroft proxy: {e}");
            5 // keeper/connection layer, per CLAUDE.md's error contract
        }
    }
}

/// `devcroft ssh-config [--write]` (ssh spec's "ssh-config emission"
/// requirement, task 6.2): prints design.md decision 3's wildcard `Host
/// *.devcroft` block, or with `--write`, idempotently inserts/updates it
/// as a marker-delimited managed section in `~/.ssh/config`.
fn cli_ssh_config(args: &[String]) -> i32 {
    const USAGE: &str = "devcroft ssh-config: usage: devcroft ssh-config [--write]";
    let write = match args {
        [] => false,
        [flag] if flag == "--write" => true,
        _ => {
            eprintln!("{USAGE}");
            return 2;
        }
    };

    if let Err(e) = ensure_client_keypair() {
        eprintln!("devcroft ssh-config: {e}");
        return 3;
    }
    let (private_path, _) = match devcroft::lifecycle::client_key_paths() {
        Ok(paths) => paths,
        Err(e) => {
            eprintln!("devcroft ssh-config: resolving client key path: {e}");
            return 1;
        }
    };
    let identity_file = private_path.to_string_lossy().into_owned();

    if !write {
        print!("{}", devcroft::ssh::render_ssh_config(&identity_file));
        return 0;
    }

    let config_path = match ssh_config_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("devcroft ssh-config: {e}");
            return 1;
        }
    };
    if let Err(e) = devcroft::ssh::write_ssh_config(&config_path, &identity_file) {
        eprintln!(
            "devcroft ssh-config: writing {}: {e}",
            config_path.display()
        );
        return 1;
    }
    println!(
        "devcroft: wrote the devcroft block to {}",
        config_path.display()
    );
    0
}

fn ssh_config_path() -> Result<std::path::PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    Ok(std::path::PathBuf::from(home).join(".ssh/config"))
}

/// The ssh spec's "First run generates keys" scenario: any ssh-related
/// command (`proxy`, `ssh-config`) ensures the client keypair exists
/// before proceeding, rather than each having its own copy of this check.
fn ensure_client_keypair() -> Result<(), String> {
    let (private_path, public_path) = devcroft::lifecycle::client_key_paths()
        .map_err(|e| format!("resolving client key path: {e}"))?;
    devcroft::ssh::ensure_client_keypair(&private_path, &public_path)
        .map_err(|e| format!("ensuring ssh client keypair: {e}"))?;
    Ok(())
}

/// `init`'s default name is the directory slug alone, so two unrelated
/// projects that happen to share a leaf directory name (`~/proiect-A/api`,
/// `~/proiect-B/api`) would otherwise collide on the same state dir and
/// control socket (`StatePaths::new` derives both from the name alone).
/// Only disambiguates on a *real* collision — state already exists for
/// `base` and belongs to a different project root — so the common case (one
/// project, or re-running `init` in the same one) keeps the plain slug.
/// A project that has never been `up` yet leaves no meta to check against;
/// that residual race is intentionally left to the operator (pick an
/// explicit `[sandbox].name`), not solved by scanning every sandbox's state
/// on every `init`.
fn disambiguate_name(base: &str, project_root: &std::path::Path) -> String {
    let Ok(paths) = devcroft::lifecycle::StatePaths::new(base) else {
        return base.to_string();
    };
    let Ok(Some(meta)) = devcroft::lifecycle::read_meta(&paths.meta) else {
        return base.to_string();
    };
    if meta.project_root == project_root.to_string_lossy() {
        return base.to_string();
    }

    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    project_root.hash(&mut hasher);
    let suffix = format!("{:06x}", hasher.finish() & 0xffffff);

    let mut trimmed = base;
    while trimmed.len() > 32 - 1 - suffix.len() {
        trimmed = &trimmed[..trimmed.len() - 1];
    }
    let trimmed = trimmed.trim_end_matches('-');
    format!("{trimmed}-{suffix}")
}

/// `devcroft init [--force]` (cli spec's "init" requirement, task 7.1):
/// detects an existing flox environment or a single-ecosystem toolchain
/// pin, generates a minimal `devcroft.toml`, and never overwrites an
/// existing one without `--force`.
/// Adds `.devcroft/` to the project's `.gitignore`, since `up` writes the
/// service supervisor's generated config, log, and unix socket there.
///
/// It has to be the project root — the state dir is baseline-denied to
/// the sandbox and `/tmp` is write-but-not-read under the baseline, so
/// it is the only place process-compose can both read its config and
/// bind its socket (`services::ARTIFACT_DIR`). That makes devcroft the
/// one writing untracked files into the user's working tree, and
/// therefore the one responsible for not leaving `git status` dirty —
/// which matters most in exactly the worktree-heavy fan-out flow
/// `add-agent-workload` targets, where it would be dirty in every one.
///
/// Best-effort and never fatal: appends only if not already covered, and
/// creates the file only when the project is a git repository, so this
/// never invents a `.gitignore` in a directory git does not track.
fn ignore_artifact_dir(project_root: &std::path::Path) {
    let entry = format!("{}/", devcroft::services::ARTIFACT_DIR);
    let gitignore = project_root.join(".gitignore");
    let existing = std::fs::read_to_string(&gitignore).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == entry) {
        return;
    }
    if existing.is_empty() && !project_root.join(".git").exists() {
        return;
    }

    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str("\n# devcroft's generated service config, log, and supervisor socket\n");
    next.push_str(&entry);
    next.push('\n');

    if std::fs::write(&gitignore, next).is_ok() {
        println!("devcroft: added {entry} to {}", gitignore.display());
    }
}

fn cli_init(args: &[String]) -> i32 {
    const USAGE: &str = "devcroft init: usage: devcroft init [--force]";
    let force = match args {
        [] => false,
        [flag] if flag == "--force" => true,
        _ => {
            eprintln!("{USAGE}");
            return 2;
        }
    };

    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("devcroft init: cannot determine current directory: {e}");
            return 1;
        }
    };

    let manifest_path = cwd.join(devcroft::config::MANIFEST_FILE_NAME);
    if manifest_path.exists() && !force {
        eprintln!(
            "devcroft init: {} already exists; use --force to overwrite",
            manifest_path.display()
        );
        return 2;
    }

    let dir_name = cwd
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sandbox".to_string());
    let base_name = devcroft::config::slugify(&dir_name);
    let name = disambiguate_name(&base_name, &cwd);

    // cli spec's init scenarios: flox, then devbox, then a bare flake — a
    // deterministic tiebreak, not a judgement that the losers are derived
    // artifacts (an earlier draft justified ranking devbox above a flake
    // by claiming a root flake.nix in a devbox project is usually
    // generated from devbox.json; devbox writes its generated flake under
    // .devbox/gen/flake/, never to the project root, so that reasoning is
    // false and is not restated — only the ordering survives). Any one of
    // the three supersedes advice about a toolchain pin it would
    // otherwise just be a fallback for.
    let has_flox = cwd.join(".flox").is_dir();
    let has_devbox = cwd.join("devbox.json").is_file();
    let has_flake = cwd.join("flake.nix").is_file();
    let provider = if has_flox {
        "flox"
    } else if has_devbox {
        "devbox"
    } else if has_flake {
        "nix"
    } else {
        "flox"
    };

    let manifest_toml = format!(
        "[sandbox]\nname = {name:?}\n\n[env]\nprovider = {provider:?}\n\n\
         # Uncomment to grant more than the project root (already allowed\n\
         # by default) or to change the network default:\n\
         #\n\
         # [filesystem]\n\
         # allow = [\".\"]\n\
         # read = [\"/path/to/a/read-only/dir\"]\n\
         #\n\
         # [network]\n\
         # default = \"deny\"\n\
         # allow = [\"api.example.com\"]\n"
    );

    if let Err(e) = std::fs::write(&manifest_path, &manifest_toml) {
        eprintln!("devcroft init: writing {}: {e}", manifest_path.display());
        return 1;
    }
    println!("devcroft: wrote {}", manifest_path.display());
    ignore_artifact_dir(&cwd);

    if has_flox {
        println!("devcroft: found an existing flox environment (.flox/); ready for `devcroft up`.");
        if has_devbox {
            println!(
                "devcroft: a devbox project (devbox.json) was also found; `provider = \"devbox\"` \
                 is available if you'd rather use that instead."
            );
        }
        if has_flake {
            println!(
                "devcroft: a nix flake (flake.nix) was also found; `provider = \"nix\"` is \
                 available if you'd rather use that instead."
            );
        }
    } else if has_devbox {
        // Any devbox project without a lockfile, not only one declaring
        // packages: devbox's stdenv comes from a base nixpkgs entry that
        // stays the floating `nixpkgs-unstable` branch until `devbox
        // install` pins it, so a zero-package project has something to
        // resolve too (env-provider spec; design.md decision 1c).
        if !cwd.join("devbox.lock").is_file() {
            println!(
                "devcroft: found an existing devbox project (devbox.json) with no devbox.lock."
            );
            println!("devcroft: run `devbox install` before `devcroft up`.");
        } else {
            println!(
                "devcroft: found an existing devbox project (devbox.json); ready for `devcroft up`."
            );
        }
        if has_flake {
            println!(
                "devcroft: a nix flake (flake.nix) was also found; `provider = \"nix\"` is \
                 available if you'd rather use that instead."
            );
        }
    } else if has_flake {
        if cwd.join("flake.lock").is_file() {
            println!(
                "devcroft: found an existing nix flake (flake.nix) with flake.lock; ready for \
                 `devcroft up`."
            );
        } else {
            println!("devcroft: found flake.nix but no flake.lock.");
            println!("devcroft: run `nix flake lock` before `devcroft up`.");
        }
    } else if cwd.join("rust-toolchain.toml").exists() {
        println!("devcroft: found rust-toolchain.toml but no .flox/ environment.");
        println!(
            "devcroft: rustup alone can't provide a complete build environment (no C toolchain, \
             no other language runtimes) — run `flox init`, then add the pinned Rust channel \
             (via the fenix or rust-overlay flox package) before `devcroft up`."
        );
    } else if cwd.join(".nvmrc").exists() {
        println!("devcroft: found .nvmrc but no .flox/ environment.");
        println!(
            "devcroft: nvm alone can't provide a complete build environment — run `flox init`, \
             then pin the Node.js version from .nvmrc before `devcroft up`."
        );
    } else if cwd.join(".python-version").exists() {
        println!("devcroft: found .python-version but no .flox/ environment.");
        println!(
            "devcroft: pyenv alone can't provide a complete build environment — run `flox init`, \
             then pin the Python version from .python-version before `devcroft up`."
        );
    } else {
        println!("devcroft: no .flox/ environment found; run `flox init` before `devcroft up`.");
    }
    0
}

/// `__bind_probe <port>`: applies a deny-default network policy that
/// grants exactly one loopback port, then tries to bind it. Exit 0 means
/// binding works under a deny-default policy on this host; anything else
/// means it does not.
///
/// A separate process because the restriction is **irreversible** —
/// applying it inside `doctor` would leave every later check running
/// under a sandbox policy, which is both wrong and undebuggable. Same
/// reasoning, and the same hidden-subcommand shape, as `__keeper`.
///
/// The probe is the real mechanism, not an approximation: it builds the
/// same `CapabilityPlan` a manifest with `network.default = "deny"` and
/// `ports = [N]` compiles to, so a host where this fails is exactly a
/// host where such a manifest's services fail to bind.
fn bind_probe_main(args: &[String]) -> i32 {
    let Some(port) = args.first().and_then(|p| p.parse::<u16>().ok()) else {
        eprintln!("devcroft __bind_probe: usage: devcroft __bind_probe <port>");
        return 2;
    };

    let plan = devcroft::policy::CapabilityPlan {
        filesystem_allow: Vec::new(),
        filesystem_read: Vec::new(),
        filesystem_deny: Vec::new(),
        network_block: true,
        network_ports: vec![port],
        network_proxy_port: None,
        // The probe is about TCP bind, which no unix-socket grant
        // affects.
        unix_socket_bind: Vec::new(),
        // The only value `to_capability_set` accepts; anything else
        // panics there rather than silently meaning something.
        signal_mode: "isolated".to_string(),
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
    let Ok(caps) = plan.to_capability_set(&cwd) else {
        return 1;
    };
    if nono::Sandbox::apply_auto(&caps).is_err() {
        return 1;
    }

    match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(_) => 0,
        Err(_) => 1,
    }
}

/// Whether this host could give a fleet agent its own network namespace,
/// which is what lets N agents each bind the same service port
/// (`add-linux-agent-fleet`'s `service-ports`) — **and**, on the same
/// line, whether it can give every sandbox its own mount namespace
/// (`add-mount-isolation`'s `filesystem-view`).
///
/// One probe, one report, deliberately (design.md M4: "extends the same
/// report rather than adding a second probe"). Both rest on the identical
/// unprivileged user namespace — `netns::probe`'s own child adds
/// `CLONE_NEWNET` to that `unshare()` call, `fleet::mount`'s adds
/// `CLONE_NEWNS`, and the three things that can independently deny either
/// (a container runtime's seccomp profile, an AppArmor policy restricting
/// unprivileged user namespaces, `max_user_namespaces`) all gate the user
/// namespace itself, not which companion namespace type rides with it. A
/// second fork here to prove that again would cost without telling the
/// operator anything the first one didn't already.
///
/// Reported as `[INFO]`, never `[FAIL]`, for network isolation:  `up`
/// already degrades gracefully when it is unavailable (a qualifying
/// sandbox falls back to the host's shared port table with a warning
/// rather than failing), so a host that cannot do this is not broken —
/// it just does not get the port-isolation fix for sandboxes with
/// `network.default = "deny"`, no `network.allow`, and declared services
/// or ports. Mount isolation has no such fallback (design.md M4: it fails
/// closed rather than starting a sandbox weaker than its rendered
/// policy claims), so this same unavailability is what `up` reports as a
/// hard failure for every sandbox on such a host, not a degrade.
///
/// No longer fleet-only, and this doc corrected accordingly: `up` itself
/// uses network isolation today for any qualifying single sandbox
/// (`CompiledPolicy::wants_network_isolation`), which is what makes two
/// sandboxes each declaring Postgres on 5432 stop colliding, and mount
/// isolation for every sandbox unconditionally. Fleet, once built, is a
/// second consumer of both primitives.
///
/// Probed by attempting it in a throwaway child rather than reading a
/// sysctl: seccomp, AppArmor and `max_user_namespaces` can each deny it
/// independently, and no single readable value predicts all three — the
/// same reason `doctor_backend` probes Landlock for real.
fn doctor_agent_namespaces() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    match devcroft::fleet::netns::probe(&exe) {
        Ok(true) => println!(
            "[INFO] namespaces: available on this host — sandboxes with \
             `network.default = \"deny\"`, no `network.allow`, and services or \
             `network.ports` each get their own port table, and every sandbox \
             gets its own filesystem view (mount isolation)"
        ),
        Ok(false) | Err(_) => println!(
            "[INFO] namespaces: unavailable on this host; qualifying sandboxes \
             fall back to the host's shared port table for network isolation \
             (and `up` warns when that happens), and `up` fails outright for \
             mount isolation rather than starting a sandbox weaker than its \
             rendered policy claims"
        ),
    }
}

/// Two probes, deliberately separated — see the `--reachable` note.
///
/// Default (no argument): can this host create a per-agent network
/// namespace *at all*? Nothing more. This is the **capability** gate: a
/// container runtime's seccomp profile, an AppArmor policy restricting
/// unprivileged user namespaces, or an exhausted `max_user_namespaces`
/// can each deny it, and none is a devcroft bug.
///
/// `--reachable`: the full thing — namespace, loopback up, bind, and a
/// real connection. This is the **behaviour** under test.
///
/// **They must stay separate, and that was learned the hard way here.**
/// The first version of this probe did both at once, and
/// `tests/fleet_netns.rs` used it for its skip guard *and* its
/// assertion. Disabling `bring_loopback_up` to check the tests had
/// teeth, they all reported `ok` — the guard had seen the same failure
/// and skipped every test silently. A regression in the feature was
/// indistinguishable from a host that cannot run it, which is the
/// failure mode this project keeps finding elsewhere (a check that
/// cannot fail on the machine you develop on) reproduced in a brand-new
/// test. The gate must therefore depend on strictly less than what the
/// test asserts.
///
/// The reachable probe binds port 0, not a fixed number: it is
/// demonstrating namespace setup, and a specific port could collide with
/// something on the *host* and read as a namespace failure. Concurrent
/// agents deliberately binding the *same* number is asserted separately,
/// where collision is the subject rather than noise.
/// Probe: with a Landlock policy applied, can this process still
/// `connect()` to a pathname unix socket?
///
/// Argument 1 is the socket path. `--grant` additionally grants that path
/// read-write before restricting; without it the path is ungranted and
/// only the project root (this process's cwd) is allowed.
///
/// Exit 0 = connected, 1 = refused, 2 = setup failure. The two runs
/// together answer the design question: if the ungranted run *also*
/// connects, Landlock does not mediate AF_UNIX connect at all and no
/// grant is needed; if it refuses and the granted run succeeds, a grant
/// is both necessary and sufficient.
/// Probe: with a Landlock policy applied *and this sandbox's mount view
/// constructed* (`add-mount-isolation` task 4.1 — this probe used to
/// apply Landlock alone, which is exactly the gap
/// `tests/unix_socket_not_mediated.rs` was written to measure), can this
/// process still `connect()` to a pathname unix socket?
///
/// Argument 1 is the socket path. `--grant` additionally grants that path
/// read-write in *both* the mount view and the `CapabilitySet` before
/// restricting; without it the path is ungranted in both and only the
/// current directory is allowed. The two are built from the identical
/// grant list, same reasoning as `up`'s own `resolved_grants`/
/// `to_capability_set` split (`policy/capability_set.rs`): a probe that
/// let the two diverge would prove nothing about what a real sandbox
/// does, only about whatever inconsistency crept into this file.
///
/// Exit 0 = connected, 1 = refused, 2 = setup failure. Before this
/// inversion, the *un*granted run connecting proved Landlock does not
/// mediate AF_UNIX connect at all; now, with the mount view in place
/// too, the ungranted run refusing is what proves the gap has closed.
fn uds_probe_main(args: &[String]) -> i32 {
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    let Some(sock) = args.first() else {
        eprintln!("devcroft __uds_probe: usage: <socket-path> [--grant]");
        return 2;
    };
    let grant = args.iter().any(|a| a == "--grant");

    // Always grant cwd, so the process can function at all — mirrors a
    // real sandbox, where the project root is granted by default.
    let Ok(cwd) = std::env::current_dir() else {
        return 2;
    };
    let mut grants = vec![devcroft::policy::ResolvedGrant {
        path: cwd,
        mode: nono::AccessMode::ReadWrite,
    }];
    if grant {
        grants.push(devcroft::policy::ResolvedGrant {
            path: std::path::PathBuf::from(sock),
            mode: nono::AccessMode::ReadWrite,
        });
    }

    // Mount isolation first, matching the real ordering (`up.rs`'s
    // `pre_exec`: view constructed, then exec, then the keeper applies
    // Landlock to itself) — a `pivot_root`ed process is what then
    // self-restricts, not the other way around.
    if let Err(e) = devcroft::fleet::mount::enter_mount_namespace() {
        eprintln!("devcroft __uds_probe: entering mount namespace: {e}");
        return 2;
    }
    if let Err(e) = devcroft::fleet::mount::make_propagation_private() {
        eprintln!("devcroft __uds_probe: making propagation private: {e}");
        return 2;
    }
    let new_root =
        std::env::temp_dir().join(format!("devcroft-uds-probe-root-{}", std::process::id()));
    if std::fs::create_dir_all(&new_root).is_err() {
        return 2;
    }
    if let Err(e) = devcroft::fleet::mount::construct_view(&new_root, &grants, None) {
        eprintln!("devcroft __uds_probe: constructing view: {e}");
        return 2;
    }

    let mut caps = nono::CapabilitySet::new();
    for g in &grants {
        // A unix socket special file is not a directory — `allow_file`,
        // not `allow_path`, matching `capability_set.rs`'s own `grant()`.
        let result = if g.path.is_dir() {
            caps.allow_path(&g.path, g.mode)
        } else {
            caps.allow_file(&g.path, g.mode)
        };
        caps = match result {
            Ok(c) => c,
            Err(e) => {
                eprintln!("devcroft __uds_probe: granting {}: {e}", g.path.display());
                return 2;
            }
        };
    }
    if let Err(e) = nono::Sandbox::apply_auto(&caps) {
        eprintln!("devcroft __uds_probe: apply_auto failed: {e}");
        return 2;
    }

    match UnixStream::connect(sock) {
        Ok(mut s) => {
            let _ = s.write_all(b"ping");
            0
        }
        Err(e) => {
            eprintln!("devcroft __uds_probe: connect refused: {e}");
            1
        }
    }
}

/// Probe: with devcroft's *default* `CapabilitySet` applied — the same
/// one `CapabilityPlan::to_capability_set` builds, `set_signal_mode`
/// called and nothing else, `IpcMode` left at whatever `nono` defaults
/// to — can this process still `connect()` to an *abstract* unix socket
/// (`@`-prefixed, no filesystem path)?
///
/// No mount view involved, deliberately: an abstract socket has no path
/// for `fleet::mount::construct_view` to remove, so this measures
/// Landlock/Seatbelt alone, exactly like `__uds_probe` did before
/// `add-mount-isolation` existed.
///
/// Argument 1 is the abstract name (no leading NUL — `SocketAddrExt`
/// adds it). Exit 0 = connected, 1 = refused, 2 = setup failure.
fn abstract_socket_probe_main(args: &[String]) -> i32 {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixStream};

    let Some(name) = args.first() else {
        eprintln!("devcroft __abstract_socket_probe: usage: <abstract-name>");
        return 2;
    };
    let Ok(addr) = SocketAddr::from_abstract_name(name.as_bytes()) else {
        eprintln!("devcroft __abstract_socket_probe: invalid abstract name {name}");
        return 2;
    };

    let Ok(cwd) = std::env::current_dir() else {
        return 2;
    };
    let caps = nono::CapabilitySet::new();
    let Ok(caps) = caps.allow_path(&cwd, nono::AccessMode::ReadWrite) else {
        return 2;
    };
    // The one knob devcroft's real compiled policy sets
    // (`capability_set.rs`'s `to_capability_set`) — everything else,
    // `IpcMode` included, stays at whatever this pinned `nono` defaults
    // to, exactly as a real sandbox gets it.
    let caps = caps.set_signal_mode(nono::SignalMode::Isolated);
    if let Err(e) = nono::Sandbox::apply_auto(&caps) {
        eprintln!("devcroft __abstract_socket_probe: apply_auto failed: {e}");
        return 2;
    }

    match UnixStream::connect_addr(&addr) {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("devcroft __abstract_socket_probe: connect refused: {e}");
            1
        }
    }
}

fn netns_probe_main(args: &[String]) -> i32 {
    use std::io::{Read, Write};

    if devcroft::fleet::netns::enter_network_namespace().is_err() {
        return 1;
    }
    // Capability gate: creating the namespace is the whole question.
    if !args.iter().any(|a| a == "--reachable") {
        return 0;
    }
    if devcroft::fleet::netns::bring_loopback_up().is_err() {
        return 1;
    }

    let Ok(listener) = std::net::TcpListener::bind("127.0.0.1:0") else {
        return 1;
    };
    let Ok(addr) = listener.local_addr() else {
        return 1;
    };
    std::thread::spawn(move || {
        if let Ok((mut conn, _)) = listener.accept() {
            let _ = conn.write_all(b"PONG");
        }
    });

    let Ok(mut client) = std::net::TcpStream::connect(addr) else {
        return 1;
    };
    let mut buf = [0u8; 4];
    match client.read_exact(&mut buf) {
        Ok(()) if &buf == b"PONG" => 0,
        _ => 1,
    }
}

/// Simulates one fleet agent: enter a namespace, bring loopback up, bind
/// the given port, and prove this agent reaches *its own* listener.
///
/// Prints `served-by-<agent>` so the caller can verify identity rather
/// than mere success — five agents that each bound something prove
/// nothing if they all reached the same listener, which is exactly what
/// a broken namespace setup would look like.
///
/// With `--hold`, prints `READY` and then blocks, so a caller can test
/// what is reachable *from outside* while the listener is genuinely
/// live. Readiness is signalled rather than slept on: a fixed sleep
/// would make the caller's assertion depend on machine speed.
fn netns_agent_sim_main(args: &[String]) -> i32 {
    use std::io::{Read, Write};

    let Some(agent) = args.first() else {
        eprintln!("devcroft __netns_agent_sim: usage: <agent> <port> [--hold]");
        return 2;
    };
    let Some(port) = args.get(1).and_then(|p| p.parse::<u16>().ok()) else {
        eprintln!("devcroft __netns_agent_sim: usage: <agent> <port> [--hold]");
        return 2;
    };
    let hold = args.iter().any(|a| a == "--hold");

    if let Err(e) = devcroft::fleet::netns::enter_network_namespace() {
        eprintln!("entering namespace: {e}");
        return 1;
    }
    if let Err(e) = devcroft::fleet::netns::bring_loopback_up() {
        eprintln!("bringing loopback up: {e}");
        return 1;
    }

    let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("binding 127.0.0.1:{port}: {e}");
            return 1;
        }
    };

    let reply = format!("served-by-{agent}");
    let served = reply.clone();
    std::thread::spawn(move || {
        while let Ok((mut conn, _)) = listener.accept() {
            let _ = conn.write_all(served.as_bytes());
        }
    });

    if hold {
        print!("READY");
        let _ = std::io::stdout().flush();
        std::thread::sleep(std::time::Duration::from_secs(30));
        return 0;
    }

    let mut client = match std::net::TcpStream::connect(("127.0.0.1", port)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("connecting to own listener: {e}");
            return 1;
        }
    };
    let mut buf = String::new();
    if let Err(e) = client.read_to_string(&mut buf) {
        eprintln!("reading from own listener: {e}");
        return 1;
    }
    print!("{buf}");
    if buf == reply { 0 } else { 1 }
}

/// Capability gate: can this host enter a private mount namespace at all?
/// Nothing more — mirrors `netns_probe_main`'s own split between "can this
/// happen" and "does it behave", and for the identical reason
/// (`tests/fleet_netns.rs`'s doc comment on `namespaces_available`): a gate
/// must depend on strictly less than what the tests it guards assert, or a
/// regression in the feature reads as an unsupported host.
fn mount_probe_main(_args: &[String]) -> i32 {
    if devcroft::fleet::mount::enter_mount_namespace().is_err() {
        return 1;
    }
    if devcroft::fleet::mount::make_propagation_private().is_err() {
        return 1;
    }
    0
}

/// Proves a mount made after [`devcroft::fleet::mount::make_propagation_private`]
/// does not leak to the host's own mount namespace — the property the
/// primitive exists for, not merely that the two calls returned `Ok`.
///
/// Mounts a `tmpfs` over the given (pre-existing, host-visible) directory
/// and writes a marker file into it, then signals `READY` and blocks.
/// The caller checks that same path from the host's own mount namespace:
/// if propagation were not private, the marker would be visible there too.
fn mount_isolation_sim_main(args: &[String]) -> i32 {
    use std::io::Write;

    let Some(dir) = args.first() else {
        eprintln!("devcroft __mount_isolation_sim: usage: <dir>");
        return 2;
    };

    if let Err(e) = devcroft::fleet::mount::enter_mount_namespace() {
        eprintln!("entering mount namespace: {e}");
        return 1;
    }
    if let Err(e) = devcroft::fleet::mount::make_propagation_private() {
        eprintln!("making propagation private: {e}");
        return 1;
    }

    let dir_c = match std::ffi::CString::new(dir.as_str()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("invalid path {dir}: {e}");
            return 2;
        }
    };
    let tmpfs = c"tmpfs";
    // SAFETY: `dir_c` and `tmpfs` are valid, NUL-terminated C strings; the
    // remaining arguments are null/zero as an ordinary `tmpfs` mount needs
    // no source device or extra data. `CAP_SYS_ADMIN` for this call comes
    // from the user namespace `enter_mount_namespace` already entered.
    let ret = unsafe {
        libc::mount(
            tmpfs.as_ptr(),
            dir_c.as_ptr(),
            tmpfs.as_ptr(),
            0,
            std::ptr::null(),
        )
    };
    if ret != 0 {
        eprintln!(
            "mounting tmpfs on {dir}: {}",
            std::io::Error::last_os_error()
        );
        return 1;
    }

    if let Err(e) = std::fs::write(std::path::Path::new(dir).join("marker"), b"present") {
        eprintln!("writing marker: {e}");
        return 1;
    }

    print!("READY");
    let _ = std::io::stdout().flush();
    std::thread::sleep(std::time::Duration::from_secs(30));
    0
}

/// Live verification for `fleet::mount::construct_view`, not a unit test.
///
/// Usage: `<project-root> <new-root-scratch-dir> [--provider-grant PATH]...
/// -- <cmd> [args...]`. Reads `<project-root>/devcroft.toml`, compiles it,
/// folds in each `--provider-grant` as though a provider resolved it
/// (`up` would call `CompiledPolicy::with_provider_grants` with the
/// provider's real resolution; this probe takes it as an argument since
/// it does not run a provider itself), resolves the grants, enters a
/// mount namespace, builds the view, and execs the given command inside
/// it. Exit code is the command's own — this is how task 4.4 ("a real
/// compile succeeds inside the view") gets checked before anything here
/// is trusted.
fn mount_view_probe_main(args: &[String]) -> i32 {
    let Some(project_root) = args.first() else {
        eprintln!(
            "devcroft __mount_view_probe: usage: <project-root> <new-root> \
             [--provider-grant PATH]... -- <cmd> [args...]"
        );
        return 2;
    };
    let Some(new_root) = args.get(1) else {
        eprintln!("devcroft __mount_view_probe: missing <new-root>");
        return 2;
    };

    let mut provider_grants = Vec::new();
    let mut proxy_socket = None;
    let mut i = 2;
    while let Some(a) = args.get(i) {
        if a == "--provider-grant" {
            let Some(v) = args.get(i + 1) else {
                eprintln!("devcroft __mount_view_probe: --provider-grant needs a value");
                return 2;
            };
            provider_grants.push(v.clone());
            i += 2;
        } else if a == "--proxy-socket" {
            let Some(v) = args.get(i + 1) else {
                eprintln!("devcroft __mount_view_probe: --proxy-socket needs a value");
                return 2;
            };
            proxy_socket = Some(std::path::PathBuf::from(v));
            i += 2;
        } else if a == "--" {
            i += 1;
            break;
        } else {
            eprintln!("devcroft __mount_view_probe: unexpected argument {a}");
            return 2;
        }
    }
    let command = &args[i..];
    let Some((cmd, cmd_args)) = command.split_first() else {
        eprintln!("devcroft __mount_view_probe: missing command after --");
        return 2;
    };

    // Must be absolute before `pivot_root`: this process's cwd stops
    // meaning what it used to the moment `construct_view` returns, so a
    // relative argument resolved *after* that point would resolve
    // against the new root instead of wherever the caller actually meant.
    let project_root = match std::fs::canonicalize(project_root) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("canonicalizing {project_root}: {e}");
            return 2;
        }
    };
    let project_root = project_root.as_path();
    let config_text = match std::fs::read_to_string(project_root.join("devcroft.toml")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("reading devcroft.toml: {e}");
            return 2;
        }
    };
    let (manifest, _warnings) = match devcroft::config::parse(&config_text) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("parsing devcroft.toml: {e}");
            return 2;
        }
    };
    let mut compiled = devcroft::policy::compile(&manifest);
    if !provider_grants.is_empty() {
        compiled = compiled.with_provider_grants(
            Box::leak(manifest.env.provider.clone().into_boxed_str()),
            &provider_grants,
        );
    }
    let plan = compiled.to_capability_plan();
    let grants = match plan.resolved_grants(project_root) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("resolving grants: {e}");
            return 2;
        }
    };

    if let Err(e) = devcroft::fleet::mount::enter_mount_namespace() {
        eprintln!("entering mount namespace: {e}");
        return 1;
    }
    if let Err(e) = devcroft::fleet::mount::make_propagation_private() {
        eprintln!("making propagation private: {e}");
        return 1;
    }
    if let Err(e) = std::fs::create_dir_all(new_root) {
        eprintln!("creating {new_root}: {e}");
        return 1;
    }
    if let Err(e) = devcroft::fleet::mount::construct_view(
        std::path::Path::new(new_root),
        &grants,
        proxy_socket.as_deref(),
    ) {
        eprintln!("constructing view: {e}");
        return 1;
    }

    let status = std::process::Command::new(cmd)
        .args(cmd_args)
        .current_dir(project_root)
        .status();
    match status {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("running {cmd}: {e}");
            1
        }
    }
}

/// Whether a sandbox on this host can bind a loopback listener under a
/// deny-default network policy (`cli` delta spec: "doctor reports whether
/// listening sockets work").
///
/// Returns `None` when the probe could not be run at all — distinct from
/// a probe that ran and said no, because reporting "listening sockets do
/// not work" on the basis of a failed fork would be a false diagnosis in
/// the one command whose job is to predict why `up` will fail.
fn probe_listening_socket() -> Option<bool> {
    // Port 0 is not usable here: the probe has to grant a *specific*
    // port in the policy and then bind that same one, which is what a
    // real `network.ports` entry does. A high, unusual port keeps the
    // chance of colliding with something already listening low; a
    // collision would report a false negative, which is why the caller
    // treats a single failure as inconclusive rather than as proof.
    const PROBE_PORT: u16 = 47823;
    let exe = std::env::current_exe().ok()?;
    let status = std::process::Command::new(exe)
        .arg("__bind_probe")
        .arg(PROBE_PORT.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    Some(status.success())
}

/// design.md decision 1's stated cost, surfaced rather than left in a
/// design doc: flox *declares* services, devcroft *supervises* them, so
/// `flox services status` run by hand reports nothing for a sandbox whose
/// services are running fine. Without this line the user's next move is
/// to conclude their services never started.
///
/// Scoped to projects that actually declare services — a project with
/// none does not need to be told who would have supervised them.
fn doctor_service_supervisor() {
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    let Ok(manifest_path) = devcroft::config::discover(&cwd) else {
        return;
    };
    let project_root = manifest_path.parent().unwrap_or(&cwd);
    let declared = devcroft::provider::services_declared_by_flox(project_root);
    if declared.is_empty() {
        return;
    }
    println!(
        "[INFO] services: {} declared ({}) — devcroft supervises these itself, so \
         `flox services status` will not list them; use `devcroft status`/`ps`/`logs`",
        declared.len(),
        declared.join(", ")
    );
}

fn doctor_listening_sockets() {
    match probe_listening_socket() {
        Some(true) => println!(
            "[PASS] listening sockets: a deny-default network policy still permits loopback \
             binding on this host, so provider-declared services that listen on a port work"
        ),
        Some(false) => {
            println!(
                "[WARN] listening sockets: this host denies bind/listen under \
                 `network.default = \"deny\"`, so a provider-declared service that listens on \
                 a port will fail to bind"
            );
            println!(
                "       workaround: `network.default = \"allow\"`, which restores binding but \
                 drops egress filtering entirely — every outbound destination becomes reachable"
            );
        }
        None => println!(
            "[INFO] listening sockets: the probe could not be run on this host; service \
             port binding is unverified"
        ),
    }
}

/// `devcroft doctor` (cli spec's "doctor" requirement, task 7.1): reports
/// backend presence/version, kernel sandboxing capability, the provider
/// binary, ssh-config managed-section state, and — if a manifest is
/// discoverable from cwd — which of its aspects would be degraded on this
/// host. Every failure names its fix (the spec's "output SHALL be
/// actionable"). `[WARN]` findings (missing ssh-config block, a degraded
/// capability) don't fail the command — sandboxes still work without
/// them; `[FAIL]` findings (missing/incompatible backend or provider)
/// do, since nothing works without those.
fn cli_doctor() -> i32 {
    println!("devcroft doctor");
    println!();

    let mut ok = true;
    ok &= doctor_backend();
    ok &= doctor_provider();
    ok &= doctor_nix_store();
    doctor_listening_sockets();
    doctor_agent_namespaces();
    doctor_ssh_config();
    doctor_manifest_degradation();
    doctor_service_supervisor();
    println!();
    doctor_backend_capabilities();

    println!();
    if ok {
        println!("devcroft: all checks passed.");
        0
    } else {
        println!("devcroft: one or more checks failed; see the FAIL lines above for the fix.");
        1
    }
}

/// `use-nono-library` task group 5: the process tier's backend is a
/// linked crate now, not a binary on `PATH` — there is no version to
/// probe and no schema to validate (lifecycle spec: "The process tier
/// requires no external backend binary"). What's left is a genuine
/// platform-support question, which the library answers directly:
/// `Sandbox::support_info()` reports Landlock ABI level on Linux or
/// Seatbelt availability on macOS, so this just surfaces its verdict
/// (policy spec: "Degraded capabilities are reported from the enforcement
/// layer... by asking the enforcement layer what the running platform
/// supports").
fn doctor_backend() -> bool {
    // Via the lib rather than `nono::` directly, so the integration
    // tests' own process-tier gate can ask the identical question — see
    // `policy::backend_support`.
    let support = devcroft::policy::backend_support();
    if support.is_supported {
        println!("[PASS] backend: {} — {}", support.platform, support.details);
        // Not a failure and not a warning `up` should repeat on every
        // run: a permanent property of this host that no manifest asks
        // for and none can avoid. `doctor` is the command that exists to
        // report what this machine can and cannot do, so it is where the
        // "degraded capabilities are surfaced, never silent" rule lands
        // for facts of this shape (`policy::host_limitations`).
        for limitation in devcroft::policy::host_limitations() {
            println!("[WARN] backend: {limitation}");
        }
    } else {
        println!(
            "[FAIL] backend: {} does not support sandboxing — {}",
            support.platform, support.details
        );
    }
    support.is_supported
}

/// `add-backend-capabilities` task 2.1/2.2: the declared capability
/// matrix (`backend_capabilities::capabilities()`) against what this
/// host can actually provide.
///
/// **Never fails `doctor`** — a `[FAIL]` here would mean "this host
/// cannot do something devcroft claims to enforce", which is real for
/// `NotAdopted`/`Unsupported`/`Unverified` entries by definition, not a
/// broken host. Reported as `[INFO]` throughout, matching
/// `doctor_agent_namespaces`'s own reasoning for the identical choice.
///
/// Spec: "`doctor` SHALL NOT probe the host" for `NotAdopted` entries —
/// enforced here by construction, not by a per-entry check: only
/// `Enforced`/`EnforcedWithNamedDegradation` entries ever reach
/// `probe_here()`, and `NotAdopted` entries never carry a probe function
/// at all (asserted in `backend_capabilities`'s own
/// `not_adopted_entries_carry_no_host_probe` test).
fn doctor_backend_capabilities() {
    use devcroft::backend_capabilities::{Status, capabilities};

    println!("capabilities (add-backend-capabilities):");
    for cap in capabilities() {
        let declared = cap.status_here();
        match declared.status {
            Status::NotAdopted => {
                println!("  [INFO] {:<32} not-adopted", cap.name);
            }
            Status::Unsupported => {
                println!("  [INFO] {:<32} unsupported on this platform", cap.name);
            }
            Status::Unverified => {
                println!(
                    "  [INFO] {:<32} unverified — declared status is a belief, not a measurement",
                    cap.name
                );
            }
            Status::Enforced | Status::EnforcedWithNamedDegradation => {
                let label = if declared.status == Status::Enforced {
                    "enforced"
                } else {
                    "enforced (degraded)"
                };
                match cap.probe_here() {
                    Some(probe) if probe() => {
                        println!(
                            "  [INFO] {:<32} {label}, and available on this host",
                            cap.name
                        );
                    }
                    Some(_) => {
                        println!(
                            "  [INFO] {:<32} {label} by devcroft, but UNAVAILABLE on this host \
                             — see `doctor`'s namespace/backend lines above for why",
                            cap.name
                        );
                    }
                    None => {
                        println!("  [INFO] {:<32} {label}", cap.name);
                    }
                }
            }
        }
    }
}

/// Checks the provider **this project actually declares**, not every
/// provider devcroft can drive.
///
/// This used to fail `doctor` whenever `flox` was absent, unconditionally
/// — so a project with `provider = "nix"`, on a host that deliberately
/// has no flox, was told its environment was broken. Worse, the check
/// short-circuited: a missing flox meant the nix probe never ran, so the
/// one provider that project depends on went entirely unreported. The
/// nix check had the rule right all along ("only needed for projects
/// with `provider = \"nix\"`"); it just wasn't applied symmetrically.
///
/// With a manifest, the declared provider is required and the others are
/// irrelevant. Without one, nothing is required — devcroft cannot know
/// what a future manifest will ask for, so both are reported as
/// information rather than as a verdict.
fn doctor_provider() -> bool {
    match discovered_provider() {
        Some(provider) => match provider.as_str() {
            "nix" => doctor_nix_provider(true),
            "devbox" => doctor_devbox_provider(true),
            // `config::parse` normalizes and rejects anything else, so
            // this is flox or a provider that could not exist.
            _ => doctor_flox_provider(true),
        },
        None => {
            println!(
                "[INFO] provider: no devcroft.toml found from here; \
                 reporting every provider, requiring none"
            );
            let flox = doctor_flox_provider(false);
            let nix = doctor_nix_provider(false);
            let devbox = doctor_devbox_provider(false);
            let _ = (flox, nix, devbox);
            true
        }
    }
}

/// The `env.provider` of the manifest discovered from the cwd, if any.
fn discovered_provider() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let path = devcroft::config::discover(&cwd).ok()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let (manifest, _warnings) = devcroft::config::parse(&text).ok()?;
    Some(manifest.env.provider)
}

/// `required` distinguishes "this project depends on it" (absence is a
/// `FAIL`) from "reporting what's available" (absence is a `WARN` and
/// does not fail `doctor`).
fn doctor_flox_provider(required: bool) -> bool {
    match std::process::Command::new("flox").arg("--version").output() {
        Ok(out) if out.status.success() => {
            println!(
                "[PASS] provider: flox found ({})",
                String::from_utf8_lossy(&out.stdout).trim()
            );
            true
        }
        _ if required => {
            println!("[FAIL] provider: flox not found on PATH — install it from https://flox.dev");
            false
        }
        _ => {
            println!(
                "[WARN] provider: flox not found on PATH — only needed for projects with \
                 `provider = \"flox\"`"
            );
            true
        }
    }
}

/// nix is an alternative to flox, not a hard requirement of every host
/// devcroft runs on the way flox currently is — a host that only ever
/// runs flox-backed sandboxes with no interest in nix shouldn't have
/// `doctor` fail over it. So absence is `[WARN]`, not `[FAIL]`. But once
/// `nix` *is* present, a project can declare `provider = "nix"` and
/// depend on it working, so a broken installation (flakes not enabled,
/// design.md decision 5) is a real `[FAIL]`, same severity flox's own
/// absence gets.
fn doctor_nix_provider(required: bool) -> bool {
    let found = std::process::Command::new("nix")
        .arg("--version")
        .output()
        .ok()
        .filter(|out| out.status.success());
    let Some(out) = found else {
        if required {
            println!(
                "[FAIL] provider: nix not found on PATH, but this project declares \
                 `provider = \"nix\"` — install it from https://nixos.org/download"
            );
            return false;
        }
        println!(
            "[WARN] provider: nix not found on PATH — only needed for projects with `provider = \"nix\"`"
        );
        return true;
    };
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();

    // A real evaluation, not `nix flake --help`. Printing help does not
    // touch the experimental feature at all, so the old probe returned
    // success on a host where flakes were genuinely disabled — a false
    // `[PASS]` in the one command whose entire job is to predict why
    // `up` will fail. Caught by hitting exactly that: `doctor` reporting
    // "flakes enabled" while provider resolution failed on the very next
    // line with "experimental Nix feature 'nix-command' is disabled".
    //
    // `nix eval --expr` requires `nix-command`, which is what provider
    // resolution actually failed on. It does not separately prove the
    // `flakes` feature is on — but it is strictly better than a probe
    // that proves nothing, and it is the same "probe the capability,
    // never infer it from the binary being present" rule the
    // gvisor-backend check already follows.
    let flakes_enabled = std::process::Command::new("nix")
        .arg("eval")
        .arg("--expr")
        .arg("1")
        .output()
        .is_ok_and(|o| o.status.success());
    if flakes_enabled {
        println!("[PASS] provider: nix found ({version}), flakes enabled");
        true
    } else {
        println!(
            "[FAIL] provider: nix found ({version}) but flake commands are rejected — add \
             `experimental-features = nix-command flakes` to nix.conf"
        );
        false
    }
}

/// The substrate all three providers share, checked once rather than
/// three times.
///
/// flox, nix and devbox build through the same store, so a daemon that is
/// not listening breaks every one of them at `up` — while each provider's
/// own probe still passes, because `flox --version`, `nix eval --expr 1`
/// and `devbox version` are all satisfied without a store. That is the
/// same "probe the capability, never infer it from the binary being
/// present" rule the nix check above already records, applied one level
/// down; it was added after `doctor` printed "all checks passed" on a host
/// where provider resolution failed on the very next command, which is the
/// precise outcome the rule exists to prevent.
fn doctor_nix_store() -> bool {
    let socket = std::path::Path::new("/nix/var/nix/daemon-socket/socket");
    if !socket.exists() {
        // No daemon is not the same as no store: a single-user install has
        // none and builds fine, and a host with no Nix at all is already
        // reported by the provider lines above. Silence is right here —
        // `doctor` should not invent a finding out of an absence that means
        // nothing.
        return true;
    }
    if devcroft::provider::host_can_build_nix_closures() {
        println!("[PASS] nix store: the daemon socket accepts connections");
        true
    } else {
        println!(
            "[FAIL] nix store: {} exists but refuses connections — the nix daemon is not \
             running, and every provider materializes through it, so `up` will fail for \
             flox, nix and devbox alike; start it with your installer's service command \
             (e.g. `systemctl start nix-daemon`)",
            socket.display()
        );
        false
    }
}

/// Two probes, not one (design.md decision 4): devbox is a frontend over
/// Nix and cannot resolve anything without it, so a missing Nix must be
/// reported as *devbox's* unmet requirement — never as "switch
/// providers" — matching `provider::devbox`'s own `up`-time precondition,
/// which names `nix` itself for exactly the same reason.
fn doctor_devbox_provider(required: bool) -> bool {
    let devbox_found = std::process::Command::new("devbox")
        .arg("version")
        .output()
        .ok()
        .filter(|out| out.status.success());
    let Some(out) = devbox_found else {
        if required {
            println!(
                "[FAIL] provider: devbox not found on PATH, but this project declares \
                 `provider = \"devbox\"` — install it from https://www.jetify.com/devbox"
            );
            return false;
        }
        println!(
            "[WARN] provider: devbox not found on PATH — only needed for projects with \
             `provider = \"devbox\"`"
        );
        return true;
    };
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();

    // `nix eval --expr 1`, not `nix --version` — the same probe the nix
    // provider's own check settled on, and for the same reason this
    // command already learned once: printing a version (or a help page)
    // proves the binary exists and nothing about whether it can
    // evaluate, which is what devbox actually needs. A version check
    // here would reproduce exactly the false `[PASS]` the nix check was
    // fixed for.
    let nix_usable = std::process::Command::new("nix")
        .arg("eval")
        .arg("--expr")
        .arg("1")
        .output()
        .is_ok_and(|o| o.status.success());
    if nix_usable {
        println!("[PASS] provider: devbox found ({version}), nix usable");
        true
    } else if required {
        println!(
            "[FAIL] provider: devbox found ({version}) but Nix is not usable — devbox is a \
             frontend over Nix and cannot resolve packages without it; install it from \
             https://nixos.org/download"
        );
        false
    } else {
        println!(
            "[WARN] provider: devbox found ({version}) but Nix is not usable — needed only if \
             a project declares `provider = \"devbox\"`"
        );
        true
    }
}

fn doctor_ssh_config() {
    match ssh_config_path() {
        Ok(path) => {
            if devcroft::ssh::ssh_config_is_installed(&path) {
                println!(
                    "[PASS] ssh-config: managed block present in {}",
                    path.display()
                );
            } else {
                println!(
                    "[WARN] ssh-config: no devcroft managed block in {} — run `devcroft ssh-config --write`",
                    path.display()
                );
            }
        }
        Err(e) => println!("[WARN] ssh-config: {e}"),
    }
}

/// Informational only (never fails `doctor`): a manifest not being
/// discoverable from `cwd` just means this check doesn't apply here, not
/// that anything is wrong.
fn doctor_manifest_degradation() {
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    let Ok(manifest_path) = devcroft::config::discover(&cwd) else {
        println!(
            "[INFO] manifest: no devcroft.toml found from here; skipping the degraded-capability check"
        );
        return;
    };
    let Ok(text) = std::fs::read_to_string(&manifest_path) else {
        return;
    };
    let Ok((manifest, _warnings)) = devcroft::config::parse(&text) else {
        println!(
            "[WARN] manifest: {} did not parse; run `devcroft up` to see the full error",
            manifest_path.display()
        );
        return;
    };
    let compiled = devcroft::policy::compile(&manifest);
    let degraded = devcroft::policy::detect_degraded(&compiled);
    if degraded.is_empty() {
        println!(
            "[PASS] manifest: no degraded capabilities for {} on this host",
            manifest_path.display()
        );
    } else {
        for d in degraded {
            println!("[WARN] manifest: {d}");
        }
    }
}

/// `devcroft up [name] [--recreate] [--yes]` (lifecycle spec's "Idempotent
/// up" requirement, task 7.2): unlike auto-up, this is the explicit,
/// first-class command — it requires an actual project root (a manifest
/// discoverable from cwd, matching `name` if given), since there's no
/// session it can fall back to failing later the way auto-up can.
/// `--recreate` is destructive (tears down and re-resolves everything),
/// so it follows the same non-interactive-safety rule as `rm`.
/// One warning, after a successful `up`, when resolving the environment
/// executed a hook the project defines (`fix-provisioning-hooks`).
///
/// Provider resolution runs host-side, before any restriction exists,
/// with this user's own network and filesystem access. The two-phase
/// rule justifies trusting that phase on the grounds it runs "pinned
/// tooling from a lockfile, not project code" — and for a flox
/// environment with `[hook].on-activate`, that is not what happened.
/// Measured against flox 1.14.0: no `flox activate` mode suppresses the
/// hook, so devcroft cannot prevent this and says so instead.
///
/// Read back from `meta.json` rather than returned from `up`: the fact
/// belongs to the resolution, `up`'s return type is compared by value in
/// dozens of tests, and recording it means `status` can answer the same
/// question later without re-resolving.
///
/// Not printed when no hook ran — a warning that always fires is one
/// people stop reading.
fn warn_if_activation_hook_ran(manifest: &devcroft::config::Manifest) {
    let Ok(paths) = devcroft::lifecycle::StatePaths::new(&manifest.sandbox.name) else {
        return;
    };
    let ran = devcroft::lifecycle::read_meta(&paths.meta)
        .ok()
        .flatten()
        .is_some_and(|m| m.ran_activation_hook);
    if !ran {
        return;
    }
    eprintln!(
        "devcroft: warning: provider '{}' ran this project's activation hook on the host \
         while resolving the environment — outside the sandbox, with your own network and \
         filesystem access. devcroft cannot capture this provider's environment without \
         running it. Treat `devcroft up` on a repository you have not read as running its \
         code.",
        manifest.env.provider
    );
}

fn cli_up(args: &[String]) -> i32 {
    const USAGE: &str =
        "devcroft up: usage: devcroft up [name] [--recreate] [--yes] [--skip-hooks]";
    let recreate = args.iter().any(|a| a == "--recreate");
    let yes = args.iter().any(|a| a == "--yes");
    let skip_hooks = args.iter().any(|a| a == "--skip-hooks");
    let name_args: Vec<&String> = args
        .iter()
        .filter(|a| *a != "--recreate" && *a != "--yes" && *a != "--skip-hooks")
        .collect();
    if name_args.len() > 1 {
        eprintln!("{USAGE}");
        return 2;
    }

    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("devcroft up: cannot determine current directory: {e}");
            return 1;
        }
    };
    let manifest = match resolve_manifest_strict(name_args.first().map(|s| s.as_str()), &cwd, "up")
    {
        Ok(m) => m,
        Err(code) => return code,
    };
    // config spec: "validation succeeds but a warning is printed at
    // `up`, once". Printed before the sandbox comes up, so a grant of
    // `~/.ssh` is visible above whatever the run produces rather than
    // scrolled off underneath it.
    print_manifest_warnings(&cwd);
    print_degraded_capabilities(&manifest);
    let project_root = match devcroft::config::discover(&cwd) {
        Ok(p) => p.parent().unwrap_or(&cwd).to_path_buf(),
        Err(_) => cwd.clone(),
    };

    if recreate && !yes && !stdout_is_tty() {
        eprintln!("devcroft up: --recreate is destructive; pass --yes to run non-interactively");
        return 2;
    }

    println!(
        "devcroft: bringing up sandbox '{}'...",
        manifest.sandbox.name
    );
    match devcroft::lifecycle::up(
        &manifest,
        &project_root,
        &devcroft::lifecycle::UpOptions {
            recreate,
            skip_hooks,
        },
    ) {
        Ok(outcome) => {
            let msg = match outcome {
                devcroft::lifecycle::UpOutcome::AlreadyUp => "already up",
                devcroft::lifecycle::UpOutcome::Recovered => "recovered from stale state",
                devcroft::lifecycle::UpOutcome::Started => "started",
                devcroft::lifecycle::UpOutcome::Recreated => "recreated",
            };
            println!("devcroft: sandbox '{}' is {msg}.", manifest.sandbox.name);
            warn_if_activation_hook_ran(&manifest);
            0
        }
        Err(e) => {
            eprintln!("devcroft up: {e}");
            match e {
                devcroft::lifecycle::UpError::State(_) => 1,
                devcroft::lifecycle::UpError::Policy(_)
                | devcroft::lifecycle::UpError::Config(_) => 2,
                devcroft::lifecycle::UpError::Provider(_) => 3,
                devcroft::lifecycle::UpError::Backend(_) => 4,
                devcroft::lifecycle::UpError::Keeper(_) | devcroft::lifecycle::UpError::Ssh(_) => 5,
            }
        }
    }
}

/// `devcroft down [name]` (lifecycle spec's "Teardown" requirement):
/// stops the keeper, keeps state and the compiled policy.
fn cli_down(args: &[String]) -> i32 {
    const USAGE: &str = "devcroft down: usage: devcroft down [name]";
    let name = match resolve_name_arg(args, USAGE, "down") {
        Ok(name) => name,
        Err(code) => return code,
    };
    match devcroft::lifecycle::down(&name) {
        Ok(()) => {
            println!("devcroft: sandbox '{name}' is down.");
            0
        }
        Err(e) => {
            eprintln!("devcroft down: {e}");
            1
        }
    }
}

/// `devcroft rm [name] [--yes]` (lifecycle spec's "Teardown" requirement):
/// stops the keeper and removes *all* state — the cli spec's "Non-
/// interactive safety" requirement's own named example of a destructive
/// operation that must refuse to run non-interactively without `--yes`.
fn cli_rm(args: &[String]) -> i32 {
    const USAGE: &str = "devcroft rm: usage: devcroft rm [name] [--yes]";
    let yes = args.iter().any(|a| a == "--yes");
    let name_args: Vec<String> = args.iter().filter(|a| *a != "--yes").cloned().collect();
    let name = match resolve_name_arg(&name_args, USAGE, "rm") {
        Ok(name) => name,
        Err(code) => return code,
    };

    if !yes && !stdout_is_tty() {
        eprintln!(
            "devcroft rm: removing '{name}' is destructive; pass --yes to run non-interactively"
        );
        return 2;
    }

    match devcroft::lifecycle::rm(&name) {
        Ok(()) => {
            println!("devcroft: removed all state for '{name}'.");
            0
        }
        Err(e) => {
            eprintln!("devcroft rm: {e}");
            1
        }
    }
}

/// `devcroft status [name]` (lifecycle spec's "Status and logs"
/// requirement): keeper health, uptime, session count, environment
/// staleness, and degraded capabilities. Needs the full manifest (for
/// degraded-capability detection), so — like `up`/`policy`/`why` — it
/// requires an actual discoverable project root, not just a name.
fn cli_status(args: &[String]) -> i32 {
    const USAGE: &str = "devcroft status: usage: devcroft status [name]";
    if args.len() > 1 {
        eprintln!("{USAGE}");
        return 2;
    }
    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("devcroft status: cannot determine current directory: {e}");
            return 1;
        }
    };
    let manifest = match resolve_manifest_strict(args.first().map(String::as_str), &cwd, "status") {
        Ok(m) => m,
        Err(code) => return code,
    };

    match devcroft::lifecycle::status(&manifest) {
        Ok(s) => {
            print_status(&s, &manifest.env.provider);
            0
        }
        Err(e) => {
            eprintln!("devcroft status: {e}");
            match e {
                devcroft::lifecycle::StatusError::State(_) => 1,
                devcroft::lifecycle::StatusError::Keeper(_) => 5,
            }
        }
    }
}

fn print_status(s: &devcroft::lifecycle::SandboxStatus, provider: &str) {
    println!("sandbox: {}", s.name);
    match &s.keeper {
        devcroft::lifecycle::KeeperStatus::None => println!("keeper: not running"),
        devcroft::lifecycle::KeeperStatus::Stale => {
            println!(
                "keeper: stale (dead pid or unresponsive socket) — run `devcroft up` to recover"
            )
        }
        devcroft::lifecycle::KeeperStatus::Healthy {
            uptime_secs,
            session_count,
        } => println!("keeper: healthy (uptime {uptime_secs}s, {session_count} session(s))"),
    }
    match s.env_stale {
        Some(true) => {
            let what = if provider == "nix" {
                "flake"
            } else {
                "flox manifest"
            };
            println!(
                "env: stale — the {what} changed since the last `up`; run `devcroft up --recreate`"
            )
        }
        Some(false) => println!("env: fresh"),
        None => println!("env: unknown"),
    }
    // add-hardened-tier: `resolved_backend` is "process" for the process
    // tier, or "<backend>/<platform>" (e.g. "gvisor/systrap") for the
    // hardened tier — printed as `isolation: hardened (gvisor/systrap)`
    // per that change's spec scenario, `isolation: process` otherwise.
    match s.isolation.as_deref() {
        Some("process") => println!("isolation: process"),
        Some(backend) => println!("isolation: hardened ({backend})"),
        None => println!("isolation: unknown (no successful `up` yet)"),
    }
    // A healthy keeper with a dead database must not read as simply
    // healthy (the `services` spec's "failure is visible, never silent"),
    // so failures are named per service rather than summarized.
    match &s.services {
        None => {}
        Some(report) => {
            for svc in &report.states {
                // The pid is only shown while running: process-compose
                // keeps reporting the last pid after a service dies, and
                // printing it next to "failed" reads as though something
                // is still there to inspect.
                match svc.pid {
                    Some(pid) if svc.health == devcroft::services::ServiceHealth::Running => {
                        println!("service {}: {} pid={pid}", svc.name, svc.health.label())
                    }
                    _ => println!("service {}: {}", svc.name, svc.health.label()),
                }
            }
            // A supervisor that never came up, or died, is the failure
            // that used to be invisible: every service disappeared from
            // the listing at once and the sandbox read as healthy.
            if let Some(err) = &report.supervisor_error {
                println!("services: {err} — see `devcroft logs` for output");
            }
            let failed = report
                .states
                .iter()
                .filter(|s| s.health.is_failure())
                .count();
            if failed > 0 {
                println!("services: {failed} failed — see `devcroft logs` for output");
            }
        }
    }

    if s.degraded.is_empty() {
        println!("policy: no degraded capabilities on this host");
    } else {
        for d in &s.degraded {
            println!("policy: {d}");
        }
    }
}

/// `devcroft logs [name] [--tail N]` (lifecycle spec's "Status and logs"
/// requirement): the keeper log tail, including session spawn/exit
/// records with timestamps (`keeper::connection` writes these to its own
/// stdout/stderr, which `up` redirects to this file).
fn cli_logs(args: &[String]) -> i32 {
    const USAGE: &str = "devcroft logs: usage: devcroft logs [name] [--tail N]";
    let mut name_arg: Option<String> = None;
    let mut tail: Option<usize> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--tail" {
            match it.next().and_then(|n| n.parse().ok()) {
                Some(n) => tail = Some(n),
                None => {
                    eprintln!("{USAGE}");
                    return 2;
                }
            }
        } else if name_arg.is_none() {
            name_arg = Some(a.clone());
        } else {
            eprintln!("{USAGE}");
            return 2;
        }
    }

    let name = match name_arg {
        Some(n) => n,
        None => {
            let cwd = match std::env::current_dir() {
                Ok(dir) => dir,
                Err(e) => {
                    eprintln!("devcroft logs: cannot determine current directory: {e}");
                    return 1;
                }
            };
            match resolve_sandbox_name(&cwd) {
                Ok(n) => n,
                Err(msg) => {
                    eprintln!("devcroft logs: {msg}");
                    return 2;
                }
            }
        }
    };

    match devcroft::lifecycle::logs(&name, tail) {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(e) => {
            eprintln!("devcroft logs: state: {e}");
            1
        }
    }
}

/// `devcroft ps` (cli spec's "ps lists all sandboxes" scenario): every
/// sandbox with existing state, name/keeper-health/session-count/project-
/// root, no name resolution needed since it lists everything.
fn cli_ps() -> i32 {
    match devcroft::lifecycle::ps() {
        Ok(sandboxes) => {
            if sandboxes.is_empty() {
                println!("no sandboxes");
                return 0;
            }
            for s in sandboxes {
                let health = match s.keeper {
                    devcroft::lifecycle::KeeperStatus::None => "not running".to_string(),
                    devcroft::lifecycle::KeeperStatus::Stale => "stale".to_string(),
                    devcroft::lifecycle::KeeperStatus::Healthy {
                        uptime_secs,
                        session_count,
                    } => format!("healthy (uptime {uptime_secs}s, {session_count} session(s))"),
                };
                println!(
                    "{}\t{}\t{}",
                    s.name,
                    health,
                    s.project_root.as_deref().unwrap_or("-")
                );
                // Services listed individually, indented under their
                // sandbox, and labelled — the `cli` spec requires services
                // and sessions be distinguishable, and the single
                // "process-compose (services)" registry entry that makes
                // teardown work is deliberately not the reporting unit.
                if let Some(root) = s.project_root.as_deref() {
                    let socket =
                        devcroft::services::socket_path(std::path::Path::new(root), &s.name);
                    // Reconciled, same as `status`: a declared service
                    // the supervisor never accepted must be listed, not
                    // omitted.
                    let report = devcroft::services::reconcile(
                        &s.declared_services,
                        devcroft::services::query(&socket),
                    );
                    for svc in &report.states {
                        println!("  service:{}\t{}", svc.name, svc.health.label());
                    }
                    if let Some(err) = &report.supervisor_error {
                        println!("  services\t{err}");
                    }
                }
            }
            0
        }
        Err(e) => {
            eprintln!("devcroft ps: state: {e}");
            1
        }
    }
}

/// `devcroft ssh [--no-up] [name]`: not itself part of any capability
/// spec beyond being named in the cli spec's command surface (no
/// Requirement/Scenario describes its behavior anywhere in the specs).
/// Interpreted here as the obvious convenience its neighbors (`proxy`,
/// `ssh-config`) exist to support: auto-up, then exec (replace this
/// process, `CommandExt::exec`) into a real system `ssh` with the exact
/// options the `ssh-config` block installs, so a user gets full real-
/// terminal fidelity without needing `ssh-config --write` run first or
/// needing to remember the `<name>.devcroft` hostname convention.
fn cli_ssh(args: &[String]) -> i32 {
    use std::os::unix::process::CommandExt;

    const USAGE: &str = "devcroft ssh: usage: devcroft ssh [--no-up] [name]";
    let no_up = args.iter().any(|a| a == "--no-up");
    let name_args: Vec<&String> = args.iter().filter(|a| *a != "--no-up").collect();
    if name_args.len() > 1 {
        eprintln!("{USAGE}");
        return 2;
    }

    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("devcroft ssh: cannot determine current directory: {e}");
            return 1;
        }
    };
    let sandbox_name = match name_args.first() {
        Some(name) => (*name).clone(),
        None => match resolve_sandbox_name(&cwd) {
            Ok(name) => name,
            Err(msg) => {
                eprintln!("devcroft ssh: {msg}");
                return 2;
            }
        },
    };

    if let Err(e) = ensure_client_keypair() {
        eprintln!("devcroft ssh: {e}");
        return 3;
    }
    if !no_up && let Err(msg) = maybe_auto_up(&sandbox_name, &cwd) {
        eprintln!("devcroft ssh: {msg}");
        return 3;
    }
    let (identity, _) = match devcroft::lifecycle::client_key_paths() {
        Ok(paths) => paths,
        Err(e) => {
            eprintln!("devcroft ssh: resolving client key path: {e}");
            return 1;
        }
    };
    let devcroft_bin = std::env::current_exe().unwrap_or_else(|_| "devcroft".into());

    // `--no-up` on the inner `proxy`: this command already brought the
    // sandbox up (or confirmed it didn't need to be), so `proxy` doesn't
    // need to repeat that check.
    let err = std::process::Command::new("ssh")
        .arg("-o")
        .arg("StrictHostKeyChecking=no")
        .arg("-o")
        .arg("UserKnownHostsFile=/dev/null")
        .arg("-o")
        .arg("IdentitiesOnly=yes")
        .arg("-o")
        .arg(format!(
            "ProxyCommand={} proxy --no-up %n",
            devcroft_bin.display()
        ))
        .arg("-i")
        .arg(&identity)
        .arg(format!("{sandbox_name}.devcroft"))
        .exec(); // replaces this process on success; only returns on failure
    eprintln!("devcroft ssh: exec ssh: {err}");
    1
}

/// `devcroft policy --render [--backend nono] [name]` (policy spec's
/// "Inspectable policy" requirement).
fn cli_policy(args: &[String]) -> i32 {
    const USAGE: &str = "devcroft policy: usage: devcroft policy --render [--backend nono] [name]";
    let mut render = false;
    let mut name_arg: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--render" => render = true,
            "--backend" => match it.next().map(String::as_str) {
                Some("nono") => {}
                _ => {
                    eprintln!("devcroft policy: --backend only supports 'nono' in MVP");
                    return 2;
                }
            },
            _ if name_arg.is_none() => name_arg = Some(a.clone()),
            _ => {
                eprintln!("{USAGE}");
                return 2;
            }
        }
    }
    if !render {
        eprintln!("{USAGE}");
        return 2;
    }

    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("devcroft policy: cannot determine current directory: {e}");
            return 1;
        }
    };
    let manifest = match resolve_manifest_strict(name_arg.as_deref(), &cwd, "policy") {
        Ok(m) => m,
        Err(code) => return code,
    };
    let compiled = compile_with_provider_grants(&manifest);
    // CLAUDE.md's own invariant: "Nothing goes to the backend that cannot
    // be shown via policy --render." That cuts both ways — this command
    // must also never show a policy as fine when it would actually fail
    // to compile. Without this, a project-relative grant that is a
    // symlink escaping the project root (`CapabilitySetError::
    // SymlinkEscapesProjectRoot`) rendered as an ordinary in-project
    // entry, silently, while `up` rejected the identical manifest —
    // inspection and enforcement disagreeing is exactly the failure mode
    // this command exists to prevent. Same validation `up` already runs
    // before creating anything, just without a real sandbox behind it.
    let Ok(project_root) = devcroft::config::discover(&cwd).map(|p| {
        p.parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or(cwd.clone())
    }) else {
        print!("{}", devcroft::policy::render(&compiled));
        return 0;
    };
    if let Err(e) = compiled
        .to_capability_plan()
        .to_capability_set(&project_root)
    {
        eprintln!("devcroft policy: {e}");
        return 2;
    }
    print!("{}", devcroft::policy::render(&compiled));
    0
}

/// `policy --render` and `why` are otherwise pure functions of the
/// manifest (`policy::compile`'s own doc comment guarantees this), but a
/// provider's store grants can only be known by actually running the
/// provider — something neither command does. This folds in whatever
/// grants the last successful `up` recorded (`lifecycle::state::Meta`,
/// add-nix-provider task 3.4), so a project that has never been `up`
/// simply shows no provider grants yet, exactly as if the provider had
/// resolved to none.
fn compile_with_provider_grants(
    manifest: &devcroft::config::Manifest,
) -> devcroft::policy::CompiledPolicy {
    let compiled = devcroft::policy::compile(manifest);
    let Ok(paths) = devcroft::lifecycle::StatePaths::new(&manifest.sandbox.name) else {
        return compiled;
    };
    let Ok(Some(meta)) = devcroft::lifecycle::read_meta(&paths.meta) else {
        return compiled;
    };
    let compiled = compiled.with_provider_grants(
        provider_static_name(&manifest.env.provider),
        &meta.read_only_grants,
    );
    // Same live-only caveat as the provider grants just above: `compile`
    // alone has no way to know the proxy's port, since knowing it means
    // an `up` actually ran one. `meta.proxy_port` is `None` for a
    // sandbox that never wanted filtering, so this is a no-op there.
    let compiled = match meta.proxy_port {
        Some(port) => compiled.with_proxy_port(port),
        None => compiled,
    };
    // Same live-only reconstruction again, from a field `Meta` already
    // carries: services are declared by the *provider's* manifest, which
    // `compile` never reads, so a rendered policy that stopped here would
    // omit a rule the keeper is actually holding.
    if meta.declared_services.is_empty() {
        compiled
    } else {
        compiled.with_services_socket_grant(
            provider_static_name(&manifest.env.provider),
            devcroft::services::socket_path(
                std::path::Path::new(&meta.project_root),
                &manifest.sandbox.name,
            )
            .to_string_lossy()
            .into_owned(),
        )
    }
}

/// `Origin::Provider` takes `&'static str`; the manifest's provider name
/// is a runtime `String` by the time it reaches here, so this maps it
/// back to a static via `ProviderKind` (`up.rs`'s own attribution at
/// compile time uses the same method, for the same reason).
fn provider_static_name(name: &str) -> &'static str {
    devcroft::provider::ProviderKind::from_name(name)
        .map(devcroft::provider::ProviderKind::static_name)
        .unwrap_or("unknown")
}

/// `devcroft why --path <p> --op <read|write|readwrite> [name]` or
/// `devcroft why --host <domain> [name]` (policy spec's "Explainable
/// decisions" requirement).
fn cli_why(args: &[String]) -> i32 {
    const USAGE: &str = "devcroft why: usage: devcroft why --path <p> --op <read|write|readwrite> [name] | devcroft why --host <domain> [name]";
    let mut path: Option<String> = None;
    let mut op: Option<String> = None;
    let mut host: Option<String> = None;
    let mut name_arg: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--path" => path = it.next().cloned(),
            "--op" => op = it.next().cloned(),
            "--host" => host = it.next().cloned(),
            _ if name_arg.is_none() => name_arg = Some(a.clone()),
            _ => {
                eprintln!("{USAGE}");
                return 2;
            }
        }
    }

    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("devcroft why: cannot determine current directory: {e}");
            return 1;
        }
    };
    let manifest = match resolve_manifest_strict(name_arg.as_deref(), &cwd, "why") {
        Ok(m) => m,
        Err(code) => return code,
    };
    let compiled = compile_with_provider_grants(&manifest);

    if let Some(host) = host {
        print_explanation(&devcroft::policy::why_host(&compiled, &host));
        return 0;
    }

    let (Some(path), Some(op)) = (path, op) else {
        eprintln!("{USAGE}");
        return 2;
    };
    let path = normalize_path_for_policy(&path);
    let op = match op.as_str() {
        "read" => devcroft::policy::Op::Read,
        "write" => devcroft::policy::Op::Write,
        "readwrite" => devcroft::policy::Op::ReadWrite,
        _ => {
            eprintln!("devcroft why: --op must be read, write, or readwrite");
            return 2;
        }
    };
    print_explanation(&devcroft::policy::why_path(&compiled, &path, op));
    0
}

fn print_explanation(e: &devcroft::policy::Explanation) {
    println!("{}", if e.allowed { "ALLOWED" } else { "DENIED" });
    println!("{}", e.detail);
}

/// Rewrites an absolute path under `$HOME` back to devcroft's own `~/...`
/// shorthand before it reaches `why_path`. Policy rules (baseline
/// credential paths, `filesystem.*` grants) are stored and compared in
/// that shorthand (`paths::is_within` treats `~/...` and `/...` as
/// disjoint root namespaces on purpose, so unrelated absolute and
/// home-relative grants can never accidentally alias) — but a real shell
/// expands `~` in `--path ~/.aws/credentials` *before* devcroft ever sees
/// the argument, so without this, every credential-path `why` query would
/// report "no matching devcroft rule" instead of the real `baseline`
/// origin, even though the underlying `nono why` verdict is still
/// correct.
fn normalize_path_for_policy(path: &str) -> String {
    let Ok(home) = std::env::var("HOME") else {
        return path.to_string();
    };
    if path == home {
        return "~".to_string();
    }
    // The extra `/` check matters: `/home/vscode2` is not under
    // `/home/vscode` just because it shares a string prefix with it.
    match path
        .strip_prefix(&home)
        .and_then(|rest| rest.strip_prefix('/'))
    {
        Some(rest) => format!("~/{rest}"),
        None => path.to_string(),
    }
}

/// Shared by `down`/`rm`: an explicit name (first positional arg) or the
/// one `resolve_sandbox_name` finds from cwd. `usage`/`cmd` are baked in
/// by each caller so the usage/error text stays command-specific.
fn resolve_name_arg(args: &[String], usage: &str, cmd: &str) -> Result<String, i32> {
    if args.len() > 1 {
        eprintln!("{usage}");
        return Err(2);
    }
    match args.first() {
        // A bare "--foo" is never a valid sandbox name (config::is_valid_name
        // requires starting with [a-z0-9]); treated as one anyway, this
        // silently swallowed unrecognized/misplaced flags as the positional
        // name instead of rejecting them — e.g. `down --yes` (down has no
        // such flag; only `rm`/`up --recreate` do) reported "sandbox
        // '--yes' is down" rather than a usage error.
        Some(name) if name.starts_with("--") => {
            eprintln!("{usage}");
            Err(2)
        }
        // config spec's "Name constraints" requirement, extended to any
        // source of a sandbox name (not just `[sandbox].name`): this
        // string becomes a `StatePaths` component unchanged, so a value
        // like `../../target` reaches `remove_dir_all` on `rm` untouched
        // — found by adversarial review, confirmed live by actually
        // deleting a scratch directory outside the state root with it.
        // Rejected here, before `StatePaths::new` is ever called, rather
        // than only inside it: this is the layer that can report exit 2
        // and name the value, which a bare `io::Error` from deep inside
        // `StatePaths::new` cannot without every caller's own error type
        // learning to distinguish it from an ordinary I/O failure.
        Some(name) if !devcroft::config::is_valid_name(name) => {
            eprintln!(
                "devcroft {cmd}: '{name}' is not a valid sandbox name \
                 ([a-z0-9][a-z0-9-]{{0,31}}); did you mean '{}'?",
                devcroft::config::slugify(name)
            );
            Err(2)
        }
        Some(name) => Ok(name.clone()),
        None => {
            let cwd = std::env::current_dir().map_err(|e| {
                eprintln!("devcroft {cmd}: cannot determine current directory: {e}");
                1
            })?;
            resolve_sandbox_name(&cwd).map_err(|msg| {
                eprintln!("devcroft {cmd}: {msg}");
                2
            })
        }
    }
}

/// Ancestor-walks from `start` for `devcroft.toml` (config::discover),
/// parses it, and returns it alongside its project root (the manifest's
/// parent dir) — the cli spec's "Name resolution" requirement's second
/// tier ("the sandbox whose project root contains the cwd") for every
/// command that needs the full manifest (`up`, `status`, `policy`, `why`),
/// not just the name (`exec`/`shell`/`down`/`rm`/`ps`, which only need
/// `resolve_sandbox_name` below).
fn discover_manifest(
    start: &std::path::Path,
) -> Result<(devcroft::config::Manifest, std::path::PathBuf), String> {
    discover_manifest_with_warnings(start).map(|(manifest, root, _)| (manifest, root))
}

/// As [`discover_manifest`], but also hands back the validation warnings
/// `config::parse` produced.
///
/// Split out rather than folded into every caller because the `config`
/// spec scopes these precisely: a sensitive-path grant "prints a warning
/// at `up`, once", not on every command that happens to read the
/// manifest. `up` is the one caller that asks for them; `status`,
/// `policy`, and `why` deliberately do not, so re-running them does not
/// re-nag about a grant the user has already been told about.
///
/// They used to be dropped at *every* call site, `up` included, so the
/// spec's two warning scenarios could not fire at all — found by
/// adversarial review, not by a failing test, because nothing tested
/// them.
fn discover_manifest_with_warnings(
    start: &std::path::Path,
) -> Result<
    (
        devcroft::config::Manifest,
        std::path::PathBuf,
        Vec<devcroft::config::Warning>,
    ),
    String,
> {
    let manifest_path = devcroft::config::discover(start).map_err(|_| {
        format!(
            "no devcroft.toml found in this directory or its ancestors; pass a sandbox name explicitly.{}",
            known_sandboxes_suffix()
        )
    })?;
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("reading {}: {e}", manifest_path.display()))?;
    let (manifest, warnings) =
        devcroft::config::parse(&text).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let project_root = manifest_path.parent().unwrap_or(start).to_path_buf();
    Ok((manifest, project_root, warnings))
}

/// The cli spec's "Name resolution": "otherwise fail with exit code 2
/// listing known sandboxes" — appended to `discover_manifest`'s
/// not-found error. Empty (no suffix at all) when `ps` itself fails or
/// there's nothing to list, rather than compounding one error with
/// another.
fn known_sandboxes_suffix() -> String {
    match devcroft::lifecycle::ps() {
        Ok(sandboxes) if !sandboxes.is_empty() => {
            let names: Vec<&str> = sandboxes.iter().map(|s| s.name.as_str()).collect();
            format!(" Known sandboxes: {}.", names.join(", "))
        }
        _ => String::new(),
    }
}

/// The minimum `exec`/`shell`/`down`/`rm`/`logs` need to resolve an
/// implicit name: just the name `discover_manifest` finds, not the full
/// manifest.
fn resolve_sandbox_name(start: &std::path::Path) -> Result<String, String> {
    discover_manifest(start).map(|(manifest, _)| manifest.sandbox.name)
}

/// For commands that need the *full* manifest (`up`, `status`, `policy`,
/// `why`): resolves it from `cwd`, and — if an explicit `name` was also
/// given — confirms the discovered manifest actually matches it, since a
/// name that happens to resolve to some *other* project's devcroft.toml
/// is a `discover_manifest` false friend, not a fresh lookup.
fn resolve_manifest_strict(
    name_arg: Option<&str>,
    cwd: &std::path::Path,
    cmd: &str,
) -> Result<devcroft::config::Manifest, i32> {
    match discover_manifest(cwd) {
        Ok((manifest, _)) => {
            if let Some(name) = name_arg
                && manifest.sandbox.name != name
            {
                eprintln!(
                    "devcroft {cmd}: the manifest discovered from this directory is for sandbox '{}', not '{name}'; run from within {name}'s own project directory",
                    manifest.sandbox.name
                );
                return Err(2);
            }
            Ok(manifest)
        }
        Err(msg) => {
            eprintln!("devcroft {cmd}: {msg}");
            Err(2)
        }
    }
}

/// Prints the manifest's validation warnings, one per line, on stderr.
///
/// Best-effort by design: the manifest has already been discovered and
/// parsed successfully by the caller at this point, so a failure to
/// re-read it here is not worth failing `up` over — the warnings are
/// advisory, and losing them is strictly better than refusing to bring
/// up a valid sandbox because a second read raced.
///
/// stderr rather than stdout so `up`'s own machine-readable-ish progress
/// lines stay unpolluted, matching how every other advisory in this
/// binary is emitted.
fn print_manifest_warnings(cwd: &std::path::Path) {
    let Ok((_, _, warnings)) = discover_manifest_with_warnings(cwd) else {
        return;
    };
    for warning in warnings {
        eprintln!("devcroft: warning: {warning}");
    }
}

/// The policy spec's "Degraded capability surfacing" requirement: report
/// aspects this host's backend cannot enforce, "once at `up` with
/// severity `warning`, never silently dropping them" — CLAUDE.md's
/// "Degraded capabilities are surfaced, never silent" invariant.
///
/// This was specified from the MVP and never wired into `up`:
/// `detect_degraded` had exactly one caller, `doctor`. On Linux that was
/// latent — nothing is degraded here, so the missing call printed
/// nothing and the absence looked identical to correctness. On macOS,
/// where domain filtering is the aspect that degrades, `up` would have
/// dropped the warning entirely, which is the precise failure the
/// requirement exists to prevent. Found by an adversarial review of
/// `add-egress-proxy`, whose own spec restates the same obligation
/// ("named at `up` and in `doctor`").
///
/// Takes the already-resolved `Manifest` rather than re-discovering from
/// cwd like `print_manifest_warnings` does: the caller has one in hand,
/// and re-reading would let the two warning sets disagree about which
/// manifest they describe.
fn print_degraded_capabilities(manifest: &devcroft::config::Manifest) {
    let compiled = devcroft::policy::compile(manifest);
    for degraded in devcroft::policy::detect_degraded(&compiled) {
        eprintln!("devcroft: warning: {degraded}");
    }
}

/// The cli spec's "Non-interactive safety": never prompt (or require an
/// interactive prompt) when stdout isn't a tty.
fn stdout_is_tty() -> bool {
    unsafe { libc::isatty(libc::STDOUT_FILENO) != 0 }
}

/// Auto-up convenience (exec spec, task 5.3): if `sandbox_name` isn't
/// healthy, brings it up before the caller proceeds, printing a line
/// first so its output precedes whatever `exec`/`shell` streams next
/// (spec scenario: "the `up` output preceding the prompt"). Silent, and
/// `Ok`, when the sandbox is already healthy *or* no manifest matching
/// `sandbox_name` can be discovered from `cwd` — the latter just leaves
/// `exec`/`shell` to fail with their own "not running" error afterward,
/// same as before this existed. Only an attempted-and-failed `up` is
/// reported as an error here.
fn maybe_auto_up(sandbox_name: &str, cwd: &std::path::Path) -> Result<(), String> {
    let paths = devcroft::lifecycle::StatePaths::new(sandbox_name)
        .map_err(|e| format!("resolving state paths for '{sandbox_name}': {e}"))?;
    let healthy = matches!(
        devcroft::lifecycle::health(&paths)
            .map_err(|e| format!("checking '{sandbox_name}' health: {e}"))?,
        devcroft::lifecycle::Health::Healthy(_)
    );
    if healthy {
        return Ok(());
    }

    let Ok((manifest, project_root)) = discover_manifest(cwd) else {
        return Ok(());
    };
    // The manifest found by walking up from `cwd` might not even be the
    // one for the sandbox the user actually named — don't `up` an
    // unrelated project just because its devcroft.toml happened to be
    // the nearest one.
    if manifest.sandbox.name != sandbox_name {
        return Ok(());
    }

    eprintln!("devcroft: sandbox '{sandbox_name}' is not up; starting it...");
    devcroft::lifecycle::up(
        &manifest,
        &project_root,
        &devcroft::lifecycle::UpOptions::default(),
    )
    .map(|_| ())
    .map_err(|e| format!("starting sandbox '{sandbox_name}': {e}"))
}

/// Runs post-restriction, inside the boundary `nono wrap` just applied
/// (CLAUDE.md's listener-before-restriction ordering: the fd was created
/// by `up`, before restriction, and inherited across nono's exec — this
/// is the load-bearing trick the task 1.1/1.2 spike proved out). Never
/// returns under normal operation.
fn keeper_main(fd: RawFd, ssh_fd: RawFd) -> ! {
    // Bound *before* `self_restrict`, and this is the one thing that has
    // to happen first. A network-isolated sandbox reaches the host's
    // egress proxy through a unix socket (the only thing that crosses a
    // network namespace), and something inside the namespace has to
    // present that as an ordinary TCP proxy endpoint for `HTTP_PROXY` to
    // mean anything. Binding here rather than after restriction is the
    // same listener-before-restriction ordering the control and SSH
    // sockets already rely on — the keeper never needs bind permission
    // for a socket it already holds.
    //
    // Absent when this sandbox is not isolated, in which case
    // `HTTP_PROXY` points straight at the host proxy's own port and no
    // relay is needed.
    let relay = std::env::var("DEVCROFT_PROXY_RELAY_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .zip(std::env::var("DEVCROFT_PROXY_SOCKET").ok())
        .and_then(|(port, sock)| {
            match std::net::TcpListener::bind(("127.0.0.1", port)) {
                Ok(l) => Some((l, std::path::PathBuf::from(sock))),
                Err(e) => {
                    // Not fatal: the sandbox comes up without egress
                    // rather than not at all, and the failure is named
                    // rather than surfacing later as an opaque refused
                    // connection.
                    eprintln!(
                        "devcroft keeper: could not bind egress relay on 127.0.0.1:{port}: {e}"
                    );
                    None
                }
            }
        });

    // The keeper's next action, before reconstructing the listener
    // fds or anything else: applies the compiled policy to *this*
    // process, irreversibly (`use-nono-library` task group 4; lifecycle
    // spec: "The keeper restricts itself with no intermediate process").
    // Landlock/Seatbelt restrictions apply to the calling process and are
    // inherited by every child it spawns afterward — hooks, services,
    // sessions — so nothing project-supplied can start before this runs.
    self_restrict();

    if let Some((listener, socket_path)) = relay {
        std::thread::spawn(move || {
            devcroft::proxy::server::relay_to_host_proxy(listener, socket_path);
        });
    }

    // SAFETY: `up` created both listeners before restriction, cleared
    // their FD_CLOEXEC, and passed the fd numbers as this process's argv
    // — they are ours alone to take ownership of.
    let listener = unsafe { UnixListener::from_raw_fd(fd) };
    let ssh_listener = unsafe { UnixListener::from_raw_fd(ssh_fd) };

    // `keeper_main` only ever runs under the `process` tier: the
    // `hardened` tier's host-side control server (`hardened_keeper_main`)
    // does not go through this entrypoint at all, so
    // `LocalSessionBackend` is the only backend reachable here.
    let keeper = Keeper::new(listener, Arc::new(LocalSessionBackend));
    // Must run before anything else spawns a thread — including the ssh
    // server below, whose tokio runtime spawns its own worker-thread pool
    // immediately. `install_shutdown_handler` blocks SIGTERM/SIGINT/
    // SIGHUP on *this* thread specifically so every thread created after
    // it inherits that block; a thread created before it would inherit
    // the signals unblocked instead, and a `SIGTERM` sent to the process
    // could then land on that thread and kill it via the kernel's default
    // disposition before the graceful-shutdown logic below ever runs —
    // exactly what broke `down`'s "terminate live sessions" guarantee the
    // first time ssh startup was ordered ahead of this.
    install_shutdown_handler(Arc::clone(keeper.registry()));

    // Services start here — at keeper startup, before hooks, which `up`
    // runs only once this process is responsive. The keeper owns their
    // lifetime because `up` cannot: it exits, and anything it started
    // over the control socket would be escalated seconds later.
    //
    // Registered in the same registry sessions use, which is what makes
    // teardown work without new machinery: `install_shutdown_handler`
    // above already terminates every registered process group on
    // SIGTERM, so `down` reaps process-compose (and, through it, the
    // services) exactly the way it reaps a live shell.
    start_services_if_requested(Arc::clone(keeper.registry()), Arc::new(LocalSessionBackend));

    // Best-effort (task 6.1): a broken ssh handoff logs to this process's
    // own stderr (redirected by `up` to `<state>/<name>/keeper.log`) and
    // leaves ssh unavailable for this sandbox rather than taking the
    // whole keeper down — exec/shell must keep working regardless.
    devcroft::ssh::start_from_env(ssh_listener, Arc::new(LocalSessionBackend));

    let _ = keeper.serve();
    std::process::exit(0);
}

/// Runs the egress proxy (add-egress-proxy). Unlike `keeper_main`, this
/// process never self-restricts — see `proxy`'s module doc for why it
/// cannot: it needs genuine outbound reach to every allowlisted host,
/// which a `NetworkMode::ProxyOnly` self-restriction would itself deny.
/// Never returns under normal operation.
fn egress_proxy_main(fd: RawFd, unix_fd: RawFd) -> ! {
    let allow: Vec<String> = std::env::var("DEVCROFT_EGRESS_ALLOW")
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| {
            eprintln!("devcroft __egress_proxy: DEVCROFT_EGRESS_ALLOW missing or invalid");
            std::process::exit(1);
        });
    // SAFETY: `crate::proxy::spawn` bound this listener before exec,
    // cleared its FD_CLOEXEC, and passed the fd number as argv — ours
    // alone to take ownership of, same contract as `keeper_main`'s.
    let listener = unsafe { std::net::TcpListener::from_raw_fd(fd) };
    let log_path = std::env::var("DEVCROFT_EGRESS_LOG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            eprintln!("devcroft __egress_proxy: DEVCROFT_EGRESS_LOG missing");
            std::process::exit(1);
        });
    let token = std::env::var("DEVCROFT_EGRESS_TOKEN").unwrap_or_else(|_| {
        eprintln!("devcroft __egress_proxy: DEVCROFT_EGRESS_TOKEN missing");
        std::process::exit(1);
    });
    // The unix listener runs on its own thread, bridging into the TCP
    // one — a network-isolated sandbox has no route to host loopback, so
    // this is its only path to the proxy. Started before `run` because
    // `run` owns this thread for the process's lifetime.
    // SAFETY: `crate::proxy::spawn` bound this listener before exec and
    // cleared its FD_CLOEXEC, same contract as the TCP listener above.
    let unix_listener = unsafe { std::os::unix::net::UnixListener::from_raw_fd(unix_fd) };
    let tcp_port = match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(e) => {
            eprintln!("devcroft __egress_proxy: reading own port: {e}");
            std::process::exit(1);
        }
    };
    std::thread::spawn(move || {
        devcroft::proxy::server::bridge_unix_to_tcp(unix_listener, tcp_port);
    });

    devcroft::proxy::server::run(listener, allow, log_path, token);
    std::process::exit(0);
}

/// Deserializes the `CapabilityPlan` `up` handed down via
/// `DEVCROFT_CAPABILITY_PLAN`, resolves it against this process's own
/// working directory (which `spawn_keeper` set to the project root — the
/// same value the plan's paths were validated against host-side before
/// `up` ever spawned this process), and applies it to the current
/// process. Failure at any step is fatal: exits rather than continuing
/// unrestricted, matching the lifecycle spec's "Failure to restrict is
/// fatal" requirement — `up`'s own `wait_until_responsive` timeout is
/// what turns this into a reported keeper-layer failure, since nothing
/// reads this process's exit code directly.
fn self_restrict() {
    let plan_json = std::env::var("DEVCROFT_CAPABILITY_PLAN").unwrap_or_else(|_| {
        eprintln!("devcroft keeper: DEVCROFT_CAPABILITY_PLAN not set");
        std::process::exit(1);
    });
    let plan: devcroft::policy::CapabilityPlan =
        serde_json::from_str(&plan_json).unwrap_or_else(|e| {
            eprintln!("devcroft keeper: parsing DEVCROFT_CAPABILITY_PLAN: {e}");
            std::process::exit(1);
        });
    let project_root = std::env::current_dir().unwrap_or_else(|e| {
        eprintln!("devcroft keeper: determining project root: {e}");
        std::process::exit(1);
    });
    let caps = plan.to_capability_set(&project_root).unwrap_or_else(|e| {
        eprintln!("devcroft keeper: building capability set: {e}");
        std::process::exit(1);
    });
    if let Err(e) = nono::Sandbox::apply_auto(&caps) {
        eprintln!("devcroft keeper: applying sandbox: {e}");
        std::process::exit(1);
    }
}

/// Starts the generated process-compose config as a supervised child,
/// when `up` asked for it via `DEVCROFT_START_SERVICES`.
///
/// Runs the config `up` wrote host-side; this process never generates it,
/// and never parses a provider manifest — that all happened in the
/// trusted phase. Failure is deliberately non-fatal: the `services` delta
/// spec requires a failed service not to take the sandbox down, so a
/// problem here is logged (to the keeper log `up` redirects) and
/// `exec`/`shell`/SSH keep working.
fn start_services_if_requested(registry: Arc<Registry>, backend: Arc<dyn SessionBackend>) {
    if std::env::var("DEVCROFT_START_SERVICES").as_deref() != Ok("1") {
        return;
    }
    // The config lives in the project root because that is the only
    // location the sandbox can both write and read — `/tmp` is write-only
    // under the baseline profile and the state dir is baseline-denied
    // outright. Named absolutely, from the root `up` passed down: at the
    // hardened tier this process runs on the *host* and the path is
    // resolved by `runsc exec --cwd` inside the sandbox, where a relative
    // path would be resolved against the wrong side of the boundary (and
    // `--cwd` requires an absolute one regardless). The project root is a
    // bind mount at the identical path inside the sandbox, so a single
    // absolute string is correct at both tiers.
    let root = std::path::PathBuf::from(
        std::env::var("DEVCROFT_SERVICES_ROOT").unwrap_or_else(|_| ".".to_string()),
    );
    // Artifacts live in a per-sandbox subdirectory of the root, so two
    // sandboxes sharing a project root stay separated — see
    // `services::artifact_dir`.
    let name = std::env::var("DEVCROFT_SANDBOX_NAME").unwrap_or_default();
    let config = devcroft::services::config_path(&root, &name)
        .to_string_lossy()
        .into_owned();
    let log = devcroft::services::log_path(&root, &name)
        .to_string_lossy()
        .into_owned();
    let sock = devcroft::services::socket_path(&root, &name)
        .to_string_lossy()
        .into_owned();

    let req = devcroft::keeper::protocol::SpawnRequest {
        cmd: "process-compose".to_string(),
        args: vec![
            "up".to_string(),
            "-f".to_string(),
            config,
            // No TUI: this has no terminal attached.
            "-t=false".to_string(),
            "-L".to_string(),
            log,
            // A unix socket for its own API, not the default TCP
            // listener. Found the hard way: process-compose binds
            // localhost:8080 by default and treats failure as fatal, so
            // inside a sandbox that has not granted 8080 it exited
            // immediately — killing services it had already started.
            // `--no-server` would also avoid the bind, but a socket keeps
            // the API reachable for `ps`/`status` to query later, and
            // costs nothing: the project root is writable.
            "-u".to_string(),
            sock,
            // Without this, process-compose exits once every service has
            // finished — taking its API socket, and therefore the only
            // record of *why* a service died, with it. Found by killing a
            // service and watching `status` go from reporting it to
            // reporting nothing at all: the failure became invisible,
            // which is the exact outcome the `services` spec forbids.
            //
            // This is also what flox's own generated config is doing with
            // its `flox_never_exit` sleep-infinity entry — a sentinel
            // process solving the same problem. `--keep-project` is the
            // supported flag for it, so no sentinel is needed here.
            "--keep-project".to_string(),
        ],
        cwd: root.to_string_lossy().into_owned(),
        env: std::collections::BTreeMap::new(),
        pty: None,
    };

    match backend.spawn(&req) {
        Ok(spawned) => {
            let id = registry.insert(spawned.pgid, "process-compose (services)".to_string());
            eprintln!("services started session={id} pgid={}", spawned.pgid);
            // The child is intentionally leaked rather than waited on:
            // the registry now owns its pgid for teardown, and reaping it
            // here would block the keeper's startup path forever.
            std::mem::forget(spawned);
        }
        Err(e) => {
            eprintln!("services failed to start: {e}");
        }
    }
}

/// `down`/`rm` (lifecycle::terminate) signal this process directly, then
/// wait for it to actually exit before touching its state files — so the
/// contract here is: on SIGTERM/SIGINT/SIGHUP, terminate every live
/// session's process group (grace period, then SIGKILL) and exit. The
/// supervisor's own outer grace period (`lifecycle::terminate::GRACE_PERIOD`,
/// 5s) is sized to leave this inner one room to finish first.
/// The process groups of the services the supervisor is running, asked
/// of the supervisor itself.
///
/// **Why this is needed at all**, and why the registry cannot supply it:
/// the registry holds one entry, process-compose, and the shutdown
/// handler kills its *process group*. A service process is not in that
/// group — process-compose puts each one in its own — so the escalation
/// never reached them. A service that ignores SIGTERM therefore outlived
/// `down`, got reparented to init, and kept running, against the
/// `services` spec's "no service process started by it remains alive on
/// the host". Found by task 3.6's test; every earlier service test used
/// a process that dies on SIGTERM, which hid it completely.
///
/// **Delegating this to process-compose does not work**, measured rather
/// than assumed. Its config takes a per-process `shutdown.timeout`, which
/// reads exactly like the escalation this needs. Against a real
/// process-compose 1.116.0, with devcroft not involved at all and ten
/// seconds to act, a service trapping SIGTERM survived and the supervisor
/// itself hung after logging "Caught terminated — Shutting down the
/// running processes...". So the guarantee has to be devcroft's, which is
/// what the spec says anyway: verified by observing process absence, not
/// by trusting a stop command.
///
/// Queried before any signal is sent, because afterwards the supervisor
/// is dying and its API goes with it. Returns empty when this sandbox
/// declares no services, when the socket is unreachable, or when a
/// service has no pid (never started, already exited) — in each case
/// there is nothing extra to kill and the registry's own group covers it.
///
/// Process tier only, and correct that way: at the hardened tier the
/// services run inside gVisor with their own pid namespace, where these
/// pids are meaningless — that tier's shutdown tears down the `runsc`
/// sandbox itself, which takes every process in it.
fn service_process_groups() -> Vec<libc::pid_t> {
    if std::env::var("DEVCROFT_START_SERVICES").as_deref() != Ok("1") {
        return Vec::new();
    }
    let root = std::path::PathBuf::from(
        std::env::var("DEVCROFT_SERVICES_ROOT").unwrap_or_else(|_| ".".to_string()),
    );
    let name = std::env::var("DEVCROFT_SANDBOX_NAME").unwrap_or_default();
    let socket = devcroft::services::socket_path(&root, &name);

    let Ok(states) = devcroft::services::query(&socket) else {
        return Vec::new();
    };
    states
        .into_iter()
        .filter_map(|s| s.pid)
        .filter(|pid| *pid > 0)
        .map(|pid| pid as libc::pid_t)
        // Used as a *process group* id, not a pid: measured against a
        // real process-compose, each service's pid is its own group
        // leader, which is precisely why they escape the registry's
        // sweep — and killing the group also reaps whatever the service
        // itself spawned. If a future process-compose stopped doing
        // that, `kill(-pid)` would simply find no such group and fail
        // harmlessly, with the registry's own group sweep still covering
        // the service (it would then be inside it). Degrades safely
        // either way rather than depending on the observation holding.
        .collect()
}

fn install_shutdown_handler(registry: Arc<Registry>) {
    // The same grace a disconnected session gets, not a second copy of
    // the number: `services::SHUTDOWN_TIMEOUT_SECS` is defined relative
    // to it (process-compose must reap its children before devcroft
    // reaps process-compose), and two independently-maintained constants
    // is how that relationship silently breaks.
    const SESSION_GRACE: std::time::Duration = devcroft::keeper::DEFAULT_GRACE_PERIOD;

    // Signal handlers can only safely do async-signal-safe work (no
    // locks, no allocation) — Registry::snapshot needs both. So instead
    // of a handler, block these signals on every thread (a mask set here,
    // before `keeper.serve()` spawns any per-connection threads, is
    // inherited by all of them) and have one dedicated thread block in
    // `sigwait`, which delivers the signal through an ordinary blocking
    // call on a normal thread — safe to do anything from.
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::sigaddset(&mut set, libc::SIGHUP);
        if libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) != 0 {
            panic!(
                "blocking shutdown signals: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    std::thread::spawn(move || {
        let mut received: libc::c_int = 0;
        if unsafe { libc::sigwait(&set, &mut received) } != 0 {
            return;
        }

        // Service pids are collected *before* anything is signalled,
        // while the supervisor is still alive to answer — see
        // `service_process_groups` for why the registry alone is not
        // enough.
        let service_pgids = service_process_groups();

        for (_, info) in registry.snapshot() {
            unsafe {
                libc::kill(-info.pgid, libc::SIGTERM);
            }
        }
        for pgid in &service_pgids {
            unsafe {
                libc::kill(-pgid, libc::SIGTERM);
            }
        }

        std::thread::sleep(SESSION_GRACE);

        for (_, info) in registry.snapshot() {
            unsafe {
                libc::kill(-info.pgid, libc::SIGKILL);
            }
        }
        for pgid in &service_pgids {
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
        std::process::exit(0);
    });
}
