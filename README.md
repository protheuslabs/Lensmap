
# LensMap

<p align="center">
<img width="800" height="450" alt="image" src="./ezgif-753b52dfe5e287da.gif" />
</p>


LensMap keeps source files lean by moving comments/docs into an external lens map anchored to stable function IDs.

## What it does

- Adds deterministic function anchor nodes (`@lensmap-anchor <HEXID>`).
- Stores comments/docs externally as references (`<HEXID>-<offset>` or `<HEXID>-<start>-<end>`).
- Extracts inline/source comments into lens entries.
- Validates marker coherence, collisions, drift, and root-path safety.

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

## Quick start

```bash
lensmap init demo --mode=group --covers=demo/src
lensmap scan --lensmap=demo/lensmap.json
lensmap extract-comments --lensmap=demo/lensmap.json
lensmap merge --lensmap=demo/lensmap.json
lensmap unmerge --lensmap=demo/lensmap.json
lensmap package --bundle-dir=.lenspack
lensmap unpackage --bundle-dir=.lenspack --on-missing=prompt
lensmap validate --lensmap=demo/lensmap.json
lensmap render --lensmap=demo/lensmap.json --out=demo/render.md
```

### Add manual annotation

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
- `render`
- `parse` (alias of `render`)
- `simplify`
- `polish`
- `import`
- `sync`
- `expose`
- `status`

## Marker format by file type

- Python: `# @lensmap-anchor ...` / `# @lensmap-ref ...`
- JS/TS/Rust: `// @lensmap-anchor ...` / `// @lensmap-ref ...`

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
