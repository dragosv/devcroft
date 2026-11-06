## 1. Define identity and eligibility

- [ ] 1.1 Implement versioned canonical definition validation and supported reconstruction-adapter qualification.
- [ ] 1.2 Implement separate environment and exact-source verification IDs, including policy/tooling/runtime compatibility and no secret bytes.
- [ ] 1.3 Add controller-owned evidence records and explicit refusal/staleness reasons; reject guest/project success markers.
- [ ] 1.4 Test floating inputs, snapshot relabeling, digest-only images and unsupported image reconstruction refusal.

## 2. Reconstruct and verify

- [ ] 2.1 Create disposable verification VMs using declared source/seed inputs and fresh writable state while preserving the local instance.
- [ ] 2.2 Reuse only eligible immutable caches; report cache reuse separately from executed setup and checks.
- [ ] 2.3 Run ordered setup, service readiness and non-empty verification with deadlines and attributable failures.
- [ ] 2.4 Record outcomes atomically and clean up failed verification; test missing interactive dependencies and skipped required probes.
- [ ] 2.5 Prevent credential-bearing verification disks from becoming shared bases and exclude secret values from records/logs.

## 3. Gate VM fleet launch

- [ ] 3.1 Add fleet preflight for every requested source context with matching evidence, capabilities and user authority.
- [ ] 3.2 Bind launch to immutable captured inputs and leased verified artifacts; test definition mutation and artifact GC races.
- [ ] 3.3 Create private per-agent workspace/home/service state and gate each task on its setup/readiness/startup checks.
- [ ] 3.4 Handle partial launch failures with explicit sibling status and cleanup of the failed instance.
- [ ] 3.5 Test branch/source changes invalidate verification while unchanged environment materialization remains reusable.
- [ ] 3.6 Test two agents cannot share writable service state or credentials and runtime divergence cannot mint new evidence.

## 4. Integrate without overstating guarantees

- [ ] 4.1 Add status/CLI output naming the exact checked workload and differentiating local availability, stale evidence and fleet eligibility.
- [ ] 4.2 Reconcile native fleet/workspace plans with VM dispatch and ensure old records never become verified by migration.
- [ ] 4.3 Run local-adapt/save/clean-verify/fleet end-to-end success and failure scenarios, including definition-only external configuration.
- [ ] 4.4 Document strict reconstruction limits, source-wide invalidation, rollback and unsupported credential-dependent shared builds.
