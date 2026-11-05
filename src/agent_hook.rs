//! Denial feedback (`add-agent-workload` 6b): an agent whose tool call the
//! sandbox refused learns that it was the sandbox, what it is granted, and
//! what would change it — instead of inferring "the file does not exist" or
//! "this needs sudo" from a bare `EACCES`.
//!
//! **Agent-specific by necessity** (6b.5). devcroft cannot rewrite the error
//! a tool sees: it comes from the kernel, through whatever binary the agent
//! ran. Only the agent's own hook system sits between that error and the
//! model, so the delivery is per agent. Claude Code's `PostToolUseFailure`
//! is the one wired ([`claude_code`]); the decision below is not specific to
//! it.
//!
//! **The policy decides, not the error text.** A path the mount view hides
//! fails with a plain `ENOENT`, exactly like a file that is really missing,
//! so a gate on the message alone either fires on every missing file or
//! misses every hidden one — and a hook that fires on everything teaches the
//! agent to ignore it (6b.6). The text only nominates: an error shaped like
//! a refusal or a missing path, and the paths and hosts it names. Each is
//! then judged against the sandbox's compiled policy
//! ([`crate::policy::why_resolved`], [`crate::policy::why_host`]), and the
//! hook speaks only for what the policy refuses.

use std::path::{Path, PathBuf};

use crate::policy::{CompiledPolicy, Op, why_host, why_resolved};

/// The sandbox a hook runs in, as it sees itself.
pub struct Sandbox<'a> {
    pub policy: &'a CompiledPolicy,
    /// What a `.` rule means: the project root, or a fleet agent's clone.
    pub project_root: &'a Path,
    /// What a `~` rule means: the host's home, from the policy artifact.
    pub host_home: Option<&'a Path>,
    /// What `~` in a command means: the sandbox's own home.
    pub home: &'a Path,
    /// What a relative path in a command means.
    pub cwd: &'a Path,
}

/// One thing the policy refused, ready to be named to the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denial {
    pub subject: String,
    pub detail: String,
}

const REFUSED: [&str; 6] = [
    "permission denied",
    "operation not permitted",
    "read-only file system",
    "eacces",
    "eperm",
    "erofs",
];
const MISSING: [&str; 2] = ["no such file or directory", "enoent"];
const NETWORK: [&str; 11] = [
    "egress denied",
    "connect tunnel failed",
    "could not resolve host",
    "502 bad gateway",
    "connection refused",
    "network is unreachable",
    "name or service not known",
    "temporary failure in name resolution",
    "getaddrinfo",
    "enotfound",
    "econnrefused",
];

/// At most this many denials are named; the rest are in `policy --render`.
const MAX_DENIALS: usize = 5;

/// What the policy refused in a failed tool call, or nothing when the
/// failure is not the sandbox's — the gate.
pub fn denials(
    sb: &Sandbox,
    tool_name: &str,
    tool_input: &serde_json::Value,
    error: &str,
) -> Vec<Denial> {
    let lower = error.to_ascii_lowercase();
    let has = |set: &[&str]| set.iter().any(|s| lower.contains(s));
    let (refused, missing, network) = (has(&REFUSED), has(&MISSING), has(&NETWORK));
    let mut out: Vec<Denial> = Vec::new();
    let mut push = |d: Denial| {
        if !out.contains(&d) && out.len() < MAX_DENIALS {
            out.push(d);
        }
    };

    if refused || missing {
        let writes = matches!(tool_name, "Write" | "Edit" | "MultiEdit" | "NotebookEdit");
        let mut paths = path_tokens(error);
        if paths.is_empty() {
            paths = input_paths(tool_input);
        }
        for token in paths {
            let path = sb.resolve(&token);
            let read = why_resolved(sb.policy, &path, sb.project_root, sb.host_home, Op::Read);
            let write = why_resolved(sb.policy, &path, sb.project_root, sb.host_home, Op::Write);
            let shown = path.display();
            if !read.allowed {
                // Linux hides what is not granted, so it reads as missing.
                let seen = if missing {
                    "; the sandbox does not show it, so it reads as missing whether or not it exists"
                } else {
                    ""
                };
                push(Denial {
                    subject: shown.to_string(),
                    detail: format!("{}{seen}", read.detail),
                });
            } else if !write.allowed && (refused || writes) {
                push(Denial {
                    subject: shown.to_string(),
                    detail: format!("{}, for reading only: writing is not granted", read.detail),
                });
            }
        }
    }

    if network && sb.policy.network_block {
        let mut text = error.to_string();
        if let Some(command) = tool_input.get("command").and_then(|c| c.as_str()) {
            text.push('\n');
            text.push_str(command);
        }
        for host in hosts(&text) {
            let verdict = why_host(sb.policy, &host);
            if !verdict.allowed {
                push(Denial {
                    subject: host,
                    detail: format!(
                        "{}; outbound traffic reaches only the hosts the policy allows",
                        verdict.detail
                    ),
                });
            }
        }
    }
    out
}

