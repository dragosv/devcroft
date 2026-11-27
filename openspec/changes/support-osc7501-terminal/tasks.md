## 1. Establish the protocol baseline

- [ ] 1.1 Pin the herdr-core revision. Extract its actual query, reply, report and framing into deterministic fixtures. Record the shell and SSH results on the unmodified implementation.
- [ ] 1.2 Give each fixture its own terminal. Put the agent-side PTY in raw mode and bound the reply wait. Cover `T01`–`T10`: both directions, immediate output, no newline, arbitrary splits, coalescing, unknown fields, silent terminals, non-reporting processes, concurrent sessions and reconnects.

## 2. Qualify the existing interactive paths

- [ ] 2.1 Handle terminal setup failures, interruptions and restoration in the exec client. Check Ctrl-C while waiting for spawn and restoration on spawn refusal, termination and hangup. Correct any relay defects demonstrated by the fixtures.
- [ ] 2.2 Verify the shell and SSH PTY paths against the same byte comparisons (`T12`). Check that viewing recorded logs on a terminal cannot produce a live status event.
- [ ] 2.3 Record a live negotiation and real state transition in `hc ls` with a pinned `fx` (or another native emitter) driven by `fake-llm`, naming the emitting agent and its version, terminal, protocol revision and platform. Publish the measured coverage.
