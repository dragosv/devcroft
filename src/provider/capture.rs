//! Shared activation-capture machinery for every `Provider` implementation:
//! the fixed baseline environment, the `env -0` diff/unset derivation, the
//! store-root grant extraction, and the fingerprint hash. Extracted from
//! `flox.rs` (task 1.1) so the determinism guarantee `canonical_base_env`
//! encodes — see its own doc comment — is enforced once for every
//! provider, not re-derived per implementation.

use super::ProviderError;
use std::collections::BTreeMap;

/// Nix/flox's default store root. Overridden per-resolution when the
/// activated `PATH` names a different one (see [`store_grants`]).
pub(super) const DEFAULT_STORE_ROOT: &str = "/nix/store";

/// What activation runs with — see [`super::flox`]/[`super::nix`]'s own
/// capture functions. A conventional Linux default `PATH` (Debian's,
/// notably): a provider's own hook scripts (`mkdir`, `cd`, etc.) need
/// *some* POSIX baseline to bootstrap from, same as any shell script
/// assumes `/bin/sh` exists; this does not depend on it being exactly
/// this list, just *a* sane one, fixed rather than inherited.
pub(super) const CANONICAL_PATH: &str =
    "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// The fixed environment every provider's activation runs with, and the
/// baseline [`changed_env`]/[`unset_env`] compare against — the same map
/// for both, so the diff reflects exactly what activation changed and
/// nothing else.
///
/// Found via review: this used to be `std::env::vars()` — literally
/// whatever shell happened to run `up` — for both roles. A `PATH` full of
/// a particular operator's own tools (nvm, rustup, cargo, ad hoc `~/bin`,
/// ...) leaking into either side means the exact same manifest can
/// resolve a different activation diff depending on who ran `up` and from
/// which shell, which is exactly the kind of non-reproducibility
/// CLAUDE.md's "no non-reproducible mode" framing rule rules out — the
/// manifest and lockfile are supposed to be the entire input.
///
/// `HOME` stays real: a provider's own state/cache/credentials
/// legitimately live there and are user-specific by design, not something
/// this determinism guarantee is about. Everything else is fixed.
pub(super) fn canonical_base_env() -> Result<BTreeMap<String, String>, ProviderError> {
    let home = std::env::var("HOME").map_err(|_| {
        ProviderError::ResolutionFailed("HOME is not set; provider activation needs it".to_string())
    })?;
    Ok(BTreeMap::from([
        ("HOME".to_string(), home),
        ("PATH".to_string(), CANONICAL_PATH.to_string()),
    ]))
}

