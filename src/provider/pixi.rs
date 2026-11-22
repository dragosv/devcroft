//! Locked, Conda-only pixi materialization (`add-pixi-provider`).
//!
//! Host-side pixi installs a byte copy of the project's manifest and lock into
//! an immutable object under `~/.local/share/devcroft-artifacts/pixi/`, under
//! an environment devcroft builds rather than inherits. Activation is emitted
//! as text (measured inert, Group 0), its staging paths are rewritten to the
//! project, and it runs once in confined provisioning.
//!
//! What Group 0 measured for pixi 0.84.2, and what follows from it:
//! - installing a PyPI package runs the environment's own interpreter on the
//!   host, so every `pypi:` lock entry is refused (owner decision: the first
//!   slice is Conda-only);
//! - the macOS binary consults the user's keychain whatever `HOME` says, so
//!   lock URLs are restricted to the qualified public hosts;
//! - pixi drops a missing activation script from the hook text, so the
//!   staging workspace carries a placeholder for each script that exists;
//! - an executable needing a host library outside the profile's inventory
//!   (tk's `wish` needing `libX11`) is named in one warning and not granted
//!   it (owner decision), rather than refusing every Python environment.
use super::{ProviderError, Resolution, ServiceSupport, artifacts, capture};
use serde_norway::Value as Yaml;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use toml::Value;

pub(crate) const PROVIDER: &str = "pixi";
pub(crate) const ADAPTER: &str = "pixi-conda-1";
pub(crate) const QUALIFIED_VERSION: &str = "0.84.2";
const LOCK_VERSION: i64 = 7;
/// Package hosts whose channels need no credential. Any other host is
/// refused before pixi runs, so no stored credential can be presented.
const PUBLIC_HOSTS: &[&str] = &["conda.anaconda.org", "repo.prefix.dev"];
/// `.pixi/config.toml` keys that change neither the bytes installed, nor what
/// executes, nor network authority (Group 0, from pixi's `Config` struct).
const ADMITTED_CONFIG: &[&str] = &[
    "concurrency",
    "pinning-strategy",
    "change-ps1",
    "default-channels",
    "exclude-newer",
];

fn fail(message: impl Into<String>) -> ProviderError {
    ProviderError::ResolutionFailed(format!("pixi: {}", message.into()))
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> ProviderError + '_ {
    move |e| fail(format!("{}: {e}", path.display()))
}

/// The conda subdir of this host, when its runtime has been qualified.
pub(crate) fn target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("osx-arm64"),
        ("linux", "aarch64") if cfg!(target_env = "gnu") => Some("linux-aarch64"),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    PixiToml,
    Pyproject,
}

/// The project's `[env.pixi]`, read from the same `devcroft.toml` `up` used.
fn selection(root: &Path) -> Result<(Option<String>, String), ProviderError> {
    let path = root.join(crate::config::MANIFEST_FILE_NAME);
    let text = std::fs::read_to_string(&path).map_err(io(&path))?;
    let (manifest, _) = crate::config::parse(&text).map_err(|e| fail(e.to_string()))?;
    let pixi = manifest.env.pixi.unwrap_or_default();
    Ok((
        pixi.manifest,
        pixi.environment.unwrap_or_else(|| "default".to_owned()),
    ))
}

fn is_pixi_pyproject(path: &Path) -> Result<bool, ProviderError> {
    let text = std::fs::read_to_string(path).map_err(io(path))?;
    let table: toml::Table =
        toml::from_str(&text).map_err(|e| fail(format!("{}: {e}", path.display())))?;
    Ok(table
        .get("tool")
        .and_then(|t| t.get("pixi"))
        .is_some_and(Value::is_table))
}

