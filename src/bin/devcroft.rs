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
        Some("__hardened_keeper") => {
            let fd: RawFd = args
                .get(2)
                .and_then(|s| s.parse().ok())
                .expect("__hardened_keeper requires a control-socket fd argument");
            let ssh_fd: RawFd = args
                .get(3)
                .and_then(|s| s.parse().ok())
                .expect("__hardened_keeper requires an ssh-socket fd argument");
            let container_id = args
                .get(4)
                .cloned()
                .expect("__hardened_keeper requires a container-id argument");
            let runsc = args
                .get(5)
                .cloned()
                .expect("__hardened_keeper requires a runsc-path argument");
            let state_root = args
                .get(6)
                .cloned()
                .expect("__hardened_keeper requires a runsc-state-root argument");
            hardened_keeper_main(fd, ssh_fd, container_id, runsc, state_root);
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
        other => {
            eprintln!(
                "devcroft: unknown command {:?}; see the cli spec for the full command surface",
                other.unwrap_or("(none given)")
            );
            std::process::exit(2);
        }
    }
}

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

    // cli spec's init scenarios: flox wins if both a flox environment and
    // a nix flake are present (the more specific, devcroft-native choice),
    // and either one supersedes advice about a toolchain pin it would
    // otherwise just be a fallback for.
    let has_flox = cwd.join(".flox").is_dir();
    let has_flake = cwd.join("flake.nix").is_file();
    let provider = if has_flox || !has_flake {
        "flox"
    } else {
        "nix"
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

    if has_flox {
        println!("devcroft: found an existing flox environment (.flox/); ready for `devcroft up`.");
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
    doctor_hardened_tier();
    ok &= doctor_gvisor_backend();
    doctor_ssh_config();
    doctor_manifest_degradation();

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
    let support = nono::Sandbox::support_info();
    if support.is_supported {
        println!("[PASS] backend: {} — {}", support.platform, support.details);
    } else {
        println!(
            "[FAIL] backend: {} does not support sandboxing — {}",
            support.platform, support.details
        );
    }
    support.is_supported
}

fn doctor_provider() -> bool {
    let flox_ok = match std::process::Command::new("flox").arg("--version").output() {
        Ok(out) if out.status.success() => {
            println!(
                "[PASS] provider: flox found ({})",
                String::from_utf8_lossy(&out.stdout).trim()
            );
            true
        }
        _ => {
            println!("[FAIL] provider: flox not found on PATH — install it from https://flox.dev");
            false
        }
    };
    flox_ok && doctor_nix_provider()
}

/// nix is an alternative to flox, not a hard requirement of every host
/// devcroft runs on the way flox currently is — a host that only ever
/// runs flox-backed sandboxes with no interest in nix shouldn't have
/// `doctor` fail over it. So absence is `[WARN]`, not `[FAIL]`. But once
/// `nix` *is* present, a project can declare `provider = "nix"` and
/// depend on it working, so a broken installation (flakes not enabled,
/// design.md decision 5) is a real `[FAIL]`, same severity flox's own
/// absence gets.
fn doctor_nix_provider() -> bool {
    let Ok(out) = std::process::Command::new("nix").arg("--version").output() else {
        println!(
            "[WARN] provider: nix not found on PATH — only needed for projects with `provider = \"nix\"`"
        );
        return true;
    };
    if !out.status.success() {
        println!(
            "[WARN] provider: nix not found on PATH — only needed for projects with `provider = \"nix\"`"
        );
        return true;
    }
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

/// add-hardened-tier's backend-generic doctor line: whether the hardened
/// tier is even conceivable on this platform at all, independent of
/// which concrete backend — `doctor_gvisor_backend` below is where the
/// real, backend-specific probe lives. The hardened tier is opt-in
/// (`[sandbox].isolation` defaults to `process`), the same posture the
/// `nix` provider's absence already has in this command, so absence
/// here is always `[WARN]`, never `[FAIL]` — a host with no hardened
/// backend at all is still fully usable for every `process`-tier
/// project.
fn doctor_hardened_tier() {
    if cfg!(target_os = "linux") {
        println!("[PASS] hardened-tier: Linux host — see the gvisor-backend check below");
    } else {
        println!(
            "[WARN] hardened-tier: unavailable on this platform — the hardened tier is Linux \
             only (a permanent limitation, not a missing install) and is only needed for \
             `isolation = \"hardened\"` projects"
        );
    }
}

/// add-gvisor-backend's `doctor` diagnostics (task 7, cli delta spec):
/// `runsc` presence and version, which platform would be selected and
/// why, and a real smoke check of that platform — never inferred from
/// binary presence alone. Silent on non-Linux: `doctor_hardened_tier`
/// above already reported the platform limitation, and there is nothing
/// gVisor-specific to add to that.
///
/// No version-range check: unlike nono/nix, this repo has not decided
/// what "tested range" means for a project that ships continuously
/// rather than by semver (add-gvisor-backend's own Open Questions section
/// says so explicitly) — reporting a fabricated range here would
/// misrepresent an undecided question as a settled one.
fn doctor_gvisor_backend() -> bool {
    if !cfg!(target_os = "linux") {
        return true;
    }

    let Some(runsc) = devcroft::gvisor::runsc_command::resolve() else {
        println!(
            "[WARN] gvisor-backend: runsc not found on PATH — only needed for \
             `isolation = \"hardened\"` projects; see https://gvisor.dev/docs/user_guide/install/"
        );
        return true;
    };

    let Some(version) = devcroft::gvisor::runsc_command::probe_version(&runsc) else {
        println!(
            "[FAIL] gvisor-backend: runsc found at {} but `runsc --version` failed — reinstall it",
            runsc.display()
        );
        return false;
    };

    let platform = devcroft::gvisor::select_platform();
    let platform_name = platform.runsc_flag();
    let smoke = std::process::Command::new(&runsc)
        .arg("--rootless")
        .arg("--platform")
        .arg(platform_name)
        .arg("do")
        .arg("true")
        .output();
    match smoke {
        Ok(out) if out.status.success() => {
            println!("[PASS] gvisor-backend: {version}, platform: {platform_name}");
            true
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let reason = stderr.lines().next_back().unwrap_or("unknown error");
            println!(
                "[FAIL] gvisor-backend: {version} found, but the {platform_name} platform \
                 does not work on this host ({reason}) — {}",
                if platform_name == "kvm" {
                    "check /dev/kvm permissions, or the hardened tier will fall back to systrap \
                     automatically on the next `up`"
                } else {
                    "check kernel support for gVisor's systrap platform"
                }
            );
            false
        }
        Err(e) => {
            println!("[FAIL] gvisor-backend: could not run `runsc do`: {e}");
            false
        }
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
            0
        }
        Err(e) => {
            eprintln!("devcroft up: {e}");
            match e {
                devcroft::lifecycle::UpError::State(_) => 1,
                devcroft::lifecycle::UpError::Policy(_) => 2,
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
        Some(services) => {
            for svc in services {
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
            let failed = services.iter().filter(|s| s.health.is_failure()).count();
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
                if let Some(root) = s.project_root.as_deref()
                    && let Ok(Some(services)) = devcroft::services::query(
                        &devcroft::services::socket_path(std::path::Path::new(root)),
                    )
                {
                    for svc in services {
                        println!("  service:{}\t{}", svc.name, svc.health.label());
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
    print!(
        "{}",
        devcroft::policy::render(&compile_with_provider_grants(&manifest))
    );
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
    compiled.with_provider_grants(
        provider_static_name(&manifest.env.provider),
        &meta.read_only_grants,
    )
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
    let manifest_path = devcroft::config::discover(start).map_err(|_| {
        format!(
            "no devcroft.toml found in this directory or its ancestors; pass a sandbox name explicitly.{}",
            known_sandboxes_suffix()
        )
    })?;
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("reading {}: {e}", manifest_path.display()))?;
    let (manifest, _warnings) =
        devcroft::config::parse(&text).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let project_root = manifest_path.parent().unwrap_or(start).to_path_buf();
    Ok((manifest, project_root))
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
    // The keeper's very first action, before reconstructing the listener
    // fds or anything else: applies the compiled policy to *this*
    // process, irreversibly (`use-nono-library` task group 4; lifecycle
    // spec: "The keeper restricts itself with no intermediate process").
    // Landlock/Seatbelt restrictions apply to the calling process and are
    // inherited by every child it spawns afterward — hooks, services,
    // sessions — so nothing project-supplied can start before this runs.
    self_restrict();

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
    let config = devcroft::services::config_path(&root)
        .to_string_lossy()
        .into_owned();
    let log = devcroft::services::log_path(&root)
        .to_string_lossy()
        .into_owned();
    let sock = devcroft::services::socket_path(&root)
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
fn install_shutdown_handler(registry: Arc<Registry>) {
    const SESSION_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

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
        for (_, info) in registry.snapshot() {
            unsafe {
                libc::kill(-info.pgid, libc::SIGTERM);
            }
        }
        std::thread::sleep(SESSION_GRACE);
        for (_, info) in registry.snapshot() {
            unsafe {
                libc::kill(-info.pgid, libc::SIGKILL);
            }
        }
        std::process::exit(0);
    });
}

/// The hardened tier's host-side control server (add-gvisor-backend
/// task 4.2): runs post-`up_hardened`, on the host, never restricted —
/// there is nothing to self-restrict at this tier (the sandbox's own
/// gVisor+Landlock confinement is the boundary). Dispatches every
/// session through `runsc exec` (`RunscExecBackend`) instead of a local
/// fork/exec, but is otherwise identical to `keeper_main`: same
/// `Keeper`, same wire protocol, same embedded ssh server — only the
/// session-spawn mechanism and the shutdown sequence's extra step
/// (tearing down the `runsc` sandbox itself) differ.
#[cfg(target_os = "linux")]
fn hardened_keeper_main(
    fd: RawFd,
    ssh_fd: RawFd,
    container_id: String,
    runsc: String,
    state_root: String,
) -> ! {
    // SAFETY: `up_hardened` created both listeners before spawning this
    // process, cleared their FD_CLOEXEC, and passed the fd numbers as
    // this process's argv — they are ours alone to take ownership of,
    // the same contract `keeper_main` has for the process tier.
    let listener = unsafe { UnixListener::from_raw_fd(fd) };
    let ssh_listener = unsafe { UnixListener::from_raw_fd(ssh_fd) };

    let runsc_path = std::path::PathBuf::from(&runsc);
    let state_root_path = std::path::PathBuf::from(&state_root);
    let backend: Arc<dyn devcroft::keeper::SessionBackend> =
        Arc::new(devcroft::gvisor::session_backend::RunscExecBackend {
            runsc: runsc_path.clone(),
            container_id: container_id.clone(),
            state_root: state_root_path.clone(),
        });

    let keeper = Keeper::new(listener, Arc::clone(&backend));
    // Must run before `ssh::start_from_env` spawns its own tokio worker
    // threads, for the identical reason `keeper_main` orders it first —
    // see that function's own comment.
    install_hardened_shutdown_handler(
        Arc::clone(keeper.registry()),
        runsc_path,
        container_id,
        state_root_path,
    );

    // Same call, same position in the sequence as `keeper_main` — before
    // ssh, before hooks — with the *only* difference being which
    // `SessionBackend` the request is dispatched through. That is the
    // parity add-flox-services task 3.2 asks for ("do not add a
    // tier-specific path"): process-compose runs inside the gVisor
    // sandbox via `runsc exec`, registered in the same registry, so
    // `install_hardened_shutdown_handler` reaps it exactly as it reaps a
    // live shell.
    start_services_if_requested(Arc::clone(keeper.registry()), Arc::clone(&backend));

    devcroft::ssh::start_from_env(ssh_listener, Arc::clone(&backend));

    let _ = keeper.serve();
    std::process::exit(0);
}

/// Unreachable in practice — `__hardened_keeper` is only ever spawned by
/// `up_hardened`, which `lifecycle::up::resolve_backend` already confines
/// to Linux hosts. This stub exists only so the crate still compiles on
/// macOS, where `gvisor::runner`/`gvisor::session_backend` (this
/// function's real dependencies) are not even built.
#[cfg(not(target_os = "linux"))]
fn hardened_keeper_main(
    _fd: RawFd,
    _ssh_fd: RawFd,
    _container_id: String,
    _runsc: String,
    _state_root: String,
) -> ! {
    unreachable!("__hardened_keeper is only ever spawned by up_hardened, which is Linux-only")
}

/// `down`/`rm` for the hardened tier: drains sessions the same way
/// `install_shutdown_handler` does (killing each local `runsc exec`
/// client's process group — design.md decision 5 notes this is expected
/// to propagate into the sandboxed process, unverified against a live
/// `runsc`), then tears down the sandbox itself (`runsc kill` +
/// `runsc delete`). That second step has no analogue at the process
/// tier: there, the sandboxed process tree *is* the keeper's own
/// restricted process tree, so killing the keeper's pid already stops
/// everything. Here the sandbox is a separate, persistent `runsc run -d`
/// container — draining sessions alone would leave it running.
#[cfg(target_os = "linux")]
fn install_hardened_shutdown_handler(
    registry: Arc<Registry>,
    runsc: std::path::PathBuf,
    container_id: String,
    state_root: std::path::PathBuf,
) {
    const SESSION_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

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
        for (_, info) in registry.snapshot() {
            unsafe {
                libc::kill(-info.pgid, libc::SIGTERM);
            }
        }
        std::thread::sleep(SESSION_GRACE);
        for (_, info) in registry.snapshot() {
            unsafe {
                libc::kill(-info.pgid, libc::SIGKILL);
            }
        }
        let container = devcroft::gvisor::runsc_command::Container {
            id: &container_id,
            state_root: &state_root,
        };
        let _ = devcroft::gvisor::runner::teardown(&runsc, &container);
        std::process::exit(0);
    });
}
