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

/// `runsc --version`'s stdout, or `None` if the binary can't be resolved
/// or run. Used by `doctor` and by `up`'s own availability check before
/// attempting a bundle it already knows can't run.
pub fn probe_version(runsc: &Path) -> Option<String> {
    let output = Command::new(runsc).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
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
) -> Vec<String> {
    let mut args = global_args(container, platform, network);
    args.push("run".to_string());
    args.push("-d".to_string());
    args.push("--bundle".to_string());
    args.push(bundle.to_string_lossy().into_owned());
    args.push(container.id.to_string());
    args
}

/// `runsc exec --cwd <cwd> <id> -- <argv>`: the native exec-into
/// primitive `add-hardened-tier`'s `SessionBackend` trait dispatches
/// sessions through — see [`super::session_backend::RunscExecBackend`].
/// `cwd` is passed through unchanged rather than translated: the OCI
/// bundle's bind mounts keep every path identical inside and outside
/// the sandbox (`oci_spec::build`'s mounts use the same `destination`
/// and `source`), so the session layer's own project-root cwd is
/// directly usable here.
pub fn exec_args(container: &Container<'_>, cwd: &str, argv: &[String]) -> Vec<String> {
    let mut args = vec![
        "--root".to_string(),
        container.state_root.to_string_lossy().into_owned(),
        "exec".to_string(),
        "--cwd".to_string(),
        cwd.to_string(),
        container.id.to_string(),
        "--".to_string(),
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

fn global_args(container: &Container<'_>, platform: Platform, network: NetworkMode) -> Vec<String> {
    vec![
        "--rootless".to_string(),
        "--platform".to_string(),
        platform.runsc_flag().to_string(),
        "--network".to_string(),
        network.runsc_flag().to_string(),
        "--root".to_string(),
        container.state_root.to_string_lossy().into_owned(),
    ]
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
        );

        assert!(args.contains(&"--rootless".to_string()));
        let platform_idx = args.iter().position(|a| a == "--platform").unwrap();
        assert_eq!(args[platform_idx + 1], "systrap");
        assert!(args.contains(&"run".to_string()));
        assert!(args.contains(&"-d".to_string()));
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
            );
            let network_idx = args.iter().position(|a| a == "--network").unwrap();
            assert_eq!(args[network_idx + 1], expected);
        }
    }

    #[test]
    fn exec_args_carry_the_argv_after_a_separator() {
        let c = container("myproj", "/state/myproj/runsc-state");
        let args = exec_args(
            &c,
            "/proj",
            &["sh".to_string(), "-c".to_string(), "echo hi".to_string()],
        );

        let sep_idx = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(&args[sep_idx + 1..], &["sh", "-c", "echo hi"]);
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
