//! Post-boot direct credential delivery, refresh/new-instance drop semantics,
//! and shared-base promotion refusal (`add-vm-project-runtime` task 3.4).

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::contract::{ErrorLayer, is_valid_hex};
use super::transport::VmControlClient;

/// Single direct credential pushed post-boot over the scoped control channel.
/// Debug formatting always redacts `value`.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct DirectCredential {
    pub name: String,
    pub value: String,
}

impl fmt::Debug for DirectCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirectCredential")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// Validated bundle of credentials authorized explicitly for one instance.
#[derive(Clone, PartialEq, Eq)]
pub struct DirectCredentialDelivery {
    pub credentials: Vec<DirectCredential>,
}

impl fmt::Debug for DirectCredentialDelivery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirectCredentialDelivery")
            .field("credentials", &self.credentials)
            .finish()
    }
}

/// Persisted credential exposure status recording only names, never values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceCredentialState {
    pub instance_id: String,
    pub source_revision_sha256: String,
    pub delivered_names: Vec<String>,
    pub ever_held_direct_credentials: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialError {
    PreBootDeliveryForbidden,
    InvalidCredential(String),
    DeliveryFailed(String),
    DiskTaintedByDirectCredentials(String),
}

impl CredentialError {
    pub const fn layer(&self) -> ErrorLayer {
        match self {
            CredentialError::DeliveryFailed(_) => ErrorLayer::Keeper,
            _ => ErrorLayer::Config,
        }
    }
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CredentialError::PreBootDeliveryForbidden => write!(
                f,
                "direct credentials can only be delivered after boot (generation >= 1)"
            ),
            CredentialError::InvalidCredential(msg) => {
                write!(f, "invalid credential delivery: {msg}")
            }
            CredentialError::DeliveryFailed(msg) => {
                write!(f, "post-boot credential delivery failed: {msg}")
            }
            CredentialError::DiskTaintedByDirectCredentials(id) => write!(
                f,
                "refusing promotion of instance `{id}` to a shared fleet base: disk held direct credentials; clean reconstruction is required"
            ),
        }
    }
}

impl std::error::Error for CredentialError {}

fn is_valid_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