/// The selected workspace file, relative to the root, and its kind.
fn discover(root: &Path, selector: Option<&str>) -> Result<(String, Kind), ProviderError> {
    if let Some(selector) = selector {
        let path = root.join(selector);
        let meta = std::fs::symlink_metadata(&path).map_err(io(&path))?;
        let real = path.canonicalize().map_err(io(&path))?;
        if !meta.is_file() || !real.starts_with(root) {
            return Err(fail(format!(
                "env.pixi.manifest {selector} must be a regular file inside the project"
            )));
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        return if name == "pyproject.toml" {
            if !is_pixi_pyproject(&path)? {
                return Err(fail(format!(
                    "{selector} has no [tool.pixi] table, so it is not a pixi workspace"
                )));
            }
            Ok((selector.to_owned(), Kind::Pyproject))
        } else if name.ends_with(".toml") {
            Ok((selector.to_owned(), Kind::PixiToml))
        } else {
            Err(fail(format!("{selector} is not a pixi manifest")))
        };
    }
    let pixi = root.join("pixi.toml").is_file();
    let pyproject =
        root.join("pyproject.toml").is_file() && is_pixi_pyproject(&root.join("pyproject.toml"))?;
    match (pixi, pyproject) {
        (true, false) => Ok(("pixi.toml".into(), Kind::PixiToml)),
        (false, true) => Ok(("pyproject.toml".into(), Kind::Pyproject)),
        (true, true) => Err(fail(
            "both pixi.toml and pyproject.toml with [tool.pixi] exist; select one with \
             [env.pixi] manifest",
        )),
        (false, false) => Err(fail(
            "no pixi.toml, and no pyproject.toml with [tool.pixi]; run `pixi init` and `pixi lock`",
        )),
    }
}

fn relative_inside(root: &Path, text: &str, key: &str) -> Result<PathBuf, ProviderError> {
    let rel = Path::new(text);
    if text.is_empty()
        || rel.is_absolute()
        || rel.components().any(|c| {
            !matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        return Err(fail(format!(
            "{key}: {text} must be a relative path inside the project"
        )));
    }
    let path = root.join(rel);
    if let Ok(real) = path.canonicalize()
        && !real.starts_with(root)
    {
        return Err(fail(format!("{key}: {text} leaves the project")));
    }
    Ok(rel.to_path_buf())
}

/// Every activation script the manifest names, at workspace, target, feature
/// and feature-target level, in declaration order.
fn activation_scripts(root: &Path, table: &toml::Table) -> Result<Vec<PathBuf>, ProviderError> {
    fn scripts_of(
        root: &Path,
        holder: &toml::Table,
        at: &str,
        out: &mut Vec<PathBuf>,
    ) -> Result<(), ProviderError> {
        let Some(activation) = holder.get("activation") else {
            return Ok(());
        };
        let Some(scripts) = activation.get("scripts") else {
            return Ok(());
        };
        let list = scripts
            .as_array()
            .ok_or_else(|| fail(format!("{at}activation.scripts must be a list")))?;
        for script in list {
            let text = script
                .as_str()
                .ok_or_else(|| fail(format!("{at}activation.scripts must name files")))?;
            let path = relative_inside(root, text, &format!("{at}activation.scripts"))?;
            if !out.contains(&path) {
                out.push(path);
            }
        }
        Ok(())
    }
    fn with_targets(
        root: &Path,
        holder: &toml::Table,
        at: &str,
        out: &mut Vec<PathBuf>,
    ) -> Result<(), ProviderError> {
        scripts_of(root, holder, at, out)?;
        if let Some(targets) = holder.get("target").and_then(Value::as_table) {
            for (name, target) in targets {
                if let Some(target) = target.as_table() {
                    scripts_of(root, target, &format!("{at}target.{name}."), out)?;
                }
            }
        }
        Ok(())
    }
    let mut out = vec![];
    with_targets(root, table, "", &mut out)?;
    if let Some(features) = table.get("feature").and_then(Value::as_table) {
        for (name, feature) in features {
            if let Some(feature) = feature.as_table() {
                with_targets(root, feature, &format!("feature.{name}."), &mut out)?;
            }
        }
    }
    Ok(out)
}

/// The project's `.pixi/config.toml`, which pixi always merges: every key
/// outside the admitted set is refused by name.
fn check_project_config(bytes: &[u8], path: &Path) -> Result<(), ProviderError> {
    let text = std::str::from_utf8(bytes).map_err(|e| fail(format!("{}: {e}", path.display())))?;
    let table: toml::Table =
        toml::from_str(text).map_err(|e| fail(format!("{}: {e}", path.display())))?;
    for key in table.keys() {
        if !ADMITTED_CONFIG.contains(&key.as_str()) {
            return Err(fail(format!(
                ".pixi/config.toml: `{key}` is not supported (it can change what is \
                 installed, what runs, or which credentials are used)"
            )));
        }
    }
    Ok(())
}

pub(crate) struct Input {
    root: PathBuf,
    manifest: String,
    environment: String,
    manifest_bytes: Vec<u8>,
    lock: Option<Vec<u8>>,
    config: Option<Vec<u8>>,
    scripts: Vec<PathBuf>,
}

impl Input {
    /// Parse every input as data. `strict` additionally requires the lock and
    /// checks it; fingerprinting does not, since a missing lock is an input
    /// state, not an error.
    fn load(root: &Path, strict: bool) -> Result<Self, ProviderError> {
        let root = root.canonicalize().map_err(io(root))?;
        let (selector, environment) = selection(&root)?;
        let (manifest, kind) = discover(&root, selector.as_deref())?;
        let manifest_path = root.join(&manifest);
        let manifest_bytes = std::fs::read(&manifest_path).map_err(io(&manifest_path))?;
        let text = std::str::from_utf8(&manifest_bytes).map_err(|e| fail(e.to_string()))?;
        let parsed: toml::Table =
            toml::from_str(text).map_err(|e| fail(format!("{manifest}: {e}")))?;
        let table = match kind {
            Kind::PixiToml => parsed,
            Kind::Pyproject => parsed["tool"]["pixi"]
                .as_table()
                .cloned()
                .unwrap_or_default(),
        };
        let scripts = activation_scripts(&root, &table)?;
        let config_path = root.join(".pixi/config.toml");
        let config = match std::fs::read(&config_path) {
            Ok(bytes) => {
                check_project_config(&bytes, &config_path)?;
                Some(bytes)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(io(&config_path)(e)),
        };
        let lock_path = root.join("pixi.lock");
        let lock = match std::fs::read(&lock_path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(io(&lock_path)(e)),
        };
        let input = Self {
            root,
            manifest,
            environment,
            manifest_bytes,
            lock,
            config,
            scripts,
        };
        if strict {
            input.check_lock()?;
        }
        Ok(input)
    }

    fn lock(&self) -> Result<&[u8], ProviderError> {
        self.lock.as_deref().ok_or_else(|| {
            fail(format!(
                "{} has no pixi.lock; run `pixi lock` (pixi >= {QUALIFIED_VERSION}), review it and \
                 commit it — devcroft does not create or update the lock",
                self.manifest
            ))
        })
    }

    /// Everything about the lock that can be refused before pixi runs.
    fn check_lock(&self) -> Result<(), ProviderError> {
        let target = target().ok_or_else(|| {
            fail("this platform has not been qualified; native osx-arm64 and linux-aarch64 (glibc) are")
        })?;
        let lock: Yaml =
            serde_norway::from_slice(self.lock()?).map_err(|e| fail(format!("pixi.lock: {e}")))?;
        let version = lock.get("version").and_then(Yaml::as_i64).unwrap_or(0);
        if version != LOCK_VERSION {
            return Err(fail(format!(
                "pixi.lock format version {version} is not supported (qualified: {LOCK_VERSION}); \
                 re-lock with pixi {QUALIFIED_VERSION}"
            )));
        }
        check_virtual_packages(&lock, target)?;
        let env = &self.environment;
        let packages = lock
            .get("environments")
            .and_then(|e| e.get(env.as_str()))
            .ok_or_else(|| {
                fail(format!(
                    "pixi.lock has no environment `{env}`; run `pixi lock`"
                ))
            })?
            .get("packages")
            .and_then(|p| p.get(target))
            .and_then(Yaml::as_sequence)
            .ok_or_else(|| {
                fail(format!(
                    "pixi.lock has no {target} solution for environment `{env}`; add the platform \
                     and run `pixi lock`"
                ))
            })?;
        let hashed: BTreeSet<&str> = lock
            .get("packages")
            .and_then(Yaml::as_sequence)
            .into_iter()
            .flatten()
            .filter(|p| {
                p.get("sha256")
                    .and_then(Yaml::as_str)
                    .is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
            })
            .filter_map(|p| p.get("conda").and_then(Yaml::as_str))
            .collect();
        for entry in packages {
            if let Some(url) = entry.get("pypi").and_then(Yaml::as_str) {
                return Err(fail(format!(
                    "{}: PyPI packages are refused; installing one runs the environment's own \
                     interpreter on the host (measured, pixi {QUALIFIED_VERSION}). Conda-only \
                     environments are supported",
                    package_name(url)
                )));
            }
            if let Some(source) = entry.get("conda_source") {
                return Err(fail(format!(
                    "{}: source packages are refused; building one runs its build backend",
                    yaml_text(source)
                )));
            }
            let url = entry.get("conda").and_then(Yaml::as_str).ok_or_else(|| {
                fail(format!(
                    "pixi.lock: unsupported package entry {}",
                    yaml_text(entry)
                ))
            })?;
            check_conda_url(url)?;
            if !hashed.contains(url) {
                return Err(fail(format!(
                    "{}: the lock records no SHA-256",
                    package_name(url)
                )));
            }
        }
        Ok(())
    }
}

fn yaml_text(value: &Yaml) -> String {
    serde_norway::to_string(value)
        .map(|s| s.trim().replace('\n', " "))
        .unwrap_or_default()
}

fn package_name(url: &str) -> &str {
    url.rsplit('/').next().unwrap_or(url)
}

fn check_conda_url(url: &str) -> Result<(), ProviderError> {
    let name = package_name(url);
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(fail(format!(
            "{name}: only HTTPS packages are supported (paths and other schemes are refused)"
        )));
    };
    let host = rest.split('/').next().unwrap_or("");
    if host.contains('@') || !PUBLIC_HOSTS.contains(&host) {
        return Err(fail(format!(
            "{name}: channel host {host} is not supported; only {} are, since devcroft does not \
             support authenticated or private channels",
            PUBLIC_HOSTS.join(" and ")
        )));
    }
    if !(name.ends_with(".conda") || name.ends_with(".tar.bz2")) {
        return Err(fail(format!("{name}: not a binary conda package")));
    }
    Ok(())
}

/// The lock's recorded system requirements for this platform, against the host.
fn check_virtual_packages(lock: &Yaml, target: &str) -> Result<(), ProviderError> {
    let Some(platforms) = lock.get("platforms").and_then(Yaml::as_sequence) else {
        return Ok(());
    };
    let Some(platform) = platforms
        .iter()
        .find(|p| p.get("name").and_then(Yaml::as_str) == Some(target))
    else {
        return Ok(());
    };
    for requirement in platform
        .get("virtual-packages")
        .and_then(Yaml::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(Yaml::as_str)
    {
        let (name, version) = requirement.split_once('=').unwrap_or((requirement, ""));
        let version = version.split('=').next().unwrap_or("");
        let host = match name {
            "__unix" | "__archspec" => continue,
            "__glibc" => host_glibc(),
            "__linux" => host_kernel(),
            "__osx" => host_macos(),
            other => {
                return Err(fail(format!(
                    "pixi.lock requires `{other}`, which is not a qualified host requirement \
                     (devices and drivers are never granted because a lock mentions them)"
                )));
            }
        };
        let host = host.ok_or_else(|| fail(format!("cannot read the host's {name} version")))?;
        if numeric(&host) < numeric(version) {
            return Err(fail(format!(
                "pixi.lock requires {name} >= {version}; this host has {host}"
            )));
        }
    }
    Ok(())
}

fn numeric(text: &str) -> Vec<u64> {
    text.split(['.', '-', '+'])
        .map_while(|p| p.parse().ok())
        .collect()
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn host_glibc() -> Option<String> {
    // SAFETY: returns a pointer to a static NUL-terminated string.
    let p = unsafe { libc::gnu_get_libc_version() };
    (!p.is_null()).then(|| {
        unsafe { std::ffi::CStr::from_ptr(p) }
            .to_string_lossy()
            .into_owned()
    })
}
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn host_glibc() -> Option<String> {
    None
}

#[cfg(target_os = "linux")]
fn host_kernel() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|s| s.trim().to_owned())
}
#[cfg(not(target_os = "linux"))]
fn host_kernel() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn host_macos() -> Option<String> {
    let mut buf = [0u8; 64];
    let mut len = buf.len();
    let name = c"kern.osproductversion";
    // SAFETY: buf and len describe a writable buffer of len bytes.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then(|| {
        String::from_utf8_lossy(&buf[..len])
            .trim_end_matches('\0')
            .to_owned()
    })
}
#[cfg(not(target_os = "macos"))]
fn host_macos() -> Option<String> {
    None
}

