## 1. Qualify and define interfaces

- [ ] 1.1 Confirm the accepted microVM report and platform-specific required controls; stop adoption if no candidate qualifies.
- [ ] 1.2 Specify versioned launch/handshake/state records, runtime dispatch and error attribution without changing native instances.
- [ ] 1.3 Define trusted artifact manifest, verification keys, update compatibility and packaged guest/helper layout.

## 2. Build the isolated execution path

- [ ] 2.1 Add helper launch with descriptor minimization, pre-vCPU confinement and bounded per-instance endpoints.
- [ ] 2.2 Add Linux guest init, mounts, identity, reaping and keeper bootstrap with handshake validation.
- [ ] 2.3 Connect host exec/control transport to the guest keeper and test PID/PTY/signal/exit behavior.
- [ ] 2.4 Connect guest SSH/SFTP/local forwarding; test guest filesystem/network semantics and distinct instance keys.
- [ ] 2.5 Add private preparation jobs and target-specific provider/tooling capture with generated profiles and canonical guest paths.
- [ ] 2.6 Add controller-owned atomic artifact publication, immutable identity/provenance and retention leases; test interrupted publication.

## 3. Enforce authority and phase transitions

- [ ] 3.1 Compile project requests against user-owned grants and runtime limits; render complete authority with origins and no secret values.
- [ ] 3.2 Connect phase-scoped host proxy channels without NIC/TSI or generic host-connect; test DNS/private-address and channel bypass cases.
- [ ] 3.3 Revoke setup descendants, tokens and existing streams before runtime; test an intentionally lingering background connection.
- [ ] 3.4 Implement scoped post-boot direct credential delivery and refuse shared publication of credential-bearing session state.
- [ ] 3.5 Enforce finite disk/log/transport budgets and qualify host-helper resource controls per platform.

## 4. Complete lifecycle and migration

- [ ] 4.1 Persist instance generation, process ownership, target/artifact identities and independent health states.
- [ ] 4.2 Implement graceful down, forced-stop reporting and cold restart recovery without losing private data.
- [ ] 4.3 Implement owned-resource removal and leased-cache collection; test PID reuse, sibling survival and incompatible disks.
- [ ] 4.4 Run adversarial transport/host-access tests with positive controls and complete guest workload regressions on each supported row.
- [ ] 4.5 Reconcile decisions, threat model, provider/lifecycle/SSH/policy specs, README and capabilities before release/default migration.
- [ ] 4.6 Verify native regression behavior and rollback/export access for retained VM disks; document unsupported targets without fallback.
