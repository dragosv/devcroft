//! Resolving the POSIX shell devcroft itself needs, from the closure it
//! already resolved.
//!
//! **Whose dependency this is.** The project never asked for a shell.
//! devcroft did: an SSH login session has to start one
//! (`ssh::server::LOGIN_SHELL`), `devcroft shell` falls back to one when
//! `$SHELL` names a host path (`exec::FALLBACK_SHELL`), and
//! process-compose launches every declared service through one
//! (`services::render_config`). Under a plain `flox activate` none of
//! this is visible, because the host's `PATH` is still there and `sh`
//! resolves to `/usr/bin/sh`. `own-policy-baseline` removed host
//! toolchain access without replacing what it removed, so all three
//! silently began resolving to a host path the compiled policy denies —
//! producing `keeper refused to spawn: Permission denied` for
//! `devcroft shell` and `shell request failed on channel 0` over SSH,
//! neither of which names the cause.
//!
//! Requiring the *project* to declare a shell was rejected: real flox,
//! nix and devbox manifests never declare one, because nothing about
//! their own tooling needs them to. Billing devcroft's runtime
//! dependency to the project's lockfile would fail every existing
//! project's first `up` for something it did not ask for.
//!
//! **Where the shell comes from instead.** Two sources, in order:
//!
//! 1. The resolved environment's own `PATH`, when the project happens to
//!    supply a shell (it declared `bash`, or a package that brings one
//!    into `bin/`). The project's own choice wins where it made one.
//! 2. Otherwise the closure's *requisites* — the store paths the
//!    resolved environment already transitively depends on. Every
//!    closure-tier provider is nix-backed, and a bash carrying `bin/sh`
//!    is present there even for an environment declaring only, say,
//!    `python3`: measured against `samples/flox-services-sample`, whose
//!    manifest declares no shell at all, three `bash-*` requisites of
//!    which all three ship `bin/sh`.
//!
//! This is deliberately **not** a scan of `/nix/store`. The store holds
//! paths from every environment on the host; picking a shell out of it
//! would work today only because devcroft grants `/nix/store` broadly,
//! which is the same "works by accident" this module exists to stop
//! (`provider::flox`'s note on consuming flox's own closure, and
//! `add-mount-isolation`, which tightens exactly those grants). Asking
//! the resolved closure what it contains is a question with one answer
//! per environment, and the answer stays true when the broad grant goes.
//!
//! The resolved path is absolute and is granted explicitly, so
//! `policy --render` shows it like any other rule.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The shell devcroft asks for by name. POSIX `sh`, never `bash` — the
/// contract is a POSIX shell, and every candidate below supplies `bin/sh`
/// whether or not it is bash underneath.
pub const SHELL_NAME: &str = "sh";

/// The store root every nix-backed provider materializes into.
const STORE_PREFIX: &str = "/nix/store";

/// An absolute shell path plus the store root to grant so it stays
/// reachable once `/nix/store` is no longer granted wholesale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedShell {
    /// Absolute path to a POSIX shell, inside the sandbox's own closure.
    pub path: PathBuf,
    /// The store path to add to the provider's read-only grants.
    ///
    /// `Option` rather than a bare `String` because a future artifact-tier
    /// provider (`docs/decisions.md` §1) would not be store-backed, and
    /// the grant it needs is its own to declare — but every closure-tier
    /// provider resolves to `Some`, and both routes below reject anything
    /// outside the store outright.
    pub grant: Option<String>,
}

