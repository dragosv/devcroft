//! The base runtime's utilities, linked into `devcroft` and dispatched on
//! `argv[0]` (`add-own-userland` D1).
//!
//! Each applet runs through upstream's own entry point, `uucore::bin!`, in
//! a process of its own: the macro's `main` sets up localisation (without
//! it, help and diagnostics print translation keys), keeps an inherited
//! `SIGPIPE` disposition, mutes the panic a closed pipe would otherwise
//! print, flushes stdout, and exits with the utility's status. Calling
//! `uumain` directly would skip all of that, and running it inside the
//! keeper would let it mutate the keeper's environment and exit it.
//!
//! The module names differ from the crate names on purpose: `bin!` calls
//! `$util::uumain`, which must resolve to the crate, and `true`, `false`
//! and `test` cannot name a module.

/// The upstream project and version every applet below comes from. Part of
/// the runtime generation ([`super::GENERATION`]): changing it is a new
/// generation.
pub const UPSTREAM: &str = "uutils coreutils";
/// The pinned version, as `Cargo.toml` pins each `uu_*` crate (`=`).
pub const VERSION: &str = "0.13.0";

/// One utility in the runtime.
#[derive(Debug, Clone, Copy)]
pub struct Applet {
    /// The name it is invoked as, and the basename of its link.
    pub name: &'static str,
    /// The crate that implements it.
    pub krate: &'static str,
    main: fn(),
}

macro_rules! applets {
    ($(($module:ident, $krate:ident, $name:literal)),* $(,)?) => {
        mod entry {
            $(
                pub mod $module {
                    ::uucore::bin!($krate);
                }
            )*
        }

        /// Every applet, in [`super::BASE_NAMES`] order.
        pub const INVENTORY: &[Applet] = &[
            $(
                Applet {
                    name: $name,
                    krate: stringify!($krate),
                    main: entry::$module::main,
                },
            )*
        ];
    };
}

applets![
    (a_env, uu_env, "env"),
    (a_ls, uu_ls, "ls"),
    (a_cat, uu_cat, "cat"),
    (a_cp, uu_cp, "cp"),
    (a_mv, uu_mv, "mv"),
    (a_rm, uu_rm, "rm"),
    (a_mkdir, uu_mkdir, "mkdir"),
    (a_rmdir, uu_rmdir, "rmdir"),
    (a_ln, uu_ln, "ln"),
    (a_touch, uu_touch, "touch"),
    (a_head, uu_head, "head"),
    (a_tail, uu_tail, "tail"),
    (a_wc, uu_wc, "wc"),
    (a_sort, uu_sort, "sort"),
    (a_uniq, uu_uniq, "uniq"),
    (a_cut, uu_cut, "cut"),
    (a_tr, uu_tr, "tr"),
    (a_echo, uu_echo, "echo"),
    (a_printf, uu_printf, "printf"),
    (a_test, uu_test, "test"),
    (a_pwd, uu_pwd, "pwd"),
    (a_true, uu_true, "true"),
    (a_false, uu_false, "false"),
    (a_seq, uu_seq, "seq"),
    (a_basename, uu_basename, "basename"),
    (a_dirname, uu_dirname, "dirname"),
    (a_readlink, uu_readlink, "readlink"),
    (a_realpath, uu_realpath, "realpath"),
    (a_stat, uu_stat, "stat"),
    (a_chmod, uu_chmod, "chmod"),
    (a_mktemp, uu_mktemp, "mktemp"),
    (a_tee, uu_tee, "tee"),
    (a_date, uu_date, "date"),
    (a_id, uu_id, "id"),
    (a_uname, uu_uname, "uname"),
    (a_hostname, uu_hostname, "hostname"),
    (a_yes, uu_yes, "yes"),
    (a_sleep, uu_sleep, "sleep"),
    (a_nohup, uu_nohup, "nohup"),
];

/// The applet `name` names, if the runtime has one.
pub fn find(name: &str) -> Option<&'static Applet> {
    INVENTORY.iter().find(|a| a.name == name)
}

/// When this process was started through a link named after an applet,
/// run that applet and exit with its status; otherwise return.
///
/// The first statement of `devcroft`'s `main`, before any argument is
/// parsed, so the published command surface is untouched: `devcroft ls`
/// has `devcroft` as its basename and falls through to the normal command
/// path, where `ls` is an unknown command.
pub fn dispatch_argv0() {
    let Some(argv0) = std::env::args_os().next() else {
        return;
    };
    let Some(base) = std::path::Path::new(&argv0)
        .file_name()
        .and_then(|b| b.to_str())
    else {
        return;
    };
    // A login shell's `argv[0]` is `-sh`.
    if base.strip_prefix('-').unwrap_or(base) == super::sh::NAME {
        super::sh::main();
    }
    if let Some(applet) = find(base) {
        // `bin!`'s `main` always exits.
        (applet.main)();
        unreachable!("{} returned instead of exiting", applet.name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inventory_is_the_coreutils_floor_of_the_base_names() {
        let floor: Vec<&str> = super::super::BASE_NAMES
            .iter()
            .copied()
            .filter(|n| find(n).is_some())
            .collect();
        let inventory: Vec<&str> = INVENTORY.iter().map(|a| a.name).collect();
        assert_eq!(floor, inventory, "inventory order follows BASE_NAMES");
        assert_eq!(INVENTORY.len(), 39);
        for a in INVENTORY {
            assert_eq!(a.krate, format!("uu_{}", a.name));
        }
    }

    #[test]
    fn devcroft_itself_is_not_an_applet() {
        assert!(find("devcroft").is_none());
        assert!(find("sh").is_none(), "the shell is not an applet");
    }
}
