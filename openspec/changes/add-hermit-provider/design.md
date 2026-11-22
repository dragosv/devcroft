## Context

Documented facts, read 2026-11-22. None is measured.

| Fact | Source |
|---|---|
| `bin/hermit.hcl` holds `env`, `sources`, `manage-git`, `inherit-parent`, `github-token-auth`, `idea` | <https://cashapp.github.io/hermit/usage/config/> |
| A git source may carry "an optional `#<tag>` suffix"; commit pins are not documented | same |
| Sources may be git repos, `file://` paths, or `env:///<dir>` ("Environment relative … Useful for local overrides") | same |
| "Hermit syncs manifest sources every 24 hours from HEAD" | <https://cashapp.github.io/hermit/packaging/reference> |
| Manifests carry `sha256`, `sha256sums` or `sha256-source`; whether one is mandatory is not stated | <https://cashapp.github.io/hermit/packaging/schema/manifest/> |
| Triggers: `unpack`, `install`, `uninstall`, `activate`, `exec` ("will run for _every_ execution") | <https://cashapp.github.io/hermit/packaging/reference> |
| Packages install to `<hermit-state>/pkg/<pkg-selector>`, e.g. `~/.cache/hermit/pkg/…` | same, and <https://cashapp.github.io/hermit/usage/management> |
| `hermit env` displays the environment's variables; `--activate` prints the commands to activate | <https://cashapp.github.io/hermit/usage/envars/> |
| Activation installs a shell hook that "executes prior to each command" | <https://cashapp.github.io/hermit/usage/shell/> |
| "designed to manage development _tools_ only"; "deliberately not in the business of installing libraries"; binary packages | <https://cashapp.github.io/hermit/faq> |

Hypotheses for group 0: the project records versions as `bin/.<pkg>-<version>.pkg`
links; a state-directory variable exists; a git source accepts `#<commit>`.

## Goals / Non-Goals

**Goals:**
- Replace an unmeasured "qualified" with a measured verdict.
- If admitted, the mise/pixi contract with no new shared machinery.

**Non-Goals:**
- Running hermit's shell hook, or `bin/activate-hermit`, anywhere.
- Hermit's self-update and `bin/hermit` bootstrap. devcroft calls a hermit
  binary from the host, at a qualified version.
- Local or project-authored manifests.

## Decisions

### H1. The verdict is the deliverable

The repository makes a claim in four places. The change is complete when that
claim is measured, whether the result is a provider or a corrected rejection.
*Alternative considered:* just deleting the "qualified" wording. Rejected
because the wording would then rest on documentation alone, which is what
went wrong the first time.

### H2. The lock is the project's versions plus a pinned source revision

A restorable environment needs both the selected versions and the manifest
text that maps them to URLs and checksums. devcroft requires every source in
`bin/hermit.hcl` to be a git source pinned to an immutable revision, and
verifies that revision (a commit, or a tag resolved to a commit and recorded
in devcroft's object). An unpinned source, a `file://` source or an `env:///`
source is refused by source. If hermit accepts no immutable pin (0.1), the
verdict is a criterion 2 rejection.

### H3. Command triggers are refused by package

devcroft reads each selected package's manifest at the pinned revision before
any install. A `run` action on any trigger, and any `exec` trigger, refuses
the package with its name and the trigger. Which non-command actions (`chmod`,
`rename`, `symlink`, `copy`, `mkdir`, `delete`, `message`) are admitted is
decided in 0.3 by whether hermit executes anything to perform them.

### H4. Checksums are required, not trusted to exist

A selected package whose manifest yields no checksum for the current platform
is refused. `sha256-source` (a URL to a checksum file) is admitted only if 0.2
shows that hermit verifies against that file and fails closed when it cannot
fetch it.

### H5. Capture and store as for mise

`hermit env` output parsed as data (0.4 records its grammar), the project's
`hermit.hcl` `env` entries interpreted by devcroft, hermit state inside
`~/.local/share/devcroft-artifacts/hermit/<id>/` through `provider::artifacts`.

## Risks / Trade-offs

- [Most published manifests have command triggers] → that is a measured
  rejection by package, reported by `doctor`. If it leaves too little, that
  is the verdict.
- [Hermit's sync-from-HEAD runs even when a pin exists] → 0.1 measures whether
  a pinned source is still fetched or rewritten. If it is, H2 fails.
- [Small user base] → not scheduled until demand. The correction of the claim
  does not need demand.

## Open Questions

- Does the owner want the verdict now (group 0 and group 6 only), separately
  from any implementation?
