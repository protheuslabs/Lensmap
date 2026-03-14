# LensMap ⇄ Prism Reference (Boundary Contract)

## Philosophy

LensMap is the "knowledge compression" half of the system.

It exists so a single operator can hold a 30M LOC estate in fast mental focus by replacing inline noise with durable, symbol-linked knowledge artifacts.

The principle is the same one Archimedes used: increase leverage by reducing cognitive friction per decision.
- Long leverage = high-quality anchors, clear context, and policy-grounded evidence.
- Stable anchors and canonical policy surfaces keep one person productive as scale grows.

## LensMap Scope

LensMap owns source-linked knowledge and governance hygiene for code artifacts.

In scope:
- Anchor lifecycle and resolution (insert/refresh/remove during code refactors)
- Note extraction and restore operations
- Policy and hygiene checks for ownership/review/evidence requirements
- Packaging and evidence artifact generation for CI/report consumers
- API and editor integrations around annotation workflows
- Atlas summaries, handoff packets, and deterministic continuity outputs for one-operator scale

Execution priority for one-operator continuity and scale-readiness requirements is tracked in [`LENSMAP_BACKLOG.md`](/Users/jay/.openclaw/workspace/apps/lensmap/docs/LENSMAP_BACKLOG.md).

Implementation sequencing and stack target are defined in:
[`LENSMAP_TECH_STACK.md`](/Users/jay/.openclaw/workspace/apps/lensmap/docs/LENSMAP_TECH_STACK.md).

Out of scope:
- Work prioritization and scheduling
- Incident lifecycle management
- Remediation execution orchestration
- Resource budgeting and operator runbook decisions

## Why this boundary exists

Without the boundary, operational policy and code annotation become entangled.

With this boundary:
- LensMap keeps the source truth compact and explainable.
- Prism keeps the action surface bounded to what one operator can execute safely.
- Both remain independently improvable and testable.

## Prism Inputs from LensMap

Prism should only consume LensMap output artifacts as signals, including:
- Policy/validation findings (violations, warnings, blockers)
- Owner/review/state metadata keyed by anchors or symbols
- Certification and compliance tags mapped to scope/domain
- Staleness and debt indicators from code-linked notes
- Any deterministic index artifacts the repo publishes for machine consumption

## Hand-off Rule

LensMap provides "what is true" about code semantics and governance.
Prism provides "what to do now" based on that signal plus change/churn and operational constraints.
