//! The `devbox` provider (add-devbox-provider): resolves a devbox
//! project's activated environment, host-side, before any sandbox
//! restriction (design.md decision 2) — the third closure-tier provider,
//! sharing capture/diff/fingerprint machinery with `flox.rs`/`nix.rs` via
//! `provider::capture`. Captures via `devbox shellenv --pure`, evaluated
//! in a controlled shell, never `devbox run`: only the former avoids
//! executing the project's `shell.init_hook` (design.md decisions 1–2 —
//! measured against devbox 0.18.0, not read from documentation).

use super::capture;
use super::{Provider, ProviderError, Resolution, ServiceSupport};
use crate::paths::resolve_on_path;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

pub struct DevboxProvider;

impl Provider for DevboxProvider {
    /// Resolve a devbox environment: verify `devbox.json` exists, then
    /// `devbox`, then Nix (devbox's own precondition, design.md decision
    /// 4), then that every declared package has a lockfile entry (design.md
    /// decision 1b), then capture activation via `shellenv --pure` and
    /// derive read-only store grants exactly as `flox.rs`/`nix.rs` do
    /// (design.md decision 1a — the coarse `/nix/store` grant already
    /// covers what the profile symlink resolves to).
    fn resolve(&self, project_root: &Path) -> Result<Resolution, ProviderError> {
        ensure_project_present(project_root)?;
        let devbox_bin = resolve_on_path("devbox").ok_or(ProviderError::MissingBinary {
            provider: "devbox",
            hint: "devcroft doctor",
        })?;
        ensure_nix_usable()?;
        ensure_everything_locked(project_root)?;

        // The lockfile as it was *before* capture. Compared again after,
        // because the precondition above cannot see every entry devbox
        // needs — see `restore_lock_if_capture_resolved`.
        let lock_before = read_lock(project_root);

        let baseline = capture::canonical_base_env()?;
        let activated = capture_activated_env(&devbox_bin, project_root, &baseline)?;
        restore_lock_if_capture_resolved(project_root, &lock_before)?;

        Ok(Resolution {
            env: capture::changed_env(&baseline, &activated),
            unset: capture::unset_env(&baseline, &activated),
            read_only_grants: capture::store_grants(&activated),
            // devbox services arrive via plugin-supplied process-compose
            // configs rather than a documented `devbox.json` schema —
            // the shape `add-flox-services` decision 1 rejected for
            // flox's own generated config. A separate change's decision
            // to make, not this one's (proposal.md — Impact).
            services: ServiceSupport::Unsupported,
            // Structurally false here, not merely unencountered: `shellenv`
            // never executes `shell.init_hook`, in any variant including
            // `--init-hook` (which only appends a source line to the
            // emitted text) — measured, design.md decision 2. Asserted by
            // a test (devbox_shellenv_does_not_run_the_init_hook) rather
            // than trusted as a property of devbox in general, since a
            // future switch to `devbox run` would silently reintroduce it.
            ran_activation_hook: false,
            // `print-dev-env --json` / `shellenv --pure` return the
            // environment without running the project's script, so there
            // is nothing to defer into the sandbox — it simply never runs.
            activation_script: None,
        })
    }
}

/// `up` fails at layer `provider` with the `devbox init` hint (spec:
/// "Missing environment, not missing feature") rather than letting devbox
/// itself produce a less specific error about a project it cannot find.
fn ensure_project_present(project_root: &Path) -> Result<(), ProviderError> {
    if project_root.join("devbox.json").is_file() {
        Ok(())
    } else {
        Err(ProviderError::NoEnvironment {
            provider: "devbox",
            hint: "devbox init",
        })
    }
}

