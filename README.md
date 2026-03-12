
# LensMap

[中文文档](./README.zh-CN.md)

<p align="center">
<img width="800" height="450" alt="image" src="./ezgif-753b52dfe5e287da.gif" />
</p>

LensMap is a code-linked documentation layer. It keeps source files lean by moving heavier notes into an external lens map anchored to stable function IDs, while leaving small local comments inline when they genuinely help readability.

## What it does

- Adds deterministic function anchor nodes (`@lensmap-anchor <HEXID>`) with smart anchoring by default.
- Stores comments/docs externally as references (`<HEXID>-<offset>` or `<HEXID>-<start>-<end>`).
- Resolves anchors using source anchor ID first, then AST-backed symbol path and fingerprint metadata, then stored line/span hints.
- Supports AST-backed symbol resolution for JavaScript, TypeScript, Python, Rust, Go, and Java.
- Extracts inline/source comments into lens entries.
- Maintains a readable Markdown sidecar alongside the canonical JSON lensmap.
- Includes a minimal VS Code integration for show/annotate/hover workflows.
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
curl -fsSL https://raw.githubusercontent.com/protheuslabs/Lensmap/main/scripts/install.sh | bash -s -- v0.3.0
```

- Windows (PowerShell):

```powershell
iwr https://raw.githubusercontent.com/protheuslabs/Lensmap/main/scripts/install.ps1 -OutFile install.ps1
./install.ps1 -Version v0.3.0
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
lensmap scan --lensmap=demo/lensmap.json --anchor-mode=smart
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

## Commands

- `init`
- `annotate`
- `template add`
- `scan`
- `extract-comments`
- `unmerge` (alias of `extract-comments`)
- `merge` (hydrate comments back into code from lensmap entries)
- `package` (collect lensmap files into one root bundle directory with a manifest map)
- `unpackage` (restore packaged lensmap files back to original dirs, with `prompt|skip|error` handling for missing dirs)
- `validate`
- `reanchor`
- `render` (writes readable Markdown; defaults to a sibling `.md`)
- `parse` (alias of `render`)
- `show` (filtered readable view by file, symbol, ref, or kind)
- `simplify`
- `polish`
- `import`
- `sync` (reanchor + simplify + render Markdown sidecar)
- `expose`
- `status`

## Marker format by file type

- Python: `# @lensmap-anchor ...` / `# @lensmap-ref ...`
- JS/TS/Rust/Go/Java: `// @lensmap-anchor ...` / `// @lensmap-ref ...`

## Workflow

```bash
# 1. Add anchors only where LensMap actually has work to do.
lensmap scan --lensmap=demo/lensmap.json --anchor-mode=smart

# 2. Pull inline comments out into the lens map.
lensmap extract-comments --lensmap=demo/lensmap.json

# 3. Add more notes without touching raw ref IDs.
lensmap annotate --lensmap=demo/lensmap.json --file=demo/src/app.ts --symbol=run --offset=1 --text="why this exists"

# 4. Inspect one file or symbol.
lensmap show --lensmap=demo/lensmap.json --file=demo/src/app.ts
lensmap show --lensmap=demo/lensmap.json --symbol=run

# 5. Reanchor drift, simplify the JSON, and refresh the Markdown sidecar.
lensmap sync --lensmap=demo/lensmap.json
```

`render` and `sync` default to a Markdown file beside the JSON lensmap, so the machine-readable map stays canonical while the human-readable sidecar stays easy to open.

## Editor integration

There is now a minimal VS Code extension scaffold in `editor/vscode/`.

Current capabilities:

- `LensMap: Show Notes for Current File`
- `LensMap: Add Note at Cursor`
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
