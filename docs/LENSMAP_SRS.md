# LensMap SRS

## Purpose

LensMap shall reduce knowledge and documentation boilerplate around source code while keeping the code itself lean, inspectable, and machine-manageable.

## Scope

This tranche covers:

- policy-driven annotation templates
- CI and policy checks
- stronger low-noise anchor ergonomics
- team workflow metadata and stale-note governance
- repo-aware summaries
- PR-oriented note reporting
- explicit machine/human/editor artifact layering

## Functional Requirements

### LM-SRS-001 Knowledge Boilerplate Positioning

- LensMap shall explicitly position itself as a `knowledge boilerplate` layer rather than a generic code-generation or framework-boilerplate system.
- LensMap documents shall carry metadata describing the preferred boilerplate scope and artifact layers.
- The human-readable render shall surface that positioning metadata.

### LM-SRS-002 Policy-Driven Templates

- LensMap shall provide built-in note templates for at least:
  - architecture
  - migration
  - audit
  - review
  - decision
  - todo
- LensMap shall support listing available templates and exporting a template file into the repo.
- `annotate` shall support applying a template so repeated note structure and metadata are not entered manually every time.
- Templates shall be able to provide:
  - default kind
  - default title prefix
  - default tags
  - default review cadence
  - default note body skeleton

### LM-SRS-003 Team Workflow Metadata

- LensMap entries shall support structured collaboration metadata including:
  - title
  - owner
  - author
  - scope
  - template
  - tags
  - review status
  - review due date
  - updated at
- `annotate`, search, render, and summaries shall preserve and surface that metadata.

### LM-SRS-004 Policy Checks

- LensMap shall support policy configuration inside the canonical lensmap document metadata.
- LensMap shall support initializing and updating policy settings from the CLI.
- LensMap shall support checking policy compliance from the CLI for CI use.
- Policy checks shall support:
  - requiring owner
  - requiring author
  - requiring template
  - requiring review status
  - stale-note threshold in days
  - required file patterns that must have at least one note
- Policy check output shall distinguish errors from warnings and return non-zero when strict policy fails.

### LM-SRS-005 Repo-Aware Summaries

- LensMap shall provide repo summaries grouped by at least:
  - file
  - directory
  - owner
  - kind
  - template
  - review status
  - scope
- Summaries shall support filtering by changed files from git base/head refs.
- Summaries shall be renderable as Markdown and as structured JSON receipts.

### LM-SRS-006 PR-Oriented Reporting

- LensMap shall provide a PR-oriented report command based on git diff, without requiring GitHub API access.
- PR reports shall summarize:
  - changed files
  - notes attached to changed files
  - stale or unreviewed notes touching the change
  - changed files that have no LensMap entries
- PR reports shall support strict mode for CI gating.

### LM-SRS-007 Artifact Layer Separation

- LensMap shall treat the canonical JSON lensmap as the machine authority.
- LensMap shall maintain a readable Markdown sidecar as the human artifact.
- LensMap shall maintain a search index artifact for editor/repo search workflows.
- `sync` shall refresh the Markdown sidecar and search index alongside canonical reanchor/simplify operations.
- `status` shall report artifact-layer paths and existence.

### LM-SRS-008 Invisible Anchor Ergonomics

- LensMap shall continue to keep anchors inline by default where safe.
- LensMap metadata shall declare editor anchor visibility policy.
- Editor-facing documentation and status output shall expose the current anchor visibility mode as an ergonomic setting rather than raw implementation detail.

### LM-SRS-009 CI and PR System Integration

- LensMap shall provide first-class command integrations for CI and PR pipelines, including:
  - deterministic `policy check` exit status for failure vs warning states,
  - machine-readable reports suitable for build status gating,
  - and optional `--fail-on-warning` behavior for strict repos.
- LensMap shall support PR-oriented report generation from git diff inputs produced by common providers (`origin/main..HEAD`, base refs, or explicit commit ranges).
- LensMap shall emit CI-ready diagnostics that map violations to file, symbol, and note references.

### LM-SRS-010 Ownership and Enforcement Model