/// devbox is a frontend over Nix and cannot materialize anything without
/// it (design.md decision 4). Reported by naming `nix` itself as the
/// missing binary — not `devbox`, which is already known to be present at
/// this point — so the message reads as "you also need nix", never as
/// "switch providers".
///
/// **Probes the capability rather than the binary's presence.** An
/// earlier version checked only `PATH`, which this repo has already been
/// bitten by once: `doctor`'s nix check used to probe with `nix flake
/// --help`, which succeeds even when the experimental features are off,
/// and so reported a healthy nix on a host where resolution then failed
/// (README, add-nix-provider). A `nix` that cannot evaluate is exactly
/// the failure this precondition exists to convert into a precise,
/// early error, and a presence check reports it as success.
///
/// `nix eval --expr 1` is the same probe `nix.rs`'s own provider path
/// settled on, chosen for the same reason: it requires `nix-command`,
/// which is what resolution actually fails on.
fn ensure_nix_usable() -> Result<(), ProviderError> {
    let nix = resolve_on_path("nix").ok_or(ProviderError::MissingBinary {
        provider: "nix",
        hint: "devcroft doctor",
    })?;

    let usable = Command::new(&nix)
        .arg("eval")
        .arg("--expr")
        .arg("1")
        .output()
        .is_ok_and(|o| o.status.success());

    if usable {
        Ok(())
    } else {
        Err(ProviderError::ResolutionFailed(
            "devbox requires a usable Nix, but `nix eval` failed on this host; devbox is a \
             frontend over Nix and cannot resolve packages without it (see `devcroft doctor`)"
                .to_string(),
        ))
    }
}

/// The lock key devbox would record for a declared package, replicating
/// devbox's own normalization (design.md decision 1b, measured against
/// devbox 0.18.0): a string value (array form, or an object value that is
/// itself a plain string) becomes `"{name}@{value}"`; a value with no
/// version at all (array form with no `@`, or an object value that is a
/// table with no `version` field) locks under the bare name — devbox's
/// "legacy" form, still accepted with a deprecation warning.
fn package_key(name: &str, value: &serde_json::Value) -> String {
    let version = match value {
        serde_json::Value::String(v) => Some(v.as_str()),
        serde_json::Value::Object(table) => table.get("version").and_then(|v| v.as_str()),
        _ => None,
    };
    match version {
        Some(v) => format!("{name}@{v}"),
        None => name.to_string(),
    }
}

/// Parses a devbox file, tolerating exactly what devbox itself tolerates.
///
/// `devbox.json` is **not** strict JSON. Measured against devbox 0.18.0
/// by feeding it each relaxation in turn: line comments (`//`), block
/// comments (`/* */`), and trailing commas are all accepted; hash
/// comments, single-quoted strings, and unquoted keys are rejected. That
/// is JSONC, and devbox's own documentation shows commented files, so
/// parsing strictly rejects projects devbox considers valid — found by
/// adversarial review, where `devcroft up` failed on a commented
/// `devbox.json` that `devbox list` read happily.
///
/// A ~40-line preprocessor rather than a JSONC crate: `use-nono-library`
/// established this repo weighs dependency tails seriously, and the
/// transformation is small enough to test exhaustively (strings
/// containing `//`, escaped quotes, and commas are the cases that matter,
/// and each has a test below).
fn parse_devbox_json(path: &Path) -> Result<serde_json::Value, ProviderError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ProviderError::ResolutionFailed(format!("reading {}: {e}", path.display())))?;
    serde_json::from_str(&strip_jsonc(&text))
        .map_err(|e| ProviderError::ResolutionFailed(format!("parsing {}: {e}", path.display())))
}

