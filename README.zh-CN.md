# LensMap

[English README](./README.md)

LensMap 是一个与代码绑定的外部文档层。它通过稳定的函数锚点把说明、设计理由、迁移注释和审计记录放到源码外部，同时让源码内部只保留真正有助于阅读的本地注释。

## 当前能力

- 为函数插入确定性的锚点注释：`@lensmap-anchor <HEXID>`
- 默认将新锚点以内联注释形式放在符号所在行，只有在内联不安全时才回退为独立行
- 使用 `<HEXID>-<offset>` 或 `<HEXID>-<start>-<end>` 引用外部注释
- 通过源码锚点、AST 符号路径、指纹和行区间重新定位锚点
- 以符号起始行为基准计算引用偏移，因此内联锚点和独立锚点保持同一套引用语义
- 在大范围重构后，优先使用签名感知的模糊匹配修复锚点，再回退到行号提示
- 在 `validate` / `reanchor` 中加入基于 Git 的脏区重叠与双向编辑冲突保护
- 支持 JavaScript、TypeScript、Python、Rust、Go、Java、C、C++、C#、Kotlin 的 AST 解析
- 支持构建仓库级索引，并通过 CLI 搜索 LensMap 条目
- 内置策略化模板与协作元数据，可记录负责人、作者、审查状态、范围与标签
- 支持面向 CI 的策略检查、仓库汇总与 PR 报告，无需依赖 GitHub API
- `policy check` 默认会聚合仓库中发现的全部 LensMap 文件，并以最严格的联合规则执行治理检查
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
lensmap scan --lensmap=demo/lensmap.json --anchor-mode=smart --anchor-placement=inline
lensmap extract-comments --lensmap=demo/lensmap.json
lensmap annotate --lensmap=demo/lensmap.json --file=demo/src/app.ts --symbol=run --offset=1 --text="Why this exists"
lensmap show --lensmap=demo/lensmap.json --file=demo/src/app.ts
lensmap index --index=demo/.lensmap-index.json
lensmap search --index=demo/.lensmap-index.json --query="Why"
lensmap template list
lensmap annotate --lensmap=demo/lensmap.json --file=demo/src/app.ts --symbol=run --offset=1 --template=architecture --owner=platform --review-status=in_review
lensmap policy init --lensmap=demo/lensmap.json --require-owner=true --require-template=true --require-review-status=true --stale-after-days=30
lensmap policy check --fail-on-warnings
lensmap summary --out=local/state/lensmap/summary.md
lensmap pr report --strict --out=local/state/lensmap/pr-report.md
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

## 规范与 Schema

- Schema：`schema/lensmap.schema.v1.json`
- 当前升级需求规格：`docs/LENSMAP_SRS.md`

## 文件中的注释格式

- Python：`# @lensmap-anchor ...` / `# @lensmap-ref ...`
- JS/TS/Rust/Go/Java/C/C++/C#/Kotlin：`// @lensmap-anchor ...` / `// @lensmap-ref ...`

## VS Code 扩展

`editor/vscode` 中包含一个最小可用的 VS Code 扩展，支持：

- 显示当前文件的 LensMap 注释
- 在光标处添加 LensMap 注释
- 在光标处编辑已有的 LensMap 注释
- Explorer 侧边栏中显示当前文件注释与工作区搜索结果
- 在当前文件行尾显示 LensMap 注释提示
- 弱化显示 `@lensmap-anchor`，降低锚点对阅读的干扰
- 在代码行上显示用于查看/编辑注释的 CodeLens
- `LensMap: Refresh Sidebar`
- `LensMap: Search Workspace Notes`
- `LensMap: Run Policy Check`
- `LensMap: Show Workspace Summary`
- `LensMap: Show PR Report`
- 侧边栏条目可直接编辑
- `@lensmap-anchor` / `@lensmap-ref` 悬停查看
- 在 `local/state/lensmap/vscode/` 下生成并预览治理报告
- 根据 VS Code 界面语言自动切换为英文或中文

打包方式：

```bash
cd editor/vscode
npm install
npm run package:vsix
```

生成的 `.vsix` 文件会输出到 `artifacts/lensmap-vscode-<version>.vsix`。

## JetBrains 插件

`editor/jetbrains` 中包含一个 JetBrains 插件，支持：

- 持久化 `LensMap` 工具窗口，带有结构化注释列表和详情面板
- `LensMap > Show Current File Notes`
- `LensMap > Search Workspace Notes`
- `LensMap > Run Policy Check`
- `LensMap > Show Summary`
- `LensMap > Show PR Report`
- `LensMap > Add Note at Caret`
- `LensMap > Edit Note at Caret`
- 在工具窗口中打开源码位置、打开对应的 LensMap 文件、复制引用或注释内容、编辑所选注释
- 在工具窗口中直接查看治理报告，报告产物写入 `local/state/lensmap/jetbrains/`
- 英文/中文提示与通知

构建方式：

```bash
cd editor/jetbrains
./gradlew buildPlugin
```

生成的插件 ZIP 位于 `editor/jetbrains/build/distributions/`。

## 新增工作流命令

- `template list`：列出内置或仓库自定义模板
- `policy init`：把所有权、模板、审查状态、过期阈值等策略写入 LensMap 元数据
- `policy check`：执行可用于 CI 的策略检查
- `policy check --lensmaps=...`：在需要时只针对指定 LensMap 子集执行聚合检查
- `summary`：输出仓库级 LensMap 汇总，并可写成 Markdown
- `pr report`：根据 git diff 生成变更文件的注释覆盖、过期与未审查报告
