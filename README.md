# LensMap

[中文文档](./README.zh-CN.md)

LensMap is an internal engineering tool for code-linked documentation, annotation governance, and reviewable knowledge capture.

It keeps source files readable by moving heavy commentary out of code while preserving deterministic anchors back to the exact symbols and spans engineers care about.

## Purpose

LensMap exists to reduce knowledge boilerplate around software maintenance.

Use it when inline comments stop scaling and you need:

- stable references for architecture, review, migration, and audit notes
- externalized documentation without losing code locality
- repo-wide search and reporting for code-linked notes
- CI-enforceable ownership, review, and note hygiene policy
- packaging and unpackaging of documentation outside normal source delivery

LensMap is not a code generator and it is not intended to replace every inline comment. Keep short local intent inline. Use LensMap for heavier, longer-lived, or operationally important context.

## Operating Model

LensMap maintains three artifact layers:

- Canonical JSON: machine authority for anchors, refs, metadata, and policy
- Markdown sidecar: human-readable operational view of the same map
- Search index: repo-wide query surface for editor and reporting workflows

Anchors are attached to code using deterministic IDs:

- `@lensmap-anchor <HEXID>` for symbol anchors
- `<HEXID>-<offset>` or `<HEXID>-<start>-<end>` for note references

Resolution order is designed to survive normal refactors:

1. source anchor ID
2. AST-backed symbol path and fingerprint
3. span metadata
4. line hints and fuzzy repair fallback

## Core Capabilities

- Smart anchor placement with inline-first insertion and safe standalone fallback
- AST-backed symbol detection for JavaScript, TypeScript, Python, Rust, Go, Java, C, C++, C#, and Kotlin
- External note capture for comments, docs, TODOs, and decisions
- Structured note metadata: title, owner, author, scope, template, review status, review due date, tags
- Built-in templates for architecture, migration, audit, review, decision, and TODO notes
- Policy initialization and CI-oriented policy checks
- Repo summaries and PR-oriented reporting from git diff without GitHub API dependence
- Merge and unmerge workflows for round-tripping notes into source when needed
- Packaging and unpackaging for post-production documentation handling
- Production stripping for marker-free source delivery (`strip` and `package --strip-sources`)
- VS Code and JetBrains integration for browsing and editing notes in-editor
- English and Chinese CLI and editor support
- Fail-closed root-path safety for generated artifacts and package operations

## Canonical implementation stack (target)

LensMap's implementation target is an enterprise-ready, deterministic Rust control plane with a modular architecture.

- Runtime: Rust 2021
- Parser layer: `tree-sitter` with deterministic symbol-based anchoring
- Policy and validation: deterministic rule engine with strict/warn/fail semantics
- Evidence plane: receipt artifacts, hash-chain-friendly run records, and signed/attestable release paths
- Packaging surface: stable strip/package/restore pipeline with replay-safe manifests
- Integration: schema-stable connectors/events for CI and enterprise tooling

Target stack and implementation sequencing is documented in:
[LENSMAP_TECH_STACK.md](/Users/jay/.openclaw/workspace/apps/lensmap/docs/LENSMAP_TECH_STACK.md)

## Recommended Use

Use LensMap for:

- architecture rationale
- migration plans and rollback notes
- review rationale and follow-up decisions
- audit and operational controls
- generated explanations that should not clutter source

Keep inline comments for:

- local intent that improves immediate readability
- language directives and preservation comments
- very short comments that are clearer directly beside the line they describe

## Installation

### Prebuilt binary

Linux and macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/protheuslabs/Lensmap/main/scripts/install.sh | bash -s -- v0.3.12
```

Windows PowerShell:

```powershell
iwr https://raw.githubusercontent.com/protheuslabs/Lensmap/main/scripts/install.ps1 -OutFile install.ps1
./install.ps1 -Version v0.3.12
```

To verify release integrity, use the published checksum file. Enable verification with:

```bash
curl -fsSL https://raw.githubusercontent.com/protheuslabs/Lensmap/main/scripts/install.sh | VERIFY_CHECKSUMS=1 bash -s -- v0.3.12
./install.ps1 -Version v0.3.12 -VerifyChecksums
```

### Build from source

```bash
git clone https://github.com/protheuslabs/Lensmap.git
cd Lensmap
cargo build --release
./target/release/lensmap --help
```

Force the CLI language if needed:

```bash
./target/release/lensmap --help --lang=zh-CN
LENSMAP_LANG=en ./target/release/lensmap validate --lensmap=demo/lensmap.json
```

## Quick Start

Initialize a project map and scan for anchors:

```bash
lensmap init demo --mode=group --covers=demo/src
lensmap scan --lensmap=demo/lensmap.json --anchor-mode=smart --anchor-placement=inline
```

Extract existing source comments into the map:

```bash
lensmap extract-comments --lensmap=demo/lensmap.json
```

Add a structured note against a symbol:

```bash
lensmap annotate \
  --lensmap=demo/lensmap.json \
  --file=demo/src/app.ts \
  --symbol=run \
  --symbol-path=App.run \
  --offset=1 \
  --template=architecture \
  --owner=platform \
  --review-status=in_review
