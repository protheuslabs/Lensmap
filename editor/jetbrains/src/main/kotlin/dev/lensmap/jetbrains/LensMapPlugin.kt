package dev.lensmap.jetbrains

import com.google.gson.JsonArray
import com.google.gson.JsonObject
import com.google.gson.JsonParser
import com.intellij.notification.NotificationGroupManager
import com.intellij.notification.NotificationType
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.actionSystem.CommonDataKeys
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.Key
import com.intellij.openapi.ui.Messages
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.openapi.wm.ToolWindow
import com.intellij.openapi.wm.ToolWindowFactory
import com.intellij.openapi.wm.ToolWindowManager
import com.intellij.psi.PsiFile
import com.intellij.psi.PsiManager
import com.intellij.psi.PsiNamedElement
import com.intellij.ui.components.JBPanel
import com.intellij.ui.components.JBScrollPane
import com.intellij.ui.components.JBTextArea
import com.intellij.ui.content.ContentFactory
import java.awt.BorderLayout
import java.awt.FlowLayout
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.Paths
import java.util.Locale
import javax.swing.JButton
import javax.swing.JLabel
import javax.swing.JPanel
import kotlin.io.path.exists
import kotlin.io.path.isRegularFile

private const val TOOL_WINDOW_ID = "LensMap"
private val TOOL_WINDOW_PANEL_KEY = Key.create<LensMapToolWindowPanel>("LensMap.ToolWindowPanel")

private data class SearchEntry(
    val lensmap: String,
    val ref: String,
    val file: String,
    val kind: String,
    val text: String,
    val symbol: String,
    val symbolPath: String,
    val startLine: Int?,
    val endLine: Int?,
)

private data class SymbolGuess(
    val symbol: String,
    val symbolPath: String,
    val offset: Int,
)

private fun isChinese(): Boolean = Locale.getDefault().language.lowercase(Locale.ROOT).startsWith("zh")
private fun t(en: String, zh: String): String = if (isChinese()) zh else en

private object LensMapCli {
    fun run(project: Project, args: List<String>): JsonObject {
        val root = projectRoot(project) ?: error(t("Project root is unavailable.", "无法获取项目根目录。"))
        val command = resolveCommand(root) + languageArgs() + args
        val process = ProcessBuilder(command)
            .directory(root.toFile())
            .redirectErrorStream(false)
            .start()
        val stdout = process.inputStream.bufferedReader().readText().trim()
        val stderr = process.errorStream.bufferedReader().readText().trim()
        val code = process.waitFor()
        val payload = parseJsonObject(stdout).takeIf { it.size() > 0 }
            ?: parseJsonObject(stderr).takeIf { it.size() > 0 }
            ?: JsonObject()
        if (code != 0) {
            val error = payload.get("error")?.asString
                ?: stderr.ifBlank { stdout.ifBlank { t("LensMap invocation failed.", "LensMap 调用失败。") } }
            error(error)
        }
        return payload
    }

    private fun resolveCommand(root: Path): List<String> {
        val configured = System.getenv("LENSMAP_BIN")?.trim().orEmpty()
        if (configured.isNotEmpty()) {
            return listOf(configured)
        }
        val cargoRoot = root.resolve("Cargo.toml")
        val cargoCrate = root.resolve("crates/lensmap-cli/Cargo.toml")
        if (cargoRoot.exists() && cargoCrate.exists()) {
            return listOf("cargo", "run", "-q", "-p", "lensmap", "--")
        }
        return listOf("lensmap")
    }

    private fun languageArgs(): List<String> = if (isChinese()) listOf("--lang=zh-CN") else emptyList()

    private fun parseJsonObject(raw: String): JsonObject {
        return runCatching { JsonParser.parseString(raw).asJsonObject }.getOrElse { JsonObject() }
    }
}

private fun projectRoot(project: Project): Path? = project.basePath?.let(Paths::get)

private fun relativeFile(project: Project, virtualFile: VirtualFile): String? {
    val root = projectRoot(project) ?: return null
    val filePath = Paths.get(virtualFile.path)
    if (!filePath.startsWith(root)) {
        return null
    }
    return root.relativize(filePath).toString().replace('\\', '/')
}

private fun notify(project: Project, message: String, type: NotificationType = NotificationType.INFORMATION) {
    NotificationGroupManager.getInstance()
        .getNotificationGroup("LensMap")
        .createNotification(message, type)
        .notify(project)
}

