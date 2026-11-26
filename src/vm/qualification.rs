//! Platform qualification table for the experimental/opt-in microVM runtime
//! (`add-vm-project-runtime` task 1.1).

use super::contract::{GuestTarget, VmAdapter};

/// Accepted evaluation report backing platform qualification decisions.
pub const ACCEPTED_EVALUATION_REPORT: &str = "results/macos-arm64-evaluation-assessment-v16.json";

/// Host OS/architecture combinations recognized by VM qualification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPlatform {
    MacosArm64,
    LinuxArm64,
    LinuxX86_64,
    MacosX86_64,
    Other,
}

impl HostPlatform {
    pub const fn as_str(&self) -> &'static str {
        match self {
            HostPlatform::MacosArm64 => "macos-arm64",
            HostPlatform::LinuxArm64 => "linux-arm64",
            HostPlatform::LinuxX86_64 => "linux-x86_64",
            HostPlatform::MacosX86_64 => "macos-x86_64",
            HostPlatform::Other => "unsupported",
        }
    }
}

/// Returns the current compile-time host platform.
pub const fn current_host_platform() -> HostPlatform {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        HostPlatform::MacosArm64
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        HostPlatform::LinuxArm64
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        HostPlatform::LinuxX86_64
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        HostPlatform::MacosX86_64
    } else {
        HostPlatform::Other
    }
}

/// Whether a host platform is qualified for the opt-in microVM runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualificationStatus {
    QualifiedOptIn {
        adapter: VmAdapter,
        guest_target: GuestTarget,
        min_os: &'static str,
        required_controls: &'static [&'static str],
    },
    Unqualified {
        reason: &'static str,
        regression_requirement: &'static str,
    },
}

/// Recorded platform qualification decision bound to `ACCEPTED_EVALUATION_REPORT`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformQualification {
    pub host: HostPlatform,
    pub status: QualificationStatus,
    pub accepted_assessment: &'static str,
}

impl PlatformQualification {
    pub fn is_qualified(&self) -> bool {
        matches!(self.status, QualificationStatus::QualifiedOptIn { .. })
    }
}

const MACOS_ARM64_CONTROLS: &[&str] = &[
    "pre-vcpu-seatbelt",
    "guest-nono-landlock",
    "no-nic-no-tsi",
    "private-ext4-disks",
];

/// Returns the accepted platform qualification status for `host`.
///
/// Only `macos-arm64` is qualified (opt-in `libkrun` 2.0 on macOS 14+); all
/// other rows remain explicitly unqualified (`not-run`) so no host silently
/// inherits another architecture's or nested row's results.
pub fn qualification_for(host: HostPlatform) -> PlatformQualification {
    let status = match host {
        HostPlatform::MacosArm64 => QualificationStatus::QualifiedOptIn {
            adapter: VmAdapter::Libkrun,
            guest_target: GuestTarget::LinuxArm64,
            min_os: "macOS 14+",
            required_controls: MACOS_ARM64_CONTROLS,
        },
        HostPlatform::LinuxArm64 => QualificationStatus::Unqualified {
            reason: "no dedicated native Linux ARM64 runner provisioned (nested VZ/KVM diagnostics do not qualify native Linux)",
            regression_requirement: "dedicated native Linux ARM64 host with KVM, Landlock ABI >= 6, pre-vCPU E3 seccomp+Landlock, and non-skipping VM tests",
        },
        HostPlatform::LinuxX86_64 => QualificationStatus::Unqualified {
            reason: "no dedicated native Linux x86-64 runner provisioned; architecture-matched kernel/closure and E3 syscall filter not run",
            regression_requirement: "dedicated native Linux x86-64 host with KVM, Landlock ABI >= 6, pre-vCPU E3 seccomp+Landlock, and non-skipping VM tests",
        },
        HostPlatform::MacosX86_64 => QualificationStatus::Unqualified {
            reason: "no native Intel Mac host provisioned in evaluation matrix",
            regression_requirement: "native Intel Mac runner qualifying libkrun support without silent fallback",
        },
        HostPlatform::Other => QualificationStatus::Unqualified {
            reason: "unsupported host operating system or CPU architecture",
            regression_requirement: "explicit hardware matrix row and full E1-E5 qualification",
        },
    };
    PlatformQualification {
        host,
        status,
        accepted_assessment: ACCEPTED_EVALUATION_REPORT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_arm64_qualifies_libkrun_linux_arm64_opt_in() {
        let q = qualification_for(HostPlatform::MacosArm64);
        assert!(q.is_qualified());
        assert_eq!(q.accepted_assessment, ACCEPTED_EVALUATION_REPORT);
        match q.status {
            QualificationStatus::QualifiedOptIn {
                adapter,
                guest_target,
                min_os,
                required_controls,
            } => {
                assert_eq!(adapter, VmAdapter::Libkrun);
                assert_eq!(guest_target, GuestTarget::LinuxArm64);
                assert_eq!(min_os, "macOS 14+");
                assert!(required_controls.contains(&"pre-vcpu-seatbelt"));
                assert!(required_controls.contains(&"guest-nono-landlock"));
                assert!(required_controls.contains(&"no-nic-no-tsi"));
                assert!(required_controls.contains(&"private-ext4-disks"));
            }
            QualificationStatus::Unqualified { .. } => panic!("expected qualified macos-arm64 row"),
        }
    }

    #[test]
    fn unmeasured_platforms_remain_explicitly_unqualified() {
        for host in [
            HostPlatform::LinuxArm64,
            HostPlatform::LinuxX86_64,
            HostPlatform::MacosX86_64,
            HostPlatform::Other,
        ] {
            let q = qualification_for(host);
            assert!(!q.is_qualified(), "{} must not be qualified", host.as_str());
            assert!(matches!(q.status, QualificationStatus::Unqualified { .. }));
        }
    }
}
