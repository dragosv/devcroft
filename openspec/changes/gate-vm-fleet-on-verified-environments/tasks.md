## 1. Define identity and eligibility

- [ ] 1.1 Implement versioned canonical definition validation and supported reconstruction-adapter qualification, naming each adapter and its hash-pinning rule.
- [ ] 1.2 Implement `environment_id` including the declared input set and runtime compatibility with no secret bytes; check policy and agent tooling at admission instead of hashing them.
- [ ] 1.3 Add controller-owned environment evidence and per-source check records with explicit refusal/staleness reasons; reject guest/project success markers.
- [ ] 1.4 Test that floating inputs fail the offline stage, and test snapshot relabeling, digest-only images and unsupported image reconstruction refusal.

## 2. Reconstruct and verify

- [ ] 2.1 Create disposable verification VMs using the reference revision, declared seed inputs and fresh writable state while preserving the local instance.
- [ ] 2.2 Reuse only eligible immutable caches; report cache reuse separately from executed setup and checks.
- [ ] 2.3 Run adapter fetchers, then ordered setup with the proxy channel closed by the host, service readiness and non-empty verification with deadlines and attributable failures.
- [ ] 2.4 Record outcomes atomically and clean up failed verification; test missing interactive dependencies and skipped required probes.
- [ ] 2.5 Prevent credential-bearing verification disks from becoming shared bases and exclude secret values from records/logs.

## 3. Gate VM fleet launch

- [ ] 3.1 Add fleet preflight for every requested source: matching environment evidence, capabilities and user authority.
- [ ] 3.2 Bind launch to immutable captured inputs and leased verified artifacts; test definition mutation and artifact GC races.
- [ ] 3.3 Create private per-agent workspace/home/service state; stop a task on an environment-class failure and attach project-class check failures to it.
- [ ] 3.4 Handle partial launch failures with explicit sibling status and cleanup of the failed instance.
- [ ] 3.5 Test that a source-only change keeps environment evidence and reruns per-source checks, a lockfile change invalidates evidence, and a branch with failing tests is still admitted.
- [ ] 3.6 Test two agents cannot share writable service state or credentials and runtime divergence cannot mint new evidence.
- [ ] 3.7 Capture per-agent source snapshots through the import path and deliver results as per-agent staged series; test that nothing runs host Git on guest metadata.

## 4. Integrate without overstating guarantees

- [ ] 4.1 Add status/CLI output naming the exact checked workload and differentiating local availability, stale evidence, per-source check results, undelivered agent results and fleet eligibility.
- [ ] 4.2 Reconcile native fleet/workspace plans with VM dispatch, including `workspace-isolation`'s fetch-from-clone integration, and ensure old records never become verified by migration.
- [ ] 4.3 Run local-adapt/save/clean-verify/fleet end-to-end success and failure scenarios, including definition-only external configuration.
- [ ] 4.4 Document strict reconstruction limits, what environment evidence does not say about code, rollback and unsupported credential-dependent shared builds.
