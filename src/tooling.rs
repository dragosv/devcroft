//! The `tooling` capability (`add-agent-workload`): a second environment,
//! the user's own, whose `PATH` entries reach a sandbox after the project's.
//!
//! **Only `PATH`, never the layer's environment** (design.md Decision 4). A
//! nix, devenv or flox environment's diff carries `CC`, `NIX_CFLAGS_COMPILE`,
//! `PKG_CONFIG_PATH` and more; laid over the project's, the user's agent
//! would change the project's build. Measured (2026-10-29): eleven coding
//! agents, moved out of a project into a separate layer, start and reach
//! their APIs with nothing from the layer but its `bin/` on `PATH`.
//!
//! **Declared by the user, never by the project** (Decision 1): the user
//! configuration's `[tools]` (`config::load_user_config`) or `up --tools`.
//! `devcroft.toml` refuses the section.
//!
//! **Closure tier only** (Decision 5), whatever the project's provider: an
//! artifact-tier layer is host-linked by definition, which is the host
//! passthrough this change refuses, one step removed.
//!
//! Resolution reuses each provider's own `resolve`, which is already the
//! hook-free route for all four closure providers (`fix-provisioning-hooks`,
//! `sandbox-provisioning` P2d). Of what it returns, the activation script and
//! the services are dropped, unread: the layer's hook never runs, inside or
//! out, and its services are not the project's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config::UserConfig;
use crate::provider::{ProviderEntry, ProviderError, ProviderKind, Tier};

/// Where the layer was named. `status` compares a recorded layer against the
/// user configuration only when it came from there: a layer passed with
/// `--tools` is not stale because the configuration names another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Config,
    Flag,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Config => "config",
            Source::Flag => "flag",
        }
    }
}

/// A selected tooling layer, before resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    /// Canonical, so a record of it compares equal however it was spelled.
    pub path: PathBuf,
    pub provider: ProviderKind,
    pub source: Source,
}

/// What a resolved layer contributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The layer's `PATH` entries inside its own store grants, in its
    /// provider's order, with duplicates removed.
    pub path_entries: Vec<String>,
    /// Its store root(s), granted read-only with origin `tools:<provider>`.
    pub grants: Vec<String>,
    /// The provider's fingerprint of the layer's definition and lock.
    pub fingerprint: String,
}

/// The layer to use: `--tools`, else the user configuration's `[tools]`,
/// else none. Detects the provider where none is named, and refuses one
/// outside the closure tier before anything resolves.
pub fn select(flag: Option<&Path>, config: &UserConfig) -> Result<Option<Layer>, ProviderError> {
    let (path, declared, source) = match (flag, &config.tools) {
        (Some(path), _) => (path.to_path_buf(), None, Source::Flag),
        (None, Some(decl)) => (decl.path.clone(), decl.provider.clone(), Source::Config),
        (None, None) => return Ok(None),
    };
    let path = std::fs::canonicalize(&path)
        .map_err(|e| ProviderError::ToolsLayer(format!("{}: {e}", path.display())))?;
    if !path.is_dir() {
        return Err(ProviderError::ToolsLayer(format!(
            "{}: not a directory; name the directory that holds the environment",
            path.display()
        )));
    }
    let provider = match declared {
        Some(name) => ProviderKind::from_name(&name)?,
        None => detect_provider(&path)?,
    };
    if provider.tier() != Tier::Closure {
        return Err(ProviderError::ToolsLayer(format!(
            "{}: `{}` is a {}-tier provider, and a tooling layer must be closure-tier \
             (flox, nix, devbox or devenv): a host-linked layer is host passthrough \
             under another name",
            path.display(),
            provider.static_name(),
            provider.tier().as_str(),
        )));
    }
    Ok(Some(Layer {
        path,
        provider,
        source,
    }))
}

