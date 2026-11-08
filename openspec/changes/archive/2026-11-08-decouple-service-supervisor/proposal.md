## Why

**devcroft requires every project that wants services to install a specific
third-party binary into its own environment manifest.** When
`process-compose` is not in the resolved environment, `up` fails at layer
`provider` telling the user to *"add it to the environment manifest"*
(`up.rs:876`).

That is devcroft's requirement leaking into the user's project. And the
choice is devcroft's alone: `add-flox-services` decision 1 rejected both
shelling out to `flox services start` and consuming flox's generated
`service-config.yaml`, so devcroft generates its own config and runs its own
supervisor. That flox and devenv also use process-compose internally is a
coincidence devcroft does not rely on — it never touches their instance.

An earlier reading of this treated the coupling as "which supervisor does the
provider use", concluded every provider uses the same one, and called an
abstraction speculative. That answered the wrong question. The right one is
*whose choice is it*, and the answer is devcroft's — which makes a second
implementation not a hypothetical provider and not a test double, but
devcroft's own alternative, whose purpose is to remove a requirement it
currently imposes on users.

## What Changes

- **NEW** `service-supervisor-seam`: the four places devcroft is
  process-compose-specific move behind one trait, with `process-compose` as
  the implementation that ships. Behaviour is unchanged; what changes is that
  a second supervisor becomes an implementation rather than edits scattered
  across three files.
- The supervisor becomes a named, inspectable thing rather than an
  assumption — `doctor`/`status` can say which one a sandbox uses, and the
  refusal message can name it.
- **Not in this change**: a second supervisor. This change makes one
  possible; building devcroft's own is separate work with its own trade-offs
  (see Non-Goals), and belongs on the roadmap rather than smuggled in here.

## Capabilities

### New Capabilities

- `service-supervisor-seam`: what a supervisor must provide, what stays
  supervisor-agnostic, and what devcroft may and may not assume about one.

### Modified Capabilities

- (none — `openspec/specs/` holds no synced specs. The `services` capability
  this refactors lives in the unarchived `add-flox-services`; this change
  preserves its requirements exactly and moves where they are implemented.)

## Impact

- **Affected code, and it is narrower than it looks.** Four points are
  process-compose-specific:
  `services::render_config` (config schema), `services::resolve_in_env`
  (the binary name), `services::query` (`GET /processes` over a unix socket),
  and `bin/devcroft.rs`'s `start_services_if_requested` (`cmd:
  "process-compose"` and its arguments). Everything else in `src/services` is
  already supervisor-agnostic — `socket_path`, `config_path`, `log_path`,
  `artifact_dir`, `reconcile`, `ServiceState`, `ServiceHealth` — which is
  half the module's public surface.
- **No behaviour change.** The shipped supervisor stays process-compose, with
  the same config, the same protocol, the same refusal when it is missing.
  The test for this change is that nothing observable moves.
- **Unblocks** a supervisor devcroft ships itself, which would remove the
  "add it to the environment manifest" requirement — but only for projects
  willing to accept less than process-compose provides. That trade is real
  and is why it is separate work.

## Non-Goals

- **Not building a second supervisor.** Restart policy, inter-service
  dependencies and daemon handling are why process-compose was chosen
  (`add-flox-services` decision 1), not incidental. A devcroft-owned
  supervisor either reimplements them or offers less, and which of those is
  acceptable is a product decision this change does not make.
- **Not consuming the provider's supervisor.** That is what decision 1
  already rejected — flox's `service-config.yaml` is an undocumented internal
  artifact and its process-compose belongs to flox's closure. Decoupling
  devcroft's *own* supervisor choice is the opposite move and does not
  reopen it.
- **Not a manifest key.** Nothing here lets a project pick its supervisor.
  Whether that should ever exist is downstream of there being more than one.
