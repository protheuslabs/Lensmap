# LensMap VS Code Extension

Minimal VS Code integration for LensMap.

## Features

- Show LensMap notes for the current file
- Explorer sidebar with current-file notes and workspace search results
- Add a LensMap note at the cursor
- Refresh the sidebar and search LensMap notes across the workspace
- Inline end-of-line decorations for current-file notes
- Hover on `@lensmap-anchor` and `@lensmap-ref`
- Follows the VS Code UI language and supports English and Chinese
- Supports JavaScript, TypeScript, Python, Rust, Go, Java, C, C++, C#, and Kotlin files

## Packaging

```bash
cd editor/vscode
npm install
npm run package:vsix
```

The packaged extension is written to `artifacts/lensmap-vscode-<version>.vsix`.

## Notes

- In the LensMap repo, the extension auto-detects `cargo run -q -p lensmap -- ...`
- Outside the repo, set `lensmap.command` to a packaged or installed `lensmap` binary
- Use `lensmap.extraArgs` for flags like `--lang=zh-CN`

## 中文

这是一个最小可用的 LensMap VS Code 扩展。

- 显示当前文件的 LensMap 注释
- 在光标位置添加 LensMap 注释
- 支持 `@lensmap-anchor` 和 `@lensmap-ref` 的悬停查看
- 自动跟随 VS Code 界面语言，支持英文和中文
- 支持 JavaScript、TypeScript、Python、Rust、Go、Java、C、C++、C#、Kotlin 文件