impl Sandbox<'_> {
    /// A path as the shell that ran the command would have resolved it.
    fn resolve(&self, token: &str) -> PathBuf {
        if token == "~" {
            self.home.to_path_buf()
        } else if let Some(rest) = token.strip_prefix("~/") {
            self.home.join(rest)
        } else {
            self.cwd.join(token)
        }
    }

    /// What the sandbox may read and write, as absolute paths.
    fn grants(&self) -> (Vec<String>, Vec<String>) {
        let resolve = |rules: &[crate::policy::AnnotatedValue]| -> Vec<String> {
            rules
                .iter()
                .filter(|r| !r.value.starts_with("/dev/") && r.value != "/dev")
                .filter_map(|r| {
                    let p = if r.value == "~" {
                        self.host_home?.to_path_buf()
                    } else if let Some(rest) = r.value.strip_prefix("~/") {
                        self.host_home?.join(rest)
                    } else {
                        self.project_root.join(&r.value)
                    };
                    Some(p.components().collect::<PathBuf>().display().to_string())
                })
                .collect()
        };
        (
            resolve(&self.policy.filesystem_allow),
            resolve(&self.policy.filesystem_read),
        )
    }
}

/// Path-shaped tokens in `text`: absolute, `~`-rooted, or relative with a
/// separator. A bare word is never one, nor is a URL.
fn path_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split(|c: char| c.is_whitespace() || "'\"`:;,()<>=|&[]{}".contains(c)) {
        let token = raw.trim_end_matches(['.', '!', '?']);
        let shaped = (token.starts_with('/') && !token.starts_with("//"))
            || token == "~"
            || token.starts_with("~/")
            || (token.contains('/') && !token.starts_with("//") && !token.contains("://"));
        if shaped && !out.iter().any(|t| t == token) {
            out.push(token.to_string());
        }
    }
    out
}

/// The paths a tool was asked to touch: its path fields, else the
/// path-shaped tokens of its command.
fn input_paths(input: &serde_json::Value) -> Vec<String> {
    let fields: Vec<String> = ["file_path", "path", "notebook_path"]
        .iter()
        .filter_map(|k| input.get(k)?.as_str().map(str::to_owned))
        .collect();
    if !fields.is_empty() {
        return fields;
    }
    input
        .get("command")
        .and_then(|c| c.as_str())
        .map(path_tokens)
        .unwrap_or_default()
}

/// Hosts in `text`: each `scheme://host[:port]`, and the host the egress
/// proxy names when it refuses one.
fn hosts(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |h: &str| {
        let h = h
            .trim_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
            .to_ascii_lowercase();
        if h.contains('.') && !out.contains(&h) {
            out.push(h);
        }
    };
    for (i, _) in text.match_indices("://") {
        let rest = &text[i + 3..];
        let end = rest
            .find(|c: char| "/:?#'\" \n\t".contains(c))
            .unwrap_or(rest.len());
        let authority = &rest[..end];
        add(authority.rsplit('@').next().unwrap_or(authority));
    }
    for (i, m) in text.match_indices("egress denied for ") {
        let rest = &text[i + m.len()..];
        add(rest.split([':', ' ', '\n']).next().unwrap_or(""));
    }
    out
}