/// Removes JSONC comments and trailing commas, leaving string contents
/// untouched — a `//` or `,` inside a string literal is data, not syntax.
fn strip_jsonc(text: &str) -> String {
    let without_comments = strip_comments(text);
    strip_trailing_commas(&without_comments)
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        // Keep the newline so reported error positions
                        // still line up with the file the user sees.
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for c in chars.by_ref() {
                    if prev == '*' && c == '/' {
                        break;
                    }
                    if c == '\n' {
                        out.push('\n');
                    }
                    prev = c;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn strip_trailing_commas(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c == ',' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if matches!(chars.get(j), Some('}') | Some(']')) {
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// The lock keys `devbox.json`'s `packages` field declares, in whichever
/// of the two accepted shapes it uses — array of `"name@version"` strings,
/// or an object map of `name` to a version string or a table. An
/// unrecognized shape fails closed (`ResolutionFailed`) rather than being
/// treated as "no packages declared": a false negative here would defeat
/// the precondition entirely, the same bias `flox.rs`'s
/// `declares_activation_hook` already uses for the same reason.
fn declared_package_keys(project_root: &Path) -> Result<Vec<String>, ProviderError> {
    let devbox_json_path = project_root.join("devbox.json");
    let parsed = parse_devbox_json(&devbox_json_path)?;

    let Some(packages) = parsed.get("packages") else {
        return Ok(Vec::new());
    };

    match packages {
        serde_json::Value::Array(items) => items
            .iter()
            .map(|v| {
                v.as_str().map(str::to_string).ok_or_else(|| {
                    ProviderError::ResolutionFailed(format!(
                        "{}'s `packages` array has a non-string entry",
                        devbox_json_path.display()
                    ))
                })
            })
            .collect(),
        serde_json::Value::Object(map) => Ok(map
            .iter()
            .map(|(name, value)| package_key(name, value))
            .collect()),
        _ => Err(ProviderError::ResolutionFailed(format!(
            "{}'s `packages` field is neither a list nor a table",
            devbox_json_path.display()
        ))),
    }
}

/// The lockfile's raw bytes, or `None` when it does not exist — both are
/// meaningful states to compare against after capture (a lockfile devbox
/// *creates* during capture is as much a resolution as one it edits).
fn read_lock(project_root: &Path) -> Option<Vec<u8>> {
    std::fs::read(project_root.join("devbox.lock")).ok()
}

/// Enforces the spec sentence [`ensure_everything_locked`] can only
/// approximate: "Resolution SHALL respect the project's lockfile and
/// SHALL NOT update it, resolve package versions, or contact a package
/// index to decide *what* to install."
///
/// **Why a post-check is needed at all**, found by adversarial review of
/// the shipped implementation rather than during design: a project whose
/// every *declared* package is locked can still have capture rewrite
/// `devbox.lock`, because devbox's lockfile also carries its own base
/// nixpkgs entry, which is not a declared package and which
/// [`ensure_everything_locked`] therefore never looks at. Measured: a
/// lockfile holding a fully-resolved `cowsay@latest` but no
/// `github:NixOS/nixpkgs/…` entry passed every precondition, and `up`
/// then resolved that entry live — against the floating
/// `nixpkgs-unstable` branch — and wrote it to disk.
///
/// **Why this is a post-check rather than one more precondition.** The
/// base entry's key is not a constant: measured, a project pinning
/// `nixpkgs.commit` in `devbox.json` locks under
/// `github:NixOS/nixpkgs/<that commit>` instead of
/// `github:NixOS/nixpkgs/nixpkgs-unstable`. Predicting the full key set
/// means reimplementing devbox's own resolution rules, which is exactly
/// what design.md decision 1 rejects ("devcroft would own a second
/// implementation of devbox's semantics, which will drift"). Comparing
/// the file's bytes needs no such knowledge and keeps working if devbox
/// changes its key scheme.
///
/// Restores the original bytes before failing, so a rejected `up` leaves
/// the working tree exactly as it found it rather than reporting a
/// violation it already committed.
fn restore_lock_if_capture_resolved(
    project_root: &Path,
    before: &Option<Vec<u8>>,
) -> Result<(), ProviderError> {
    let after = read_lock(project_root);
    if &after == before {
        return Ok(());
    }

    let lock_path = project_root.join("devbox.lock");
    match before {
        Some(bytes) => std::fs::write(&lock_path, bytes),
        // Capture created a lockfile where the project had none.
        None => std::fs::remove_file(&lock_path),
    }
    .map_err(|e| {
        ProviderError::ResolutionFailed(format!(
            "devbox resolved during `up` and rewrote {}, and restoring it failed: {e}",
            lock_path.display()
        ))
    })?;

    Err(ProviderError::ResolutionFailed(format!(
        "devbox resolved packages while capturing the environment and rewrote {} \
         (devcroft restored it); provisioning must not resolve versions or contact a \
         package index — run `devbox install` and commit the result",
        lock_path.display()
    )))
}

/// Preconditions SHALL be expressed as "nothing resolves at `up`", not as
/// "a lockfile exists" (spec, design.md decision 1b): a project declaring
/// no packages needs no lockfile at all, and — corrected by measurement,
/// superseding an earlier draft that required per-system lock coverage —
/// a lock entry present for any system resolves correctly for every
/// system from its pinned commit reference.
///
/// This is a **necessary but not sufficient** check, and deliberately so:
/// it names the offending package precisely, before anything runs, which
/// a byte comparison cannot do. What it cannot see is devbox's own base
/// nixpkgs entry, so [`restore_lock_if_capture_resolved`] backstops it
/// after capture — see that function for the measurement.
fn ensure_everything_locked(project_root: &Path) -> Result<(), ProviderError> {
    let declared = declared_package_keys(project_root)?;
    if declared.is_empty() {
        return Ok(());
    }

    let lock_path = project_root.join("devbox.lock");
    if !lock_path.is_file() {
        return Err(ProviderError::MissingLock {
            provider: "devbox",
            hint: "devbox install",
        });
    }

    // Same tolerant parse as `devbox.json`. `devbox.lock` is generated
    // rather than hand-authored, so comments in it are unlikely — but
    // reading the two files by different rules would be a surprise
    // waiting to happen, and costs nothing to avoid.
    let parsed = parse_devbox_json(&lock_path)?;
    let locked: BTreeSet<&str> = parsed
        .get("packages")
        .and_then(|v| v.as_object())
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default();

    for key in &declared {
        if !locked.contains(key.as_str()) {
            return Err(ProviderError::ResolutionFailed(format!(
                "package `{key}` is declared in devbox.json but has no resolution in \
                 devbox.lock; run `devbox install`"
            )));
        }
    }
    Ok(())
}

/// Reads devbox's activated environment by evaluating `shellenv --pure`
/// in a controlled shell and dumping the result — never by running a
/// command inside the activated shell (`devbox run`), which executes
/// `shell.init_hook` (design.md decisions 1–2).
///
/// `--pure` is mandatory, not a refinement: without it, `shellenv`
/// re-exports the invoking shell's entire ambient environment into its
/// output — measured to carry operator-specific variables that would
/// silently break the "activation diff is independent of who ran `up`"
/// guarantee every provider shares.
///
/// The devbox binary's path reaches the script as a **positional
/// argument** (`sh -c '…' devcroft-devbox <path>`, read back as `$1`),
/// never interpolated into the script text. This is the only provider
/// that goes through a shell at all — flox and nix build argv directly —
/// so it is the only one where a path containing a quote could change
/// what the shell parses. Passing it as an argument removes the question
/// entirely rather than relying on the path being well-behaved.
fn capture_activated_env(
    devbox_bin: &Path,
    project_root: &Path,
    base: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, ProviderError> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(r#"eval "$("$1" shellenv --pure)" && env -0"#)
        // `$0` for the shell's own error messages, then `$1`.
        .arg("devcroft-devbox-capture")
        .arg(devbox_bin)
        .current_dir(project_root)
        .env_clear()
        .envs(base)
        .output()
        .map_err(|e| ProviderError::ResolutionFailed(format!("running `devbox shellenv`: {e}")))?;

    if !output.status.success() {
        return Err(ProviderError::ResolutionFailed(format!(
            "`devbox shellenv` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(capture::parse_env_dump(&output.stdout))
}

/// Content fingerprint of a devbox project's `devbox.json` + `devbox.lock`,
/// for staleness detection (spec: "Stale environment after devbox file
/// change"). `provider::is_stale` (dispatched by provider name) compares
/// this against the fingerprint recorded at the last `up`.
///
/// A lockfile's absence is hashed as a distinct state, not folded into
/// "empty" (spec: "A lockfile appearing is itself a change"). This was
/// open-coded here first and is now `capture::optional_file_part`, shared
/// with flox and nix — which had the same stated intent and the
/// `unwrap_or_default()` implementation that does not deliver it.
pub fn devbox_fingerprint(project_root: &Path) -> Result<String, ProviderError> {
    ensure_project_present(project_root)?;
    let json_path = project_root.join("devbox.json");
    let json = std::fs::read(&json_path).map_err(|e| {
        ProviderError::ResolutionFailed(format!("reading {}: {e}", json_path.display()))
    })?;
    let lock = capture::optional_file_part(&project_root.join("devbox.lock"));

    Ok(capture::fingerprint(&[&json, &lock]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tempdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-devbox-test-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn real_devbox() -> Option<PathBuf> {
        resolve_on_path("devbox").filter(|devbox| {
            Command::new(devbox)
                .arg("version")
                .output()
                .is_ok_and(|o| o.status.success())
        })
    }

    fn write_devbox_project(root: &Path, devbox_json: &str, lock: Option<&str>) {
        fs::write(root.join("devbox.json"), devbox_json).unwrap();
        if let Some(lock) = lock {
            fs::write(root.join("devbox.lock"), lock).unwrap();
        }
    }

    /// Materializes a **complete** lockfile the way a real project would.
    ///
    /// `devbox install`, specifically — not `devbox add`. Measured: `add`
    /// writes the package's own entry but *not* devbox's base nixpkgs
    /// entry, leaving a lockfile that capture would still have to
    /// complete (and therefore rewrite). `install` writes both, and a
    /// lockfile it produced survives capture byte-identically. Returns
    /// false when the environment cannot resolve at all (no network for
    /// nixpkgs), so callers skip rather than fail.
    fn devbox_install(devbox: &Path, root: &Path) -> bool {
        Command::new(devbox)
            .arg("install")
            .current_dir(root)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[test]
    fn resolve_fails_with_no_environment_when_devbox_json_missing() {
        let root = tempdir("no-env");
        let err = DevboxProvider.resolve(&root).unwrap_err();
        assert_eq!(
            err,
            ProviderError::NoEnvironment {
                provider: "devbox",
                hint: "devbox init"
            }
        );
    }

    #[test]
    fn package_key_uses_object_value_string_as_version() {
        let value = serde_json::json!("latest");
        assert_eq!(package_key("ripgrep", &value), "ripgrep@latest");
    }

    #[test]
    fn package_key_reads_version_field_from_object_table() {
        let value = serde_json::json!({ "version": "14.1.0", "platforms": ["aarch64-linux"] });
        assert_eq!(package_key("ripgrep", &value), "ripgrep@14.1.0");
    }

    #[test]
    fn package_key_falls_back_to_bare_name_with_no_version() {
        let value = serde_json::json!({});
        assert_eq!(package_key("ripgrep", &value), "ripgrep");
    }

    #[test]
    fn declared_package_keys_reads_array_form_verbatim() {
        let root = tempdir("declared-array");
        write_devbox_project(&root, r#"{"packages": ["ripgrep@latest", "jq"]}"#, None);
        let keys = declared_package_keys(&root).unwrap();
        assert_eq!(keys, vec!["ripgrep@latest".to_string(), "jq".to_string()]);
    }

    #[test]
    fn declared_package_keys_reads_object_form() {
        let root = tempdir("declared-object");
        write_devbox_project(&root, r#"{"packages": {"ripgrep": "latest"}}"#, None);
        let keys = declared_package_keys(&root).unwrap();
        assert_eq!(keys, vec!["ripgrep@latest".to_string()]);
    }

    #[test]
    fn strip_jsonc_removes_line_and_block_comments() {
        let src = r#"{
  // line
  "packages": [], /* block */
  "shell": {}
}"#;
        let parsed: serde_json::Value = serde_json::from_str(&strip_jsonc(src)).unwrap();
        assert!(parsed.get("packages").is_some());
        assert!(parsed.get("shell").is_some());
    }

    #[test]
    fn strip_jsonc_removes_trailing_commas_in_objects_and_arrays() {
        let src = r#"{"packages": ["a", "b",], "shell": {"x": 1,},}"#;
        let parsed: serde_json::Value = serde_json::from_str(&strip_jsonc(src)).unwrap();
        assert_eq!(parsed["packages"].as_array().unwrap().len(), 2);
    }

    /// The case that breaks a naive implementation: `//`, `/*` and `,`
    /// inside a string literal are data, not syntax.
    #[test]
    fn strip_jsonc_leaves_string_contents_alone() {
        let src = r#"{"packages": ["https://example.com/x", "a,b", "/* not a comment */"]}"#;
        let parsed: serde_json::Value = serde_json::from_str(&strip_jsonc(src)).unwrap();
        let pkgs = parsed["packages"].as_array().unwrap();
        assert_eq!(pkgs[0], "https://example.com/x");
        assert_eq!(pkgs[1], "a,b");
        assert_eq!(pkgs[2], "/* not a comment */");
    }

    /// An escaped quote must not be read as the end of the string, or
    /// everything after it is treated as syntax.
    #[test]
    fn strip_jsonc_handles_escaped_quotes_inside_strings() {
        let src = r#"{"packages": ["say \"hi\" // now"]}"#;
        let parsed: serde_json::Value = serde_json::from_str(&strip_jsonc(src)).unwrap();
        assert_eq!(parsed["packages"][0], r#"say "hi" // now"#);
    }

    #[test]
    fn declared_package_keys_reads_a_commented_devbox_json() {
        let root = tempdir("declared-jsonc");
        write_devbox_project(
            &root,
            "{\n  // devbox's own docs show commented files\n  \"packages\": [\"ripgrep@latest\",],\n}",
            None,
        );
        assert_eq!(
            declared_package_keys(&root).unwrap(),
            vec!["ripgrep@latest".to_string()]
        );
    }

    #[test]
    fn declared_package_keys_is_empty_when_packages_field_absent() {
        let root = tempdir("declared-absent");
        write_devbox_project(&root, r#"{}"#, None);
        assert!(declared_package_keys(&root).unwrap().is_empty());
    }

    /// **Corrected by adversarial review.** An earlier version of this
    /// test asserted a zero-package project resolves with *no lockfile
    /// at all*, matching a spec scenario written on the reasoning "no
    /// packages declared, so nothing to resolve". Measurement falsified
    /// both: devbox's stdenv (gcc, coreutils, bash — visible on the
    /// captured `PATH` even here) comes from its base nixpkgs, and with
    /// no lockfile that base is the *floating* `nixpkgs-unstable`
    /// branch, resolved at `up` and written to disk. A zero-package
    /// devbox project is reproducible only once that base is pinned.
    #[test]
    fn resolve_succeeds_with_no_packages_once_the_base_is_locked() {
        let Some(devbox) = real_devbox() else { return };
        if resolve_on_path("nix").is_none() {
            return;
        }
        let root = tempdir("zero-packages-resolve");
        write_devbox_project(&root, "{}", None);
        if !devbox_install(&devbox, &root) {
            return;
        }
        let resolution = DevboxProvider.resolve(&root).unwrap();
        assert!(!resolution.ran_activation_hook);
    }

    /// The other half of the correction above: without that lockfile,
    /// `up` must refuse rather than silently pinning whatever
    /// `nixpkgs-unstable` points at today.
    #[test]
    fn resolve_refuses_a_zero_package_project_with_no_lockfile() {
        if real_devbox().is_none() || resolve_on_path("nix").is_none() {
            return;
        }
        let root = tempdir("zero-packages-unlocked");
        write_devbox_project(&root, "{}", None);

        let err = DevboxProvider.resolve(&root).unwrap_err();
        match err {
            ProviderError::ResolutionFailed(msg) => assert!(msg.contains("devbox install")),
            other => panic!("expected ResolutionFailed naming `devbox install`, got {other:?}"),
        }
        assert!(
            !root.join("devbox.lock").is_file(),
            "a refused `up` must not leave behind the lockfile capture created"
        );
    }

    /// Regression for the gap adversarial review found in the shipped
    /// implementation: every *declared* package is fully locked, so
    /// `ensure_everything_locked` passes, but devbox's own base nixpkgs
    /// entry is absent — so capture resolves it live (against the
    /// floating `nixpkgs-unstable` branch) and rewrites the user's
    /// lockfile. `up` must now refuse, and must leave the file untouched.
    #[test]
    fn resolve_refuses_and_restores_when_capture_would_rewrite_the_lockfile() {
        if real_devbox().is_none() || resolve_on_path("nix").is_none() {
            return;
        }
        let root = tempdir("lock-rewrite-refused");
        // `cowsay@latest`, fully resolved for this system, with no
        // `github:NixOS/nixpkgs/...` base entry beside it.
        let lock = r#"{
  "lockfile_version": "1",
  "packages": {
    "cowsay@latest": {
      "last_modified": "2026-08-12T11:28:58Z",
      "resolved": "github:NixOS/nixpkgs/044bfe75bfe4c7bbe043dc17b5e42ea823b84a09#cowsay",
      "source": "devbox-search",
      "version": "3.8.4"
    }
  }
}"#;
        write_devbox_project(&root, r#"{"packages": ["cowsay@latest"]}"#, Some(lock));

        let err = DevboxProvider.resolve(&root).unwrap_err();
        match err {
            ProviderError::ResolutionFailed(msg) => {
                assert!(msg.contains("devbox install"), "got: {msg}");
                assert!(msg.contains("restored"), "got: {msg}");
            }
            other => panic!("expected ResolutionFailed about a rewritten lockfile, got {other:?}"),
        }

        assert_eq!(
            std::fs::read_to_string(root.join("devbox.lock")).unwrap(),
            lock,
            "a refused `up` must leave devbox.lock byte-identical"
        );
    }

    /// The companion property: a project whose lockfile is genuinely
    /// complete resolves without the post-check firing, so the guard
    /// above cannot be satisfied by simply always failing.
    #[test]
    fn resolve_leaves_a_complete_lockfile_untouched() {
        let Some(devbox) = real_devbox() else { return };
        if resolve_on_path("nix").is_none() {
            return;
        }
        let root = tempdir("lock-untouched");
        write_devbox_project(&root, r#"{"packages": ["cowsay@latest"]}"#, None);
        let install = Command::new(&devbox)
            .arg("install")
            .current_dir(&root)
            .output()
            .unwrap();
        if !install.status.success() {
            return;
        }

        let before = std::fs::read(root.join("devbox.lock")).unwrap();
        DevboxProvider.resolve(&root).unwrap();
        let after = std::fs::read(root.join("devbox.lock")).unwrap();

        assert_eq!(
            before, after,
            "a complete lockfile must survive `up` unchanged"
        );
    }

    #[test]
    fn ensure_everything_locked_fails_when_lockfile_missing_entirely() {
        let root = tempdir("locked-no-lockfile");
        write_devbox_project(&root, r#"{"packages": ["ripgrep@latest"]}"#, None);
        let err = ensure_everything_locked(&root).unwrap_err();
        assert_eq!(
            err,
            ProviderError::MissingLock {
                provider: "devbox",
                hint: "devbox install"
            }
        );
    }

    #[test]
    fn ensure_everything_locked_fails_when_declared_package_has_no_lock_entry() {
        let root = tempdir("locked-partial");
        write_devbox_project(
            &root,
            r#"{"packages": ["ripgrep@latest", "jq@latest"]}"#,
            Some(r#"{"packages": {"ripgrep@latest": {"version": "15.2.0"}}}"#),
        );
        let err = ensure_everything_locked(&root).unwrap_err();
        match err {
            ProviderError::ResolutionFailed(msg) => {
                assert!(msg.contains("jq@latest"));
                assert!(msg.contains("devbox install"));
            }
            other => panic!("expected ResolutionFailed naming jq@latest, got {other:?}"),
        }
    }

    #[test]
    fn ensure_everything_locked_passes_when_lock_covers_only_a_different_system() {
        // The precondition this guards is "does a lock entry exist", not
        // "does it cover this system" (design.md decision 1b, measured:
        // an entry pinned to a different system still resolves here from
        // its fixed commit reference, without touching the lockfile).
        let root = tempdir("locked-other-system");
        write_devbox_project(
            &root,
            r#"{"packages": ["ripgrep@latest"]}"#,
            Some(
                r#"{"packages": {"ripgrep@latest": {"version": "15.2.0", "systems": {"x86_64-darwin": {}}}}}"#,
            ),
        );
        assert!(ensure_everything_locked(&root).is_ok());
    }

    #[test]
    fn ensure_everything_locked_passes_with_no_declared_packages_and_no_lockfile() {
        let root = tempdir("locked-nothing-declared");
        write_devbox_project(&root, "{}", None);
        assert!(ensure_everything_locked(&root).is_ok());
    }

    #[test]
    fn devbox_fingerprint_distinguishes_absent_from_present_empty_lock() {
        let absent = tempdir("fingerprint-absent-lock");
        write_devbox_project(&absent, r#"{"packages": []}"#, None);
        let present_empty = tempdir("fingerprint-present-empty-lock");
        write_devbox_project(&present_empty, r#"{"packages": []}"#, Some(""));

        assert_ne!(
            devbox_fingerprint(&absent).unwrap(),
            devbox_fingerprint(&present_empty).unwrap()
        );
    }

    #[test]
    fn devbox_fingerprint_changes_when_a_lockfile_appears() {
        let root = tempdir("fingerprint-lock-appears");
        write_devbox_project(&root, r#"{"packages": ["ripgrep@latest"]}"#, None);
        let before = devbox_fingerprint(&root).unwrap();

        write_devbox_project(
            &root,
            r#"{"packages": ["ripgrep@latest"]}"#,
            Some(r#"{"packages": {"ripgrep@latest": {}}}"#),
        );
        let after = devbox_fingerprint(&root).unwrap();

        assert_ne!(before, after);
    }

    #[test]
    fn devbox_fingerprint_changes_when_devbox_json_changes() {
        let root = tempdir("fingerprint-json-changes");
        write_devbox_project(&root, r#"{"packages": []}"#, None);
        let before = devbox_fingerprint(&root).unwrap();

        write_devbox_project(&root, r#"{"packages": ["jq@latest"]}"#, None);
        let after = devbox_fingerprint(&root).unwrap();

        assert_ne!(before, after);
    }

    #[test]
    fn devbox_fingerprint_is_stable_for_unchanged_content() {
        let root = tempdir("fingerprint-stable");
        write_devbox_project(
            &root,
            r#"{"packages": ["ripgrep@latest"]}"#,
            Some(r#"{"packages": {"ripgrep@latest": {}}}"#),
        );
        let first = devbox_fingerprint(&root).unwrap();
        let second = devbox_fingerprint(&root).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn devbox_fingerprint_fails_without_devbox_json() {
        let root = tempdir("fingerprint-missing");
        let err = devbox_fingerprint(&root).unwrap_err();
        assert_eq!(
            err,
            ProviderError::NoEnvironment {
                provider: "devbox",
                hint: "devbox init"
            }
        );
    }

    /// The property that qualified devbox in the first place (design.md
    /// decision 2): capture must never run `shell.init_hook`. Asserted
    /// live so a future switch back to `devbox run` for any reason would
    /// break this test rather than silently reintroducing the violation.
    #[test]
    fn devbox_shellenv_does_not_run_the_init_hook() {
        let Some(devbox) = real_devbox() else { return };
        if resolve_on_path("nix").is_none() {
            return;
        }
        let root = tempdir("init-hook-does-not-run");
        let sentinel = root.join("hook-ran");
        write_devbox_project(
            &root,
            &format!(
                r#"{{"packages": [], "shell": {{"init_hook": ["touch {}"]}}}}"#,
                sentinel.display()
            ),
            None,
        );
        if !devbox_install(&devbox, &root) {
            return;
        }

        let resolution = DevboxProvider.resolve(&root).unwrap();

        assert!(!resolution.ran_activation_hook);
        assert!(
            !sentinel.exists(),
            "init_hook ran during resolution; sentinel file was created"
        );
    }

    /// Non-stdenv marker package (design.md decision 1a): if grant
    /// derivation were narrower than the whole `/nix/store` root, this is
    /// the package whose absence from a scraped grant would be
    /// observable — ripgrep is not part of devbox's own stdenv wrapper.
    #[test]
    fn resolve_against_a_real_project_grants_the_store_root() {
        let Some(devbox) = real_devbox() else { return };
        if resolve_on_path("nix").is_none() {
            return;
        }
        let root = tempdir("real-resolve-grants");
        write_devbox_project(&root, r#"{"packages": ["ripgrep@latest"]}"#, None);
        // `install`, not `add` — see `devbox_install`'s own doc comment:
        // `add` leaves the base nixpkgs entry unlocked, which capture
        // would then have to write, and `resolve` now refuses that.
        if !devbox_install(&devbox, &root) {
            return;
        }

        let resolution = DevboxProvider.resolve(&root).unwrap();
        assert_eq!(resolution.read_only_grants, vec!["/nix/store".to_string()]);
    }
}
