# agent-tasks Delta Specification (add-fleet-tasks)

## ADDED Requirements

### Requirement: A fleet agent runs tasks with stable identities

`fleet run <id> -- <command>` SHALL run the command in the agent as a task,
and `fleet up` SHALL accept a command to run as each new agent's first
task. Each task SHALL have an ID never reused within its agent, and SHALL
record its command, its start, its end, and a log of its output. An agent
SHALL have at most one task running; starting another while one runs SHALL
be refused at layer `config` with exit code 2.

#### Scenario: Two tasks in turn
- **WHEN** a task finishes and another is run on the same agent
- **THEN** the second has a new ID, and the first's record and log remain

#### Scenario: A task is already running
- **WHEN** `fleet run` targets an agent whose task is still running
- **THEN** it is refused, naming the running task

### Requirement: Completion is observed, not reported

A task's outcome SHALL be what devcroft observed: the exit code or signal
of the command, or that it ended with its agent, or that it was lost and
why. It SHALL be recorded where the agent cannot write.

#### Scenario: A task exits with a code
- **WHEN** a task's command exits with code 3
- **THEN** the task is reported as exited with code 3, and its output is in
  its log

#### Scenario: The agent is stopped during a task
- **WHEN** an agent is stopped while its task runs
- **THEN** the task is reported as ended with its agent, not as running

### Requirement: An agent reports attention and progress from inside

A process in an agent SHALL be able to raise attention with a message,
clear it, report progress, and claim the task done or failed, with no grant
the agent lacks. Reports SHALL survive the reporting process exiting, SHALL
NOT outlive the agent's clone, and SHALL NOT carry over to the agent's next
task.

#### Scenario: Attention raised and the process exits
- **WHEN** a process in an agent raises attention and exits
- **THEN** `fleet ls` marks the agent as needing attention and
  `fleet inspect` shows the message verbatim

#### Scenario: A new task starts clean
- **WHEN** a task raised attention and a new task is run
- **THEN** the agent is not reported as needing attention

### Requirement: Reported and observed are never merged

What an agent reports SHALL be shown as reported, distinct from the
observed outcome, and SHALL NOT change it.

#### Scenario: An agent claims done and fails
- **WHEN** a task's process reports `done` and exits with code 1
- **THEN** the task is shown as exited with code 1, and the claim as the
  agent's

### Requirement: Reports cannot drive the operator's terminal

Messages SHALL be displayed with control characters escaped, and read with
a size limit.

#### Scenario: A message with an escape sequence
- **WHEN** an agent's attention message contains a terminal escape sequence
- **THEN** `fleet inspect` shows it escaped, not interpreted

### Requirement: devcroft's files in a clone are not the agent's to commit

Each agent's clone SHALL ignore its `.devcroft/` directory in its own git,
whatever the project's ignore rules.

#### Scenario: An agent commits everything
- **WHEN** an agent runs `git add -A` in its workspace
- **THEN** nothing under `.devcroft/` is staged
