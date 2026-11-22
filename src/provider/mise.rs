//! Locked, tools-only mise materialization. Original project code is parsed as
//! data; its environment script is handed to confined provisioning.
use super::{ProviderError, Resolution, ServiceSupport, artifacts, capture};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use toml::Value;

pub(crate) const ADAPTER: &str = "mise-http-2";
pub(crate) const QUALIFIED_VERSION: &str = "2026.11.9";

/// Read-only project diagnostics. Tool installation and activation are never
/// invoked; version discovery runs against an empty temporary configuration.
pub fn diagnose(root: &Path) -> Result<String, ProviderError> {
    let input = Input::load(root, false)?;
    let binary = crate::paths::resolve_on_path("mise").ok_or(ProviderError::MissingBinary {
        provider: "mise",
        hint: "install mise and run devcroft doctor",
    })?;
    let scratch = std::env::temp_dir().join(format!(
        "devcroft-mise-doctor-{}-{:x}",
        std::process::id(),
        rand::random::<u64>()
    ));
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&scratch)
        .map_err(|e| fail(e.to_string()))?;
    let env = isolated_env(&scratch)?;
    let output = run(&binary, &["--version"], &scratch, &env);
    let _ = std::fs::remove_dir_all(&scratch);
    let output = String::from_utf8(output?).map_err(|e| fail(e.to_string()))?;
    if version(&output)? < version(QUALIFIED_VERSION)? {
        return Err(fail(format!(
            "requires mise >= {QUALIFIED_VERSION}; found {}",
            output.trim()
        )));
    }
    if target() == "unqualified" {
        return Err(fail("runtime platform has not been qualified"));
    }
    let shell = Path::new("/bin/sh")
        .canonicalize()
        .map_err(|e| fail(e.to_string()))?;
    fn size(root: &Path) -> u64 {
        std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| {
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    size(&e.path())
                } else {
                    e.metadata().map_or(0, |m| m.len())
                }
            })
            .sum()
    }
    let store = store_root()?;
    Ok(format!(
        "{}; {} locked HTTP tool(s), {}; shell {}; store {} bytes; [tasks] are not runnable by name (mise run is absent)",
        output.trim(),
        input.tools.len(),
        target(),
        shell.display(),
        size(&store)
    ))
}

fn fail(message: impl Into<String>) -> ProviderError {
    ProviderError::ResolutionFailed(format!("mise: {}", message.into()))
}

fn read(path: &Path) -> Result<Vec<u8>, ProviderError> {
    std::fs::read(path).map_err(|e| fail(format!("{}: {e}", path.display())))
}

pub(crate) fn target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64",
        ("linux", "aarch64") => "linux-arm64",
        // Locks for x64 have been measured; runtime qualification has not.
        _ => "unqualified",
    }
}

pub(crate) fn manifest_path(root: &Path) -> Result<PathBuf, ProviderError> {
    let paths: Vec<_> = ["mise.toml", ".mise.toml"]
        .iter()
        .map(|p| root.join(p))
        .filter(|p| p.exists())
        .collect();
    match paths.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(fail(
            "missing mise.toml or .mise.toml; initialize and lock the project with mise",
        )),
        _ => Err(fail(
            "both mise.toml and .mise.toml exist; select a single root definition",
        )),
    }
}