private fun parseResults(payload: JsonObject): List<SearchEntry> {
    val results = payload.getAsJsonArray("results") ?: JsonArray()
    return results.mapNotNull { element ->
        val item = element.asJsonObject
        SearchEntry(
            lensmap = item.get("lensmap")?.asString.orEmpty(),
            ref = item.get("ref")?.asString.orEmpty(),
            file = item.get("file")?.asString.orEmpty(),
            kind = item.get("kind")?.asString ?: "comment",
            text = item.get("text")?.asString.orEmpty(),
            symbol = item.get("symbol")?.asString.orEmpty(),
            symbolPath = item.get("symbol_path")?.asString.orEmpty(),
            startLine = item.get("start_line")?.takeIf { !it.isJsonNull }?.asInt,
            endLine = item.get("end_line")?.takeIf { !it.isJsonNull }?.asInt,
        )
    }
}

private fun formatEntry(entry: SearchEntry): String {
    val label = entry.symbolPath.ifBlank { entry.symbol.ifBlank { entry.ref } }
    val line = when {
        entry.startLine == null -> t("line ?", "第 ? 行")
        entry.endLine != null && entry.endLine != entry.startLine -> {
            if (isChinese()) "第 ${entry.startLine}-${entry.endLine} 行" else "line ${entry.startLine}-${entry.endLine}"
        }
        else -> if (isChinese()) "第 ${entry.startLine} 行" else "line ${entry.startLine}"
    }
    val body = entry.text.ifBlank { t("(no text)", "（无文本）") }
    return buildString {
        append("- [${entry.ref}] $label • ${entry.kind} • $line")
        append("\n  ")
        append(body)
    }
}

private fun renderEntries(title: String, entries: List<SearchEntry>): String {
    if (entries.isEmpty()) {
        return "$title\n\n${t("No LensMap notes matched.", "没有匹配的 LensMap 注释。")}"
    }
    return buildString {
        append(title)
        append("\n\n")
        append(entries.joinToString("\n\n") { formatEntry(it) })
    }
}

private fun showMultiline(project: Project, title: String, content: String) {
    Messages.showMultilineInputDialog(project, content, title, content, null, null)
}

private class LensMapToolWindowPanel(project: Project) : JBPanel<LensMapToolWindowPanel>(BorderLayout()) {
    private val titleLabel = JLabel(TOOL_WINDOW_ID)
    private val bodyArea = JBTextArea().apply {
        isEditable = false
        lineWrap = true
        wrapStyleWord = true
    }
    private val refreshButton = JButton(t("Refresh", "刷新")).apply {
        isEnabled = false
    }
    private var refreshAction: (() -> Unit)? = null

    init {
        val toolbar = JPanel(FlowLayout(FlowLayout.LEFT, 8, 8))
        val currentFileButton = JButton(t("Current File", "当前文件")).apply {
            addActionListener {
                selectedFile(project)?.let { showCurrentFileNotes(project, it) }
                    ?: notify(project, t("Open a file first.", "请先打开一个文件。"), NotificationType.WARNING)
            }
        }
        val searchButton = JButton(t("Search", "搜索")).apply {
            addActionListener {
                val query = Messages.showInputDialog(
                    project,
                    t("Search LensMap notes across this project.", "在当前项目中搜索 LensMap 注释。"),
                    t("LensMap Workspace Search", "LensMap 工作区搜索"),
                    null,
                )?.trim().orEmpty()
                if (query.isNotEmpty()) {
                    searchWorkspaceNotes(project, query)
                }
            }
        }
        val annotateButton = JButton(t("Add Note", "添加注释")).apply {
            addActionListener {
                val editor = selectedEditor(project)
                val file = selectedFile(project)
                when {
                    editor == null -> notify(project, t("Open an editor first.", "请先打开一个编辑器。"), NotificationType.WARNING)
                    file == null -> notify(project, t("Open a file first.", "请先打开一个文件。"), NotificationType.WARNING)
                    else -> annotateAtCaret(project, editor, file)
                }
            }
        }
        val editButton = JButton(t("Edit Note", "编辑注释")).apply {
            addActionListener {
                val editor = selectedEditor(project)
                val file = selectedFile(project)
                when {
                    editor == null -> notify(project, t("Open an editor first.", "请先打开一个编辑器。"), NotificationType.WARNING)
                    file == null -> notify(project, t("Open a file first.", "请先打开一个文件。"), NotificationType.WARNING)
                    else -> editNoteAtCaret(project, editor, file)
                }
            }
        }

        refreshButton.addActionListener { refreshAction?.invoke() }

        toolbar.add(currentFileButton)
        toolbar.add(searchButton)
        toolbar.add(annotateButton)
        toolbar.add(editButton)
        toolbar.add(refreshButton)

        val header = JPanel(BorderLayout())
        header.add(titleLabel, BorderLayout.NORTH)
        header.add(toolbar, BorderLayout.CENTER)

        add(header, BorderLayout.NORTH)
        add(JBScrollPane(bodyArea), BorderLayout.CENTER)
    }

