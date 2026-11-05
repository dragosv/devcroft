//! `add-agent-workload` 6b.3–6b.6: the denial hook `devcroft init --agent
//! claude-code` writes, run inside a real devenv sandbox the way Claude Code
//! runs it (`sh <script>`, its JSON on stdin), against errors the sandbox
//! itself produced — not hand-written ones, since the gate is only as good
//! as its reading of what a refusal really looks like.
//!
//! Claude Code itself is not run: that needs a login this host does not
//! have. What it would hand the hook is its documented `PostToolUseFailure`
//! input, which is what the test builds.
//!
//! Guards on the capability, never on the binary (CLAUDE.md).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn devcroft_bin() -> &'static str {
    env!("CARGO_BIN_EXE_devcroft")
}

fn capable_host() -> bool {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return false;
    }
    let devenv = Command::new("devenv")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !devenv || !devcroft::provider::host_can_build_nix_closures() {
        eprintln!("skipping: devenv or a usable nix store not on this host");
        return false;
    }
    true
}

struct Sandbox {
    root: PathBuf,
    name: String,
}

impl Sandbox {
    /// A devenv project with a shell and nothing else, deny-by-default, its
    /// network limited to one host.
    fn new(tag: &str) -> Self {
        unsafe {
            std::env::set_var("DEVCROFT_KEEPER_EXE", devcroft_bin());
        }
        let root = PathBuf::from(format!("/tmp/dcd-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/devenv-claude-sample");
        std::fs::copy(sample.join("devenv.lock"), root.join("devenv.lock")).unwrap();
        std::fs::write(
            root.join("devenv.yaml"),
            "inputs:\n  nixpkgs:\n    url: github:cachix/devenv-nixpkgs/rolling\n",
        )
        .unwrap();
        std::fs::write(
            root.join("devenv.nix"),
            "{ pkgs, ... }: { packages = [ pkgs.bash pkgs.coreutils ]; }\n",
        )
        .unwrap();
        let name = format!("dcden{tag}{}", std::process::id());
        std::fs::write(
            root.join("devcroft.toml"),
            format!(
                "[sandbox]\nname = {name:?}\n\n[env]\nprovider = \"devenv\"\n\n\
                 [network]\ndefault = \"deny\"\nallow = [\"api.anthropic.com\"]\n"
            ),
        )
        .unwrap();
        Sandbox { root, name }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(devcroft_bin())
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .output()
            .unwrap()
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

    /// `script` under the sandbox's shell, inside it: (exit code, output).
    fn inside(&self, script: &str) -> (i32, String) {
        let out = self.run(&["exec", "--", "sh", "-c", script]);
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned()
                + &String::from_utf8_lossy(&out.stderr),
        )
    }

    /// The hook's answer, inside, to a Bash call of `command` failing as it
    /// really fails there. Empty when the hook stays silent.
    fn hook_on(&self, command: &str) -> String {
        let (code, output) = self.inside(&format!("{command} 2>&1"));
        assert_ne!(code, 0, "`{command}` was meant to fail inside:\n{output}");
        let input = serde_json::json!({
            "session_id": "test",
            "cwd": self.root,
            "hook_event_name": "PostToolUseFailure",
            "tool_name": "Bash",
            "tool_input": { "command": command },
            "error": format!("Exit code {code}\n{output}"),
            "is_interrupt": false,
        });
        std::fs::write(self.root.join("hook-input.json"), input.to_string()).unwrap();
        let (code, out) = self.inside(&format!(
            "sh {} < hook-input.json",
            devcroft::agent_hook::CLAUDE_CODE_SCRIPT
        ));
        assert_eq!(code, 0, "the hook itself failed:\n{out}");
        out
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = self.run(&["rm", "--yes"]);
        if let Ok(paths) = devcroft::lifecycle::StatePaths::new(&self.name) {
            let _ = std::fs::remove_dir_all(paths.root);
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn context(hook_output: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(hook_output.trim())
        .unwrap_or_else(|e| panic!("not the hook's JSON ({e}):\n{hook_output}"));
    assert_eq!(
        v["hookSpecificOutput"]["hookEventName"],
        "PostToolUseFailure"
    );
    v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .to_string()
}

/// 6b.3 and 6b.6 together, since each is meaningless without the other: a
/// path the sandbox hides fails exactly like a missing file, and only the
/// hidden one is explained.
#[test]
fn a_refusal_is_explained_and_an_ordinary_failure_is_not() {
    if !capable_host() {
        return;
    }
    let s = Sandbox::new("gate");
    s.ok(&["init", "--agent", "claude-code"]);
    s.ok(&["up"]);

    // Hidden by the view: ENOENT, like a missing file.
    let ctx = context(&s.hook_on("cat /etc/hostname"));
    assert!(
        ctx.contains("/etc/hostname: denied: not granted by any rule"),
        "{ctx}"
    );
    assert!(ctx.contains("reads as missing"), "{ctx}");
    assert!(
        ctx.contains(&s.root.display().to_string()),
        "the grants: {ctx}"
    );
    assert!(ctx.contains("api.anthropic.com only"), "{ctx}");

    // A read-only grant, written to.
    let store = s.inside("command -v cat").1;
    let store_dir = Path::new(store.trim())
        .parent()
        .unwrap()
        .display()
        .to_string();
    let ctx = context(&s.hook_on(&format!("touch {store_dir}/x")));
    assert!(ctx.contains("for reading only"), "{ctx}");

    // 6b.6: genuinely missing, inside the grants, and other failures.
    for ordinary in [
        "cat nope.txt",
        "cat src/nope.rs",
        "ls ~/nope",
        "cat /tmp/nope-devcroft",
        "false",
        "sh -c 'exit 3'",
    ] {
        assert_eq!(s.hook_on(ordinary), "", "fired on `{ordinary}`");
    }
    // A permission error the sandbox did not cause.
    std::fs::write(s.root.join("locked"), "x").unwrap();
    s.inside("chmod 000 locked");
    assert_eq!(s.hook_on("cat locked"), "", "fired on a mode-000 file");
}

/// The same hook outside any sandbox does nothing, so committing it costs a
/// user who runs Claude Code on the host nothing.
#[test]
fn outside_a_sandbox_the_hook_is_silent() {
    let dir = std::env::temp_dir().join(format!("dcd-host-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("devcroft.toml"), "[sandbox]\nname = \"dcdhost\"\n").unwrap();
    let init = Command::new(devcroft_bin())
        .args(["init", "--agent", "claude-code"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(init.status.success(), "{init:?}");
    let input = r#"{"hook_event_name":"PostToolUseFailure","tool_name":"Bash","tool_input":{"command":"cat /etc/shadow"},"error":"cat: /etc/shadow: Permission denied"}"#;
    let mut child = Command::new("sh")
        .arg(dir.join(devcroft::agent_hook::CLAUDE_CODE_SCRIPT))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut child.stdin.take().unwrap(), input.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 6b.4: written once, merged into what the user has, and refused rather
/// than repaired where that is not JSON.
#[test]
fn init_writes_the_hook_once_and_keeps_the_users_settings() {
    let dir = std::env::temp_dir().join(format!("dcd-init-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".claude")).unwrap();
    std::fs::write(dir.join("devcroft.toml"), "[sandbox]\nname = \"dcdinit\"\n").unwrap();
    let settings = dir.join(".claude/settings.json");
    std::fs::write(&settings, r#"{"model": "x"}"#).unwrap();
    let init = |args: &[&str]| {
        Command::new(devcroft_bin())
            .args(args)
            .current_dir(&dir)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };

    let out = init(&["init", "--agent", "claude-code"]);
    assert!(out.status.success(), "{out:?}");
    let manifest = std::fs::read_to_string(dir.join("devcroft.toml")).unwrap();
    assert_eq!(
        manifest, "[sandbox]\nname = \"dcdinit\"\n",
        "the manifest is not touched"
    );
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(written["model"], "x");
    let hook = &written["hooks"]["PostToolUseFailure"][0]["hooks"][0];
    assert_eq!(hook["command"], "sh");
    assert_eq!(
        hook["args"][0],
        "${CLAUDE_PROJECT_DIR}/.claude/hooks/devcroft-denial.sh"
    );
    assert!(dir.join(devcroft::agent_hook::CLAUDE_CODE_SCRIPT).is_file());

    let before = std::fs::read_to_string(&settings).unwrap();
    assert!(init(&["init", "--agent", "claude-code"]).status.success());
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), before, "once");

    std::fs::write(&settings, "{ not json").unwrap();
    let out = init(&["init", "--agent", "claude-code"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{ not json");

    let out = init(&["init", "--agent", "aider"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not yet supported"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The `why` the hook points the agent at, inside, for absolute paths: the
/// project, and the sandbox's own home in it, were `DENIED` before.
#[test]
fn why_inside_answers_for_absolute_paths() {
    if !capable_host() {
        return;
    }
    let s = Sandbox::new("why");
    s.ok(&["up"]);
    let (code, out) = s.inside("$HOME/../devcroft why --path \"$PWD/src/x\" --op write");
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("ALLOWED"), "{out}");
    let (_, out) = s.inside("$HOME/../devcroft why --path \"$HOME/.ssh/x\" --op write");
    assert!(
        out.starts_with("ALLOWED"),
        "the sandbox's home is not the host's:\n{out}"
    );
    let (_, out) = s.inside("$HOME/../devcroft why --path /etc/hostname --op read");
    assert!(out.starts_with("DENIED"), "{out}");
}
