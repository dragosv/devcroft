//! Why the activation script failed in the provisioning process: the
//! provisioning policy, or the script itself (`sandbox-provisioning` P5 and
//! task group 3; env-provider spec, "Provisioning failures name their
//! cause").
//!
//! **Nothing in the boundary reports a denial.** Landlock returns an errno
//! and the mount view simply lacks the path, so the only evidence is what the
//! script printed. Measured (2026-10-18), a confined hook meets three
//! spellings, none of which names the sandbox:
//!
//! - `No such file or directory` for a path the mount view leaves out (the
//!   real home, `/etc/hostname`, `/var/lib`): there on the host, absent in the
//!   view;
//! - `Permission denied` for a create in a view directory nothing grants
//!   (`/opt`, `/etc`);
//! - `Read-only file system` for a write into a read-only grant (`/usr`,
//!   `/nix/store`).
//!
//! **So a line is attributed only when devcroft's own policy agrees** (`why`
//! is the verdict, not an explanation of someone else's): a path some rule
//! grants is the host's own permission, and a missing path is the script's
//! own bug unless the host has it. What this can miss is a tool that
//! swallows the errno and prints something of its own; what it cannot do is
//! blame the policy for a path the policy grants.
//!
//! Hosts are not inferred: a refusal by the provisioning proxy is in its
//! audit log. The one network inference is for a provisioning context with no
//! allowlist, where there is no proxy and no route, so any network failure
//! the script reports is necessarily the policy's.

use std::path::Path;

use crate::paths::is_within;
use crate::policy::{CompiledPolicy, Origin};
use crate::policy::{Op, why_path};

/// Where the policy's relative spellings are rooted: a grant is written
/// absolute, `~/…` or project-relative (the default is `.`), and
/// `is_within` never matches across the three, while a script's errors
/// always print absolute paths.
pub(crate) struct Roots<'a> {
    pub project: &'a Path,
    pub home: Option<&'a Path>,
}

impl Roots<'_> {
    /// Every spelling of `path` a rule could be written in.
    fn spellings(&self, path: &str) -> Vec<String> {
        let mut out = vec![path.to_owned()];
        let p = Path::new(path);
        if let Ok(rest) = p.strip_prefix(self.project) {
            out.push(match rest.to_str() {
                Some("") => ".".to_owned(),
                Some(rest) => rest.to_owned(),
                None => return out,
            });
        }
        if let Some(rest) = self.home.and_then(|h| p.strip_prefix(h).ok()) {
            out.push(match rest.to_str() {
                Some("") => "~".to_owned(),
                Some(rest) => format!("~/{rest}"),
                None => return out,
            });
        }
        out
    }

    /// `why_path` across every spelling: a deny rule in any wins, as
    /// baseline denials do; else any grant allows. `None` when allowed,
    /// else the denying rule (`Some(None)` when nothing grants it).
    fn denied(&self, policy: &CompiledPolicy, path: &str, op: Op) -> Option<Option<Origin>> {
        let verdicts: Vec<_> = self
            .spellings(path)
            .iter()
            .map(|s| why_path(policy, s, op))
            .collect();
        if let Some(e) = verdicts.iter().find(|e| !e.allowed && e.origin.is_some()) {
            return Some(e.origin.clone());
        }
        if verdicts.iter().any(|e| e.allowed) {
            return None;
        }
        Some(None)
    }

    /// Whether some grant lies at or under `dir`, in any spelling: then the
    /// mount view has `dir`, since that is how the grant is reached.
    fn leads_to_a_grant(&self, policy: &CompiledPolicy, dir: &str) -> bool {
        let spellings = self.spellings(dir);
        policy
            .filesystem_allow
            .iter()
            .chain(&policy.filesystem_read)
            .any(|g| spellings.iter().any(|s| is_within(&g.value, s)))
    }
}

