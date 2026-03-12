
# LensMap

[中文文档](./README.zh-CN.md)

<p align="center">
<img width="800" height="450" alt="image" src="./ezgif-753b52dfe5e287da.gif" />
</p>

LensMap is a code-linked documentation layer. It keeps source files lean by moving heavier notes into an external lens map anchored to stable function IDs, while leaving small local comments inline when they genuinely help readability.

## What it does

- Adds deterministic function anchor nodes (`@lensmap-anchor <HEXID>`) with smart anchoring by default.
- Places new anchors inline on the symbol line by default, with standalone fallback when inline placement is unsafe.
- Stores comments/docs externally as references (`<HEXID>-<offset>` or `<HEXID>-<start>-<end>`).
- Resolves anchors using source anchor ID first, then AST-backed symbol path and fingerprint metadata, then stored line/span hints.
- Keeps refs symbol-relative so inline and standalone anchors resolve the same way.
- Repairs large refactors with signature-aware fuzzy matching before falling back to line hints.
- Adds git-aware validate/reanchor protection for dirty overlap and dual-edit conflict cases.
- Supports AST-backed symbol resolution for JavaScript, TypeScript, Python, Rust, Go, Java, C, C++, C#, and Kotlin.
- Builds a searchable repo-wide note index and supports structured CLI search.
- Adds policy-driven note templates and structured collaboration metadata for owners, review state, scope, and tags.
- Supports CI-oriented policy checks, repo summaries, and PR reports without GitHub API coupling.
- Extracts inline/source comments into lens entries.
- Maintains a readable Markdown sidecar alongside the canonical JSON lensmap.
- Includes VS Code sidebar, decorations, search, show/annotate, and hover workflows.
- Includes a JetBrains plugin with a persistent note browser tool window, plus current-file, workspace-search, jump, copy-ref, and caret annotation/edit flows.
- Supports English and Chinese in the CLI and editor integration.
- Validates marker coherence, collisions, drift, and root-path safety.

## Positioning

LensMap is best for:

- design rationale
- review notes
- migration notes
- audit and operational notes
- generated explanations

Keep inline comments for:

- short local intent that helps while reading the file
- language directives and preserve comments
- comments that are clearer directly beside the code than in an external map

## Install

### Prebuilt binary

- Linux/macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/protheuslabs/Lensmap/main/scripts/install.sh | bash -s -- v0.3.11
```

- Windows (PowerShell):

```powershell
iwr https://raw.githubusercontent.com/protheuslabs/Lensmap/main/scripts/install.ps1 -OutFile install.ps1
./install.ps1 -Version v0.3.11
```

### From source

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

## Quick start

```bash
lensmap init demo --mode=group --covers=demo/src
lensmap scan --lensmap=demo/lensmap.json --anchor-mode=smart --anchor-placement=inline
lensmap extract-comments --lensmap=demo/lensmap.json
lensmap annotate --lensmap=demo/lensmap.json --file=demo/src/app.ts --symbol=run --offset=1 --text="Why this exists"
lensmap show --lensmap=demo/lensmap.json --file=demo/src/app.ts
lensmap sync --lensmap=demo/lensmap.json
lensmap merge --lensmap=demo/lensmap.json
lensmap unmerge --lensmap=demo/lensmap.json
lensmap package --bundle-dir=.lenspack
lensmap unpackage --bundle-dir=.lenspack --on-missing=prompt
lensmap validate --lensmap=demo/lensmap.json
```

### Add annotation by symbol

```bash
lensmap annotate \
  --lensmap=demo/lensmap.json \
  --file=demo/src/app.ts \
  --symbol=run \
  --symbol-path=App.run \
  --offset=1 \
  --text="Reason for this branch" \
  --kind=comment
```

### Add annotation by raw ref

```bash
lensmap annotate \
  --lensmap=demo/lensmap.json \
  --ref=ABC123-2 \
  --text="Reason for this branch" \
  --kind=comment
```

## Common mistake

Do not run commands with literal placeholder paths like `path/to/lensmap.json`.

Use a real file path, for example:

```bash
lensmap validate --lensmap=demo/lensmap.json
```

## Schema

- Canonical schema: `schema/lensmap.schema.v1.json`
- Type: `lensmap`
- Version: `1.0.0`
- SRS for the current upgrade tranche: `docs/LENSMAP_SRS.md`

## Commands

- `init`
- `annotate`
- `template add`
- `template list`
- `scan` (`--anchor-placement=inline|standalone`)
- `extract-comments`
- `unmerge` (alias of `extract-comments`)
- `merge` (hydrate comments back into code from lensmap entries)
- `package` (collect lensmap files into one root bundle directory with a manifest map)
- `unpackage` (restore packaged lensmap files back to original dirs, with `prompt|skip|error` handling for missing dirs)
- `validate`
- `policy init` (store repo policy such as required owners/templates/review status and stale thresholds)
- `policy check` (CI-friendly validation against LensMap policy)
- `reanchor` (git-aware dirty-overlap protection)
- `render` (writes readable Markdown; supports filtering by owner/template/review/scope/tag)
- `parse` (alias of `render`)
- `show` (filtered readable view by file, symbol, ref, kind, owner, template, review, scope, or tag)
- `simplify`
- `index` (build a repo-wide `.lensmap-index.json`)
- `search` (search repo notes live or through a saved index, including owner/template/review/scope/tag filters)
- `summary` (repo-aware note rollups in JSON and optional Markdown)
- `pr report` (git diff oriented report for changed files, stale notes, and missing-note coverage)
- `polish`
- `import`
- `sync` (reanchor + simplify + refresh canonical JSON, Markdown sidecar, and search index)
- `expose`
- `status`

## Marker format by file type

- Python: `# @lensmap-anchor ...` / `# @lensmap-ref ...`
- JS/TS/Rust/Go/Java/C/C++/C#/Kotlin: `// @lensmap-anchor ...` / `// @lensmap-ref ...`