/// Refuse the finite discovery surface of the qualified CLI, including early
/// miserc settings, without following project directory symlinks out of scope.
fn check_other_config(root: &Path, chosen: &Path) -> Result<(), ProviderError> {
    fn walk(dir: &Path, depth: usize, chosen: &Path) -> Result<(), ProviderError> {
        if depth > 4 || !dir.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(dir).map_err(|e| fail(e.to_string()))? {
            let entry = entry.map_err(|e| fail(e.to_string()))?;
            let path = entry.path();
            if path == chosen {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().map_err(|e| fail(e.to_string()))?.is_dir() {
                walk(&path, depth + 1, chosen)?;
            } else if name.ends_with(".toml") || name == ".tool-versions" {
                return Err(fail(format!(
                    "unsupported configuration location {}",
                    path.display()
                )));
            }
        }
        Ok(())
    }
    for entry in std::fs::read_dir(root).map_err(|e| fail(e.to_string()))? {
        let path = entry.map_err(|e| fail(e.to_string()))?.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if path != chosen
            && (name == ".tool-versions"
                || ((name.starts_with("mise.")
                    || name.starts_with(".mise.")
                    || name.starts_with(".miserc."))
                    && name.ends_with(".toml")))
        {
            return Err(fail(format!(
                "unsupported configuration location {}",
                path.display()
            )));
        }
    }
    for dir in [".config/mise", ".mise", "mise"] {
        walk(&root.join(dir), 0, chosen)?;
    }
    for name in [
        ".config/mise.toml",
        ".config/mise.local.toml",
        ".config/miserc.toml",
    ] {
        if root.join(name).exists() {
            return Err(fail(format!("unsupported configuration location {name}")));
        }
    }
    // .config/mise.<env>.toml and .config/mise.<env>.local.toml.
    if root.join(".config").is_dir() {
        for entry in std::fs::read_dir(root.join(".config")).map_err(|e| fail(e.to_string()))? {
            let path = entry.map_err(|e| fail(e.to_string()))?.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with("mise.") && name.ends_with(".toml") {
                return Err(fail(format!(
                    "unsupported configuration location {}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

pub(crate) struct Input {
    pub definition: PathBuf,
    pub tools: toml::Table,
    pub lock: Vec<u8>,
    pub activation: String,
    pub files: Vec<PathBuf>,
    pub min_version: Option<String>,
}

impl Input {
    pub fn load(root: &Path, skip_hooks: bool) -> Result<Self, ProviderError> {
        Self::load_inner(root, skip_hooks, true)
    }

    fn load_inner(root: &Path, skip_hooks: bool, strict: bool) -> Result<Self, ProviderError> {
        let root = root.canonicalize().map_err(|e| fail(e.to_string()))?;
        let definition = manifest_path(&root)?;
        check_other_config(&root, &definition)?;
        let raw = read(&definition)?;
        let text = std::str::from_utf8(&raw).map_err(|e| fail(e.to_string()))?;
        let value: toml::Table =
            toml::from_str(text).map_err(|e| fail(format!("{}: {e}", definition.display())))?;
        for (key, val) in &value {
            match key.as_str() {
                "tools" | "env" | "min_version" | "tasks" | "task_config" | "vars" => {}
                "daemons" if skip_hooks => {}
                "daemons" => {
                    return Err(fail(
                        "[daemons]: services are not yet supported; see add-mise-daemons",
                    ));
                }
                "settings" => {
                    let settings = val
                        .as_table()
                        .ok_or_else(|| fail("settings must be a table"))?;
                    for (key, value) in settings {
                        if !["experimental", "lockfile", "locked"].contains(&key.as_str()) {
                            return Err(fail(format!("unsupported settings.{key}")));
                        }
                        let enabled = value
                            .as_bool()
                            .ok_or_else(|| fail(format!("settings.{key} must be boolean")))?;
                        if key != "experimental" && !enabled {
                            return Err(fail(format!(
                                "settings.{key} cannot disable strict locks"
                            )));
                        }
                    }
                }
                _ => return Err(fail(format!("unsupported [{key}]"))),
            }
        }
        let tools = value
            .get("tools")
            .and_then(Value::as_table)
            .cloned()
            .ok_or_else(|| fail("[tools] must be a table"))?;
        if tools.is_empty() {
            return Err(fail("[tools] is empty"));
        }
        for (name, value) in &tools {
            if !name.starts_with("http:") {
                return Err(fail(format!(
                    "tool {name}: backend is not admitted; core backends execute downloaded tools on the host; the qualified backend is http"
                )));
            }
            if !name[5..]
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
                || name.len() == 5
            {
                return Err(fail(format!("tool {name}: invalid HTTP tool name")));
            }
            let options = value
                .as_table()
                .ok_or_else(|| fail(format!("tools.{name} requires version and literal url")))?;
            for key in options.keys() {
                if !["version", "url"].contains(&key.as_str()) {
                    return Err(fail(format!("unsupported tools.{name}.{key}")));
                }
            }
            for key in ["version", "url"] {
                let text = options
                    .get(key)
                    .and_then(Value::as_str)
                    .ok_or_else(|| fail(format!("tools.{name}.{key} must be a string")))?;
                no_template(text, &format!("tools.{name}.{key}"))?;
            }
            let url = options["url"].as_str().unwrap();
            if !url.starts_with("https://") || url.contains('@') || url.contains('#') {
                return Err(fail(format!(
                    "tools.{name}.url requires HTTPS without credentials or fragments"
                )));
            }
        }
        let mut files = vec![];
        let activation = environment(text, &root, &mut files)?;
        let min_version = value
            .get("min_version")
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| fail("min_version must be a string"))
            })
            .transpose()?;
        let lock = if strict {
            let path = root.join("mise.lock");
            if !path.is_file() {
                // devcroft never creates or updates a lock: an unlocked
                // install would resolve versions and URLs at `up` time.
                return Err(fail(format!(
                    "{} has no mise.lock; run `mise lock` (mise >= {QUALIFIED_VERSION}) on this \
                     platform, review the checksums and commit it — devcroft does not create \
                     or update the lock",
                    definition.display()
                )));
            }
            read(&path)?
        } else {
            std::fs::read(root.join("mise.lock")).unwrap_or_default()
        };
        let input = Self {
            definition,
            tools,
            lock,
            activation,
            files,
            min_version,
        };
        if strict {
            input.check_lock()?;
        }
        Ok(input)
    }

    fn check_lock(&self) -> Result<(), ProviderError> {
        let lock: Value =
            toml::from_str(std::str::from_utf8(&self.lock).map_err(|e| fail(e.to_string()))?)
                .map_err(|e| fail(format!("mise.lock: {e}")))?;
        let format = lock
            .get("lockfile_version")
            .and_then(Value::as_integer)
            .unwrap_or(0);
        if !(2..=4).contains(&format) {
            return Err(fail(format!(
                "unsupported mise.lock format {format}; run mise lock with {QUALIFIED_VERSION}"
            )));
        }
        for (tool, options) in &self.tools {
            let entries = lock
                .get("tools")
                .and_then(|v| v.get(tool))
                .and_then(Value::as_array)
                .ok_or_else(|| fail(format!("mise.lock: {tool} is missing; run mise lock")))?;
            let request = options["version"].as_str().unwrap();
            let matches: Vec<_> = entries
                .iter()
                .filter(|e| {
                    e.get("specifiers")
                        .and_then(Value::as_array)
                        .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(request)))
                })
                .collect();
            if matches.len() != 1 {
                return Err(fail(format!(
                    "mise.lock: {tool}: expected one entry for {request}"
                )));
            }
            let entry = matches[0];
            if entry.get("backend").and_then(Value::as_str) != Some(tool) {
                return Err(fail(format!("mise.lock: {tool} backend does not match")));
            }
            let platform = entry
                .get(format!("platforms.{}", target()))
                .ok_or_else(|| fail(format!("mise.lock: {tool}: missing platform {}", target())))?;
            if platform.get("url").and_then(Value::as_str)
                != options.get("url").and_then(Value::as_str)
            {
                return Err(fail(format!(
                    "mise.lock: {tool}: URL differs from tools declaration"
                )));
            }
            let checksum = platform
                .get("checksum")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !checksum
                .strip_prefix("sha256:")
                .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err(fail(format!(
                    "mise.lock: {tool}: {} needs a SHA-256 checksum",
                    target()
                )));
            }
        }
        Ok(())
    }
}

fn no_template(text: &str, key: &str) -> Result<(), ProviderError> {
    if text.contains("{{") || text.contains("{%") || text.contains("{#") {
        return Err(fail(format!("unsupported template in {key}")));
    }
    Ok(())
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn env_name(name: &str) -> Result<(), ProviderError> {
    if name.is_empty()
        || name.as_bytes()[0].is_ascii_digit()
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(fail(format!("invalid environment variable {name}")));
    }
    Ok(())
}

fn project_path(root: &Path, text: &str) -> Result<PathBuf, ProviderError> {
    let path = root.join(text);
    // For staleness a missing file still has an identity. Existing ancestors
    // must stay inside the project even when a leaf does not yet exist.
    let mut ancestor = path.as_path();
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| fail("invalid project path"))?;
    }
    if !ancestor
        .canonicalize()
        .map_err(|e| fail(e.to_string()))?
        .starts_with(root)
        || path
            .components()
            .any(|c| c == std::path::Component::ParentDir)
    {
        return Err(fail(format!("path {} escapes the project", path.display())));
    }
    Ok(path)
}

