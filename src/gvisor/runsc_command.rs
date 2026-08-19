//! Resolves the `runsc` binary and assembles its argument vectors. Pure
//! except for [`resolve`] and [`probe_version`], which touch the host —
//! everything else here is plain argument-vector construction, testable
//! on every platform (matching [`super::oci_spec`]'s split).

use std::path::{Path, PathBuf};
use std::process::Command;

use super::Platform;
use super::oci_spec::NetworkMode;

/// Resolves the `runsc` binary the same way `up.rs` resolves `nono`:
/// against *this process's* ambient `PATH`, before any provider env
/// replaces it — see `crate::paths::resolve_on_path`'s own doc comment
/// for why that ordering matters.
pub fn resolve() -> Option<PathBuf> {
    crate::paths::resolve_on_path("runsc")
}

/// `runsc --version`'s first line (`"runsc version release-YYYYMMDD.N"`),
/// or `None` if the binary can't be resolved or run. Only the first line:
/// confirmed live against a real binary that the full output is multiple
/// lines (`runsc version ...` then a separate `spec: ...` line) — taking
/// the whole trimmed blob, as an earlier version of this function did,
/// produces a version string with an embedded newline that breaks
/// `doctor`'s single-line `[FAIL]`/`[PASS]` message mid-sentence. Used by
/// `doctor` and by `up`'s own availability check before attempting a
/// bundle it already knows can't run.
pub fn probe_version(runsc: &Path) -> Option<String> {
    let output = Command::new(runsc).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// One running sandbox's identity within `runsc`'s own bookkeeping: the
/// container id it was started with, and the `--root` state directory
/// scoping that id — kept together because every `runsc` invocation past
/// `run` needs both, and passing them separately invites a call site
/// that forgets to point `--root` at the same place `run` used.
pub struct Container<'a> {
    pub id: &'a str,
    /// `runsc`'s own per-execution state dir (container metadata, not
    /// the sandbox's filesystem state) — kept under the same
    /// `<state>/<name>/` tree as everything else devcroft already
    /// case in `paths::state`, one directory below it (design.md
    /// decision 2: a persistent, sandbox-scoped path, not a per-run
    /// temp dir, so concurrent sandboxes never share `runsc` state and
    /// rootless mode needs no `/run` access).
    pub state_root: &'a Path,
}

/// `runsc run -d --bundle <bundle> --root <state_root> <id>`: starts the
/// sandbox detached, with the init process from [`super::oci_spec`]'s
/// `config.json` as PID 1. Platform/network flags come first since
/// `runsc` parses global flags before the subcommand's own. `network`
/// must agree with what the bundle's own `config.json` requested (a
/// fresh netns for `NetworkMode::None`, none for `NetworkMode::Host`) —
/// see `oci_spec::build`'s network-namespace handling, which this must
/// stay consistent with rather than decide independently.
pub fn run_args(
    container: &Container<'_>,
    bundle: &Path,
    platform: Platform,
    network: NetworkMode,
    host_uds: bool,
) -> Vec<String> {
    let mut args = global_args(container, platform, network, host_uds);
    args.push("run".to_string());
    // `runsc`'s flags are Go stdlib `flag`, which has no short-alias
    // concept — `-d` alone is not `-detach`, it is simply undefined and
    // rejected outright. Found live, not from documentation: `runsc run`
    // itself printed its own usage and refused to start over exactly
    // this.
    args.push("-detach".to_string());
    args.push("--bundle".to_string());
    args.push(bundle.to_string_lossy().into_owned());
    args.push(container.id.to_string());
    args
}

