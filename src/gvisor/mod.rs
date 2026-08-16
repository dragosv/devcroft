//! The `gvisor-backend` capability: the `runsc` adapter behind
//! `isolation = "hardened"`. Concretizes `add-hardened-tier`'s
//! backend-generic tier abstraction the same way `add-nix-provider`
//! concretized `env-provider` — this module owns everything specific to
//! gVisor, while `crate::lifecycle`/`crate::keeper`/`crate::policy` stay
//! backend-agnostic.
//!
//! - [`oci_spec`] builds the OCI runtime `config.json` from a
//!   [`CompiledPolicy`](crate::policy::CompiledPolicy). Pure JSON
//!   generation, tested on every platform.
//! - [`runsc_command`] resolves the `runsc` binary and assembles its
//!   argument vectors. Likewise pure except for the availability probe.
//! - [`runner`] materializes the OCI bundle and drives `runsc` as real
//!   subprocesses; gated to `target_os = "linux"` since gVisor itself
//!   is Linux-only.
//! - [`session_backend`] implements `keeper::session::SessionBackend`
//!   over `runsc exec`, so sessions dispatched at this tier are
//!   indistinguishable in behavior from the process tier's local
//!   fork/exec (add-hardened-tier's whole point).

pub mod oci_spec;
pub mod runsc_command;
pub mod session_backend;

#[cfg(target_os = "linux")]
pub mod runner;

/// Which `runsc` platform a sandbox runs on. Systrap is the default
/// (gVisor's own default since mid-2023, needs no special host access);
/// KVM is selected only when actually usable, never just present.
/// ptrace is deprecated upstream and intentionally not a variant here —
/// `add-gvisor-backend`'s spec is explicit that it is not targeted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Systrap,
    Kvm,
}

impl Platform {
    pub(crate) fn runsc_flag(self) -> &'static str {
        match self {
            Platform::Systrap => "systrap",
            Platform::Kvm => "kvm",
        }
    }
}

/// Systrap by default; KVM only when `/dev/kvm` exists *and* this
/// process can actually open it — presence alone is not enough (a
/// device node can exist with permissions that make it unusable), and
/// `doctor`'s own platform probe (task 7.2) needs the same distinction
/// for a smoke check, not just this file check.
pub fn select_platform() -> Platform {
    if kvm_usable() {
        Platform::Kvm
    } else {
        Platform::Systrap
    }
}

fn kvm_usable() -> bool {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_flag_names_match_runsc_cli() {
        assert_eq!(Platform::Systrap.runsc_flag(), "systrap");
        assert_eq!(Platform::Kvm.runsc_flag(), "kvm");
    }

    #[test]
    fn select_platform_never_panics() {
        // Host-dependent (this devcontainer has no /dev/kvm today), but
        // the call itself must always resolve to *something* rather than
        // fail — mirrors doctor's own "probe, don't infer from presence
        // alone" posture.
        let _ = select_platform();
    }
}
