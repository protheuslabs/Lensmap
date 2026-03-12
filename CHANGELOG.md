# Changelog

## 0.3.6
- Added AST-backed symbol/span detection for C, C++, C#, and Kotlin.
- Extended the VS Code integration to C/C++/C#/Kotlin files and bumped the packaged extension to `0.2.1`.
- Preserved English/Chinese localization across the CLI, Markdown renderers, and VS Code workflows while expanding language coverage.

## 0.3.5
- Added AST-backed symbol/span detection for Go and Java, keeping regex fallback for degraded parses.
- Added English/Chinese localization support to the CLI (`--lang=en|zh-CN` or `LENSMAP_LANG`), including localized help, prompts, and messages.
- Localized the VS Code integration and extended it to Go and Java files.
- Added a packageable VS Code extension workflow that produces a `.vsix` bundle from `editor/vscode`.

## 0.3.4
- Added AST-backed symbol/span detection for JavaScript, TypeScript, Python, and Rust via tree-sitter, with regex fallback for unsupported or degraded parses.
- Extended anchor records with symbol path and span metadata so resolution is less brittle when code moves.
- Added a minimal VS Code integration in `editor/vscode` with current-file show, cursor annotation, and hover support for anchors/refs.

## 0.3.3
- Repositioned LensMap as a code-linked external documentation layer instead of a total inline-comment replacement.
- Added smart scan mode (`--anchor-mode=smart`) so anchors are inserted only where LensMap already has work to do.
- Hardened anchor resolution so render/validate/reanchor prefer source anchor ID, then symbol + fingerprint, then stored line hints.
- Extended `annotate` to support `--file + --symbol + --offset`, including on-demand anchor creation for uncommented symbols.
- Added `show` for filtered Markdown views by file, symbol, ref, or kind.
- Replaced the placeholder `sync` command with a real workflow: reanchor, simplify, and refresh a readable Markdown sidecar.
- Changed `render` defaults to write the Markdown sidecar beside the canonical JSON lensmap.

## 0.3.2
- Added `package` command to bundle LensMap files into a single root directory with a manifest map.
- Added `unpackage` command to restore bundled files, including missing-directory handling:
  - `--on-missing=prompt` (enter new dir or skip)
  - `--on-missing=skip`
  - `--on-missing=error`
- Added directory remapping support for unpack (`--map=old_dir=new_dir`).

## 0.3.1
- Added `annotate` command for manual LensMap entries by `ref`.
- Added `merge` command to hydrate LensMap comment entries back into source files.
- Added `unmerge` alias for `extract-comments` to pull comments back out of code.
- Added `parse` alias for `render`.
- Improved missing path diagnostics (detects placeholder `path/to/...` usage with actionable hints).
- Fixed `render` stats to report actual rendered file count.
- Hardened de-duplication so repeated extract/unmerge cycles avoid comment-collision duplicates by `[file + ref]`.

## 0.3.0
- Migrated LensMap CLI runtime to Rust.
- Added canonical LensMap v1 schema (`schema/lensmap.schema.v1.json`).
- Added anchor/reference extraction and validation commands.
- Added release workflow and cross-platform installers.