fn environment(text: &str, root: &Path, files: &mut Vec<PathBuf>) -> Result<String, ProviderError> {
    let doc = toml_edit::Document::parse(text).map_err(|e| fail(e.to_string()))?;
    let Some(env) = doc.get("env") else {
        return Ok(String::new());
    };
    let env = env
        .as_table_like()
        .ok_or_else(|| fail("env must be a table"))?;
    let mut ordered = vec![];
    for (key, item) in env.iter() {
        if key == "_" {
            let table = item
                .as_table_like()
                .ok_or_else(|| fail("env._ must be a table"))?;
            for (directive, value) in table.iter() {
                ordered.push((
                    value.span().map_or(usize::MAX, |s| s.start),
                    format!("_.{directive}"),
                    value,
                ));
            }
        } else {
            ordered.push((
                item.span().map_or(usize::MAX, |s| s.start),
                key.to_owned(),
                item,
            ));
        }
    }
    ordered.sort_by_key(|(start, _, _)| *start);
    let mut script = String::new();
    for (_, key, item) in ordered {
        let raw = item.to_string();
        let value: Value = if item.is_table() {
            Value::Table(toml::from_str(&raw).map_err(|e| fail(format!("env.{key}: {e}")))?)
        } else {
            toml::from_str::<toml::Table>(&format!("value = {raw}"))
                .map_err(|e| fail(format!("env.{key}: {e}")))?
                .remove("value")
                .unwrap()
        };
        let expand = |text: &str| -> Result<String, ProviderError> {
            let text = text
                .replace("{{config_root}}", &root.to_string_lossy())
                .replace("{{ config_root }}", &root.to_string_lossy());
            no_template(&text, &format!("env.{key}"))?;
            Ok(text)
        };
        if key.starts_with("_.") {
            let values: Vec<_> = match &value {
                Value::String(s) => vec![s.as_str()],
                Value::Array(a) => a
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .ok_or_else(|| fail(format!("env.{key} must name paths")))
                    })
                    .collect::<Result<_, _>>()?,
                _ => return Err(fail(format!("unsupported env.{key}"))),
            };
            for text in values {
                let path = project_path(root, &expand(text)?)?;
                match key.as_str() {
                    "_.path" => script.push_str(&format!(
                        "export PATH={}:\"$PATH\"\n",
                        quote(&path.to_string_lossy())
                    )),
                    "_.source" => {
                        files.push(path.clone());
                        script.push_str(&format!(". {}\n", quote(&path.to_string_lossy())));
                    }
                    "_.file" => {
                        // mise reads JSON, YAML and TOML here as well. Parsing
                        // those as dotenv would change their meaning silently.
                        if path.extension().is_some_and(|e| {
                            ["json", "yaml", "yml", "toml"]
                                .contains(&e.to_string_lossy().to_ascii_lowercase().as_str())
                        }) {
                            return Err(fail(format!(
                                "env._.file {}: only dotenv files are supported",
                                path.display()
                            )));
                        }
                        files.push(path.clone());
                        // Missing inputs remain fingerprintable; resolution reports
                        // the missing dotenv before any mise subprocess.
                        if let Ok(body) = std::fs::read_to_string(&path) {
                            for line in body
                                .lines()
                                .map(str::trim)
                                .filter(|s| !s.is_empty() && !s.starts_with('#'))
                            {
                                let (name, value) = line
                                    .strip_prefix("export ")
                                    .unwrap_or(line)
                                    .split_once('=')
                                    .ok_or_else(|| {
                                        fail(format!(
                                            "{}: invalid dotenv assignment",
                                            path.display()
                                        ))
                                    })?;
                                let name = name.trim();
                                env_name(name)?;
                                let value = value.trim();
                                let value = if value.len() >= 2
                                    && ((value.starts_with('\'') && value.ends_with('\''))
                                        || (value.starts_with('"') && value.ends_with('"')))
                                {
                                    &value[1..value.len() - 1]
                                } else {
                                    value
                                };
                                if value.contains(['$', '`', '\\']) {
                                    return Err(fail(format!(
                                        "{}: dotenv expansion/escapes are not supported",
                                        path.display()
                                    )));
                                }
                                script.push_str(&format!("export {name}={}\n", quote(value)));
                            }
                        }
                    }
                    _ => return Err(fail(format!("unsupported env.{key}"))),
                }
            }
        } else {
            env_name(&key)?;
            let scalar = if let Value::Table(table) = &value {
                for field in table.keys() {
                    if !["value", "tools"].contains(&field.as_str()) {
                        return Err(fail(format!("unsupported env.{key}.{field}")));
                    }
                }
                if let Some(tools) = table.get("tools")
                    && tools.as_bool().is_none()
                {
                    return Err(fail(format!("env.{key}.tools must be boolean")));
                }
                table
                    .get("value")
                    .ok_or_else(|| fail(format!("env.{key}.value is missing")))?
            } else {
                &value
            };
            if scalar == &Value::Boolean(false) {
                script.push_str(&format!("unset {key}\n"));
            } else {
                let text = match scalar {
                    Value::String(s) => expand(s)?,
                    Value::Integer(n) => n.to_string(),
                    Value::Float(n) => n.to_string(),
                    Value::Boolean(b) => b.to_string(),
                    _ => return Err(fail(format!("unsupported env.{key}"))),
                };
                script.push_str(&format!("export {key}={}\n", quote(&text)));
            }
        }
    }
    Ok(script)
}