- LensMap shall define and enforce ownership semantics for each note, including:
  - owner,
  - steward/backup owner,
  - ownership scope boundaries (directory, module, or entire repo),
  - and policy ownership overrides per module.
- LensMap shall support policy inheritance with explicit precedence:
  - repository default policy,
  - module or directory override policy,
  - explicit per-note policy metadata.
- LensMap shall support rule actions for governance workflows:
  - warning-only,
  - warning escalation to owner,
  - and hard failure for policy-critical requirements.

### LM-SRS-011 Migration and Adoption Playbook

- LensMap shall document and execute a phased rollout path for existing codebases:
  - bootstrap, where all functions may be scanned without intrusive anchor insertion,
  - stabilization, where only targeted symbols receive anchors,
  - normalization, where default templates and policies are applied,
  - steady state, where policy gates are enabled.
- LensMap shall support a migration manifest that records:
  - migration scope,
  - excluded paths,
  - target policy level per phase,
  - and completion checkpoints.
- The migration flow shall produce before/after metrics to confirm baseline, progress, and residual debt.

### LM-SRS-012 Access, Identity, and Audit Controls

- LensMap shall support identifying action origin for sensitive operations (`add`, `annotate`, `policy update`, `merge`).
- LensMap shall support audit logs with immutable-style records containing:
  - actor identifier,
  - action,
  - target file/reference,
  - timestamp,
  - and command arguments hash.
- LensMap shall support a least-privilege operating profile for CI runners, local users, and reviewer identities, including:
  - read-only policy validation modes,
  - write-protected canonical files unless explicitly allowed,
  - and explicit permission failures for restricted paths.

### LM-SRS-013 Adoption and Health Metrics

- LensMap shall generate adoption metrics for governance and planning:
  - coverage ratio by directory and module,
  - note density by file and symbol class,
  - stale note backlog by age and owner,
  - review compliance and overdue percentage.
- LensMap shall expose trend data over time (e.g., week-over-week or run-over-run deltas).
- LensMap shall support an evidence export for leadership reporting:
  - high-level adoption score,
  - risk hotspots,
  - and remediation backlog.

### LM-SRS-030 Quantified Maintainability Outcomes

- LensMap shall expose explicit enterprise scorecard metrics and make them queryable via a `metrics` command:
  - `note_coverage_rate` by repo, directory, module, and critical path classification.
  - `stale_note_ratio` by age bucket and owner.
  - `policy_pass_rate` by check type and protection scope.
  - `mean_time_to_fix_violation` by violation class.
  - `anchor_fidelity_after_refactor` and `symbol_repair_success_rate`.
  - `notes_per_pr` and `notes_reviewed_per_pr`.
  - `orphan_notes_rate` and `no_owner_notes_rate`.
- LensMap shall export scorecards in machine-readable JSON and human-readable Markdown via `metrics` and `scorecard` commands.
- LensMap shall include configurable thresholds and SLO bands (green/yellow/red) in policy and output a structured health report.
- LensMap shall provide delta reports by period (`weekly`, `sprint`, `release`) and trend direction.
- LensMap shall support trend snapshots persisted in repo-local evidence and optional centralized evidence sinks.
- Metrics that depend on operator judgment or team process shall be separated as human-required KPIs in a linked operations document.

### LM-SRS-031 Human Governance Items for Metric Integrity

- LensMap shall maintain a human operations document for non-automatable quality metrics in [LENSMAP_HUMAN_METRICS_OPERATIONS.md](/Users/jay/.openclaw/workspace/apps/lensmap/docs/LENSMAP_HUMAN_METRICS_OPERATIONS.md):
  - review quality rubric,
  - context lookup time sampling,
  - adoption friction review,
  - and policy exception review cadence.
- Human operations outputs shall be linked to scorecard snapshots and tracked as evidence in periodic governance reviews.
- The operations document shall define owners, cadence, and escalation criteria for manually-assessed KPIs.
- Manual KPIs shall be explicitly labeled with confidence and sample methodology.

### LM-SRS-032 Automated Adoption Signal Pipeline

