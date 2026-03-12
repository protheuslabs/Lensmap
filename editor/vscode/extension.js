const cp = require("child_process");
const fs = require("fs");
const path = require("path");
const vscode = require("vscode");

const SUPPORTED_LANGUAGES = [
  "javascript",
  "javascriptreact",
  "typescript",
  "typescriptreact",
  "python",
  "rust",
  "go",
  "java",
  "c",
  "cpp",
  "csharp",
  "kotlin",
];
const CURRENT_FILE_LIMIT = 200;
const WORKSPACE_SEARCH_LIMIT = 80;

let sidebarProvider = null;
let sidebarView = null;
let noteDecorationType = null;
let anchorDecorationType = null;
let refreshTimer = null;
let codeLensEmitter = null;

function isChineseLocale() {
  return String(vscode.env.language || "").toLowerCase().startsWith("zh");
}

function t(en, zh) {
  return isChineseLocale() ? zh : en;
}

function localizedKind(kind) {
  const map = {
    comment: t("comment", "注释"),
    doc: t("doc", "文档"),
    todo: t("todo", "待办"),
    decision: t("decision", "决策"),
  };
  return map[kind] || kind || "comment";
}

function iconForKind(kind) {
  const map = {
    comment: "comment-discussion",
    doc: "book",
    todo: "checklist",
    decision: "git-commit",
  };
  return map[kind] || "comment-discussion";
}

function decorationColor(kind) {
  const map = {
    comment: "rgba(120, 120, 120, 0.9)",
    doc: "rgba(31, 120, 180, 0.95)",
    todo: "rgba(186, 120, 0, 0.95)",
    decision: "rgba(0, 140, 90, 0.95)",
  };
  return map[kind] || "rgba(120, 120, 120, 0.9)";
}

class LensMapSectionItem extends vscode.TreeItem {
  constructor(label, description, children, expanded) {
    super(
      label,
      expanded
        ? vscode.TreeItemCollapsibleState.Expanded
        : vscode.TreeItemCollapsibleState.Collapsed,
    );
    this.description = description;
    this.children = children;
    this.contextValue = "lensmapSection";
  }
}

class LensMapEntryItem extends vscode.TreeItem {
  constructor(entry) {
    const symbolLabel = entry.symbol_path || entry.symbol || entry.ref || entry.file;
    super(symbolLabel, vscode.TreeItemCollapsibleState.None);
    this.entry = entry;
    this.description = [localizedKind(entry.kind), formatEntryLine(entry)].filter(Boolean).join(" • ");
    this.tooltip = buildEntryTooltip(entry);
    this.iconPath = new vscode.ThemeIcon(iconForKind(entry.kind));
    this.contextValue = "lensmapEntry";
    this.command = {
      command: "lensmap.revealEntry",
      title: t("Reveal LensMap entry", "定位 LensMap 条目"),
      arguments: [entry],
    };
  }
}

class LensMapActionItem extends vscode.TreeItem {
  constructor(label, description, tooltip, command, icon) {
    super(label, vscode.TreeItemCollapsibleState.None);
    this.description = description;
    this.tooltip = tooltip;
    this.iconPath = new vscode.ThemeIcon(icon || "play");
    this.contextValue = "lensmapAction";
    this.command = {
      command,
      title: label,
    };
  }
}

class LensMapSidebarProvider {
  constructor() {
    this._onDidChangeTreeData = new vscode.EventEmitter();
    this.onDidChangeTreeData = this._onDidChangeTreeData.event;
    this.currentFileEntries = [];
    this.searchResults = [];
    this.searchQuery = "";
    this.currentFileLabel = t("Open a supported file to inspect LensMap notes.", "打开受支持的文件以查看 LensMap 注释。");
    this.policyStatus = t("Run policy check", "运行策略检查");
    this.summaryStatus = t("Open workspace summary", "打开工作区汇总");
    this.prReportStatus = t("Generate PR report", "生成 PR 报告");
  }

  getTreeItem(item) {
    return item;
  }

  getChildren(item) {
    if (item && Array.isArray(item.children)) {
      return item.children;
    }

    const currentLabel = this.currentFileTarget
      ? `${this.currentFileTarget}`
      : this.currentFileLabel;
    const currentSection = new LensMapSectionItem(
      t("Current File", "当前文件"),
      `${this.currentFileEntries.length}`,
      this.currentFileEntries.map((entry) => new LensMapEntryItem(entry)),
      true,
    );
    currentSection.tooltip = currentLabel;

    const searchSection = new LensMapSectionItem(
      this.searchQuery
        ? `${t("Workspace Search", "工作区搜索")}: ${this.searchQuery}`
        : t("Workspace Search", "工作区搜索"),
      `${this.searchResults.length}`,
      this.searchResults.map((entry) => new LensMapEntryItem(entry)),
      false,
    );
    const governanceSection = new LensMapSectionItem(
      t("Governance", "治理"),
      "",
      [
        new LensMapActionItem(
          t("Policy Check", "策略检查"),
          this.policyStatus,
          t("Run the aggregated LensMap policy check for this workspace.", "对当前工作区运行聚合 LensMap 策略检查。"),
          "lensmap.runPolicyCheck",
          "shield",
        ),
        new LensMapActionItem(
          t("Summary", "汇总"),
          this.summaryStatus,
          t("Render the aggregated LensMap workspace summary.", "渲染聚合 LensMap 工作区汇总。"),
          "lensmap.showSummary",
          "graph",
        ),
        new LensMapActionItem(
          t("PR Report", "PR 报告"),
          this.prReportStatus,
          t("Render the aggregated LensMap PR report for the current workspace.", "为当前工作区渲染聚合 LensMap PR 报告。"),
          "lensmap.showPrReport",
          "git-pull-request",
        ),
      ],
      false,
    );

    return [currentSection, searchSection, governanceSection];
  }

