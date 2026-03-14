# LensMap Backlog (Scale-Readiness Requirements)

This backlog converts selected SRS items into actionable implementation tasks for the mission of high-leverage operation at 30M LOC scale.

## Backlog Convention

- `P0`: critical for scaling and continuity
- `P1`: high-value reliability/performance
- `P2`: quality hardening / UX clarity
- Status: `queued` / `in_progress` / `blocked` / `done`

### LBM-001 — Continuity handoff packet for single-operator state
- **SRS Linkage**: LM-SRS-037
- **Priority**: P0
- **Scope**: `.lensmap/handoff/` or `.lensmap/state/`
- **Description**
  - Add a deterministic handoff packet export command (for example `lensmap handoff`) that captures current operator-critical context.
  - Include unresolved critical risks, latest policy/gate outcomes, confidence thresholds, and continuation state.
  - Ensure packet carries format version and checksum for deterministic replay.
- **Acceptance**
  - Two generated packets from identical inputs are identical byte-for-byte.
  - Import path can continue context with no full re-ingest required.
  - Packet explicitly flags incomplete context and any unresolved confidence blockers.
- **Tests**
  - Deterministic output test across stable inputs.
  - Import/reload test validates preserved continuation state.

### LBM-002 — Cognitive load profile for operator triage
- **SRS Linkage**: LM-SRS-038
- **Priority**: P0
- **Scope**: `lensmap atlas`, report command rendering, `lensmap config`
- **Description**
  - Implement profile presets (`focused`, `incident`, `context-build`) that filter and weight surfaced signals.
  - Preserve all policy-failing and risk-blocking signals even under bounded suppression.
  - Persist profile definitions and replay parameters for audit.
- **Acceptance**
  - Profile output is deterministic for same inputs.
  - Report output is strictly bounded by profile while never omitting hard-blocking signals.
  - Profile cut-off criteria are explainable in command output.
- **Tests**
  - Profile determinism test for each preset.
  - Blocking-signal preservation test under aggressive suppression.

### LBM-003 — Cross-boundary risk amplification modeling
- **SRS Linkage**: LM-SRS-039
- **Priority**: P0
- **Scope**: atlas/risk scoring inputs, `pr report`, policy gate output
- **Description**
  - Add cross-boundary detection between policy/compliance/team/domain transitions at symbol/module level.
  - Introduce deterministic boundary amplification multiplier in risk computation and operator prioritization.
  - Emit actionable boundary guidance in PR and atlas-style reports.
- **Acceptance**
  - Boundary-coupled entities are tagged with origin domain and crossing type.
  - Risk amplification affects ranking in a deterministic, traceable way.
  - Guidance links to review steps exist for all detected high-risk transitions.
- **Tests**
  - Coupling detection fixture test.
  - Ranking determinism test with/without boundary-amplification term.

### LBM-004 — Atlas delta and operator trend continuity
- **SRS Linkage**: LM-SRS-040
- **Priority**: P1
- **Scope**: `lensmap atlas --delta`, persistent evidence snapshots
- **Description**
  - Implement deterministic delta mode between snapshots/refs with score shifts, violation deltas, and impact estimate.
  - Make output machine-readable for long-range trend tooling and CI extraction.
- **Acceptance**
  - Deltas are stable on replay of same input set.
  - Added/removed risk units and score shifts are explicit and parseable.
  - Operators can tune confidence thresholds and auto-accept behavior in config.
- **Tests**
  - Atlas delta test for add/remove/score-shift cases.
  - Machine-readability schema test for delta artifact.

### LBM-005 — CLI core modularization and command boundary split
- **SRS Linkage**: LM-SRS-047, LM-SRS-028
- **Priority**: P0
- **Scope**: `crates/lensmap-cli/src/`
- **Description**
  - Split `main.rs` into bounded modules (`args`, `domain`, `policy`, `parser`, `index`, `gov`, `pack`, `audit`).
  - Preserve existing command signatures while moving ownership of deterministic behaviors into dedicated modules.
  - Introduce typed command context structs for cross-command reuse and replay consistency.