    fun render(title: String, content: String, onRefresh: (() -> Unit)? = null) {
        titleLabel.text = title
        bodyArea.text = content
        bodyArea.caretPosition = 0
        refreshAction = onRefresh
        refreshButton.isEnabled = onRefresh != null
    }
}

class LensMapToolWindowFactory : ToolWindowFactory {
    override fun createToolWindowContent(project: Project, toolWindow: ToolWindow) {
        ensureToolWindowPanel(project, toolWindow)
    }
}

private fun findLensmapForFile(project: Project, virtualFile: VirtualFile): Path? {
    val root = projectRoot(project) ?: return null
    var current: Path? = Paths.get(virtualFile.parent.path)
    while (current != null && current.startsWith(root)) {
        val direct = current.resolve("lensmap.json")
        if (direct.exists()) {
            return direct
        }
        Files.list(current).use { stream ->
            val match = stream
                .filter { it.isRegularFile() }
                .filter { it.fileName.toString().endsWith(".lensmap.json") }
                .findFirst()
            if (match.isPresent) {
                return match.get()
            }
        }
        current = current.parent
    }

    Files.walk(root).use { stream ->
        return stream
            .filter { it.isRegularFile() }
            .filter {
                val name = it.fileName.toString()
                name == "lensmap.json" || name.endsWith(".lensmap.json")
            }
            .findFirst()
            .orElse(null)
    }
}

private fun guessSymbolAtCaret(project: Project, editor: Editor, virtualFile: VirtualFile): SymbolGuess? {
    val psiFile: PsiFile = PsiManager.getInstance(project).findFile(virtualFile) ?: return null
    val element = psiFile.findElementAt(editor.caretModel.offset) ?: return null
    val fileStem = virtualFile.name.substringBeforeLast('.')
    val namedChain = generateSequence(element) { it.parent }
        .filterIsInstance<PsiNamedElement>()
        .mapNotNull { named -> named.name?.takeIf { it.isNotBlank() }?.let { named to it } }
        .filter { (named, name) -> name != fileStem && named.textRange.length > 0 }
        .distinctBy { (named, name) -> named.textRange.startOffset to name }
        .toList()
    if (namedChain.isEmpty()) {
        return null
    }

    val ordered = namedChain.asReversed().map { it.second }
    val symbolPath = ordered.joinToString(".")
    val symbol = ordered.last()
    val anchorElement = namedChain.first().first
    val startLine = editor.document.getLineNumber(anchorElement.textRange.startOffset)
    val offset = (editor.caretModel.logicalPosition.line - startLine + 1).coerceAtLeast(1)
    return SymbolGuess(symbol = symbol, symbolPath = symbolPath, offset = offset)
}

private fun promptKind(project: Project): String? {
    val labels = arrayOf(
        t("comment", "注释"),
        t("doc", "文档"),
        t("todo", "待办"),
        t("decision", "决策"),
    )
    val index = Messages.showDialog(
        project,
        t("Choose a LensMap note kind.", "选择 LensMap 注释类型。"),
        t("LensMap Note Kind", "LensMap 注释类型"),
        labels,
        0,
        null,
    )
    return when (index) {
        0 -> "comment"
        1 -> "doc"
        2 -> "todo"
        3 -> "decision"
        else -> null
    }
}

private fun contentFactory(): ContentFactory = ContentFactory.SERVICE.getInstance()

private fun toolWindowManager(project: Project): ToolWindowManager? =
    project.getService(ToolWindowManager::class.java)

private fun ensureToolWindowPanel(project: Project, toolWindow: ToolWindow): LensMapToolWindowPanel {
    project.getUserData(TOOL_WINDOW_PANEL_KEY)?.let { existing ->
        if (toolWindow.contentManager.contentCount == 0) {
            val content = contentFactory().createContent(existing, "", false)
            toolWindow.contentManager.addContent(content)
        }
        return existing
    }

    val panel = LensMapToolWindowPanel(project)
    val content = contentFactory().createContent(panel, "", false)
    toolWindow.contentManager.addContent(content)
    project.putUserData(TOOL_WINDOW_PANEL_KEY, panel)
    return panel
}

