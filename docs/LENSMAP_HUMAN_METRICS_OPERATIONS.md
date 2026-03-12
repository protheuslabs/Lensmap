# LensMap Human Metrics Operations

This document captures non-automatable outcomes and governance actions that must remain human-reviewed, per the SRS requirement [LM-SRS-031] in [LENSMAP_SRS.md](/Users/jay/.openclaw/workspace/apps/lensmap/docs/LENSMAP_SRS.md).

## Scope

- Human validation tasks that are not reliably machine-measurable.
- Review-quality judgments that require context and intent.
- Adoption and process metrics requiring peer confirmation and interviews.
- Exception governance and escalation quality checks.

## Non-Automatable KPI Set

- Context retrieval quality: periodic sampling of whether developers can correctly answer design questions from LensMap artifacts and notes.
- Note usefulness: reviewer assessment of whether notes reduce ambiguity or duplicate existing inline docs.
- Adoption friction: qualitative blockers, hesitation points, and manual workarounds used by engineers.
- Policy exception appropriateness: whether temporary relaxations are justified, timeboxed, and documented.
- Cross-team alignment: whether architecture owners and reviewers agree that policy thresholds are set to realistic and valuable levels.
- Auto-extraction ambiguity review: periodic review of low-confidence imports and conflict cases routed out of automation.

## Process

- Cadence
  - Weekly: 30-minute team scorecard review in engineering operations.
  - Bi-weekly: 60-minute governance review for critical-path repos.
  - Monthly: executive readout on trend quality and adoption blockers.
- Data sources
  - Latest scorecards from automated `metrics/scorecard` outputs.
  - PR review feedback and sprint retrospective notes.
  - Incident and rollback notes from CI policy violations.
  - `autobot` low-confidence proposal batch logs and manual resolutions.
- Ownership
  - By default, the LensMap owner or delegated team lead owns the review.
  - Governance lead validates exception and escalation decisions.

## Review Artifacts

- Keep a dated note each review cycle with:
  - what improved,
  - what degraded,
  - action decisions,
  - owner assignments,
  - follow-up target dates.
- Add links to the relevant automated evidence snapshots.

## Escalation Triggers

- Escalate when:
  - context lookup quality falls below agreed threshold twice in a row,
  - unresolved policy exceptions exceed agreed cap for two consecutive cycles,
   - auto-import ambiguity volume exceeds agreed ratio over two consecutive cycles,
  - adoption friction is reported in more than two major teams in the same cycle.
- Escalation path:
  - team lead → governance lead → engineering leadership.

## Closure Criteria

- A metric cycle is considered complete when:
  - all required fields above are updated,
  - decisions are assigned,
  - and unresolved actions are carried to the next governance cycle.
