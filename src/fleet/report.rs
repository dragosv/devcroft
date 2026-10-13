//! What an agent reports about its own work (`add-fleet-tasks` D3):
//! attention, progress, and a claim that its task is done or failed.
//!
//! **Reported, never observed.** These are the agent's words, written from
//! inside it by `devcroft agent …`. What devcroft observed of a task (its
//! exit status) lives in `fleet::task`, where the agent cannot write, and
//! the two are shown side by side, never merged: an agent that says `done`
//! and exits 1 shows both.
//!
//! **Beside the agent's home.** `HOME` inside a sandbox is
//! `<project>/.devcroft/<x>/home`, which devcroft sets and nothing else
//! does (the rule `why` uses to find its own policy), so the report is
//! `.devcroft/<x>/report.json`: in the workspace, already writable, no
//! socket and no grant. It survives the reporting process, and goes with
//! the clone.
//!
//! **Untrusted on the way out.** The host reads it with a size cap and
//! displays it through [`escape`], so a message cannot drive the
//! operator's terminal.

use serde::{Deserialize, Serialize};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// The variable a devcroft task's process gets, naming its task; recorded
/// with what it reports.
pub const TASK_ENV: &str = "DEVCROFT_TASK";

/// The file's name, beside the agent's home.
pub const FILE: &str = "report.json";

/// The most of a report the host reads. A larger one is reported as
/// unreadable rather than truncated into something it did not say.
pub const MAX_BYTES: u64 = 64 << 10;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// The devcroft task it was written during (`DEVCROFT_TASK`), if any.
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub attention: Option<Note>,
    #[serde(default)]
    pub progress: Option<Note>,
    #[serde(default)]
    pub claim: Option<Claim>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub message: String,
    pub at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub outcome: ClaimOutcome,
    #[serde(default)]
    pub message: Option<String>,
    pub at_unix: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimOutcome {
    Done,
    Failed,
}

impl std::fmt::Display for ClaimOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ClaimOutcome::Done => "done",
            ClaimOutcome::Failed => "failed",
        })
    }
}

/// Agent `id`'s report, as the host finds it in the agent's workspace.
pub fn path(workspace: &Path, id: &str) -> PathBuf {
    crate::services::artifact_dir(workspace, id).join(FILE)
}

/// The report at `path`: `Ok(None)` when there is none, `Err` with why when
/// there is one devcroft will not show (too large, or not a report).
pub fn read(path: &Path) -> Result<Option<Report>, String> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!(
            "{} is larger than {MAX_BYTES} bytes; not shown",
            path.display()
        ));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| format!("{} is not a report: {e}", path.display()))
}

/// Change the report at `path` and write it back atomically. A report
/// from another task is started over, so nothing an earlier task said is
/// shown against this one.
pub fn update(path: &Path, task: Option<&str>, change: impl FnOnce(&mut Report)) -> io::Result<()> {
    let mut report = read(path).ok().flatten().unwrap_or_default();
    if report.task.as_deref() != task {
        report = Report {
            task: task.map(str::to_owned),
            ..Report::default()
        };
    }
    change(&mut report);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(&report).map_err(io::Error::other)?,
    )?;
    std::fs::rename(tmp, path)
}

/// Remove the report at `path`, if any: a new task starts with none.
pub fn clear(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// `text` safe to print to a terminal: every control character but a
/// newline, and the format characters that reorder or hide text, written
/// as `\u{…}` escapes. Everything else is shown as it is.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let hidden = matches!(c as u32, 0x200B..=0x200F | 0x202A..=0x202E | 0x2066..=0x2069);
        if (c.is_control() && c != '\n') || hidden {
            out.extend(c.escape_unicode());
        } else {
            out.push(c);
        }
    }
    out
}

/// Seconds since the epoch, now.
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempfile(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("devcroft-report-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(FILE)
    }

    #[test]
    fn escape_neutralises_terminal_control_and_keeps_text() {
        assert_eq!(escape("plain, ünïcode\nnext"), "plain, ünïcode\nnext");
        assert_eq!(escape("a\x1b[2Jb"), "a\\u{1b}[2Jb");
        assert_eq!(escape("x\ry\u{7}"), "x\\u{d}y\\u{7}");
        assert_eq!(escape("ab\u{202e}cd"), "ab\\u{202e}cd");
    }

    #[test]
    fn a_report_from_another_task_is_started_over() {
        let path = tempfile("task");
        update(&path, Some("t1"), |r| {
            r.attention = Some(Note {
                message: "old".into(),
                at_unix: 1,
            })
        })
        .unwrap();
        update(&path, Some("t2"), |r| {
            r.progress = Some(Note {
                message: "new".into(),
                at_unix: 2,
            })
        })
        .unwrap();
        let report = read(&path).unwrap().unwrap();
        assert_eq!(report.task.as_deref(), Some("t2"));
        assert_eq!(report.attention, None);
        assert_eq!(report.progress.unwrap().message, "new");
    }

    #[test]
    fn an_oversized_or_foreign_file_is_not_shown() {
        let path = tempfile("big");
        std::fs::write(&path, vec![b' '; MAX_BYTES as usize + 1]).unwrap();
        assert!(read(&path).unwrap_err().contains("larger than"));
        std::fs::write(&path, "not json").unwrap();
        assert!(read(&path).unwrap_err().contains("not a report"));
        clear(&path).unwrap();
        assert_eq!(read(&path), Ok(None));
        clear(&path).unwrap();
    }
}
