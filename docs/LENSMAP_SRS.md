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

## Non-Functional Requirements

### LM-SRS-009 Fail-Closed Safety

- Any path written by LensMap shall remain inside the repository root.
- Packaging and policy-report output shall fail closed when asked to write outside the repo root.

### LM-SRS-010 Backward Compatibility

- Existing lensmap files without the new collaboration metadata shall remain readable.
- Existing commands shall continue to work unless explicitly replaced by a stricter equivalent.

### LM-SRS-011 Auditability

- Every new command added in this tranche shall emit JSON receipts suitable for CI and audit use.

## Acceptance Criteria

- A repo can define strict LensMap policy and check it from CI.
- A user can add a structured note from a built-in template without manually retyping the same metadata.
- A user can generate a repo summary grouped by owner or directory.
- A user can generate a PR report from git diff.
- `sync` refreshes both the Markdown sidecar and search index.
- `status` shows the canonical JSON path, Markdown sidecar path, and search index path.
- Validation, regression tests, and a fail-closed security proof all pass after the tranche.
