# LensMap Tech Stack and Implementation Plan

## Canonical Stack (Target)

LensMap's execution goal is unchanged: deterministic symbol-linked knowledge governance for very large estates.
The stack below is the target architecture for scaling safely and maintainably.

- Core runtime: Rust 2021.
- CLI framework: `clap` command surface (`lensmap` subcommands and strict mode semantics).
- Parsing: `tree-sitter` parser set (Rust/JS/TS/Python/Go/Java/C/C++/C#/Kotlin) with deterministic symbol fingerprinting.
- Serialization: `serde` + canonical JSON (`serde_json`) for receipts, policy artifacts, reports, and manifests.
- Storage: repo-local `.lensmap` artifacts plus immutable run/receipt ledger (`jsonl`) for replay.
- Scheduling: deterministic bounded parallel scanning and checkpointed resumability.
- Integrity and trust: hashing (`sha2`/`blake3`) plus optional signatures for release evidence and policy-sensitive operations.
- Integration: stable event/report schema for CI, editor plugins, and automation surfaces.
- Observability: structured logging and deterministic exit semantics with explicit `pass|warn|fail` outputs.
- Distribution: `cargo-dist` + reproducible build manifests/checksums and attestation-ready release pipeline.

## Why this stack is the implementation target

- Determinism and replay: same inputs produce byte-stable command outputs and policy decisions.
- Maintainability at scale: split current monolith into bounded modules while preserving current behavior.
- Enterprise controls: stronger trust boundaries for audit and release workflows without changing operator ergonomics.
- Operational leverage: clearer ownership of ingestion, policy, indexing, reporting, strip/package, and evidence surfaces.

## Planned architecture decomposition

- `crates/lensmap-cli/src/args.rs`
  - clap command surface and shared parse context.
- `crates/lensmap-cli/src/domain.rs`
  - shared data models (`AnchorRecord`, `EntryRecord`, `Note`, `Policy`, `Receipt`, report envelopes).
- `crates/lensmap-cli/src/policy/`
  - policy load/merge/precedence + deterministic violation classification + CI exit semantics.
- `crates/lensmap-cli/src/parser/`
  - parser adapters by language and anchor/ref resolution routines.
- `crates/lensmap-cli/src/index/`
  - scan/inventory, search index serialization, changed-file filtering, and snapshotting.
- `crates/lensmap-cli/src/gov/`
  - summary, PR report, atlas, metrics, scorecard, human-op guidance.
- `crates/lensmap-cli/src/pack/`
  - strip, package, unpackage, evidence packaging, verify, and restore.
- `crates/lensmap-cli/src/audit/`
  - run receipts, ledger entries, integrity hash chain, and redaction policy hooks.
- `crates/lensmap-cli/src/connect/` (Phase 3)
  - optional connectors/adapters (GitHub/Jira/etc) behind schema locks.

## Implementation phases (execution order)

### Phase 1 — Deterministic core split (P0)
1. Split CLI argument parsing from business handlers.
2. Introduce stable module boundaries for core models and policy.
3. Define canonical command envelopes for all machine outputs.

Outcome:
- deterministic behavior preserved despite file modularization.
- no change in external command contracts.

### Phase 2 — State and evidence hardening (P0/P1)
1. Introduce append-only run registry and signed evidence hashes.
2. Normalize policy result envelopes across `policy check`, `pr report`, `summary`, and `strip`.
3. Add replay verification command for historical command runs.

Outcome:
- reproducible audit artifacts for enterprise evidence and release workflows.

### Phase 3 — Scale control and resilience (P1)
1. Add bounded concurrency controls for scan/anchor/report operations.
2. Add resumable checkpoints for long scans and package workflows.
3. Add failure-mode handling for interrupted runs and corrupted intermediates.

Outcome:
- predictable behavior in 1M+ LOC and beyond without silent partial writes.

### Phase 4 — Enterprise integration (P2)
1. Add connector/ingestion adapters for external systems and CI pipelines.
2. Add policy federation surface (fleet package/reference + local cache/provenance).
3. Add SLO and health outputs for command-level reliability thresholds.

Outcome:
- repeatable enterprise operation without changing LensMap mission boundaries.

## Implementation policy

- One-command run must produce deterministic outputs for identical snapshots and policy contexts.
- Changes in implementation must be feature-gated behind command flags until validated in strict mode.
- Audit and release-facing artifacts take precedence over UX convenience when conflicts occur.
- Every change that affects `policy`, `strip`, `package`, `summary`, or `pr report` must be receipt-backed.

## Mapping to backlog and SRS

- This stack implements existing SRS and backlog direction and maps directly to high-priority work in:
  - `docs/LENSMAP_BACKLOG.md`
  - `docs/LENSMAP_SRS.md`
- This document is normative for planned module boundaries and ordering when implementation work is scheduled.
