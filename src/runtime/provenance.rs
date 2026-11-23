//! Where each base name resolves in a sandbox, by name and by absolute
//! path — computed once at `up` from data `up` already holds, never by
//! spawning anything (`add-runtime-provenance` design D2).
//!
//! The inputs are the ones the backend and the view are built from: the
//! sandbox's `PATH`, the canonical grants the compiled policy resolved to,
//! and on Linux the identity entries the view writes (`/bin/sh`,
//! `/usr/bin/env`). Nothing here decides what a sandbox may do; it records
//! what the decisions already made amount to for shell and userland.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{BASE_NAMES, BASE_NAMES_REVISION, absolute_paths};

/// The store every closure-tier provider materializes into.
const STORE_PREFIX: &str = "/nix/store";

/// Where one base name, in one lookup form, resolves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provenance {
    /// Inside the provider's closure (the Nix store).
    Closure { path: String },
    /// Inside an artifact provider's own install (the artifact store).
    Provider { provider: String, path: String },
    /// devcroft's own runtime, at a generation (`add-own-userland`). `path`
    /// is the runtime link that runs, not the binary it points at: the
    /// link's name is what selects the utility.
    Runtime { generation: u32, path: String },
    /// A host file in a sandbox whose provider declares native mode
    /// (`add-own-userland` D5): the host's userland on purpose, with a
    /// reason, rather than by omission. Recorded and shown, not warned
    /// about: `up` states the mode and its reason once instead.
    Native { provider: String, path: String },
    /// A host file.
    Host { path: String },
    /// Does not resolve inside the sandbox.
    Absent { reason: Absence },
}

/// Why a base name does not resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Absence {
    /// Nothing by that name on the sandbox's `PATH`, or no file at that path.
    NotPresent,
    /// The file exists on the host and the compiled policy does not grant it.
    RefusedByPolicy,
    /// The file exists on the host and the sandbox's view does not contain it.
    NotInView,
}

impl Provenance {
    pub fn is_host(&self) -> bool {
        matches!(self, Provenance::Host { .. })
    }

    /// The executable that runs, when one does.
    pub fn path(&self) -> Option<&str> {
        match self {
            Provenance::Closure { path }
            | Provenance::Provider { path, .. }
            | Provenance::Runtime { path, .. }
            | Provenance::Native { path, .. }
            | Provenance::Host { path } => Some(path),
            Provenance::Absent { .. } => None,
        }
    }

    /// The short kind, for counting.
    pub fn kind(&self) -> &'static str {
        match self {
            Provenance::Closure { .. } => "closure",
            Provenance::Provider { .. } => "provider",
            Provenance::Runtime { .. } => "runtime",
            Provenance::Native { .. } => "native",
            Provenance::Host { .. } => "host",
            Provenance::Absent { .. } => "absent",
        }
    }
}

impl fmt::Display for Provenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Provenance::Closure { path } => write!(f, "closure:{path}"),
            Provenance::Provider { provider, path } => write!(f, "provider:{provider}:{path}"),
            Provenance::Runtime { generation, path } => {
                write!(f, "runtime:gen-{generation}:{path}")
            }
            Provenance::Native { provider, path } => write!(f, "native:{provider}:{path}"),
            Provenance::Host { path } => write!(f, "host:{path}"),
            Provenance::Absent { reason } => match reason {
                Absence::NotPresent => write!(f, "absent (not present)"),
                Absence::RefusedByPolicy => write!(f, "absent (refused by policy)"),
                Absence::NotInView => write!(f, "absent (not in the view)"),
            },
        }
    }
}

/// One base name in one lookup form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    /// `PATH` for the by-name lookup, otherwise the absolute path.
    pub lookup: String,
    pub provenance: Provenance,
}

/// The by-name lookup's `lookup` value.
pub const BY_NAME: &str = "PATH";

/// The whole record for one sandbox.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// [`BASE_NAMES_REVISION`] at the time it was computed.
    pub revision: u32,
    pub entries: Vec<Entry>,
    /// The provider's native-mode reason, when it declares one
    /// (`add-own-userland` D5), so `policy --render` can state it beside the
    /// `native:` entries it explains.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<String>,
}