private fun showInToolWindow(project: Project, title: String, content: String, onRefresh: (() -> Unit)? = null) {
    val toolWindow = toolWindowManager(project)?.getToolWindow(TOOL_WINDOW_ID)
    if (toolWindow == null) {
        showMultiline(project, title, content)
        return
    }

    toolWindow.show {
        ensureToolWindowPanel(project, toolWindow).render(title, content, onRefresh)
    }
}

private fun selectedFile(project: Project): VirtualFile? =
    FileEditorManager.getInstance(project).selectedFiles.firstOrNull()

private fun selectedEditor(project: Project): Editor? =
    FileEditorManager.getInstance(project).selectedTextEditor

private fun showCurrentFileNotes(project: Project, virtualFile: VirtualFile) {
    val relative = relativeFile(project, virtualFile) ?: run {
        notify(project, t("File is outside the project root.", "文件位于项目根目录之外。"), NotificationType.WARNING)
        return
    }

    try {
        val payload = LensMapCli.run(project, listOf("search", "--query=$relative", "--file=$relative", "--limit=200"))
        val entries = parseResults(payload)
        showInToolWindow(
            project,
            t("LensMap Current File", "LensMap 当前文件"),
            renderEntries(relative, entries),
        ) { showCurrentFileNotes(project, virtualFile) }
    } catch (error: Throwable) {
        notify(project, error.message ?: t("LensMap failed.", "LensMap 执行失败。"), NotificationType.ERROR)
    }
}

private fun loadCurrentFileEntries(project: Project, virtualFile: VirtualFile): List<SearchEntry> {
    val relative = relativeFile(project, virtualFile) ?: return emptyList()
    val payload = LensMapCli.run(project, listOf("search", "--query=$relative", "--file=$relative", "--limit=200"))
    return parseResults(payload)
}

private fun searchWorkspaceNotes(project: Project, query: String) {
    try {
        val payload = LensMapCli.run(project, listOf("search", "--query=$query", "--limit=80"))
        val entries = parseResults(payload)
        showInToolWindow(
            project,
            t("LensMap Workspace Search", "LensMap 工作区搜索"),
            renderEntries(query, entries),
        ) { searchWorkspaceNotes(project, query) }
    } catch (error: Throwable) {
        notify(project, error.message ?: t("LensMap search failed.", "LensMap 搜索失败。"), NotificationType.ERROR)
    }
}

private fun selectEntry(project: Project, entries: List<SearchEntry>, title: String): SearchEntry? {
    if (entries.isEmpty()) {
        return null
    }
    if (entries.size == 1) {
        return entries.first()
    }
    val labels = entries.map { entry ->
        val line = when {
            entry.startLine == null -> "?"
            entry.endLine != null && entry.endLine != entry.startLine -> "${entry.startLine}-${entry.endLine}"
            else -> "${entry.startLine}"
        }
        "${entry.symbolPath.ifBlank { entry.ref }} • ${entry.kind} • ${line}"
    }.toTypedArray()
    val index = Messages.showDialog(
        project,
        title,
        TOOL_WINDOW_ID,
        labels,
        0,
        null,
    )
    return entries.getOrNull(index)
}

private fun editEntry(project: Project, entry: SearchEntry) {
    val projectRoot = projectRoot(project) ?: return
    val lensmapRel = entry.lensmap.ifBlank {
        notify(project, t("LensMap file is missing for this note.", "当前注释缺少对应的 LensMap 文件。"), NotificationType.ERROR)
        return
    }
    val kind = promptKind(project) ?: return
    val text = Messages.showMultilineInputDialog(
        project,
        t("LensMap note text", "LensMap 注释内容"),
        t("Edit LensMap Note", "编辑 LensMap 注释"),
        entry.text,
        null,
        null,
    )?.trim().orEmpty()
    if (text.isEmpty()) {
        return
    }

    try {
        LensMapCli.run(
            project,
            listOf(
                "annotate",
                "--lensmap=${projectRoot.resolve(lensmapRel).toString().replace('\\', '/')}",
                "--ref=${entry.ref}",
                "--file=${entry.file}",
                "--kind=$kind",
                "--text=$text",
            ),
        )
        notify(project, t("LensMap note updated.", "LensMap 注释已更新。"))
    } catch (error: Throwable) {
        notify(project, error.message ?: t("LensMap edit failed.", "LensMap 编辑失败。"), NotificationType.ERROR)
    }
}