pub(crate) fn fingerprint(root: &Path) -> Result<String, ProviderError> {
    let input = Input::load(root, false)?;
    let mut parts = vec![
        input.manifest.as_bytes().to_vec(),
        input.manifest_bytes.clone(),
        capture::optional_file_part(&input.root.join("pixi.lock")),
        capture::optional_file_part(&input.root.join(".pixi/config.toml")),
        input.environment.as_bytes().to_vec(),
        ADAPTER.as_bytes().to_vec(),
        target().unwrap_or("unqualified").as_bytes().to_vec(),
    ];
    for script in &input.scripts {
        parts.push(script.as_os_str().as_encoded_bytes().to_vec());
        parts.push(capture::optional_file_part(&input.root.join(script)));
    }
    let parts: Vec<_> = parts.iter().map(Vec::as_slice).collect();
    Ok(capture::fingerprint(&parts))
}

/// The environment host-side pixi runs under: built, never inherited.
fn isolated_env(store: &Path) -> Result<BTreeMap<String, String>, ProviderError> {
    let state = store.join(".state");
    let mut env = BTreeMap::from([
        ("PATH".to_owned(), capture::CANONICAL_PATH.to_owned()),
        ("PIXI_NO_CONFIG".to_owned(), "true".to_owned()),
        ("PIXI_COLOR".to_owned(), "never".to_owned()),
        ("PIXI_NO_PROGRESS".to_owned(), "true".to_owned()),
    ]);
    for (key, path) in [
        ("HOME", state.join("home")),
        ("TMPDIR", state.join("tmp")),
        ("PIXI_HOME", state.join("pixi-home")),
        ("PIXI_CACHE_DIR", store.join("cache")),
        ("RATTLER_CACHE_DIR", store.join("cache")),
    ] {
        std::fs::create_dir_all(&path).map_err(io(&path))?;
        env.insert(key.to_owned(), path.to_string_lossy().into_owned());
    }
    for key in ["HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY", "SSL_CERT_FILE"] {
        if let Ok(value) = std::env::var(key) {
            env.insert(key.to_owned(), value);
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
        .map_err(|e| fail(format!("starting pixi: {e}")))?;
    if !output.status.success() {
        // The last lines name what pixi refused (a stale lock, a download
        // failure); proxy credentials never reach pixi's own output here,
        // since only the forwarded names cross and pixi does not echo them.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<_> = stderr.lines().rev().take(6).collect();
        let tail: Vec<_> = tail.into_iter().rev().collect();
        return Err(fail(format!(
            "pixi {} failed ({}): {}",
            args.first().unwrap_or(&""),
            output.status,
            tail.join(" | ")
        )));
    }
    Ok(output.stdout)
}

fn pixi_binary() -> Result<PathBuf, ProviderError> {
    crate::paths::resolve_on_path("pixi").ok_or(ProviderError::MissingBinary {
        provider: "pixi",
        hint: "install pixi (https://pixi.sh) and run devcroft doctor",
    })
}

/// `pixi --version` against the qualified release; one warning when newer.
fn checked_version(binary: &Path, store: &Path) -> Result<String, ProviderError> {
    let env = isolated_env(store)?;
    let scratch = store.join(".state");
    let out = String::from_utf8(run(binary, &["--version"], &scratch, &env)?)
        .map_err(|e| fail(e.to_string()))?;
    let found = artifacts::release(PROVIDER, &out)?;
    let qualified = artifacts::release(PROVIDER, QUALIFIED_VERSION)?;
    if found < qualified {
        return Err(fail(format!(
            "requires pixi >= {QUALIFIED_VERSION}; found {}",
            out.trim()
        )));
    }
    if found > qualified {
        eprintln!(
            "devcroft: warning: {} is newer than the qualified pixi {QUALIFIED_VERSION}",
            out.trim()
        );
    }
    Ok(out.trim().to_owned())
}

/// Replace every staging path that is not inside the prefix with the
/// project root, and let the sandbox's own `PATH` follow the prefix's.
fn rewrite_hook(text: &str, object: &Path, root: &Path) -> Result<String, ProviderError> {
    let staging = object.to_string_lossy();
    let keep = format!("{staging}/.pixi/");
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(staging.as_ref()) {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        if tail.starts_with(&keep) {
            out.push_str(&staging);
        } else {
            out.push_str(&root.to_string_lossy());
        }
        rest = &tail[staging.len()..];
    }
    out.push_str(rest);
    let host_tail = format!(":{}\"", capture::CANONICAL_PATH);
    let out = out
        .lines()
        .map(|line| {
            if line.starts_with("export PATH=") {
                line.replace(&host_tail, ":${PATH}\"")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let residue: Vec<_> = out
        .lines()
        .filter(|l| l.contains(staging.as_ref()) && !l.contains(&keep))
        .collect();
    if !residue.is_empty() {
        return Err(fail(format!(
            "activation still names the staging directory after rewriting: {}",
            residue.join(" | ")
        )));
    }
    Ok(out)
}

/// Keep one placeholder in the staging workspace for each activation script
/// that exists in the project: pixi leaves a missing script out of the hook,
/// and the rewritten hook sources the project's real file.
fn sync_script_placeholders(input: &Input, object: &Path) -> Result<(), ProviderError> {
    let record = object.join(".devcroft-scripts");
    if let Ok(previous) = std::fs::read_to_string(&record) {
        for line in previous.lines().filter(|l| !l.is_empty()) {
            let _ = std::fs::remove_file(object.join(line));
        }
    }
    let mut placed = vec![];
    for script in &input.scripts {
        if input.root.join(script).is_file() {
            let at = object.join(script);
            if let Some(parent) = at.parent() {
                std::fs::create_dir_all(parent).map_err(io(parent))?;
            }
            std::fs::write(&at, b"").map_err(io(&at))?;
            placed.push(script.to_string_lossy().into_owned());
        }
    }
    std::fs::write(&record, placed.join("\n")).map_err(io(&record))
}

pub(crate) fn resolve(root: &Path) -> Result<Resolution, ProviderError> {
    let input = Input::load(root, true)?;
    let target = target().expect("checked by check_lock");
    let binary = pixi_binary()?;
    let store = artifacts::root(PROVIDER)?;
    let _lease = artifacts::lease(PROVIDER)?;
    let version = checked_version(&binary, &store)?;

    let lock = input.lock()?;
    let mut hash = Sha256::new();
    for part in [
        ADAPTER.as_bytes(),
        input.manifest.as_bytes(),
        &input.manifest_bytes,
        lock,
        input.config.as_deref().unwrap_or(b"\0no-config"),
        input.environment.as_bytes(),
        target.as_bytes(),
        version.as_bytes(),
    ] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    let id: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let object = store.join(&id);
    let _lock = artifacts::lock(&store.join(format!(".{id}.lock")), true)?;
    let receipt = object.join("complete.sha256");
    let prefix = object.join(".pixi/envs").join(&input.environment);
    let env = isolated_env(&store)?;
    let manifest_arg = object.join(&input.manifest);
    let manifest_arg = manifest_arg.to_string_lossy();
    if !receipt.exists() {
        if object.exists() {
            std::fs::remove_dir_all(&object).map_err(io(&object))?;
        }
        std::fs::create_dir_all(&object).map_err(io(&object))?;
        let staged_manifest = object.join(&input.manifest);
        if let Some(parent) = staged_manifest.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        std::fs::write(&staged_manifest, &input.manifest_bytes).map_err(io(&staged_manifest))?;
        std::fs::write(object.join("pixi.lock"), lock).map_err(io(&object))?;
        if let Some(config) = &input.config {
            let path = object.join(".pixi/config.toml");
            std::fs::create_dir_all(object.join(".pixi")).map_err(io(&object))?;
            std::fs::write(&path, config).map_err(io(&path))?;
        }
        run(
            &binary,
            &[
                "install",
                "--locked",
                "--environment",
                &input.environment,
                "--manifest-path",
                &manifest_arg,
            ],
            &object,
            &env,
        )?;
        if std::fs::read(object.join("pixi.lock")).map_err(io(&object))? != lock
            || std::fs::read(&staged_manifest).map_err(io(&object))? != input.manifest_bytes
        {
            return Err(fail(
                "locked installation rewrote the manifest or lock; no object was published",
            ));
        }
        let digest = artifacts::installed_digest(PROVIDER, &prefix)?;
        let pending = object.join("complete.pending");
        std::fs::write(&pending, digest).map_err(io(&pending))?;
        std::fs::rename(&pending, &receipt).map_err(io(&receipt))?;
    } else if std::fs::read(&receipt).map_err(io(&receipt))?
        != artifacts::installed_digest(PROVIDER, &prefix)?.as_bytes()
    {
        return Err(fail(format!(
            "object {id} failed its content digest; remove its consumers and recreate it"
        )));
    }

    sync_script_placeholders(&input, &object)?;
    let hook = String::from_utf8(run(
        &binary,
        &[
            "shell-hook",
            "--shell",
            "bash",
            "--as-is",
            "--no-completions",
            "--change-ps1",
            "false",
            "--environment",
            &input.environment,
            "--manifest-path",
            &manifest_arg,
        ],
        &object,
        &env,
    )?)
    .map_err(|e| fail(e.to_string()))?;
    let hook = rewrite_hook(&hook, &object, &input.root)?;
    let activation = format!(
        "cd {}\n{hook}\nunset PIXI_EXE\n",
        shell_quote(&input.root.to_string_lossy())
    );

    let bin = prefix.join("bin");
    let mut grants = vec![prefix.to_string_lossy().into_owned()];
    let prefix_shell = ["sh", "bash"]
        .iter()
        .map(|s| bin.join(s))
        .filter_map(|p| p.canonicalize().ok())
        .find(|p| p.starts_with(&prefix) && p.is_file());
    let shell = match &prefix_shell {
        Some(shell) => shell.clone(),
        None => Path::new("/bin/sh")
            .canonicalize()
            .map_err(|e| fail(format!("host POSIX shell: {e}")))?,
    };
    let shell_dir = object.join("shell");
    std::fs::create_dir_all(&shell_dir).map_err(io(&shell_dir))?;
    let link = shell_dir.join("sh");
    if std::fs::read_link(&link).ok().as_deref() != Some(shell.as_path()) {
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&shell, &link).map_err(io(&link))?;
    }
    grants.push(shell_dir.to_string_lossy().into_owned());
    if prefix_shell.is_none() {
        #[cfg(target_os = "macos")]
        grants.extend(["/bin".to_owned(), "/var/select".to_owned()]);
        #[cfg(target_os = "linux")]
        grants.push(shell.to_string_lossy().into_owned());
    }
    let (runtime, unmet) = runtime_grants(&prefix, &shell)?;
    grants.extend(runtime);
    grants.sort();
    grants.dedup();
    if !unmet.is_empty() {
        eprintln!(
            "devcroft: warning: host libraries outside the qualified inventory are not granted, \
             so loading these fails inside the sandbox: {}",
            describe_unmet(&unmet)
        );
    }
    let env = BTreeMap::from([(
        "PATH".to_owned(),
        format!("{}:{}", bin.display(), shell_dir.display()),
    )]);
    Ok(Resolution {
        env,
        unset: vec![],
        read_only_grants: grants,
        activation_script: Some(activation),
        services: ServiceSupport::Unsupported,
        ran_activation_hook: false,
    })
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// `lib/libtk8.6.dylib needs AppKit, Carbon; bin/x needs libX11.so.6`.
fn describe_unmet(unmet: &Unmet) -> String {
    let mut by_exe: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (exe, lib) in unmet {
        let short = lib
            .strip_prefix("/System/Library/Frameworks/")
            .and_then(|f| f.split(".framework/").next())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                Path::new(lib)
                    .file_name()
                    .map_or_else(|| lib.clone(), |n| n.to_string_lossy().into_owned())
            });
        by_exe.entry(exe).or_default().push(short);
    }
    by_exe
        .into_iter()
        .map(|(exe, libs)| format!("{exe} needs {}", libs.join(", ")))
        .collect::<Vec<_>>()
        .join("; ")
}

/// `(executable, library)` pairs the inventory does not carry.
type Unmet = BTreeSet<(String, String)>;

/// The host libraries the prefix's `bin/` executables need, read from their
/// headers: granted when the profile's inventory carries them, otherwise
/// returned as `(executable, library)` for one warning.
fn runtime_grants(prefix: &Path, shell: &Path) -> Result<(Vec<String>, Unmet), ProviderError> {
    // Every file in the prefix by name: a library found through an rpath
    // (perl's `@rpath/libperl.dylib` lives under `lib/perl5/.../CORE`) is
    // inside the prefix wherever it sits, so it needs no host grant.
    let mut by_name: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let mut stack = vec![prefix.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)
            .map_err(io(&dir))?
            .filter_map(Result::ok)
        {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(entry.path());
            } else {
                by_name
                    .entry(entry.file_name().to_string_lossy().into_owned())
                    .or_default()
                    .push(entry.path());
            }
        }
    }
    let lib = prefix.join("lib");
    let mut queue: Vec<PathBuf> = std::fs::read_dir(prefix.join("bin"))
        .map_err(io(prefix))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter_map(|p| p.canonicalize().ok())
        .filter(|p| p.starts_with(prefix) && p.is_file())
        .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0))
        .collect();
    // A host shell's own needs are read on Linux, where it is granted as a
    // file. On macOS it is a universal binary under the granted `/bin`, and
    // its libraries live in the dyld cache.
    if cfg!(target_os = "linux") && !shell.starts_with(prefix) {
        queue.push(shell.to_path_buf());
    }
    let mut grants = BTreeSet::new();
    let mut unmet = BTreeSet::new();
    let mut seen = BTreeSet::new();
    while let Some(binary) = queue.pop() {
        if !seen.insert(binary.clone()) {
            continue;
        }
        for needed in artifacts::header_dependencies(PROVIDER, &binary)? {
            // Inside the prefix: @rpath/@loader_path names and bare sonames.
            let base = Path::new(&needed)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let relative = needed.starts_with('@') || !needed.starts_with('/');
            if relative && !base.is_empty() {
                let mut candidates = by_name.get(&base).cloned().unwrap_or_default();
                // The conventional location first, when several share a name.
                candidates.sort_by_key(|p| !p.starts_with(&lib));
                if let Some(real) = candidates
                    .iter()
                    .filter_map(|p| p.canonicalize().ok())
                    .find(|p| p.starts_with(prefix))
                {
                    queue.push(real);
                    continue;
                }
            }
            if needed.starts_with(&*prefix.to_string_lossy()) {
                if let Ok(real) = Path::new(&needed).canonicalize() {
                    queue.push(real);
                }
                continue;
            }
            match inventory(&needed, &base) {
                Some((grant, follow)) => {
                    grants.extend(grant);
                    if let Some(follow) = follow {
                        queue.push(follow);
                    }
                }
                None => {
                    let exe = binary
                        .strip_prefix(prefix)
                        .unwrap_or(&binary)
                        .to_string_lossy()
                        .into_owned();
                    unmet.insert((exe, needed.clone()));
                }
            }
        }
    }
    Ok((grants.into_iter().collect(), unmet))
}

