## 1. Establish the protocol baseline

- [ ] 1.1 Pin the herdr-core revision. Extract its actual query, reply, report and framing into deterministic fixtures. Record the shell and SSH results on the unmodified implementation.
- [ ] 1.2 Give each fixture its own terminal. Put the agent-side PTY in raw mode and bound the reply wait. Cover `T01`–`T10`: both directions, immediate output, no newline, arbitrary splits, coalescing, unknown fields, silent terminals, non-reporting processes, concurrent sessions and reconnects.

## 2. Qualify the existing interactive paths

- [ ] 2.1 Handle terminal setup failures, interruptions and restoration in the exec client. Check Ctrl-C while waiting for spawn and restoration on spawn refusal, termination and hangup. Correct any relay defects demonstrated by the fixtures.
- [ ] 2.2 Verify the shell and SSH PTY paths against the same byte comparisons (`T12`). Make `devcroft logs` escape C0 and C1 controls on a TTY and leave piped output unchanged, and check that viewing recorded logs cannot produce a live status event.
- [ ] 2.3 After `add-agent-packages` task 0.3, record a live negotiation and real state transition in `hc ls` with `fx` pinned in a mise `http:` qualification project (or another native emitter) and driven by `fake-llm`, naming the emitting agent and its version, terminal, protocol revision and platform. Publish the measured coverage.
