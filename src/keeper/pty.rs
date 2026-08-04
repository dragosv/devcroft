//! Pty allocation for `shell`-style sessions (task 4.1's `resize` verb;
//! wired to the CLI by task 5.2). Built on `openpty`/`TIOCSWINSZ`, which
//! POSIX/BSD `libc` exposes identically on Linux (glibc) and macOS — the
//! same two backends the spike (tasks 1.1/1.2) proved fd inheritance on —
//! so one implementation covers both. Only the Linux path has been
//! exercised live in this environment; the macOS path is written to the
//! same libc signatures and is unverified beyond compiling, matching the
//! posture already noted for flox in the devcontainer (task 3.2).

use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};

use super::protocol::PtySize;

pub struct OpenedPty {
    pub master: File,
    /// Still open in this process; the caller dup2s it onto the child's
    /// stdio in `pre_exec` and is responsible for closing this end
    /// afterward.
    pub slave_fd: RawFd,
}

/// Opens a pty pair sized to `size` up front, so no separate resize call
/// is needed before the child's first read of its window size.
pub fn open_pty(size: &PtySize) -> io::Result<OpenedPty> {
    let mut master_fd: libc::c_int = -1;
    let mut slave_fd: libc::c_int = -1;
    let ws = libc::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let ret = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &ws as *const libc::winsize as *mut libc::winsize,
        )
    };
    if ret != 0 {
        return Err(io::Error::last_os_error());
    }

    // Neither end should leak across an unrelated exec: the slave is
    // dup2'd onto 0/1/2 explicitly in pre_exec (dup2'd copies are never
    // cloexec themselves), and the master must never reach the child at
    // all.
    if let Err(e) = set_cloexec(master_fd).and_then(|()| set_cloexec(slave_fd)) {
        unsafe {
            libc::close(master_fd);
            libc::close(slave_fd);
        }
        return Err(e);
    }

    let master = unsafe { File::from_raw_fd(master_fd) };
    Ok(OpenedPty { master, slave_fd })
}

fn set_cloexec(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn resize(master: &File, size: &PtySize) -> io::Result<()> {
    let ws = libc::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let ret = unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &ws) };
    if ret != 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn open_pty_allocates_a_working_master_slave_pair() {
        let mut pty = open_pty(&PtySize { rows: 24, cols: 80 }).unwrap();
        // Takes ownership of slave_fd: its Drop closes the fd exactly once.
        let mut slave = unsafe { File::from_raw_fd(pty.slave_fd) };

        slave.write_all(b"hello from slave\n").unwrap();
        let mut buf = [0u8; 64];
        let n = pty.master.read(&mut buf).unwrap();
        // Default (cooked-mode) termios settings apply ONLCR: a bare `\n`
        // written by the slave side arrives at the master as `\r\n`, same
        // as any real terminal.
        assert_eq!(&buf[..n], b"hello from slave\r\n");
    }

    #[test]
    fn resize_reports_updated_window_size() {
        let pty = open_pty(&PtySize { rows: 24, cols: 80 }).unwrap();
        resize(
            &pty.master,
            &PtySize {
                rows: 50,
                cols: 120,
            },
        )
        .unwrap();

        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::ioctl(pty.master.as_raw_fd(), libc::TIOCGWINSZ, &mut ws) };
        assert_eq!(ret, 0);
        assert_eq!(ws.ws_row, 50);
        assert_eq!(ws.ws_col, 120);

        unsafe { libc::close(pty.slave_fd) };
    }
}
