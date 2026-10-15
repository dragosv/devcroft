//! An agent's tasks (`add-fleet-tasks`): commands devcroft runs in an
//! agent, each with an identity and an **observed** outcome.
//!
//! **A task is a keeper session, held by a host-side runner** (design D1).
//! `devcroft __task` opens an ordinary `Spawn` session on the agent's
//! control socket, appends its output to the task's log, and records the
//! exit status the keeper reports. No keeper protocol change, and no
//! lifeline: when the agent dies, its keeper's end of the session closes,
//! which the runner sees.
//!
//! **Recorded where the agent cannot write** (D2): the fleet's state
//! directory, `agents/<id>/tasks/<task>.json`, which no agent's view
//! contains. What the agent says about its work is `fleet::report`'s, and
//! is never merged into this.
//!
//! **IDs are never reused within an agent**: `t1`, `t2`, …, the highest
//! issued kept in `last-task`, as agent IDs keep theirs in `last-id`.

use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::keeper::protocol::{self, Frame, SpawnRequest};

/// The variable a task's process gets, naming its task.
pub const ENV: &str = super::report::TASK_ENV;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: String,
    pub command: Vec<String>,
    /// Where it runs, as the agent sees it.
    pub cwd: PathBuf,
    pub started_unix: u64,
    #[serde(default)]
    pub ended_unix: Option<u64>,
    pub outcome: Outcome,
    /// The runner holding its session, once it has started.
    #[serde(default)]
    pub runner_pid: Option<u32>,
}

/// What devcroft observed of a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Outcome {
    Running,
    Exited {
        code: i32,
    },
    Signaled {
        signal: i32,
    },
    /// The agent ended (stopped, or died) while it ran.
    EndedWithAgent,
    /// Its end was not observed, and why.
    Lost {
        reason: String,
    },
}

impl Outcome {
    /// A word or two, for a listing: `lost`'s reason is `inspect`'s.
    pub fn short(&self) -> String {
        match self {
            Outcome::Lost { .. } => "lost".into(),
            other => other.to_string(),
        }
    }
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::Running => f.write_str("running"),
            Outcome::Exited { code } => write!(f, "exited {code}"),
            Outcome::Signaled { signal } => write!(f, "killed by signal {signal}"),
            Outcome::EndedWithAgent => f.write_str("ended with its agent"),
            Outcome::Lost { reason } => write!(f, "lost: {reason}"),
        }
    }
}

pub fn tasks_dir(agent_dir: &Path) -> PathBuf {
    agent_dir.join("tasks")
}

pub fn log_path(agent_dir: &Path, task: &str) -> PathBuf {
    tasks_dir(agent_dir).join(format!("{task}.log"))
}

fn record_path(agent_dir: &Path, task: &str) -> PathBuf {
    tasks_dir(agent_dir).join(format!("{task}.json"))
}

fn number(task: &str) -> Option<u64> {
    let digits = task.strip_prefix('t')?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}

/// The next task ID for the agent at `agent_dir`, recorded as issued.
pub fn next_id(agent_dir: &Path) -> io::Result<String> {
    std::fs::create_dir_all(tasks_dir(agent_dir))?;
    let issued = std::fs::read_to_string(agent_dir.join("last-task"))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let highest = all(agent_dir)?
        .iter()
        .filter_map(|t| number(&t.id))
        .max()
        .unwrap_or(0);
    let n = issued.max(highest) + 1;
    std::fs::write(agent_dir.join("last-task"), n.to_string())?;
    Ok(format!("t{n}"))
}

pub fn write(agent_dir: &Path, task: &TaskRecord) -> io::Result<()> {
    std::fs::create_dir_all(tasks_dir(agent_dir))?;
    let path = record_path(agent_dir, &task.id);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(task).map_err(io::Error::other)?,
    )?;
    std::fs::rename(tmp, path)
}

pub fn read(agent_dir: &Path, task: &str) -> io::Result<TaskRecord> {
    let bytes = std::fs::read(record_path(agent_dir, task))?;
    serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Every task of the agent at `agent_dir`, in ID order.
pub fn all(agent_dir: &Path) -> io::Result<Vec<TaskRecord>> {
    let mut tasks = Vec::new();
    let entries = match std::fs::read_dir(tasks_dir(agent_dir)) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(tasks),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let name = entry?.file_name();
        let Some(id) = name.to_str().and_then(|n| n.strip_suffix(".json")) else {
            continue;
        };
        if number(id).is_some() {
            tasks.push(read(agent_dir, id)?);
        }
    }
    tasks.sort_by_key(|t| number(&t.id));
    Ok(tasks)
}

/// The agent's most recent task.
pub fn latest(agent_dir: &Path) -> io::Result<Option<TaskRecord>> {
    Ok(all(agent_dir)?.pop())
}

/// Whether `pid` is a live task runner: not reaped, not a zombie, and
/// still `devcroft __task` (a reused pid is some other process).
fn runner_alive(pid: u32) -> bool {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    let zombie = stat
        .rsplit_once(')')
        .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'));
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    !zombie && cmdline.split(|b| *b == 0).any(|arg| arg == b"__task")
}

