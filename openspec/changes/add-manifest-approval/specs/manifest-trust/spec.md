## ADDED Requirements

### Requirement: A manifest is untrusted until approved
Every checkout's manifest SHALL be `untrusted` unless an approval record for
that checkout is `approved` for the exact bytes being loaded. An untrusted
manifest SHALL still select the provider, services, hooks, identity and
sandbox name, and its narrowing keys (`filesystem.deny`, `network.default =
"deny"`, `env.require_native_apple_evidence`) and literal `[env.vars]` SHALL
apply. These widening keys SHALL NOT apply:

- `filesystem.allow` beyond the workspace, and `filesystem.read`;
- `network.default = "allow"`, `network.allow`, `network.loopback = "any"`, `network.ports` and service ports;
- `provisioning.network.allow`;
- `env.forward`;
- `ssh.forward_agent`, which would bridge the user's SSH agent socket into the sandbox.

`up` SHALL print one line naming each key it did not apply, with a summary
and never a value. When the remaining combination is invalid, `up` SHALL fail
at layer `config` naming `devcroft trust`.

#### Scenario: Widening keys of an untrusted manifest
- **WHEN** an unapproved manifest declares `filesystem.read = ["~/notes"]`, `network.allow = ["example.com"]` and `[env] forward = ["MARKER"]`, and `up` runs
- **THEN** the sandbox cannot read `~/notes`, cannot reach `example.com`, has no `MARKER`, and `up` names the three keys

#### Scenario: Narrowing still applies
- **WHEN** an unapproved manifest declares `filesystem.deny = ["secrets"]`
- **THEN** `secrets` under the workspace is denied

#### Scenario: A workload cannot approve its own grants
- **WHEN** a process in the sandbox adds `network.allow` hosts by any route and the user runs `up --recreate`
- **THEN** the manifest is untrusted and the hosts are not reachable

### Requirement: An approval is bound to one checkout and the exact bytes
An approval SHALL be created only by an explicit host action (`devcroft trust`,
or `init` for the bytes it writes). It SHALL record the checkout's canonical
root, the root directory's device and inode, and the SHA-256 of the manifest
bytes. devcroft SHALL hash and parse the same single read of a regular file
opened without following symlinks, and SHALL fail at layer `config` on a
symlinked manifest or one with more than one link. A missing or unreadable
manifest SHALL NOT be covered by any earlier approval. A Git remote, sandbox
name or path alone SHALL NOT count as an approval. Approval records SHALL live
in devcroft's data directory, unreachable from every sandbox.

#### Scenario: Two worktrees with the same manifest
- **WHEN** the manifest in worktree A is approved and worktree B holds identical bytes
- **THEN** B is untrusted

#### Scenario: Re-clone at the same path
- **WHEN** an approved checkout is deleted and cloned again at the same path
- **THEN** the new checkout is untrusted

#### Scenario: Hard-linked manifest
- **WHEN** `devcroft.toml` has a second hard link and `up` runs
- **THEN** `up` fails at layer `config` naming the manifest and its link count

### Requirement: A changed manifest invalidates its approval for good
When a start or an attach loads bytes whose hash differs from the approved
hash, devcroft SHALL mark the approval `invalidated` and treat the checkout as
untrusted. A later return to the approved bytes SHALL NOT restore it. Only
`devcroft trust` SHALL approve again. `up --recreate` and every automatic start
SHALL NOT approve.

#### Scenario: One byte changes
- **WHEN** an approved manifest gains a trailing space and `up` runs
- **THEN** the checkout is untrusted and `up` names the keys not applied and `devcroft trust`

#### Scenario: Reverting does not restore trust
- **WHEN** that byte is removed again and `up` runs
- **THEN** the checkout is still untrusted

#### Scenario: Recreation does not approve
- **WHEN** a sandbox of an invalidated checkout is brought up with `up --recreate` or by `devcroft shell`
- **THEN** it starts untrusted

### Requirement: Admission covers every attachment
`Meta` SHALL record each generation's trust state, manifest hash and approval.
Before attaching through `up` (already up), `exec`, `shell`, `ssh`, `proxy`,
`agent run` or `agent acp`, devcroft SHALL resolve the checkout and refuse,
naming `--name`, a sandbox whose recorded root differs. It SHALL attach to a
generation started `trusted` only when the current bytes hash to that
generation's recorded hash and the approval is `approved` for it. Otherwise it
SHALL fail at layer `config` (exit code 2) naming `devcroft down` followed by
`devcroft up`, or `devcroft trust` followed by `devcroft up --recreate`. An
untrusted generation SHALL be attachable whatever the current bytes. Sessions
already running SHALL NOT be ended, and `status` SHALL say that a changed hash
does not revoke them. Every refusal SHALL be written to `stderr` only. `proxy`
SHALL write nothing to `stdout` when it refuses, so an SSH client running it as
`ProxyCommand` reports the error instead of a protocol failure.