```

Inspect the map for a file or symbol:

```bash
lensmap show --lensmap=demo/lensmap.json --file=demo/src/app.ts
lensmap show --lensmap=demo/lensmap.json --symbol=App.run
```

Refresh all managed artifacts:

```bash
lensmap sync --lensmap=demo/lensmap.json
```

## Governance Workflow

Initialize LensMap policy inside the canonical JSON:

```bash
lensmap policy init \
  --lensmap=demo/lensmap.json \
  --require-owner=true \
  --require-template=true \
  --require-review-status=true \
  --stale-after-days=30 \
  --required-patterns='demo/src/*.rs,demo/src/*.ts' \
  --production-strip-anchors=true \
  --production-strip-refs=true \
  --production-strip-on-package=true \
  --production-exclude-patterns='**/tests/**,**/generated/**'
```

Run CI-facing policy checks. By default, LensMap now discovers and aggregates every map under the repository root:

```bash
lensmap policy check --fail-on-warnings
```

Generate repo summaries and PR reports:

```bash
lensmap summary --out=local/state/lensmap/summary.md
lensmap pr report --strict --out=local/state/lensmap/pr-report.md
```

Production enforcement can be enabled explicitly:

```bash
lensmap policy check --production --fail-on-warnings
```

Narrow the governance scope only when required:

```bash
lensmap policy check --lensmaps=demo/api/lensmap.json,demo/ui/lensmap.json --report-only
```

## Common Operator Workflows

### Release Hardening Gates

```bash
lensmap command parity --docs=README.md,README.zh-CN.md
lensmap release manifest --version=v0.3.12 --strict=1
lensmap release preflight --strict=1 --smoke=1
lensmap public-health --strict=1 --check-remote=0
```

### Command Catalog Additions

```bash
lensmap template add <type>
lensmap package evidence --bundle-dir=.lenspack
lensmap restore --bundle-dir=.lenspack
lensmap verify --bundle-dir=.lenspack
lensmap reanchor --lensmap=demo/lensmap.json
lensmap render --lensmap=demo/lensmap.json --file=demo/src/app.ts
lensmap parse --lensmap=demo/lensmap.json
lensmap simplify --lensmap=demo/lensmap.json
lensmap metrics --lensmaps=demo/lensmap.json --period=run
lensmap scorecard --lensmaps=demo/lensmap.json --period=run
lensmap polish
lensmap import --from=path/to/lensmap.json
lensmap expose --name=default
lensmap status --lensmap=demo/lensmap.json
```

### Add a note by raw reference

```bash
lensmap annotate \
  --lensmap=demo/lensmap.json \
  --ref=ABC123-2 \
  --text="Reason for this branch" \
  --kind=comment
```

### Merge notes back into source

```bash
lensmap merge --lensmap=demo/lensmap.json
```

### Pull them back out again

```bash
lensmap unmerge --lensmap=demo/lensmap.json
lensmap unmerge --lensmap=demo/lensmap.json --strip --clean-anchors=true --clean-refs=true
```

### Package documentation to a root bundle

```bash
lensmap package --bundle-dir=.lenspack
lensmap unpackage --bundle-dir=.lenspack --on-missing=prompt
```

### Produce marker-free production sources

```bash
lensmap strip --source=demo/src --out-dir=demo/dist/prod --clean-anchors=true --clean-refs=true
```

### Fail build if production markers remain

```bash
lensmap strip --source=. --check --clean-anchors=true --clean-refs=true
```

### Package docs + stripped production sources together

```bash
lensmap package --bundle-dir=.lenspack --strip-sources --out-format=tar.gz
lensmap package --bundle-dir=.lenspack --production --out-format=tar.gz
```

## Command Surface

| Area | Commands |
| --- | --- |
| Initialization | `init`, `scan`, `reanchor`, `sync`, `status` |
| Notes | `annotate`, `extract-comments`, `merge`, `unmerge`, `simplify` |
| Templates and policy | `template add`, `template list`, `policy init`, `policy check` (aggregates all discovered LensMaps by default) |
| Reading and reporting | `render`, `parse`, `show`, `index`, `search`, `summary`, `pr report` |
| Packaging and utility | `package`, `package evidence`, `strip`, `verify`, `restore`, `unpackage`, `polish`, `import`, `expose` |

Run `lensmap --help` for the full command signature set.

## Schema and Specification

- Canonical schema: `schema/lensmap.schema.v1.json`
- Current document version: `1.0.0`
- Requirements specification for the current tranche: `docs/LENSMAP_SRS.md`

## Marker Format by File Type

- Python: `# @lensmap-anchor ...` and `# @lensmap-ref ...`
- JS/TS/Rust/Go/Java/C/C++/C#/Kotlin: `// @lensmap-anchor ...` and `// @lensmap-ref ...`

