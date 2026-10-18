//! The CA bundle a provider's environment names, made reachable inside the
//! sandbox (`add-agent-workload` 6.3; `add-linux-agent-fleet`'s "CA
//! certificates are present in the constructed view").
//!
//! **Measured, not anticipated:** flox sets `SSL_CERT_FILE=/usr/lib/ssl/
//! cert.pem` on this host, a symlink to `/etc/ssl/certs/ca-certificates.crt`.
//! `/etc` is not in the view, so the file is `ENOENT` inside, and every
//! closure tool that trusts OpenSSL's store (nixpkgs' node, curl, git,
//! python) fails TLS: npm reported `UNABLE_TO_GET_ISSUER_CERT_LOCALLY`
//! against `registry.npmjs.org`. A nix provider names a bundle in the store,
//! which is already granted; flox and devbox fall back to the host's, which
//! is how the host's own added CAs (a corporate proxy's, OrbStack's) keep
//! working, so substituting a closure bundle would trade one failure for
//! another.
//!
//! So the bundle is granted, read-only, with its own origin (`trust-store`)
//! so `policy --render` says why it is there, and the variable is rewritten
//! to the canonical path the grant is mounted at: a spelling through a
//! symlink in a directory the view lacks (`/etc/ssl/cert.pem`) would
//! otherwise still be missing.
//!
//! **What keeps this from being a way to read any host file** through an
//! environment variable: only the provider's environment is read, never
//! `[env.vars]`; the file must be a PEM bundle of certificates with no
//! private key in it; and nothing under the home directory is granted,
//! since no system trust store lives there. Certificates are public; that
//! is the whole of what this can expose.

use std::collections::BTreeMap;
use std::path::Path;

/// The variables a CA bundle is named by: OpenSSL's, nix's, and the ones
/// curl, git, Python's requests and Node read instead of or beside it.
const BUNDLE_VARS: [&str; 6] = [
    "SSL_CERT_FILE",
    "NIX_SSL_CERT_FILE",
    "CURL_CA_BUNDLE",
    "GIT_SSL_CAINFO",
    "REQUESTS_CA_BUNDLE",
    "NODE_EXTRA_CA_CERTS",
];

/// Read at most this much of a candidate: Mozilla's bundle is ~220 KiB.
const MAX_BUNDLE: u64 = 4 * 1024 * 1024;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct TrustStore {
    /// Canonical paths to grant read-only, each once.
    pub grants: Vec<String>,
    /// Variables to rewrite to the canonical path they name.
    pub rewrites: Vec<(String, String)>,
    /// One line each for a bundle named but not granted, and why: `up`
    /// prints them (degraded capabilities are surfaced, never silent).
    pub refused: Vec<String>,
}

impl TrustStore {
    /// Apply [`TrustStore::rewrites`] to `env`.
    pub fn rewrite(&self, env: &mut BTreeMap<String, String>) {
        for (key, value) in &self.rewrites {
            env.insert(key.clone(), value.clone());
        }
    }
}

/// What `env` (a provider's resolved environment) needs granted for the CA
/// bundles it names, given the grants the sandbox already has.
pub fn resolve(env: &BTreeMap<String, String>, granted: &[String]) -> TrustStore {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    resolve_with(env, granted, home.as_deref(), &|p| {
        let canonical = std::fs::canonicalize(p).ok()?;
        let file = std::fs::File::open(&canonical).ok()?;
        if !file.metadata().ok()?.is_file() {
            return None;
        }
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::Read::take(file, MAX_BUNDLE), &mut text)
            .ok()?;
        Some((canonical.to_string_lossy().into_owned(), text))
    })
}

/// [`resolve`] with the filesystem injected: `read` returns a path's
/// canonical spelling and its contents, `None` when it is not a readable
/// regular file.
fn resolve_with(
    env: &BTreeMap<String, String>,
    granted: &[String],
    home: Option<&Path>,
    read: &dyn Fn(&Path) -> Option<(String, String)>,
) -> TrustStore {
    let mut out = TrustStore::default();
    for var in BUNDLE_VARS {
        let Some(value) = env.get(var).filter(|v| v.starts_with('/')) else {
            continue;
        };
        // Decided on the canonical path alone: the measured spelling is a
        // symlink *inside* a granted directory (`/usr/lib`) to a file outside
        // every grant (`/etc/ssl/certs/…`).
        //
        // A bundle that does not exist fails the same way on the host; that
        // is not something the sandbox took away.
        let Some((canonical, text)) = read(Path::new(value)) else {
            continue;
        };
        if within_any(&canonical, granted) {
            if canonical != *value {
                out.rewrites.push((var.to_owned(), canonical));
            }
            continue;
        }
        let refusal = if home.is_some_and(|h| Path::new(&canonical).starts_with(h)) {
            Some("it is in the home directory, where no system trust store lives")
        } else if !text.contains("-----BEGIN CERTIFICATE-----") {
            Some("it holds no PEM certificate")
        } else if text.contains("PRIVATE KEY") {
            Some("it holds a private key")
        } else {
            None
        };
        if let Some(why) = refusal {
            out.refused.push(format!(
                "{var} names {value}, which is not granted: {why} (fallback: TLS inside the \
                 sandbox trusts what the tool bundles, if anything; grant it with \
                 [filesystem] read if it is meant)"
            ));
            continue;
        }
        if !out.grants.contains(&canonical) {
            out.grants.push(canonical.clone());
        }
        if canonical != *value {
            out.rewrites.push((var.to_owned(), canonical));
        }
    }
    out
}