/// `runsc exec --cwd <cwd> <id> <argv>`: the native exec-into primitive
/// `add-hardened-tier`'s `SessionBackend` trait dispatches sessions
/// through — see [`super::session_backend::RunscExecBackend`]. `cwd` is
/// passed through unchanged rather than translated: the OCI bundle's
/// bind mounts keep every path identical inside and outside the sandbox
/// (`oci_spec::build`'s mounts use the same `destination` and `source`),
/// so the session layer's own project-root cwd is directly usable here.
///
/// **No `--` separator before `argv`.** Found live, not reasoned about:
/// unlike `run` (whose own doc comment covers a real Go-`flag` quirk with
/// `-detach`), `runsc exec`'s own usage is `exec [options] <container-id>
/// <command> [args...]` — it does not expect or consume a bare `--`, and
/// including one makes it the literal argv\[0\] of the command to run
/// ("error finding executable \"--\" in PATH"), silently breaking every
/// `exec`/`shell` session at the hardened tier since add-hardened-tier
/// shipped this. Confirmed against `runsc exec --help` and a real
/// container.
pub fn exec_args(container: &Container<'_>, cwd: &str, argv: &[String]) -> Vec<String> {
    let mut args = vec![
        "--root".to_string(),
        container.state_root.to_string_lossy().into_owned(),
        "exec".to_string(),
        "--cwd".to_string(),
        cwd.to_string(),
        container.id.to_string(),
    ];
    args.extend(argv.iter().cloned());
    args
}

/// `runsc kill <id> SIGTERM` — the first half of teardown. mxc's own
/// implementation experience is explicit about why this matters: the
/// sandbox's process tree is separate from the `runsc` client process,
/// so killing only the client that issued `run`/`exec` does not stop the
/// sandbox itself.
pub fn kill_args(container: &Container<'_>, signal: &str) -> Vec<String> {
    vec![
        "--root".to_string(),
        container.state_root.to_string_lossy().into_owned(),
        "kill".to_string(),
        container.id.to_string(),
        signal.to_string(),
    ]
}

/// `runsc delete -force <id>` — removes the container's `runsc`-side
/// state after `kill`. `-force` tolerates a container that's already
/// stopped, matching `state::clear_runtime_state`'s own idempotency.
pub fn delete_args(container: &Container<'_>) -> Vec<String> {
    vec![
        "--root".to_string(),
        container.state_root.to_string_lossy().into_owned(),
        "delete".to_string(),
        "-force".to_string(),
        container.id.to_string(),
    ]
}