/// Resolves a shell for `env`, preferring one the project supplies.
///
/// `grants` is what the provider declared in
/// [`Resolution::read_only_grants`](crate::provider::Resolution) — the
/// paths this sandbox will be allowed to read and execute from.
///
/// Returns `None` only when neither source has one, which `up` reports
/// rather than letting the sandbox come up with `shell`, `ssh` and
/// services all broken in ways that name nothing.
///
/// **Why `grants` rather than a hardcoded `/nix/store`.** The rule this
/// function enforces is *"the shell must be inside something the sandbox is
/// granted and can execute"*; the store prefix was a proxy for that, tight
/// only because every closure-tier provider grants store paths and nothing
/// else. Measured, so this is not a widening in practice: all three
/// providers get their grants from `capture::store_grants`, which returns a
/// `/nix/store`-rooted path in every branch — so for flox, nix and devbox
/// the two formulations select exactly the same candidates.
///
/// What it *does* unblock is a provider whose environment is not
/// store-backed at all. `ResolvedShell::grant` has been an `Option` for
/// precisely that anticipated case since before this change.
///
/// The guard protects **correctness, not a boundary**, which is what makes
/// generalizing it the right call rather than a risk: its recorded failure
/// was picking `/usr/bin/dash` and every service then dying with
/// `permission denied` — a sandbox that comes up broken, not one that
/// escapes. A host shell is still refused, because no provider declares a
/// grant containing one.
pub fn resolve(env: &BTreeMap<String, String>, grants: &[String]) -> Option<ResolvedShell> {
    resolve_on_path(env, grants).or_else(|| resolve_in_closure(env))
}

/// The grant `candidate` lives under, if any — the generalized form of
/// [`store_root`].
///
/// Compares canonicalized paths on both sides so a declared grant that is
/// itself a symlink (macOS's `/tmp` → `/private/tmp` being the case this
/// project keeps meeting) matches the resolved candidate.
fn granted_root(candidate: &Path, grants: &[String]) -> Option<PathBuf> {
    grants.iter().find_map(|g| {
        let root = Path::new(g);
        let real_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        candidate.starts_with(&real_root).then_some(real_root)
    })
}

/// A `PATH` search for [`SHELL_NAME`] through the *resolved* environment,
/// host-side at `up` — never the ambient `PATH` of whoever ran it.
///
/// Mirrors `services::resolve_in_env`, which does the same for
/// `process-compose`; the difference in what happens next is the point of
/// this module's header. A missing `process-compose` is the project's to
/// fix, because the project declared the services. A missing shell is
/// devcroft's.
///
/// **Only store-backed candidates count**, and that is the whole
/// correctness of this function rather than a refinement of it. A
/// provider's resolved `PATH` is its own `bin` directories *prepended to
/// the host's* — `/usr/local/bin:/usr/bin:/bin` are still on the end of
/// it. A plain search therefore succeeds on a host `sh` for exactly the
/// environments this module exists to serve: measured against
/// `samples/flox-services-sample`, the first version of this function
/// picked `/usr/bin/dash`, recorded it in `meta.json`, and every service
/// then died with `fork/exec /usr/bin/dash: permission denied` — the
/// original bug, moved rather than fixed.
pub fn resolve_on_path(env: &BTreeMap<String, String>, grants: &[String]) -> Option<ResolvedShell> {
    let path = env.get("PATH")?;
    for dir in path.split(':').filter(|d| !d.is_empty()) {
        let candidate = Path::new(dir).join(SHELL_NAME);
        if !is_executable_file(&candidate) {
            continue;
        }
        // Canonicalized before the store test, so a `.flox/run/...`
        // symlink is judged by what it points at — and so the path
        // recorded in `meta.json` is the store path itself, which
        // `up --recreate` cannot repoint out from under a client.
        let real = candidate.canonicalize().unwrap_or(candidate);
        // The store first, so a store-backed shell keeps reporting its own
        // store entry as the grant rather than the broader `/nix/store` a
        // provider declares — narrower, and unchanged from before.
        if let Some(root) = store_root(&real).or_else(|| granted_root(&real, grants)) {
            return Some(ResolvedShell {
                path: real,
                grant: Some(root.to_string_lossy().into_owned()),
            });
        }
    }
    None
}

/// The fallback: ask the closure `env` was resolved from what it already
/// depends on, and take a shell out of that.
///
/// Deterministic by construction — the requisite set is a property of the
/// closure, and candidates are sorted before one is chosen, so the same
/// environment yields the same shell on every `up` and on every host.
fn resolve_in_closure(env: &BTreeMap<String, String>) -> Option<ResolvedShell> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    for root in closure_roots(env) {
        candidates.extend(requisites(&root));
    }
    candidates.sort();
    candidates.dedup();

    for store_path in candidates {
        let candidate = store_path.join("bin").join(SHELL_NAME);
        if is_executable_file(&candidate) {
            return Some(ResolvedShell {
                path: candidate,
                grant: Some(store_path.to_string_lossy().into_owned()),
            });
        }
    }
    None
}

