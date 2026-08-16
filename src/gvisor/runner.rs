//! Materializes the OCI bundle and drives `runsc` as real subprocesses:
//! `run` to start a sandbox, `kill`+`delete` to tear one down. Gated to
//! Linux since gVisor itself is Linux-only; everything here is the
//! host-touching half of the split [`super`]'s module doc describes —
//! [`super::oci_spec`] and [`super::runsc_command`] stay pure precisely
//! so only this file needs the `cfg`.
//!
//! **Unverified beyond compiling.** This devcontainer has no `runsc` on
//! `PATH`, no `/dev/kvm`, and currently cannot even create an
//! unprivileged user namespace (`unshare --user` fails `EPERM`) — see
//! the crate's devcontainer task group for why. Every function here is
//! written to the same standard as the rest of this codebase, but the
//! actual subprocess behavior (does `-d` really detach cleanly, does a
//! killed `runsc exec` client really propagate the signal, does the
//! Landlock ruleset applied in `run`'s `pre_exec` really get inherited
//! into the detached Sentry) needs confirming against a live sandbox
//! before any of it is relied on in production.

use std::io;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use landlock::{
    ABI, Access, AccessFs, Ruleset, RulesetAttr, RulesetCreatedAttr, path_beneath_rules,
};

use super::Platform;
use super::oci_spec::{NetworkMode, OciSpec};
use super::runsc_command::{self, Container};

/// Filesystem grants to apply as a Landlock profile on the process about
/// to `exec` into `runsc run` — the same host paths [`super::oci_spec`]
/// mounts into the sandbox (its `BundleInputs`), so the OS-level
/// restriction and the OCI mount list agree by construction rather than
/// by two call sites staying in sync manually.
pub struct LandlockGrants<'a> {
    pub read_write: &'a [String],
    pub read_only: &'a [String],
}

/// Writes `bundle_dir/config.json` and creates the (empty — `oci_spec`
/// never populates it, everything reachable comes from mounts)
/// `bundle_dir/rootfs`. Overwrites unconditionally, matching `up`'s own
/// "recompile the profile every run" behavior for the process tier
/// (`policy.json`) — `up --recreate` rebuilds this the same way.
pub fn materialize_bundle(bundle_dir: &Path, spec: &OciSpec) -> io::Result<()> {
    std::fs::create_dir_all(bundle_dir.join("rootfs"))?;
    std::fs::write(bundle_dir.join("config.json"), spec.to_json())
}

/// Standard dynamic-linker search paths, granted read+execute alongside
/// `runsc`'s own directory in [`run`] — see that function's doc comment
/// for why.
const DYNAMIC_LINKER_DIRS: &[&str] = &["/lib", "/lib64", "/usr/lib"];