/// The qualified host runtime: the grant a library needs (none when the
/// baseline already covers it), and the host file to read for its own needs;
/// `None` when the profile does not carry it.
type Covered = (Option<String>, Option<PathBuf>);

#[cfg(target_os = "linux")]
fn inventory(needed: &str, base: &str) -> Option<Covered> {
    const GLIBC: &[&str] = &[
        "ld-linux-aarch64.so.1",
        "libc.so.6",
        "libm.so.6",
        "libdl.so.2",
        "libpthread.so.0",
        "libutil.so.1",
        "librt.so.1",
        "libresolv.so.2",
        // The host shells' own needs (bash).
        "libtinfo.so.6",
    ];
    if !GLIBC.contains(&base) {
        return None;
    }
    let candidate = if needed.starts_with('/') {
        PathBuf::from(needed)
    } else {
        [
            "/lib/aarch64-linux-gnu",
            "/usr/lib/aarch64-linux-gnu",
            "/lib",
            "/usr/lib",
        ]
        .iter()
        .map(|d| Path::new(d).join(base))
        .find(|p| p.is_file())?
    };
    let real = candidate.canonicalize().ok()?;
    Some((Some(real.to_string_lossy().into_owned()), Some(real)))
}

/// macOS: system libraries and frameworks live in the dyld shared cache, and
/// the baseline already grants `/usr/lib` and `/System/Library` (measured:
/// `policy --render` attributes both to `baseline`). Nothing to add, nothing
/// to warn about; anything else outside the prefix is not carried.
#[cfg(target_os = "macos")]
fn inventory(needed: &str, _base: &str) -> Option<Covered> {
    (needed.starts_with("/usr/lib/") || needed.starts_with("/System/Library/"))
        .then_some((None, None))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn inventory(_needed: &str, _base: &str) -> Option<Covered> {
    None
}

/// Read-only diagnostics for `doctor`: installs nothing, runs only
/// `pixi --version` under the isolated environment.
pub fn diagnose(root: &Path) -> Result<String, ProviderError> {
    let input = Input::load(root, true)?;
    let binary = pixi_binary()?;
    let store = artifacts::root(PROVIDER)?;
    let version = checked_version(&binary, &store)?;
    fn size(path: &Path) -> u64 {
        std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| match e.file_type() {
                Ok(t) if t.is_dir() => size(&e.path()),
                Ok(t) if t.is_file() => e.metadata().map_or(0, |m| m.len()),
                _ => 0,
            })
            .sum()
    }
    Ok(format!(
        "{version}; {} environment `{}` on {}, Conda-only; store {} bytes; tasks are not \
         runnable by name in the sandbox (pixi is not granted)",
        input.manifest,
        input.environment,
        target().unwrap_or("unqualified"),
        size(&store)
    ))
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

    const MANIFEST: &str = "[workspace]\nname = \"t\"\nchannels = [\"conda-forge\"]\nplatforms = [\"osx-arm64\", \"linux-aarch64\"]\n\n[dependencies]\nzlib = \"*\"\n\n[activation]\nscripts = [\"a.sh\"]\n\n[target.linux-aarch64.activation]\nscripts = [\"t.sh\"]\n\n[feature.f.activation]\nscripts = [\"f.sh\"]\n";

    fn lock_with(entries: &str) -> String {
        let t = target().unwrap_or("osx-arm64");
        format!(
            "version: 7\nplatforms:\n- name: {t}\n  virtual-packages:\n  - __unix=0\nenvironments:\n  default:\n    channels:\n    - url: https://conda.anaconda.org/conda-forge/\n    packages:\n      {t}:\n{entries}packages:\n- conda: https://conda.anaconda.org/conda-forge/{t}/zlib-1.3.2-h0_0.conda\n  sha256: {}\n  md5: {}\n",
            "ab".repeat(32),
            "cd".repeat(16)
        )
    }

    fn zlib_entry() -> String {
        let t = target().unwrap_or("osx-arm64");
        format!("      - conda: https://conda.anaconda.org/conda-forge/{t}/zlib-1.3.2-h0_0.conda\n")
    }

    fn fixture(devcroft_extra: &str) -> Fixture {
        let path = std::env::temp_dir().join(format!(
            "devcroft-pixi-input-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("devcroft.toml"),
            format!("[sandbox]\nname = \"t\"\n[env]\nprovider = \"pixi\"\n{devcroft_extra}"),
        )
        .unwrap();
        std::fs::write(path.join("pixi.toml"), MANIFEST).unwrap();
        std::fs::write(path.join("pixi.lock"), lock_with(&zlib_entry())).unwrap();
        Fixture(path.canonicalize().unwrap())
    }

    fn refusal(f: &Fixture) -> String {
        match Input::load(&f.0, true) {
            Ok(_) => panic!("expected a refusal"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn a_conda_only_lock_on_a_public_host_is_accepted() {
        if target().is_none() {
            return;
        }
        let f = fixture("");
        let input = Input::load(&f.0, true).unwrap();
        assert_eq!(input.environment, "default");
        assert_eq!(input.manifest, "pixi.toml");
        let scripts: Vec<_> = input
            .scripts
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(scripts, ["a.sh", "t.sh", "f.sh"]);
    }

    #[test]
    fn pypi_source_private_hosts_and_unknown_versions_are_refused_before_pixi_runs() {
        if target().is_none() {
            return;
        }
        let t = target().unwrap();
        for (entries, needle) in [
            (
                format!(
                    "{}      - pypi: https://files.pythonhosted.org/packages/x/six-1.16.0-py2.py3-none-any.whl\n",
                    zlib_entry()
                ),
                "six-1.16.0-py2.py3-none-any.whl: PyPI packages are refused",
            ),
            (
                format!("{}      - conda_source: mypkg[abc] @ .\n", zlib_entry()),
                "source packages are refused",
            ),
            (
                format!("      - conda: https://private.example.com/c/{t}/zlib-1.3.2-h0_0.conda\n"),
                "channel host private.example.com is not supported",
            ),
            (
                format!(
                    "      - conda: https://user:pw@conda.anaconda.org/c/{t}/zlib-1.3.2-h0_0.conda\n"
                ),
                "is not supported",
            ),
            (
                "      - conda: ./local/zlib-1.3.2-h0_0.conda\n".to_owned(),
                "only HTTPS packages are supported",
            ),
        ] {
            let f = fixture("");
            std::fs::write(f.0.join("pixi.lock"), lock_with(&entries)).unwrap();
            let error = refusal(&f);
            assert!(error.contains(needle), "{needle}: {error}");
        }
        let f = fixture("");
        let raw = std::fs::read_to_string(f.0.join("pixi.lock")).unwrap();
        std::fs::write(
            f.0.join("pixi.lock"),
            raw.replace("version: 7", "version: 6"),
        )
        .unwrap();
        assert!(refusal(&f).contains("format version 6"));
        std::fs::remove_file(f.0.join("pixi.lock")).unwrap();
        let error = refusal(&f);
        assert!(
            error.contains("has no pixi.lock") && error.contains("pixi lock"),
            "{error}"
        );
    }

    #[test]
    fn a_lock_without_the_selected_environment_or_platform_is_refused() {
        if target().is_none() {
            return;
        }
        let f = fixture("[env.pixi]\nenvironment = \"dev\"\n");
        assert!(refusal(&f).contains("no environment `dev`"));
        let f = fixture("");
        let raw = std::fs::read_to_string(f.0.join("pixi.lock")).unwrap();
        let t = target().unwrap();
        std::fs::write(
            f.0.join("pixi.lock"),
            raw.replacen(&format!("      {t}:"), "      win-64:", 1),
        )
        .unwrap();
        assert!(refusal(&f).contains(&format!("no {t} solution")));
    }

    #[test]
    fn unadmitted_project_config_and_device_requirements_are_refused_by_name() {
        if target().is_none() {
            return;
        }
        for (config, key) in [
            (
                "run-post-link-scripts = \"insecure\"\n",
                "run-post-link-scripts",
            ),
            ("tls-no-verify = true\n", "tls-no-verify"),
            ("detached-environments = true\n", "detached-environments"),
            (
                "[mirrors]\n\"https://conda.anaconda.org\" = [\"https://m\"]\n",
                "mirrors",
            ),
        ] {
            let f = fixture("");
            std::fs::create_dir(f.0.join(".pixi")).unwrap();
            std::fs::write(f.0.join(".pixi/config.toml"), config).unwrap();
            let error = refusal(&f);
            assert!(error.contains(&format!("`{key}`")), "{key}: {error}");
        }
        let f = fixture("");
        std::fs::create_dir(f.0.join(".pixi")).unwrap();
        std::fs::write(f.0.join(".pixi/config.toml"), "concurrency.downloads = 4\n").unwrap();
        assert!(Input::load(&f.0, true).is_ok());
        let f = fixture("");
        let raw = std::fs::read_to_string(f.0.join("pixi.lock")).unwrap();
        std::fs::write(
            f.0.join("pixi.lock"),
            raw.replace("  - __unix=0", "  - __cuda=12.0"),
        )
        .unwrap();
        assert!(refusal(&f).contains("`__cuda`"));
        let f = fixture("");
        let req = if cfg!(target_os = "macos") {
            "__osx=99.0"
        } else {
            "__glibc=99.0"
        };
        let raw = std::fs::read_to_string(f.0.join("pixi.lock")).unwrap();
        std::fs::write(
            f.0.join("pixi.lock"),
            raw.replace("  - __unix=0", &format!("  - {req}")),
        )
        .unwrap();
        assert!(refusal(&f).contains(">= 99.0"));
    }

    #[test]
    fn workspace_discovery_and_selectors() {
        let f = fixture("");
        std::fs::write(
            f.0.join("pyproject.toml"),
            "[project]\nname = \"x\"\n[tool.pixi.workspace]\nchannels = []\n",
        )
        .unwrap();
        assert!(
            Input::load(&f.0, false)
                .err()
                .unwrap()
                .to_string()
                .contains("both pixi.toml and pyproject.toml")
        );
        let f = fixture("[env.pixi]\nmanifest = \"pyproject.toml\"\n");
        std::fs::write(f.0.join("pyproject.toml"), "[project]\nname = \"x\"\n").unwrap();
        assert!(
            Input::load(&f.0, false)
                .err()
                .unwrap()
                .to_string()
                .contains("no [tool.pixi]")
        );
        let f = fixture("[env.pixi]\nmanifest = \"pixi.toml\"\n");
        std::fs::write(
            f.0.join("pyproject.toml"),
            "[project]\nname = \"x\"\n[tool.pixi.workspace]\n",
        )
        .unwrap();
        assert_eq!(Input::load(&f.0, false).unwrap().manifest, "pixi.toml");
        let f = fixture("");
        std::fs::remove_file(f.0.join("pixi.toml")).unwrap();
        std::fs::write(f.0.join("pyproject.toml"), "[project]\nname = \"x\"\n[tool.pixi.workspace]\n[tool.pixi.activation]\nscripts = [\"p.sh\"]\n").unwrap();
        let input = Input::load(&f.0, false).unwrap();
        assert_eq!(input.manifest, "pyproject.toml");
        assert_eq!(input.scripts, [PathBuf::from("p.sh")]);
        let f = fixture("");
        std::fs::write(
            f.0.join("pixi.toml"),
            MANIFEST.replace("\"a.sh\"", "\"../out.sh\""),
        )
        .unwrap();
        assert!(
            Input::load(&f.0, false)
                .err()
                .unwrap()
                .to_string()
                .contains("relative path inside the project")
        );
    }

    #[test]
    fn fingerprint_covers_scripts_config_environment_and_a_missing_lock() {
        let f = fixture("");
        std::fs::write(f.0.join("a.sh"), "export A=1\n").unwrap();
        let base = fingerprint(&f.0).unwrap();
        std::fs::write(f.0.join("a.sh"), "export A=2\n").unwrap();
        let edited = fingerprint(&f.0).unwrap();
        assert_ne!(base, edited);
        std::fs::remove_file(f.0.join("a.sh")).unwrap();
        let deleted = fingerprint(&f.0).unwrap();
        assert_ne!(edited, deleted);
        std::fs::create_dir(f.0.join(".pixi")).unwrap();
        std::fs::write(f.0.join(".pixi/config.toml"), "concurrency.downloads = 2\n").unwrap();
        let configured = fingerprint(&f.0).unwrap();
        assert_ne!(deleted, configured);
        std::fs::remove_file(f.0.join("pixi.lock")).unwrap();
        assert_ne!(configured, fingerprint(&f.0).unwrap());
        let g = fixture("[env.pixi]\nenvironment = \"dev\"\n");
        let h = fixture("");
        assert_ne!(fingerprint(&g.0).unwrap(), fingerprint(&h.0).unwrap());
    }

    /// Design P4 against the hook shape Group 0 recorded.
    #[test]
    fn the_hook_rewrite_moves_project_paths_and_keeps_the_prefix() {
        let object = Path::new("/s/pixi/aaaa");
        let root = Path::new("/home/u/proj");
        let hook = format!(
            "export PATH=\"/s/pixi/aaaa/.pixi/envs/default/bin:{}\"\n\
             export CONDA_PREFIX=/s/pixi/aaaa/.pixi/envs/default\n\
             export PIXI_PROJECT_ROOT=/s/pixi/aaaa\n\
             export PIXI_PROJECT_MANIFEST=/s/pixi/aaaa/pixi.toml\n\
             . /s/pixi/aaaa/.pixi/envs/default/etc/conda/activate.d/x.sh\n\
             . /s/pixi/aaaa/scripts/a.sh\n",
            capture::CANONICAL_PATH
        );
        let out = rewrite_hook(&hook, object, root).unwrap();
        assert!(
            out.contains("export PATH=\"/s/pixi/aaaa/.pixi/envs/default/bin:${PATH}\""),
            "{out}"
        );
        assert!(out.contains("export CONDA_PREFIX=/s/pixi/aaaa/.pixi/envs/default"));
        assert!(out.contains("export PIXI_PROJECT_ROOT=/home/u/proj\n"));
        assert!(out.contains("export PIXI_PROJECT_MANIFEST=/home/u/proj/pixi.toml"));
        assert!(out.contains(". /s/pixi/aaaa/.pixi/envs/default/etc/conda/activate.d/x.sh"));
        assert!(out.contains(". /home/u/proj/scripts/a.sh"));
    }

    #[test]
    fn unmet_libraries_are_grouped_per_executable() {
        let unmet = Unmet::from([
            (
                "lib/libtk8.6.dylib".to_owned(),
                "/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit".to_owned(),
            ),
            (
                "lib/libtk8.6.dylib".to_owned(),
                "/System/Library/Frameworks/Carbon.framework/Versions/A/Carbon".to_owned(),
            ),
            ("bin/wish8.6".to_owned(), "libX11.so.6".to_owned()),
        ]);
        assert_eq!(
            describe_unmet(&unmet),
            "bin/wish8.6 needs libX11.so.6; lib/libtk8.6.dylib needs AppKit, Carbon"
        );
    }

    #[test]
    fn isolated_env_is_an_allowlist() {
        let f = fixture("");
        let env = isolated_env(&f.0.join("store")).unwrap();
        let forwarded = ["HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY", "SSL_CERT_FILE"];
        for key in env.keys() {
            assert!(
                ["PATH", "HOME", "TMPDIR"].contains(&key.as_str())
                    || key.starts_with("PIXI_")
                    || key == "RATTLER_CACHE_DIR"
                    || forwarded.contains(&key.as_str()),
                "unexpected {key}"
            );
        }
        assert_eq!(env["PIXI_NO_CONFIG"], "true");
        assert!(!env.contains_key("PIXI_ENVIRONMENT_NAME"));
        assert_ne!(env["HOME"], std::env::var("HOME").unwrap_or_default());
        assert!(env["HOME"].starts_with(&*f.0.join("store").to_string_lossy()));
    }
}
