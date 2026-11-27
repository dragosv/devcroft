## Context

1.0 is a promise, and the changes that build 1.0’s features don’t make one. This change turns the release scope accepted on 2026-11-27 into checks a release can pass or fail. Today, `src/lib.rs` already declares the Rust items unstable, `README.md` sends security reports to a public issue, and `docs/roadmap.md` ties 1.0 to fleet on both platforms.

## Goals / Non-Goals

**Goals:**

- Say exactly what 1.0 users can rely on.
- Give a reporter a private channel before 1.0 exists.
- Make the supported matrix something CI proves, not something the README asserts.
- Make an upgrade and a crash safe and visible.

**Non-Goals:**

- Fleet, microVM, Mode 2 or a catalog of agents. Each stays experimental unless it is qualified.
- A stable Rust library API.

## Decisions

**D1. The stable surface is what a user types and reads.** It covers:

- commands, flags, exit codes and error layers;
- the `devcroft.toml` and user-configuration keys and their meaning;
- the behaviors `docs/stability.md` lists.

Message wording is not stable, but the layer and code are. After 1.0, semver applies to that surface only. The alternative of covering the `pub` items too was rejected: `src/lib.rs` publishes internals so `tests/` can drive them, and freezing them would freeze the implementation.

**D2. A private channel before 1.0.** `SECURITY.md` names the channel, what to include and the response window. GitHub private vulnerability reporting needs no infrastructure, so it’s the default proposal. The owner sets it up, since it needs the owner’s account.

**D3. The release gate reads recorded evidence.** It joins the agent qualification records (`add-agent-packages`) with the provider rows and the backend checks. A combination is stable only when every mandatory probe for it ran and passed on its platform. A skipped probe doesn’t fail the build; it keeps the combination out of the stable set, and the release notes list it as unqualified.

**D4. An upgrade never misreads old state.** Each release declares the oldest `Meta` and configuration it can read. On `up`, an older sandbox either runs unchanged, or is refused at layer `config` naming `up --recreate`, and never runs with fields defaulted silently. CI tests both against the previous release’s state.

**D5. A crash leaves nothing with authority.** If a keeper, egress proxy, relay or agent process dies, its siblings with the sandbox’s authority are ended. On Linux this happens through the PID namespace and the lifeline that `fleet::lifeline` already provides; on macOS through a lifeline that still has to be measured. `status` reports the crash, and the next `up` starts clean.

## Risks / Trade-offs

- [macOS can’t end every descendant of a crashed keeper] → Measure it. Where it can’t, state the limit in `docs/known-gaps.md` and keep that combination out of the stable set.
- [The gate makes few combinations stable at first] → Intended. The release notes list the rest as available but unqualified.
- [The owner hasn’t chosen the security channel] → The proposal defaults to GitHub private reporting, and `SECURITY.md` waits for the owner’s setup.