/// Starts the sandbox detached (`runsc run -d`), with a Landlock profile
/// derived from `landlock` applied to this process — and, by Landlock's
/// own inheritance-across-fork-and-exec semantics, to `runsc` and
/// whatever it becomes/spawns as the Sentry — as defense in depth,
/// additive to gVisor's own seccomp confinement of Sentry (design.md
/// decision 4: no existing devcroft code applied Landlock directly
/// before this).
///
/// `runsc`'s own parent directory, plus [`DYNAMIC_LINKER_DIRS`], are
/// granted read+execute automatically — the same requirement and the
/// same fix `up.rs` already applies for the process tier's keeper binary
/// (`exe_dir`, "must be readable+executable inside the boundary it's
/// about to apply to itself"). A first version of this function granted
/// only `landlock`'s caller-supplied paths and nothing for `runsc`
/// itself, which — proven directly, not just reasoned about, by a test
/// that applies the exact same ruleset and then tries to exec a real
/// dynamically-linked binary — makes Landlock correctly deny the very
/// `execve` this function is about to perform: it fails on the ELF
/// interpreter and libc, not just the binary's own path. gVisor's
/// release binaries are static Go builds with no CGO and likely need
/// none of this, but granting the standard library search path anyway
/// is cheap, read-only, and means this does not silently break if that
/// ever changes — cheaper than re-deriving "is `runsc` actually static"
/// as a hard dependency of this function's correctness.
///
/// # Soundness
/// The `pre_exec` closure below runs in the forked child, after `fork`
/// and before `exec` — the same constraint every use of
/// `std::os::unix::process::CommandExt::pre_exec` carries: allocating in
/// a child with inherited-but-possibly-locked allocator state is only
/// sound if the parent was effectively single-threaded at the moment of
/// `fork`. `up` is a short-lived CLI invocation that has not yet started
/// any worker threads at the point it calls this (no tokio runtime, no
/// keeper accept loop), so that holds here — but it is a property of
/// *how this is called*, not something this function can enforce, so
/// this is not a general-purpose `pre_exec` helper.
pub fn run(
    runsc: &Path,
    container: &Container<'_>,
    bundle_dir: &Path,
    platform: Platform,
    network: NetworkMode,
    landlock: &LandlockGrants<'_>,
) -> io::Result<()> {
    std::fs::create_dir_all(container.state_root)?;

    let args = runsc_command::run_args(container, bundle_dir, platform, network);
    let read_write: Vec<String> = landlock.read_write.to_vec();
    let mut read_only: Vec<String> = landlock.read_only.to_vec();
    let runsc_dir = runsc
        .parent()
        .ok_or_else(|| io::Error::other("runsc executable path has no parent directory"))?;
    read_only.push(runsc_dir.to_string_lossy().into_owned());
    read_only.extend(DYNAMIC_LINKER_DIRS.iter().map(|p| p.to_string()));

    let mut cmd = Command::new(runsc);
    cmd.args(&args);
    // SAFETY: see this function's own "Soundness" doc section above.
    unsafe {
        cmd.pre_exec(move || restrict_self(&read_write, &read_only).map_err(io::Error::other));
    }

    let status = cmd.status()?;
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

fn restrict_self(
    read_write: &[String],
    read_only: &[String],
) -> Result<(), landlock::RulesetError> {
    Ruleset::default()
        .handle_access(AccessFs::from_all(ABI::V1))?
        .create()?
        .add_rules(path_beneath_rules(read_write, AccessFs::from_all(ABI::V1)))?
        .add_rules(path_beneath_rules(read_only, AccessFs::from_read(ABI::V1)))?
        .restrict_self()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;
    use crate::policy;

    #[test]
    fn materialize_bundle_writes_config_json_and_an_empty_rootfs() {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-gvisor-bundle-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let compiled = policy::compile(&manifest);
        let project_root = Path::new("/proj");
        let grants = Vec::new();
        let env = std::collections::BTreeMap::new();
        let spec = super::super::oci_spec::build(
            &compiled,
            &super::super::oci_spec::BundleInputs {
                project_root,
                read_only_grants: &grants,
                env: &env,
            },
        );

        materialize_bundle(&dir, &spec).unwrap();

        assert!(dir.join("rootfs").is_dir());
        let written = std::fs::read_to_string(dir.join("config.json")).unwrap();
        assert_eq!(written, spec.to_json());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Runs on this host directly (not against a container) — Landlock
    /// itself is real and testable even where `runsc` is not, since it's
    /// a kernel feature this crate now depends on regardless of gVisor.
    ///
    /// This test is what caught `run`'s original bug: an earlier version
    /// granted only the caller-supplied paths, with nothing for the
    /// binary about to be exec'd — Landlock correctly denied the
    /// `execve` a self-inflicted ruleset like that leaves unreachable.
    /// Exec'ing a real binary and asserting success, rather than only
    /// checking `restrict_self`'s own return status, is what makes that
    /// failure visible instead of silently "succeeding" while denying
    /// the one exec that matters.
    #[test]
    fn restrict_self_still_permits_exec_of_the_granted_binary_dir() {
        // A subprocess, not this test process directly: `restrict_self`
        // is meant to run once, immediately before exec, in a process
        // about to give up broad filesystem access — calling it in the
        // test harness process itself would leak that restriction into
        // every other test sharing this process.
        let dir = std::env::temp_dir().join(format!(
            "devcroft-gvisor-landlock-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let true_bin = crate::paths::resolve_on_path("true")
            .expect("`true` must be on PATH for this test to run");
        let true_bin_dir = true_bin.parent().unwrap().to_string_lossy().into_owned();

        let mut child = Command::new(&true_bin);
        let dir_rw = vec![dir.to_string_lossy().into_owned()];
        // Exactly what `run` grants for `runsc`: the exec'd binary's own
        // directory, plus the standard dynamic-linker search path — a
        // real coreutils `true` is dynamically linked against libc, same
        // as `runsc` might turn out to be despite being a static Go
        // build in practice (see `run`'s own doc comment).
        let mut dir_ro = vec![true_bin_dir];
        dir_ro.extend(DYNAMIC_LINKER_DIRS.iter().map(|p| p.to_string()));
        // SAFETY: same posture as `run`'s own pre_exec — this test
        // process spawns no threads before this call.
        unsafe {
            child.pre_exec(move || restrict_self(&dir_rw, &dir_ro).map_err(io::Error::other));
        }
        let status = child.status().unwrap();
        assert!(
            status.success(),
            "exec of a binary in a granted read-only dir must succeed under the applied ruleset"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
