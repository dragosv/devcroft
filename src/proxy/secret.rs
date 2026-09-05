//! Resolving a `[[broker]]` entry's `secret` indirection, host-side at `up`
//! (`adopt-nono-proxy` D4, task 3.2).
//!
//! **Deliberately the narrowest thing that works.** `nono-proxy` can load from
//! a system keystore or 1Password; devcroft takes neither here. A keystore is a
//! product decision with its own platform matrix (Keychain, Secret Service,
//! DBus) and its own dependency tail, and adopting one inside a proxy change
//! would be the second unannounced adoption in a single commit. This shape is
//! reversible; a keystore is not.
//!
//! **Resolution runs in the trusted phase** — host-side at `up`, before any
//! boundary exists, alongside provider resolution and under the same trust
//! assumption. Nothing here ever runs inside the sandbox.
//!
//! What this protects, stated exactly: the secret never enters the *sandbox*.
//! It is still readable by anything running as the same host user that can
//! inspect the proxy process, which is the same exposure
//! `DEVCROFT_EGRESS_TOKEN` already has. Brokering is not a defence against the
//! user's own account.

use std::fmt;

#[derive(Debug)]
pub enum SecretError {
    /// The indirection named a scheme devcroft does not implement. Listed
    /// rather than described, so a typo and an unsupported source read
    /// differently.
    UnknownScheme { spec: String },
    /// The scheme is known and the source held nothing.
    Missing { spec: String, detail: String },
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretError::UnknownScheme { spec } => write!(
                f,
                "`{spec}` is not a supported secret reference; devcroft understands `env:NAME`"
            ),
            SecretError::Missing { spec, detail } => write!(f, "`{spec}`: {detail}"),
        }
    }
}

/// Resolves one `secret = "..."` reference to its value.
///
/// The only scheme is `env:NAME`, read from **devcroft's own** environment at
/// `up` — the host user's shell, not the sandbox's. An empty value counts as
/// missing: a variable set to `""` is far more often an unset variable that
/// went through a shell than a deliberate empty credential, and treating it as
/// present would defer the failure to first use, which is exactly what task 3.2
/// exists to prevent.
pub fn resolve(spec: &str) -> Result<String, SecretError> {
    let Some(name) = spec.strip_prefix("env:") else {
        return Err(SecretError::UnknownScheme {
            spec: spec.to_string(),
        });
    };
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => Ok(v),
        Ok(_) => Err(SecretError::Missing {
            spec: spec.to_string(),
            detail: format!("${name} is set but empty"),
        }),
        Err(_) => Err(SecretError::Missing {
            spec: spec.to_string(),
            detail: format!("${name} is not set in this shell"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_scheme_is_distinguished_from_a_missing_value() {
        // The distinction matters because the remedies differ: one is a typo
        // in the manifest, the other is a missing export.
        assert!(matches!(
            resolve("op://vault/item"),
            Err(SecretError::UnknownScheme { .. })
        ));
        assert!(matches!(
            resolve("env:DEVCROFT_TEST_DEFINITELY_UNSET_VAR"),
            Err(SecretError::Missing { .. })
        ));
    }

    #[test]
    fn a_bare_name_is_not_silently_treated_as_an_env_reference() {
        // Accepting `secret = "ANTHROPIC_API_KEY"` would make the scheme
        // optional, and an unschemed value is far likelier to be someone
        // pasting the credential itself into a committed file.
        assert!(matches!(
            resolve("ANTHROPIC_API_KEY"),
            Err(SecretError::UnknownScheme { .. })
        ));
    }
}
