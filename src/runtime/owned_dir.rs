//! Host-side writes into a directory the sandbox can write.
//!
//! The artifact directory (`<project>/.devcroft/<name>/`) is inside the
//! project root, so a process in the sandbox can replace any part of it with
//! a symlink before the next `up`. Writing there by path would follow it:
//! measured 2026-11-23, a `runtime` swapped for a link to another directory
//! had that directory's files deleted as stale entries. Every component is
//! therefore opened with `O_NOFOLLOW`, and every change is made relative to
//! the resulting descriptor, so a swap made at any moment is refused or
//! harmless.

use std::ffi::{CStr, CString, OsStr};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub struct OwnedDir(OwnedFd);

fn c(s: &OsStr) -> io::Result<CString> {
    CString::new(s.as_bytes()).map_err(io::Error::other)
}

fn check(rc: libc::c_int) -> io::Result<libc::c_int> {
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(rc)
    }
}

impl OwnedDir {
    /// `<artifact_dir>/<sub>`, created as needed. The last two components of
    /// `artifact_dir` (`.devcroft/<name>`) and `sub` must be real directories.
    pub fn open(artifact_dir: &Path, sub: &str) -> io::Result<Self> {
        let bad = || io::Error::other(format!("{}: not an artifact dir", artifact_dir.display()));
        let name = artifact_dir.file_name().ok_or_else(bad)?;
        let parent = artifact_dir.parent().ok_or_else(bad)?;
        let top = parent.file_name().ok_or_else(bad)?;
        let root = parent.parent().ok_or_else(bad)?;
        let mut dir: OwnedFd = std::fs::File::open(root)?.into();
        let mut at = root.to_path_buf();
        for part in [top, name, OsStr::new(sub)] {
            at.push(part);
            let part = c(part)?;
            let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
            // SAFETY: a live descriptor and a NUL-terminated name; the new
            // descriptor is owned here.
            dir = unsafe {
                if libc::mkdirat(dir.as_raw_fd(), part.as_ptr(), 0o755) != 0 {
                    let e = io::Error::last_os_error();
                    if e.kind() != io::ErrorKind::AlreadyExists {
                        return Err(e);
                    }
                }
                let fd =
                    check(libc::openat(dir.as_raw_fd(), part.as_ptr(), flags)).map_err(|e| {
                        io::Error::new(
                            e.kind(),
                            format!(
                                "{}: must be a real directory, not a link ({e})",
                                at.display()
                            ),
                        )
                    })?;
                OwnedFd::from_raw_fd(fd)
            };
        }
        Ok(Self(dir))
    }

    fn remove(&self, name: &CStr) -> io::Result<()> {
        // SAFETY: a live descriptor and a NUL-terminated name.
        match check(unsafe { libc::unlinkat(self.0.as_raw_fd(), name.as_ptr(), 0) }) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Make `name` a symlink to `target`, replacing whatever it was.
    pub fn link(&self, name: &str, target: &Path) -> io::Result<()> {
        let name = c(OsStr::new(name))?;
        let mut buf = [0u8; libc::PATH_MAX as usize];
        // SAFETY: the buffer and its length.
        let n = unsafe {
            libc::readlinkat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                buf.as_mut_ptr().cast(),
                buf.len(),
            )
        };
        if n >= 0 && &buf[..n as usize] == target.as_os_str().as_bytes() {
            return Ok(());
        }
        self.remove(&name)?;
        let target = c(target.as_os_str())?;
        // SAFETY: NUL-terminated strings and a live descriptor.
        check(unsafe { libc::symlinkat(target.as_ptr(), self.0.as_raw_fd(), name.as_ptr()) })?;
        Ok(())
    }

    /// Put `bytes` at `name` through a fresh file and a rename.
    pub fn write(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let tmp = c(OsStr::new(&format!("{name}.tmp")))?;
        let name = c(OsStr::new(name))?;
        self.remove(&tmp)?;
        let flags =
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: as above; `O_EXCL` means the file is new and this process's.
        let fd = unsafe {
            OwnedFd::from_raw_fd(check(libc::openat(
                self.0.as_raw_fd(),
                tmp.as_ptr(),
                flags,
                0o644 as libc::c_uint,
            ))?)
        };
        io::Write::write_all(&mut std::fs::File::from(fd), bytes)?;
        let dir = self.0.as_raw_fd();
        // SAFETY: as above.
        check(unsafe { libc::renameat(dir, tmp.as_ptr(), dir, name.as_ptr()) })?;
        Ok(())
    }

    /// Remove every entry `keep` rejects. A directory in the way is an error.
    pub fn retain(&self, keep: impl Fn(&OsStr) -> bool) -> io::Result<()> {
        for name in self.names()? {
            if !keep(OsStr::from_bytes(name.as_bytes())) {
                self.remove(&name)?;
            }
        }
        Ok(())
    }

    fn names(&self) -> io::Result<Vec<CString>> {
        let mut out = Vec::new();
        // SAFETY: a descriptor of its own for the stream, which `closedir`
        // closes; every name is copied before the next `readdir`.
        unsafe {
            let fd = check(libc::openat(
                self.0.as_raw_fd(),
                c".".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            ))?;
            let stream = libc::fdopendir(fd);
            if stream.is_null() {
                let e = io::Error::last_os_error();
                libc::close(fd);
                return Err(e);
            }
            loop {
                let entry = libc::readdir(stream);
                if entry.is_null() {
                    break;
                }
                let name = CStr::from_ptr((*entry).d_name.as_ptr()).to_owned();
                if name.as_bytes() != b"." && name.as_bytes() != b".." {
                    out.push(name);
                }
            }
            libc::closedir(stream);
        }
        Ok(out)
    }
}

/// Make `<artifact_dir>/<sub>` hold exactly `links`.
pub fn sync_links(artifact_dir: &Path, sub: &str, links: &[(String, PathBuf)]) -> io::Result<()> {
    let dir = OwnedDir::open(artifact_dir, sub)?;
    dir.retain(|n| links.iter().any(|(l, _)| OsStr::new(l) == n))?;
    for (name, target) in links {
        dir.link(name, target)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn a_link_at_any_component_is_refused_and_its_target_untouched() {
        let t = std::env::temp_dir().join(format!("dc-owned-{}", std::process::id()));
        for at in [".devcroft", ".devcroft/n", ".devcroft/n/ns"] {
            let _ = std::fs::remove_dir_all(&t);
            let victim = t.join("victim");
            std::fs::create_dir_all(&victim).unwrap();
            std::fs::write(victim.join("precious"), "x").unwrap();
            let link = t.join("p").join(at);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            symlink(&victim, &link).unwrap();
            let art = t.join("p/.devcroft/n");
            assert!(sync_links(&art, "ns", &[]).is_err(), "{at}");
            assert_eq!(std::fs::read_dir(&victim).unwrap().count(), 1, "{at}");
        }
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn writing_replaces_a_planted_link_without_following_it() {
        let t = std::env::temp_dir().join(format!("dc-owned-w-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let dir = t.join("p/.devcroft/n/d");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(t.join("precious"), "x").unwrap();
        symlink(t.join("precious"), dir.join("f")).unwrap();
        symlink(t.join("precious"), dir.join("f.tmp")).unwrap();
        let d = OwnedDir::open(&t.join("p/.devcroft/n"), "d").unwrap();
        d.write("f", b"new").unwrap();
        assert_eq!(std::fs::read(t.join("precious")).unwrap(), b"x");
        assert_eq!(std::fs::read(dir.join("f")).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&t);
    }
}
