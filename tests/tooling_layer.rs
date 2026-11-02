//! `add-agent-workload` task groups 3 and 4: what the user's configuration
//! adds to a sandbox — a tooling layer, and credentials from the invoking
//! shell — end to end, through the real binary against a real devenv. The project and the layer
//! are both devenv environments pinned by the agent samples' own
//! `devenv.lock`, so nothing is fetched and the store already has them.
//!
//! Every run points `XDG_CONFIG_HOME` at the test's own directory: the
//! binary reads the user configuration, and a test that read the
//! developer's real `~/.config` would test their machine.
//!
//! Guards on the capability, never on the binary (CLAUDE.md).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The variable the credential tests request; removed from every other run.
const CREDENTIAL: &str = "DCT_TEST_CREDENTIAL";

fn devcroft_bin() -> &'static str {
    env!("CARGO_BIN_EXE_devcroft")
}

fn devenv_available() -> bool {
    Command::new("devenv")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
        && devcroft::provider::host_can_build_nix_closures()
}

/// A devenv environment at `dir` with `packages`, pinned by a sample's lock.
fn devenv_env(dir: &Path, packages: &str, enter_shell: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/devenv-claude-sample");
    for file in ["devenv.yaml", "devenv.lock"] {
        std::fs::copy(sample.join(file), dir.join(file)).unwrap();
    }
    std::fs::write(
        dir.join("devenv.nix"),
        format!("{{ pkgs, ... }}: {{\n  packages = [ {packages} ];\n  enterShell = ''{enter_shell}'';\n}}\n"),
    )
    .unwrap();
}

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    layer: PathBuf,
    name: String,
}

impl Fixture {
    fn new(tag: &str) -> Option<Self> {
        if !devenv_available() {
            eprintln!("skipping: devenv or a usable nix store not on this host");
            return None;
        }
        unsafe {
            std::env::set_var("DEVCROFT_KEEPER_EXE", devcroft_bin());
        }
        // Short, under `/tmp`: the reason `devenv_provider_e2e` gives.
        let root = PathBuf::from(format!("/tmp/dct-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("xdg")).unwrap();
        let root = root.canonicalize().unwrap();
        let (project, layer) = (root.join("project"), root.join("layer"));
        // The project knows nothing of any layer, and has a `git` of its own:
        // a different store path from the layer's, since one store path on
        // both sides is the same tool, not a shadowed one.
        devenv_env(&project, "pkgs.jq pkgs.gitMinimal", "");
        // The layer's hook writes a marker it must never get to write.
        devenv_env(
            &layer,
            "pkgs.ripgrep pkgs.git",
            "touch $DEVENV_ROOT/hook-ran",
        );
        let name = format!("dctools{tag}{}", std::process::id());
        std::fs::write(
            project.join("devcroft.toml"),
            format!("[sandbox]\nname = {name:?}\n\n[env]\nprovider = \"devenv\"\n"),
        )
        .unwrap();
        Some(Fixture {
            root,
            project,
            layer,
            name,
        })
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        self.run_with(CREDENTIAL, None, args)
    }

    /// `run`, with `name` set to `value` in the invoking shell, or removed
    /// from it for `None`.
    fn run_with(&self, name: &str, value: Option<&str>, args: &[&str]) -> std::process::Output {
        let mut cmd = Command::new(devcroft_bin());
        cmd.args(args)
            .current_dir(&self.project)
            .env("XDG_CONFIG_HOME", self.root.join("xdg"))
            .stdin(Stdio::null());
        match value {
            Some(v) => cmd.env(name, v),
            None => cmd.env_remove(name),
        };
        cmd.output().unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "devcroft {args:?} failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn layer_arg(&self) -> String {
        self.layer.to_string_lossy().into_owned()
    }

    fn configure(&self, text: &str) {
        let dir = self.root.join("xdg/devcroft");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), text).unwrap();
    }