/// Something the provisioning policy refused the script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Denial {
    Path {
        path: String,
        write: bool,
        origin: Option<Origin>,
        /// The error the script printed for it, as it spelled it.
        seen: &'static str,
    },
    Host {
        host: String,
        port: String,
        reason: String,
    },
    /// No allowlist, so no network at all; `seen` is the line that says so.
    NoNetwork { seen: String },
}

#[derive(Clone, Copy)]
enum Errno {
    Missing,
    Access,
    ReadOnly,
}

const ERRNOS: [(&str, &str, Errno); 4] = [
    (
        "no such file or directory",
        "No such file or directory",
        Errno::Missing,
    ),
    ("permission denied", "Permission denied", Errno::Access),
    (
        "operation not permitted",
        "Operation not permitted",
        Errno::Access,
    ),
    (
        "read-only file system",
        "Read-only file system",
        Errno::ReadOnly,
    ),
];

/// How a resolver or a connect with no route reports itself: curl, glibc's
/// `gai_strerror`, Node and Go.
const NETWORK_FAILURES: [&str; 9] = [
    "could not resolve host",
    "temporary failure in name resolution",
    "name or service not known",
    "network is unreachable",
    "no route to host",
    "enotfound",
    "eai_again",
    "enetunreach",
    "server misbehaving",
];