impl DirectCredentialDelivery {
    pub fn new(credentials: Vec<DirectCredential>) -> Result<Self, CredentialError> {
        if credentials.is_empty() {
            return Err(CredentialError::InvalidCredential(
                "credential delivery must contain at least one entry".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        for c in &credentials {
            if !is_valid_env_name(&c.name) {
                return Err(CredentialError::InvalidCredential(format!(
                    "`{}` is not a valid environment variable name",
                    c.name
                )));
            }
            if c.value.is_empty() {
                return Err(CredentialError::InvalidCredential(format!(
                    "credential `{}` has empty value",
                    c.name
                )));
            }
            if !seen.insert(c.name.clone()) {
                return Err(CredentialError::InvalidCredential(format!(
                    "duplicate credential `{}`",
                    c.name
                )));
            }
        }
        Ok(Self { credentials })
    }
}

/// Deliver authorized credentials post-boot (`generation >= 1`) over
/// `control_socket`, persisting only variable names and exposure taint.
pub fn deliver_post_boot_credentials(
    control_socket: &Path,
    generation: u64,
    delivery: &DirectCredentialDelivery,
    state: &mut InstanceCredentialState,
    timeout: Duration,
) -> Result<(), CredentialError> {
    if generation == 0 {
        return Err(CredentialError::PreBootDeliveryForbidden);
    }
    let payload_json = serde_json::to_string(&delivery.credentials)
        .map_err(|e| CredentialError::InvalidCredential(e.to_string()))?;
    let cmd = format!("CRED-SET {generation} {payload_json}");
    let reply = VmControlClient::new(control_socket, timeout)
        .send_command(&cmd)
        .map_err(|e| CredentialError::DeliveryFailed(e.to_string()))?;
    let expected = format!("CRED-OK {generation}");
    if reply.trim() != expected {
        return Err(CredentialError::DeliveryFailed(format!(
            "unexpected credential acknowledgement `{reply}`; expected `{expected}`"
        )));
    }
    for c in &delivery.credentials {
        if !state.delivered_names.contains(&c.name) {
            state.delivered_names.push(c.name.clone());
        }
    }
    state.ever_held_direct_credentials = true;
    Ok(())
}

/// Drop all active direct credentials when creating a new instance or when a
/// refresh changes the source revision; preserve disk taint when the existing
/// instance disk is kept.
pub fn reset_credentials_for_new_instance_or_refresh(
    prev: &InstanceCredentialState,
    new_instance_id: &str,
    new_source_sha256: &str,
) -> Result<InstanceCredentialState, CredentialError> {
    if !is_valid_hex(new_source_sha256, 64) {
        return Err(CredentialError::InvalidCredential(
            "source_revision_sha256 must be 64 lowercase hex chars".into(),
        ));
    }
    let same_instance = new_instance_id == prev.instance_id;
    let same_source = new_source_sha256 == prev.source_revision_sha256;
    let delivered_names = if same_instance && same_source {
        prev.delivered_names.clone()
    } else {
        Vec::new()
    };
    let ever_held_direct_credentials = if same_instance {
        prev.ever_held_direct_credentials
    } else {
        false
    };
    Ok(InstanceCredentialState {
        instance_id: new_instance_id.to_owned(),
        source_revision_sha256: new_source_sha256.to_owned(),
        delivered_names,
        ever_held_direct_credentials,
    })
}

/// Refuse promoting any writable session disk that ever received direct
/// credentials into a shared base image.
pub fn guard_shared_base_promotion(state: &InstanceCredentialState) -> Result<(), CredentialError> {
    if state.ever_held_direct_credentials {
        return Err(CredentialError::DiskTaintedByDirectCredentials(
            state.instance_id.clone(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::thread;

    const SRC_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SRC_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn post_boot_delivery_redacts_secrets_and_gates_base_promotion_and_refresh() {
        let dir = std::env::temp_dir().join(format!("dc-vm-cred-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("control.sock");
        let listener = UnixListener::bind(&sock).unwrap();

        thread::spawn(move || {
            if let Ok((conn, _)) = listener.accept() {
                let mut reader = BufReader::new(conn.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_ok() {
                    let mut w = conn;
                    let _ = w.write_all(b"CRED-OK 4\n");
                }
            }
        });

        let secret_val = "sk-ant-very-secret-test-token-value";
        let delivery = DirectCredentialDelivery::new(vec![DirectCredential {
            name: "ANTHROPIC_API_KEY".into(),
            value: secret_val.into(),
        }])
        .unwrap();

        let debug_str = format!("{delivery:?}");
        assert!(!debug_str.contains(secret_val));
        assert!(debug_str.contains("<redacted>"));

        let mut state = InstanceCredentialState {
            instance_id: "inst-1".into(),
            source_revision_sha256: SRC_A.into(),
            delivered_names: Vec::new(),
            ever_held_direct_credentials: false,
        };

        assert!(matches!(
            deliver_post_boot_credentials(&sock, 0, &delivery, &mut state, Duration::from_secs(2)),
            Err(CredentialError::PreBootDeliveryForbidden)
        ));

        deliver_post_boot_credentials(&sock, 4, &delivery, &mut state, Duration::from_secs(2))
            .unwrap();
        assert_eq!(state.delivered_names, vec!["ANTHROPIC_API_KEY"]);
        assert!(state.ever_held_direct_credentials);
        let serialized = serde_json::to_string(&state).unwrap();
        assert!(!serialized.contains(secret_val));

        // Direct credential exposure blocks promoting this disk as a shared base.
        assert!(matches!(
            guard_shared_base_promotion(&state),
            Err(CredentialError::DiskTaintedByDirectCredentials(_))
        ));

        // Source refresh drops delivered credentials while retaining taint for that disk.
        let refreshed =
            reset_credentials_for_new_instance_or_refresh(&state, "inst-1", SRC_B).unwrap();
        assert!(refreshed.delivered_names.is_empty());
        assert!(refreshed.ever_held_direct_credentials);

        // New instance never inherits credentials from project identity match.
        let new_inst =
            reset_credentials_for_new_instance_or_refresh(&state, "inst-2", SRC_A).unwrap();
        assert!(new_inst.delivered_names.is_empty());
        assert!(!new_inst.ever_held_direct_credentials);
        guard_shared_base_promotion(&new_inst).unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }
}
