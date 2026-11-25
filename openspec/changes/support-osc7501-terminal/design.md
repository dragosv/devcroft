## Context

A supporting program asks the terminal for its capabilities and reports its own state after the response. Pin the herdr-core revision for the fixtures. A real emitting agent can be started manually in an existing shell or SSH sandbox.

## Decisions

On the local PTY client, wait for `SpawnOk` while Ctrl-C still works normally. Block forwarded signals and enable raw mode before displaying the first session byte. Check `tcsetattr`, retry on `EINTR` and restore the original attributes on observable exits. The SSH client owns its local terminal mode. A synthetic agent also sets its own PTY to raw before the query.

Relay both byte streams unchanged, including fragmented and unfamiliar fields. Bind input to its connection. Keep PTY output out of the persistent lifecycle log and escape terminal controls when displaying that log on a TTY.

## Qualification

Compare bytes with independent terminal and agent fixtures, a finite timeout and concurrent sessions. Record a real negotiated state transition visible in `hc ls`, with protocol revision, terminal, agent and platform. The package change reuses this suite for `agent run`. The editor change checks its separate piped protocol.
