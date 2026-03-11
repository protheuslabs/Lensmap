# Changelog

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
