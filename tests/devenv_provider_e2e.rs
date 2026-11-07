//! End-to-end coverage for the `devenv` provider — add-devenv-provider's
//! task groups 3 and 5 — through the real built binary against a real
//! `devenv` plus `nix`, the pattern `devbox_provider_e2e.rs` uses.
//!
//! Guards on the **capability**, never on the binary: `devenv version`
//! succeeds against a store this host cannot reach, and every test that
//! then built an environment failed in a way that reads as a devcroft
//! regression (CLAUDE.md's standing rule).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn devcroft_bin() -> &'static str {
    env!("CARGO_BIN_EXE_devcroft")
}

fn run(cwd: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(devcroft_bin())
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn devenv_available() -> bool {
    Command::new("devenv")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
        && Command::new("nix").arg("--version").output().is_ok()
        && devcroft::provider::host_can_build_nix_closures()
}

struct Sandbox {
    name: String,
    project_root: PathBuf,
    /// Written by the project's `enterShell`, **inside** the project root
    /// — the sandbox denies writes anywhere else, so a marker outside it
    /// would be testing the filesystem policy rather than the hook.
    ///
    /// "The hook does not run host-side during resolution" is asserted
    /// where it can be asserted cleanly, against `resolve` itself with a
    /// marker outside the project root
    /// (`provider::devenv::tests::build_shell_does_not_run_enter_shell`).
    /// What this one adds is the other half: it ran, once, inside.
    marker: PathBuf,
}

impl Sandbox {
    /// `packages`: nixpkgs attribute names for `devenv.nix`.
    /// `enter_shell`: extra shell appended after the marker write.
    /// `locked`: run `devenv update` first — `false` leaves the project
    /// unlocked, for the missing-lock refusal.
    fn new(tag: &str, packages: &[&str], enter_shell: &str, locked: bool) -> Option<Self> {
        if !devenv_available() {
            eprintln!("skipping: devenv or nix not usable on this host");
            return None;
        }
        unsafe {
            std::env::set_var("DEVCROFT_KEEPER_EXE", devcroft_bin());
        }

        // `/tmp` rather than `std::env::temp_dir()`, and short on
        // purpose. macOS hands back a `/var/folders/…` path deep enough
        // that the service supervisor's unix socket under it exceeds the
        // OS `sun_path` limit — `up` refuses with exactly that message,
        // which is correct behaviour and a useless test failure.
        // Canonicalized because `/tmp` is itself a symlink there, and a
        // grant is compiled from the canonical path.
        let project_root =
            std::path::PathBuf::from(format!("/tmp/dce-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&project_root);
        std::fs::create_dir_all(&project_root).unwrap();
        let project_root = project_root.canonicalize().unwrap();
        let marker = project_root.join("hook-ran");

        let package_list = packages
            .iter()
            .map(|p| format!("pkgs.{p}"))
            .collect::<Vec<_>>()
            .join(" ");
        std::fs::write(
            project_root.join("devenv.nix"),
            format!(
                // Relative, deliberately. The hook runs with the project
                // root as its cwd, and on macOS `std::env::temp_dir()`
                // hands back `/var/folders/…` — the *symlinked* spelling
                // of a path the policy granted as `/private/var/folders/…`,
                // which the sandbox denies (docs/known-gaps.md, "A grant
                // does not cover the symlinked spelling of its own path").
                // An absolute marker here would fail the hook for a reason
                // that has nothing to do with devenv.
                "{{ pkgs, ... }}: {{\n  packages = [ {package_list} ];\n  \
                 enterShell = ''\n    echo ran >> hook-ran\n{enter_shell}\n  '';\n}}\n"
            ),
        )
        .unwrap();
        std::fs::write(
            project_root.join("devenv.yaml"),
            "inputs:\n  nixpkgs:\n    url: github:cachix/devenv-nixpkgs/rolling\n",
        )
        .unwrap();

        if locked {
            let update = Command::new("devenv")
                .arg("update")
                .current_dir(&project_root)
                .output()
                .unwrap();
            if !update.status.success() {
                eprintln!(
                    "skipping: devenv update failed (likely no network for nixpkgs): {}",
                    String::from_utf8_lossy(&update.stderr)
                );
                return None;
            }
        }

        let name = format!("e2edevenv{tag}{}", std::process::id());
        std::fs::write(
            project_root.join("devcroft.toml"),
            format!("[sandbox]\nname = {name:?}\n\n[env]\nprovider = \"devenv\"\n"),
        )
        .unwrap();

        Some(Sandbox {
            name,
            project_root,
            marker,
        })
    }

    /// A project whose `devenv.nix` body is supplied verbatim, for the
    /// service tests — `new` builds a fixed shape, and these need
    /// arbitrary `processes` blocks.
    fn with_processes(tag: &str, processes: &str) -> Option<Self> {
        let sandbox = Self::new(tag, &[], "", true)?;
        std::fs::write(
            sandbox.project_root.join("devenv.nix"),
            // `process-compose` comes from the project's own closure,
            // never the host — `services::resolve_in_env`. Any project
            // declaring services must supply it, the same requirement a
            // flox project has, and `decouple-service-supervisor` is the
            // change that exists to remove it.
            format!(
                "{{ pkgs, ... }}: {{\n  packages = [ pkgs.process-compose ];\n{processes}\n}}\n"
            ),
        )
        .unwrap();
        Some(sandbox)
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        run(&self.project_root, args)
    }

    fn marker_lines(&self) -> usize {
        std::fs::read_to_string(&self.marker)
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    fn state_root(&self) -> PathBuf {
        devcroft::lifecycle::StatePaths::new(&self.name)
            .unwrap()
            .root
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = self.run(&["rm", "--yes"]);
        let _ = std::fs::remove_dir_all(self.state_root());
        let _ = std::fs::remove_dir_all(&self.project_root);
    }
}

/// env-provider spec: "Environment captured host-side", and the toolchain
/// is visible in a session under the default deny-all network —
/// materialization happened at `up`, before any restriction.
#[test]
fn up_resolves_devenv_and_the_toolchain_is_visible_in_a_session() {
    let Some(sandbox) = Sandbox::new("up", &["ripgrep"], "", true) else {
        return;
    };

    let out = sandbox.run(&["up"]);
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("is started"));

    let out = sandbox.run(&["exec", "--", "rg", "--version"]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("ripgrep"),
        "expected the devenv-resolved ripgrep to run inside the session"
    );
}

/// env-provider spec: "The hook does not run during provisioning", and
/// "Hook runs once, inside" — the criterion-4 guarantee as a test rather
/// than a claim, with the sentinel method the proposal was written with.
#[test]
fn enter_shell_does_not_run_during_up_and_runs_once_inside() {
    let Some(sandbox) = Sandbox::new("hook", &[], "", true) else {
        return;
    };

    let out = sandbox.run(&["up"]);
    assert!(out.status.success(), "{out:?}");

    assert_eq!(
        sandbox.marker_lines(),
        1,
        "enterShell must run exactly once — inside the sandbox, after restriction. \
         0 means it never ran; 2 or more means devcroft ran it as well as the provider"
    );
}

/// env-provider spec: "No host-side hook warning". A devenv project with
/// an `enterShell` must not produce the warning a flox project with
/// `on-activate` produces, because the condition it describes — project
/// code executed host-side, unconfined — does not hold here.
#[test]
fn up_does_not_warn_that_a_hook_ran_host_side() {
    let Some(sandbox) = Sandbox::new("nowarn", &[], "", true) else {
        return;
    };

    let out = sandbox.run(&["up"]);
    assert!(out.status.success(), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("activation hook on the host"),
        "devenv captures without executing the hook, so the warning must not fire: {stderr}"
    );
}

/// env-provider spec: "Store paths become readable", origin
/// `provider:devenv`, and provider resolution adds no write grants.
#[test]
fn policy_render_shows_the_devenv_store_grant_after_up() {
    let Some(sandbox) = Sandbox::new("policygrant", &["ripgrep"], "", true) else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());

    let out = sandbox.run(&["policy", "--render"]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("/nix/store") && stdout.contains("provider:devenv"),
        "got: {stdout}"
    );
}

/// env-provider spec: "Missing environment, not missing feature".
#[test]
fn up_fails_at_provider_layer_when_devenv_nix_is_missing() {
    if !devenv_available() {
        eprintln!("skipping: devenv or nix not usable on this host");
        return;
    }
    let dir =
        std::env::temp_dir().join(format!("devcroft-devenv-e2e-noenv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("devcroft.toml"),
        format!(
            "[sandbox]\nname = \"e2edevenvnoenv{}\"\n\n[env]\nprovider = \"devenv\"\n",
            std::process::id()
        ),
    )
    .unwrap();

    let out = run(&dir, &["up"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("provider"), "got: {stderr}");
    assert!(stderr.contains("devenv init"), "got: {stderr}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// env-provider spec: "Unlocked project is told to lock, not told capture
/// resolved". Measured (task 0.4): `devenv build shell` *writes* a
/// missing lockfile, so without the up-front refusal this `up` would
/// resolve inputs and leave a lockfile behind.
#[test]
fn up_refuses_an_unlocked_project_before_capture_can_write_a_lockfile() {
    let Some(sandbox) = Sandbox::new("nolock", &[], "", false) else {
        return;
    };

    let out = sandbox.run(&["up"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("devenv update"), "got: {stderr}");
    assert!(
        !sandbox.project_root.join("devenv.lock").is_file(),
        "a refused `up` must not leave a lockfile capture wrote"
    );
}

/// env-provider spec: "Capture writes only where the provider owns the
/// path" (design.md decision 6). `.devenv/` is devenv's own cache and
/// GC-root store and is left alone; nothing else in the tree changes.
#[test]
fn up_leaves_the_declaration_files_byte_identical() {
    let Some(sandbox) = Sandbox::new("worktree", &[], "", true) else {
        return;
    };
    let before: Vec<(PathBuf, Vec<u8>)> = ["devenv.nix", "devenv.yaml", "devenv.lock"]
        .iter()
        .map(|n| {
            let p = sandbox.project_root.join(n);
            let b = std::fs::read(&p).unwrap();
            (p, b)
        })
        .collect();

    assert!(sandbox.run(&["up"]).status.success());

    for (path, bytes) in before {
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "{} changed across `up`",
            path.display()
        );
    }
}

/// env-provider spec: "Editing any declaration file flips status" — all
/// three, because `devenv.yaml` carries the inputs and can change what
/// resolves with `devenv.nix` untouched.
#[test]
fn editing_any_of_the_three_declaration_files_flips_status_to_stale() {
    // Tags deliberately free of the word this test greps for: an earlier
    // version used "stalenix", and the sandbox *name* derived from it
    // matched the freshness check, so the pre-edit assertion failed
    // against a sandbox that was in fact fresh.
    for (tag, file, contents) in [
        (
            "editnix",
            "devenv.nix",
            "{ pkgs, ... }: { packages = [ ]; }\n",
        ),
        ("edityaml", "devenv.yaml", "inputs: {}\n"),
        ("editlock", "devenv.lock", "{\"nodes\": {}}\n"),
    ] {
        let Some(sandbox) = Sandbox::new(tag, &[], "", true) else {
            return;
        };
        assert!(sandbox.run(&["up"]).status.success());

        // The `env:` line specifically, not the whole output — the
        // sandbox name appears there too.
        let env_line = |out: &std::process::Output| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .find(|l| l.starts_with("env:"))
                .unwrap_or("<no env line>")
                .to_string()
        };

        let fresh = sandbox.run(&["status"]);
        assert_eq!(
            env_line(&fresh),
            "env: fresh",
            "should be fresh before the edit"
        );

        std::fs::write(sandbox.project_root.join(file), contents).unwrap();

        let out = sandbox.run(&["status"]);
        assert!(
            env_line(&out).contains("stale"),
            "editing {file} must flip status to stale, got: {}",
            env_line(&out)
        );
    }
}

/// env-provider spec: "Login session uses a shell from the closure". The
/// invariant `own-policy-baseline` broke silently for three providers at
/// once — and `exec` passing proves nothing about it, because `exec` does
/// not go through the resolved shell.
#[test]
fn a_login_shell_resolves_out_of_the_devenv_closure() {
    let Some(sandbox) = Sandbox::new("loginshell", &[], "", true) else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());

    let meta = devcroft::lifecycle::StatePaths::new(&sandbox.name)
        .unwrap()
        .meta;
    let shell = devcroft::lifecycle::read_meta(&meta)
        .unwrap()
        .and_then(|m| m.shell)
        .expect("`up` must record an absolute shell resolved from the closure");
    assert!(
        std::path::Path::new(&shell).is_absolute() && shell != "sh",
        "the recorded shell must be an absolute path from the closure, got {shell:?}"
    );

    // Not started here. `devcroft shell` is interactive by contract —
    // it takes no command — and macOS refuses interactive pty sessions
    // (docs/known-gaps.md), so starting one would assert the platform
    // rather than the provider. What is provider-specific, and what
    // `own-policy-baseline` broke silently for three providers at once,
    // is whether the closure yields a shell at all: that is asserted
    // above, and again against `shell::resolve` directly in
    // `provider::devenv`'s own tests.
    assert!(
        shell.contains("/nix/store/"),
        "the shell must come from the closure, not the host: {shell:?}"
    );
}

/// env-provider spec: "A hook needing host tooling is denied, not
/// silently granted". The hook writes outside the project root, which the
/// compiled policy denies, and `up` must fail naming the hook rather than
/// reporting a sandbox that came up as though the hook had succeeded.
///
/// **Layer `provider` since `sandbox-provisioning`, not `keeper`, despite
/// the name** (kept: other changes cite it). The hook now runs in
/// `__provision`, before any keeper, and a refusal there is attributed by
/// the policy (`lifecycle::attribution`). This test still expected exit 5
/// until 2026-11-03; it runs only where devenv is on `PATH`, so the
/// mismatch went unseen.
#[test]
fn a_hook_denied_by_the_policy_fails_up_at_the_keeper_layer() {
    // Under the real home, which no sandbox is granted on either platform.
    // It was under the temp dir, which is only denied on macOS (its
    // symlinked `/tmp`): on Linux the sandbox has a private `/tmp`, so the
    // write *succeeded*, harmlessly and inside the sandbox, and `up`
    // rightly came up. Found on this test's first Linux run.
    let denied = std::path::PathBuf::from(std::env::var("HOME").unwrap())
        .join(format!(".devcroft-devenv-denied-{}", std::process::id()));
    let _ = std::fs::remove_file(&denied);
    let Some(sandbox) = Sandbox::new(
        "deniedhook",
        &[],
        &format!("    echo denied > {}", denied.display()),
        true,
    ) else {
        return;
    };

    let out = sandbox.run(&["up"]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "a failing activation hook is a provider-layer failure (exit 3): {out:?}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("provider: provisioning")
            && stderr.contains("activation")
            && stderr.contains("refused"),
        "the failure must name the layer, the hook and the refusal, got: {stderr}"
    );
    assert!(
        !denied.exists(),
        "the sandbox must not have been able to write outside the project root"
    );
}

/// The measurement that makes "closure tier" a claim about *this
/// provider* rather than about Nix in the abstract (task 5.3): a real
/// compile inside the sandbox needs the project root and the store, and
/// the host's own toolchain is denied.
///
/// **The compile half is Linux only, and the reason is not convenience:**
/// a C toolchain from a closure cannot *link* on macOS (`docs/known-gaps.md`,
/// "A C toolchain from a closure cannot link on macOS" — the nix linker
/// wrapper reads `/dev/fd/63`, which the baseline does not grant). The
/// denial half runs on both platforms: it used to be macOS-skipped too,
/// because host binaries executed there at ungranted paths, and that gap
/// is closed (same file, "Host binaries execute on macOS — fixed";
/// `tests/devbox_provider_e2e.rs` is the same split, measured on a macOS
/// host). A version of this test whose compile half passed on macOS
/// would be asserting the platform's gap, not the provider's behaviour.
#[test]
fn a_compile_inside_the_sandbox_uses_the_closures_toolchain_and_not_the_hosts() {
    let Some(sandbox) = Sandbox::new("closuretier", &["gcc"], "", true) else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());

    #[cfg(target_os = "linux")]
    {
        std::fs::write(
            sandbox.project_root.join("probe.c"),
            "int main(void) { return 0; }\n",
        )
        .unwrap();

        let out = sandbox.run(&["exec", "--", "sh", "-c", "cc -o probe probe.c && ./probe"]);
        assert!(
            out.status.success(),
            "a compile must work from the closure's own toolchain, under deny-all network: {out:?}"
        );
    }

    let host = sandbox.run(&["exec", "--", "/usr/bin/cc", "--version"]);
    assert!(
        !host.status.success(),
        "the host's toolchain must be denied — a closure-tier provider that falls back to \
         host libraries has smuggled `host` passthrough back in: {host:?}"
    );
}

/// `services` spec, asserted for devenv: declared processes are
/// supervised inside the boundary, enumerable while the sandbox is up,
/// and reaped at teardown.
#[test]
fn declared_processes_run_as_supervised_services() {
    let Some(sandbox) = Sandbox::with_processes(
        "services",
        r#"  processes.slow.exec = "sleep 600";
  processes.alsoslow = { exec = "sleep 600"; after = [ "devenv:processes:slow" ]; };"#,
    ) else {
        return;
    };

    let out = sandbox.run(&["up"]);
    assert!(out.status.success(), "{out:?}");

    let ps = sandbox.run(&["ps"]);
    assert!(ps.status.success(), "{ps:?}");
    let listing = String::from_utf8_lossy(&ps.stdout);
    assert!(
        listing.contains("slow") && listing.contains("alsoslow"),
        "both declared processes must be enumerable while the sandbox is up, got: {listing}"
    );

    assert!(sandbox.run(&["down"]).status.success());
    // The `services` spec's own wording: verified by observing process
    // absence rather than by a stop command's exit status.
    let survivors = Command::new("pgrep")
        .args(["-f", "sleep 600"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    assert!(
        survivors.is_empty(),
        "no service process may outlive the sandbox, but these did: {survivors}"
    );
}

/// `add-devenv-services` design.md decision 2a, end to end: the spelling
/// devenv itself ignores is refused rather than being honoured, so
/// devcroft and devenv cannot disagree about what the same project does.
#[test]
fn a_bare_ordering_name_is_refused_at_the_provider_layer() {
    let Some(sandbox) = Sandbox::with_processes(
        "bareorder",
        r#"  processes.web.exec = "sleep 600";
  processes.worker = { exec = "sleep 600"; after = [ "web" ]; };"#,
    ) else {
        return;
    };

    let out = sandbox.run(&["up"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("worker") && stderr.contains("after"),
        "the refusal must name the process and the field, got: {stderr}"
    );
    assert!(
        stderr.contains("devenv:processes:web"),
        "and it must show the spelling that works, got: {stderr}"
    );
}

/// `add-service-readiness`: a dependency on a target that declares a
/// probe waits for **ready**, not merely for started.
///
/// Observed by order rather than by reading the generated config — the
/// config only says what devcroft asked for, and the claim is about what
/// the supervisor did.
#[test]
fn a_dependent_waits_for_its_targets_readiness_probe() {
    let Some(sandbox) = Sandbox::with_processes(
        "readywait",
        r#"  processes.slowstart = {
    exec = "sleep 600";
    ready = { exec = "test -f ready-marker"; period = 1; };
  };
  processes.dependent = {
    exec = "sleep 600";
    after = [ "devenv:processes:slowstart" ];
  };"#,
    ) else {
        return;
    };

    let out = sandbox.run(&["up"]);
    assert!(
        out.status.success(),
        "a never-ready probe must not block `up`: {out:?}"
    );

    // The probe cannot pass yet: the marker does not exist. So the
    // dependent must not be running, while its target is.
    let status = String::from_utf8_lossy(&sandbox.run(&["status"]).stdout).into_owned();
    assert!(
        status.contains("slowstart"),
        "the probed service should be started: {status}"
    );
    let dependent_line = status
        .lines()
        .find(|l| l.contains("dependent"))
        .unwrap_or("")
        .to_string();
    assert!(
        !dependent_line.contains("running"),
        "a dependent must wait while its target's probe has not passed, got: {dependent_line:?}"
    );
}

/// `add-service-readiness` design.md decision 4: a probe that never
/// passes does not block `up`. The sandbox comes up and the state is
/// reported, because a hang hides the diagnosis.
#[test]
fn a_probe_that_never_passes_leaves_the_sandbox_usable() {
    let Some(sandbox) = Sandbox::with_processes(
        "neverready",
        r#"  processes.never = {
    exec = "sleep 600";
    ready = { exec = "false"; period = 1; };
  };"#,
    ) else {
        return;
    };

    assert!(sandbox.run(&["up"]).status.success());
    // The sandbox is usable even though a service will never be ready.
    let out = sandbox.run(&["exec", "--", "true"]);
    assert!(
        out.status.success(),
        "a never-ready service must not make the sandbox unusable: {out:?}"
    );
}

/// What `enterShell` exports reaches a session. devenv's own `enterShell`
/// sets `MANPATH` and, on Linux, `LOCALE_ARCHIVE`; the
/// hook-free capture has none of them, and design.md counted on running the
/// hook in the sandbox to restore them. It did not, until the keeper read
/// back what the hook exported: measured unset in a real session on Linux.
/// The project's own export rides along as a control that does not depend
/// on what devenv's preamble happens to set.
#[test]
fn what_enter_shell_exports_reaches_a_session() {
    let Some(sandbox) = Sandbox::new("exports", &[], "    export FROM_ENTER_SHELL=yes", true)
    else {
        return;
    };
    let up = sandbox.run(&["up"]);
    assert!(up.status.success(), "{up:?}");
    let out = sandbox.run(&[
        "exec",
        "--",
        "sh",
        "-c",
        "echo \"${FROM_ENTER_SHELL-unset}|${MANPATH:+set}|${LOCALE_ARCHIVE:+set}\"",
    ]);
    assert!(out.status.success(), "{out:?}");
    let seen = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    let fields: Vec<&str> = seen.split('|').collect();
    assert_eq!(fields[0], "yes", "the project's own export: {seen}");
    assert_eq!(fields[1], "set", "MANPATH from enterShell: {seen}");
    if cfg!(target_os = "linux") {
        assert_eq!(fields[2], "set", "LOCALE_ARCHIVE from enterShell: {seen}");
    }
}

/// `sandbox-provisioning` task 23: `up` holds what `enterShell` exported as
/// data, so a `PATH` entry it added that the policy does not grant is named
/// once at `up` rather than found missing in a session. A directory in the
/// project draws nothing, nor does what devenv's own `PATH` already held.
#[test]
fn an_exported_path_the_policy_does_not_grant_is_named_at_up() {
    let Some(sandbox) = Sandbox::new(
        "pathwarn",
        &[],
        "    export PATH=/opt/devcroft-nowhere/bin:$DEVENV_ROOT/tools/bin:$PATH",
        true,
    ) else {
        return;
    };
    let up = sandbox.run(&["up"]);
    assert!(up.status.success(), "{up:?}");
    let stderr = String::from_utf8_lossy(&up.stderr);
    let warnings: Vec<&str> = stderr
        .lines()
        .filter(|l| l.contains("activation script put"))
        .collect();
    assert_eq!(warnings.len(), 1, "exactly one warning:\n{stderr}");
    assert!(
        warnings[0].contains("/opt/devcroft-nowhere/bin"),
        "{stderr}"
    );
    assert!(
        !warnings[0].contains("tools/bin"),
        "the project is granted: {stderr}"
    );
    assert!(
        !warnings[0].contains("/usr/bin"),
        "not the script's: {stderr}"
    );
}

/// `sandbox-provisioning` task 30: where the activation script ran, `up`
/// says what it ran under, at the strength the tier has, and says nothing
/// of the kind for an `up` that started nothing. Every devenv project has a
/// script — devenv's own `enterShell` preamble is one, even when the project
/// writes none — so the case of no script is not a devenv one.
#[test]
fn up_states_what_the_activation_script_ran_under() {
    let Some(with) = Sandbox::new("reach", &[], "    true", true) else {
        return;
    };
    let up = with.run(&["up"]);
    assert!(up.status.success(), "{up:?}");
    let stdout = String::from_utf8_lossy(&up.stdout);
    let line = stdout
        .lines()
        .find(|l| l.starts_with("devcroft: provisioning:"))
        .unwrap_or_else(|| panic!("no provisioning line:\n{stdout}"));
    assert!(line.contains("ran inside the sandbox"), "{line}");
    assert!(line.contains("no network"), "{line}");
    assert!(
        line.contains("not a boundary for code written to escape"),
        "{line}"
    );
    let again = with.run(&["up"]);
    assert!(
        !String::from_utf8_lossy(&again.stdout).contains("devcroft: provisioning:"),
        "an `up` that started nothing ran nothing"
    );
}

/// `fix-service-hook-ordering` tasks 2.1, 2.3, 2.6 and 2.7: every hook
/// completes before the first service starts, observed from the times
/// each one recorded itself rather than inferred from the service coming
/// up, which a racing order also produces.
///
/// The hooks are slow to start on purpose, each sourcing a file and
/// sleeping before it writes anything. A one-liner beat process-compose's
/// boot by 9 ms before the fix, so a test built on one passed against the
/// race too. With a second of work ahead of the timestamp, a service
/// started in parallel stamps first by about that second.
#[test]
fn every_hook_completes_before_the_first_service_starts() {
    // The service's `sleep` carries a duration no other process has, so
    // the absence check below finds this service and nothing else: after
    // `exec`, its command line is the `sleep` alone.
    let token = format!("sleep 600.{}", std::process::id());
    let Some(sandbox) = Sandbox::with_processes(
        "hookorder",
        &format!(
            r#"  enterShell = ''
    sleep 1
    date +%s%N > activation-done
  '';
  processes.stamp.exec = "date +%s%N > service-started; exec {token}";"#
        ),
    ) else {
        return;
    };
    let root = &sandbox.project_root;
    std::fs::write(
        root.join("slow-start.sh"),
        "sleep 1\ndate +%s%N > post-create-done\n",
    )
    .unwrap();
    let manifest = std::fs::read_to_string(root.join("devcroft.toml")).unwrap();
    std::fs::write(
        root.join("devcroft.toml"),
        format!("{manifest}\n[hooks]\npost_create = \". ./slow-start.sh\"\n"),
    )
    .unwrap();

    let out = sandbox.run(&["up"]);
    assert!(out.status.success(), "{out:?}");

    let started = root.join("service-started");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !started.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let stamp = |name: &str| -> i128 {
        let text = std::fs::read_to_string(root.join(name))
            .unwrap_or_else(|e| panic!("{name} was never written: {e}"));
        text.trim()
            .parse()
            .unwrap_or_else(|e| panic!("{name} holds {text:?}: {e}"))
    };
    let service = stamp("service-started");
    for hook in ["activation-done", "post-create-done"] {
        let margin_ms = (service - stamp(hook)) / 1_000_000;
        // The margin, not only the sign (task 2.7): printed so a run
        // records it, and asserted positive. Ordering is one sequence in
        // the keeper, so what is left is process-compose's own start-up.
        eprintln!("{hook}: the first service started {margin_ms} ms after it");
        assert!(
            margin_ms > 0,
            "the service started {} ms before `{hook}` finished",
            -margin_ms
        );
    }

    // Task 2.3, by process absence: `down` leaves no service behind,
    // hooks or not. Found first, so the absence is not that of a pattern
    // that never matched.
    let live = || {
        Command::new("pgrep")
            .args(["-f", &token])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    assert!(!live().is_empty(), "the service must be found while up");
    assert!(sandbox.run(&["down"]).status.success());
    let survivors = live();
    assert!(
        survivors.is_empty(),
        "no service process may outlive `down`, but these did: {survivors}"
    );
}

/// A `post_create` that failed runs again on the next `up`. The rule is
/// "once, after the first successful `up`", and recovery used to read any
/// existing state as that success: the second `up` reported "recovered",
/// exited 0, and the sandbox came up without its setup ever having
/// completed. Measured on a devbox postgres project whose `initdb` hook
/// failed. Once it has succeeded, it is not run again.
#[test]
fn a_failed_post_create_runs_again_on_the_next_up() {
    let Some(sandbox) = Sandbox::new("pcretry", &[], "", true) else {
        return;
    };
    let root = &sandbox.project_root;
    let manifest = std::fs::read_to_string(root.join("devcroft.toml")).unwrap();
    std::fs::write(
        root.join("devcroft.toml"),
        format!(
            "{manifest}\n[hooks]\npost_create = \"test -f ready && echo >> post-create-ran\"\n"
        ),
    )
    .unwrap();
    let ran = || {
        std::fs::read_to_string(root.join("post-create-ran"))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    };

    let first = sandbox.run(&["up"]);
    assert_eq!(first.status.code(), Some(5), "{first:?}");
    assert_eq!(ran(), 0);

    std::fs::write(root.join("ready"), "").unwrap();
    let second = sandbox.run(&["up"]);
    assert!(second.status.success(), "{second:?}");
    assert_eq!(ran(), 1, "the failed post_create must run on the next up");

    // Done now: a keeper lost and recovered does not run it again.
    let pidfile = devcroft::lifecycle::StatePaths::new(&sandbox.name)
        .unwrap()
        .pidfile;
    let pid: i32 = std::fs::read_to_string(pidfile)
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse().ok())
        .expect("a pidfile for the running keeper");
    unsafe { libc::kill(pid, libc::SIGKILL) };
    std::thread::sleep(std::time::Duration::from_millis(300));
    let third = sandbox.run(&["up"]);
    assert!(third.status.success(), "{third:?}");
    assert!(
        String::from_utf8_lossy(&third.stdout).contains("recovered"),
        "{third:?}"
    );
    assert_eq!(ran(), 1, "a post_create that succeeded must not run again");
}
