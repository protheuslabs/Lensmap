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

function activate(context) {
  context.subscriptions.push(
    vscode.commands.registerCommand("lensmap.showFileNotes", showFileNotes),
    vscode.commands.registerCommand("lensmap.annotateAtCursor", annotateAtCursor),
    vscode.languages.registerHoverProvider(SUPPORTED_LANGUAGES, {
      provideHover(document, position) {
        return provideLensmapHover(document, position);
      },
    }),
  );
}

function deactivate() {}

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
    vscode.window.showInformationMessage(
      t(`LensMap note added at ${symbol.path}.`, `已在 ${symbol.path} 添加 LensMap 注释。`),
    );
  } catch (error) {
    vscode.window.showErrorMessage(
      t(`LensMap annotate failed: ${error.message}`, `LensMap 注释失败：${error.message}`),
    );
  }
}

async function provideLensmapHover(document, position) {
  const ctx = await resolveLensmapContext(document.uri);
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

async function resolveLensmapContext(uri) {
  const folder = vscode.workspace.getWorkspaceFolder(uri);
  if (!folder) {
    vscode.window.showErrorMessage(
      t(
        "LensMap needs the file to be inside an open workspace.",
        "LensMap 要求当前文件位于已打开的工作区中。",
      ),
    );
    return null;
  }

  const lensmapPath = await findLensmapForFile(uri, folder);
  if (!lensmapPath) {
    vscode.window.showErrorMessage(
      t(
        "LensMap could not find a lensmap.json for this file.",
        "LensMap 找不到当前文件对应的 lensmap.json。",
      ),
    );
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

function toPosix(value) {
  return value.split(path.sep).join("/");
}

module.exports = {
  activate,
  deactivate,
};