- LensMap shall provide an adoption signal pipeline that continuously publishes:
  - PR-level annotation usage,
  - editor action usage,
  - and note lifecycle state transitions.
- The pipeline shall feed:
  - CI trend badges,
  - release readiness reports,
  - and leadership visibility artifacts.
- LensMap shall support lightweight privacy-safe telemetry and allow strict local-only mode where adoption telemetry is disabled by policy.
- The pipeline shall support attribution windows by team, repo, and critical subsystem.

### LM-SRS-033 Automated Source Annotation Ingestion

- LensMap shall support automatic extraction and transformation of comment/documentation sources into provisional notes:
  - inline source comments (including TODO/FIXME tags),
  - API/doc block style documentation,
  - and PR/issue-referenced text when provided in supported sidecar files.
- LensMap shall infer likely owner scope and candidate tags from file path, symbol metadata, and annotation markers.
- LensMap shall perform deterministic conflict resolution when duplicate anchors or overlapping candidates are found.
- Provisional imports shall be emitted as staging proposals with confidence scores and conflict metadata.
- Operators shall be able to accept, reject, or reroute provisional proposals in batch mode.

### LM-SRS-034 Boilerplate Normalization and Canonicalization

- LensMap shall normalize imported and newly added comments into schema-compliant entries with:
  - standardized note structure,
  - canonical reference formatting,
  - and configurable template application.
- Normalization shall include:
  - language-aware parsing for supported languages,
  - line-to-symbol anchor binding using symbol signatures and AST context,
  - duplicate merge deduplication with stable precedence rules.
- LensMap shall optionally preserve imported text verbatim while adding required metadata fields and governance tags.

### LM-SRS-035 Autonomous Onboarding Pipeline

- LensMap shall provide an `autobot` command that orchestrates:
  - detection of target scope,
  - extraction/import staging,
  - conflict surfacing,
  - migration plan generation,
  - policy enforcement dry-runs,
  - and optional commit-ready artifact preparation.
- The autopipeline shall support profiles:
  - conservative,
  - standard,
  - aggressive.
- Each profile shall control automatic acceptance thresholds, conflict tolerance, and telemetry output.
- `autobot` shall support dry-run mode and approval checkpoints with resumable checkpoints for interruption recovery.
- Autobot outcomes shall produce an adoption receipt containing:
  - files touched,
  - comments harvested,
  - confidence aggregate,
  - unresolved ambiguities,
  - and human review requirements.

### LM-SRS-036 Deterministic Human-Gated Decision Model

- LensMap shall classify every import proposal and policy exception by automation confidence.
- Proposals below confidence threshold shall be routed to human gating workflows and tracked as non-automatable actions in [LENSMAP_HUMAN_METRICS_OPERATIONS.md](/Users/jay/.openclaw/workspace/apps/lensmap/docs/LENSMAP_HUMAN_METRICS_OPERATIONS.md).
- Operators shall be able to define confidence thresholds and allowed auto-accept lists by repo/team/path.
- Human-gated decisions shall be auditable and replay-linked to the originating source context and auto-run attempt ID.
- Unresolved low-confidence proposals shall block strict-mode policy if configured.

## Non-Functional Requirements

### LM-SRS-014 Fail-Closed Safety

- Any path written by LensMap shall remain inside the repository root.
- Packaging and policy-report output shall fail closed when asked to write outside the repo root.

### LM-SRS-015 Backward Compatibility

- Existing lensmap files without the new collaboration metadata shall remain readable.
- Existing commands shall continue to work unless explicitly replaced by a stricter equivalent.

### LM-SRS-016 Auditability

- Every new command added in this tranche shall emit JSON receipts suitable for CI and audit use.

### LM-SRS-017 Rollout and Governance Auditability

- All command runs that perform policy enforcement, migration, or bulk operations shall write a compact run receipt containing:
  - operation type,
  - policy hash,
  - affected counts,
  - and violation summary.
- Receipts shall be deterministic for the same input and policy snapshot.
- Receipts shall support diffing against prior runs to identify drift.

### LM-SRS-018 Scale and Platform Reliability

