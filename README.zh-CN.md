# LensMap

[English README](./README.md)

LensMap 是一个与代码绑定的外部文档层。它通过稳定的函数锚点把说明、设计理由、迁移注释和审计记录放到源码外部，同时让源码内部只保留真正有助于阅读的本地注释。

## 当前能力

- 为函数插入确定性的锚点注释：`@lensmap-anchor <HEXID>`
- 使用 `<HEXID>-<offset>` 或 `<HEXID>-<start>-<end>` 引用外部注释
- 通过源码锚点、AST 符号路径、指纹和行区间重新定位锚点
- 在大范围重构后，优先使用签名感知的模糊匹配修复锚点，再回退到行号提示
- 支持 JavaScript、TypeScript、Python、Rust、Go、Java、C、C++、C#、Kotlin 的 AST 解析
- 支持构建仓库级索引，并通过 CLI 搜索 LensMap 条目
- 将源码中的注释提取到 LensMap 文件
- 将 LensMap 条目重新合并回源码
- 生成便于阅读的 Markdown 侧边文档
- 支持把 LensMap 文件打包到根目录并恢复回原始目录
- CLI、VS Code 扩展和 JetBrains 插件均支持英文与中文

## 适用场景

适合放到 LensMap 中的内容：

- 设计理由
- 评审说明
- 迁移记录
- 运维与审计注释
- 生成式解释

仍然适合保留在源码里的内容：

- 紧贴代码才更清晰的短注释
- 语言指令和保留注释
- 只服务于局部阅读的意图说明

## 快速开始

```bash
lensmap init demo --mode=group --covers=demo/src
lensmap scan --lensmap=demo/lensmap.json --anchor-mode=smart
lensmap extract-comments --lensmap=demo/lensmap.json
lensmap annotate --lensmap=demo/lensmap.json --file=demo/src/app.ts --symbol=run --offset=1 --text="Why this exists"
lensmap show --lensmap=demo/lensmap.json --file=demo/src/app.ts
lensmap index --index=demo/.lensmap-index.json
lensmap search --index=demo/.lensmap-index.json --query="Why"
lensmap sync --lensmap=demo/lensmap.json
lensmap merge --lensmap=demo/lensmap.json
lensmap unmerge --lensmap=demo/lensmap.json
lensmap package --bundle-dir=.lenspack
lensmap unpackage --bundle-dir=.lenspack --on-missing=prompt
lensmap validate --lensmap=demo/lensmap.json
```

## 语言支持

可以通过参数或环境变量切换 CLI 语言：

```bash
lensmap --help --lang=zh-CN
LENSMAP_LANG=en lensmap validate --lensmap=demo/lensmap.json
```

## 文件中的注释格式

- Python：`# @lensmap-anchor ...` / `# @lensmap-ref ...`
- JS/TS/Rust/Go/Java/C/C++/C#/Kotlin：`// @lensmap-anchor ...` / `// @lensmap-ref ...`

## VS Code 扩展

`editor/vscode` 中包含一个最小可用的 VS Code 扩展，支持：

- 显示当前文件的 LensMap 注释
- 在光标处添加 LensMap 注释
- Explorer 侧边栏中显示当前文件注释与工作区搜索结果
- 在当前文件行尾显示 LensMap 注释提示
- `LensMap: Refresh Sidebar`
- `LensMap: Search Workspace Notes`
- `@lensmap-anchor` / `@lensmap-ref` 悬停查看
- 根据 VS Code 界面语言自动切换为英文或中文

打包方式：

```bash
cd editor/vscode
npm install
npm run package:vsix
```

生成的 `.vsix` 文件会输出到 `artifacts/lensmap-vscode-<version>.vsix`。

## JetBrains 插件

`editor/jetbrains` 中包含一个最小可用的 JetBrains 插件，支持：

- 持久化 `LensMap` 工具窗口，用于展示当前文件或搜索结果
- `LensMap > Show Current File Notes`
- `LensMap > Search Workspace Notes`
- `LensMap > Add Note at Caret`
- 英文/中文提示与通知

构建方式：

```bash
cd editor/jetbrains
./gradlew buildPlugin
```

生成的插件 ZIP 位于 `editor/jetbrains/build/distributions/`。
