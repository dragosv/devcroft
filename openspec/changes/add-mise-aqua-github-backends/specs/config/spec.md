## ADDED Requirements

### Requirement: Portable mise tool declarations and option filtering
For `env.provider = "mise"`, devcroft SHALL accept `[tools]` entries declared as version strings or tables with `version` and only qualified non-executable options (`url` for `http:`, bounded relative `exe`/`bin_path` without `..`, leading `/` or templates). Any `[tools]` entry declaring `postinstall`, `install_env`, template expressions, or an unadmitted backend prefix (`core:`, `npm:`, `pypi:`, `cargo:`, `gem:`, `asdf:`, `vfox:`) SHALL fail before invoking `mise`, naming the tool and the offending key or backend.

#### Scenario: Bare core runtime shorthand is refused with an actionable remedy
- **WHEN** `mise.toml` declares `node = "24.0.0"` whose `mise.lock` entry resolves to `backend = "core:node"`
- **THEN** resolution fails before running `mise install`, explaining that `core:node` executes downloaded binaries on the host and naming `conda:` or `http:` alternatives

#### Scenario: Unsafe postinstall option is refused
- **WHEN** `mise.toml` declares a tool with `postinstall = "./setup.sh"`
- **THEN** resolution fails naming `postinstall` before any `mise` subprocess starts
