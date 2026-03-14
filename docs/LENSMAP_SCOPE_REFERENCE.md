# LensMap ⇄ Prism Reference (Boundary Contract)

## LensMap Scope

LensMap owns source-linked knowledge and governance hygiene for code artifacts.

In scope:
- Anchor lifecycle and resolution (insert/refresh/remove during code refactors)
- Note extraction and restore operations
- Policy and hygiene checks for ownership/review/evidence requirements
- Packaging and evidence artifact generation for CI/report consumers
- API and editor integrations around annotation workflows

Out of scope:
- Work prioritization and scheduling
- Incident lifecycle management
- Remediation execution orchestration
- Resource budgeting and operator runbook decisions

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