  async refresh(editor) {
    const targetEditor = editor || vscode.window.activeTextEditor;
    const snapshot = await loadCurrentFileEntries(targetEditor);
    this.currentFileEntries = snapshot.entries;
    this.currentFileTarget = snapshot.label;
    this.currentFileLabel = snapshot.message;

    if (sidebarView) {
      sidebarView.message = snapshot.info;
    }
    this._onDidChangeTreeData.fire();
    return snapshot.entries;
  }

  async runWorkspaceSearch(query, workspaceRoot) {
    const payload = await runLensmap(workspaceRoot, [
      "search",
      `--query=${query}`,
      `--limit=${WORKSPACE_SEARCH_LIMIT}`,
    ]);
    this.searchQuery = query;
    this.searchResults = normalizeSearchResults(payload.results || [], workspaceRoot);
    if (sidebarView) {
      sidebarView.message = this.searchResults.length
        ? ""
        : t("No LensMap results matched this search.", "没有匹配该搜索的 LensMap 结果。");
    }
    this._onDidChangeTreeData.fire();
  }

  setGovernanceStatus(kind, description) {
    if (kind === "policy") {
      this.policyStatus = description;
    } else if (kind === "summary") {
      this.summaryStatus = description;
    } else if (kind === "pr") {
      this.prReportStatus = description;
    }
    this._onDidChangeTreeData.fire();
  }
}

function activate(context) {
  sidebarProvider = new LensMapSidebarProvider();
  sidebarView = vscode.window.createTreeView("lensmapSidebar", {
    treeDataProvider: sidebarProvider,
    showCollapseAll: true,
  });
  noteDecorationType = vscode.window.createTextEditorDecorationType({
    after: {
      margin: "0 0 0 1.5rem",
      fontStyle: "italic",
    },
    rangeBehavior: vscode.DecorationRangeBehavior.ClosedClosed,
  });
  anchorDecorationType = vscode.window.createTextEditorDecorationType({
    color: new vscode.ThemeColor("editorCodeLens.foreground"),
    opacity: "0.45",
    fontStyle: "italic",
    rangeBehavior: vscode.DecorationRangeBehavior.ClosedClosed,
  });
  codeLensEmitter = new vscode.EventEmitter();

  context.subscriptions.push(
    sidebarView,
    noteDecorationType,
    anchorDecorationType,
    codeLensEmitter,
    vscode.commands.registerCommand("lensmap.showFileNotes", showFileNotes),
    vscode.commands.registerCommand("lensmap.annotateAtCursor", annotateAtCursor),
    vscode.commands.registerCommand("lensmap.editNoteAtCursor", editNoteAtCursor),
    vscode.commands.registerCommand("lensmap.editEntry", editEntry),
    vscode.commands.registerCommand("lensmap.refreshSidebar", async () => {
      await refreshLensmapUi(vscode.window.activeTextEditor);
    }),
    vscode.commands.registerCommand("lensmap.searchWorkspaceNotes", searchWorkspaceNotes),
    vscode.commands.registerCommand("lensmap.runPolicyCheck", runPolicyCheck),
    vscode.commands.registerCommand("lensmap.showSummary", showSummary),
    vscode.commands.registerCommand("lensmap.showPrReport", showPrReport),
    vscode.commands.registerCommand("lensmap.revealEntry", revealEntry),
    vscode.commands.registerCommand("lensmap.revealEntryGroup", revealEntryGroup),
    vscode.languages.registerHoverProvider(SUPPORTED_LANGUAGES, {
      provideHover(document, position) {
        return provideLensmapHover(document, position);
      },
    }),
    vscode.languages.registerCodeLensProvider(SUPPORTED_LANGUAGES, {
      onDidChangeCodeLenses: codeLensEmitter.event,
      provideCodeLenses(document) {
        return provideLensmapCodeLenses(document);
      },
    }),
    vscode.window.onDidChangeActiveTextEditor((editor) => scheduleRefresh(editor)),
    vscode.workspace.onDidSaveTextDocument((document) => {
      if (vscode.window.activeTextEditor?.document.uri.toString() === document.uri.toString()) {
        scheduleRefresh(vscode.window.activeTextEditor);
      }
    }),
    vscode.workspace.onDidChangeTextDocument((event) => {
      if (vscode.window.activeTextEditor?.document.uri.toString() === event.document.uri.toString()) {
        scheduleRefresh(vscode.window.activeTextEditor);
      }
    }),
  );

  scheduleRefresh(vscode.window.activeTextEditor);
}

function deactivate() {
  if (refreshTimer) {
    clearTimeout(refreshTimer);
    refreshTimer = null;
  }
}

function scheduleRefresh(editor) {
  if (refreshTimer) {
    clearTimeout(refreshTimer);
  }
  refreshTimer = setTimeout(() => {
    refreshLensmapUi(editor).catch(() => {});
  }, 150);
}

async function refreshLensmapUi(editor) {
  const entries = await sidebarProvider.refresh(editor);
  applyDecorations(editor || vscode.window.activeTextEditor, entries);
  if (codeLensEmitter) {
    codeLensEmitter.fire();
  }
}