## Build and Release Hooks

### Cargo / Rust

```bash
cargo run --bin lensmap -- strip --source=. --check --clean-anchors=true --clean-refs=true
cargo run --bin lensmap -- package --bundle-dir=.lenspack --production --out-format=tar.gz
```

### npm / JS-TS

```json
{
  "scripts": {
    "lensmap:strip": "lensmap strip --source=src --out-dir=dist/prod",
    "lensmap:check": "lensmap strip --source=. --check --clean-anchors=true --clean-refs=true"
  }
}
```

### Gradle / Maven pre-release hook

Run `lensmap strip --source=src --out-dir=build/lensmap-prod` before archive tasks.

### GitHub Actions

```yaml
- name: Policy and packaging gate
  run: cargo run --locked -- policy check --fail-on-warnings
- name: LensMap strip gate
  run: cargo run --locked -- strip --source=. --check --clean-anchors=true --clean-refs=true
- name: LensMap production package
  run: cargo run --locked -- package --bundle-dir=.lenspack --production --out-format=tar.gz
```

### Release artifact hardening

- Release workflows now publish SHA-256 checksum files and deterministic build manifests for each platform target.
- Release assets are attestation-ready through GitHub artifact attestation (`actions/attest-build-provenance`).
- The release manifest can be used for reproducibility checks:
  - artifact hash
  - source commit
  - target platform
  - build format

## Editor Integration

### VS Code

The VS Code extension lives in `editor/vscode` and provides:

- current-file note browsing
- add and edit note at cursor
- workspace note search
- workspace governance actions for `policy check`, `summary`, and `pr report`
- sidebar views
- inline decorations and code lenses
- hover support for LensMap anchors and refs
- Markdown preview of generated governance reports under `local/state/lensmap/vscode/`
- English and Chinese prompts and messages

Package it with:

```bash
cd editor/vscode
npm install
npm run package:vsix
```

Artifact output:

- `artifacts/lensmap-vscode-<version>.vsix`

### JetBrains

The JetBrains plugin lives in `editor/jetbrains` and provides:

- persistent LensMap tool window
- current-file note browsing
- workspace search
- add and edit note at caret
- workspace governance actions for `policy check`, `summary`, and `pr report`
- open-in-source and open-map actions
- copy-ref and copy-note-text actions
- in-tool-window rendering for governance reports, with artifacts stored under `local/state/lensmap/jetbrains/`
- English and Chinese prompts and notifications

Build it with:

```bash
cd editor/jetbrains
./gradlew buildPlugin
```

Artifact output:

- `editor/jetbrains/build/distributions/lensmap-jetbrains-<version>.zip`

## Safety Model

LensMap is intended to fail closed in operationally sensitive cases.

Current protections include:

- blocking generated outputs outside the repository root
- blocking package bundle directories outside the repository root
- validating marker coherence by file type
- detecting comment collisions and unresolved refs
- git-aware protection for dirty overlap and dual-edit conflict cases during reanchor flows

## Operational Notes

- Do not use literal placeholder paths such as `path/to/lensmap.json`.
- Prefer `sync` after meaningful note churn so JSON, Markdown, and search index stay aligned.
- Prefer `policy check` in CI when using LensMap as a governed documentation surface.
- Prefer templates for repeated note classes so ownership and review metadata stay coherent.

## Support

LensMap is maintained as an internal engineering tool. For feature work, schema changes, or editor workflow changes, start with the SRS and keep the canonical JSON contract stable unless migration is explicit.
