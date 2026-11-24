//! Who a sandbox's processes run as (`add-sandbox-identity`): a name and ids
//! of the sandbox's own inside, and which host ids they stand for.
//!
//! Only `shared` exists today: the host side is the user who ran `up`. On
//! Linux a user namespace gives the inside its own name and ids; macOS maps
//! no ids, so there the inside is the host user, and `up` says so.

/// The default `[identity] user`.
pub const DEFAULT_USER: &str = "devcroft";

/// The inside uid and gid on Linux, the same in every sandbox.
pub const INSIDE_ID: u32 = 1000;

/// Which host identity the inside one stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// The user who ran `up`.
    Shared,
}

/// One sandbox's identity, as `up` decided it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SandboxIdentity {
    pub user: String,
    pub uid: u32,
    pub gid: u32,
    pub host_uid: u32,
    pub host_gid: u32,
    pub mode: Mode,
}

impl SandboxIdentity {
    /// `user` inside, the caller outside.
    pub fn shared(user: &str) -> Self {
        // SAFETY: plain getters.
        let (host_uid, host_gid) = unsafe { (libc::getuid(), libc::getgid()) };
        let (user, uid, gid) = if cfg!(target_os = "linux") {
            (user.to_owned(), INSIDE_ID, INSIDE_ID)
        } else {
            let host = host_user_name(host_uid).unwrap_or_else(|| host_uid.to_string());
            (host, host_uid, host_gid)
        };
        Self {
            user,
            uid,
            gid,
            host_uid,
            host_gid,
            mode: Mode::Shared,
        }
    }
}

impl std::fmt::Display for SandboxIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Mode::Shared = self.mode;
        write!(
            f,
            "{} (uid {}, gid {}), host uid {}, gid {}; mode: shared",
            self.user, self.uid, self.gid, self.host_uid, self.host_gid
        )
    }
}

/// `[identity] user`: a POSIX user name that is no system one the view's
/// `/etc/passwd` or the kernel gives another meaning.
pub fn valid_user(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 32
        && !matches!(name, "root" | "nobody")
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-".contains(c))
}

/// The host's name for `uid`, from the host's own `/etc/passwd` through libc.
fn host_user_name(uid: libc::uid_t) -> Option<String> {
    // SAFETY: `getpwuid` returns a pointer into static storage or null; the
    // name is copied out at once, and `up` is single-threaded here.
    unsafe {
        let pw = libc::getpwuid(uid);
        if pw.is_null() || (*pw).pw_name.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr((*pw).pw_name)
            .to_str()
            .ok()
            .map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_names() {
        for ok in ["devcroft", "dev", "_svc", "a-b_1"] {
            assert!(valid_user(ok), "{ok}");
        }
        for bad in ["", "root", "nobody", "Dev", "1a", "-a", "a:b", "a b", "a/b"] {
            assert!(!valid_user(bad), "{bad}");
        }
        assert!(!valid_user(&"a".repeat(33)));
    }

    #[test]
    fn nobody_inside_is_root_unless_the_host_user_is_and_there_is_no_mapping() {
        let id = SandboxIdentity::shared("dev");
        if cfg!(target_os = "linux") {
            assert_eq!((id.user.as_str(), id.uid, id.gid), ("dev", 1000, 1000));
        } else {
            assert_eq!(id.uid, id.host_uid);
        }
    }
}