private fun editNoteAtCaret(project: Project, editor: Editor, virtualFile: VirtualFile) {
    try {
        val entries = loadCurrentFileEntries(project, virtualFile)
        val caretLine = editor.document.getLineNumber(editor.caretModel.offset) + 1
        val candidates = entries.filter { entry ->
            val start = entry.startLine ?: 0
            val end = entry.endLine ?: entry.startLine ?: 0
            caretLine in start..end
        }
        if (candidates.isEmpty()) {
            annotateAtCaret(project, editor, virtualFile)
            return
        }
        val picked = selectEntry(
            project,
            candidates,
            t("Choose a LensMap note to edit.", "选择要编辑的 LensMap 注释。"),
        ) ?: return
        editEntry(project, picked)
        showCurrentFileNotes(project, virtualFile)
    } catch (error: Throwable) {
        notify(project, error.message ?: t("LensMap edit failed.", "LensMap 编辑失败。"), NotificationType.ERROR)
    }
}

private fun annotateAtCaret(project: Project, editor: Editor, virtualFile: VirtualFile) {
    val relative = relativeFile(project, virtualFile) ?: return
    val lensmapPath = findLensmapForFile(project, virtualFile) ?: run {
        notify(project, t("LensMap file not found for this file.", "未找到当前文件对应的 LensMap 文件。"), NotificationType.ERROR)
        return
    }
    val guess = guessSymbolAtCaret(project, editor, virtualFile)
    val symbolPath = Messages.showInputDialog(
        project,
        t("LensMap symbol path", "LensMap 符号路径"),
        t("Add LensMap Note", "添加 LensMap 注释"),
        null,
        guess?.symbolPath.orEmpty(),
        null,
    )?.trim().orEmpty()
    if (symbolPath.isEmpty()) {
        return
    }
    val symbol = symbolPath.substringAfterLast('.')
    val kind = promptKind(project) ?: return
    val text = Messages.showMultilineInputDialog(
        project,
        t("LensMap note text", "LensMap 注释内容"),
        t("Add LensMap Note", "添加 LensMap 注释"),
        "",
        null,
        null,
    )?.trim().orEmpty()
    if (text.isEmpty()) {
        return
    }
    val offset = guess?.takeIf { it.symbolPath == symbolPath }?.offset ?: 1

    try {
        LensMapCli.run(
            project,
            listOf(
                "annotate",
                "--lensmap=${lensmapPath.toString().replace('\\', '/')}",
                "--file=$relative",
                "--symbol=$symbol",
                "--symbol-path=$symbolPath",
                "--offset=$offset",
                "--kind=$kind",
                "--text=$text",
            ),
        )
        notify(project, t("LensMap note added.", "LensMap 注释已添加。"))
        showCurrentFileNotes(project, virtualFile)
    } catch (error: Throwable) {
        notify(project, error.message ?: t("LensMap annotate failed.", "LensMap 注释失败。"), NotificationType.ERROR)
    }
}

class ShowFileNotesAction : AnAction() {
    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val virtualFile = e.getData(CommonDataKeys.VIRTUAL_FILE) ?: run {
            notify(project, t("Open a file first.", "请先打开一个文件。"), NotificationType.WARNING)
            return
        }
        showCurrentFileNotes(project, virtualFile)
    }
}

class SearchWorkspaceNotesAction : AnAction() {
    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val query = Messages.showInputDialog(
            project,
            t("Search LensMap notes across this project.", "在当前项目中搜索 LensMap 注释。"),
            t("LensMap Workspace Search", "LensMap 工作区搜索"),
            null,
        )?.trim().orEmpty()
        if (query.isEmpty()) {
            return
        }
        searchWorkspaceNotes(project, query)
    }
}

class AnnotateAtCaretAction : AnAction() {
    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val editor = e.getData(CommonDataKeys.EDITOR) ?: selectedEditor(project) ?: run {
            notify(project, t("Open an editor first.", "请先打开一个编辑器。"), NotificationType.WARNING)
            return
        }
        val virtualFile = e.getData(CommonDataKeys.VIRTUAL_FILE) ?: selectedFile(project) ?: run {
            notify(project, t("Open a file first.", "请先打开一个文件。"), NotificationType.WARNING)
            return
        }
        annotateAtCaret(project, editor, virtualFile)
    }
}

class EditNoteAtCaretAction : AnAction() {
    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val editor = e.getData(CommonDataKeys.EDITOR) ?: selectedEditor(project) ?: run {
            notify(project, t("Open an editor first.", "请先打开一个编辑器。"), NotificationType.WARNING)
            return
        }
        val virtualFile = e.getData(CommonDataKeys.VIRTUAL_FILE) ?: selectedFile(project) ?: run {
            notify(project, t("Open a file first.", "请先打开一个文件。"), NotificationType.WARNING)
            return
        }
        editNoteAtCaret(project, editor, virtualFile)
    }
}
