//! Editor client trust classification and `ssh-config` emission gating
//! (`add-vm-project-runtime` R8, task 2.7).

use std::path::Path;

use super::grants::{ProjectIdentity, RuntimeGrantStore};
use super::ssh::{
    LocalPortForward, VmSshError, build_vm_ssh_client_args, render_vm_ssh_config_host,
};

/// Client workflows connecting to a VM project instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorClientKind {
    TerminalSsh,
    Sftp,
    GuestServedBrowser { guest_port: u16 },
    VsCodeRemoteSsh,
    CursorRemoteSsh,
    JetBrainsGateway,
    Zed,
}

/// Risk classification relative to `add-vm-project-runtime`'s untrusted claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorTrustClass {
    UntrustedSafe,
    RequiresTrustedProjectGrant,
    Unclassified,
}

/// Classify an editor/client route by what a hostile guest can do to the host.
pub const fn classify_editor(kind: EditorClientKind) -> EditorTrustClass {
    match kind {
        EditorClientKind::TerminalSsh
        | EditorClientKind::Sftp
        | EditorClientKind::GuestServedBrowser { .. } => EditorTrustClass::UntrustedSafe,
        EditorClientKind::VsCodeRemoteSsh | EditorClientKind::CursorRemoteSsh => {
            EditorTrustClass::RequiresTrustedProjectGrant
        }
        EditorClientKind::JetBrainsGateway | EditorClientKind::Zed => {
            EditorTrustClass::Unclassified
        }
    }
}

/// Result of querying `ssh-config` for a VM instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmSshConfigOutcome {
    Emitted(String),
    Omitted {
        reason: String,
        guest_served_editor_hint: String,
        grant_hint: String,
    },
}

/// Gate VM `ssh-config` entries on an explicit user-owned trusted-editor grant
/// (`R8`): untrusted VM projects receive no Remote-SSH host block, and the
/// response names the guest-served editor alternative and the required grant.
pub fn ssh_config_for_vm_instance(
    sandbox_name: &str,
    project: &ProjectIdentity,
    grants: &RuntimeGrantStore,
    proxy_command: &str,
    identity_file: &Path,
    known_hosts: &Path,
) -> VmSshConfigOutcome {
    if !grants.has_trusted_editor_grant(project) {
        return VmSshConfigOutcome::Omitted {
            reason: format!(
                "omitted Remote-SSH host entry for `{sandbox_name}` because `{}` is not marked trusted-editor",
                project.as_path().display()
            ),
            guest_served_editor_hint: format!(
                "use guest-served browser editor over local loopback forward (`devcroft ssh {sandbox_name} -N -L 127.0.0.1:<local>:127.0.0.1:<guest>`)"
            ),
            grant_hint: format!(
                "devcroft runtime grant-trusted-editor --project {}",
                project.as_path().display()
            ),
        };
    }
    let host_alias = format!("{sandbox_name}.devcroft");
    VmSshConfigOutcome::Emitted(render_vm_ssh_config_host(
        &host_alias,
        proxy_command,
        identity_file,
        known_hosts,
    ))
}

/// Hardened SSH `-N -L 127.0.0.1:<local>:127.0.0.1:<guest>` tunnel arguments for
/// a guest-served editor in the untrusted-project path.
pub fn guest_served_editor_forward_args(
    host_alias: &str,
    proxy_command: &str,
    identity_file: &Path,
    known_hosts: &Path,
    local_port: u16,
    guest_port: u16,
) -> Result<Vec<String>, VmSshError> {
    let mut args = build_vm_ssh_client_args(
        host_alias,
        proxy_command,
        identity_file,
        known_hosts,
        &[LocalPortForward {
            local_port,
            guest_port,
        }],
    )?;
    // Insert `-N` right before the trailing destination host alias so SSH
    // runs only the loopback tunnel without requesting a remote shell.
    let tail = args.pop().expect("host_alias present");
    args.push("-N".into());
    args.push(tail);
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_classification_distinguishes_untrusted_trusted_and_unclassified() {
        assert_eq!(
            classify_editor(EditorClientKind::TerminalSsh),
            EditorTrustClass::UntrustedSafe
        );
        assert_eq!(
            classify_editor(EditorClientKind::Sftp),
            EditorTrustClass::UntrustedSafe
        );
        assert_eq!(
            classify_editor(EditorClientKind::GuestServedBrowser { guest_port: 9090 }),
            EditorTrustClass::UntrustedSafe
        );
        assert_eq!(
            classify_editor(EditorClientKind::VsCodeRemoteSsh),
            EditorTrustClass::RequiresTrustedProjectGrant
        );
        assert_eq!(
            classify_editor(EditorClientKind::CursorRemoteSsh),
            EditorTrustClass::RequiresTrustedProjectGrant
        );
        assert_eq!(
            classify_editor(EditorClientKind::JetBrainsGateway),
            EditorTrustClass::Unclassified
        );
        assert_eq!(
            classify_editor(EditorClientKind::Zed),
            EditorTrustClass::Unclassified
        );
    }

    #[test]
    fn ssh_config_omits_untrusted_projects_and_gates_trusted_block() {
        let project = ProjectIdentity::from_root(Path::new("/work/untrusted-app")).unwrap();
        let mut store = RuntimeGrantStore::default();
        let id = Path::new("/ctrl/id_ed25519");
        let kh = Path::new("/ctrl/known_hosts");

        let omitted =
            ssh_config_for_vm_instance("demo", &project, &store, "devcroft vm-proxy demo", id, kh);
        match omitted {
            VmSshConfigOutcome::Omitted {
                reason,
                guest_served_editor_hint,
                grant_hint,
            } => {
                assert!(reason.contains("demo"));
                assert!(guest_served_editor_hint.contains("guest-served"));
                assert!(grant_hint.contains("grant-trusted-editor"));
            }
            VmSshConfigOutcome::Emitted(_) => {
                panic!("untrusted project must omit ssh-config block")
            }
        }

        store.grant_trusted_editor(&project, "2026-11-26T08:52:00Z");
        let emitted =
            ssh_config_for_vm_instance("demo", &project, &store, "devcroft vm-proxy demo", id, kh);
        match emitted {
            VmSshConfigOutcome::Emitted(block) => {
                assert!(block.contains("Host demo.devcroft"));
                assert!(block.contains("ForwardAgent no"));
                assert!(block.contains("ForwardX11 no"));
                assert!(block.contains("ClearAllForwardings yes"));
            }
            VmSshConfigOutcome::Omitted { .. } => panic!("trusted project should emit block"),
        }

        let tunnel_args = guest_served_editor_forward_args(
            "demo.devcroft",
            "devcroft vm-proxy demo",
            id,
            kh,
            19090,
            9090,
        )
        .unwrap();
        assert!(tunnel_args.contains(&"-N".to_string()));
        assert!(tunnel_args.contains(&"127.0.0.1:19090:127.0.0.1:9090".to_string()));
        assert_eq!(
            tunnel_args.last().map(String::as_str),
            Some("demo.devcroft")
        );
    }
}