function applyDecorations(editor, entries) {
  if (!editor || !noteDecorationType || !anchorDecorationType || !isSupportedDocument(editor.document)) {
    if (editor && noteDecorationType && anchorDecorationType) {
      editor.setDecorations(noteDecorationType, []);
      editor.setDecorations(anchorDecorationType, []);
    }
    return;
  }

  const decorations = [];
  const anchorDecorations = [];
  for (const entry of entries) {
    if (!entry.start_line) {
      continue;
    }
    const line = Math.max(entry.start_line - 1, 0);
    if (line >= editor.document.lineCount) {
      continue;
    }
    const lineText = editor.document.lineAt(line).text;
    const position = new vscode.Position(line, lineText.length);
    decorations.push({
      range: new vscode.Range(position, position),
      hoverMessage: buildEntryTooltip(entry),
      renderOptions: {
        after: {
          color: decorationColor(entry.kind),
          contentText: ` LensMap ${localizedKind(entry.kind)} ${entry.ref}: ${truncate(entry.text || "", 56)}`,
        },
      },
    });
  }
  editor.setDecorations(noteDecorationType, decorations);

  for (let line = 0; line < editor.document.lineCount; line += 1) {
    const text = editor.document.lineAt(line).text;
    const match = text.match(/(?:\/\/|#)\s*@lensmap-anchor\s+[A-Fa-f0-9]{6,16}\b/);
    if (!match || match.index === undefined) {
      continue;
    }
    anchorDecorations.push({
      range: new vscode.Range(line, match.index, line, match.index + match[0].length),
      hoverMessage: t("LensMap anchor", "LensMap 锚点"),
    });
  }
  editor.setDecorations(anchorDecorationType, anchorDecorations);
}

async function showFileNotes() {
  const editor = vscode.window.activeTextEditor;
  if (!editor) {
    vscode.window.showErrorMessage(t("LensMap needs an active editor.", "LensMap 需要一个当前激活的编辑器。"));
    return;
  }

  const ctx = await resolveLensmapContext(editor.document.uri);
  if (!ctx) {
    return;
  }

  const outPath = path.join(path.dirname(ctx.lensmapPath), "lensmap.vscode.show.md");
  try {
    await runLensmap(ctx.workspaceRoot, [
      "show",
      `--lensmap=${ctx.lensmapPath}`,
      `--file=${ctx.relativeFile}`,
      `--out=${outPath}`,
    ]);
    const uri = vscode.Uri.file(outPath);
    await vscode.commands.executeCommand("markdown.showPreview", uri);
  } catch (error) {
    vscode.window.showErrorMessage(
      t(`LensMap show failed: ${error.message}`, `LensMap 显示失败：${error.message}`),
    );
  }
}

async function annotateAtCursor() {
  const editor = vscode.window.activeTextEditor;
  if (!editor) {
    vscode.window.showErrorMessage(t("LensMap needs an active editor.", "LensMap 需要一个当前激活的编辑器。"));
    return;
  }

  const ctx = await resolveLensmapContext(editor.document.uri);
  if (!ctx) {
    return;
  }

  const symbol = await getSymbolAtCursor(editor);
  if (!symbol) {
    vscode.window.showErrorMessage(
      t(
        "LensMap could not resolve a function or method at the cursor.",
        "LensMap 无法解析光标位置所在的函数或方法。",
      ),
    );
    return;
  }

  const text = await vscode.window.showInputBox({
    prompt: t(`LensMap note for ${symbol.path}`, `为 ${symbol.path} 添加 LensMap 注释`),
    ignoreFocusOut: true,
  });
  if (!text || !text.trim()) {
    return;
  }

  const kind = await vscode.window.showQuickPick(
    [
      { label: localizedKind("comment"), value: "comment" },
      { label: localizedKind("doc"), value: "doc" },
      { label: localizedKind("todo"), value: "todo" },
      { label: localizedKind("decision"), value: "decision" },
    ],
    {
      title: t("LensMap note kind", "LensMap 注释类型"),
      canPickMany: false,
      ignoreFocusOut: true,
    },
  );
  if (!kind) {
    return;
  }

  const offset = Math.max(1, editor.selection.active.line - symbol.range.start.line + 1);
  const args = [
    "annotate",
    `--lensmap=${ctx.lensmapPath}`,
    `--file=${ctx.relativeFile}`,
    `--symbol=${symbol.name}`,
    `--offset=${offset}`,
    `--kind=${kind.value}`,
    `--text=${text.trim()}`,
  ];
  if (symbol.path && symbol.path !== symbol.name) {
    args.push(`--symbol-path=${symbol.path}`);
  }

  try {
    await runLensmap(ctx.workspaceRoot, args);
    await refreshLensmapUi(editor);
    vscode.window.showInformationMessage(
      t(`LensMap note added at ${symbol.path}.`, `已在 ${symbol.path} 添加 LensMap 注释。`),
    );
  } catch (error) {
    vscode.window.showErrorMessage(
      t(`LensMap annotate failed: ${error.message}`, `LensMap 注释失败：${error.message}`),
    );
  }
}

async function editNoteAtCursor() {
  const editor = vscode.window.activeTextEditor;
  if (!editor) {
    vscode.window.showErrorMessage(t("LensMap needs an active editor.", "LensMap 需要一个当前激活的编辑器。"));
    return;
  }

  const entries = await ensureCurrentFileEntries(editor);
  const cursorLine = editor.selection.active.line + 1;
  const candidates = entries.filter((entry) => {
    const start = entry.start_line || 0;
    const end = entry.end_line || entry.start_line || 0;
    return cursorLine >= start && cursorLine <= end;
  });
  if (candidates.length === 0) {
    await annotateAtCursor();
    return;
  }
  if (candidates.length === 1) {
    await editEntry(candidates[0]);
    return;
  }
  await editEntryGroup(candidates);
}

async function searchWorkspaceNotes() {
  const workspaceRoot = getWorkspaceRootFromActiveEditor() || getAnyWorkspaceRoot();
  if (!workspaceRoot) {
    vscode.window.showErrorMessage(
      t("Open a workspace before running LensMap search.", "运行 LensMap 搜索前请先打开工作区。"),
    );
    return;
  }

  const query = await vscode.window.showInputBox({
    prompt: t("Search LensMap notes across the workspace", "在整个工作区中搜索 LensMap 注释"),
    ignoreFocusOut: true,
  });
  if (!query || !query.trim()) {
    return;
  }

  try {
    await sidebarProvider.runWorkspaceSearch(query.trim(), workspaceRoot);
  } catch (error) {
    vscode.window.showErrorMessage(
      t(`LensMap search failed: ${error.message}`, `LensMap 搜索失败：${error.message}`),
    );
  }
}

async function runPolicyCheck() {
  const workspaceRoot = getWorkspaceRootFromActiveEditor() || getAnyWorkspaceRoot();
  if (!workspaceRoot) {
    vscode.window.showErrorMessage(
      t("Open a workspace before running LensMap policy checks.", "运行 LensMap 策略检查前请先打开工作区。"),
    );
    return;
  }

  try {
    const payload = await runWorkspaceReport(workspaceRoot, {
      kind: "policy",
      filename: "policy-check.md",
      args: (lensmaps, outPath) => [
        "policy",
        "check",
        `--lensmaps=${lensmaps.join(",")}`,
        "--report-only",
        `--out=${outPath}`,
      ],
      successMessage: (payload) => {
        const errors = payload.findings?.summary?.error_count || 0;
        const warnings = payload.findings?.summary?.warning_count || 0;
        const lensmaps = payload.stats?.lensmap_count || 0;
        return t(
          `LensMap policy check finished: ${errors} errors, ${warnings} warnings across ${lensmaps} maps.`,
          `LensMap 策略检查完成：${lensmaps} 个映射中有 ${errors} 个错误、${warnings} 个警告。`,
        );
      },
      status: (payload) => {
        const errors = payload.findings?.summary?.error_count || 0;
        const warnings = payload.findings?.summary?.warning_count || 0;
        return t(`${errors} errors • ${warnings} warnings`, `${errors} 个错误 • ${warnings} 个警告`);
      },
    });
    vscode.window.showInformationMessage(payload.message);
  } catch (error) {
    if (sidebarProvider) {
      sidebarProvider.setGovernanceStatus("policy", t("Policy check failed", "策略检查失败"));
    }
    vscode.window.showErrorMessage(
      t(`LensMap policy check failed: ${error.message}`, `LensMap 策略检查失败：${error.message}`),
    );
  }
}

async function showSummary() {
  const workspaceRoot = getWorkspaceRootFromActiveEditor() || getAnyWorkspaceRoot();
  if (!workspaceRoot) {
    vscode.window.showErrorMessage(
      t("Open a workspace before running LensMap summary.", "运行 LensMap 汇总前请先打开工作区。"),
    );
    return;
  }

  try {
    const payload = await runWorkspaceReport(workspaceRoot, {
      kind: "summary",
      filename: "summary.md",
      args: (lensmaps, outPath) => [
        "summary",
        `--lensmaps=${lensmaps.join(",")}`,
        `--out=${outPath}`,
      ],
      successMessage: (payload) => {
        const entries = payload.summary?.entry_count || 0;
        const files = payload.summary?.files_with_notes || 0;
        const stale = payload.summary?.stale_entries || 0;
        return t(
          `LensMap summary rendered: ${entries} notes across ${files} files, ${stale} stale.`,
          `LensMap 汇总已生成：${files} 个文件中共 ${entries} 条注释，${stale} 条过期。`,
        );
      },
      status: (payload) => {
        const entries = payload.summary?.entry_count || 0;
        const stale = payload.summary?.stale_entries || 0;
        return t(`${entries} notes • ${stale} stale`, `${entries} 条注释 • ${stale} 条过期`);
      },
    });
    vscode.window.showInformationMessage(payload.message);
  } catch (error) {
    if (sidebarProvider) {
      sidebarProvider.setGovernanceStatus("summary", t("Summary failed", "汇总失败"));
    }
    vscode.window.showErrorMessage(
      t(`LensMap summary failed: ${error.message}`, `LensMap 汇总失败：${error.message}`),
    );
  }
}

async function showPrReport() {
  const workspaceRoot = getWorkspaceRootFromActiveEditor() || getAnyWorkspaceRoot();
  if (!workspaceRoot) {
    vscode.window.showErrorMessage(
      t("Open a workspace before running LensMap PR report.", "运行 LensMap PR 报告前请先打开工作区。"),
    );
    return;
  }

  try {
    const payload = await runWorkspaceReport(workspaceRoot, {
      kind: "pr",
      filename: "pr-report.md",
      args: (lensmaps, outPath) => [
        "pr",
        "report",
        `--lensmaps=${lensmaps.join(",")}`,
        `--out=${outPath}`,
      ],
      successMessage: (payload) => {
        const entries = payload.entry_count || 0;
        const uncovered = Array.isArray(payload.uncovered_files) ? payload.uncovered_files.length : 0;
        const stale = Array.isArray(payload.stale_refs) ? payload.stale_refs.length : 0;
        return t(
          `LensMap PR report rendered: ${entries} notes, ${uncovered} uncovered files, ${stale} stale refs.`,
          `LensMap PR 报告已生成：${entries} 条注释、${uncovered} 个未覆盖文件、${stale} 个过期引用。`,
        );
      },
      status: (payload) => {
        const uncovered = Array.isArray(payload.uncovered_files) ? payload.uncovered_files.length : 0;
        const strictFailures = Array.isArray(payload.strict_failures) ? payload.strict_failures.length : 0;
        return t(
          `${uncovered} uncovered • ${strictFailures} strict issues`,
          `${uncovered} 个未覆盖 • ${strictFailures} 个严格失败`,
        );
      },
    });
    vscode.window.showInformationMessage(payload.message);
  } catch (error) {
    if (sidebarProvider) {
      sidebarProvider.setGovernanceStatus("pr", t("PR report failed", "PR 报告失败"));
    }
    vscode.window.showErrorMessage(
      t(`LensMap PR report failed: ${error.message}`, `LensMap PR 报告失败：${error.message}`),
    );
  }
}

async function revealEntry(entry) {
  if (!entry || !entry.workspaceRoot || !entry.file) {
    return;
  }
  const absPath = path.join(entry.workspaceRoot, fromPosix(entry.file));
  const uri = vscode.Uri.file(absPath);
  const document = await vscode.workspace.openTextDocument(uri);
  const editor = await vscode.window.showTextDocument(document, { preview: false });
  const line = Math.max((entry.start_line || 1) - 1, 0);
  const targetLine = Math.min(line, Math.max(document.lineCount - 1, 0));
  const position = new vscode.Position(targetLine, 0);
  editor.selection = new vscode.Selection(position, position);
  editor.revealRange(new vscode.Range(position, position), vscode.TextEditorRevealType.InCenter);
}

async function revealEntryGroup(entries) {
  if (!Array.isArray(entries) || entries.length === 0) {
    return;
  }
  if (entries.length === 1) {
    await revealEntry(entries[0]);
    return;
  }

  const picked = await vscode.window.showQuickPick(
    entries.map((entry) => ({
      label: entry.symbol_path || entry.ref,
      description: `${localizedKind(entry.kind)} • ${formatEntryLine(entry)}`,
      detail: truncate(entry.text || "", 100),
      entry,
    })),
    {
      title: t("Choose a LensMap note", "选择一个 LensMap 注释"),
      canPickMany: false,
      ignoreFocusOut: true,
    },
  );
  if (picked?.entry) {
    await revealEntry(picked.entry);
  }
}

async function editEntryGroup(entries) {
  if (!Array.isArray(entries) || entries.length === 0) {
    return;
  }
  if (entries.length === 1) {
    await editEntry(entries[0]);
    return;
  }

  const picked = await vscode.window.showQuickPick(
    entries.map((entry) => ({
      label: entry.symbol_path || entry.ref,
      description: `${localizedKind(entry.kind)} • ${formatEntryLine(entry)}`,
      detail: truncate(entry.text || "", 100),
      entry,
    })),
    {
      title: t("Edit which LensMap note?", "要编辑哪个 LensMap 注释？"),
      canPickMany: false,
      ignoreFocusOut: true,
    },
  );
  if (picked?.entry) {
    await editEntry(picked.entry);
  }
}

async function editEntry(entry) {
  if (!entry || !entry.ref) {
    return;
  }

  const workspaceRoot = entry.workspaceRoot || getWorkspaceRootFromActiveEditor() || getAnyWorkspaceRoot();
  if (!workspaceRoot) {
    vscode.window.showErrorMessage(
      t("Open a workspace before editing LensMap notes.", "编辑 LensMap 注释前请先打开工作区。"),
    );
    return;
  }

  const kind = await vscode.window.showQuickPick(
    [
      { label: localizedKind("comment"), value: "comment" },
      { label: localizedKind("doc"), value: "doc" },
      { label: localizedKind("todo"), value: "todo" },
      { label: localizedKind("decision"), value: "decision" },
    ],
    {
      title: t(`LensMap kind for ${entry.ref}`, `${entry.ref} 的 LensMap 类型`),
      canPickMany: false,
      ignoreFocusOut: true,
      placeHolder: localizedKind(entry.kind || "comment"),
    },
  );
  if (!kind) {
    return;
  }

  const text = await vscode.window.showInputBox({
    prompt: t(`Edit LensMap note ${entry.ref}`, `编辑 LensMap 注释 ${entry.ref}`),
    value: entry.text || "",
    ignoreFocusOut: true,
  });
  if (!text || !text.trim()) {
    return;
  }

  const lensmapPath = entry.lensmap
    ? path.join(workspaceRoot, fromPosix(entry.lensmap))
    : null;
  if (!lensmapPath) {
    vscode.window.showErrorMessage(
      t("LensMap could not resolve the backing lensmap file.", "LensMap 无法定位对应的 lensmap 文件。"),
    );
    return;
  }

  try {
    await runLensmap(workspaceRoot, [
      "annotate",
      `--lensmap=${lensmapPath}`,
      `--ref=${entry.ref}`,
      `--file=${entry.file}`,
      `--kind=${kind.value}`,
      `--text=${text.trim()}`,
    ]);
    await refreshLensmapUi(vscode.window.activeTextEditor);
    vscode.window.showInformationMessage(
      t(`LensMap note ${entry.ref} updated.`, `LensMap 注释 ${entry.ref} 已更新。`),
    );
  } catch (error) {
    vscode.window.showErrorMessage(
      t(`LensMap edit failed: ${error.message}`, `LensMap 编辑失败：${error.message}`),
    );
  }
}

async function provideLensmapHover(document, position) {
  const ctx = await resolveLensmapContext(document.uri, { silent: true });
  if (!ctx) {
    return undefined;
  }

  const line = document.lineAt(position.line).text;
  const refMatch = line.match(/@lensmap-ref\s+([A-Fa-f0-9]{6,16}-\d+(?:-\d+)?)/);
  const anchorMatch = line.match(/@lensmap-anchor\s+([A-Fa-f0-9]{6,16})/);
  if (!refMatch && !anchorMatch) {
    return undefined;
  }

  const doc = await loadLensmapDoc(ctx.lensmapPath);
  if (!doc) {
    return undefined;
  }

  const markdown = new vscode.MarkdownString(undefined, true);
  markdown.isTrusted = false;

  if (refMatch) {
    const refId = refMatch[1].toUpperCase();
    const entry = (doc.entries || []).find(
      (item) =>
        String(item.ref || "").toUpperCase() === refId &&
        (!item.file || item.file === ctx.relativeFile),
    );
    if (!entry) {
      return undefined;
    }
    markdown.appendMarkdown(`**LensMap ${refId}**\n\n`);
    markdown.appendMarkdown(`${t("Kind", "类型")}: \`${localizedKind(entry.kind || "comment")}\`\n\n`);
    markdown.appendText(entry.text || "");
    return new vscode.Hover(markdown);
  }

  const anchorId = anchorMatch[1].toUpperCase();
  const anchor = (doc.anchors || []).find(
    (item) =>
      String(item.id || "").toUpperCase() === anchorId &&
      (!item.file || item.file === ctx.relativeFile),
  );
  const entries = (doc.entries || []).filter(
    (item) =>
      String(item.anchor_id || "").toUpperCase() === anchorId &&
      (!item.file || item.file === ctx.relativeFile),
  );
  if (!anchor && entries.length === 0) {
    return undefined;
  }

  markdown.appendMarkdown(`**LensMap ${anchorId}**\n\n`);
  if (anchor) {
    const symbolPath = anchor.symbol_path || anchor.symbol || "?";
    markdown.appendMarkdown(`${t("Symbol", "符号")}: \`${symbolPath}\`\n\n`);
  }
  if (entries.length > 0) {
    for (const entry of entries.slice(0, 5)) {
      markdown.appendMarkdown(`- \`${entry.ref}\` ${localizedKind(entry.kind || "comment")}: `);
      markdown.appendText(entry.text || "");
      markdown.appendMarkdown("\n");
    }
  }
  return new vscode.Hover(markdown);
}

async function loadCurrentFileEntries(editor) {
  if (!editor || !isSupportedDocument(editor.document)) {
    return {
      entries: [],
      label: t("No supported file selected.", "当前未选择受支持的文件。"),
      message: t("Open a supported file to inspect LensMap notes.", "打开受支持的文件以查看 LensMap 注释。"),
      info: t("Open a supported file to inspect LensMap notes.", "打开受支持的文件以查看 LensMap 注释。"),
    };
  }

  const folder = vscode.workspace.getWorkspaceFolder(editor.document.uri);
  if (!folder) {
    return {
      entries: [],
      label: t("No workspace", "没有工作区"),
      message: t("LensMap needs the file to be inside an open workspace.", "LensMap 要求当前文件位于已打开的工作区中。"),
      info: t("LensMap needs the file to be inside an open workspace.", "LensMap 要求当前文件位于已打开的工作区中。"),
    };
  }

  const workspaceRoot = folder.uri.fsPath;
  const relativeFile = toPosix(path.relative(workspaceRoot, editor.document.uri.fsPath));
  try {
    const payload = await runLensmap(workspaceRoot, [
      "search",
      `--query=${relativeFile}`,
      `--file=${relativeFile}`,
      `--limit=${CURRENT_FILE_LIMIT}`,
    ]);
    const entries = normalizeSearchResults(payload.results || [], workspaceRoot);
    return {
      entries,
      label: relativeFile,
      message: relativeFile,
      info: entries.length
        ? ""
        : t("No LensMap notes for the current file.", "当前文件没有 LensMap 注释。"),
    };
  } catch (error) {
    return {
      entries: [],
      label: relativeFile,
      message: relativeFile,
      info: t(`LensMap search failed: ${error.message}`, `LensMap 搜索失败：${error.message}`),
    };
  }
}

function normalizeSearchResults(results, workspaceRoot) {
  return (Array.isArray(results) ? results : []).map((entry) => ({
    lensmap: entry.lensmap,
    file: entry.file,
    ref: entry.ref,
    anchor_id: entry.anchor_id,
    kind: entry.kind || "comment",
    text: entry.text || "",
    symbol: entry.symbol || "",
    symbol_path: entry.symbol_path || entry.symbol || "",
    start_line: Number.isInteger(entry.start_line) ? entry.start_line : null,
    end_line: Number.isInteger(entry.end_line) ? entry.end_line : null,
    resolve_strategy: entry.resolve_strategy || "",
    workspaceRoot,
  }));
}

function formatEntryLine(entry) {
  if (!entry.start_line) {
    return t("line ?", "第 ? 行");
  }
  if (entry.end_line && entry.end_line !== entry.start_line) {
    return isChineseLocale()
      ? `第 ${entry.start_line}-${entry.end_line} 行`
      : `line ${entry.start_line}-${entry.end_line}`;
  }
  return isChineseLocale() ? `第 ${entry.start_line} 行` : `line ${entry.start_line}`;
}

async function ensureCurrentFileEntries(editor) {
  if (!editor) {
    return [];
  }
  const snapshot = await loadCurrentFileEntries(editor);
  if (sidebarProvider) {
    sidebarProvider.currentFileEntries = snapshot.entries;
    sidebarProvider.currentFileTarget = snapshot.label;
    sidebarProvider.currentFileLabel = snapshot.message;
  }
  return snapshot.entries;
}

function provideLensmapCodeLenses(document) {
  const editor = vscode.window.activeTextEditor;
  if (!editor || editor.document.uri.toString() !== document.uri.toString()) {
    return [];
  }

  const grouped = groupEntriesByLine(sidebarProvider?.currentFileEntries || []);
  const out = [];
  for (const [line, entries] of grouped.entries()) {
    const zeroLine = Math.max(line - 1, 0);
    const range = new vscode.Range(zeroLine, 0, zeroLine, 0);
    const noteTitle = entries.length === 1
      ? t("LensMap: 1 note", "LensMap：1 条注释")
      : t(`LensMap: ${entries.length} notes`, `LensMap：${entries.length} 条注释`);
    out.push(
      new vscode.CodeLens(range, {
        command: "lensmap.revealEntryGroup",
        title: noteTitle,
        arguments: [entries],
      }),
    );
    out.push(
      new vscode.CodeLens(range, {
        command: entries.length === 1 ? "lensmap.editEntry" : "lensmap.editNoteAtCursor",
        title: entries.length === 1
          ? t("Edit LensMap note", "编辑 LensMap 注释")
          : t("Edit notes here", "编辑此处注释"),
        arguments: entries.length === 1 ? [entries[0]] : [],
      }),
    );
  }
  return out;
}

function groupEntriesByLine(entries) {
  const grouped = new Map();
  for (const entry of Array.isArray(entries) ? entries : []) {
    if (!entry.start_line) {
      continue;
    }
    const bucket = grouped.get(entry.start_line) || [];
    bucket.push(entry);
    grouped.set(entry.start_line, bucket);
  }
  return grouped;
}

function buildEntryTooltip(entry) {
  const markdown = new vscode.MarkdownString(undefined, true);
  markdown.isTrusted = false;
  markdown.appendMarkdown(`**${entry.symbol_path || entry.ref}**\n\n`);
  markdown.appendMarkdown(`- ${t("Ref", "引用")}: \`${entry.ref}\`\n`);
  markdown.appendMarkdown(`- ${t("Kind", "类型")}: \`${localizedKind(entry.kind)}\`\n`);
  markdown.appendMarkdown(`- ${t("File", "文件")}: \`${entry.file}\`\n`);
  if (entry.start_line) {
    markdown.appendMarkdown(`- ${t("Position", "位置")}: ${formatEntryLine(entry)}\n`);
  }
  if (entry.text) {
    markdown.appendMarkdown("\n");
    markdown.appendText(entry.text);
  }
  return markdown;
}

function truncate(value, limit) {
  const text = String(value || "").replace(/\s+/g, " ").trim();
  if (text.length <= limit) {
    return text;
  }
  return `${text.slice(0, Math.max(limit - 1, 1))}…`;
}

function isSupportedDocument(document) {
  return !!document && SUPPORTED_LANGUAGES.includes(document.languageId);
}

async function resolveLensmapContext(uri, options = {}) {
  const folder = vscode.workspace.getWorkspaceFolder(uri);
  if (!folder) {
    if (!options.silent) {
      vscode.window.showErrorMessage(
        t(
          "LensMap needs the file to be inside an open workspace.",
          "LensMap 要求当前文件位于已打开的工作区中。",
        ),
      );
    }
    return null;
  }

  const lensmapPath = await findLensmapForFile(uri, folder);
  if (!lensmapPath) {
    if (!options.silent) {
      vscode.window.showErrorMessage(
        t(
          "LensMap could not find a lensmap.json for this file.",
          "LensMap 找不到当前文件对应的 lensmap.json。",
        ),
      );
    }
    return null;
  }

  return {
    workspaceRoot: folder.uri.fsPath,
    lensmapPath,
    relativeFile: toPosix(path.relative(folder.uri.fsPath, uri.fsPath)),
  };
}

async function findLensmapForFile(uri, folder) {
  const root = folder.uri.fsPath;
  let current = path.dirname(uri.fsPath);
  while (current.startsWith(root)) {
    const direct = path.join(current, "lensmap.json");
    if (fs.existsSync(direct)) {
      return direct;
    }

    const entries = safeReadDir(current).filter((name) => name.endsWith(".lensmap.json"));
    if (entries.length > 0) {
      return path.join(current, entries[0]);
    }
    if (current === root) {
      break;
    }
    current = path.dirname(current);
  }

  const patterns = ["**/lensmap.json", "**/*.lensmap.json"];
  let matches = [];
  for (const pattern of patterns) {
    const found = await vscode.workspace.findFiles(
      new vscode.RelativePattern(folder, pattern),
      "**/{node_modules,target,.git}/**",
      50,
    );
    matches = matches.concat(found);
  }
  if (matches.length === 0) {
    return null;
  }

  matches.sort((left, right) => {
    const leftScore = sharedPrefixDepth(path.dirname(left.fsPath), path.dirname(uri.fsPath));
    const rightScore = sharedPrefixDepth(path.dirname(right.fsPath), path.dirname(uri.fsPath));
    return rightScore - leftScore;
  });
  return matches[0].fsPath;
}

function safeReadDir(dirPath) {
  try {
    return fs.readdirSync(dirPath);
  } catch (_error) {
    return [];
  }
}

function sharedPrefixDepth(left, right) {
  const leftParts = left.split(path.sep);
  const rightParts = right.split(path.sep);
  let depth = 0;
  while (depth < leftParts.length && depth < rightParts.length) {
    if (leftParts[depth] !== rightParts[depth]) {
      break;
    }
    depth += 1;
  }
  return depth;
}

async function getSymbolAtCursor(editor) {
  const symbols = await vscode.commands.executeCommand(
    "vscode.executeDocumentSymbolProvider",
    editor.document.uri,
  );
  if (!Array.isArray(symbols)) {
    return null;
  }
  return findInnermostSymbol(symbols, editor.selection.active, []);
}

function findInnermostSymbol(symbols, position, parents) {
  let best = null;
  for (const symbol of symbols) {
    if (!symbol.range.contains(position)) {
      continue;
    }
    const nextParents = parents.concat(symbol.name);
    const childBest = findInnermostSymbol(symbol.children || [], position, nextParents);
    if (childBest) {
      best = childBest;
      continue;
    }
    if (
      symbol.kind === vscode.SymbolKind.Function ||
      symbol.kind === vscode.SymbolKind.Method ||
      symbol.kind === vscode.SymbolKind.Constructor
    ) {
      best = {
        name: symbol.name,
        path: nextParents.join("."),
        range: symbol.range,
      };
    }
  }
  return best;
}

async function loadLensmapDoc(lensmapPath) {
  try {
    const raw = await fs.promises.readFile(lensmapPath, "utf8");
    return JSON.parse(raw);
  } catch (_error) {
    return null;
  }
}

async function runWorkspaceReport(workspaceRoot, options) {
  const lensmaps = await discoverWorkspaceLensmaps(workspaceRoot);
  if (!lensmaps.length) {
    throw new Error(t("No LensMap files were found in this workspace.", "当前工作区中未找到 LensMap 文件。"));
  }

  const outPath = await ensureReportArtifactPath(workspaceRoot, options.filename);
  const payload = await runLensmap(workspaceRoot, options.args(lensmaps, outPath));
  await openMarkdownPreview(outPath);
  if (sidebarProvider) {
    sidebarProvider.setGovernanceStatus(options.kind, options.status(payload));
  }
  return {
    payload,
    message: options.successMessage(payload),
  };
}

async function ensureReportArtifactPath(workspaceRoot, filename) {
  const baseDir = path.join(workspaceRoot, "local", "state", "lensmap", "vscode");
  await fs.promises.mkdir(baseDir, { recursive: true });
  return path.join(baseDir, filename);
}

async function openMarkdownPreview(fsPath) {
  const uri = vscode.Uri.file(fsPath);
  await vscode.commands.executeCommand("markdown.showPreview", uri);
}

async function discoverWorkspaceLensmaps(workspaceRoot) {
  const folder = vscode.workspace.workspaceFolders?.find((item) => item.uri.fsPath === workspaceRoot)
    || vscode.workspace.workspaceFolders?.find((item) => workspaceRoot.startsWith(item.uri.fsPath));
  if (!folder) {
    return discoverLensmapsByWalking(workspaceRoot);
  }

  const patterns = ["**/lensmap.json", "**/*.lensmap.json"];
  const found = [];
  for (const pattern of patterns) {
    const matches = await vscode.workspace.findFiles(
      new vscode.RelativePattern(folder, pattern),
      "**/{node_modules,target,.git,local/state}/**",
      400,
    );
    found.push(...matches.map((item) => toPosix(path.relative(workspaceRoot, item.fsPath))));
  }
  const unique = Array.from(new Set(found)).sort();
  return unique.length ? unique : discoverLensmapsByWalking(workspaceRoot);
}

function discoverLensmapsByWalking(workspaceRoot) {
  const out = [];
  const stack = [workspaceRoot];
  while (stack.length > 0) {
    const current = stack.pop();
    if (!current) {
      continue;
    }
    for (const entry of safeReadDir(current)) {
      const abs = path.join(current, entry);
      let stat;
      try {
        stat = fs.statSync(abs);
      } catch (_error) {
        continue;
      }
      if (stat.isDirectory()) {
        if ([".git", "node_modules", "target", "artifacts"].includes(entry)) {
          continue;
        }
        if (entry === "state" && path.basename(current) === "local") {
          continue;
        }
        stack.push(abs);
        continue;
      }
      if (entry === "lensmap.json" || entry.endsWith(".lensmap.json")) {
        out.push(toPosix(path.relative(workspaceRoot, abs)));
      }
    }
  }
  return Array.from(new Set(out)).sort();
}

async function runLensmap(workspaceRoot, args) {
  const invocation = resolveInvocation(workspaceRoot);
  return await execFileJson(invocation.command, invocation.args.concat(args), workspaceRoot);
}

function resolveInvocation(workspaceRoot) {
  const config = vscode.workspace.getConfiguration("lensmap");
  const configuredCommand = String(config.get("command") || "").trim();
  const extraArgs = Array.isArray(config.get("extraArgs")) ? config.get("extraArgs").slice() : [];
  if (!extraArgs.some((value) => String(value).startsWith("--lang=")) && isChineseLocale()) {
    extraArgs.push("--lang=zh-CN");
  }
  if (configuredCommand) {
    return {
      command: configuredCommand,
      args: extraArgs,
    };
  }

  const cargoRoot = path.join(workspaceRoot, "Cargo.toml");
  const cargoCrate = path.join(workspaceRoot, "crates", "lensmap-cli", "Cargo.toml");
  if (fs.existsSync(cargoRoot) && fs.existsSync(cargoCrate)) {
    return {
      command: "cargo",
      args: ["run", "-q", "-p", "lensmap", "--"].concat(extraArgs),
    };
  }

  return {
    command: "lensmap",
    args: extraArgs,
  };
}

function execFileJson(command, args, cwd) {
  return new Promise((resolve, reject) => {
    cp.execFile(
      command,
      args,
      {
        cwd,
        maxBuffer: 8 * 1024 * 1024,
      },
      (error, stdout, stderr) => {
        const payload = parseLensmapPayload(stdout) || parseLensmapPayload(stderr);
        if (error) {
          const message =
            (payload && payload.error) || stderr.trim() || stdout.trim() || error.message;
          reject(new Error(message));
          return;
        }
        resolve(payload || {});
      },
    );
  });
}

function parseLensmapPayload(text) {
  const trimmed = String(text || "").trim();
  if (!trimmed) {
    return null;
  }
  try {
    return JSON.parse(trimmed);
  } catch (_error) {
    return null;
  }
}

function getWorkspaceRootFromActiveEditor() {
  const editor = vscode.window.activeTextEditor;
  const folder = editor ? vscode.workspace.getWorkspaceFolder(editor.document.uri) : null;
  return folder ? folder.uri.fsPath : null;
}

function getAnyWorkspaceRoot() {
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath || null;
}

function toPosix(value) {
  return value.split(path.sep).join("/");
}

function fromPosix(value) {
  return String(value || "").split("/").join(path.sep);
}

module.exports = {
  activate,
  deactivate,
};