- **Acceptance**
  - Main command and legacy surface remain behavior-compatible.
  - Deterministic command outputs remain stable for unchanged input snapshots.
  - New module boundaries are documented and referenced from `LENSMAP_TECH_STACK.md`.
- **Tests**
  - Behavior parity test for a representative command set.
  - CLI help surface regression test.

### LBM-006 — Deterministic evidence ledger and replay
- **SRS Linkage**: LM-SRS-027, LM-SRS-046, LM-SRS-047
- **Priority**: P0
- **Scope**: policy/check outputs, `summary`, `pr report`, `package evidence`, `verify`
- **Description**
  - Add hash-chainable run ledger and deterministic evidence records for policy/report/packaging commands.
  - Emit machine-parseable integrity metadata and support `lensmap verify` replay checks.
  - Ensure evidence can be revalidated without re-running all heavy commands.
- **Acceptance**
  - Replay verification identifies tampering deterministically.
  - Evidence envelopes include command, policy hash, parameter hash, and timestamps.
  - Replay output is stable for unchanged historical inputs.
- **Tests**
  - Ledger tamper detection test.
  - Replay verification and deterministic envelope test.

### LBM-007 — Policy federation and precedence engine
- **SRS Linkage**: LM-SRS-026, LM-SRS-030, LM-SRS-029
- **Priority**: P0
- **Scope**: `policy init`, `policy check`, policy config loader
- **Description**
  - Add explicit policy source precedence: global -> repo -> profile -> per-command.
- Implement strict conflict reporting and deterministic precedence witness logs.
  - Add optional policy bundle reference/import flow for fleet rollout while preserving local overrides.
- **Acceptance**
  - Conflict reports include deterministic precedence chain.
  - Federation mode does not alter local-restricted policy behavior unless explicitly acknowledged.
  - Missing policy version metadata blocks strict mode.
- **Tests**
  - Precedence consistency test.
  - Fleet baseline drift detection and block-on-unknown-path test.

### LBM-008 — Resilient scaling and resumable execution
- **SRS Linkage**: LM-SRS-028, LM-SRS-025
- **Priority**: P1
- **Scope**: scanning pipeline, package pipeline, restore flow
- **Description**
  - Add bounded parallel execution with checkpointed progress for scan+report+package commands.
  - Add resumable restore checkpoints and interruption-safe state commit strategy.
  - Add lock/ownership model for overlapping command work areas when operating under automation pressure.
- **Acceptance**
  - Resume from checkpoint reproduces identical state lineage.
  - No corrupted partial writes in aborted or interrupted executions.
  - Resume behavior is deterministic and auditable.
- **Tests**
  - Resume-from-checkpoint deterministic regression test.
  - Interruption safety and recovery test.

### LBM-009 — Deterministic enterprise integrations plane
- **SRS Linkage**: LM-SRS-029
- **Priority**: P1
- **Scope**: CI event hooks, optional connectors, external evidence endpoints
- **Description**
  - Define schema-locked event payloads for policy failures, report artifacts, and package/restore outcomes.
  - Add typed connectors with idempotent retries for external systems.
  - Document connector compatibility and deprecation policy.
- **Acceptance**
  - Connector payloads are deterministic and versioned.
- Integration failures are fail-closed in strict mode.
  - Existing behavior is unchanged when connectors are disabled.
- **Tests**
  - Connector schema compatibility test.
  - Idempotent retry test.

### LBM-010 — Enterprise release integrity hardening
- **SRS Linkage**: LM-SRS-046, LM-SRS-047
- **Priority**: P1
- **Scope**: install scripts, release manifests, package artifacts
- **Description**
  - Add explicit checksum verification defaults and signed manifest verification for install flows.
  - Normalize release metadata schema for reproducibility and audit replay.
  - Link install/package verification failures to standard evidence envelopes.
- **Acceptance**
  - Default install path includes explicit integrity verification controls.
  - Release manifest includes enough inputs to verify command reproducibility.
  - Integrity failures block release-grade commands.
- **Tests**
  - Corrupted artifact rejection test.
  - Reproducible manifest and verification path test.

