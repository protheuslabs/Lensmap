# Changelog

## 0.3.12
- Added an explicit SRS for the knowledge-boilerplate tranche in `docs/LENSMAP_SRS.md`.
- Added built-in template listing plus structured annotate metadata for owner, author, scope, tags, template, and review fields.
- Added `policy init` and `policy check` for CI-oriented LensMap governance, including stale-note detection and required file-pattern coverage.
- Added `summary` and `pr report` commands for repo rollups, optional Markdown output, and git-diff-based review reporting.
- Extended `sync`, `status`, `render`, `show`, search indexing, and the canonical schema so the JSON, Markdown, and search-index artifact layers stay coherent.

## 0.3.11
- Polished the JetBrains note browser so it retains the selected entry across refreshes instead of snapping back to the first note.
- Added JetBrains tool-window actions to open the backing LensMap file directly and copy the selected note text.
- Updated published version strings and installation docs for the `v0.3.11` release.

## 0.3.10
- Normalized the release version across the Rust CLI, VS Code extension, and JetBrains plugin so artifacts and tags line up again.
- Upgraded the JetBrains tool window from a plain text dump to a structured note browser with a selectable list and detail pane.
- Added JetBrains note-browser actions for opening the selected note in source, copying its LensMap ref, and editing the selected entry in place.
- Updated installation and editor docs to reflect the unified `v0.3.10` release.

## 0.3.9
- Switched new anchors to inline placement by default, with standalone fallback when inline comments are unsafe for the source line.
- Normalized ref math to symbol-relative offsets so inline and standalone anchors preserve the same reference semantics.
- Added git-aware validate/reanchor protection for dirty overlap and dual-edit conflict cases, plus git dirty summary in `status`.
- Expanded the VS Code integration with edit-at-cursor, entry editing, anchor dimming, and inline code lenses.
- Expanded the JetBrains plugin with `Edit Note at Caret` and tool-window editing flow parity.

## 0.3.8
- Upgraded the JetBrains plugin from action-only dialogs to a persistent `LensMap` tool window with refreshable current-file and search output.
- Kept the JetBrains current-file, workspace-search, and caret-annotation actions, but routed their output through the tool window.
- Removed the deprecated JetBrains chooser call from the note-kind prompt.

## 0.3.7
- Added signature-aware fuzzy anchor repair so reanchor/render/search can recover more large refactors before falling back to line hints.
- Added repo-wide `index` and `search` commands plus `.lensmap-index.json` output for searchable note catalogs.
- Expanded the VS Code integration with an Explorer sidebar, workspace search, and inline current-file decorations.
- Added a buildable JetBrains plugin scaffold with actions for current-file notes, workspace search, and caret annotation.

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
