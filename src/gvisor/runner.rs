//! Materializes the OCI bundle and drives `runsc` as real subprocesses:
//! `run` to start a sandbox, `kill`+`delete` to tear one down. Gated to
//! Linux since gVisor itself is Linux-only; everything here is the
//! host-touching half of the split [`super`]'s module doc describes —
//! [`super::oci_spec`] and [`super::runsc_command`] stay pure precisely
//! so only this file needs the `cfg`.
//!
//! **No Landlock wrapping `runsc run`.** An earlier version of this file
//! applied a Landlock ruleset to this process before exec'ing into
//! `runsc run`, as defense in depth additive to gVisor's own seccomp
//! confinement of the Sentry (add-gvisor-backend design.md decision 4).
//! Live testing (add-flox-services task 6.5, the first time a real
//! `runsc` was ever exercised in this repo's devcontainer) found that
//! this makes `--rootless` bootstrap fail unconditionally: `runsc run`'s
//! own chroot setup issues a `mount()` call to change mount propagation
//! (`MS_SLAVE|MS_REC`), and that call returns `EPERM` whenever *any*
//! Landlock ruleset is active on the calling process — confirmed by
//! elimination, not guessed, since granting the ruleset full read-write
//! access to `/` still failed identically. Landlock does not mediate
//! `mount()` in any current ABI, so there was no grant that could have
//! fixed this. Removed rather than narrowed: gVisor's own Sentry
//! seccomp/ptrace confinement is this tier's real boundary regardless
//! (CLAUDE.md's tier-qualified framing already treats Landlock at the
//! process tier as accident protection, not the security boundary), and
//! a defense-in-depth layer that makes the tier it defends non-functional
//! is a net loss. See
//! `openspec/changes/add-flox-services/tasks.md` task 6.5 and
//! `openspec/changes/add-gvisor-backend/design.md` decision 4 for the
//! full evidence trail and the reversal.
//!
//! **`use-nono-library` does not reopen this.** That rewrite landed after
//! this removal, and moved the *process* tier from exec'ing `nono wrap` to
//! calling `nono::Sandbox::apply_auto` in-process — unrelated twice over.
//! This layer used the `landlock` crate directly and never went through
//! nono in either form; and the blocker is the kernel's, since any active
//! ruleset, however installed, is what `mount()` returns `EPERM` under.
//!
//! **Verified live**, not just "compiles": with this ruleset removed, a
//! real `up` at `isolation = "hardened"` — including one with `[services]`
//! declared — completed end to end against a real rootless `runsc`:
//! `runsc run` started the sandbox, `exec` and the SSH round trip both
//! worked, `process-compose` came up under `runsc exec` with its control
//! socket reachable from the host, and `down` reaped everything cleanly.

use std::io;
use std::path::Path;
use std::process::Command;

use super::Platform;
use super::oci_spec::{NetworkMode, OciSpec};
use super::runsc_command::{self, Container};

/// Writes `bundle_dir/config.json` and creates `bundle_dir/rootfs`, plus
/// an empty directory at every mount's `destination` beneath it. Found
/// live, not reasoned about: gVisor's gofer resolves each mount target by
/// walking the rootfs tree with an open-based "safe mount" check, and
/// with no directory there to find, it fails ("expected to open
/// rootfs/<path>, but found <host path>") instead of creating one
/// implicitly the way `runc` does — every other OCI runtime this bundle
/// could have been modeled on pre-creates mount points, so this was
/// always required, just never exercised until a real `runsc run`
/// finally got far enough to hit it. Overwrites unconditionally, matching
/// `up`'s own "recompile the profile every run" behavior for the process
/// tier (`policy.json`) — `up --recreate` rebuilds this the same way.
pub fn materialize_bundle(bundle_dir: &Path, spec: &OciSpec) -> io::Result<()> {
    let rootfs = bundle_dir.join("rootfs");
    std::fs::create_dir_all(&rootfs)?;
    for mount in &spec.mounts {
        let relative = mount.destination.trim_start_matches('/');
        if !relative.is_empty() {
            std::fs::create_dir_all(rootfs.join(relative))?;
        }
    }
    std::fs::write(bundle_dir.join("config.json"), spec.to_json())
}

/// Starts the sandbox detached (`runsc run -detach`).
pub fn run(
    runsc: &Path,
    container: &Container<'_>,
    bundle_dir: &Path,
    platform: Platform,
    network: NetworkMode,
    host_uds: bool,
) -> io::Result<()> {
    std::fs::create_dir_all(container.state_root)?;

    let args = runsc_command::run_args(container, bundle_dir, platform, network, host_uds);
    let status = Command::new(runsc).args(&args).status()?;
    if !status.success() {
        return Err(io::Error::other(format!("runsc run exited with {status}")));
    }
    Ok(())
}

/// `runsc kill` then `runsc delete -force` — teardown must go through
/// both, in order: killing only the local client process does not stop
/// the sandbox (design.md decision 5, learned from mxc's own
/// implementation experience — the sandbox's process tree is separate
/// from whatever process issued `run`/`exec`). Best-effort: a container
/// that is already gone is not an error, matching
/// `state::clear_runtime_state`'s own idempotency.
pub fn teardown(runsc: &Path, container: &Container<'_>) -> io::Result<()> {
    let kill_args = runsc_command::kill_args(container, "SIGTERM");
    let _ = Command::new(runsc).args(&kill_args).status();

    let delete_args = runsc_command::delete_args(container);
    let _ = Command::new(runsc).args(&delete_args).status();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;
    use crate::policy;

    #[test]
    fn materialize_bundle_writes_config_json_and_pre_creates_every_mount_point() {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-gvisor-bundle-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let compiled = policy::compile(&manifest);
        let project_root = Path::new("/proj");
        let grants = vec!["/nix/store".to_string()];
        let env = std::collections::BTreeMap::new();
        let spec = super::super::oci_spec::build(
            &compiled,
            &super::super::oci_spec::BundleInputs {
                project_root,
                bundle_dir: &dir,
                read_only_grants: &grants,
                env: &env,
            },
        );

        materialize_bundle(&dir, &spec).unwrap();

        assert!(dir.join("rootfs").is_dir());
        // gVisor's gofer resolves each mount destination by walking the
        // rootfs tree; a missing mount point fails the whole sandbox
        // start (found live, not reasoned about — see this function's
        // doc comment).
        assert!(dir.join("rootfs/nix/store").is_dir());
        assert!(dir.join("rootfs/proj").is_dir());
        let written = std::fs::read_to_string(dir.join("config.json")).unwrap();
        assert_eq!(written, spec.to_json());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