/// What the agent is told: what was refused, that it was the sandbox, what
/// the sandbox grants, and what changing it takes. Stated as facts — Claude
/// Code's documentation advises against imperative, system-styled text in
/// injected context, which reads as a prompt injection.
pub fn context(sb: &Sandbox, denials: &[Denial]) -> String {
    let mut s = String::from(
        "This tool call ran inside a devcroft sandbox, and the sandbox's policy refused it:\n",
    );
    for d in denials {
        s.push_str(&format!("- {}: {}\n", d.subject, d.detail));
    }
    s.push_str(
        "This is the sandbox's policy, not a missing file, a file permission or a need for \
         sudo: retrying, sudo, or another route to the same place fails the same way.\n",
    );
    let (rw, ro) = sb.grants();
    let list = |v: &[String]| -> String {
        const SHOWN: usize = 12;
        let mut l = v.iter().take(SHOWN).cloned().collect::<Vec<_>>().join(", ");
        if v.len() > SHOWN {
            l.push_str(&format!(", and {} more", v.len() - SHOWN));
        }
        if l.is_empty() { "nothing".into() } else { l }
    };
    s.push_str(&format!("Granted read and write: {}.\n", list(&rw)));
    s.push_str(&format!("Granted read only: {}.\n", list(&ro)));
    let network = if !sb.policy.network_block {
        "any host".to_string()
    } else if sb.policy.network_allow_domain.is_empty() {
        "no host".to_string()
    } else {
        sb.policy
            .network_allow_domain
            .iter()
            .map(|d| d.value.as_str())
            .collect::<Vec<_>>()
            .join(", ")
            + " only"
    };
    s.push_str(&format!("Network: {network}.\n"));
    s.push_str(
        "`$HOME/../devcroft why --path <path> --op <read|write>` and `--host <domain>` answer \
         for one path or host; `$HOME/../devcroft policy --render` lists every rule and where \
         it came from. The policy is the project's devcroft.toml ([filesystem] allow and read, \
         [network] allow), and a change to it takes effect when the user runs \
         `devcroft up --recreate` on the host; nothing inside the sandbox can change it. If the \
         task can be done within these grants, that avoids the change; if it cannot, the user \
         is the one to ask, naming the grant the task needs and why.",
    );
    s
}

/// Claude Code's `PostToolUseFailure` hook: its stdin JSON in, the
/// `hookSpecificOutput` to print out, or `None` to stay silent — which is
/// what every failure the policy did not cause gets.
pub fn claude_code(sb: &Sandbox, input: &serde_json::Value) -> Option<serde_json::Value> {
    if input.get("hook_event_name").and_then(|e| e.as_str()) != Some("PostToolUseFailure")
        || input.get("is_interrupt").and_then(|i| i.as_bool()) == Some(true)
    {
        return None;
    }
    let tool = input
        .get("tool_name")
        .and_then(|t| t.as_str())
        .unwrap_or("");
    let error = input.get("error").and_then(|e| e.as_str()).unwrap_or("");
    let empty = serde_json::Value::Null;
    let found = denials(sb, tool, input.get("tool_input").unwrap_or(&empty), error);
    (!found.is_empty()).then(|| {
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PostToolUseFailure",
                "additionalContext": context(sb, &found),
            }
        })
    })
}

/// Where `devcroft init --agent claude-code` writes the hook, relative to
/// the project root. Committed with the project (6b.4).
pub const CLAUDE_CODE_SCRIPT: &str = ".claude/hooks/devcroft-denial.sh";

