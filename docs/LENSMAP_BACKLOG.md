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

## Execution Sequence

1. LBM-001
2. LBM-002
3. LBM-003
4. LBM-004