/// The store paths `env`'s own `PATH` points into, reduced to their store
/// roots — the entry points for a requisites query.
///
/// Derived from `PATH` rather than from a provider-specific field so this
/// stays one implementation across flox, nix and devbox: all three are
/// nix-backed, and all three put their `bin` directories in the store
/// (reached, for flox, through a `.flox/run/...` symlink, which is why
/// each entry is canonicalized first).
///
/// **Each entry contributes its parent's root as well as its own**, and
/// that is not belt-and-braces. flox's `.flox/run/<system>.<name>-dev/bin`
/// is a symlink farm only when more than one package contributes
/// binaries; with a single one it is a symlink *straight into that
/// package*, so canonicalizing the `bin` entry yields
/// `/nix/store/...-coreutils-9.11` rather than the environment. Its
/// requisites are coreutils' alone, which contain no shell — measured
/// against `tests/concurrency_and_suspend.rs`, whose environment is
/// exactly `flox init` plus `coreutils` and which failed to come up at
/// all until the parent was included. The parent canonicalizes to
/// `/nix/store/...-environment-dev`, whose requisites do contain one.
fn closure_roots(env: &BTreeMap<String, String>) -> Vec<PathBuf> {
    let Some(path) = env.get("PATH") else {
        return Vec::new();
    };
    let mut roots: Vec<PathBuf> = Vec::new();
    for dir in path.split(':').filter(|d| !d.is_empty()) {
        let entry = Path::new(dir);
        for candidate in [Some(entry), entry.parent()].into_iter().flatten() {
            let Ok(real) = candidate.canonicalize() else {
                continue;
            };
            if let Some(root) = store_root(&real)
                && !roots.contains(&root)
            {
                roots.push(root);
            }
        }
    }
    roots
}

/// `/nix/store/<hash>-<name>/any/deeper/path` → `/nix/store/<hash>-<name>`.
///
/// `None` for anything outside the store, which is what keeps a host path
/// from being granted by this route.
fn store_root(path: &Path) -> Option<PathBuf> {
    let rest = path.strip_prefix(STORE_PREFIX).ok()?;
    let first = rest.components().next()?;
    Some(Path::new(STORE_PREFIX).join(first))
}