impl Record {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every entry that resolves to a host file.
    pub fn host_entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.provenance.is_host())
    }

    /// How many entries of each kind, in a fixed order.
    pub fn counts(&self) -> [(&'static str, usize); 6] {
        let count = |k: &str| {
            self.entries
                .iter()
                .filter(|e| e.provenance.kind() == k)
                .count()
        };
        [
            ("closure", count("closure")),
            ("provider", count("provider")),
            ("runtime", count("runtime")),
            ("native", count("native")),
            ("host", count("host")),
            ("absent", count("absent")),
        ]
    }

    /// The one-time `up` warning for host entries, or `None` when there
    /// are none (`add-runtime-provenance`: warned, never refused).
    pub fn host_warning(&self) -> Option<String> {
        let hosts: Vec<String> = self
            .host_entries()
            .map(|e| {
                let path = e.provenance.path().unwrap_or_default();
                if e.lookup == BY_NAME {
                    format!("{} → {path}", e.name)
                } else {
                    format!("{} → {path}", e.lookup)
                }
            })
            .collect();
        if hosts.is_empty() {
            return None;
        }
        Some(format!(
            "shell and userland resolve to the host for {} base name(s), so this \
             sandbox is not reproducible at that level (fallback: the host's files \
             run; see `devcroft policy --render` and docs/base-runtime.md): {}",
            hosts.len(),
            hosts.join(", ")
        ))
    }
}

/// The identity entries a Linux view writes, which win over the grants for
/// the paths they cover (`fleet::mount::Identity`).
#[derive(Debug, Clone, Copy)]
pub struct ViewEntries<'a> {
    /// What the view puts at `/bin/sh`, when it writes one.
    pub sh: Option<&'a Path>,
    /// What the view puts at `/usr/bin/env`, when it writes one.
    pub env: Option<&'a Path>,
}

impl<'a> ViewEntries<'a> {
    /// What the view wrote at `path`, if anything. Compared by where each
    /// lands in the view, so `/usr/bin/sh` is the written `/bin/sh` on a
    /// host whose `/bin` is a symlink to `usr/bin`.
    fn written_at(self, path: &Path) -> Option<&'a Path> {
        let at = view_location(path);
        [("/bin/sh", self.sh), ("/usr/bin/env", self.env)]
            .into_iter()
            .find_map(|(p, target)| target.filter(|_| view_location(Path::new(p)) == at))
    }
}

/// Where an absolute path lands in a Linux view: its first component
/// followed when the host has it as a symlink, since the view recreates
/// exactly those (`fleet::mount::setup_merged_usr_compat`, and `in_view`
/// for how identity files are placed through them).
fn view_location(path: &Path) -> PathBuf {
    let mut components = path.components();
    let (Some(std::path::Component::RootDir), Some(first)) = (components.next(), components.next())
    else {
        return path.to_path_buf();
    };
    let head = Path::new("/").join(first);
    let head = match std::fs::read_link(&head) {
        Ok(target) => Path::new("/").join(target.strip_prefix("/").unwrap_or(&target)),
        Err(_) => head,
    };
    head.join(components.as_path())
}

fn resolve_written(inputs: &Inputs<'_>, target: &Path) -> Provenance {
    match target.canonicalize() {
        Ok(c) => classify(inputs, &c),
        Err(_) => Provenance::Absent {
            reason: Absence::NotPresent,
        },
    }
}

/// What [`compute`] needs.
#[derive(Debug, Clone, Copy)]
pub struct Inputs<'a> {
    /// The sandbox's `PATH`, exactly as its processes get it.
    pub path: &'a str,
    /// Canonical roots the sandbox may read and execute.
    pub grants: &'a [PathBuf],
    /// The artifact store, whose objects are a provider's own install.
    pub artifact_root: Option<&'a Path>,
    /// The provider's name, for `provider:` entries.
    pub provider: &'a str,
    /// devcroft's runtime directory and its generation, for `runtime:`
    /// entries; `None` where the sandbox has no runtime.
    pub runtime: Option<(&'a Path, u32)>,
    /// The provider's native-mode reason, when it declares one: host files
    /// then record as `native:<provider>` rather than `host:`.
    pub native: Option<&'a str>,
    /// `Some` where the sandbox has a mount view (Linux); `None` where the
    /// host filesystem is visible and the policy decides (macOS).
    pub view: Option<ViewEntries<'a>>,
    /// The namespace an exec of an absolute base-name path is redirected
    /// into (`super::namespace`, macOS); `None` where none is in effect.
    pub namespace: Option<&'a [(String, PathBuf)]>,
}

/// The record for one sandbox.
pub fn compute(inputs: &Inputs<'_>) -> Record {
    let mut entries = Vec::new();
    for name in BASE_NAMES {
        entries.push(Entry {
            name: (*name).to_owned(),
            lookup: BY_NAME.to_owned(),
            provenance: by_name(inputs, name),
        });
        for abs in absolute_paths(name) {
            let provenance = absolute(inputs, name, Path::new(&abs));
            entries.push(Entry {
                name: (*name).to_owned(),
                lookup: abs,
                provenance,
            });
        }
    }
    Record {
        revision: BASE_NAMES_REVISION,
        entries,
        native: inputs.native.map(str::to_owned),
    }
}