#### Scenario: Refusal through ProxyCommand
- **WHEN** `ssh NAME.devcroft` reaches `devcroft proxy` for a trusted generation whose manifest changed
- **THEN** nothing is written to `stdout`, and the refusal and its remedies appear on `stderr`

#### Scenario: Attaching after the manifest changed
- **WHEN** a sandbox started trusted is running, its manifest changes, and the user runs `devcroft shell`
- **THEN** it fails at layer `config` naming both remedies, and the running generation is not reported as untrusted

#### Scenario: Untrusted generation
- **WHEN** a sandbox started untrusted is running and its manifest changes
- **THEN** `devcroft shell` attaches

#### Scenario: Same name, another checkout
- **WHEN** `devcroft shell` runs in checkout B and a healthy sandbox of that name belongs to checkout A
- **THEN** it fails naming both roots and `--name`, and does not attach

### Requirement: The decision precedes every phase that reads project input
Admission SHALL decide the trust state immediately after loading the manifest.
This SHALL happen before credential and broker preflight, the egress-proxy
decision, provider resolution, provisioning, environment capture and
compilation, and each of those SHALL read only the effective manifest.

#### Scenario: Forwarded value never read
- **WHEN** an unapproved manifest forwards `MARKER` and `up` runs from a shell that exports it
- **THEN** no host-side phase, log or `Meta` contains its value

#### Scenario: Provisioning network
- **WHEN** an unapproved manifest's activation hook needs a host listed only in `provisioning.network.allow`
- **THEN** the hook fails at layer `provider`, saying the policy refused that host because the manifest is untrusted

### Requirement: The user's credentials reach trusted checkouts only
The user configuration's `[credentials] env` and `[[credentials.broker]]` SHALL
be delivered only to trusted checkouts, and `up` SHALL name each one it
withheld. A selected agent recipe's credential SHALL still be delivered, since
selecting it is an explicit per-sandbox request.

#### Scenario: Global credential and an untrusted checkout
- **WHEN** the user configuration delivers `GITHUB_TOKEN` and an unapproved checkout is brought up
- **THEN** the sandbox has no `GITHUB_TOKEN`, and `up` names it as withheld

#### Scenario: Agent recipe on an untrusted checkout
- **WHEN** an unapproved checkout is brought up with `--agent opencode` under `api-key`
- **THEN** the recipe's broker works and its hosts are reachable

### Requirement: Nothing reintroduces a withheld value or route
For an untrusted checkout, no session option, user-configuration entry,
environment overlay or forwarded socket SHALL deliver a value, a host socket
or a route that admission withheld. This covers the user's SSH agent, a
socket under the user's home directory and every `[env] forward` name.

#### Scenario: SSH agent forwarding
- **WHEN** an unapproved manifest sets `[ssh] forward_agent = true` and the user connects with `ssh -A NAME.devcroft`
- **THEN** no agent socket exists in the session, and `up` named `ssh.forward_agent` as not applied

#### Scenario: Value through a session overlay
- **WHEN** a withheld variable is supplied again through any per-session option or overlay
- **THEN** the session does not receive it

### Requirement: Each provider is qualified for untrusted checkouts
A provider SHALL resolve an untrusted checkout only after its host-side phases
have been shown to read no key that admission did not apply. Those phases are
resolution, capture and the provisioning of its activation script. A provider
not yet qualified SHALL fail at layer `config` for an untrusted checkout,
naming the provider and `devcroft trust`.

#### Scenario: Provider not qualified
- **WHEN** an unapproved checkout selects a provider whose untrusted qualification has not run
- **THEN** `up` fails at layer `config` naming the provider and `devcroft trust`, and resolves nothing

### Requirement: Fleet applies the same decision
Fleet SHALL load, admit and record a project's manifest through the same
admission. A fleet agent SHALL NOT receive a grant the same checkout would not
receive from `up`.

#### Scenario: Fleet on an unapproved checkout
- **WHEN** `fleet up` runs on an unapproved checkout whose manifest allows a host
- **THEN** no fleet agent can reach that host
