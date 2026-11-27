## Context

A supporting program asks the terminal for its capabilities and reports its own state after the response. The agent writes the report, the terminal (herdr-core) writes the capability reply, and devcroft only carries the bytes. A report is the program’s claim, not an attestation. Pin the herdr-core revision for the fixtures, and take framing, terminators and field syntax from that revision rather than defining them here.

## Decisions

On the local PTY client, wait for `SpawnOk` while Ctrl-C still works normally. Block forwarded signals and enable raw mode before displaying the first session byte. Check `tcsetattr`, retry on `EINTR` and restore the original attributes on observable exits. The SSH client owns its local terminal mode. A synthetic agent also sets its own PTY to raw before the query.

Relay both byte streams unchanged, including fragmented and unfamiliar fields. Bind input to its connection. Keep PTY output out of the persistent lifecycle log and escape terminal controls when displaying that log on a TTY. Transporting the protocol grants nothing: a `kind=permission` report is a report, not an approval.

## Qualification cases

Every supported interactive path must pass these cases. Other changes cite them by ID.

| ID | Case | Required result |
|---|---|---|
| `T01` | The agent emits a query immediately after spawn | The query reaches the terminal, and the reply is not lost in the switch to interactive mode |
| `T02` | The terminal replies without a newline | The agent receives the reply without the user pressing Enter |
| `T03` | Query, reply and report are fragmented, including across framing bytes | Content and order are preserved, and negotiation succeeds |
| `T04` | Several reports and ordinary output arrive in one buffer | Each report and the output arrive unchanged, in emitted order |
| `T05` | The agent negotiates and reports its application, states and qualified kinds | The terminal interprets exactly the agent’s reports |
| `T06` | The terminal does not support the protocol or does not reply | devcroft fabricates no reply and does not classify the process as an agent |
| `T07` | The program emits no OSC 7501 | It stays an ordinary terminal, with no synthetic reports |
| `T08` | Two sessions in one sandbox receive different replies | Each agent receives only the bytes meant for its own connection |
| `T09` | The agent emits values or extensions devcroft does not know | The bytes pass through, with no filtering against a local list |
| `T10` | A session closes and another starts | No reply or report from the old connection is reused |
| `T11` | An ACP client runs with a valid stream | devcroft puts no OSC sequence or message of its own on the ACP `stdout` (owned by `add-agent-editor-acp`) |
| `T12` | Interactive session through each supported PTY path (`devcroft shell`, SSH, and `agent run` once it exists) | `T01`–`T05` pass on every path declared supported |

`OSC-05` names the rule behind `T06` and `T07` for packaged agents: devcroft never emits identity or state on a process’s behalf because it was started from a recipe. Exit code `0` does not justify `done`, silence does not justify `idle` or `blocked`, and a policy denial does not justify a blocked kind.

## Qualification

Compare bytes with independent terminal and agent fixtures, a finite timeout and concurrent sessions. Record a real negotiated state transition visible in `hc ls`, with protocol revision, terminal, agent and platform. The first candidate emitter is `fx` (Vercel Labs), which reports `app=fx` natively, so this change can qualify a real emitter without waiting for agent packages. Pin its version and record its model route. Its own shell tool is not part of this measurement. The package change reuses this suite for `agent run`. The editor change owns `T11`.