    /// A session's environment, as a map.
    fn session_env(&self) -> BTreeMap<String, String> {
        self.ok(&["exec", "--", "env"])
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.run(&["rm", "--yes"]);
        if let Ok(paths) = devcroft::lifecycle::StatePaths::new(&self.name) {
            let _ = std::fs::remove_dir_all(paths.root);
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Tasks 3.10 and 3.11, and the hook half of 3.1: with and without a layer,
/// a session differs only in `PATH`; the policy only by the layer's grant;
/// a name in both is the project's, and `status` names it shadowed; the
/// layer's own hook never runs.
#[test]
fn a_layer_adds_path_entries_and_one_grant_and_nothing_else() {
    let Some(f) = Fixture::new("only") else {
        return;
    };

    let up = f.ok(&["up"]);
    assert!(!up.contains("tools:"), "no layer, no line: {up}");
    let without_env = f.session_env();
    let without_policy = f.ok(&["policy", "--render"]);
    assert!(!without_policy.contains("tools:"), "{without_policy}");
    let project_git = f.ok(&["exec", "--", "sh", "-c", "command -v git"]);

    let layer = f.layer_arg();
    let up = f.ok(&["up", "--recreate", "--yes", "--tools", &layer]);
    assert!(
        up.contains("devcroft: tools: ") && up.contains("(devenv)"),
        "{up}"
    );
    let with_env = f.session_env();
    let with_policy = f.ok(&["policy", "--render"]);

    // The layer's tool is there, from the layer.
    let rg = f.ok(&["exec", "--", "sh", "-c", "command -v rg"]);
    assert!(rg.contains("ripgrep"), "{rg}");

    // Every variable but PATH is the project's, unchanged.
    let strip = |mut env: BTreeMap<String, String>| {
        env.remove("PATH");
        env
    };
    assert_eq!(strip(with_env.clone()), strip(without_env.clone()));
    // And PATH is the project's, followed by the layer's.
    assert!(
        with_env["PATH"].starts_with(&without_env["PATH"]),
        "the project's PATH must come first:\n  with: {}\n  without: {}",
        with_env["PATH"],
        without_env["PATH"]
    );

    // The policy differs by the layer's read-only store grant alone.
    let added: Vec<&str> = with_policy
        .lines()
        .filter(|l| !without_policy.lines().any(|w| w == *l))
        .collect();
    assert!(
        added.len() == 1 && added[0].contains("/nix/store") && added[0].contains("tools:devenv"),
        "expected exactly the layer's grant to be added, got {added:?}"
    );
    assert!(
        without_policy
            .lines()
            .all(|l| with_policy.lines().any(|w| w == l)),
        "a layer must not remove a rule"
    );

    // `git` is in both, and the project's wins.
    assert_eq!(
        f.ok(&["exec", "--", "sh", "-c", "command -v git"]),
        project_git
    );
    let status = f.ok(&["status"]);
    assert!(
        status.contains("tools: shadowed by the project's PATH: ") && status.contains("git"),
        "{status}"
    );

    assert!(
        !f.layer.join("hook-ran").exists(),
        "the layer's enterShell ran; a layer's hook must never run"
    );
}

/// Task 3.9: the same project and layer, resolved again, give byte-identical
/// sessions and policy.
#[test]
fn resolving_the_same_layer_again_is_byte_identical() {
    let Some(f) = Fixture::new("det") else {
        return;
    };
    let layer = f.layer_arg();
    f.ok(&["up", "--tools", &layer]);
    let (env1, policy1) = (f.session_env(), f.ok(&["policy", "--render"]));
    f.ok(&["up", "--recreate", "--yes", "--tools", &layer]);
    let (env2, policy2) = (f.session_env(), f.ok(&["policy", "--render"]));
    assert_eq!(env1, env2);
    assert_eq!(policy1, policy2);
}

/// Task 3.6: a layer from the configuration is recorded; a change to the
/// configuration, or to the layer's own definition, makes it stale, and a
/// plain `up` says so without applying it.
#[test]
fn a_changed_layer_is_stale_until_recreate() {
    let Some(f) = Fixture::new("stale") else {
        return;
    };
    f.configure(&format!("[tools]\npath = {:?}\n", f.layer_arg()));
    f.ok(&["up"]);
    let status = f.ok(&["status"]);
    assert!(
        status.contains("tools: ") && !status.contains("tools: stale"),
        "{status}"
    );

    // The layer's definition changes: stale, and a plain `up` keeps it.
    devenv_env(&f.layer, "pkgs.ripgrep pkgs.git pkgs.fd", "");
    let up = f.ok(&["up"]);
    assert!(
        up.contains("already up") && up.contains("tools: stale"),
        "{up}"
    );
    assert!(f.ok(&["status"]).contains("tools: stale"));
    assert!(
        !f.run(&["exec", "--", "sh", "-c", "command -v fd"])
            .status
            .success(),
        "a plain `up` must not apply the change"
    );

    // Recreate applies it.
    f.ok(&["up", "--recreate", "--yes"]);
    assert!(!f.ok(&["status"]).contains("tools: stale"));
    f.ok(&["exec", "--", "sh", "-c", "command -v fd"]);

    // The configuration no longer names a layer: stale again.
    f.configure("");
    let status = f.ok(&["status"]);
    assert!(
        status.contains("tools: stale") && status.contains("none is configured"),
        "{status}"
    );
}

/// The `config` spec's "Unknown key in the user configuration": a mistake
/// in the file fails `up` at layer `config`, naming the file and the key,
/// before anything resolves.
#[test]
fn a_bad_user_configuration_fails_up_naming_the_file() {
    let Some(f) = Fixture::new("badcfg") else {
        return;
    };
    f.configure("[tools]\npath = \"/x\"\nprovder = \"nix\"\n");
    let out = f.run(&["up"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains("config.toml") && stderr.contains("tools.provder"),
        "{stderr}"
    );
}

/// Task 5.1: `doctor` tells "none configured", which is not a problem, from
/// "configured but unusable", which is. Needs no devenv: neither case
/// resolves anything.
#[test]
fn doctor_tells_no_layer_from_a_broken_one() {
    let root = std::env::temp_dir().join(format!("dct-doctor-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("devcroft")).unwrap();
    let doctor = || {
        let out = Command::new(devcroft_bin())
            .arg("doctor")
            .current_dir(&root)
            .env("XDG_CONFIG_HOME", &root)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.contains("] tools:"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };

    let none = doctor();
    assert!(
        none.len() == 1 && none[0].starts_with("[INFO] tools: no tooling layer configured"),
        "{none:?}"
    );

    std::fs::write(
        root.join("devcroft/config.toml"),
        "[tools]\npath = \"/nonexistent-devcroft-layer\"\n",
    )
    .unwrap();
    let broken = doctor();
    assert!(
        broken.len() == 1
            && broken[0].starts_with("[FAIL] tools:")
            && broken[0].contains("/nonexistent-devcroft-layer"),
        "{broken:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Every regular file under `dir`, not following symlinks.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            out.extend(files_under(&entry.path()));
        } else if kind.is_file() {
            out.push(entry.path());
        }
    }
    out
}

/// Tasks 4.1, 4.2 and 4.6: a credential the user's configuration requests
/// reaches sessions from the invoking shell; `up` names it once, on a repeat
/// `up` too; and its value is in no file devcroft writes — not `meta.json`,
/// not the compiled policy, not the policy artifact, not the log.
#[test]
fn a_user_credential_reaches_sessions_is_disclosed_and_is_never_written_down() {
    let Some(f) = Fixture::new("cred") else {
        return;
    };
    f.configure(&format!("[credentials]\nenv = [\"{CREDENTIAL}\"]\n"));
    let secret = format!(
        "dct-secret-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let up = |args: &[&str]| {
        let out = f.run_with(CREDENTIAL, Some(&secret), args);
        assert!(out.status.success(), "{out:?}");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let disclosure = format!("devcroft: credential: {CREDENTIAL},");

    let first = up(&["up"]);
    assert_eq!(first.matches(&disclosure).count(), 1, "{first}");
    let again = up(&["up"]);
    assert!(again.contains("already up"), "{again}");
    assert_eq!(again.matches(&disclosure).count(), 1, "{again}");

    let inside = f.ok(&["exec", "--", "printenv", CREDENTIAL]);
    assert_eq!(inside.trim(), secret);
    let render = f.ok(&["policy", "--render"]);
    assert!(!render.contains(&secret));

    let state = devcroft::lifecycle::StatePaths::new(&f.name).unwrap().root;
    let written: Vec<PathBuf> = files_under(&state)
        .into_iter()
        .chain(files_under(&f.project.join(".devcroft")))
        .collect();
    assert!(
        written.iter().any(|p| p.ends_with("meta.json")),
        "the walk must reach the state it is meant to check: {written:?}"
    );
    for file in written {
        let bytes = std::fs::read(&file).unwrap_or_default();
        assert!(
            !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "the credential's value was written to {}",
            file.display()
        );
    }
}

/// `lifecycle` spec, "Tooling and credentials resolve before restriction":
/// a requested credential this shell does not hold fails `up` at layer
/// `config`, naming it, before any state exists; so does one the project
/// also sets as a committed literal.
#[test]
fn a_credential_the_shell_lacks_or_the_project_sets_fails_up() {
    let Some(f) = Fixture::new("credmiss") else {
        return;
    };
    f.configure(&format!("[credentials]\nenv = [\"{CREDENTIAL}\"]\n"));
    let out = f.run(&["up"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains(CREDENTIAL) && stderr.contains("not set"),
        "{stderr}"
    );
    let state = devcroft::lifecycle::StatePaths::new(&f.name).unwrap().root;
    assert!(!state.exists(), "a refused `up` must leave no state behind");

    std::fs::write(
        f.project.join("devcroft.toml"),
        format!(
            "[sandbox]\nname = {:?}\n\n[env]\nprovider = \"devenv\"\n\n[env.vars]\n{CREDENTIAL} = \"committed\"\n",
            f.name
        ),
    )
    .unwrap();
    let out = f.run_with(CREDENTIAL, Some("x"), &["up"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("[env.vars]"), "{stderr}");
}