pub(crate) fn fingerprint(root: &Path) -> Result<String, ProviderError> {
    let input = Input::load_inner(root, true, false)?;
    let mut parts = vec![
        read(&input.definition)?,
        capture::optional_file_part(&root.join("mise.lock")),
        ADAPTER.as_bytes().to_vec(),
        target().as_bytes().to_vec(),
    ];
    for path in input.files {
        parts.push(capture::optional_file_part(&path));
    }
    let parts: Vec<_> = parts.iter().map(Vec::as_slice).collect();
    Ok(capture::fingerprint(&parts))
}

pub(crate) const PROVIDER: &str = "mise";

pub(crate) fn store_root() -> Result<PathBuf, ProviderError> {
    artifacts::root(PROVIDER)
}

pub(crate) fn lease() -> Result<artifacts::StoreLock, ProviderError> {
    artifacts::lease(PROVIDER)
}

fn isolated_env(object: &Path) -> Result<BTreeMap<String, String>, ProviderError> {
    let mut env = BTreeMap::from([("PATH".into(), capture::CANONICAL_PATH.into())]);
    for (key, path) in [
        ("HOME", "home"),
        ("MISE_DATA_DIR", "data"),
        // Both default from other roots: installs from the data directory,
        // system installs from a host path mise would otherwise treat as
        // already-installed tools living outside the object.
        ("MISE_INSTALLS_DIR", "data/installs"),
        ("MISE_SYSTEM_INSTALLS_DIR", "system-installs"),
        ("MISE_CONFIG_DIR", "config"),
        ("MISE_STATE_DIR", "state"),
        ("MISE_CACHE_DIR", "cache"),
        ("MISE_SYSTEM_CONFIG_DIR", "system-config"),
        ("MISE_SYSTEM_DATA_DIR", "system-data"),
        ("MISE_TMP_DIR", "tmp"),
    ] {
        let path = object.join(path);
        std::fs::create_dir_all(&path).map_err(|e| fail(e.to_string()))?;
        env.insert(key.into(), path.to_string_lossy().into_owned());
    }
    let empty = object.join("empty.toml");
    std::fs::write(&empty, b"").map_err(|e| fail(e.to_string()))?;
    for key in ["MISE_GLOBAL_CONFIG_FILE", "MISE_SYSTEM_CONFIG_FILE"] {
        env.insert(key.into(), empty.to_string_lossy().into_owned());
    }
    for (key, value) in [
        (
            "MISE_CEILING_PATHS",
            object.parent().unwrap().to_string_lossy().into_owned(),
        ),
        (
            "MISE_TRUSTED_CONFIG_PATHS",
            object.to_string_lossy().into_owned(),
        ),
        ("MISE_LOCKED", "1".into()),
        ("MISE_LOCKED_REQUIRE_CHECKSUM", "1".into()),
        ("MISE_USE_VERSIONS_HOST", "0".into()),
        ("MISE_COLOR", "0".into()),
    ] {
        env.insert(key.into(), value);
    }
    for key in ["HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY", "SSL_CERT_FILE"] {
        if let Ok(value) = std::env::var(key) {
            env.insert(key.into(), value);
        }
    }
    Ok(env)
}

fn run(
    binary: &Path,
    args: &[&str],
    cwd: &Path,
    env: &BTreeMap<String, String>,
) -> Result<Vec<u8>, ProviderError> {
    let output = Command::new(binary)
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(env)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| fail(format!("starting mise: {e}")))?;
    if !output.status.success() {
        // Do not forward arbitrary installer output: it can contain proxy URLs
        // and credentials. The user can diagnose the command with doctor.
        return Err(fail(format!(
            "mise {} failed ({}); check locked URLs/checksums, network access and `devcroft doctor`",
            args.join(" "),
            output.status
        )));
    }
    Ok(output.stdout)
}

fn version(text: &str) -> Result<(u32, u32, u32), ProviderError> {
    artifacts::release(PROVIDER, text)
}