### LBM-011 — Enterprise support, security policy, and trust documentation
- **SRS Linkage**: LM-SRS-041, LM-SRS-045, LM-SRS-047
- **Priority**: P1
- **Scope**: repository metadata, release documentation, `README.md`, `SECURITY.md`
- **Description**
  - Add and maintain public enterprise trust files (`SECURITY.md`, support matrix, governance policy, incident contact), and version them with repository release tags.
  - Add a trust banner and release policy section in README with explicit scope, supported versions, and support channels.
  - Require these files to be included in release artifact manifests and evidence envelopes.
- **Acceptance**
  - `SECURITY.md` and support policy file are discoverable and referenced in root docs.
  - Release manifest includes doc hash set and policy hash used for trust verification.
  - Evidence checks fail closed if required trust files are missing.
- **Tests**
  - Trust surface lint and presence test.
  - Manifest digest mismatch test for modified trust docs.

### LBM-012 — Enterprise CI observability and auto-remediation
- **SRS Linkage**: LM-SRS-047, LM-SRS-029, LM-SRS-025
- **Priority**: P1
- **Scope**: `.github/workflows/`, workflow telemetry artifacts
- **Description**
  - Add explicit CI health budget checks for critical workflows (policy, strip, package, release, verify), including failure trend tracking.
  - Emit deterministic workflow health receipts with recurring failure provenance.
- **Acceptance**
  - Every critical workflow emits health status for each required gate.
  - Repeated failure signatures include root cause category and expected remediation suggestions.
  - Health gate can optionally fail release commands when strict governance is enabled.
- **Tests**
  - CI health receipt generation test.
  - Workflow failure pattern replay test.

### LBM-013 — Cross-repo policy federation rollout plane
- **SRS Linkage**: LM-SRS-042, LM-SRS-026, LM-SRS-029
- **Priority**: P0
- **Scope**: policy init/check, policy distribution bundle
- **Description**
  - Add signed policy bundle publishing/import flow for fleet org-level rollout.
  - Add versioned policy provenance with deterministic drift detection and enforcement warnings for local overrides.
- **Acceptance**
  - Policy bundles are deterministic and signed.
  - Drift detection reports include exact precedence witness and conflicting rule source.
  - Local override mode can be approved but must emit explicit risk statement in run evidence.
- **Tests**
  - Policy bundle signature and drift test.
  - Deterministic override conflict resolution test.

### LBM-014 — Distribution and install pipeline hardening
- **SRS Linkage**: LM-SRS-046, LM-SRS-047, LM-SRS-040
- **Priority**: P1
- **Scope**: `scripts/install.sh`, release packaging, package manifests
- **Description**
  - Make integrity checks on install default-on and fail-closed, including mandatory checksum and signed manifest validation.
  - Add deterministic fallback mode for fully offline installation with local artifact trust cache.
- **Acceptance**
  - Offline installation path validates local cache provenance.
  - Corrupted or unsigned artifacts always block install.
  - Install receipts include installer command inputs and verification outcomes.
- **Tests**
  - Offline integrity test.
  - Corrupted cache and unsigned artifact rejection tests.

### LBM-015 — Enterprise toolchain interoperability and extension governance
- **SRS Linkage**: LM-SRS-029, LM-SRS-030, LM-SRS-045
- **Priority**: P1
- **Scope**: `lensmap ext`/editor extension integration, connector registry
- **Description**
  - Add deterministic extension contract tests for editor plugins/extension outputs to avoid drift against core command outputs.
  - Add extension compatibility matrix and deprecation policy with pinned minimum LensMap version expectations.
- **Acceptance**
  - Extension contracts remain stable across patch releases.
  - Version mismatch policy failures are deterministic and include upgrade path.
- **Tests**
  - Extension compatibility matrix regression test.
  - Version/contract mismatch enforcement test.

## Execution Sequence

1. LBM-001
2. LBM-002
3. LBM-003
4. LBM-004
5. LBM-005
6. LBM-006
7. LBM-007
8. LBM-008
9. LBM-009
10. LBM-010
11. LBM-011
12. LBM-012
13. LBM-013
14. LBM-014
15. LBM-015