/// The activation script's own output in a provisioning log: what follows
/// its `[hook activation] $ …` record, without the record (whose command text
/// is the script, not its output) or devcroft's own lines.
pub(crate) fn script_output(log: &str) -> String {
    let Some(start) = log.rfind("[hook activation] $ ") else {
        return String::new();
    };
    let rest = &log[start..];
    // The record is the wrapped script, which ends in this line
    // (`hooks::ActivationCapture::wrap`); unwrapped, it is one line.
    let body = match rest.find("\nexit $__devcroft_rc\n") {
        Some(i) => &rest[i + "\nexit $__devcroft_rc\n".len()..],
        None => rest.split_once('\n').map_or("", |(_, b)| b),
    };
    body.lines()
        .filter(|l| {
            !l.starts_with(super::hooks::KEEPER_HOOK_FAILURE_PREFIX)
                && !l.starts_with("[hook activation] note:")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What `policy` refused the script, from its `output` and the provisioning
/// proxy's `egress` log (`None` when no proxy ran: there is no allowlist, and
/// a log on disk is an earlier run's). `on_host` says whether a path exists
/// outside the sandbox; injected so the rule can be tested without one.
pub(crate) fn denials(
    output: &str,
    egress: Option<&str>,
    policy: &CompiledPolicy,
    roots: &Roots<'_>,
    on_host: &dyn Fn(&Path) -> bool,
) -> Vec<Denial> {
    let mut found: Vec<Denial> = Vec::new();
    let mut push = |d: Denial| {
        let key = |d: &Denial| match d {
            Denial::Path { path, .. } => path.clone(),
            Denial::Host { host, port, .. } => format!("{host}:{port}"),
            Denial::NoNetwork { .. } => String::new(),
        };
        if !found.iter().any(|f| key(f) == key(&d)) {
            found.push(d);
        }
    };

    for line in output.lines() {
        let lower = line.to_ascii_lowercase();
        for (needle, seen, errno) in ERRNOS {
            let Some(at) = lower.find(needle) else {
                continue;
            };
            // The path an error is about precedes it (`cat: /x: …`, `mkdir:
            // cannot create directory '/x': …`), after the program's own
            // path when that is printed too (`/nix/store/…/bash: line 3:
            // /x: …`); Python and Node put it after (`Permission denied:
            // '/x'`, `permission denied, open '/x'`).
            let Some(path) = paths_in(&line[..at])
                .pop()
                .or_else(|| paths_in(&line[at + needle.len()..]).into_iter().next())
            else {
                continue;
            };
            if let Some(d) = path_denial(path, errno, seen, policy, roots, on_host) {
                push(d);
            }
        }
    }

    match egress {
        Some(egress) => {
            for line in egress.lines().filter(|l| l.contains("refuse ")) {
                let field = |k: &str| {
                    line.split_whitespace()
                        .find_map(|w| w.strip_prefix(k))
                        .map(str::to_owned)
                };
                let (Some(host), Some(port)) = (field("host="), field("port=")) else {
                    continue;
                };
                let reason = line
                    .split_once(" reason=")
                    .map_or(String::new(), |(_, r)| r.to_owned());
                push(Denial::Host { host, port, reason });
            }
        }
        None => {
            if let Some(seen) = output.lines().find(|l| {
                let lower = l.to_ascii_lowercase();
                NETWORK_FAILURES.iter().any(|n| lower.contains(n))
            }) {
                push(Denial::NoNetwork {
                    seen: seen.trim().to_owned(),
                });
            }
        }
    }
    found
}

fn path_denial(
    path: &str,
    errno: Errno,
    seen: &'static str,
    policy: &CompiledPolicy,
    roots: &Roots<'_>,
    on_host: &dyn Fn(&Path) -> bool,
) -> Option<Denial> {
    let denied = |p: &str, op| roots.denied(policy, p, op);
    let (write, origin) = match errno {
        // Absent from the view, not from the host: the path itself, or the
        // directory a create went into. A directory some grant lies under is
        // in the view (that is how the grant is reached), so a create
        // missing it is missing on the host too: `/` above all.
        Errno::Missing => {
            let p = Path::new(path);
            let target = if on_host(p) {
                path
            } else {
                let parent = p.parent()?.to_str()?;
                if roots.leads_to_a_grant(policy, parent) || !on_host(Path::new(parent)) {
                    return None;
                }
                parent
            };
            (false, denied(target, Op::Read)?)
        }
        Errno::Access => match denied(path, Op::Read) {
            Some(origin) => (false, origin),
            None => (true, denied(path, Op::Write)?),
        },
        Errno::ReadOnly => (true, denied(path, Op::Write)?),
    };
    Some(Denial::Path {
        path: path.to_owned(),
        write,
        origin,
        seen,
    })
}

/// Absolute paths in `s`, in order: a `/` at the start or after whitespace,
/// a quote or a bracket, up to the next of those or a `:`/`,`. `https://`
/// is not one (its `/` follows a `:`), nor is a bare `/`.
fn paths_in(s: &str) -> Vec<&str> {
    const OPEN: &[char] = &[' ', '\t', '\'', '"', '`', '(', '[', '=', '‘', '“'];
    const CLOSE: &[char] = &[' ', '\t', '\'', '"', '`', ')', ']', ':', ',', '’', '”'];
    let mut out = Vec::new();
    let mut prev: Option<char> = None;
    for (i, c) in s.char_indices() {
        if c == '/' && prev.is_none_or(|p| OPEN.contains(&p)) {
            let end = s[i..].find(CLOSE).map_or(s.len(), |n| i + n);
            let path = s[i..end].trim_end_matches('.');
            if path.len() > 1 {
                out.push(path);
            }
        }
        prev = Some(c);
    }
    out
}

/// `up`'s message for a failed activation script: `failure` (the hook's own
/// error, `hook \`activation\` failed: …`), then either what the policy
/// refused it or, when nothing, the end of what it printed.
pub(crate) fn explain(failure: &str, denials: &[Denial], output: &str, sandbox: &str) -> String {
    const SHOWN: usize = 5;
    if denials.is_empty() {
        let tail: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
        let tail = &tail[tail.len().saturating_sub(3)..];
        let mut msg = format!(
            "{failure} in provisioning; its output names nothing the provisioning \
             policy refused, so the failure is the script's own"
        );
        if !tail.is_empty() {
            msg.push_str(". It ended with:");
            for line in tail {
                msg.push_str(&format!("\n  | {line}"));
            }
        }
        msg.push_str(&format!("\n(`devcroft logs {sandbox}` has all of it)"));
        return msg;
    }
    let mut msg = format!("{failure}: the provisioning policy refused it");
    for d in denials.iter().take(SHOWN) {
        msg.push_str("\n  - ");
        msg.push_str(&match d {
            Denial::Path {
                path,
                write,
                origin,
                seen,
            } => {
                let by = match origin {
                    Some(origin) => format!("denied by rule {origin}"),
                    None => "not granted by any rule".to_owned(),
                };
                let what = if *write { " (write)" } else { "" };
                let hidden = if *seen == "No such file or directory" {
                    ", so the sandbox does not show it,"
                } else {
                    ""
                };
                format!("{path}{what}: {by}{hidden} (the script saw \"{seen}\")")
            }
            Denial::Host { host, port, reason } => format!(
                "{host}:{port}: refused by the provisioning proxy ({reason}); \
                 not in [provisioning.network] allow"
            ),
            Denial::NoNetwork { seen } => format!(
                "the network: provisioning has none, since [provisioning.network] \
                 allow is empty (the script saw \"{seen}\")"
            ),
        });
    }
    if denials.len() > SHOWN {
        msg.push_str(&format!("\n  - and {} more", denials.len() - SHOWN));
    }
    msg.push_str(&format!(
        "\ngrant what it needs in devcroft.toml ([filesystem], or [provisioning.network] \
         allow for a host); `devcroft why --provisioning` explains each; \
         `devcroft logs {sandbox}` has the script's output"
    ));
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A default manifest (the project, `.`) with the store and `/usr` as a
    /// provider row grants them; the baseline denies `~/.ssh`.
    fn policy() -> CompiledPolicy {
        let (manifest, _) = crate::config::parse("[sandbox]\nname = \"t\"\n").unwrap();
        crate::policy::compile(&manifest)
            .with_provider_grants("nix", &["/nix/store".to_owned(), "/usr".to_owned()])
            .for_provisioning(&[])
    }

    fn roots() -> Roots<'static> {
        Roots {
            project: Path::new("/tmp/proj"),
            home: Some(Path::new("/home/u")),
        }
    }

    /// The host this measured on: a real home, `/etc`, `/opt`.
    fn on_host(p: &Path) -> bool {
        [
            "/",
            "/home",
            "/home/u",
            "/home/u/.ssh",
            "/home/u/.ssh/id_ed25519",
            "/home/u/.npmrc",
            "/etc",
            "/etc/hostname",
            "/opt",
            "/usr",
            "/nix",
            "/nix/store",
            "/tmp",
            "/tmp/proj",
        ]
        .contains(&p.to_str().unwrap())
    }

    fn of(output: &str) -> Vec<Denial> {
        denials(output, None, &policy(), &roots(), &on_host)
    }

    fn path(d: &Denial) -> (&str, bool) {
        match d {
            Denial::Path { path, write, .. } => (path, *write),
            other => panic!("not a path: {other:?}"),
        }
    }

    /// The lines the measurement produced, each attributed to the path it
    /// names and not to the shell that printed it.
    #[test]
    fn the_measured_spellings_name_their_path() {
        let found = of("cat: /home/u/.npmrc: No such file or directory\n\
             /nix/store/abc-bash/bin/bash: line 3: /home/u/newfile: No such file or directory\n\
             mkdir: cannot create directory '/opt': Permission denied\n\
             /nix/store/abc-bash/bin/bash: line 3: /usr/dcm: Read-only file system\n\
             touch: cannot touch '/nix/store/dcm': Read-only file system\n");
        let found: Vec<_> = found.iter().map(path).collect();
        assert_eq!(
            found,
            [
                ("/home/u/.npmrc", false),
                ("/home/u/newfile", false),
                ("/opt", false),
                ("/usr/dcm", true),
                ("/nix/store/dcm", true),
            ]
        );
    }

    /// The baseline writes `~/.ssh`; the script prints an absolute path.
    #[test]
    fn a_baseline_denial_is_named_as_the_baseline() {
        let found = of("open /home/u/.ssh/id_ed25519: permission denied\n");
        assert!(
            matches!(
                &found[..],
                [Denial::Path {
                    origin: Some(Origin::Baseline),
                    ..
                }]
            ),
            "{found:?}"
        );
    }

    #[test]
    fn python_and_node_put_the_path_after_the_error() {
        let found = of(
            "PermissionError: [Errno 13] Permission denied: '/etc/dcm'\n\
             Error: EACCES: permission denied, open '/opt/x'\n",
        );
        let found: Vec<_> = found.iter().map(path).collect();
        assert_eq!(found, [("/etc/dcm", false), ("/opt/x", false)]);
    }

    /// The other half: what the policy grants, or the host lacks, is the
    /// script's own failure, however it is spelled.
    #[test]
    fn what_the_policy_does_not_refuse_is_not_attributed() {
        let found = of(concat!(
            // Missing on the host too, inside the project (granted as `.`).
            "cat: /tmp/proj/missing.txt: No such file or directory\n",
            // Missing on the host, at the top: `/` is in every view.
            "cat: /nonexistent: No such file or directory\n",
            // A directory missing on the host, not hidden.
            "cat: /nope/x: No such file or directory\n",
            // Granted writable: the host's own permission bits.
            "sh: /tmp/proj/locked: Permission denied\n",
            // Not a path.
            "curl: (22) error: 403 https://x/y: Permission denied\n",
        ));
        assert_eq!(found, [], "{found:?}");
    }

    #[test]
    fn a_proxy_refusal_names_the_host_from_its_log() {
        let found = denials(
            "npm ERR! 403 Forbidden\n",
            Some(
                "allow peer=127.0.0.1:1 host=ok.example port=443\n\
                 refuse peer=127.0.0.1:2 host=registry.npmjs.org port=443 reason=not in allowlist\n\
                 refuse peer=127.0.0.1:3 reason=unauthenticated\n",
            ),
            &policy(),
            &roots(),
            &on_host,
        );
        assert_eq!(
            found,
            [Denial::Host {
                host: "registry.npmjs.org".into(),
                port: "443".into(),
                reason: "not in allowlist".into(),
            }]
        );
    }

    /// With no allowlist there is no proxy and no route; with one, a network
    /// error is not inferred, since the proxy's log says what it refused.
    #[test]
    fn a_network_failure_is_the_policy_only_without_an_allowlist() {
        let out = "curl: (6) Could not resolve host: registry.npmjs.org\n";
        assert_eq!(
            of(out),
            [Denial::NoNetwork {
                seen: "curl: (6) Could not resolve host: registry.npmjs.org".into()
            }]
        );
        assert_eq!(denials(out, Some(""), &policy(), &roots(), &on_host), []);
    }

    #[test]
    fn the_script_record_is_not_its_output() {
        let log = "[hook activation] $ __devcroft_exe='/x'\n\
             cat /etc/hostname: Permission denied\n\
             __devcroft_rc=$?\n\
             exit $__devcroft_rc\n\
             real output\n\
             [hook activation] note: it exited before devcroft could read its environment\n\
             devcroft-hook-failure: hook `activation` failed: exited with status 1\n";
        assert_eq!(script_output(log), "real output");
    }

    #[test]
    fn the_message_says_which_it_was() {
        let failure = "hook `activation` failed: exited with status 3";
        let denied = explain(
            failure,
            &of("cat: /etc/hostname: No such file or directory\n"),
            "",
            "s",
        );
        assert!(
            denied.contains("the provisioning policy refused it"),
            "{denied}"
        );
        assert!(
            denied.contains("/etc/hostname: not granted by any rule"),
            "{denied}"
        );
        let own = explain(failure, &[], "one\ntwo\nboom\n", "s");
        assert!(!own.contains("refused it"), "{own}");
        assert!(own.contains("the failure is the script's own"), "{own}");
        assert!(
            own.ends_with("| boom\n(`devcroft logs s` has all of it)"),
            "{own}"
        );
    }
}