fn granted(inputs: &Inputs<'_>, canonical: &Path) -> bool {
    inputs.grants.iter().any(|g| canonical.starts_with(g))
}

fn is_executable_file(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn classify(inputs: &Inputs<'_>, canonical: &Path) -> Provenance {
    let path = canonical.to_string_lossy().into_owned();
    if canonical.starts_with(STORE_PREFIX) {
        Provenance::Closure { path }
    } else if inputs
        .artifact_root
        .is_some_and(|r| canonical.starts_with(r))
    {
        Provenance::Provider {
            provider: inputs.provider.to_owned(),
            path,
        }
    } else if inputs.native.is_some() {
        Provenance::Native {
            provider: inputs.provider.to_owned(),
            path,
        }
    } else {
        Provenance::Host { path }
    }
}

/// The first `PATH` hit the sandbox can see. An ungranted candidate is
/// skipped rather than reported: it is absent from a Linux view, and
/// Seatbelt refuses even its metadata, so a shell's search moves past it
/// (measured: `command -v ls` finds nothing in a flox sandbox whose
/// `PATH` ends in the host's `/usr/bin` and `/bin`).
fn by_name(inputs: &Inputs<'_>, name: &str) -> Provenance {
    for dir in inputs.path.split(':').filter(|d| !d.is_empty()) {
        let candidate = Path::new(dir).join(name);
        // What the view wrote wins over the host's file at that place,
        // and a `PATH` of `/usr/bin` reaches the `/bin/sh` the view wrote
        // on a merged-`/usr` host (measured: a flox sandbox without bash
        // on its `PATH` runs the closure's bash as `sh`, from `/usr/bin`).
        if let Some(target) = inputs.view.and_then(|v| v.written_at(&candidate)) {
            return resolve_written(inputs, target);
        }
        if !is_executable_file(&candidate) {
            continue;
        }
        // Before canonicalizing: a runtime link canonicalizes to the
        // devcroft binary, which says nothing about which utility runs.
        if let Some((dir, generation)) = inputs.runtime
            && Path::new(dir) == dir_of(&candidate)
        {
            return Provenance::Runtime {
                generation,
                path: candidate.to_string_lossy().into_owned(),
            };
        }
        let Ok(canonical) = candidate.canonicalize() else {
            continue;
        };
        if granted(inputs, &canonical) {
            return classify(inputs, &canonical);
        }
    }
    Provenance::Absent {
        reason: Absence::NotPresent,
    }
}

fn dir_of(path: &Path) -> &Path {
    path.parent().unwrap_or(Path::new(""))
}

fn absolute(inputs: &Inputs<'_>, name: &str, abs: &Path) -> Provenance {
    if let Some((_, target)) = inputs
        .namespace
        .and_then(|ns| ns.iter().find(|(n, _)| n == name))
    {
        return match inputs.runtime {
            Some((dir, generation)) if dir == dir_of(target) => Provenance::Runtime {
                generation,
                path: target.to_string_lossy().into_owned(),
            },
            _ => resolve_written(inputs, target),
        };
    }
    if let Some(target) = inputs.view.and_then(|v| v.written_at(abs)) {
        return resolve_written(inputs, target);
    }
    if !is_executable_file(abs) {
        return Provenance::Absent {
            reason: Absence::NotPresent,
        };
    }
    let Ok(canonical) = abs.canonicalize() else {
        return Provenance::Absent {
            reason: Absence::NotPresent,
        };
    };
    if granted(inputs, &canonical) {
        classify(inputs, &canonical)
    } else if inputs.view.is_some() {
        Provenance::Absent {
            reason: Absence::NotInView,
        }
    } else {
        Provenance::Absent {
            reason: Absence::RefusedByPolicy,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("dc-prov-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
        fn exe(&self, rel: &str) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn find<'a>(r: &'a Record, name: &str, lookup: &str) -> &'a Provenance {
        &r.entries
            .iter()
            .find(|e| e.name == name && e.lookup == lookup)
            .unwrap()
            .provenance
    }

    #[test]
    fn a_path_entry_outside_every_grant_is_skipped() {
        let s = Scratch::new("skip");
        s.exe("host/bin/ls");
        let granted_ls = s.exe("env/bin/ls");
        let path = format!(
            "{}:{}",
            s.0.join("host/bin").display(),
            s.0.join("env/bin").display()
        );
        let grants = [s.0.join("env")];
        let r = compute(&Inputs {
            path: &path,
            grants: &grants,
            artifact_root: None,
            provider: "test",
            runtime: None,
            native: None,
            view: None,
            namespace: None,
        });
        assert_eq!(
            find(&r, "ls", BY_NAME),
            &Provenance::Host {
                path: granted_ls.to_string_lossy().into_owned()
            }
        );
    }

    #[test]
    fn a_grant_that_covers_a_path_by_its_parent_counts() {
        let s = Scratch::new("parent");
        s.exe("env/bin/cat");
        let path = s.0.join("env/bin").to_string_lossy().into_owned();
        let grants = [s.0.clone()];
        let r = compute(&Inputs {
            path: &path,
            grants: &grants,
            artifact_root: None,
            provider: "test",
            runtime: None,
            native: None,
            view: None,
            namespace: None,
        });
        assert!(find(&r, "cat", BY_NAME).path().is_some());
    }

    #[test]
    fn an_artifact_object_is_the_providers_own_install() {
        let s = Scratch::new("artifact");
        let sh = s.exe("artifacts/pixi/obj/bin/sh");
        let path =
            s.0.join("artifacts/pixi/obj/bin")
                .to_string_lossy()
                .into_owned();
        let root = s.0.join("artifacts");
        let grants = [root.clone()];
        let r = compute(&Inputs {
            path: &path,
            grants: &grants,
            artifact_root: Some(&root),
            provider: "pixi",
            runtime: None,
            native: None,
            view: None,
            namespace: None,
        });
        assert_eq!(
            find(&r, "sh", BY_NAME),
            &Provenance::Provider {
                provider: "pixi".into(),
                path: sh.to_string_lossy().into_owned()
            }
        );
    }

    #[test]
    fn nothing_on_path_is_absent_not_present() {
        let r = compute(&Inputs {
            path: "",
            grants: &[],
            artifact_root: None,
            provider: "test",
            runtime: None,
            native: None,
            view: None,
            namespace: None,
        });
        assert_eq!(
            find(&r, "grep", BY_NAME),
            &Provenance::Absent {
                reason: Absence::NotPresent
            }
        );
    }

    #[test]
    fn an_ungranted_host_file_is_refused_without_a_view_and_missing_with_one() {
        // `/bin/sh` exists on every Unix host this suite runs on.
        let without = compute(&Inputs {
            path: "",
            grants: &[],
            artifact_root: None,
            provider: "test",
            runtime: None,
            native: None,
            view: None,
            namespace: None,
        });
        assert_eq!(
            find(&without, "sh", "/bin/sh"),
            &Provenance::Absent {
                reason: Absence::RefusedByPolicy
            }
        );
        let with = compute(&Inputs {
            path: "",
            grants: &[],
            artifact_root: None,
            provider: "test",
            runtime: None,
            native: None,
            view: Some(ViewEntries {
                sh: None,
                env: None,
            }),
            namespace: None,
        });
        assert_eq!(
            find(&with, "sh", "/bin/sh"),
            &Provenance::Absent {
                reason: Absence::NotInView
            }
        );
    }

    #[test]
    fn a_view_entry_wins_over_the_hosts_file() {
        let s = Scratch::new("view");
        let shell = s.exe("closure/bin/bash");
        let r = compute(&Inputs {
            path: "",
            grants: &[],
            artifact_root: None,
            provider: "test",
            runtime: None,
            native: None,
            view: Some(ViewEntries {
                sh: Some(&shell),
                env: None,
            }),
            namespace: None,
        });
        assert_eq!(
            find(&r, "sh", "/bin/sh").path(),
            Some(shell.to_str().unwrap())
        );
        assert_eq!(
            find(&r, "env", "/usr/bin/env"),
            &Provenance::Absent {
                reason: Absence::NotInView
            }
        );
    }

    #[test]
    fn a_path_hit_at_what_the_view_wrote_is_the_written_target() {
        // On a merged-`/usr` host `/usr/bin/sh` is the view's `/bin/sh`;
        // elsewhere the search reaches `/bin/sh` itself. Either way the
        // host's own `sh` must not be what is recorded, even though it is
        // the first executable on `PATH` and its canonical path may be
        // granted by the identity rule (measured on a flox sandbox).
        let s = Scratch::new("viewpath");
        let shell = s.exe("closure/bin/bash");
        let host_sh = Path::new("/bin/sh").canonicalize().unwrap();
        let grants = [host_sh];
        let r = compute(&Inputs {
            path: "/usr/bin:/bin",
            grants: &grants,
            artifact_root: None,
            provider: "test",
            runtime: None,
            native: None,
            view: Some(ViewEntries {
                sh: Some(&shell),
                env: None,
            }),
            namespace: None,
        });
        assert_eq!(
            find(&r, "sh", BY_NAME).path(),
            Some(shell.to_str().unwrap())
        );
    }

    #[test]
    fn a_runtime_link_is_the_runtime_not_the_binary_it_points_at() {
        let s = Scratch::new("runtime");
        let exe = s.exe("host/devcroft");
        let dir = s.0.join("rt");
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&exe, dir.join("ls")).unwrap();
        let path = dir.to_string_lossy().into_owned();
        let grants = [s.0.clone()];
        let r = compute(&Inputs {
            path: &path,
            grants: &grants,
            artifact_root: None,
            provider: "flox",
            runtime: Some((&dir, 3)),
            native: None,
            view: None,
            namespace: None,
        });
        assert_eq!(
            find(&r, "ls", BY_NAME),
            &Provenance::Runtime {
                generation: 3,
                path: dir.join("ls").to_string_lossy().into_owned()
            }
        );
        assert_eq!(
            find(&r, "ls", BY_NAME).to_string(),
            format!("runtime:gen-3:{}", dir.join("ls").display())
        );
    }

    #[test]
    fn a_redirected_absolute_path_is_its_namespace_entry() {
        let s = Scratch::new("namespace");
        let exe = s.exe("host/devcroft");
        let sh = s.exe("env/bin/bash");
        let dir = s.0.join("rt");
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&exe, dir.join("ls")).unwrap();
        let ns = [
            ("sh".to_owned(), sh.clone()),
            ("ls".to_owned(), dir.join("ls")),
        ];
        let r = compute(&Inputs {
            path: "",
            grants: &[],
            artifact_root: None,
            provider: "test",
            runtime: Some((&dir, 3)),
            native: None,
            view: None,
            namespace: Some(&ns),
        });
        assert_eq!(find(&r, "sh", "/bin/sh").path(), sh.to_str());
        assert_eq!(find(&r, "ls", "/bin/ls").kind(), "runtime");
        assert_eq!(find(&r, "ls", "/bin/ls").path(), dir.join("ls").to_str());
        // No entry: the policy decides, as without the redirect.
        assert_eq!(find(&r, "cat", "/bin/cat").path(), None);
    }

    #[test]
    fn host_entries_produce_one_warning_naming_each() {
        let r = Record {
            revision: 1,
            entries: vec![
                Entry {
                    name: "sh".into(),
                    lookup: BY_NAME.into(),
                    provenance: Provenance::Host {
                        path: "/bin/sh".into(),
                    },
                },
                Entry {
                    name: "ls".into(),
                    lookup: "/bin/ls".into(),
                    provenance: Provenance::Host {
                        path: "/bin/ls".into(),
                    },
                },
                Entry {
                    name: "cat".into(),
                    lookup: BY_NAME.into(),
                    provenance: Provenance::Absent {
                        reason: Absence::NotPresent,
                    },
                },
            ],
            native: None,
        };
        let w = r.host_warning().unwrap();
        assert!(
            w.contains("sh → /bin/sh") && w.contains("/bin/ls → /bin/ls"),
            "{w}"
        );
        assert!(!w.contains("cat"));
        assert!(Record::default().host_warning().is_none());
    }

    #[test]
    fn a_native_provider_records_host_files_as_native_and_warns_of_none() {
        let s = Scratch::new("native");
        let sh = s.exe("bin/sh");
        let path = sh.parent().unwrap().to_string_lossy().into_owned();
        let grants = [s.0.clone()];
        let r = compute(&Inputs {
            path: &path,
            grants: &grants,
            artifact_root: None,
            provider: "swift",
            runtime: None,
            native: Some("the reason"),
            view: None,
            namespace: None,
        });
        assert_eq!(
            find(&r, "sh", BY_NAME),
            &Provenance::Native {
                provider: "swift".into(),
                path: sh.to_string_lossy().into_owned()
            }
        );
        assert!(
            find(&r, "sh", BY_NAME)
                .to_string()
                .starts_with("native:swift:")
        );
        assert_eq!(r.native.as_deref(), Some("the reason"));
        assert!(r.host_warning().is_none());
    }

    #[test]
    fn the_record_round_trips_through_json() {
        let r = compute(&Inputs {
            path: "",
            grants: &[],
            artifact_root: None,
            provider: "test",
            runtime: None,
            native: None,
            view: None,
            namespace: None,
        });
        let back: Record = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }
}