/// Settle the agent's running task when nothing will record its end: its
/// agent is gone (`agent_running` false), or its runner is. The agent
/// being gone wins, whatever the runner was doing: the task cannot be
/// running in an agent that is not.
pub fn settle(agent_dir: &Path, agent_running: bool) -> io::Result<()> {
    let Some(mut task) = latest(agent_dir)? else {
        return Ok(());
    };
    if task.outcome != Outcome::Running {
        return Ok(());
    }
    let outcome = if !agent_running {
        Outcome::EndedWithAgent
    } else if task.runner_pid.is_some_and(runner_alive) {
        return Ok(());
    } else {
        Outcome::Lost {
            reason: "its runner exited without recording an end".into(),
        }
    };
    task.outcome = outcome;
    task.ended_unix = Some(super::report::now_unix());
    write(agent_dir, &task)
}

/// `devcroft __task <control socket> <agent dir> <task>`: hold the task's
/// keeper session and record what it ended with. Returns the process's
/// exit code.
pub fn run(control_socket: &Path, agent_dir: &Path, id: &str) -> i32 {
    match run_inner(control_socket, agent_dir, id) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("devcroft __task {id}: {e}");
            1
        }
    }
}

fn run_inner(control_socket: &Path, agent_dir: &Path, id: &str) -> io::Result<()> {
    let mut task = read(agent_dir, id)?;
    // First, so whoever started this can see it is running.
    task.runner_pid = Some(std::process::id());
    write(agent_dir, &task)?;

    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path(agent_dir, id))?;
    let outcome = match session(control_socket, &task, &mut log) {
        Ok(outcome) => outcome,
        // The keeper's end closed with no exit status: the keeper is gone,
        // and an agent's keeper is its main process, so the agent ended
        // (stopped, killed whole, or crashed). Recorded as that, not as
        // lost: measured, a `stop` lost the race to settle it otherwise.
        // A reset is the same end: Linux reports a peer that died with
        // unread data in its socket as `ECONNRESET`, not EOF.
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset
            ) =>
        {
            Outcome::EndedWithAgent
        }
        Err(e) => Outcome::Lost {
            reason: format!("the agent's keeper session ended without an exit status: {e}"),
        },
    };
    // Only over a running record: `stop` may have settled it already, and
    // what it said (ended with its agent) is the truer account.
    let mut now = read(agent_dir, id)?;
    if now.outcome == Outcome::Running {
        now.outcome = outcome;
        now.ended_unix = Some(super::report::now_unix());
        write(agent_dir, &now)?;
    }
    Ok(())
}

fn session(
    control_socket: &Path,
    task: &TaskRecord,
    log: &mut std::fs::File,
) -> io::Result<Outcome> {
    let mut stream = std::os::unix::net::UnixStream::connect(control_socket)?;
    let (cmd, args) = task
        .command
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty command"))?;
    protocol::write_frame(
        &mut stream,
        &Frame::Spawn(SpawnRequest {
            cmd: cmd.clone(),
            args: args.to_vec(),
            cwd: task.cwd.to_string_lossy().into_owned(),
            env: [(ENV.to_owned(), task.id.clone())].into(),
            pty: None,
        }),
    )?;
    // Nothing to read: a task is unattended.
    protocol::write_frame(&mut stream, &Frame::StdinClosed)?;
    loop {
        match protocol::read_frame(&mut stream)? {
            Frame::Stdout(b) | Frame::Stderr(b) => log.write_all(&b)?,
            Frame::Exit(status) => {
                return Ok(match (status.code, status.signal) {
                    (Some(code), _) => Outcome::Exited { code },
                    (None, Some(signal)) => Outcome::Signaled { signal },
                    (None, None) => Outcome::Lost {
                        reason: "the keeper reported neither a code nor a signal".into(),
                    },
                });
            }
            Frame::SpawnErr { message } => {
                writeln!(
                    log,
                    "devcroft: the keeper could not start the task: {message}"
                )?;
                return Ok(Outcome::Lost {
                    reason: format!("the keeper could not start it: {message}"),
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("devcroft-task-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn running(id: &str, runner_pid: Option<u32>) -> TaskRecord {
        TaskRecord {
            id: id.into(),
            command: vec!["true".into()],
            cwd: "/workspace".into(),
            started_unix: 1,
            ended_unix: None,
            outcome: Outcome::Running,
            runner_pid,
        }
    }

    #[test]
    fn ids_are_never_reused_even_after_their_records_go() {
        let dir = agent_dir("ids");
        assert_eq!(next_id(&dir).unwrap(), "t1");
        write(&dir, &running("t1", None)).unwrap();
        assert_eq!(next_id(&dir).unwrap(), "t2");
        std::fs::remove_dir_all(tasks_dir(&dir)).unwrap();
        assert_eq!(next_id(&dir).unwrap(), "t3");
    }

    #[test]
    fn a_running_task_is_settled_by_whoever_notices_its_end() {
        let dir = agent_dir("settle");
        // A runner that is alive: this test process is not one, so use a
        // pid whose cmdline cannot contain `__task`.
        write(&dir, &running("t1", Some(std::process::id()))).unwrap();
        settle(&dir, true).unwrap();
        assert!(matches!(
            read(&dir, "t1").unwrap().outcome,
            Outcome::Lost { .. }
        ));

        write(&dir, &running("t2", Some(std::process::id()))).unwrap();
        settle(&dir, false).unwrap();
        assert_eq!(read(&dir, "t2").unwrap().outcome, Outcome::EndedWithAgent);

        // An ended task is left as it is.
        let mut done = running("t3", None);
        done.outcome = Outcome::Exited { code: 0 };
        write(&dir, &done).unwrap();
        settle(&dir, false).unwrap();
        assert_eq!(
            read(&dir, "t3").unwrap().outcome,
            Outcome::Exited { code: 0 }
        );
    }
}
