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
/// compiled policy denies, and `up` must fail at layer `keeper` naming
/// the hook rather than reporting a sandbox that came up as though the
/// hook had succeeded.
#[test]
fn a_hook_denied_by_the_policy_fails_up_at_the_keeper_layer() {
    let denied =
        std::env::temp_dir().join(format!("devcroft-devenv-denied-{}", std::process::id()));
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
        Some(5),
        "a failing activation hook is a keeper-layer failure (exit 5): {out:?}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("keeper") && stderr.contains("activation"),
        "the failure must name the layer and the hook, got: {stderr}"
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
/// **Linux only, and the reason is not convenience.** macOS cannot make
/// either half of this measurement: host binaries execute there even at
/// ungranted paths, so `/usr/bin/cc` being reachable would say nothing,
/// and devenv's own preamble already fails on `/tmp` there — both
/// recorded in `docs/known-gaps.md`. A version of this test that passed
/// on macOS would be asserting the platform's gaps, not the provider's
/// behaviour.
#[test]
fn a_compile_inside_the_sandbox_uses_the_closures_toolchain_and_not_the_hosts() {
    if !cfg!(target_os = "linux") {
        eprintln!(
            "skipping: the closure-tier measurement is Linux-only — macOS executes host \
             binaries at ungranted paths, so the denial half cannot be observed \
             (docs/known-gaps.md)"
        );
        return;
    }
    let Some(sandbox) = Sandbox::new("closuretier", &["gcc"], "", true) else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());

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