fn within_any(path: &str, granted: &[String]) -> bool {
    granted
        .iter()
        .any(|g| g.starts_with('/') && crate::paths::is_within(path, g))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLE: &str =
        "## Mozilla roots\n-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";

    /// The host as measured: `/usr/lib/ssl/cert.pem` links to the Debian
    /// bundle; the home holds a certificate-shaped file and a key.
    fn read(p: &Path) -> Option<(String, String)> {
        let p = p.to_str()?;
        let (canonical, text) = match p {
            "/usr/lib/ssl/cert.pem" | "/etc/ssl/certs/ca-certificates.crt" => {
                ("/etc/ssl/certs/ca-certificates.crt", BUNDLE.to_owned())
            }
            "/nix/store/abc-nss-cacert/etc/ssl/certs/ca-bundle.crt" => (p, BUNDLE.to_owned()),
            "/home/u/.local/ca.pem" => (p, BUNDLE.to_owned()),
            "/etc/ssl/private/both.pem" => {
                (p, format!("{BUNDLE}-----BEGIN PRIVATE KEY-----\nAAAA\n"))
            }
            "/etc/hostname" => (p, "devcontainer\n".to_owned()),
            _ => return None,
        };
        Some((canonical.to_owned(), text))
    }

    fn of(pairs: &[(&str, &str)]) -> TrustStore {
        let env = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        resolve_with(
            &env,
            &["/nix/store".to_owned(), "/usr/lib".to_owned()],
            Some(Path::new("/home/u")),
            &read,
        )
    }

    /// The measured case: granted at its target, and the variable follows.
    #[test]
    fn the_host_bundle_flox_names_is_granted_at_its_target() {
        let t = of(&[
            ("SSL_CERT_FILE", "/usr/lib/ssl/cert.pem"),
            ("CURL_CA_BUNDLE", "/etc/ssl/certs/ca-certificates.crt"),
        ]);
        assert_eq!(t.grants, ["/etc/ssl/certs/ca-certificates.crt"]);
        assert_eq!(
            t.rewrites,
            [(
                "SSL_CERT_FILE".to_owned(),
                "/etc/ssl/certs/ca-certificates.crt".to_owned()
            )]
        );
        assert!(t.refused.is_empty());
    }

    /// A nix provider's bundle is in the store, already granted.
    #[test]
    fn a_bundle_inside_a_grant_adds_nothing() {
        let t = of(&[(
            "NIX_SSL_CERT_FILE",
            "/nix/store/abc-nss-cacert/etc/ssl/certs/ca-bundle.crt",
        )]);
        assert_eq!(t, TrustStore::default());
    }

    /// The guard: a variable is not a way to read an arbitrary host file.
    #[test]
    fn what_is_not_a_system_certificate_bundle_is_refused_and_said() {
        let t = of(&[
            ("SSL_CERT_FILE", "/home/u/.local/ca.pem"),
            ("CURL_CA_BUNDLE", "/etc/ssl/private/both.pem"),
            ("GIT_SSL_CAINFO", "/etc/hostname"),
            ("REQUESTS_CA_BUNDLE", "/nonexistent.pem"),
            ("NODE_EXTRA_CA_CERTS", "relative.pem"),
        ]);
        assert!(t.grants.is_empty(), "{t:?}");
        assert!(t.rewrites.is_empty(), "{t:?}");
        let refused = t.refused.join("\n");
        assert!(
            refused.contains("SSL_CERT_FILE names /home/u/.local/ca.pem"),
            "{refused}"
        );
        assert!(refused.contains("in the home directory"), "{refused}");
        assert!(refused.contains("holds a private key"), "{refused}");
        assert!(refused.contains("holds no PEM certificate"), "{refused}");
        // Missing or relative: nothing the sandbox took away.
        assert_eq!(t.refused.len(), 3, "{refused}");
    }
}