## Workflow

```bash
# 1. Add anchors only where LensMap actually has work to do.
lensmap scan --lensmap=demo/lensmap.json --anchor-mode=smart

# 2. Pull inline comments out into the lens map.
lensmap extract-comments --lensmap=demo/lensmap.json

# 3. Add more notes without touching raw ref IDs.
lensmap annotate --lensmap=demo/lensmap.json --file=demo/src/app.ts --symbol=run --offset=1 --text="why this exists"

# 3b. Or use a structured template with policy metadata.
lensmap annotate --lensmap=demo/lensmap.json --file=demo/src/app.ts --symbol=run --offset=1 --template=architecture --owner=platform --review-status=in_review

# 3c. Initialize repo policy for CI.
lensmap policy init --lensmap=demo/lensmap.json --require-owner=true --require-template=true --require-review-status=true --stale-after-days=30

# 4. Inspect one file or symbol.
lensmap show --lensmap=demo/lensmap.json --file=demo/src/app.ts
lensmap show --lensmap=demo/lensmap.json --symbol=run

# 5. Reanchor drift, simplify the JSON, and refresh the Markdown sidecar + search index.
lensmap sync --lensmap=demo/lensmap.json

# 6. Summarize or report note coverage for CI/review.
lensmap summary --lensmaps=demo/lensmap.json --owner=platform --out=demo/lensmap-summary.md
lensmap pr report --lensmaps=demo/lensmap.json --base=origin/main --head=HEAD --strict
```

`render` and `sync` default to a Markdown file beside the JSON lensmap, while `sync` also refreshes the repo search index. The canonical JSON stays authoritative, the Markdown sidecar stays human-readable, and the search index stays editor/repo-query friendly.

## Editor integration

There is now a minimal VS Code extension scaffold in `editor/vscode/`.

Current capabilities:

- `LensMap: Show Notes for Current File`
- `LensMap: Add Note at Cursor`
- `LensMap: Edit Note at Cursor`
- Explorer sidebar with current-file notes and workspace search results
- Inline end-of-line note decorations for current-file entries
- Anchor comment dimming so anchors stay low-noise while editing
- Inline code lenses for showing and editing notes on the current line
- `LensMap: Refresh Sidebar`
- `LensMap: Search Workspace Notes`
- sidebar entry edit action
- hover support for `@lensmap-anchor` and `@lensmap-ref`
- follows the VS Code UI language for English/Chinese prompts and messages

The extension auto-detects a local LensMap repo and uses `cargo run -q -p lensmap -- ...` during development. Outside the repo, point it at an installed binary with the VS Code setting `lensmap.command`.

### Package the extension

```bash
cd editor/vscode
npm install
npm run package:vsix
```

That writes a `.vsix` bundle to `artifacts/lensmap-vscode-<version>.vsix`.

## JetBrains plugin

There is now a JetBrains plugin in `editor/jetbrains/` with a persistent note browser tool window.

Current capabilities:

- Persistent `LensMap` note browser for current-file or search output
- `LensMap > Show Current File Notes`
- `LensMap > Search Workspace Notes`
- `LensMap > Add Note at Caret`
- `LensMap > Edit Note at Caret`
- Open the selected note in source, open the backing LensMap file, copy its ref or note text, and edit the selected entry in place
- English/Chinese prompts and notifications

Build the plugin:

```bash
cd editor/jetbrains
./gradlew buildPlugin
```

The packaged plugin ZIP is written to `editor/jetbrains/build/distributions/`.

## Packaging workflow

```bash
# Package lensmap files to one root dir
lensmap package --bundle-dir=.lenspack

# Restore to original locations. If a dir is missing, prompt for new dir or skip.
lensmap unpackage --bundle-dir=.lenspack --on-missing=prompt

# Non-interactive option:
lensmap unpackage --bundle-dir=.lenspack --on-missing=skip

# Provide remap(s) for moved directories:
lensmap unpackage --bundle-dir=.lenspack --map=apps/old=apps/new,docs/legacy=docs/archive
```

When `unpackage` skips a file, it remains in `.lenspack/files/` and processing continues to the next file.

## License

MIT