/// Which provider's environment `dir` holds, from the file each one keeps.
/// More than one is refused rather than guessed: the configuration's
/// `provider` key is how to say which.
pub fn detect_provider(dir: &Path) -> Result<ProviderKind, ProviderError> {
    const MARKERS: [(&str, ProviderKind); 5] = [
        (".flox", ProviderKind::Flox),
        ("devenv.nix", ProviderKind::Devenv),
        ("devbox.json", ProviderKind::Devbox),
        ("flake.nix", ProviderKind::Nix),
        ("Package.swift", ProviderKind::Swift),
    ];
    let found: Vec<(&str, ProviderKind)> = MARKERS
        .into_iter()
        .filter(|(marker, _)| dir.join(marker).exists())
        .collect();
    match found.as_slice() {
        [(_, kind)] => Ok(*kind),
        [] => Err(ProviderError::ToolsLayer(format!(
            "{}: holds no flox, devenv, devbox or nix environment (looked for {})",
            dir.display(),
            MARKERS.map(|(m, _)| m).join(", ")
        ))),
        several => Err(ProviderError::ToolsLayer(format!(
            "{}: holds more than one environment ({}); say which with `provider` in \
             [tools]",
            dir.display(),
            several
                .iter()
                .map(|(m, _)| *m)
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// Resolve the layer host-side, through its provider's hook-free route.
pub fn resolve(layer: &Layer) -> Result<Resolved, ProviderError> {
    let in_layer =
        |e: ProviderError| ProviderError::ToolsLayer(format!("{}: {e}", layer.path.display()));
    // devbox, devenv and nix refuse a missing lock in their own `resolve`;
    // flox locks one on the spot instead, which for a layer would be a
    // resolution nothing pinned (`tooling` spec: "Unlocked tooling layer
    // rejected").
    if layer.provider == ProviderKind::Flox && !layer.path.join(".flox/env/manifest.lock").is_file()
    {
        return Err(in_layer(ProviderError::MissingLock {
            provider: "flox",
            hint: "flox list",
        }));
    }
    let resolution = ProviderEntry::resolve(&layer.provider, &layer.path).map_err(in_layer)?;
    let path_entries = path_entries(&resolution.env, &resolution.read_only_grants);
    if path_entries.is_empty() {
        return Err(in_layer(ProviderError::ResolutionFailed(
            "its environment puts nothing from its own store on PATH, so it would add no \
             tool to a session"
                .to_string(),
        )));
    }
    let fingerprint = layer.provider.fingerprint(&layer.path).map_err(in_layer)?;
    Ok(Resolved {
        path_entries,
        grants: resolution.read_only_grants,
        fingerprint,
    })
}

/// Why the layer a sandbox was brought up with is not the one `wanted` would
/// give it now, or `None` when it still is. One rule for `up` against a
/// running sandbox and for `status`, applied by `up --recreate` alone: the
/// environment resolves once, at `up` (`tooling` spec: "The layer is
/// recorded, and goes stale like the project").
pub fn stale_reason(
    recorded: Option<&crate::lifecycle::ToolsRecord>,
    wanted: Option<&Layer>,
) -> Option<String> {
    match (recorded, wanted) {
        (None, None) => None,
        (None, Some(w)) => Some(format!(
            "a tooling layer is configured ({}), and this sandbox was brought up without one",
            w.path.display()
        )),
        (Some(r), None) => Some(format!(
            "this sandbox has the tooling layer at {}, and none is configured now",
            r.path
        )),
        (Some(r), Some(w))
            if Path::new(&r.path) != w.path || r.provider != w.provider.static_name() =>
        {
            Some(format!(
                "the tooling layer is now {} ({}), and this sandbox has {} ({})",
                w.path.display(),
                w.provider.static_name(),
                r.path,
                r.provider
            ))
        }
        (Some(r), Some(w)) => match w.provider.fingerprint(&w.path) {
            Ok(current) if current == r.fingerprint => None,
            Ok(_) => Some(format!(
                "the tooling layer at {} changed since this sandbox's `up`",
                r.path
            )),
            Err(e) => Some(format!(
                "the tooling layer at {} cannot be read: {e}",
                r.path
            )),
        },
    }
}

/// The layer a recorded one would resolve to again: itself, for a layer
/// given with `--tools`, which no configuration change makes stale.
pub fn recorded_layer(record: &crate::lifecycle::ToolsRecord) -> Option<Layer> {
    Some(Layer {
        path: PathBuf::from(&record.path),
        provider: ProviderKind::from_name(&record.provider).ok()?,
        source: Source::Flag,
    })
}

/// The layer's `PATH` entries that lie inside its own grants. A provider's
/// `PATH` ends in the host's directories (`crate::shell`'s guard exists for
/// the same reason), and those are not the layer's: denied inside anyway,
/// and not what it declares.
fn path_entries(env: &BTreeMap<String, String>, grants: &[String]) -> Vec<String> {
    let mut entries: Vec<String> = Vec::new();
    for entry in env.get("PATH").map(String::as_str).unwrap_or("").split(':') {
        let inside = grants.iter().any(|g| Path::new(entry).starts_with(g));
        if inside && !entries.iter().any(|e| e == entry) {
            entries.push(entry.to_string());
        }
    }
    entries
}

/// `project`'s `PATH` with the layer's entries appended, skipping any it
/// already has: the project's tools are found first.
pub fn compose_path(project: Option<&str>, layer: &[String]) -> String {
    let mut path: Vec<&str> = project
        .unwrap_or("")
        .split(':')
        .filter(|e| !e.is_empty())
        .collect();
    for entry in layer {
        if !path.contains(&entry.as_str()) {
            path.push(entry);
        }
    }
    path.join(":")
}

/// The names in the layer's entries that an entry of `project`'s `PATH`
/// provides first, sorted: what `status` reports as shadowed. Read from the
/// directories host-side at `up`, since only the store paths are recorded.
pub fn shadowed(project: Option<&str>, layer: &[String]) -> Vec<String> {
    let names = |dir: &str| -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|it| {
                it.filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    let project: std::collections::BTreeSet<String> = project
        .unwrap_or("")
        .split(':')
        .filter(|e| !e.is_empty() && !layer.iter().any(|l| l == e))
        .flat_map(names)
        .collect();
    let mut out: Vec<String> = layer
        .iter()
        .flat_map(|dir| names(dir))
        .filter(|n| project.contains(n))
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("devcroft-tooling-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_layer_follows_the_project_and_repeats_nothing() {
        let layer = vec![
            "/nix/store/a-agent/bin".to_string(),
            "/nix/store/p/bin".to_string(),
        ];
        assert_eq!(
            compose_path(Some("/nix/store/p/bin:/nix/store/q/bin"), &layer),
            "/nix/store/p/bin:/nix/store/q/bin:/nix/store/a-agent/bin"
        );
        assert_eq!(
            compose_path(None, &layer),
            "/nix/store/a-agent/bin:/nix/store/p/bin"
        );
    }

    /// Only entries inside the layer's grants: the host directories a
    /// provider's `PATH` ends in are not the layer's.
    #[test]
    fn host_entries_on_the_layers_path_are_not_taken() {
        let env = BTreeMap::from([(
            "PATH".to_string(),
            "/nix/store/x-env/bin:/usr/bin:/nix/store/x-env/bin:/bin".to_string(),
        )]);
        assert_eq!(
            path_entries(&env, &["/nix/store".to_string()]),
            vec!["/nix/store/x-env/bin".to_string()]
        );
    }

    #[test]
    fn a_provider_is_detected_from_its_file_and_ambiguity_is_refused() {
        let dir = tempdir("detect");
        assert!(detect_provider(&dir).is_err());
        std::fs::write(dir.join("devenv.nix"), "{}").unwrap();
        assert_eq!(detect_provider(&dir).unwrap(), ProviderKind::Devenv);
        std::fs::write(dir.join("flake.nix"), "{}").unwrap();
        let err = detect_provider(&dir).unwrap_err().to_string();
        assert!(
            err.contains("more than one") && err.contains("provider"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `tooling` spec: "Artifact-tier layer rejected", before anything
    /// resolves.
    #[test]
    fn an_artifact_tier_layer_is_refused() {
        let dir = tempdir("artifact");
        std::fs::write(dir.join("Package.swift"), "").unwrap();
        let err = select(Some(&dir), &UserConfig::default()).unwrap_err();
        assert!(matches!(err, ProviderError::ToolsLayer(_)), "{err:?}");
        assert!(err.to_string().contains("closure-tier"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_flag_wins_over_the_configuration() {
        let flag = tempdir("flag");
        std::fs::write(flag.join("flake.nix"), "{}").unwrap();
        let config = UserConfig {
            tools: Some(crate::config::ToolsDecl {
                provider: Some("devenv".into()),
                path: PathBuf::from("/nonexistent/configured"),
            }),
            ..Default::default()
        };
        let layer = select(Some(&flag), &config).unwrap().unwrap();
        assert_eq!(layer.provider, ProviderKind::Nix);
        assert_eq!(layer.source, Source::Flag);
        assert_eq!(select(None, &UserConfig::default()).unwrap(), None);
        let _ = std::fs::remove_dir_all(&flag);
    }

    /// An unlocked flox layer fails before flox gets a chance to lock it.
    #[test]
    fn an_unlocked_flox_layer_is_refused() {
        let dir = tempdir("flox-unlocked");
        std::fs::create_dir_all(dir.join(".flox/env")).unwrap();
        let layer = select(Some(&dir), &UserConfig::default()).unwrap().unwrap();
        let err = resolve(&layer).unwrap_err().to_string();
        assert!(
            err.contains("tooling layer") && err.contains("lockfile"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shadowing_names_what_the_project_provides_first() {
        let root = tempdir("shadow");
        let (project, layer) = (root.join("project/bin"), root.join("layer/bin"));
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&layer).unwrap();
        for (dir, name) in [(&project, "git"), (&layer, "git"), (&layer, "claude")] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        let layer = vec![layer.to_string_lossy().into_owned()];
        assert_eq!(
            shadowed(Some(&project.to_string_lossy()), &layer),
            vec!["git".to_string()]
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
