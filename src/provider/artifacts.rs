//! The artifact store shared by artifact-tier providers that materialize
//! immutable objects (`add-mise-provider` M4, `add-pixi-provider` P3).
//!
//! Each provider owns `~/.local/share/devcroft-artifacts/<provider>/`, a
//! sibling of devcroft's baseline-denied data directory. An object is a
//! directory named by a 64-hex-digit recipe hash. A sandbox's `Meta` naming
//! the id is the only reference; `rm` sweeps what nothing names, under an
//! exclusive store lock that every `up` holds shared from acquiring its object
//! until its `Meta` is written.
use super::ProviderError;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Every provider's store lives under this directory, so a manifest write
/// grant is checked against it once for all of them.
pub(crate) const ARTIFACTS_DIR: &str = ".local/share/devcroft-artifacts";

/// Providers whose objects live in the store and are swept by `rm`.
pub(crate) const STORE_PROVIDERS: &[&str] = &["mise", "pixi"];

fn fail(provider: &str, message: impl std::fmt::Display) -> ProviderError {
    ProviderError::ResolutionFailed(format!("{provider}: {message}"))
}

/// `path` with its existing ancestors canonicalized, so a symlinked home
/// compares equal to the path a grant resolves to.
fn canonical_prefix(path: PathBuf) -> Result<PathBuf, String> {
    let mut ancestor = path.as_path();
    let mut missing = vec![];
    while !ancestor.exists() {
        missing.push(
            ancestor
                .file_name()
                .ok_or("invalid artifact root")?
                .to_owned(),
        );
        ancestor = ancestor.parent().ok_or("invalid artifact root")?;
    }
    let mut real = ancestor.canonicalize().map_err(|e| e.to_string())?;
    for part in missing.into_iter().rev() {
        real.push(part);
    }
    Ok(real)
}

/// The directory holding every provider's store.
pub(crate) fn artifacts_root() -> Result<PathBuf, ProviderError> {
    let home = std::env::var_os("HOME").ok_or_else(|| fail("artifacts", "HOME is not set"))?;
    canonical_prefix(PathBuf::from(home).join(ARTIFACTS_DIR)).map_err(|e| fail("artifacts", e))
}

/// One provider's store.
pub(crate) fn root(provider: &str) -> Result<PathBuf, ProviderError> {
    Ok(artifacts_root()?.join(provider))
}

