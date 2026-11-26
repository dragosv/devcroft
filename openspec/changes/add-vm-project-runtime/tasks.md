## 1. Qualify and define interfaces

- [x] 1.1 Confirm the accepted microVM report and platform-specific required controls; stop adoption if no candidate qualifies. Evidence: `src/vm/qualification.rs` (`macos-arm64` opt-in `libkrun` 2.0 on `macOS 14+` backed by `results/macos-arm64-evaluation-assessment-v16.json`; `linux-arm64`, `linux-x86_64`, `macos-x86_64` explicitly unqualified).
- [x] 1.2 Specify versioned launch/handshake/state records, runtime dispatch and error attribution under the existing layers and exit codes, without changing native instances. Evidence: `src/vm/contract.rs` (`LaunchRecordV1`, `HandshakeV1`, `VmStateRecordV1`, `ErrorLayer`, `#[serde(deny_unknown_fields)]`).
- [x] 1.3 Define trusted artifact manifest, verification keys, update compatibility and packaged guest/helper layout, including the preparation image. Evidence: `src/vm/manifest.rs` (`TrustedArtifactManifestV1`, `PackagedLayout`, repo-bootstrap rejection).
- [x] 1.4 Decide the helper and artifact channel separate from the crate: source obligations and licence inventory for guest artifacts, macOS entitlement and signing, and Git-pinned VMM dependencies. Evidence: `openspec/changes/add-vm-project-runtime/distribution.md` (zero VMM/Git crates in root `Cargo.toml`).
- [x] 1.5 Implement the user-owned runtime grant; test that a manifest or Darwin-only provider cannot reach native without it, with and without a terminal. Evidence: `src/vm/grants.rs` (`RuntimeGrantStore`, `resolve_runtime`, TTY and non-TTY tests).

## 2. Build the isolated execution path

- [x] 2.1 Add helper launch with descriptor minimization, pre-vCPU confinement and bounded per-instance endpoints. Evidence: `src/vm/launcher.rs` (`prepare_instance_layout`, `write_private_launch_record`, `spawn_helper`, `BoundedLogCapture`, `HostProcessIdentity`).
- [x] 2.2 Add Linux guest init, mounts, identity, reaping and keeper bootstrap with handshake validation. Evidence: `src/vm/guest_init.rs` (`GuestInitPlanV1`, `wait_for_verified_handshake`, `StopQuiescePlan`, `verify_stop_acknowledgement`).
- [x] 2.3 Connect host exec/control transport to the guest keeper and test PID/PTY/signal/exit behavior. Evidence: `src/exec.rs` (`exec_on_socket`, `connect_and_spawn_socket`) and `src/vm/transport.rs` (`VmControlClient`, `VmKeeperClient`, `VmSession`).
- [x] 2.4 Connect guest SSH/SFTP/local forwarding; test guest filesystem/network semantics, distinct instance keys and client configuration that offers no agent, X11 or remote forwarding. Evidence: `src/vm/ssh.rs` (`VmSshKeyMaterial`, `VmSshBootstrapSeed`, `render_vm_ssh_config_host`, `build_vm_ssh_client_args`).
- [x] 2.5 Add private preparation jobs and target-specific provider/tooling capture with declared input sets, per-provider Linux requalification, generated profiles and canonical guest paths. Evidence: `src/vm/preparation.rs` (`requalify_provider_for_linux`, `collect_recipe_identity`, `PreparationJobSpecV1`, `CapturedLinuxProfileV1`).
- [x] 2.6 Add controller-owned atomic artifact publication, immutable identity/provenance, per-project namespaces and retention leases; test interrupted publication and that one project's output cannot reach another's consumer. Evidence: `src/vm/publication.rs` (`ArtifactCacheStore`, `stage_guest_output`, `commit_publication`, `lookup_for_project`, `acquire_lease`, `collect_unleased`).
- [x] 2.7 Classify editor clients, gate `ssh-config` entries on the trusted-editor grant, and connect a guest-served editor through a local forward. Evidence: `src/vm/editor.rs` (`classify_editor`, `ssh_config_for_vm_instance`, `guest_served_editor_forward_args`) and `src/vm/grants.rs` (`grant_trusted_editor`).

## 3. Enforce authority and phase transitions

- [ ] 3.1 Compile project requests against user-owned grants, the exploration profile and runtime limits; render complete authority with origins and no secret values.
- [ ] 3.2 Connect phase-scoped host proxy channels without NIC/TSI or generic host-connect, deriving instance identity from the channel; add private-address refusal beyond `HostFilter`, report refused destinations, and test DNS/private-address and channel bypass cases.
- [ ] 3.3 End setup on the host: switch channel policy, close streams and reboot before runtime; test an intentionally lingering background connection and process.
- [ ] 3.4 Implement scoped post-boot direct credential delivery, stated per instance and dropped on refresh, and refuse shared publication of credential-bearing session state.
- [ ] 3.5 Enforce finite disk/log/transport budgets and qualify host-helper resource controls per platform.
- [ ] 3.6 Implement setup windows with an init-owned administrative session reached over control; test that keeper sessions cannot elevate.

## 4. Complete lifecycle and migration

- [ ] 4.1 Persist instance generation, process ownership, target/artifact identities and independent health states.
- [ ] 4.2 Implement graceful down, forced-stop reporting and cold restart recovery without losing private data.
- [ ] 4.3 Implement owned-resource removal and leased-cache collection; test PID reuse, sibling survival and incompatible disks.
- [ ] 4.4 Run adversarial transport/host-access tests with positive controls and complete guest workload regressions on each supported row.
- [ ] 4.5 Reconcile decisions, threat model, provider/lifecycle/SSH/policy specs, README, `CLAUDE.md`, capabilities, each changed invariant, `add-agent-interaction` and `add-macos-service-vm` before release/default migration.
- [ ] 4.6 Verify native regression behavior and rollback/export access for retained VM disks; document unsupported targets without fallback.