- LensMap shall support enterprise-scale repositories with predictable performance envelopes.
- LensMap shall process large repository batches in bounded parallel jobs with deterministic ordering and reproducible outputs.
- LensMap shall support command resume checkpoints and rerunnable partial runs for interrupted scans, policies, summaries, and reports.
- LensMap shall include lock handling for concurrent invocations to avoid file corruption and race conditions.
- LensMap shall enforce configurable runtime guardrails for memory and time limits with clear degradation behavior.

### LM-SRS-019 External Workflow Integration

- LensMap shall provide standardized outputs for integration with GitHub, GitLab, Jira, and Azure DevOps style review tooling.
- LensMap shall support PR-comment style rendering mode suitable for review threads and bot-postable annotations.
- LensMap shall provide optional webhook or API hook points for policy violations and PR-risk thresholds.
- LensMap shall support dashboard-friendly status APIs and summary payloads for engineering leadership visibility.

### LM-SRS-020 Enterprise Governance and Lifecycle Policy

- LensMap shall define ownership and support models including team ownership, approver roles, and delegated review authority.
- LensMap shall define and monitor service-level objectives for key LensMap operations, including policy checks, report generation, and packaging/unpack workflows.
- LensMap shall define release and compatibility rules for schema, command behavior, and plugin integrations.
- LensMap shall define escalation and incident handling procedures for policy regressions, security check failures, and migration blockers.

### LM-SRS-021 Production Incident Readiness

- LensMap shall provide explicit rollback paths for schema changes, policy defaults, and bulk migration actions.
- LensMap shall maintain compatibility windows and safe mode behavior for failing upgrades.
- LensMap shall provide incident evidence capture for failed enforcement runs, including evidence bundle collection and reproducibility context.

### LM-SRS-022 Enterprise Identity and Authentication Integration

- LensMap shall support integration with enterprise identity providers for CLI and editor surfaces.
- LensMap shall map local actors to enterprise identities using one of:
  - SSO subject claims,
  - directory identity lookup,
  - signed operation credentials.
- LensMap authorization decisions shall support role-based access control with at least:
  - viewer,
  - reviewer,
  - approver,
  - policy administrator,
  - and repository owner roles.
- LensMap shall support delegated access with scoped time-bound tokens for automation identities and CI service accounts.

### LM-SRS-023 Centralized Governance and Policy Federation

- LensMap shall support a policy federation mode where multiple repositories can inherit policy from a central governance source.
- Central policy must include schema version, release signature, and override rules.
- LensMap shall detect and report policy drift between repository-local policy and central governance policy.
- In federated mode, LensMap shall support explicit per-repo exception overlays with auditability and expiry.
- Fleet-wide governance reports shall be produced for policy compliance across tracked repositories.

### LM-SRS-024 Trust, Provenance, and Artifact Integrity

- LensMap shall issue verifiable provenance for generated artifacts:
  - canonical JSON,
  - sidecar markdown,
  - search index,
  - and report bundles.
- Each artifact emission shall include:
  - hash chain or cryptographic signature,
  - build metadata,
  - command identity,
  - and immutable timestamp.
- LensMap shall provide an artifact verification mode that checks signatures and chain continuity before consuming or merging artifacts.
- LensMap shall preserve provenance across packaging/unpack operations and report mismatches as hard failures in strict mode.

### LM-SRS-025 Enterprise Onboarding and Fleet Rollout

- LensMap shall support bulk repository onboarding workflows with:
  - repo discovery,
  - policy templating,
  - policy + template preflight checks.
- LensMap shall support staged rollout plans with controlled enablement by repo and team.
- Onboarding shall emit implementation receipts with:
  - installed command surface version,
  - policy baseline and thresholds,
  - migration phase state,
  - and first-run adoption score.
- LensMap shall support rollback for onboarding decisions and phased canary promotion across repository sets.

### LM-SRS-026 Platform Control Plane and Deterministic Replay

- LensMap shall support a centrally managed control plane for policy distribution, command versions, and rollout state.
- Control plane operations shall include:
  - repository enrollment,
  - policy bundle delivery,
  - migration wave scheduling,
  - and centralized artifact publication.