fn installed_digest(root: &Path) -> Result<String, ProviderError> {
    artifacts::installed_digest(PROVIDER, root)
}

pub(crate) fn resolve(root: &Path, skip_hooks: bool) -> Result<Resolution, ProviderError> {
    let input = Input::load(root, skip_hooks)?;
    if target() == "unqualified" {
        return Err(fail(
            "runtime is qualified only on native Linux/macOS ARM64 (glibc on Linux)",
        ));
    }
    #[cfg(all(target_os = "linux", target_env = "musl"))]
    return Err(fail("musl runtime has not been qualified"));
    let binary = crate::paths::resolve_on_path("mise").ok_or(ProviderError::MissingBinary {
        provider: "mise",
        hint: "devcroft doctor",
    })?;
    for path in &input.files {
        if !path.is_file() {
            return Err(fail(format!(
                "activation input {} is missing",
                path.display()
            )));
        }
    }
    let _lease = lease()?;
    // The version invocation also gets an empty HOME and directory overrides.
    // It sees no project definition. Its directory is never a runtime grant.
    let version_dir = store_root()?.join(".version");
    let bootstrap_env = isolated_env(&version_dir)?;
    let version_output =
        String::from_utf8(run(&binary, &["--version"], &version_dir, &bootstrap_env)?)
            .map_err(|e| fail(e.to_string()))?;
    let installed_version = version(&version_output)?;
    if installed_version < version(QUALIFIED_VERSION)? {
        return Err(fail(format!(
            "requires mise >= {QUALIFIED_VERSION}; found {}",
            version_output.trim()
        )));
    }
    if let Some(min) = &input.min_version
        && installed_version < version(min)?
    {
        return Err(fail(format!("project requires mise {min}")));
    }
    if installed_version > version(QUALIFIED_VERSION)? {
        eprintln!(
            "devcroft: warning: mise {} is newer than qualified {QUALIFIED_VERSION}",
            version_output.split_whitespace().next().unwrap()
        );
    }
    let derived = toml::to_string(&BTreeMap::from([("tools", &input.tools)]))
        .map_err(|e| fail(e.to_string()))?;
    let mut hash = Sha256::new();
    for part in [
        ADAPTER.as_bytes(),
        derived.as_bytes(),
        &input.lock,
        target().as_bytes(),
        version_output.as_bytes(),
    ] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    let id: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let store = store_root()?;
    let object = store.join(&id);
    let _lock = artifacts::lock(&store.join(format!(".{id}.lock")), true)?;
    let receipt = object.join("complete.sha256");
    if !receipt.exists() && object.exists() {
        std::fs::remove_dir_all(&object).map_err(|e| fail(e.to_string()))?;
    }
    std::fs::create_dir_all(&object).map_err(|e| fail(e.to_string()))?;
    let env = isolated_env(&object)?;
    if !receipt.exists() {
        std::fs::write(object.join("mise.toml"), &derived).map_err(|e| fail(e.to_string()))?;
        std::fs::write(object.join("mise.lock"), &input.lock).map_err(|e| fail(e.to_string()))?;
        run(&binary, &["install", "--locked"], &object, &env)?;
        if read(&object.join("mise.lock"))? != input.lock {
            return Err(fail(
                "locked installation rewrote mise.lock; no object was published",
            ));
        }
        let digest = installed_digest(&object.join("data/installs"))?;
        let pending = object.join("complete.pending");
        std::fs::write(&pending, digest).map_err(|e| fail(e.to_string()))?;
        std::fs::rename(pending, &receipt).map_err(|e| fail(e.to_string()))?;
    } else if read(&receipt)? != installed_digest(&object.join("data/installs"))?.as_bytes() {
        return Err(fail(format!(
            "object {id} failed its content digest; remove its consumers and recreate it"
        )));
    }
    if read(&object.join("mise.toml"))? != derived.as_bytes()
        || read(&object.join("mise.lock"))? != input.lock
    {
        return Err(fail(
            "stored configuration differs from its recipe; refusing activation",
        ));
    }
    let activated: BTreeMap<String, String> =
        serde_json::from_slice(&run(&binary, &["env", "--json"], &object, &env)?)
            .map_err(|e| fail(format!("mise env JSON: {e}")))?;
    if read(&object.join("mise.lock"))? != input.lock {
        return Err(fail("environment capture rewrote mise.lock"));
    }
    let installs = object.join("data/installs");
    let mut resolved = BTreeMap::new();
    let paths: Vec<_> = activated
        .get("PATH")
        .into_iter()
        .flat_map(|v| v.split(':'))
        .filter(|p| {
            Path::new(p)
                .canonicalize()
                .is_ok_and(|p| p.starts_with(&installs))
        })
        .map(str::to_owned)
        .collect();
    if paths.is_empty() {
        return Err(fail("mise produced no installed tool PATH"));
    }
    // Nothing else from the installer process (HOME, proxy credentials, mise
    // directories, env snapshots) enters a runtime environment.
    resolved.insert("PATH".into(), paths.join(":"));
    let mut grants = vec![installs.to_string_lossy().into_owned()];
    let installed_shell = paths
        .iter()
        .map(|p| Path::new(p).join("sh"))
        .filter_map(|p| p.canonicalize().ok())
        .find(|p| p.starts_with(&installs) && p.is_file());
    let needs_host_shell = installed_shell.is_none();
    let shell = installed_shell.map(Ok).unwrap_or_else(|| {
        Path::new("/bin/sh")
            .canonicalize()
            .map_err(|e| fail(format!("host POSIX shell: {e}")))
    })?;
    let shell_dir = object.join("shell");
    std::fs::create_dir_all(&shell_dir).map_err(|e| fail(e.to_string()))?;
    if !shell_dir.join("sh").exists() {
        std::os::unix::fs::symlink(&shell, shell_dir.join("sh"))
            .map_err(|e| fail(e.to_string()))?;
    }
    grants.push(shell_dir.to_string_lossy().into_owned());
    #[cfg(target_os = "macos")]
    if needs_host_shell {
        grants.extend(["/bin".into(), "/var/select".into()]);
    }
    #[cfg(target_os = "linux")]
    {
        if needs_host_shell {
            grants.push(shell.to_string_lossy().into_owned());
        }
        if Path::new("/etc/ssl/openssl.cnf").is_file() {
            grants.push("/etc/ssl/openssl.cnf".into());
        }
    }
    resolved
        .get_mut("PATH")
        .unwrap()
        .push_str(&format!(":{}", shell_dir.display()));
    // Go's default caches are below the sandbox HOME; its toolchain chooser
    // must never fetch a different compiler during a supposedly locked session.
    resolved.insert("GOTOOLCHAIN".into(), "local".into());
    grants.extend(runtime_grants(&installs, &shell)?);
    grants.sort();
    grants.dedup();
    Ok(Resolution {
        env: resolved,
        unset: vec![],
        read_only_grants: grants,
        activation_script: (!input.activation.is_empty()).then_some(input.activation),
        services: ServiceSupport::Unsupported,
        ran_activation_hook: false,
    })
}

