# LensMap
<img width="200" height="280" alt="lensmap" src="https://github.com/user-attachments/assets/6e25c5ed-3c78-4040-b0da-b96f6e14b255" />

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
lensmap validate --lensmap=demo/lensmap.json
lensmap render --lensmap=demo/lensmap.json --out=demo/render.md
```

## Schema

- Canonical schema: `schema/lensmap.schema.v1.json`
- Type: `lensmap`
- Version: `1.0.0`

## Commands

- `init`
- `template add`
- `scan`
- `extract-comments`
- `validate`
- `reanchor`
- `render`
- `simplify`
- `polish`
- `import`
- `sync`
- `expose`
- `status`

## Marker format by file type

- Python: `# @lensmap-anchor ...` / `# @lensmap-ref ...`
- JS/TS/Rust: `// @lensmap-anchor ...` / `// @lensmap-ref ...`

## License

MIT