- LensMap shall produce immutable run manifests with input hashes, environment hashes, and fixed execution seeds.
- Given the same source state, policy snapshot, and parameters, LensMap shall produce bit-for-bit reproducible outputs.
- LensMap shall support replay of historical runs from archived run manifests.

### LM-SRS-027 Compliance Mapping and Security Assurance

- LensMap shall include an explicit compliance control mapping for common enterprise frameworks (at minimum SOC 2 Type II, ISO/IEC 27001, and CIS-style controls).
- LensMap audit receipts shall include control IDs and evidence references for control review workflows.
- LensMap shall support retention policies for audit evidence by repository, policy class, and legal requirement.
- LensMap shall support privacy-aware redaction options in audit artifacts.
- LensMap shall support non-repudiation-grade signatures for high-sensitivity operations.

### LM-SRS-028 Reliability Engineering and Chaos Readiness

- LensMap shall define SLO/SLI budgets per command family with explicit error, latency, and throughput objectives.
- LensMap shall include chaos and fault-injection test profiles to verify behavior under:
  - interrupted writes,
  - lock contention,
  - partial file corruption,
  - repository size spikes,
  - and transient command failures.
- LensMap shall include automatic retry, backoff, and bounded-circuit-breaker behavior for external integrations.
- LensMap shall generate reliability risk signals and alert-ready events when SLO violations trend toward breach.

### LM-SRS-029 API, Schema, and Plugin Compatibility Governance

- LensMap shall maintain a documented compatibility matrix for:
  - CLI interfaces,
  - JSON schema versions,
  - search index formats,
  - editor extension versions (VS Code and JetBrains),
  - and output report contracts.
- LensMap shall provide deprecation notices, sunset windows, and migration assistants for breaking changes.
- LensMap shall support staged API migration flags to preserve compatibility for downstream tooling.
- LensMap shall provide machine-readable compatibility manifests for enterprise packaging and dependency automation.

## Acceptance Criteria

- A repo can define strict LensMap policy and check it from CI.
- A user can add a structured note from a built-in template without manually retyping the same metadata.
- A user can generate a repo summary grouped by owner or directory.
- A user can generate a PR report from git diff.
- `sync` refreshes both the Markdown sidecar and search index.
- `status` shows the canonical JSON path, Markdown sidecar path, and search index path.
- An operator can run migration phases and prove adoption progress with metric receipts.
- CI can gate PR merges using deterministic policy and PR-report checks.
- Governance can trace ownership and audit actions for compliance reviews.
- LensMap can operate at monorepo scale with parallel execution, resumable checkpoints, and concurrency safety.
- LensMap can publish/consume structured review-ready artifacts for GitHub/Jira/Azure workflows.
- Governance can monitor and enforce SLOs, incidents, and release rollback behavior.
- LensMap can enforce enterprise identity mappings and role-based authorization for sensitive operations.
- LensMap supports centralized policy federation and fleet-wide compliance reporting.
- LensMap artifacts are signed, provenance-verified, and reproducible across runs with the same policy and source state.
- LensMap supports bulk repo onboarding, staged rollout, and canary promotion in enterprise environments.
- LensMap can be centrally governed through a control plane with deterministic replay and policy version lineage.
- Compliance reporting can produce evidence bundles mapped to control frameworks with retention and redaction guarantees.
- Reliability behavior is governed by SLO contracts, failure-mode simulations, and alertable risk signals.
- API/schema/extension compatibility is governed with documented migration paths and machine-readable compatibility manifests.
- LensMap emits a periodic maintainability scorecard and publishes trend and threshold breaches for executive and team review.
- Human governance KPIs are tracked in the operations playbook and linked to automated scorecard evidence.
- LensMap can run an automated onboarding/autobot pipeline from extraction through policy-enforced staging, with explicit confidence routing.
- Auto-extracted notes are converted to canonical format with deterministic conflict behavior and human-gated ambiguity handling.
- Validation, regression tests, and a fail-closed security proof all pass after the tranche.