/// `nix-store --query --requisites`, or an empty list when nix is not
/// reachable.
///
/// Failure is silent on purpose: the caller has already tried the
/// project's own `PATH`, and `up` reports "no shell" once, at the end,
/// rather than surfacing a nix invocation the user did not make.
fn requisites(root: &Path) -> Vec<PathBuf> {
    let Ok(out) = Command::new("nix-store")
        .arg("--query")
        .arg("--requisites")
        .arg(root)
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with_path(path: &str) -> BTreeMap<String, String> {
        BTreeMap::from([("PATH".to_string(), path.to_string())])
    }

    /// **The measurement that gated generalizing the guard**
    /// (`add-test-runtime-fixture` task 0.4): a shell on the host's own
    /// `PATH` must stay refused after the change, or the guard has been
    /// removed rather than widened.
    ///
    /// The concrete case, not an abstract one: this reproduces what the
    /// first version of `resolve_on_path` actually did against
    /// `samples/flox-services-sample` — picked `/usr/bin/dash`, recorded it
    /// in `meta.json`, and every service then died with
    /// `fork/exec /usr/bin/dash: permission denied`.
    ///
    /// It is asserted against the grants a real provider declares
    /// (`/nix/store`), because that is the only shape any of the three
    /// produce — `capture::store_grants` returns a store-rooted path in
    /// every branch.
    #[test]
    fn a_host_shell_is_still_refused_when_the_provider_grants_the_store() {
        let store_grant = vec!["/nix/store".to_string()];
        assert!(
            resolve_on_path(&env_with_path("/usr/local/bin:/usr/bin:/bin"), &store_grant).is_none(),
            "a host shell must not become resolvable just because the provider \
             declared a grant; the guard would be removed, not widened"
        );
    }

    /// The other half of the same measurement: the generalization does what
    /// it was for. A grant that actually contains the shell admits it.
    ///
    /// Without this the previous test could pass for the wrong reason — a
    /// guard that refuses *everything* also refuses host shells.
    #[test]
    fn a_shell_inside_a_declared_grant_is_accepted() {
        let dir = std::env::temp_dir().join(format!("dcshellgrant{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let sh = bin.join(SHELL_NAME);
        std::fs::write(&sh, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&sh, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let dir = dir.canonicalize().unwrap();

        let env = env_with_path(&dir.join("bin").to_string_lossy());
        let grants = vec![dir.to_string_lossy().into_owned()];

        let resolved = resolve_on_path(&env, &grants);
        let _ = std::fs::remove_dir_all(&dir);

        let resolved = resolved.expect("a shell inside a declared grant must resolve");
        assert!(resolved.path.starts_with(&dir));
        assert_eq!(resolved.grant, Some(dir.to_string_lossy().into_owned()));
    }

    #[test]
    fn store_root_reduces_a_deep_path_to_its_store_entry() {
        assert_eq!(
            store_root(Path::new("/nix/store/abc123-bash-5.3/bin/sh")),
            Some(PathBuf::from("/nix/store/abc123-bash-5.3"))
        );
    }

    /// The guard that keeps this route from ever granting a host path:
    /// a `PATH` entry outside the store contributes no grant and no
    /// requisites query.
    #[test]
    fn store_root_rejects_a_path_outside_the_store() {
        assert_eq!(store_root(Path::new("/usr/bin/sh")), None);
        assert_eq!(store_root(Path::new("/bin")), None);
    }

    #[test]
    fn closure_roots_ignores_host_path_entries() {
        // Nothing here canonicalizes into the store, so nothing is a root
        // — asserted rather than assumed, because a host entry slipping
        // through would put `/usr/bin/sh` back in play.
        assert!(closure_roots(&env_with_path("/usr/bin:/bin:/sbin")).is_empty());
    }

    #[test]
    fn resolve_on_path_finds_nothing_without_a_path_variable() {
        assert!(resolve_on_path(&BTreeMap::new(), &[]).is_none());
    }

    /// The regression that matters most here, because the first version
    /// of this module shipped the opposite behaviour: a perfectly good,
    /// executable `sh` that is not in the store is **not** selected.
    ///
    /// A provider's resolved `PATH` ends in the host's own directories, so
    /// accepting the first executable `sh` on it picks a host binary —
    /// `/usr/bin/dash` on this repo's devcontainer — which the compiled
    /// policy then denies at spawn, exactly reproducing the bug the module
    /// exists to remove.
    #[test]
    fn a_shell_outside_the_store_is_never_selected() {
        let dir = std::env::temp_dir().join(format!("devcroft-shell-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sh = dir.join(SHELL_NAME);
        std::fs::write(&sh, "#!/bin/sh\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&sh, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        assert!(resolve_on_path(&env_with_path(&dir.to_string_lossy()), &[]).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The host's real `PATH` is the case above in its natural habitat:
    /// this devcontainer has `/usr/bin/sh`, and it must still resolve to
    /// nothing.
    #[test]
    fn the_hosts_own_path_yields_no_shell() {
        assert!(resolve_on_path(&env_with_path("/usr/local/bin:/usr/bin:/bin"), &[]).is_none());
    }

    /// A non-executable file named `sh` is not a shell — otherwise a
    /// stray `sh` data file in a `bin/` directory would be selected and
    /// then fail to spawn, which is the failure mode this module exists
    /// to remove rather than relocate.
    #[test]
    fn a_non_executable_sh_is_not_selected() {
        let dir =
            std::env::temp_dir().join(format!("devcroft-shell-noexec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(SHELL_NAME), "not a program").unwrap();

        assert!(resolve_on_path(&env_with_path(&dir.to_string_lossy()), &[]).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }
}