pub(crate) struct StoreLock(std::fs::File);
impl Drop for StoreLock {
    fn drop(&mut self) {
        // SAFETY: unlocking a descriptor this value owns.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

pub(crate) fn lock(path: &Path, exclusive: bool) -> Result<StoreLock, ProviderError> {
    let err = |e: std::io::Error| fail("artifacts", format!("{}: {e}", path.display()));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(err)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(err)?;
    let op = if exclusive {
        libc::LOCK_EX
    } else {
        libc::LOCK_SH
    };
    // SAFETY: flock on a descriptor `file` owns for the call.
    if unsafe { libc::flock(file.as_raw_fd(), op) } != 0 {
        return Err(err(std::io::Error::last_os_error()));
    }
    Ok(StoreLock(file))
}

/// The shared lock an `up` holds from acquiring its object until its `Meta`
/// is written.
pub(crate) fn lease(provider: &str) -> Result<StoreLock, ProviderError> {
    lock(&root(provider)?.join(".store.lock"), false)
}

pub(crate) fn is_object_id(name: &str) -> bool {
    name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The object a set of provider grants lives in, if any.
pub(crate) fn object_id(provider: &str, grants: &[String]) -> Option<String> {
    let store = root(provider).ok()?;
    grants.iter().find_map(|grant| {
        let part = Path::new(grant)
            .strip_prefix(&store)
            .ok()?
            .components()
            .next()?
            .as_os_str()
            .to_str()?;
        is_object_id(part).then(|| part.to_owned())
    })
}

/// Remove every object of `provider` that no recorded `Meta` names.
pub(crate) fn sweep(provider: &str) -> Result<(), ProviderError> {
    let store = root(provider)?;
    if !store.exists() {
        return Ok(());
    }
    let _lock = lock(&store.join(".store.lock"), true)?;
    let references = crate::lifecycle::recorded_artifact_objects()
        .map_err(|e| fail(provider, format!("reading object references: {e}")))?;
    sweep_unreferenced(provider, &store, &references)
}

/// Remove every object, published or not, that no recorded `Meta` names, and
/// its per-object lock file. The caller holds the store lock exclusively, so
/// no `up` sits between acquiring an object and recording it.
pub(crate) fn sweep_unreferenced(
    provider: &str,
    store: &Path,
    references: &BTreeSet<String>,
) -> Result<(), ProviderError> {
    let err = |e: std::io::Error| fail(provider, e);
    for entry in std::fs::read_dir(store).map_err(err)? {
        let entry = entry.map_err(err)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_object_id(&name) && !references.contains(&name) {
            std::fs::remove_dir_all(entry.path()).map_err(err)?;
        } else if name
            .strip_prefix('.')
            .and_then(|n| n.strip_suffix(".lock"))
            .is_some_and(|id| is_object_id(id) && !references.contains(id))
        {
            std::fs::remove_file(entry.path()).map_err(err)?;
        }
    }
    Ok(())
}

/// Parse `X.Y.Z` from the first word of a `--version` line, after an optional
/// leading program name (`pixi 0.84.2`) or `v`.
pub(crate) fn release(provider: &str, text: &str) -> Result<(u32, u32, u32), ProviderError> {
    let word = text
        .split_whitespace()
        .find(|w| {
            w.trim_start_matches('v')
                .starts_with(|c: char| c.is_ascii_digit())
        })
        .unwrap_or("")
        .trim_start_matches('v');
    let parts: Vec<_> = word
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<_, _>>()
        .map_err(|_| fail(provider, "version is not a numeric release"))?;
    match parts.as_slice() {
        [a, b, c] => Ok((*a, *b, *c)),
        _ => Err(fail(provider, "version is not a numeric release")),
    }
}

/// Digest installed bytes without executing them. Refuse escaping symlinks so
/// neither a receipt nor a grant legitimizes a host-path injection.
pub(crate) fn installed_digest(provider: &str, root: &Path) -> Result<String, ProviderError> {
    fn visit(
        provider: &str,
        path: &Path,
        root: &Path,
        digest: &mut Sha256,
    ) -> Result<(), ProviderError> {
        let err = |e: std::io::Error| fail(provider, format!("{}: {e}", path.display()));
        let metadata = std::fs::symlink_metadata(path).map_err(err)?;
        let name = path
            .strip_prefix(root)
            .unwrap_or(path)
            .as_os_str()
            .as_encoded_bytes();
        digest.update((name.len() as u64).to_be_bytes());
        digest.update(name);
        digest.update(metadata.permissions().mode().to_be_bytes());
        if metadata.file_type().is_symlink() {
            digest.update(b"symlink");
            let target = path.canonicalize().map_err(err)?;
            if !target.starts_with(root) {
                return Err(fail(
                    provider,
                    format!("installed symlink {} escapes its object", path.display()),
                ));
            }
            digest.update(
                std::fs::read_link(path)
                    .map_err(err)?
                    .as_os_str()
                    .as_encoded_bytes(),
            );
        } else if metadata.is_dir() {
            digest.update(b"directory");
            let mut children = std::fs::read_dir(path)
                .map_err(err)?
                .map(|e| e.map(|v| v.path()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?;
            children.sort();
            for child in children {
                visit(provider, &child, root, digest)?;
            }
        } else if metadata.is_file() {
            digest.update(b"file");
            digest.update(metadata.len().to_be_bytes());
            digest.update(std::fs::read(path).map_err(err)?);
        } else {
            return Err(fail(
                provider,
                format!("unsupported installed special file {}", path.display()),
            ));
        }
        Ok(())
    }
    let mut digest = Sha256::new();
    visit(provider, root, root, &mut digest)?;
    Ok(digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// What a native executable asks the loader for, read from its headers:
/// the ELF interpreter and `DT_NEEDED` names, or Mach-O dylib load commands.
/// Never runs anything, since `ldd` executes the binary's interpreter.
/// Scripts and non-native files yield nothing; unqualified binary formats
/// fail closed.
pub(crate) fn header_dependencies(
    provider: &str,
    path: &Path,
) -> Result<Vec<String>, ProviderError> {
    use object::read::elf::{Dyn, ProgramHeader};
    let data =
        std::fs::read(path).map_err(|e| fail(provider, format!("{}: {e}", path.display())))?;
    let error = |e: object::Error| fail(provider, format!("{}: {e}", path.display()));
    let utf8 = |b: &[u8]| String::from_utf8(b.to_vec()).map_err(|e| fail(provider, e));
    let mut out = vec![];
    if data.starts_with(b"\x7fELF") {
        let elf = object::read::elf::ElfFile64::<object::Endianness>::parse(data.as_slice())
            .map_err(error)?;
        let endian = elf.endian();
        let table = elf.elf_section_table();
        let dynamic = table.dynamic(endian, data.as_slice()).map_err(error)?;
        for header in elf.elf_program_headers() {
            if let Some(interpreter) = header.interpreter(endian, data.as_slice()).map_err(error)? {
                out.push(utf8(interpreter)?);
            }
            if header.p_type(endian) == object::elf::PT_DYNAMIC && dynamic.is_none() {
                return Err(fail(
                    provider,
                    format!(
                        "{}: dynamic ELF without section metadata is unqualified",
                        path.display()
                    ),
                ));
            }
        }
        if let Some((entries, index)) = dynamic {
            let strings = table
                .strings(endian, data.as_slice(), index)
                .map_err(error)?;
            for entry in entries {
                if entry.tag32(endian) == Some(object::elf::DT_NEEDED) {
                    out.push(utf8(entry.string(endian, strings).map_err(error)?)?);
                }
            }
        }
    } else if data.starts_with(b"\xcf\xfa\xed\xfe") {
        let macho = object::read::macho::MachOFile64::<object::Endianness>::parse(data.as_slice())
            .map_err(error)?;
        let mut commands = macho.macho_load_commands().map_err(error)?;
        while let Some(command) = commands.next().map_err(error)? {
            // LC_ID_DYLIB names the file itself, not a dependency.
            if command.cmd() == object::macho::LC_ID_DYLIB {
                continue;
            }
            if let Some(dylib) = command.dylib().map_err(error)? {
                out.push(utf8(
                    command
                        .string(macho.endian(), dylib.dylib.name)
                        .map_err(error)?,
                )?);
            }
        }
    } else if data.starts_with(b"\xca\xfe\xba\xbe") || data.starts_with(b"\xfe\xed\xfa") {
        return Err(fail(
            provider,
            format!("{}: binary format is not qualified", path.display()),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_reads_the_first_numeric_word() {
        assert_eq!(release("t", "pixi 0.84.2\n").unwrap(), (0, 84, 2));
        assert_eq!(
            release("t", "2026.11.9 macos-arm64 (2026-11-01)").unwrap(),
            (2026, 11, 9)
        );
        assert_eq!(release("t", "v1.2.3").unwrap(), (1, 2, 3));
        assert!(release("t", "pixi dev").is_err());
    }

    #[test]
    fn the_sweep_keeps_referenced_objects_only() {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-artifacts-sweep-{}-{:x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let store = dir.join("store");
        let kept = "a".repeat(64);
        let gone = "b".repeat(64);
        let half = "c".repeat(64);
        for id in [&kept, &gone, &half] {
            std::fs::create_dir_all(store.join(id).join("data")).unwrap();
            std::fs::write(store.join(format!(".{id}.lock")), "").unwrap();
        }
        std::fs::write(store.join(&kept).join("complete.sha256"), "x").unwrap();
        std::fs::write(store.join(&gone).join("complete.sha256"), "x").unwrap();
        std::fs::create_dir(store.join(".version")).unwrap();
        std::fs::write(store.join(".store.lock"), "").unwrap();
        let refs = BTreeSet::from([kept.clone()]);
        sweep_unreferenced("t", &store, &refs).unwrap();
        assert!(store.join(&kept).is_dir());
        assert!(store.join(format!(".{kept}.lock")).exists());
        for id in [&gone, &half] {
            assert!(!store.join(id).exists());
            assert!(!store.join(format!(".{id}.lock")).exists());
        }
        assert!(store.join(".version").is_dir());
        assert!(store.join(".store.lock").exists());
        sweep_unreferenced("t", &store, &Default::default()).unwrap();
        assert!(!store.join(&kept).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Concurrent `up` and sweep: the sweep's exclusive store lock waits for
    /// every `up` lease, so an object acquired but not yet recorded in `Meta`
    /// cannot be removed under it. Two leases do not exclude each other.
    #[test]
    fn a_sweep_waits_for_every_up_lease() {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-artifacts-lock-{}-{:x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let path = dir.join(".store.lock");
        let first = lock(&path, false).unwrap();
        let second = lock(&path, false).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let sweeper = {
            let path = path.clone();
            std::thread::spawn(move || {
                let _exclusive = lock(&path, true).unwrap();
                tx.send(()).unwrap();
            })
        };
        let wait = std::time::Duration::from_millis(200);
        assert!(rx.recv_timeout(wait).is_err(), "sweep ran under two leases");
        drop(first);
        assert!(rx.recv_timeout(wait).is_err(), "sweep ran under one lease");
        drop(second);
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("sweep never ran after the leases ended");
        sweeper.join().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn digest_frames_files_and_refuses_escaping_symlinks() {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-artifacts-digest-{}-{:x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        std::fs::write(dir.join("a"), "bc").unwrap();
        let before = installed_digest("t", &dir).unwrap();
        std::fs::remove_file(dir.join("a")).unwrap();
        std::fs::write(dir.join("ab"), "c").unwrap();
        assert_ne!(before, installed_digest("t", &dir).unwrap());
        std::os::unix::fs::symlink("/bin/sh", dir.join("escaped")).unwrap();
        assert!(
            installed_digest("t", &dir)
                .unwrap_err()
                .to_string()
                .contains("escapes")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
