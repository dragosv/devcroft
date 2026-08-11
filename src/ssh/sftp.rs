//! A filesystem-backed SFTP subsystem handler (ssh spec's "SFTP SHALL be
//! supported sufficiently for scp/rsync and editor file operations",
//! task 6.3). Every operation here is a direct, synchronous `std::fs`
//! call with no access-control logic of its own — same posture as the
//! rest of the keeper: the sandbox (Landlock/Seatbelt, applied to this
//! whole process before any of this ever runs) is the real boundary, not
//! anything this handler decides. An operation Landlock would deny just
//! surfaces as a normal `PermissionDenied` I/O error, translated below to
//! the matching SFTP status code like any other filesystem error.
//!
//! Modern OpenSSH `scp` (9.0+) speaks SFTP by default, so this alone also
//! covers `scp`; `sftp` and editor file operations (VS Code Remote-SSH's
//! server bootstrap) go through the same protocol.

use std::collections::HashMap;
use std::fs::{self, ReadDir};
use std::io::{Read, Seek, SeekFrom, Write};

use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};

#[derive(Default)]
pub struct FsHandler {
    open_files: HashMap<String, fs::File>,
    open_dirs: HashMap<String, ReadDir>,
    next_handle: u64,
}

impl FsHandler {
    fn alloc_handle(&mut self) -> String {
        let handle = self.next_handle.to_string();
        self.next_handle += 1;
        handle
    }
}

fn ok_status(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".to_string(),
        language_tag: "en-US".to_string(),
    }
}

/// Maps a filesystem error to the closest SFTP status code — including a
/// Landlock/Seatbelt denial, which the kernel reports as a plain
/// `PermissionDenied` `io::Error` like any other.
fn map_io_err(e: std::io::Error) -> StatusCode {
    match e.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

/// A batch size for `readdir` responses: large enough that a typical
/// project directory drains in one or two round trips, small enough not
/// to build an unbounded reply for a directory with millions of entries.
const READDIR_BATCH: usize = 256;

impl russh_sftp::server::Handler for FsHandler {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        let options: fs::OpenOptions = pflags.into();
        let file = options.open(&filename).map_err(map_io_err)?;
        let handle = self.alloc_handle();
        self.open_files.insert(handle.clone(), file);
        Ok(Handle { id, handle })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.open_files.remove(&handle);
        self.open_dirs.remove(&handle);
        Ok(ok_status(id))
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let file = self
            .open_files
            .get_mut(&handle)
            .ok_or(StatusCode::Failure)?;
        file.seek(SeekFrom::Start(offset)).map_err(map_io_err)?;
        let mut buf = vec![0u8; len as usize];
        let n = file.read(&mut buf).map_err(map_io_err)?;
        if n == 0 {
            return Err(StatusCode::Eof);
        }
        buf.truncate(n);
        Ok(Data { id, data: buf })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let file = self
            .open_files
            .get_mut(&handle)
            .ok_or(StatusCode::Failure)?;
        file.seek(SeekFrom::Start(offset)).map_err(map_io_err)?;
        file.write_all(&data).map_err(map_io_err)?;
        Ok(ok_status(id))
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let meta = fs::symlink_metadata(&path).map_err(map_io_err)?;
        Ok(Attrs {
            id,
            attrs: (&meta).into(),
        })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let file = self.open_files.get(&handle).ok_or(StatusCode::Failure)?;
        let meta = file.metadata().map_err(map_io_err)?;
        Ok(Attrs {
            id,
            attrs: (&meta).into(),
        })
    }

    async fn fsetstat(
        &mut self,
        id: u32,
        handle: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        if let Some(size) = attrs.size {
            let file = self.open_files.get(&handle).ok_or(StatusCode::Failure)?;
            file.set_len(size).map_err(map_io_err)?;
        }
        Ok(ok_status(id))
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let entries = fs::read_dir(&path).map_err(map_io_err)?;
        let handle = self.alloc_handle();
        self.open_dirs.insert(handle.clone(), entries);
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        let entries = self.open_dirs.get_mut(&handle).ok_or(StatusCode::Failure)?;
        let mut files = Vec::new();
        for entry in entries.by_ref().take(READDIR_BATCH) {
            let entry = entry.map_err(map_io_err)?;
            let meta = entry.metadata().map_err(map_io_err)?;
            files.push(File::new(
                entry.file_name().to_string_lossy().into_owned(),
                (&meta).into(),
            ));
        }
        if files.is_empty() {
            return Err(StatusCode::Eof);
        }
        Ok(Name { id, files })
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        fs::remove_file(&filename).map_err(map_io_err)?;
        Ok(ok_status(id))
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        fs::create_dir(&path).map_err(map_io_err)?;
        Ok(ok_status(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        fs::remove_dir(&path).map_err(map_io_err)?;
        Ok(ok_status(id))
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let resolved = fs::canonicalize(&path).unwrap_or_else(|_| path.clone().into());
        Ok(Name {
            id,
            files: vec![File::dummy(resolved.to_string_lossy().into_owned())],
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let meta = fs::metadata(&path).map_err(map_io_err)?;
        Ok(Attrs {
            id,
            attrs: (&meta).into(),
        })
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        fs::rename(&oldpath, &newpath).map_err(map_io_err)?;
        Ok(ok_status(id))
    }

    async fn readlink(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let target = fs::read_link(&path).map_err(map_io_err)?;
        Ok(Name {
            id,
            files: vec![File::dummy(target.to_string_lossy().into_owned())],
        })
    }

    async fn symlink(
        &mut self,
        id: u32,
        linkpath: String,
        targetpath: String,
    ) -> Result<Status, Self::Error> {
        std::os::unix::fs::symlink(&targetpath, &linkpath).map_err(map_io_err)?;
        Ok(ok_status(id))
    }
}