fn dependencies(path: &Path) -> Result<Vec<String>, ProviderError> {
    artifacts::header_dependencies(PROVIDER, path)
}

fn runtime_grants(installs: &Path, shell: &Path) -> Result<Vec<String>, ProviderError> {
    fn collect(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), ProviderError> {
        for entry in std::fs::read_dir(root).map_err(|e| fail(e.to_string()))? {
            let entry = entry.map_err(|e| fail(e.to_string()))?;
            let kind = entry.file_type().map_err(|e| fail(e.to_string()))?;
            if kind.is_dir() {
                collect(&entry.path(), out)?;
            } else if kind.is_file() {
                out.push(entry.path());
            }
        }
        Ok(())
    }
    let mut installed = vec![];
    collect(installs, &mut installed)?;
    let mut queue: Vec<_> = installed
        .iter()
        .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0))
        .cloned()
        .collect();
    #[cfg(target_os = "linux")]
    queue.push(shell.to_path_buf());
    #[cfg(target_os = "macos")]
    let _ = shell; // The OS shell/loader is the measured macOS runtime inventory.
    let env = Path::new("/usr/bin/env")
        .canonicalize()
        .map_err(|e| fail(e.to_string()))?;
    #[cfg(target_os = "linux")]
    queue.push(env.clone());
    let mut grants = vec![env.to_string_lossy().into_owned()];
    let mut visited = std::collections::BTreeSet::new();
    while let Some(binary) = queue.pop() {
        if !visited.insert(binary.clone()) {
            continue;
        }
        for needed in dependencies(&binary)? {
            let basename = Path::new(&needed)
                .file_name()
                .and_then(|p| p.to_str())
                .ok_or_else(|| {
                    fail(format!("{}: invalid dependency {needed}", binary.display()))
                })?;
            let local: Vec<_> = installed
                .iter()
                .filter(|p| p.file_name().is_some_and(|s| s == basename))
                .collect();
            if local.len() == 1 {
                queue.push(local[0].to_path_buf());
                continue;
            }
            #[cfg(target_os = "macos")]
            {
                if [
                    "/usr/lib/libSystem.B.dylib",
                    "/usr/lib/libc++.1.dylib",
                    "/usr/lib/libresolv.9.dylib",
                    "/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation",
                    "/System/Library/Frameworks/Security.framework/Versions/A/Security",
                ]
                .contains(&needed.as_str())
                {
                    // These OS libraries live in the dyld cache, not necessarily
                    // as separate files. The baseline already exposes that cache.
                    grants.push(if needed.starts_with("/usr/lib/") {
                        "/usr/lib".into()
                    } else {
                        needed.split("/Versions/").next().unwrap().into()
                    });
                    continue;
                }
            }
            #[cfg(target_os = "linux")]
            {
                let allowed = [
                    "ld-linux-aarch64.so.1",
                    "libc.so.6",
                    "libm.so.6",
                    "libdl.so.2",
                    "libpthread.so.0",
                    "libstdc++.so.6",
                    "libgcc_s.so.1",
                    "librt.so.1",
                ];
                if allowed.contains(&basename) {
                    let candidate = if needed.starts_with('/') {
                        PathBuf::from(&needed)
                    } else {
                        [
                            "/lib/aarch64-linux-gnu",
                            "/usr/lib/aarch64-linux-gnu",
                            "/lib",
                        ]
                        .iter()
                        .map(|p| Path::new(p).join(&needed))
                        .find(|p| p.is_file())
                        .unwrap_or_default()
                    };
                    if let Ok(real) = candidate.canonicalize() {
                        grants.push(real.to_string_lossy().into_owned());
                        queue.push(real);
                        continue;
                    }
                }
            }
            return Err(fail(format!(
                "{} needs unqualified library {needed}",
                binary.display()
            )));
        }
    }
    Ok(grants)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fixture(env: &str) -> Fixture {
        let path = std::env::temp_dir().join(format!(
            "devcroft-mise-input-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("mise.toml"),format!("[tools]\n'http:probe'={{version='1.0.0',url='https://example.com/probe.tar.gz'}}\n{env}")).unwrap();
        std::fs::write(path.join("mise.lock"),format!("lockfile_version=4\n[[tools.'http:probe']]\nversion='1.0.0'\nbackend='http:probe'\nspecifiers=['1.0.0']\n[tools.'http:probe'.'platforms.{}']\nurl='https://example.com/probe.tar.gz'\nchecksum='sha256:{}'\n",target(),"0".repeat(64))).unwrap();
        Fixture(path.canonicalize().unwrap())
    }
    #[test]
    fn directives_keep_source_order_and_never_run_while_parsing() {
        let f = fixture(
            "[env]\n_.path='bin'\nBEFORE='one'\n_.source='activation.sh'\nAFTER='two'\nREMOVED=false\n",
        );
        std::fs::write(
            f.0.join("activation.sh"),
            "echo forbidden > marker\nexport AFTER=source\n",
        )
        .unwrap();
        let input = Input::load(&f.0, false).unwrap();
        let script = &input.activation;
        assert!(script.find("export BEFORE").unwrap() < script.find(". '").unwrap());
        assert!(script.find(". '").unwrap() < script.find("export AFTER").unwrap());
        assert!(script.contains("unset REMOVED"));
        assert!(!f.0.join("marker").exists());
        assert!(script.contains(&format!("{}/bin", f.0.display())));
    }
    #[test]
    fn settings_false_and_hooks_are_not_silently_ignored() {
        for (text, key) in [
            ("[settings]\nnode.compile=false", "settings.node"),
            ("[hooks]\nenter='touch host'", "[hooks]"),
            ("[env]\nBAD='{{ exec(command=\"touch host\") }}'", "env.BAD"),
        ] {
            let f = fixture(text);
            let error = Input::load(&f.0, false)
                .err()
                .expect("declaration must be refused")
                .to_string();
            assert!(error.contains(key), "{error}");
        }
    }
    #[test]
    fn missing_scripts_and_locks_change_fingerprints() {
        let f = fixture("[env]\n_.source='activation.sh'\n_.file='.env'\n");
        std::fs::write(f.0.join("activation.sh"), "export X=one").unwrap();
        std::fs::write(f.0.join(".env"), "X=two\n").unwrap();
        let before = fingerprint(&f.0).unwrap();
        std::fs::remove_file(f.0.join("activation.sh")).unwrap();
        let after = fingerprint(&f.0).unwrap();
        assert_ne!(before, after);
        std::fs::remove_file(f.0.join("mise.lock")).unwrap();
        assert_ne!(after, fingerprint(&f.0).unwrap());
    }
    #[test]
    fn daemons_require_the_explicit_skip_context() {
        let f = fixture("[daemons]\npostgres='18'\n");
        assert!(
            Input::load(&f.0, false)
                .err()
                .unwrap()
                .to_string()
                .contains("[daemons]")
        );
        assert!(Input::load(&f.0, true).is_ok());
    }
    #[test]
    fn ambiguous_and_alternate_configuration_is_refused() {
        for extra in [
            ".mise.toml",
            "mise.local.toml",
            ".miserc.toml",
            ".config/mise/conf.d/extra.toml",
            ".config/mise/conf.d/extra/mise.toml",
            ".config/mise/config.toml",
            ".config/mise/mise.toml",
            ".config/mise.toml",
            ".mise/conf.d/extra.toml",
            ".mise/conf.d/extra/mise.toml",
            ".mise/config.toml",
            "mise/conf.d/extra.toml",
            "mise/conf.d/extra/mise.toml",
            "mise/config.toml",
            ".config/mise/config.local.toml",
            ".config/mise/mise.local.toml",
            ".config/mise.local.toml",
            ".config/miserc.toml",
            ".miserc.local.toml",
            ".tool-versions",
            ".mise/locks/dependency.toml",
            ".mise/config.production.local.toml",
            "mise.production.local.toml",
        ] {
            let f = fixture("");
            let path = f.0.join(extra);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "").unwrap();
            assert!(
                Input::load(&f.0, false)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains(extra)
            );
        }
    }
    #[test]
    fn unsupported_declarations_fail_by_name() {
        for table in [
            "hooks",
            "watch_files",
            "plugins",
            "alias",
            "tool_alias",
            "future",
        ] {
            let f = fixture(&format!("[{table}]\nx=false\n"));
            assert!(
                Input::load(&f.0, false)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains(table)
            );
        }
        for tool in [
            "node",
            "core:go",
            "aqua:junegunn/fzf",
            "github:a/b",
            "conda:python",
            "asdf:x",
        ] {
            let f = fixture("");
            let file = f.0.join("mise.toml");
            std::fs::write(&file, format!("[tools]\n'{tool}'='1'\n")).unwrap();
            assert!(
                Input::load(&f.0, false)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains(tool)
            );
        }
    }
    /// Task 2.2: the subprocess environment is an allowlist. Nothing ambient
    /// crosses except the four proxy/CA names, so a caller's `MISE_ENV`,
    /// `MISE_LOCKED_REQUIRE_CHECKSUM=0`, a token, or the real `HOME` (and the
    /// default-package files under it) cannot reach mise.
    #[test]
    fn the_installer_environment_is_built_not_inherited() {
        let f = fixture("");
        let object = f.0.join("object");
        let env = isolated_env(&object).unwrap();
        let forwarded = ["HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY", "SSL_CERT_FILE"];
        for key in env.keys() {
            assert!(
                key == "HOME"
                    || key == "PATH"
                    || key.starts_with("MISE_")
                    || forwarded.contains(&key.as_str()),
                "unexpected variable {key}"
            );
        }
        assert!(!env.contains_key("MISE_ENV"));
        assert_eq!(env["MISE_LOCKED"], "1");
        assert_eq!(env["MISE_LOCKED_REQUIRE_CHECKSUM"], "1");
        assert_eq!(env["PATH"], capture::CANONICAL_PATH);
        let real_home = std::env::var("HOME").unwrap_or_default();
        assert_ne!(env["HOME"], real_home);
        for key in [
            "HOME",
            "MISE_DATA_DIR",
            "MISE_INSTALLS_DIR",
            "MISE_SYSTEM_INSTALLS_DIR",
            "MISE_CONFIG_DIR",
            "MISE_STATE_DIR",
            "MISE_CACHE_DIR",
            "MISE_SYSTEM_CONFIG_DIR",
            "MISE_SYSTEM_DATA_DIR",
            "MISE_TMP_DIR",
            "MISE_GLOBAL_CONFIG_FILE",
            "MISE_SYSTEM_CONFIG_FILE",
            "MISE_TRUSTED_CONFIG_PATHS",
        ] {
            assert!(
                Path::new(&env[key]).starts_with(&object),
                "{key} = {}",
                env[key]
            );
        }
        assert_eq!(
            env["MISE_INSTALLS_DIR"],
            object.join("data/installs").to_string_lossy()
        );
        assert!(std::fs::read_dir(&env["HOME"]).unwrap().next().is_none());
    }
    /// Task 6.4: `{{config_root}}` and relative paths name the project, never
    /// the derived directory; an edited source file is a different input.
    #[test]
    fn config_root_is_the_project_and_an_edited_source_is_stale() {
        let f = fixture(
            "[env]\nCACHE='{{config_root}}/.cache'\nSPACED='{{ config_root }}/x'\n_.source='a.sh'\n",
        );
        std::fs::write(f.0.join("a.sh"), "export A=1\n").unwrap();
        let input = Input::load(&f.0, false).unwrap();
        let root = f.0.display().to_string();
        assert!(
            input
                .activation
                .contains(&format!("export CACHE='{root}/.cache'"))
        );
        assert!(
            input
                .activation
                .contains(&format!("export SPACED='{root}/x'"))
        );
        assert!(input.activation.contains(&format!(". '{root}/a.sh'")));
        let before = fingerprint(&f.0).unwrap();
        std::fs::write(f.0.join("a.sh"), "export A=2\n").unwrap();
        assert_ne!(before, fingerprint(&f.0).unwrap());
    }
    #[test]
    fn env_inputs_are_refused_when_they_leave_the_project_or_are_not_dotenv() {
        for (text, needle) in [
            ("[env]\n_.source='../outside.sh'\n", "escapes the project"),
            ("[env]\n_.path='/usr/bin'\n", "escapes the project"),
            ("[env]\n_.file='vars.json'\n", "only dotenv"),
            ("[env]\n_.file='vars.yaml'\n", "only dotenv"),
            ("[env]\n_.python.venv='.venv'\n", "env._.python"),
            ("[env]\nX={value='a',redact=true}\n", "env.X.redact"),
        ] {
            let f = fixture(text);
            let error = Input::load(&f.0, false).err().expect(text).to_string();
            assert!(error.contains(needle), "{text}: {error}");
        }
    }
    #[test]
    fn competing_manifests_are_named_together() {
        let f = fixture("");
        std::fs::copy(f.0.join("mise.toml"), f.0.join(".mise.toml")).unwrap();
        let error = Input::load(&f.0, false).err().unwrap().to_string();
        assert!(
            error.contains("mise.toml") && error.contains(".mise.toml"),
            "{error}"
        );
    }
    #[test]
    fn a_lock_without_this_platform_or_checksum_is_refused_before_mise_runs() {
        let f = fixture("");
        let p = f.0.join("mise.lock");
        let raw = std::fs::read_to_string(&p).unwrap();
        std::fs::write(
            &p,
            raw.replace(&format!("platforms.{}", target()), "platforms.other"),
        )
        .unwrap();
        let error = Input::load(&f.0, false).err().unwrap().to_string();
        assert!(error.contains("missing platform"), "{error}");
        let no_checksum: String = raw
            .lines()
            .filter(|l| !l.starts_with("checksum"))
            .map(|l| format!("{l}\n"))
            .collect();
        std::fs::write(&p, no_checksum).unwrap();
        let error = Input::load(&f.0, false).err().unwrap().to_string();
        assert!(error.contains("SHA-256"), "{error}");
        // No lock at all: refused with the corrective hint, never relocked.
        std::fs::remove_file(&p).unwrap();
        let error = Input::load(&f.0, false).err().unwrap().to_string();
        assert!(error.contains("has no mise.lock"), "{error}");
        assert!(error.contains("run `mise lock`"), "{error}");
        assert!(!p.exists(), "the refusal created a lock");
    }
    #[test]
    fn lock_checks_happen_without_a_mise_process() {
        let f = fixture("");
        let p = f.0.join("mise.lock");
        let raw = std::fs::read_to_string(&p).unwrap();
        for (from, to) in [
            ("sha256:", "sha512:"),
            ("backend='http:probe'", "backend='core:node'"),
            ("probe.tar.gz'", "different.tar.gz'"),
        ] {
            std::fs::write(&p, raw.replace(from, to)).unwrap();
            assert!(Input::load(&f.0, false).is_err());
        }
    }
}
