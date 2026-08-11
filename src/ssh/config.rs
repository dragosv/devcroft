//! `devcroft ssh-config` (ssh spec's "ssh-config emission" requirement,
//! task 6.2): the wildcard `Host *.devcroft` block from design.md
//! decision 3, and an idempotent marker-delimited insert/update into
//! `~/.ssh/config` for `--write`.

use std::io;
use std::path::Path;

const BEGIN_MARKER: &str = "# >>> devcroft managed block — do not edit >>>";
const END_MARKER: &str = "# <<< devcroft managed block — do not edit <<<";

/// Renders the managed block on its own (no markers) — what `ssh-config`
/// prints with no `--write`.
pub fn render(identity_file: &str) -> String {
    format!(
        "Host *.devcroft\n  ProxyCommand devcroft proxy %n\n  IdentityFile {identity_file}\n  StrictHostKeyChecking no\n  UserKnownHostsFile /dev/null\n"
    )
}

/// Inserts or updates the marker-delimited managed section in the file at
/// `path`, creating it (and its parent dir) if neither exists yet.
/// Idempotent: running this twice with the same `identity_file` leaves
/// the file byte-for-byte identical after the second run, and content
/// outside the markers — anything the user put in `~/.ssh/config`
/// themselves — is never touched.
pub fn write_managed_section(path: &Path, identity_file: &str) -> io::Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let block = format!("{BEGIN_MARKER}\n{}{END_MARKER}\n", render(identity_file));

    let updated = match (existing.find(BEGIN_MARKER), existing.find(END_MARKER)) {
        (Some(start), Some(end)) if start < end => {
            let end = end + END_MARKER.len();
            let mut out = String::with_capacity(existing.len() + block.len());
            out.push_str(&existing[..start]);
            out.push_str(&block);
            // Drop one trailing newline off the old end marker line so
            // re-running this doesn't accumulate blank lines between the
            // managed section and whatever follows it.
            let rest = existing[end..]
                .strip_prefix('\n')
                .unwrap_or(&existing[end..]);
            out.push_str(rest);
            out
        }
        _ => {
            let mut out = existing;
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&block);
            out
        }
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_matches_design_decision_3s_block() {
        let out = render("~/.local/share/devcroft/id_ed25519");
        assert_eq!(
            out,
            "Host *.devcroft\n  ProxyCommand devcroft proxy %n\n  IdentityFile ~/.local/share/devcroft/id_ed25519\n  StrictHostKeyChecking no\n  UserKnownHostsFile /dev/null\n"
        );
    }

    #[test]
    fn write_creates_a_fresh_file_with_the_managed_block() {
        let dir =
            std::env::temp_dir().join(format!("devcroft-sshconfig-fresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config");

        write_managed_section(&path, "~/.local/share/devcroft/id_ed25519").unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains(BEGIN_MARKER));
        assert!(contents.contains(END_MARKER));
        assert!(contents.contains("Host *.devcroft"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_is_idempotent() {
        let dir =
            std::env::temp_dir().join(format!("devcroft-sshconfig-idem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config");

        write_managed_section(&path, "~/.local/share/devcroft/id_ed25519").unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        write_managed_section(&path, "~/.local/share/devcroft/id_ed25519").unwrap();
        let second = std::fs::read_to_string(&path).unwrap();

        assert_eq!(first, second);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_preserves_content_outside_the_markers_and_updates_inside() {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-sshconfig-preserve-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config");
        std::fs::write(&path, "Host example\n  HostName example.com\n  User me\n").unwrap();

        write_managed_section(&path, "~/.local/share/devcroft/id_ed25519").unwrap();
        write_managed_section(&path, "/different/path").unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("Host example\n  HostName example.com\n  User me\n"));
        assert!(contents.contains("IdentityFile /different/path"));
        assert!(!contents.contains("id_ed25519"));
        assert_eq!(contents.matches(BEGIN_MARKER).count(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
