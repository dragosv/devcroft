//! Short, sandbox-specific scratch paths for the macOS process backend.

use std::io;
use std::path::{Path, PathBuf};

fn state_key(state: &Path) -> String {
    use sha2::{Digest, Sha256};
    let canonical = state
        .parent()
        .and_then(|parent| Some(parent.canonicalize().ok()?.join(state.file_name()?)));
    Sha256::digest(
        canonical
            .as_deref()
            .unwrap_or(state)
            .as_os_str()
            .as_encoded_bytes(),
    )
    .iter()
    .map(|byte| format!("{byte:02x}"))
    .collect()
}

/// The managed parent is protected separately from host temp, even when
/// a sandbox opts in to the latter.
pub fn parent() -> PathBuf {
    Path::new("/private/tmp").join(format!("dc-{}", unsafe { libc::geteuid() }))
}

pub fn path(state: &Path) -> PathBuf {
    parent().join(&state_key(state)[..8])
}

#[cfg(target_os = "macos")]
fn checked(
    state: &Path,
    create: bool,
) -> io::Result<(
    super::owned_dir::OwnedDir,
    super::owned_dir::OwnedDir,
    PathBuf,
    String,
)> {
    use super::owned_dir::OwnedDir;
    use std::ffi::OsStr;

    let parent = parent();
    let key = state_key(state);
    let path = parent.join(&key[..8]);
    let marker = format!("{}.key", &key[..8]);
    let anchor = OwnedDir::anchor(Path::new("/"))?
        .child(OsStr::new("private"), None)?
        .child(OsStr::new("tmp"), None)?;
    let parent_dir = anchor
        .child(parent.file_name().unwrap(), create.then_some(0o700))?
        .require_private(&parent)?;
    parent_dir.check_key(&marker, key.as_bytes(), create)?;
    let dir = parent_dir
        .child(path.file_name().unwrap(), create.then_some(0o700))?
        .require_private(&path)?;
    Ok((parent_dir, dir, path, marker))
}

pub fn start(state: &Path) -> io::Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let (_, dir, path, _) = checked(state, true)?;
        dir.empty()?;
        Ok(path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = state;
        Ok(PathBuf::from("/tmp"))
    }
}

pub fn clean(state: &Path, remove: bool) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let (parent, dir, path, key) = match checked(state, false) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            result => result?,
        };
        dir.empty()?;
        if remove {
            parent.remove_child_dir(path.file_name().unwrap())?;
            parent.unlink_key(&key)?;
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (state, remove);
    Ok(())
}