/// Parse `env -0` output: NUL-separated `KEY=VALUE` entries.
pub(super) fn parse_env_dump(raw: &[u8]) -> BTreeMap<String, String> {
    String::from_utf8_lossy(raw)
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| entry.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Entries added or changed by activation — see [`unset_env`] for entries
/// activation removed instead, which this deliberately excludes: a
/// `BTreeMap<String, String>` has no way to represent "unset", only "set
/// to this value", so folding a removal into this map would either drop it
/// silently (the bug [`unset_env`] closes) or require a sentinel value.
pub(super) fn changed_env(
    baseline: &BTreeMap<String, String>,
    activated: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    activated
        .iter()
        .filter(|(k, v)| baseline.get(*k) != Some(*v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Baseline keys activation's output no longer has at all — as opposed to
/// changed (a key both sides have, with a different value, which
/// [`changed_env`] already covers). The caller (`up`) must explicitly
/// remove these from the keeper's own process environment rather than
/// merely not setting them, since the keeper process otherwise inherits
/// whatever `up`'s own ambient environment happened to hold for that key.
pub(super) fn unset_env(
    baseline: &BTreeMap<String, String>,
    activated: &BTreeMap<String, String>,
) -> Vec<String> {
    baseline
        .keys()
        .filter(|k| !activated.contains_key(*k))
        .cloned()
        .collect()
}

/// The read-only store root(s) the compiled policy must grant for the
/// activated toolchain to run (spec: "Store paths become readable"). Reads
/// the store root out of the activated `PATH` so a non-default store
/// location is still detected, falling back to the conventional path.
pub(super) fn store_grants(activated: &BTreeMap<String, String>) -> Vec<String> {
    let path = activated.get("PATH").map(String::as_str).unwrap_or("");
    for entry in path.split(':') {
        if let Some(idx) = entry.find(DEFAULT_STORE_ROOT) {
            return vec![entry[..idx + DEFAULT_STORE_ROOT.len()].to_string()];
        }
    }
    vec![DEFAULT_STORE_ROOT.to_string()]
}

/// FNV-1a 64-bit over `parts`, each separated so `("ab", "c")` and
/// `("a", "bc")` never collide. Not cryptographic — this is a change
/// detector, not a security boundary, so no external hashing crate is
/// pulled in for it.
pub(super) fn fingerprint(parts: &[&[u8]]) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for part in parts {
        for &byte in *part {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_env_dump_splits_nul_separated_pairs() {
        let raw = b"FOO=bar\0PATH=/a:/b\0EMPTY=\0";
        let parsed = parse_env_dump(raw);
        assert_eq!(parsed.get("FOO"), Some(&"bar".to_string()));
        assert_eq!(parsed.get("PATH"), Some(&"/a:/b".to_string()));
        assert_eq!(parsed.get("EMPTY"), Some(&"".to_string()));
    }

    #[test]
    fn changed_env_keeps_only_added_or_changed_keys() {
        let baseline = BTreeMap::from([
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("UNCHANGED".to_string(), "same".to_string()),
        ]);
        let activated = BTreeMap::from([
            (
                "PATH".to_string(),
                "/nix/store/xyz-flox/bin:/usr/bin".to_string(),
            ),
            ("UNCHANGED".to_string(), "same".to_string()),
            ("FLOX_ENV".to_string(), "/nix/store/xyz-flox".to_string()),
        ]);

        let diff = changed_env(&baseline, &activated);

        assert_eq!(diff.len(), 2);
        assert_eq!(
            diff.get("PATH"),
            Some(&"/nix/store/xyz-flox/bin:/usr/bin".to_string())
        );
        assert_eq!(
            diff.get("FLOX_ENV"),
            Some(&"/nix/store/xyz-flox".to_string())
        );
        assert!(!diff.contains_key("UNCHANGED"));
    }

    #[test]
    fn changed_env_excludes_keys_activation_removed() {
        // A key present in baseline but absent from activated is neither
        // "changed" (changed_env's job) nor should it silently vanish —
        // unset_env below is what's responsible for it.
        let baseline = BTreeMap::from([
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("REMOVED".to_string(), "gone".to_string()),
        ]);
        let activated = BTreeMap::from([("PATH".to_string(), "/usr/bin".to_string())]);

        let diff = changed_env(&baseline, &activated);
        assert!(!diff.contains_key("REMOVED"));
    }

    #[test]
    fn unset_env_reports_baseline_keys_activation_dropped() {
        let baseline = BTreeMap::from([
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("REMOVED".to_string(), "gone".to_string()),
        ]);
        let activated = BTreeMap::from([("PATH".to_string(), "/usr/bin".to_string())]);

        assert_eq!(
            unset_env(&baseline, &activated),
            vec!["REMOVED".to_string()]
        );
    }

    #[test]
    fn unset_env_is_empty_when_nothing_was_removed() {
        let baseline = BTreeMap::from([("PATH".to_string(), "/usr/bin".to_string())]);
        let activated = BTreeMap::from([
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("FLOX_ENV".to_string(), "/nix/store/xyz-flox".to_string()),
        ]);

        assert!(unset_env(&baseline, &activated).is_empty());
    }

    #[test]
    fn store_grants_reads_root_from_activated_path() {
        let activated = BTreeMap::from([(
            "PATH".to_string(),
            "/nix/store/xyz-flox/bin:/usr/bin".to_string(),
        )]);
        assert_eq!(store_grants(&activated), vec!["/nix/store".to_string()]);
    }

    #[test]
    fn store_grants_falls_back_to_default_when_path_has_no_store_entry() {
        let activated = BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]);
        assert_eq!(
            store_grants(&activated),
            vec![DEFAULT_STORE_ROOT.to_string()]
        );
    }

    #[test]
    fn fingerprint_changes_when_any_part_changes() {
        let a = fingerprint(&[b"one", b"two"]);
        let b = fingerprint(&[b"one", b"three"]);
        assert_ne!(a, b);
    }

    #[test]
    fn fingerprint_does_not_collide_across_part_boundaries() {
        // ("ab", "c") and ("a", "bc") must hash differently even though
        // concatenated they'd be the same bytes — the per-part separator
        // is what this test exercises.
        let a = fingerprint(&[b"ab", b"c"]);
        let b = fingerprint(&[b"a", b"bc"]);
        assert_ne!(a, b);
    }

    #[test]
    fn fingerprint_is_stable_for_the_same_input() {
        let a = fingerprint(&[b"x", b"y"]);
        let b = fingerprint(&[b"x", b"y"]);
        assert_eq!(a, b);
    }
}