fn global_args(
    container: &Container<'_>,
    platform: Platform,
    network: NetworkMode,
    host_uds: bool,
) -> Vec<String> {
    let mut args = vec![
        "--rootless".to_string(),
        "--platform".to_string(),
        platform.runsc_flag().to_string(),
        "--network".to_string(),
        network.runsc_flag().to_string(),
        "--root".to_string(),
        container.state_root.to_string_lossy().into_owned(),
    ];
    if host_uds {
        // `create`, never `open`/`all`: this permits the sandbox to
        // *bind* a unix socket on a gofer-backed mount (which is how
        // process-compose exposes its API, and the only way the host-side
        // control process can read per-service state back out of the
        // sandbox), while still refusing to let it *connect* to any host
        // socket — the direction that would reach things like a docker
        // socket. runsc's default is `none`, under which the bind is not
        // visible on the host at all.
        //
        // Requested only when this sandbox actually declares services, so
        // a sandbox without them keeps the stricter default.
        args.push("--host-uds=create".to_string());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn container(id: &str, state_root: &'static str) -> Container<'static> {
        Container {
            id: Box::leak(id.to_string().into_boxed_str()),
            state_root: Path::new(state_root),
        }
    }

    #[test]
    fn run_args_are_rootless_with_the_selected_platform() {
        let c = container("myproj", "/state/myproj/runsc-state");
        let args = run_args(
            &c,
            &PathBuf::from("/state/myproj/bundle"),
            Platform::Systrap,
            NetworkMode::None,
            false,
        );

        assert!(args.contains(&"--rootless".to_string()));
        let platform_idx = args.iter().position(|a| a == "--platform").unwrap();
        assert_eq!(args[platform_idx + 1], "systrap");
        assert!(args.contains(&"run".to_string()));
        assert!(args.contains(&"-detach".to_string()));
        assert_eq!(args.last(), Some(&"myproj".to_string()));
    }

    #[test]
    fn kvm_platform_selected_when_requested() {
        let c = container("myproj", "/state/myproj/runsc-state");
        let args = run_args(
            &c,
            &PathBuf::from("/state/myproj/bundle"),
            Platform::Kvm,
            NetworkMode::None,
            false,
        );
        let platform_idx = args.iter().position(|a| a == "--platform").unwrap();
        assert_eq!(args[platform_idx + 1], "kvm");
    }

    #[test]
    fn network_flag_matches_the_selected_mode() {
        let c = container("myproj", "/state/myproj/runsc-state");
        for (mode, expected) in [(NetworkMode::None, "none"), (NetworkMode::Host, "host")] {
            let args = run_args(
                &c,
                &PathBuf::from("/state/myproj/bundle"),
                Platform::Systrap,
                mode,
                false,
            );
            let network_idx = args.iter().position(|a| a == "--network").unwrap();
            assert_eq!(args[network_idx + 1], expected);
        }
    }

    /// add-flox-services task 3.2: per-service state at the hardened tier
    /// is read back over the unix socket process-compose binds *inside*
    /// the sandbox, on a mount the host shares. Under runsc's default
    /// `--host-uds=none` that bind is invisible to the host, so a sandbox
    /// with services must ask for `create` — and only `create`, never a
    /// value that also permits connecting outward to host sockets.
    #[test]
    fn host_uds_is_requested_only_for_services_and_only_to_create() {
        let c = container("myproj", "/state/myproj/runsc-state");
        let bundle = PathBuf::from("/state/myproj/bundle");

        let without = run_args(&c, &bundle, Platform::Systrap, NetworkMode::None, false);
        assert!(
            !without.iter().any(|a| a.starts_with("--host-uds")),
            "a sandbox with no services must keep runsc's stricter default"
        );

        let with = run_args(&c, &bundle, Platform::Systrap, NetworkMode::None, true);
        assert!(with.contains(&"--host-uds=create".to_string()));
        for forbidden in ["--host-uds=open", "--host-uds=all"] {
            assert!(!with.contains(&forbidden.to_string()));
        }
    }

    #[test]
    fn exec_args_carry_the_argv_directly_with_no_separator() {
        let c = container("myproj", "/state/myproj/runsc-state");
        let args = exec_args(
            &c,
            "/proj",
            &["sh".to_string(), "-c".to_string(), "echo hi".to_string()],
        );

        // `runsc exec`'s own usage is `exec [options] <container-id>
        // <command> [args...]` — no `--` separator, unlike `runsc run`.
        assert!(
            !args.contains(&"--".to_string()),
            "a bare `--` becomes the literal argv[0] runsc tries to exec"
        );
        assert_eq!(&args[args.len() - 3..], &["sh", "-c", "echo hi"]);
        assert!(args.contains(&"exec".to_string()));
        assert!(args.contains(&"myproj".to_string()));
        let cwd_idx = args.iter().position(|a| a == "--cwd").unwrap();
        assert_eq!(args[cwd_idx + 1], "/proj");
    }

    #[test]
    fn kill_and_delete_target_the_same_state_root_as_run() {
        let c = container("myproj", "/state/myproj/runsc-state");
        let run = run_args(
            &c,
            &PathBuf::from("/state/myproj/bundle"),
            Platform::Systrap,
            NetworkMode::None,
            false,
        );
        let kill = kill_args(&c, "SIGTERM");
        let delete = delete_args(&c);

        for args in [&run, &kill, &delete] {
            let root_idx = args.iter().position(|a| a == "--root").unwrap();
            assert_eq!(args[root_idx + 1], "/state/myproj/runsc-state");
        }
        assert!(delete.contains(&"-force".to_string()));
    }
}