/// The hook script. POSIX, and run as `sh <script>` rather than through a
/// shebang: inside a sandbox `/bin/sh` is not in the view, while `sh` is on
/// `PATH` (devcroft's own shell, `src/shell.rs`), and Claude Code's exec
/// form resolves its command on `PATH`. The binary is reached through the
/// link beside the sandbox's home (`services::EXE_LINK`), since its own
/// path differs per machine.
pub const CLAUDE_CODE_SCRIPT_BODY: &str = r#"# Written by `devcroft init --agent claude-code`; commit it with the project.
#
# Inside a devcroft sandbox, when a tool call fails because the sandbox's
# policy refused it, this tells Claude Code so, with what the sandbox grants.
# Every other failure, and every failure outside a sandbox, it leaves alone.
case "$HOME" in
*/.devcroft/*/home) ;;
*) exit 0 ;;
esac
exe="$HOME/../devcroft"
[ -x "$exe" ] || exit 0
exec "$exe" agent hook claude-code
"#;

/// `.claude/settings.json` with the hook added: `existing` is the file's
/// text, if there is one. `Ok(None)` when it already runs the hook. Every
/// other key, and the order of keys, is kept. Refused, never repaired, when
/// the file is not a JSON object: it is the user's.
pub fn claude_code_settings(existing: Option<&str>) -> Result<Option<String>, String> {
    let mut settings: serde_json::Value = match existing {
        Some(text) => serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))?,
        None => serde_json::json!({}),
    };
    let present = settings
        .pointer("/hooks/PostToolUseFailure")
        .and_then(|v| v.as_array())
        .is_some_and(|groups| {
            groups.iter().any(|g| {
                g.get("hooks")
                    .and_then(|h| h.as_array())
                    .is_some_and(|hooks| {
                        hooks
                            .iter()
                            .any(|h| h.to_string().contains(CLAUDE_CODE_SCRIPT))
                    })
            })
        });
    if present {
        return Ok(None);
    }
    let entry = serde_json::json!({
        "hooks": [{
            "type": "command",
            "command": "sh",
            "args": [format!("${{CLAUDE_PROJECT_DIR}}/{CLAUDE_CODE_SCRIPT}")],
        }]
    });
    let object = settings.as_object_mut().ok_or("not a JSON object")?;
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or("`hooks` is not an object")?;
    hooks
        .entry("PostToolUseFailure")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or("`hooks.PostToolUseFailure` is not an array")?
        .push(entry);
    Ok(Some(
        serde_json::to_string_pretty(&settings).expect("a Value always serializes") + "\n",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_are_merged_once_and_keep_what_was_there() {
        let fresh = claude_code_settings(None).unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(&fresh).unwrap();
        assert_eq!(
            v["hooks"]["PostToolUseFailure"][0]["hooks"][0]["command"],
            "sh"
        );
        assert_eq!(
            claude_code_settings(Some(&fresh)).unwrap(),
            None,
            "idempotent"
        );

        let theirs = r#"{"permissions":{"allow":["Bash(ls)"]},"hooks":{"PostToolUseFailure":[{"matcher":"Bash","hooks":[{"type":"command","command":"log"}]}]},"model":"x"}"#;
        let merged = claude_code_settings(Some(theirs)).unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(v["permissions"]["allow"][0], "Bash(ls)");
        assert_eq!(
            v["hooks"]["PostToolUseFailure"][0]["hooks"][0]["command"],
            "log"
        );
        assert_eq!(
            v["hooks"]["PostToolUseFailure"].as_array().unwrap().len(),
            2
        );
        let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["permissions", "hooks", "model"], "order kept");

        assert!(claude_code_settings(Some("[1]")).is_err());
        assert!(claude_code_settings(Some("{not json")).is_err());
    }

    fn policy(toml: &str) -> CompiledPolicy {
        let (manifest, _) = crate::config::parse(toml).unwrap();
        crate::policy::compile(&manifest).with_provider_grants("nix", &["/nix/store".to_string()])
    }

    fn sb(policy: &CompiledPolicy) -> Sandbox<'_> {
        Sandbox {
            policy,
            project_root: Path::new("/work/p"),
            host_home: Some(Path::new("/home/u")),
            home: Path::new("/work/p/.devcroft/p/home"),
            cwd: Path::new("/work/p"),
        }
    }

    const DENY: &str = "[sandbox]\nname = \"p\"\n\n[network]\ndefault = \"deny\"\nallow = [\"api.anthropic.com\"]\n";

    fn bash(command: &str, error: &str) -> serde_json::Value {
        json!({
            "hook_event_name": "PostToolUseFailure",
            "tool_name": "Bash",
            "tool_input": { "command": command },
            "error": error,
            "is_interrupt": false,
        })
    }

    #[test]
    fn a_hidden_path_fires_and_a_missing_one_does_not() {
        let p = policy(DENY);
        let hidden = bash(
            "cat /etc/hostname",
            "Exit code 1\ncat: /etc/hostname: No such file or directory",
        );
        let out = claude_code(&sb(&p), &hidden).expect("a hidden path is the sandbox's");
        let ctx = out["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(
            ctx.contains("/etc/hostname: denied: not granted by any rule"),
            "{ctx}"
        );
        assert!(ctx.contains("reads as missing"), "{ctx}");
        assert!(ctx.contains("/work/p"), "the grants, resolved: {ctx}");
        assert!(ctx.contains("api.anthropic.com only"), "{ctx}");

        for missing in [
            bash(
                "cat nope.txt",
                "Exit code 1\ncat: nope.txt: No such file or directory",
            ),
            bash(
                "cat src/nope.rs",
                "Exit code 1\ncat: src/nope.rs: No such file or directory",
            ),
            bash(
                "ls ~/x",
                "Exit code 2\nls: cannot access '/work/p/.devcroft/p/home/x': No such file or directory",
            ),
            bash(
                "cat /nix/store/abc-x/y",
                "Exit code 1\ncat: /nix/store/abc-x/y: No such file or directory",
            ),
        ] {
            assert_eq!(claude_code(&sb(&p), &missing), None, "{missing}");
        }
    }

    #[test]
    fn an_ordinary_failure_never_fires() {
        let p = policy(DENY);
        for input in [
            bash(
                "cargo test",
                "Exit code 101\ntest result: FAILED. 3 passed; 1 failed",
            ),
            bash("gcc x.c", "Exit code 127\nsh: gcc: command not found"),
            bash("cat /etc/hostname", "Exit code 1\nsomething else entirely"),
        ] {
            assert_eq!(claude_code(&sb(&p), &input), None, "{input}");
        }
        let interrupted = json!({
            "hook_event_name": "PostToolUseFailure", "tool_name": "Bash", "is_interrupt": true,
            "tool_input": {"command": "cat /etc/x"}, "error": "/etc/x: Permission denied",
        });
        assert_eq!(claude_code(&sb(&p), &interrupted), None);
    }

    #[test]
    fn a_read_only_grant_is_named_as_one() {
        let p = policy(DENY);
        let write = json!({
            "hook_event_name": "PostToolUseFailure",
            "tool_name": "Write",
            "tool_input": { "file_path": "/nix/store/abc-x/y", "content": "" },
            "error": "EACCES: permission denied, open '/nix/store/abc-x/y'",
        });
        let out = claude_code(&sb(&p), &write).unwrap();
        let ctx = out["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(ctx.contains("for reading only"), "{ctx}");
        // A permission error on a path the sandbox may write is not its own.
        let own = bash(
            "cat locked",
            "Exit code 1\ncat: ./locked: Permission denied",
        );
        assert_eq!(claude_code(&sb(&p), &own), None);
    }

    #[test]
    fn a_refused_host_fires_and_an_allowed_one_does_not() {
        let p = policy(DENY);
        let refused = bash(
            "curl -sf https://example.com/x",
            "Exit code 56\ncurl: (56) CONNECT tunnel failed, response 502",
        );
        let out = claude_code(&sb(&p), &refused).unwrap();
        let ctx = out["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(ctx.contains("- example.com: denied"), "{ctx}");
        let named = bash(
            "wget http://x",
            "devcroft: egress denied for registry.npmjs.org: not allowed",
        );
        assert!(claude_code(&sb(&p), &named).is_some());
        let allowed = bash(
            "curl https://api.anthropic.com/v1",
            "Exit code 7\ncurl: (7) Connection refused",
        );
        assert_eq!(claude_code(&sb(&p), &allowed), None);
        // Nothing to refuse where the network is open.
        let open = policy("[sandbox]\nname = \"p\"\n\n[network]\ndefault = \"allow\"\n");
        assert_eq!(claude_code(&sb(&open), &refused), None);
    }

    #[test]
    fn a_user_grant_under_the_host_home_counts() {
        let p = policy("[sandbox]\nname = \"p\"\n\n[filesystem]\nread = [\"~/.cargo\"]\n");
        let granted = bash(
            "cat /home/u/.cargo/registry/x",
            "Exit code 1\ncat: /home/u/.cargo/registry/x: No such file or directory",
        );
        assert_eq!(claude_code(&sb(&p), &granted), None);
        let ssh = bash(
            "cat /home/u/.ssh/id",
            "cat: /home/u/.ssh/id: No such file or directory",
        );
        let out = claude_code(&sb(&p), &ssh).unwrap();
        let ctx = out["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(ctx.contains("denied by rule baseline"), "{ctx}");
    }

    #[test]
    fn tokens_and_hosts() {
        assert_eq!(
            path_tokens("ls: cannot access '/a/b': No such file; see ./c and x/y. not a word"),
            vec!["/a/b", "./c", "x/y"]
        );
        assert!(path_tokens("https://example.com/x //net").is_empty());
        assert_eq!(
            hosts("git clone https://user@github.com:443/a/b and http://Example.ORG"),
            vec!["github.com", "example.org"]
        );
    }
}
