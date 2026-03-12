use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use tree_sitter::{Node, Parser};
use walkdir::WalkDir;

const SKIP_PREFIXES: &[&str] = &[
    ".git/",
    "node_modules/",
    "dist/",
    "target/",
    "artifacts/",
    "coverage/",
    "local/state/",
    "local/private-lenses/",
];
const SUPPORTED_EXTS: &[&str] = &[
    ".js", ".ts", ".tsx", ".jsx", ".mjs", ".cjs", ".py", ".rs", ".go", ".java", ".c", ".h", ".cc",
    ".cpp", ".cxx", ".hh", ".hpp", ".hxx", ".cs", ".kt", ".kts",
];
const PRESERVE_COMMENT_PREFIXES: &[&str] = &[
    "!/usr/bin/env",
    "!/bin/",
    "@license",
    "@preserve",
    "@ts-ignore",
    "@ts-expect-error",
    "eslint",
    "region",
    "endregion",
    "cspell:",
    "istanbul",
    "pragma",
];
const ANCHOR_TAG: &str = "@lensmap-anchor";
const REF_TAG: &str = "@lensmap-ref";

#[derive(Debug, Default)]
struct ParsedArgs {
    positional: Vec<String>,
    flags: HashMap<String, String>,
}

impl ParsedArgs {
    fn parse(argv: &[String]) -> Self {
        let mut out = Self::default();
        for tok in argv {
            if !tok.starts_with("--") {
                out.positional.push(tok.clone());
                continue;
            }
            if let Some((k, v)) = tok[2..].split_once('=') {
                out.flags.insert(k.to_string(), v.to_string());
            } else {
                out.flags.insert(tok[2..].to_string(), "true".to_string());
            }
        }
        out
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.flags.get(key).map(String::as_str)
    }

    fn has(&self, key: &str) -> bool {
        self.flags.get(key).map(|v| v == "true").unwrap_or(false)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
struct AnchorRecord {
    id: String,
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    symbol_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line_anchor: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line_symbol: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    span_start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    span_end: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    placement: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
struct EntryRecord {
    #[serde(rename = "ref")]
    ref_id: String,
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    anchor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
struct LensMapDoc {
    #[serde(rename = "type")]
    doc_type: String,
    version: String,
    mode: String,
    created_at: String,
    updated_at: String,
    covers: Vec<String>,
    anchors: Vec<AnchorRecord>,
    entries: Vec<EntryRecord>,
    metadata: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
struct LensMapIndexDoc {
    #[serde(rename = "type")]
    doc_type: String,
    version: String,
    root: String,
    generated_at: String,
    lensmaps: Vec<String>,
    entries: Vec<SearchEntryRecord>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
struct SearchEntryRecord {
    lensmap: String,
    file: String,
    #[serde(rename = "ref")]
    ref_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    anchor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    symbol_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end_line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolve_strategy: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
struct PackageItem {
    id: String,
    original_path: String,
    packaged_path: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolved_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
struct PackageManifest {
    #[serde(rename = "type")]
    doc_type: String,
    version: String,
    root: String,
    bundle_dir: String,
    created_at: String,
    updated_at: String,
    items: Vec<PackageItem>,
}

#[derive(Clone, Debug)]
struct FunctionHit {
    line_index: usize,
    span_end_index: usize,
    symbol: String,
    symbol_path: String,
    indent: String,
    fingerprint: String,
    signature_text: String,
}

#[derive(Clone, Debug)]
struct AnchorLineMatch {
    id: String,
    marker: String,
    is_inline: bool,
}

#[derive(Clone, Debug)]
struct RefLineMatch {
    ref_id: String,
    marker: String,
}

#[derive(Clone, Debug)]
struct CommentStyle {
    line: &'static str,
    block_start: Option<&'static str>,
    block_end: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AnchorPlacement {
    Standalone,
    Inline,
}

#[derive(Clone, Debug, Default)]
struct GitHunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
}

#[derive(Clone, Debug)]
struct CommentBlock {
    kind: String,
    start: usize,
    end: usize,
    indent: String,
    marker: String,
    text: String,
    replace_mode: String,
    inline_prefix: String,
    inline_spacing: String,
}

#[derive(Clone, Debug)]
struct InlineCommentMatch {
    content: String,
    prefix: String,
    spacing: String,
}

#[derive(Clone, Debug)]
struct InlineBlockMatch {
    content: String,
    prefix: String,
    spacing: String,
}

#[derive(Clone, Debug)]
struct RefParts {
    anchor_id: String,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug)]
struct RefSite {
    ref_id: String,
    is_inline: bool,
    indent: String,
    prefix: String,
    spacing: String,
}

#[derive(Clone, Debug)]
struct AnchorResolution {
    anchor_line_index: Option<usize>,
    function_hit: Option<FunctionHit>,
    strategy: String,
    anchor_found_in_source: bool,
}

#[derive(Clone, Copy)]
struct SmartAnchorContext<'a> {
    functions: &'a [FunctionHit],
    lines: &'a [String],
    blocks: &'a [CommentBlock],
    style: &'a CommentStyle,
    tracked_anchors: &'a [AnchorRecord],
}

#[derive(Clone, Copy)]
struct RenderFilters<'a> {
    file: Option<&'a str>,
    symbol: Option<&'a str>,
    ref_id: Option<&'a str>,
    kind: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UiLang {
    En,
    ZhCn,
}

impl UiLang {
    fn code(self) -> &'static str {
        match self {
            UiLang::En => "en",
            UiLang::ZhCn => "zh-CN",
        }
    }
}

fn parse_ui_lang(raw: &str) -> UiLang {
    let normalized = raw.trim().to_lowercase().replace('_', "-");
    if normalized.starts_with("zh") {
        UiLang::ZhCn
    } else {
        UiLang::En
    }
}

fn detect_ui_lang_from_process() -> UiLang {
    for arg in env::args().skip(1) {
        if let Some(raw) = arg.strip_prefix("--lang=") {
            return parse_ui_lang(raw);
        }
    }
    for key in ["LENSMAP_LANG", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = env::var(key) {
            if !value.trim().is_empty() {
                return parse_ui_lang(&value);
            }
        }
    }
    UiLang::En
}

fn ui_lang() -> UiLang {
    static LANG: OnceLock<UiLang> = OnceLock::new();
    *LANG.get_or_init(detect_ui_lang_from_process)
}

fn tr(en: &str, zh_cn: &str) -> String {
    match ui_lang() {
        UiLang::En => en.to_string(),
        UiLang::ZhCn => zh_cn.to_string(),
    }
}

fn localized_error_message(error: &str) -> Option<String> {
    let message = match error {
        "lensmap_missing" => tr(
            "LensMap file was not found. Run init first or pass a real --lensmap path.",
            "未找到 LensMap 文件。请先运行 init，或传入真实的 --lensmap 路径。",
        ),
        "invalid_anchor_mode" => tr(
            "Invalid anchor mode. Use smart or all.",
            "无效的锚点模式。请使用 smart 或 all。",
        ),
        "no_files_resolved" => tr(
            "No source files matched the requested covers.",
            "没有解析到符合 covers 的源码文件。",
        ),
        "annotate_requires_text" => tr(
            "Annotate requires --text.",
            "annotate 命令需要提供 --text。",
        ),
        "annotate_requires_ref_or_file_symbol" => tr(
            "Annotate requires either --ref or --file with --symbol.",
            "annotate 需要 --ref，或者同时提供 --file 和 --symbol。",
        ),
        "invalid_offset" => tr("Invalid --offset value.", "无效的 --offset 值。"),
        "invalid_end_offset" => tr("Invalid --end-offset value.", "无效的 --end-offset 值。"),
        "symbol_anchor_failed" => tr(
            "LensMap could not resolve or create an anchor for the requested symbol.",
            "LensMap 无法为请求的符号解析或创建锚点。",
        ),
        "invalid_ref" => tr(
            "Invalid LensMap reference format.",
            "无效的 LensMap 引用格式。",
        ),
        "entry_anchor_missing" => tr(
            "The requested reference points to an anchor that does not exist.",
            "请求的引用指向了不存在的锚点。",
        ),
        "file_required" => tr("A file path is required here.", "这里需要提供文件路径。"),
        "security_entry_outside_root" => tr(
            "The entry target is outside the repository root. Operation blocked.",
            "目标条目位于仓库根目录之外。操作已阻止。",
        ),
        "invalid_kind" => tr(
            "Invalid entry kind. Use comment, doc, todo, or decision.",
            "无效的条目类型。请使用 comment、doc、todo 或 decision。",
        ),
        "invalid_mode" => tr(
            "Invalid packaging mode. Use move or copy.",
            "无效的打包模式。请使用 move 或 copy。",
        ),
        "security_bundle_outside_root" => tr(
            "Bundle directory is outside the repository root. Operation blocked.",
            "打包目录位于仓库根目录之外。操作已阻止。",
        ),
        "no_lensmap_files_found" => tr("No LensMap files were found.", "没有找到 LensMap 文件。"),
        "invalid_on_missing" => tr(
            "Invalid --on-missing mode. Use prompt, skip, or error.",
            "无效的 --on-missing 模式。请使用 prompt、skip 或 error。",
        ),
        "manifest_missing" => tr("Package manifest is missing.", "打包清单缺失。"),
        "security_target_outside_root" => tr(
            "Restore target is outside the repository root. Operation blocked.",
            "恢复目标位于仓库根目录之外。操作已阻止。",
        ),
        "missing_dir_error" => tr(
            "Original target directory is missing and --on-missing=error was selected.",
            "原始目标目录不存在，并且选择了 --on-missing=error。",
        ),
        "security_prompt_target_outside_root" => tr(
            "Prompt-provided target is outside the repository root. Operation blocked.",
            "交互输入的目标位于仓库根目录之外。操作已阻止。",
        ),
        "target_exists_use_overwrite" => tr(
            "Target already exists. Re-run with --overwrite to replace it.",
            "目标已存在。如需覆盖，请重新运行并添加 --overwrite。",
        ),
        "security_output_outside_root" => tr(
            "Output path is outside the repository root. Operation blocked.",
            "输出路径位于仓库根目录之外。操作已阻止。",
        ),
        "from_required" => tr("Import requires --from.", "import 命令需要提供 --from。"),
        "query_required" => tr("Search requires --query.", "search 命令需要提供 --query。"),
        _ if error.starts_with("copy_failed:") => tr(
            "A file copy failed during packaging or unpackaging.",
            "打包或解包过程中出现文件复制失败。",
        ),
        _ if error.starts_with("remove_source_failed:") => tr(
            "A packaged source file could not be removed after copy.",
            "复制完成后，无法移除原始打包文件。",
        ),
        _ => return None,
    };
    Some(message)
}

fn localized_action_message(action: &str) -> Option<String> {
    let message = match action {
        "init" => tr("LensMap initialized.", "LensMap 已初始化。"),
        "template_add" => tr("Template created.", "模板已创建。"),
        "scan" => tr("Anchor scan completed.", "锚点扫描已完成。"),
        "extract_comments" => tr("Comments extracted into LensMap.", "注释已提取到 LensMap。"),
        "merge" => tr(
            "LensMap entries merged back into source files.",
            "LensMap 条目已合并回源码文件。",
        ),
        "annotate" => tr("Annotation saved.", "注释已保存。"),
        "package" => tr("LensMap files packaged.", "LensMap 文件已打包。"),
        "unpackage" => tr(
            "LensMap files restored from the package bundle.",
            "LensMap 文件已从打包目录恢复。",
        ),
        "validate" => tr("Validation completed.", "校验已完成。"),
        "reanchor" => tr("Anchor re-resolution completed.", "锚点重新解析已完成。"),
        "render" => tr(
            "Rendered Markdown view written.",
            "渲染后的 Markdown 视图已写入。",
        ),
        "show" => tr(
            "Filtered LensMap view written.",
            "筛选后的 LensMap 视图已写入。",
        ),
        "simplify" => tr("LensMap document simplified.", "LensMap 文档已简化。"),
        "polish" => tr("Polish artifacts refreshed.", "整理产物已刷新。"),
        "import" => tr("Import receipt created.", "导入回执已创建。"),
        "sync" => tr("LensMap sync completed.", "LensMap 同步已完成。"),
        "index" => tr("LensMap index refreshed.", "LensMap 索引已刷新。"),
        "search" => tr("LensMap search completed.", "LensMap 搜索已完成。"),
        "expose" => tr(
            "Lens exposed to the private store.",
            "镜头已暴露到私有存储。",
        ),
        "status" => tr("Status collected.", "状态已收集。"),
        _ => return None,
    };
    Some(message)
}

fn localize_payload(payload: &mut Value) {
    let Some(obj) = payload.as_object_mut() else {
        return;
    };
    obj.entry("lang".to_string())
        .or_insert_with(|| Value::String(ui_lang().code().to_string()));
    if obj.contains_key("message") {
        return;
    }
    if let Some(error) = obj.get("error").and_then(Value::as_str) {
        if let Some(message) = localized_error_message(error) {
            obj.insert("message".to_string(), Value::String(message));
        }
        return;
    }
    if obj.get("ok").and_then(Value::as_bool) != Some(true) {
        return;
    }
    if let Some(action) = obj.get("action").and_then(Value::as_str) {
        if let Some(message) = localized_action_message(action) {
            obj.insert("message".to_string(), Value::String(message));
        }
    }
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn emit(mut payload: Value, code: i32) -> ! {
    localize_payload(&mut payload);
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
    );
    std::process::exit(code);
}

fn to_posix_str(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(seg) => out.push(seg),
        }
    }
    out
}

fn detect_root() -> PathBuf {
    let mut dir = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        if dir.join(".git").exists() {
            return dir;
        }
        if !dir.pop() {
            break;
        }
    }
    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn normalize_relative(root: &Path, p: &Path) -> String {
    if let Ok(rel) = p.strip_prefix(root) {
        return to_posix_str(rel);
    }
    to_posix_str(p)
}

fn resolve_from_root(root: &Path, raw: &str) -> PathBuf {
    let p = Path::new(raw);
    if p.is_absolute() {
        normalize_path(p)
    } else {
        normalize_path(&root.join(p))
    }
}

fn is_within_root(root: &Path, path: &Path) -> bool {
    let nr = normalize_path(root);
    let np = normalize_path(path);
    np.starts_with(&nr)
}

fn ensure_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
}

fn split_csv(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or("")
        .split(',')
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect()
}

fn hash_text(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    let hex = hex::encode(hasher.finalize());
    hex[..12].to_string()
}

fn metadata_string_array(values: &[&str]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|v| Value::String((*v).to_string()))
            .collect(),
    )
}

fn apply_default_metadata(metadata: &mut Map<String, Value>) {
    metadata
        .entry("positioning".to_string())
        .or_insert_with(|| Value::String("external-doc-layer".to_string()));
    metadata
        .entry("default_anchor_mode".to_string())
        .or_insert_with(|| Value::String("smart".to_string()));
    metadata
        .entry("primary_artifact".to_string())
        .or_insert_with(|| Value::String("json+markdown".to_string()));
    metadata
        .entry("anchor_placement".to_string())
        .or_insert_with(|| Value::String("inline".to_string()));
    metadata
        .entry("editor_anchor_visibility".to_string())
        .or_insert_with(|| Value::String("dimmed".to_string()));
    metadata
        .entry("git_reanchor_policy".to_string())
        .or_insert_with(|| Value::String("protect-dirty-overlaps".to_string()));
    metadata
        .entry("inline_keeps".to_string())
        .or_insert_with(|| {
            metadata_string_array(&[
                "local intent that improves immediate readability",
                "language directives and preserve comments",
                "short comments that belong directly beside the code",
            ])
        });
    metadata
        .entry("external_best_for".to_string())
        .or_insert_with(|| {
            metadata_string_array(&[
                "design rationale",
                "review notes",
                "migration notes",
                "audit and operational notes",
                "generated explanations",
            ])
        });
}

fn parse_anchor_placement(raw: &str) -> Option<AnchorPlacement> {
    match raw.trim().to_lowercase().as_str() {
        "" => None,
        "standalone" | "line" | "before" => Some(AnchorPlacement::Standalone),
        "inline" | "eol" => Some(AnchorPlacement::Inline),
        _ => None,
    }
}

fn preferred_anchor_placement(
    doc: &LensMapDoc,
    args: Option<&ParsedArgs>,
) -> Option<AnchorPlacement> {
    if let Some(args) = args {
        if let Some(parsed) = args
            .get("anchor-placement")
            .and_then(parse_anchor_placement)
        {
            return Some(parsed);
        }
    }
    doc.metadata
        .get("anchor_placement")
        .and_then(Value::as_str)
        .and_then(parse_anchor_placement)
        .or(Some(AnchorPlacement::Inline))
}

fn make_lensmap_doc(mode: &str, covers: Vec<String>) -> LensMapDoc {
    let ts = now_iso();
    let mut uniq = BTreeMap::new();
    for c in covers {
        let v = c.trim();
        if !v.is_empty() {
            uniq.insert(v.to_string(), true);
        }
    }
    let mut metadata = Map::new();
    apply_default_metadata(&mut metadata);
    LensMapDoc {
        doc_type: "lensmap".to_string(),
        version: "1.0.0".to_string(),
        mode: if mode == "file" {
            "file".to_string()
        } else {
            "group".to_string()
        },
        created_at: ts.clone(),
        updated_at: ts,
        covers: uniq.keys().cloned().collect(),
        anchors: vec![],
        entries: vec![],
        metadata,
    }
}

fn normalize_doc(mut doc: LensMapDoc, fallback_mode: &str) -> LensMapDoc {
    if doc.doc_type.trim().is_empty() {
        doc.doc_type = "lensmap".to_string();
    }
    if doc.version.trim().is_empty() {
        doc.version = "1.0.0".to_string();
    }
    if doc.mode != "file" && doc.mode != "group" {
        doc.mode = if fallback_mode == "file" {
            "file".to_string()
        } else {
            "group".to_string()
        };
    }
    if doc.created_at.trim().is_empty() {
        doc.created_at = now_iso();
    }
    if doc.updated_at.trim().is_empty() {
        doc.updated_at = now_iso();
    }
    let mut uniq = BTreeMap::new();
    for c in &doc.covers {
        let v = c.trim();
        if !v.is_empty() {
            uniq.insert(v.to_string(), true);
        }
    }
    doc.covers = uniq.keys().cloned().collect();
    apply_default_metadata(&mut doc.metadata);
    doc
}

fn resolve_lensmap_path(
    root: &Path,
    args: &ParsedArgs,
    init_project_dir: Option<&Path>,
) -> PathBuf {
    if let Some(raw) = args.get("lensmap") {
        let p = Path::new(raw);
        if p.is_absolute() {
            return normalize_path(p);
        }
        if let Some(project_dir) = init_project_dir {
            if !raw.contains('/') && !raw.contains('\\') {
                return normalize_path(&project_dir.join(raw));
            }
        }
        return normalize_path(&root.join(raw));
    }
    if let Some(project_dir) = init_project_dir {
        return normalize_path(&project_dir.join("lensmap.json"));
    }
    normalize_path(&root.join("lensmap.json"))
}

fn looks_like_placeholder_path(raw: &str) -> bool {
    let v = raw.trim().to_lowercase();
    v == "path/to/lensmap.json"
        || v == "path/to/render.md"
        || v.starts_with("path/to/")
        || v.contains("<path>")
}

fn quickstart_examples() -> Vec<&'static str> {
    vec![
        "lensmap init demo --mode=group --covers=demo/src",
        "lensmap scan --lensmap=demo/lensmap.json",
        "lensmap extract-comments --lensmap=demo/lensmap.json",
        "lensmap validate --lensmap=demo/lensmap.json",
        "lensmap render --lensmap=demo/lensmap.json --out=demo/render.md",
    ]
}

fn lensmap_missing_payload(
    root: &Path,
    action: &str,
    lensmap_path: &Path,
    args: &ParsedArgs,
) -> Value {
    let raw = args.get("lensmap").unwrap_or("");
    let placeholder = looks_like_placeholder_path(raw);
    let mut hint = tr(
        "LensMap file was not found. Run init first or pass a real --lensmap path.",
        "未找到 LensMap 文件。请先运行 init，或传入真实的 --lensmap 路径。",
    );
    if placeholder {
        hint = tr(
            "You passed a placeholder path literally (path/to/...). Replace it with a real path.",
            "你直接传入了占位符路径（path/to/...）。请替换为真实路径。",
        );
    }
    json!({
        "ok": false,
        "type": "lensmap",
        "action": action,
        "error": "lensmap_missing",
        "lensmap": normalize_relative(root, lensmap_path),
        "hint": hint,
        "examples": quickstart_examples(),
    })
}

fn make_package_manifest(root: &Path, bundle_dir: &str) -> PackageManifest {
    let ts = now_iso();
    PackageManifest {
        doc_type: "lensmap_package_manifest".to_string(),
        version: "1.0.0".to_string(),
        root: normalize_relative(root, root),
        bundle_dir: bundle_dir.to_string(),
        created_at: ts.clone(),
        updated_at: ts,
        items: vec![],
    }
}

fn normalize_package_manifest(
    mut manifest: PackageManifest,
    root: &Path,
    bundle_dir: &str,
) -> PackageManifest {
    if manifest.doc_type.trim().is_empty() {
        manifest.doc_type = "lensmap_package_manifest".to_string();
    }
    if manifest.version.trim().is_empty() {
        manifest.version = "1.0.0".to_string();
    }
    if manifest.root.trim().is_empty() {
        manifest.root = normalize_relative(root, root);
    }
    if manifest.bundle_dir.trim().is_empty() {
        manifest.bundle_dir = bundle_dir.to_string();
    }
    if manifest.created_at.trim().is_empty() {
        manifest.created_at = now_iso();
    }
    if manifest.updated_at.trim().is_empty() {
        manifest.updated_at = now_iso();
    }
    manifest
}

fn load_package_manifest(path: &Path, root: &Path, bundle_dir: &str) -> PackageManifest {
    if !path.exists() {
        return make_package_manifest(root, bundle_dir);
    }
    let raw = fs::read_to_string(path).unwrap_or_else(|_| "{}".to_string());
    let parsed = serde_json::from_str::<PackageManifest>(&raw)
        .unwrap_or_else(|_| make_package_manifest(root, bundle_dir));
    normalize_package_manifest(parsed, root, bundle_dir)
}

fn save_package_manifest(
    path: &Path,
    mut manifest: PackageManifest,
    root: &Path,
    bundle_dir: &str,
) {
    manifest.updated_at = now_iso();
    let manifest = normalize_package_manifest(manifest, root, bundle_dir);
    ensure_dir(path);
    let _ = fs::write(
        path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest).unwrap_or_else(|_| "{}".to_string())
        ),
    );
}

fn is_lensmap_filename(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
    name.eq_ignore_ascii_case("lensmap.json")
        || name.ends_with(".lensmap.json")
        || name.eq_ignore_ascii_case("lens_map.json")
}

fn discover_lensmap_files(root: &Path, bundle_dir_rel: &str) -> Vec<String> {
    let bundle_abs = resolve_from_root(root, bundle_dir_rel);
    let mut out = BTreeMap::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = normalize_path(entry.path());
        let rel = normalize_relative(root, &abs);
        if should_skip_rel(&rel) {
            continue;
        }
        if abs.starts_with(&bundle_abs) {
            continue;
        }
        if !is_lensmap_filename(&abs) {
            continue;
        }
        out.insert(rel, true);
    }
    out.keys().cloned().collect()
}

fn parse_dir_map(raw: Option<&str>) -> Vec<(String, String)> {
    let mut out = vec![];
    for part in split_csv(raw) {
        if let Some((src, dst)) = part.split_once('=') {
            let left = src.trim().trim_end_matches('/').to_string();
            let right = dst.trim().trim_end_matches('/').to_string();
            if !left.is_empty() && !right.is_empty() {
                out.push((left, right));
            }
        }
    }
    out
}

fn copy_or_move_file(src: &Path, dst: &Path, keep_source: bool) -> Result<(), String> {
    ensure_dir(dst);
    fs::copy(src, dst).map_err(|e| {
        format!(
            "copy_failed:{}->{}:{}",
            to_posix_str(src),
            to_posix_str(dst),
            e
        )
    })?;
    if !keep_source {
        fs::remove_file(src)
            .map_err(|e| format!("remove_source_failed:{}:{}", to_posix_str(src), e))?;
    }
    Ok(())
}

fn prompt_line(prompt: &str) -> Option<String> {
    if !io::stdin().is_terminal() {
        return None;
    }
    let mut stderr = io::stderr();
    let _ = write!(stderr, "{}", prompt);
    let _ = stderr.flush();
    let mut line = String::new();
    io::stdin().read_line(&mut line).ok()?;
    Some(line.trim().to_string())
}

fn load_doc(path: &Path, fallback_mode: &str) -> LensMapDoc {
    if !path.exists() {
        return make_lensmap_doc(fallback_mode, vec![]);
    }
    let raw = fs::read_to_string(path).unwrap_or_else(|_| "{}".to_string());
    let parsed = serde_json::from_str::<LensMapDoc>(&raw)
        .unwrap_or_else(|_| make_lensmap_doc(fallback_mode, vec![]));
    normalize_doc(parsed, fallback_mode)
}

fn save_doc(path: &Path, mut doc: LensMapDoc) {
    doc.updated_at = now_iso();
    let mode = doc.mode.clone();
    let doc = normalize_doc(doc, &mode);
    ensure_dir(path);
    let _ = fs::write(
        path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_string())
        ),
    );
}

fn git_output(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Some(stdout)
}

fn git_diff_text(root: &Path, rel: &str) -> Option<String> {
    let with_head = git_output(root, &["diff", "--unified=0", "HEAD", "--", rel]);
    if with_head.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
        return with_head;
    }
    let working = git_output(root, &["diff", "--unified=0", "--", rel]);
    if working.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
        return working;
    }
    None
}

fn parse_diff_hunks(diff: &str) -> Vec<GitHunk> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@").unwrap());
    diff.lines()
        .filter_map(|line| {
            let cap = re.captures(line)?;
            Some(GitHunk {
                old_start: cap.get(1)?.as_str().parse::<usize>().ok()?,
                old_count: cap
                    .get(2)
                    .and_then(|m| m.as_str().parse::<usize>().ok())
                    .unwrap_or(1),
                new_start: cap.get(3)?.as_str().parse::<usize>().ok()?,
                new_count: cap
                    .get(4)
                    .and_then(|m| m.as_str().parse::<usize>().ok())
                    .unwrap_or(1),
            })
        })
        .collect()
}

fn git_dirty_hunks(root: &Path, rel: &str) -> Vec<GitHunk> {
    git_diff_text(root, rel)
        .map(|text| parse_diff_hunks(&text))
        .unwrap_or_default()
}

fn git_dirty_ranges(root: &Path, rel: &str) -> Vec<(usize, usize)> {
    git_dirty_hunks(root, rel)
        .into_iter()
        .map(|hunk| {
            let start = hunk.new_start.max(1);
            let len = hunk.new_count.max(1);
            let end = start + len.saturating_sub(1);
            (start, end)
        })
        .collect()
}

fn git_is_dirty(root: &Path, rel: &str) -> bool {
    !git_dirty_hunks(root, rel).is_empty()
}

fn git_project_line(root: &Path, rel: &str, old_line: usize) -> Option<usize> {
    if old_line == 0 {
        return None;
    }
    let hunks = git_dirty_hunks(root, rel);
    if hunks.is_empty() {
        return None;
    }
    project_line_from_hunks(&hunks, old_line)
}

fn project_line_from_hunks(hunks: &[GitHunk], old_line: usize) -> Option<usize> {
    if old_line == 0 {
        return None;
    }
    if hunks.is_empty() {
        return Some(old_line);
    }

    let mut delta: isize = 0;
    for hunk in hunks {
        if old_line < hunk.old_start {
            return Some((old_line as isize + delta).max(1) as usize);
        }

        let old_end = hunk.old_start + hunk.old_count.saturating_sub(1);
        if hunk.old_count > 0 && old_line <= old_end {
            let offset = old_line.saturating_sub(hunk.old_start);
            let mapped = hunk.new_start + offset.min(hunk.new_count.saturating_sub(1));
            return Some(mapped.max(1));
        }

        delta += hunk.new_count as isize - hunk.old_count as isize;
    }

    Some((old_line as isize + delta).max(1) as usize)
}

fn git_projected_function_hit(
    root: &Path,
    rel: &str,
    anchor: &AnchorRecord,
    functions: &[FunctionHit],
) -> Option<FunctionHit> {
    let original_line = anchor
        .line_symbol
        .or(anchor.span_start)
        .or(anchor.line_anchor)?;
    let projected_line = git_project_line(root, rel, original_line)?;
    let candidate = functions
        .iter()
        .min_by_key(|f| (f.line_index + 1).abs_diff(projected_line))
        .cloned()?;
    if (candidate.line_index + 1).abs_diff(projected_line) <= 6 {
        return Some(candidate);
    }
    None
}

fn line_overlaps_dirty_ranges(line: usize, ranges: &[(usize, usize)]) -> bool {
    ranges
        .iter()
        .any(|(start, end)| line >= *start && line <= *end)
}

fn anchor_focus_lines(anchor: &AnchorRecord) -> Vec<usize> {
    let mut lines = vec![];
    if let (Some(start), Some(end)) = (anchor.span_start, anchor.span_end) {
        for line in start..=end {
            lines.push(line);
        }
        return lines;
    }
    if let Some(line) = anchor.line_symbol {
        lines.push(line);
    }
    if let Some(line) = anchor.line_anchor {
        lines.push(line);
    }
    lines
}

fn anchor_overlaps_dirty_ranges(anchor: &AnchorRecord, ranges: &[(usize, usize)]) -> bool {
    anchor_focus_lines(anchor)
        .into_iter()
        .any(|line| line_overlaps_dirty_ranges(line, ranges))
}

fn default_index_path(root: &Path) -> PathBuf {
    root.join(".lensmap-index.json")
}

fn make_index_doc(
    root: &Path,
    lensmaps: Vec<String>,
    mut entries: Vec<SearchEntryRecord>,
) -> LensMapIndexDoc {
    entries.sort_by(|a, b| {
        if a.file != b.file {
            return a.file.cmp(&b.file);
        }
        a.start_line
            .unwrap_or(0)
            .cmp(&b.start_line.unwrap_or(0))
            .then_with(|| a.ref_id.cmp(&b.ref_id))
    });
    LensMapIndexDoc {
        doc_type: "lensmap-index".to_string(),
        version: "1.0.0".to_string(),
        root: normalize_relative(root, root),
        generated_at: now_iso(),
        lensmaps,
        entries,
    }
}

fn load_index_doc(path: &Path) -> LensMapIndexDoc {
    if !path.exists() {
        return LensMapIndexDoc::default();
    }
    let raw = fs::read_to_string(path).unwrap_or_else(|_| "{}".to_string());
    serde_json::from_str::<LensMapIndexDoc>(&raw).unwrap_or_default()
}

fn save_index_doc(path: &Path, doc: &LensMapIndexDoc) {
    ensure_dir(path);
    let _ = fs::write(
        path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(doc).unwrap_or_else(|_| "{}".to_string())
        ),
    );
}

fn append_history(root: &Path, row: &Value) {
    let history_path = root.join("local/state/ops/lensmap/history.jsonl");
    ensure_dir(&history_path);
    let mut existing = fs::read_to_string(&history_path).unwrap_or_default();
    existing.push_str(&serde_json::to_string(row).unwrap_or_else(|_| "{}".to_string()));
    existing.push('\n');
    let _ = fs::write(&history_path, existing);
}

fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|v| v.to_str())
        .map(|v| format!(".{}", v.to_lowercase()))
        .unwrap_or_default()
}

fn comment_style_for(path: &Path) -> CommentStyle {
    if ext_of(path) == ".py" {
        CommentStyle {
            line: "#",
            block_start: None,
            block_end: None,
        }
    } else {
        CommentStyle {
            line: "//",
            block_start: Some("/*"),
            block_end: Some("*/"),
        }
    }
}

fn should_skip_rel(rel: &str) -> bool {
    let mut with_slash = rel.trim_start_matches("./").to_string();
    if !with_slash.ends_with('/') {
        with_slash.push('/');
    }
    SKIP_PREFIXES.iter().any(|p| with_slash.starts_with(p))
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let mut dp = vec![vec![false; t.len() + 1]; p.len() + 1];
    dp[0][0] = true;
    for i in 1..=p.len() {
        if p[i - 1] == '*' {
            dp[i][0] = dp[i - 1][0];
        }
    }
    for i in 1..=p.len() {
        for j in 1..=t.len() {
            if p[i - 1] == '*' {
                dp[i][j] = dp[i - 1][j] || dp[i][j - 1];
            } else if p[i - 1] == '?' || p[i - 1] == t[j - 1] {
                dp[i][j] = dp[i - 1][j - 1];
            }
        }
    }
    dp[p.len()][t.len()]
}

fn collect_supported_files(root: &Path, start: &Path, out: &mut BTreeMap<String, bool>) {
    if !start.exists() {
        return;
    }
    for entry in WalkDir::new(start)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = normalize_path(entry.path());
        let rel = normalize_relative(root, &path);
        if should_skip_rel(&rel) {
            continue;
        }
        let ext = ext_of(&path);
        if !SUPPORTED_EXTS.contains(&ext.as_str()) {
            continue;
        }
        out.insert(rel, true);
    }
}

fn all_supported_files(root: &Path) -> Vec<String> {
    let mut out = BTreeMap::new();
    collect_supported_files(root, root, &mut out);
    out.keys().cloned().collect()
}

fn resolve_covers_to_files(root: &Path, covers: &[String]) -> Vec<String> {
    let mut out = BTreeMap::new();
    let all = all_supported_files(root);

    for raw in covers {
        let item = raw.trim();
        if item.is_empty() {
            continue;
        }
        if item.contains('*') || item.contains('?') {
            for rel in &all {
                if wildcard_match(item, rel) {
                    out.insert(rel.clone(), true);
                }
            }
            continue;
        }

        let abs = resolve_from_root(root, item);
        if !is_within_root(root, &abs) || !abs.exists() {
            continue;
        }

        if abs.is_file() {
            let ext = ext_of(&abs);
            if SUPPORTED_EXTS.contains(&ext.as_str()) {
                out.insert(normalize_relative(root, &abs), true);
            }
            continue;
        }

        if abs.is_dir() {
            collect_supported_files(root, &abs, &mut out);
        }
    }

    out.keys().cloned().collect()
}

fn split_lines(content: &str) -> Vec<String> {
    content
        .replace('\r', "")
        .split('\n')
        .map(|s| s.to_string())
        .collect()
}

fn join_lines(lines: &[String]) -> String {
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn js_fn_decl_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+([A-Za-z_$][\w$]*)\s*\(",
        )
        .unwrap()
    })
}

fn js_assign_fn_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:async\s*)?(?:function\b|\([^)]*\)\s*(?::\s*[^=]+)?\s*=>|[A-Za-z_$][\w$]*(?:\s*:\s*[^=]+)?\s*=>)").unwrap()
    })
}

fn js_method_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\s*(?:public\s+|private\s+|protected\s+|static\s+|readonly\s+|async\s+)*([A-Za-z_$][\w$]*)\s*\([^;]*\)\s*\{").unwrap()
    })
}

fn py_fn_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*(?:async\s+)?def\s+([A-Za-z_][\w]*)\s*\(").unwrap())
}

fn rs_fn_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_][\w]*)\s*\(").unwrap()
    })
}

fn go_fn_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*func\s*(?:\([^)]*\)\s*)?([A-Za-z_][\w]*)\s*\(").unwrap())
}

fn java_fn_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\s*(?:public\s+|private\s+|protected\s+|static\s+|final\s+|native\s+|synchronized\s+|abstract\s+|default\s+|strictfp\s+)*(?:<[^>]+>\s+)?[A-Za-z_][\w<>\[\], ?]*\s+([A-Za-z_][\w]*)\s*\([^;]*\)\s*\{").unwrap()
    })
}

fn java_ctor_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\s*(?:public\s+|private\s+|protected\s+)([A-Za-z_][\w]*)\s*\([^;]*\)\s*\{")
            .unwrap()
    })
}

fn c_family_fn_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^\s*(?:template\s*<[^>]+>\s*)?(?:[\w:&*<>\[\]~]+\s+)+([A-Za-z_~][\w:~]*)\s*\([^;]*\)\s*(?:const\s*)?(?:noexcept\s*)?\{",
        )
        .unwrap()
    })
}

fn csharp_fn_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^\s*(?:public\s+|private\s+|protected\s+|internal\s+|static\s+|virtual\s+|override\s+|sealed\s+|async\s+|partial\s+|unsafe\s+|new\s+)*(?:<[^>]+>\s+)?[A-Za-z_][\w<>\[\], ?]*\s+([A-Za-z_][\w]*)\s*\([^;]*\)\s*(?:=>|\{)",
        )
        .unwrap()
    })
}

fn csharp_ctor_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^\s*(?:public\s+|private\s+|protected\s+|internal\s+)([A-Za-z_][\w]*)\s*\([^;]*\)\s*(?:=>|\{)",
        )
        .unwrap()
    })
}

fn kotlin_fn_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^\s*(?:(?:public|private|protected|internal|open|override|suspend|inline|tailrec|operator|infix|external|abstract|final|actual|expect|lateinit|data|enum|sealed|value)\s+)*fun\s+([A-Za-z_][\w]*)\s*\(",
        )
        .unwrap()
    })
}

fn kotlin_ctor_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\s*(?:(?:public|private|protected|internal)\s+)?constructor\s*\(").unwrap()
    })
}

fn namespace_prefix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*namespace\s+([A-Za-z_][A-Za-z0-9_.]*)\s*[;{]").unwrap())
}

fn package_prefix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*package\s+([A-Za-z_][A-Za-z0-9_.]*)\b").unwrap())
}

fn line_indent(lines: &[String], line_index: usize) -> String {
    lines
        .get(line_index)
        .map(|line| line.chars().take_while(|c| c.is_whitespace()).collect())
        .unwrap_or_default()
}

fn normalize_fingerprint_text(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn signature_text_from_source(raw: &str) -> String {
    raw.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(normalize_fingerprint_text)
        .unwrap_or_else(|| normalize_fingerprint_text(raw))
}

fn node_text(node: Node<'_>, source: &[u8]) -> Option<String> {
    let text = node.utf8_text(source).ok()?.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn path_with_symbol(scope: &[String], symbol: &str) -> String {
    let mut parts = scope.to_vec();
    parts.push(symbol.to_string());
    parts.join(".")
}

fn go_receiver_type_name(raw: &str) -> Option<String> {
    let trimmed = raw
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();
    if trimmed.is_empty() {
        return None;
    }
    let candidate = trimmed.split_whitespace().last().unwrap_or("").trim();
    if candidate.is_empty() {
        return None;
    }
    let candidate = candidate.trim_start_matches('*');
    let candidate = candidate.trim_start_matches("[]");
    let candidate = candidate.split('[').next().unwrap_or(candidate);
    let candidate = candidate.rsplit('.').next().unwrap_or(candidate);
    if candidate.is_empty() {
        None
    } else {
        Some(candidate.to_string())
    }
}

fn strip_generic_segments(raw: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for ch in raw.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

fn c_family_symbol_from_declarator(raw: &str, scope: &[String]) -> Option<(String, String)> {
    let prefix = raw.split('(').next()?.trim();
    if prefix.is_empty() {
        return None;
    }
    let token = prefix
        .split_whitespace()
        .last()
        .unwrap_or("")
        .trim_matches(|c: char| c == '*' || c == '&' || c == '(' || c == ')');
    if token.is_empty() {
        return None;
    }
    let cleaned = strip_generic_segments(token)
        .trim_matches(|c: char| c == '*' || c == '&' || c == ':')
        .to_string();
    if cleaned.is_empty() {
        return None;
    }
    if cleaned.contains("::") {
        let parts = cleaned
            .split("::")
            .filter(|part| !part.trim().is_empty())
            .map(|part| part.trim().to_string())
            .collect::<Vec<_>>();
        let symbol = parts.last()?.clone();
        return Some((symbol, parts.join(".")));
    }
    let symbol = cleaned;
    Some((symbol.clone(), path_with_symbol(scope, &symbol)))
}

fn file_scope_prefix(lines: &[String], ext: &str) -> Option<String> {
    if ext == ".cs" {
        for line in lines {
            if let Some(captures) = namespace_prefix_re().captures(line) {
                return captures.get(1).map(|m| m.as_str().to_string());
            }
        }
    }
    if ext == ".kt" || ext == ".kts" {
        for line in lines {
            if let Some(captures) = package_prefix_re().captures(line) {
                return captures.get(1).map(|m| m.as_str().to_string());
            }
        }
    }
    None
}

fn ast_scope_name(node: Node<'_>, source: &[u8], ext: &str) -> Option<String> {
    match ext {
        ".js" | ".jsx" | ".ts" | ".tsx" | ".mjs" | ".cjs" => match node.kind() {
            "class_declaration"
            | "class"
            | "function_declaration"
            | "method_definition"
            | "public_field_definition"
            | "variable_declarator" => node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source)),
            _ => None,
        },
        ".py" => match node.kind() {
            "class_definition" | "function_definition" | "async_function_definition" => node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source)),
            _ => None,
        },
        ".rs" => match node.kind() {
            "function_item" => node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source)),
            "impl_item" => node
                .child_by_field_name("type")
                .and_then(|child| node_text(child, source)),
            _ => None,
        },
        ".java" => match node.kind() {
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "annotation_type_declaration" => node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source)),
            _ => None,
        },
        ".cc" | ".cpp" | ".cxx" | ".hh" | ".hpp" | ".hxx" => match node.kind() {
            "namespace_definition" | "class_specifier" | "struct_specifier" => node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source))
                .map(|name| strip_generic_segments(&name)),
            _ => None,
        },
        ".cs" => match node.kind() {
            "namespace_declaration"
            | "file_scoped_namespace_declaration"
            | "class_declaration"
            | "struct_declaration"
            | "interface_declaration"
            | "record_declaration"
            | "enum_declaration" => node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source)),
            _ => None,
        },
        ".kt" | ".kts" => match node.kind() {
            "class_declaration" | "object_declaration" => node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source)),
            _ => None,
        },
        _ => None,
    }
}

fn ast_function_hit(
    node: Node<'_>,
    source: &[u8],
    lines: &[String],
    ext: &str,
    scope: &[String],
) -> Option<FunctionHit> {
    let symbol = match ext {
        ".js" | ".jsx" | ".ts" | ".tsx" | ".mjs" | ".cjs" => match node.kind() {
            "function_declaration" | "method_definition" | "public_field_definition" => node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source)),
            "variable_declarator" => {
                let value = node.child_by_field_name("value")?;
                if !["function", "arrow_function"].contains(&value.kind()) {
                    return None;
                }
                node.child_by_field_name("name")
                    .and_then(|child| node_text(child, source))
            }
            "pair" => {
                let value = node.child_by_field_name("value")?;
                if !["function", "arrow_function"].contains(&value.kind()) {
                    return None;
                }
                node.child_by_field_name("key")
                    .and_then(|child| node_text(child, source))
            }
            _ => None,
        },
        ".py" => match node.kind() {
            "function_definition" | "async_function_definition" => node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source)),
            _ => None,
        },
        ".rs" => match node.kind() {
            "function_item" => node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source)),
            _ => None,
        },
        ".go" => match node.kind() {
            "function_declaration" | "method_declaration" => node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source)),
            _ => None,
        },
        ".java" => match node.kind() {
            "method_declaration" | "constructor_declaration" => node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source)),
            _ => None,
        },
        ".c" | ".h" | ".cc" | ".cpp" | ".cxx" | ".hh" | ".hpp" | ".hxx" => {
            if node.kind() != "function_definition" {
                None
            } else {
                let declarator = node.child_by_field_name("declarator")?;
                let raw = node_text(declarator, source)?;
                c_family_symbol_from_declarator(&raw, scope).map(|(symbol, _)| symbol)
            }
        }
        ".cs" => match node.kind() {
            "method_declaration" | "constructor_declaration" => node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source)),
            _ => None,
        },
        ".kt" | ".kts" => match node.kind() {
            "function_declaration" => node
                .child_by_field_name("name")
                .and_then(|child| node_text(child, source)),
            "secondary_constructor" => scope.last().cloned(),
            _ => None,
        },
        _ => None,
    }?;

    let text = node_text(node, source)?;
    let normalized = normalize_fingerprint_text(&text);
    let start_line = node.start_position().row;
    let end_line = node.end_position().row.max(start_line);
    let symbol_path = if ext == ".go" && node.kind() == "method_declaration" {
        if let Some(receiver) = node
            .child_by_field_name("receiver")
            .and_then(|child| node_text(child, source))
        {
            if let Some(receiver_name) = go_receiver_type_name(&receiver) {
                format!("{}.{}", receiver_name, symbol)
            } else {
                path_with_symbol(scope, &symbol)
            }
        } else {
            path_with_symbol(scope, &symbol)
        }
    } else if [".c", ".h", ".cc", ".cpp", ".cxx", ".hh", ".hpp", ".hxx"].contains(&ext)
        && node.kind() == "function_definition"
    {
        if let Some(declarator) = node
            .child_by_field_name("declarator")
            .and_then(|child| node_text(child, source))
        {
            if let Some((_resolved_symbol, resolved_path)) =
                c_family_symbol_from_declarator(&declarator, scope)
            {
                resolved_path
            } else {
                path_with_symbol(scope, &symbol)
            }
        } else {
            path_with_symbol(scope, &symbol)
        }
    } else {
        path_with_symbol(scope, &symbol)
    };
    Some(FunctionHit {
        line_index: start_line,
        span_end_index: end_line,
        symbol: symbol.clone(),
        symbol_path,
        indent: line_indent(lines, start_line),
        fingerprint: hash_text(&normalized),
        signature_text: signature_text_from_source(&text),
    })
}

fn parser_for_extension(ext: &str) -> Option<Parser> {
    let mut parser = Parser::new();
    let language = match ext {
        ".js" | ".jsx" | ".mjs" | ".cjs" => tree_sitter_javascript::LANGUAGE.into(),
        ".ts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        ".tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        ".py" => tree_sitter_python::LANGUAGE.into(),
        ".rs" => tree_sitter_rust::LANGUAGE.into(),
        ".go" => tree_sitter_go::LANGUAGE.into(),
        ".java" => tree_sitter_java::LANGUAGE.into(),
        ".c" | ".h" => tree_sitter_c::LANGUAGE.into(),
        ".cc" | ".cpp" | ".cxx" | ".hh" | ".hpp" | ".hxx" => tree_sitter_cpp::LANGUAGE.into(),
        ".cs" => tree_sitter_c_sharp::LANGUAGE.into(),
        ".kt" | ".kts" => tree_sitter_kotlin_ng::LANGUAGE.into(),
        _ => return None,
    };
    parser.set_language(&language).ok()?;
    Some(parser)
}

fn collect_ast_functions(
    node: Node<'_>,
    source: &[u8],
    lines: &[String],
    ext: &str,
    scope: &mut Vec<String>,
    hits: &mut Vec<FunctionHit>,
) {
    if let Some(hit) = ast_function_hit(node, source, lines, ext, scope) {
        hits.push(hit);
    }

    let pushed_scope = ast_scope_name(node, source, ext);
    if let Some(name) = &pushed_scope {
        scope.push(name.clone());
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_ast_functions(child, source, lines, ext, scope, hits);
    }

    if pushed_scope.is_some() {
        let _ = scope.pop();
    }
}

fn detect_functions_ast(lines: &[String], abs_file: &Path) -> Vec<FunctionHit> {
    let ext = ext_of(abs_file);
    let mut parser = if let Some(parser) = parser_for_extension(&ext) {
        parser
    } else {
        return vec![];
    };
    let source = join_lines(lines);
    let tree = if let Some(tree) = parser.parse(&source, None) {
        tree
    } else {
        return vec![];
    };
    let mut scope = vec![];
    let mut hits = vec![];
    collect_ast_functions(
        tree.root_node(),
        source.as_bytes(),
        lines,
        &ext,
        &mut scope,
        &mut hits,
    );
    if let Some(prefix) = file_scope_prefix(lines, &ext) {
        for hit in &mut hits {
            if hit.symbol_path != prefix && !hit.symbol_path.starts_with(&format!("{}.", prefix)) {
                hit.symbol_path = format!("{}.{}", prefix, hit.symbol_path);
            }
        }
    }
    hits.sort_by(|a, b| {
        if a.line_index != b.line_index {
            return a.line_index.cmp(&b.line_index);
        }
        a.symbol_path.cmp(&b.symbol_path)
    });
    let mut deduped = vec![];
    let mut seen = HashSet::new();
    for hit in hits {
        let key = format!(
            "{}:{}:{}",
            hit.line_index, hit.span_end_index, hit.symbol_path
        );
        if seen.insert(key) {
            deduped.push(hit);
        }
    }
    deduped
}

fn detect_functions_regex(lines: &[String], abs_file: &Path) -> Vec<FunctionHit> {
    let ext = ext_of(abs_file);
    let mut out = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let mut symbol: Option<String> = None;

        if [".js", ".ts", ".tsx", ".jsx", ".mjs", ".cjs"].contains(&ext.as_str()) {
            if let Some(c) = js_fn_decl_re().captures(line) {
                symbol = c.get(1).map(|m| m.as_str().to_string());
            }
            if symbol.is_none() {
                if let Some(c) = js_assign_fn_re().captures(line) {
                    symbol = c.get(1).map(|m| m.as_str().to_string());
                }
            }
            if symbol.is_none() {
                if let Some(c) = js_method_re().captures(line) {
                    let s = c.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
                    let kw = s.to_lowercase();
                    if ![
                        "if", "for", "while", "switch", "catch", "return", "function",
                    ]
                    .contains(&kw.as_str())
                    {
                        symbol = Some(s);
                    }
                }
            }
        } else if ext == ".py" {
            if let Some(c) = py_fn_re().captures(line) {
                symbol = c.get(1).map(|m| m.as_str().to_string());
            }
        } else if ext == ".rs" {
            if let Some(c) = rs_fn_re().captures(line) {
                symbol = c.get(1).map(|m| m.as_str().to_string());
            }
        } else if ext == ".go" {
            if let Some(c) = go_fn_re().captures(line) {
                symbol = c.get(1).map(|m| m.as_str().to_string());
            }
        } else if ext == ".java" {
            if let Some(c) = java_fn_re().captures(line) {
                symbol = c.get(1).map(|m| m.as_str().to_string());
            }
            if symbol.is_none() {
                if let Some(c) = java_ctor_re().captures(line) {
                    symbol = c.get(1).map(|m| m.as_str().to_string());
                }
            }
        } else if [".c", ".h", ".cc", ".cpp", ".cxx", ".hh", ".hpp", ".hxx"].contains(&ext.as_str())
        {
            if let Some(c) = c_family_fn_re().captures(line) {
                symbol = c.get(1).map(|m| {
                    m.as_str()
                        .split("::")
                        .last()
                        .unwrap_or(m.as_str())
                        .to_string()
                });
            }
        } else if ext == ".cs" {
            if let Some(c) = csharp_fn_re().captures(line) {
                symbol = c.get(1).map(|m| m.as_str().to_string());
            }
            if symbol.is_none() {
                if let Some(c) = csharp_ctor_re().captures(line) {
                    symbol = c.get(1).map(|m| m.as_str().to_string());
                }
            }
        } else if ext == ".kt" || ext == ".kts" {
            if let Some(c) = kotlin_fn_re().captures(line) {
                symbol = c.get(1).map(|m| m.as_str().to_string());
            }
            if symbol.is_none() && kotlin_ctor_re().is_match(line) {
                symbol = Some("constructor".to_string());
            }
        }

        if let Some(symbol) = symbol {
            let indent = line_indent(lines, idx);
            let normalized = normalize_fingerprint_text(line);
            let fingerprint = hash_text(&normalized);
            out.push(FunctionHit {
                line_index: idx,
                span_end_index: idx,
                symbol: symbol.clone(),
                symbol_path: symbol,
                indent,
                fingerprint,
                signature_text: signature_text_from_source(line),
            });
        }
    }

    out
}

fn detect_functions(lines: &[String], abs_file: &Path) -> Vec<FunctionHit> {
    let ast_hits = detect_functions_ast(lines, abs_file);
    if !ast_hits.is_empty() {
        return ast_hits;
    }
    let mut regex_hits = detect_functions_regex(lines, abs_file);
    for hit in &mut regex_hits {
        if hit.symbol_path.is_empty() {
            hit.symbol_path = hit.symbol.clone();
        }
    }
    regex_hits
}

fn parse_anchor_marker_comment(comment: &str) -> Option<AnchorLineMatch> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"^\s*(//|#)\s*@lensmap-anchor\s+([A-Fa-f0-9]{6,16})\b").unwrap()
    });
    let cap = re.captures(comment)?;
    Some(AnchorLineMatch {
        marker: cap.get(1)?.as_str().to_string(),
        id: cap.get(2)?.as_str().to_uppercase(),
        is_inline: false,
    })
}

fn parse_anchor_line(line: &str) -> Option<AnchorLineMatch> {
    if let Some(parsed) = parse_anchor_marker_comment(line) {
        return Some(parsed);
    }
    for marker in ["//", "#"] {
        let Some(idx) = find_line_comment_index_outside_strings(line, marker) else {
            continue;
        };
        let prefix_raw = &line[..idx];
        if prefix_raw.trim().is_empty() {
            continue;
        }
        let comment = &line[idx..];
        let mut parsed = parse_anchor_marker_comment(comment)?;
        parsed.is_inline = true;
        return Some(parsed);
    }
    None
}

fn anchor_match(line: &str, expected_marker: Option<&str>) -> Option<AnchorLineMatch> {
    let m = parse_anchor_line(line)?;
    if let Some(marker) = expected_marker {
        if m.marker != marker {
            return None;
        }
    }
    Some(m)
}

fn parse_ref_in_line(line: &str) -> Option<RefLineMatch> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(//|#)\s*@lensmap-ref\s+([A-Fa-f0-9]{6,16}-\d+(?:-\d+)?)\b").unwrap()
    });
    let cap = re.captures(line)?;
    Some(RefLineMatch {
        marker: cap.get(1)?.as_str().to_string(),
        ref_id: cap.get(2)?.as_str().to_uppercase(),
    })
}

fn ref_match(line: &str, expected_marker: Option<&str>) -> Option<RefLineMatch> {
    let m = parse_ref_in_line(line)?;
    if let Some(marker) = expected_marker {
        if m.marker != marker {
            return None;
        }
    }
    Some(m)
}

fn collect_anchor_nodes(
    lines: &[String],
    expected_marker: Option<&str>,
) -> Vec<(String, String, usize, bool)> {
    let mut out = vec![];
    for (i, line) in lines.iter().enumerate() {
        if let Some(m) = anchor_match(line, expected_marker) {
            out.push((m.id, m.marker, i, m.is_inline));
        }
    }
    out
}

fn is_comment_like_line(trimmed: &str, style: &CommentStyle) -> bool {
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.starts_with(style.line) {
        return true;
    }
    if let Some(bs) = style.block_start {
        if trimmed.starts_with(bs) {
            return true;
        }
    }
    if let Some(be) = style.block_end {
        if trimmed.starts_with(be) {
            return true;
        }
    }
    trimmed.starts_with('*')
}

fn find_anchor_before_function(
    lines: &[String],
    fn_line: usize,
    style: &CommentStyle,
) -> Option<(String, String, usize, bool)> {
    if fn_line < lines.len() {
        if let Some(m) = anchor_match(&lines[fn_line], None) {
            if m.is_inline {
                return Some((m.id, m.marker, fn_line, true));
            }
        }
    }
    let mut i = fn_line;
    while i > 0 {
        i -= 1;
        let trimmed = lines[i].trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(m) = anchor_match(&lines[i], None) {
            return Some((m.id, m.marker, i, m.is_inline));
        }
        if is_comment_like_line(trimmed, style) {
            continue;
        }
        return None;
    }
    None
}

fn compute_anchor_insert_index(lines: &[String], fn_line: usize, style: &CommentStyle) -> usize {
    let mut idx = fn_line;
    let mut saw_comment = false;
    let mut i = fn_line;
    while i > 0 {
        i -= 1;
        let trimmed = lines[i].trim();
        if trimmed.is_empty() {
            if saw_comment {
                idx = i;
                continue;
            }
            break;
        }
        if is_comment_like_line(trimmed, style) {
            saw_comment = true;
            idx = i;
            continue;
        }
        break;
    }
    idx
}

fn has_anchor_in_range(lines: &[String], start: usize, end: usize) -> bool {
    if lines.is_empty() {
        return false;
    }
    let s = start.min(lines.len() - 1);
    let e = end.min(lines.len() - 1);
    if s > e {
        return false;
    }
    for line in lines.iter().take(e + 1).skip(s) {
        if anchor_match(line, None).is_some() {
            return true;
        }
    }
    false
}

fn make_anchor_line(indent: &str, marker: &str, id: &str) -> String {
    format!("{}{} {} {}", indent, marker, ANCHOR_TAG, id)
}

fn make_inline_anchor_line(line: &str, marker: &str, id: &str) -> String {
    format!("{} {} {} {}", line.trim_end(), marker, ANCHOR_TAG, id)
}

fn can_place_inline_anchor(line: &str, marker: &str) -> bool {
    if anchor_match(line, None).is_some() {
        return false;
    }
    if ref_match(line, None).is_some() {
        return false;
    }
    find_line_comment_index_outside_strings(line, marker).is_none()
}

fn rewrite_anchor_on_line(line: &str, expected_marker: &str, id: &str) -> String {
    if let Some(parsed) = parse_anchor_line(line) {
        if parsed.is_inline {
            if let Some(idx) = find_line_comment_index_outside_strings(line, &parsed.marker) {
                return make_inline_anchor_line(&line[..idx], expected_marker, id);
            }
        }
    }
    let indent = line
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect::<String>();
    make_anchor_line(&indent, expected_marker, id)
}

fn place_anchor_for_function(
    lines: &mut Vec<String>,
    fn_hit: &FunctionHit,
    style: &CommentStyle,
    id: &str,
    placement: AnchorPlacement,
) -> usize {
    if placement == AnchorPlacement::Inline
        && fn_hit.line_index < lines.len()
        && can_place_inline_anchor(&lines[fn_hit.line_index], style.line)
    {
        lines[fn_hit.line_index] =
            make_inline_anchor_line(&lines[fn_hit.line_index], style.line, id);
        return fn_hit.line_index;
    }

    let insert_at = compute_anchor_insert_index(lines, fn_hit.line_index, style);
    lines.insert(insert_at, make_anchor_line(&fn_hit.indent, style.line, id));
    insert_at
}

fn make_ref_line(indent: &str, marker: &str, ref_id: &str) -> String {
    format!("{}{} {} {}", indent, marker, REF_TAG, ref_id)
}

fn generate_anchor_id(existing: &HashSet<String>) -> String {
    for i in 0..1024 {
        let seed = format!(
            "{}:{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
            i
        );
        let mut hasher = Sha256::new();
        hasher.update(seed.as_bytes());
        let hex = hex::encode(hasher.finalize());
        let id = hex[..6].to_uppercase();
        if !existing.contains(&id) {
            return id;
        }
    }
    "AAAAAA".to_string()
}

fn materialize_anchors_for_file(root: &Path, abs: &Path, lines: &[String]) -> Vec<AnchorRecord> {
    let rel = normalize_relative(root, abs);
    let style = comment_style_for(abs);
    let functions = detect_functions(lines, abs);
    let anchors = collect_anchor_nodes(lines, Some(style.line));
    let mut out = vec![];

    for (id, _marker, line_idx, is_inline) in anchors {
        let fn_hit = if is_inline {
            functions.iter().find(|f| f.line_index == line_idx)
        } else {
            functions.iter().find(|f| f.line_index > line_idx)
        };
        out.push(AnchorRecord {
            id,
            file: rel.clone(),
            symbol: fn_hit.map(|f| f.symbol.clone()),
            symbol_path: fn_hit.map(|f| f.symbol_path.clone()),
            line_anchor: Some(line_idx + 1),
            line_symbol: fn_hit.map(|f| f.line_index + 1),
            span_start: fn_hit.map(|f| f.line_index + 1),
            span_end: fn_hit.map(|f| f.span_end_index + 1),
            fingerprint: fn_hit.map(|f| f.fingerprint.clone()),
            signature_text: fn_hit.map(|f| f.signature_text.clone()),
            placement: Some(if is_inline {
                "inline".to_string()
            } else {
                "standalone".to_string()
            }),
            updated_at: Some(now_iso()),
        });
    }

    out
}

fn replace_doc_anchors_for_file(doc: &mut LensMapDoc, rel: &str, anchors: Vec<AnchorRecord>) {
    doc.anchors.retain(|anchor| anchor.file != rel);
    doc.anchors.extend(anchors);
    doc.anchors.sort_by(|a, b| {
        if a.file != b.file {
            return a.file.cmp(&b.file);
        }
        a.line_anchor.unwrap_or(0).cmp(&b.line_anchor.unwrap_or(0))
    });
}

fn ensure_anchor_for_symbol(
    root: &Path,
    doc: &mut LensMapDoc,
    file: &str,
    symbol: &str,
    symbol_path: Option<&str>,
) -> Result<(AnchorRecord, bool), String> {
    let abs = resolve_from_root(root, file);
    if !is_within_root(root, &abs) {
        return Err(format!("security_entry_outside_root:{}", file));
    }
    if !abs.exists() {
        return Err(format!("file_missing:{}", file));
    }

    let style = comment_style_for(&abs);
    let placement = preferred_anchor_placement(doc, None).unwrap_or(AnchorPlacement::Inline);
    let mut lines = split_lines(&fs::read_to_string(&abs).unwrap_or_default());
    let functions = detect_functions(&lines, &abs);
    let rel = normalize_relative(root, &abs);
    let mut candidates = functions
        .iter()
        .filter(|f| {
            if let Some(symbol_path) = symbol_path {
                return f.symbol_path == symbol_path;
            }
            f.symbol == symbol
        })
        .cloned()
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Err(format!(
            "symbol_not_found:{}:{}:{}",
            rel,
            symbol,
            symbol_path.unwrap_or(symbol)
        ));
    }
    if candidates.len() > 1 {
        let tracked = doc.anchors.iter().find(|anchor| {
            anchor.file == rel
                && (anchor.symbol_path.as_deref() == symbol_path
                    || anchor.symbol.as_deref() == Some(symbol))
        });
        if let Some(anchor) = tracked {
            if let Some(fp) = &anchor.fingerprint {
                candidates = candidates
                    .into_iter()
                    .filter(|f| &f.fingerprint == fp)
                    .collect::<Vec<_>>();
            }
            if candidates.len() > 1 {
                if let Some(signature_text) = anchor.signature_text.as_deref() {
                    let expected = normalize_fingerprint_text(signature_text);
                    candidates = candidates
                        .into_iter()
                        .filter(|f| normalize_fingerprint_text(&f.signature_text) == expected)
                        .collect::<Vec<_>>();
                }
            }
        }
    }
    if candidates.len() != 1 {
        return Err(format!(
            "symbol_ambiguous:{}:{}:{}",
            rel,
            symbol,
            symbol_path.unwrap_or(symbol)
        ));
    }
    let fn_hit = candidates.remove(0);

    if let Some((id, marker, line_idx, _is_inline)) =
        find_anchor_before_function(&lines, fn_hit.line_index, &style)
    {
        if marker != style.line {
            lines[line_idx] = rewrite_anchor_on_line(&lines[line_idx], style.line, &id);
            let _ = fs::write(&abs, join_lines(&lines));
        }
        let materialized = materialize_anchors_for_file(root, &abs, &lines);
        replace_doc_anchors_for_file(doc, &rel, materialized.clone());
        if let Some(anchor) = materialized
            .into_iter()
            .find(|anchor| anchor.id.eq_ignore_ascii_case(&id))
        {
            return Ok((anchor, false));
        }
        return Err(format!("anchor_refresh_failed:{}:{}", rel, symbol));
    }

    let existing_ids: HashSet<String> = doc
        .anchors
        .iter()
        .map(|anchor| anchor.id.to_uppercase())
        .chain(
            collect_anchor_nodes(&lines, Some(style.line))
                .into_iter()
                .map(|(id, _, _, _)| id),
        )
        .collect();
    let id = generate_anchor_id(&existing_ids);
    place_anchor_for_function(&mut lines, &fn_hit, &style, &id, placement);
    fs::write(&abs, join_lines(&lines))
        .map_err(|e| format!("anchor_insert_failed:{}:{}", rel, e))?;

    let materialized = materialize_anchors_for_file(root, &abs, &lines);
    replace_doc_anchors_for_file(doc, &rel, materialized.clone());
    if let Some(anchor) = materialized
        .into_iter()
        .find(|anchor| anchor.id.eq_ignore_ascii_case(&id))
    {
        return Ok((anchor, true));
    }

    Err(format!("anchor_insert_refresh_failed:{}:{}", rel, symbol))
}

fn parse_ref(ref_id: &str) -> Option<RefParts> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^([A-Fa-f0-9]{6,16})-(\d+)(?:-(\d+))?$").unwrap());
    let c = re.captures(ref_id)?;
    let anchor_id = c.get(1)?.as_str().to_uppercase();
    let start = c.get(2)?.as_str().parse::<usize>().ok()?;
    let end = if let Some(v) = c.get(3) {
        v.as_str().parse::<usize>().ok()?
    } else {
        start
    };
    Some(RefParts {
        anchor_id,
        start,
        end,
    })
}

fn anchor_symbol_line_index(
    anchor: &AnchorRecord,
    resolution: Option<&AnchorResolution>,
) -> Option<usize> {
    if let Some(line) =
        resolution.and_then(|resolved| resolved.function_hit.as_ref().map(|f| f.line_index))
    {
        return Some(line);
    }
    anchor.line_symbol.map(|line| line.saturating_sub(1))
}

fn anchor_reference_base_line_index(
    anchor: &AnchorRecord,
    resolution: Option<&AnchorResolution>,
) -> Option<usize> {
    if let Some(symbol_line) = anchor_symbol_line_index(anchor, resolution) {
        return Some(symbol_line);
    }
    if let Some(anchor_line) = resolution.and_then(|resolved| resolved.anchor_line_index) {
        return Some(anchor_line);
    }
    anchor.line_anchor.map(|line| line.saturating_sub(1))
}

fn entry_line_indexes(
    anchor: &AnchorRecord,
    resolution: Option<&AnchorResolution>,
    parts: &RefParts,
) -> Option<(usize, usize)> {
    let base = anchor_reference_base_line_index(anchor, resolution)?;
    if anchor_symbol_line_index(anchor, resolution).is_some() {
        Some((
            base + parts.start.saturating_sub(1),
            base + parts.end.saturating_sub(1),
        ))
    } else {
        Some((base + parts.start, base + parts.end))
    }
}

fn ref_offsets_for_block(
    anchor: &AnchorRecord,
    resolution: Option<&AnchorResolution>,
    start_line_index: usize,
    end_line_index: usize,
) -> Option<(usize, usize)> {
    if end_line_index < start_line_index {
        return None;
    }
    let base = anchor_reference_base_line_index(anchor, resolution)?;
    if start_line_index < base {
        return None;
    }
    if anchor_symbol_line_index(anchor, resolution).is_some() {
        Some((start_line_index - base + 1, end_line_index - base + 1))
    } else {
        Some((start_line_index - base, end_line_index - base))
    }
}

fn canonical_ref_id(parts: &RefParts) -> String {
    if parts.start == parts.end {
        format!("{}-{}", parts.anchor_id, parts.start)
    } else {
        format!("{}-{}-{}", parts.anchor_id, parts.start, parts.end)
    }
}

fn default_render_output_path(lensmap_path: &Path) -> PathBuf {
    let parent = lensmap_path
        .parent()
        .map(normalize_path)
        .unwrap_or_else(|| PathBuf::from("."));
    let filename = lensmap_path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("lensmap.json");
    let stem = filename.strip_suffix(".json").unwrap_or(filename);
    parent.join(format!("{}.md", stem))
}

fn tracked_anchor_matches_function(anchor: &AnchorRecord, fn_hit: &FunctionHit) -> bool {
    if let (Some(symbol_path), Some(fp)) = (&anchor.symbol_path, &anchor.fingerprint) {
        return symbol_path == &fn_hit.symbol_path && fp == &fn_hit.fingerprint;
    }
    if let Some(symbol_path) = &anchor.symbol_path {
        return symbol_path == &fn_hit.symbol_path;
    }
    if let (Some(symbol), Some(fp)) = (&anchor.symbol, &anchor.fingerprint) {
        return symbol == &fn_hit.symbol && fp == &fn_hit.fingerprint;
    }
    if let Some(symbol) = &anchor.symbol {
        return symbol == &fn_hit.symbol;
    }
    if let Some(fp) = &anchor.fingerprint {
        return fp == &fn_hit.fingerprint;
    }
    anchor.line_symbol == Some(fn_hit.line_index + 1)
}

fn symbol_path_suffix_depth(left: &str, right: &str) -> usize {
    let left_parts = left.split('.').collect::<Vec<_>>();
    let right_parts = right.split('.').collect::<Vec<_>>();
    let mut shared = 0usize;
    while shared < left_parts.len() && shared < right_parts.len() {
        let left_idx = left_parts.len() - 1 - shared;
        let right_idx = right_parts.len() - 1 - shared;
        if left_parts[left_idx] != right_parts[right_idx] {
            break;
        }
        shared += 1;
    }
    shared
}

fn token_set(raw: &str) -> HashSet<String> {
    raw.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .map(|part| part.trim().to_lowercase())
        .filter(|part| part.len() >= 2)
        .collect()
}

fn token_overlap_score(left: &str, right: &str) -> i32 {
    let left_tokens = token_set(left);
    let right_tokens = token_set(right);
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return 0;
    }
    let shared = left_tokens.intersection(&right_tokens).count() as f64;
    let union = left_tokens.union(&right_tokens).count() as f64;
    ((shared / union) * 100.0).round() as i32
}

fn span_length(start_line: usize, end_line: usize) -> usize {
    end_line.saturating_sub(start_line) + 1
}

fn line_proximity_score(expected_line: Option<usize>, actual_line: usize) -> i32 {
    let expected_line = if let Some(expected_line) = expected_line {
        expected_line
    } else {
        return 0;
    };
    let distance = expected_line.abs_diff(actual_line) as i32;
    (70 - distance.min(70)).max(0)
}

fn span_similarity_score(anchor: &AnchorRecord, fn_hit: &FunctionHit) -> i32 {
    let (span_start, span_end) =
        if let (Some(span_start), Some(span_end)) = (anchor.span_start, anchor.span_end) {
            (span_start, span_end)
        } else {
            return 0;
        };
    let anchor_len = span_length(span_start, span_end);
    let fn_len = span_length(fn_hit.line_index + 1, fn_hit.span_end_index + 1);
    match anchor_len.abs_diff(fn_len) {
        0 => 40,
        1..=2 => 28,
        3..=5 => 16,
        _ => 0,
    }
}

fn fuzzy_candidate_score(anchor: &AnchorRecord, fn_hit: &FunctionHit) -> i32 {
    let mut score = 0i32;

    if let Some(symbol_path) = &anchor.symbol_path {
        if symbol_path == &fn_hit.symbol_path {
            score += 140;
        } else {
            score += match symbol_path_suffix_depth(symbol_path, &fn_hit.symbol_path) {
                0 => 0,
                1 => 28,
                2 => 54,
                _ => 80,
            };
        }
    }

    if let Some(symbol) = &anchor.symbol {
        if symbol == &fn_hit.symbol {
            score += 60;
        }
    }

    if let Some(fingerprint) = &anchor.fingerprint {
        if fingerprint == &fn_hit.fingerprint {
            score += 160;
        }
    }

    if let Some(signature_text) = anchor.signature_text.as_deref() {
        let normalized_left = normalize_fingerprint_text(signature_text);
        let normalized_right = normalize_fingerprint_text(&fn_hit.signature_text);
        if normalized_left == normalized_right {
            score += 120;
        } else {
            score += token_overlap_score(&normalized_left, &normalized_right);
        }
    }

    score += span_similarity_score(anchor, fn_hit);
    score += line_proximity_score(
        anchor.line_symbol.or(anchor.span_start),
        fn_hit.line_index + 1,
    );
    score += line_proximity_score(anchor.line_anchor, fn_hit.line_index + 1) / 2;
    score
}

fn fuzzy_resolve_anchor(anchor: &AnchorRecord, fns: &[FunctionHit]) -> Option<FunctionHit> {
    let mut scored = fns
        .iter()
        .cloned()
        .map(|fn_hit| {
            let score = fuzzy_candidate_score(anchor, &fn_hit);
            let line_bias = anchor
                .line_symbol
                .or(anchor.span_start)
                .or(anchor.line_anchor)
                .map(|line| line.abs_diff(fn_hit.line_index + 1))
                .unwrap_or(usize::MAX);
            (fn_hit, score, line_bias)
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.0.symbol_path.cmp(&right.0.symbol_path))
    });

    let (best_hit, best_score, _) = scored.first()?.clone();
    let next_score = scored.get(1).map(|item| item.1).unwrap_or(0);
    if best_score >= 140 && best_score - next_score >= 20 {
        Some(best_hit)
    } else {
        None
    }
}

fn comment_block_in_range(blocks: &[CommentBlock], start: usize, end_exclusive: usize) -> bool {
    blocks.iter().any(|block| {
        (block.start >= start && block.start < end_exclusive)
            || (block.end >= start && block.end < end_exclusive)
    })
}

fn resolve_anchor_in_lines(
    anchor: &AnchorRecord,
    lines: &[String],
    fns: &[FunctionHit],
    style: &CommentStyle,
) -> AnchorResolution {
    let id = anchor.id.to_uppercase();
    let nodes = collect_anchor_nodes(lines, Some(style.line));
    if let Some((_nid, _marker, line_idx, is_inline)) =
        nodes.iter().find(|(nid, _, _, _)| nid == &id)
    {
        let fn_hit = if *is_inline {
            fns.iter().find(|f| f.line_index == *line_idx).cloned()
        } else {
            fns.iter().find(|f| f.line_index > *line_idx).cloned()
        };
        return AnchorResolution {
            anchor_line_index: Some(*line_idx),
            function_hit: fn_hit,
            strategy: "anchor_id".to_string(),
            anchor_found_in_source: true,
        };
    }

    let mut candidate: Option<FunctionHit> = None;
    let mut strategy = "unresolved".to_string();

    if let Some(symbol_path) = &anchor.symbol_path {
        let by_path = fns
            .iter()
            .filter(|f| &f.symbol_path == symbol_path)
            .cloned()
            .collect::<Vec<_>>();
        if by_path.len() == 1 {
            candidate = by_path.first().cloned();
            strategy = "symbol_path".to_string();
        } else if by_path.len() > 1 {
            if let Some(fp) = &anchor.fingerprint {
                let by_fp = by_path
                    .into_iter()
                    .filter(|f| &f.fingerprint == fp)
                    .collect::<Vec<_>>();
                if by_fp.len() == 1 {
                    candidate = by_fp.first().cloned();
                    strategy = "symbol_path+fingerprint".to_string();
                }
            }
        }
    }

    if let Some(symbol) = &anchor.symbol {
        if candidate.is_none() {
            let by_symbol = fns
                .iter()
                .filter(|f| &f.symbol == symbol)
                .cloned()
                .collect::<Vec<_>>();
            if by_symbol.len() == 1 {
                candidate = by_symbol.first().cloned();
                strategy = "symbol".to_string();
            } else if by_symbol.len() > 1 {
                if let Some(fp) = &anchor.fingerprint {
                    let by_fp = by_symbol
                        .into_iter()
                        .filter(|f| &f.fingerprint == fp)
                        .collect::<Vec<_>>();
                    if by_fp.len() == 1 {
                        candidate = by_fp.first().cloned();
                        strategy = "symbol+fingerprint".to_string();
                    }
                }
            }
        }
    }

    if candidate.is_none() {
        if let Some(fp) = &anchor.fingerprint {
            let by_fp = fns
                .iter()
                .filter(|f| &f.fingerprint == fp)
                .cloned()
                .collect::<Vec<_>>();
            if by_fp.len() == 1 {
                candidate = by_fp.first().cloned();
                strategy = "fingerprint".to_string();
            }
        }
    }

    if candidate.is_none() {
        if let (Some(span_start), Some(span_end)) = (anchor.span_start, anchor.span_end) {
            candidate = fns
                .iter()
                .find(|f| f.line_index + 1 == span_start && f.span_end_index + 1 == span_end)
                .cloned();
            if candidate.is_some() {
                strategy = "span".to_string();
            }
        }
    }

    if candidate.is_none() {
        candidate = fuzzy_resolve_anchor(anchor, fns);
        if candidate.is_some() {
            strategy = "fuzzy_refactor".to_string();
        }
    }

    if candidate.is_none() {
        if let Some(line_symbol) = anchor.line_symbol {
            candidate = fns
                .iter()
                .find(|f| f.line_index + 1 == line_symbol)
                .cloned();
            if candidate.is_some() {
                strategy = "line_symbol".to_string();
            }
        }
    }

    if candidate.is_none() {
        if let Some(line_anchor) = anchor.line_anchor {
            candidate = fns
                .iter()
                .find(|f| f.line_index + 1 >= line_anchor)
                .cloned();
            if candidate.is_some() {
                strategy = "line_anchor_fallback".to_string();
            }
        }
    }

    let anchor_line_index = if let Some(fn_hit) = &candidate {
        if let Some((_existing, _marker, line_idx, _is_inline)) =
            find_anchor_before_function(lines, fn_hit.line_index, style)
        {
            Some(line_idx)
        } else {
            Some(compute_anchor_insert_index(lines, fn_hit.line_index, style))
        }
    } else {
        None
    };

    AnchorResolution {
        anchor_line_index,
        function_hit: candidate,
        strategy,
        anchor_found_in_source: false,
    }
}

fn parse_ref_marker_comment(comment: &str) -> Option<RefLineMatch> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"^\s*(//|#)\s*@lensmap-ref\s+([A-Fa-f0-9]{6,16}-\d+(?:-\d+)?)\s*$").unwrap()
    });
    let cap = re.captures(comment)?;
    Some(RefLineMatch {
        marker: cap.get(1)?.as_str().to_string(),
        ref_id: cap.get(2)?.as_str().to_uppercase(),
    })
}

fn locate_ref_site(line: &str, marker: &str) -> Option<RefSite> {
    let idx = find_line_comment_index_outside_strings(line, marker)?;
    let prefix_raw = &line[..idx];
    let comment = &line[idx..];
    let parsed = parse_ref_marker_comment(comment)?;
    if parsed.marker != marker {
        return None;
    }

    let is_inline = !prefix_raw.trim().is_empty();
    if is_inline {
        let prefix = prefix_raw.trim_end().to_string();
        let spacing = if prefix_raw.len() > prefix.len() {
            prefix_raw[prefix.len()..].to_string()
        } else {
            " ".to_string()
        };
        return Some(RefSite {
            ref_id: parsed.ref_id,
            is_inline: true,
            indent: String::new(),
            prefix,
            spacing,
        });
    }

    let indent = prefix_raw
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect::<String>();
    Some(RefSite {
        ref_id: parsed.ref_id,
        is_inline: false,
        indent,
        prefix: String::new(),
        spacing: " ".to_string(),
    })
}

fn line_comment_lines(indent: &str, marker: &str, text: &str) -> Vec<String> {
    let mut out = vec![];
    let normalized_text = text.replace('\r', "");
    let raw_lines = normalized_text
        .split('\n')
        .map(|l| l.trim_end())
        .collect::<Vec<_>>();
    for line in raw_lines {
        let body = line.trim();
        if body.is_empty() {
            out.push(format!("{}{}", indent, marker));
        } else {
            out.push(format!("{}{} {}", indent, marker, body));
        }
    }
    if out.is_empty() {
        out.push(format!("{}{}", indent, marker));
    }
    out
}

fn find_latest_anchor_for_line(
    nodes: &[(String, String, usize, bool)],
    line_idx: usize,
) -> Option<(String, usize)> {
    let mut out: Option<(String, usize)> = None;
    for (id, _marker, idx, _is_inline) in nodes {
        if *idx <= line_idx {
            out = Some((id.clone(), *idx));
        } else {
            break;
        }
    }
    out
}

fn should_preserve_comment_content(content: &str) -> bool {
    let s = content.trim().to_lowercase();
    if s.is_empty() {
        return false;
    }
    if s.starts_with("@@") || s.starts_with(ANCHOR_TAG) || s.starts_with(REF_TAG) {
        return true;
    }
    PRESERVE_COMMENT_PREFIXES.iter().any(|p| s.starts_with(p))
}

fn clean_line_comment_text(line: &str, marker: &str) -> String {
    if let Some(idx) = line.find(marker) {
        return line[idx + marker.len()..].trim().to_string();
    }
    String::new()
}

fn find_line_comment_index_outside_strings(line: &str, marker: &str) -> Option<usize> {
    if marker.is_empty() {
        return None;
    }
    let chars: Vec<char> = line.chars().collect();
    let mut in_single = false;
    let mut in_double = false;
    let mut in_template = false;
    let mut escaped = false;

    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        let next = if i + 1 < chars.len() {
            chars[i + 1]
        } else {
            '\0'
        };

        if escaped {
            escaped = false;
            i += 1;
            continue;
        }

        if in_single || in_double || in_template {
            if ch == '\\' {
                escaped = true;
                i += 1;
                continue;
            }
            if in_single && ch == '\'' {
                in_single = false;
                i += 1;
                continue;
            }
            if in_double && ch == '"' {
                in_double = false;
                i += 1;
                continue;
            }
            if in_template && ch == '`' {
                in_template = false;
                i += 1;
                continue;
            }
            i += 1;
            continue;
        }

        if ch == '\'' {
            in_single = true;
            i += 1;
            continue;
        }
        if ch == '"' {
            in_double = true;
            i += 1;
            continue;
        }
        if ch == '`' {
            in_template = true;
            i += 1;
            continue;
        }

        if marker == "//" && ch == '/' && next == '/' {
            return Some(i);
        }
        if marker == "#" && ch == '#' {
            return Some(i);
        }

        i += 1;
    }

    None
}

fn parse_inline_line_comment(line: &str, marker: &str) -> Option<InlineCommentMatch> {
    let idx = find_line_comment_index_outside_strings(line, marker)?;
    if idx == 0 {
        return None;
    }
    let prefix_raw = &line[..idx];
    if prefix_raw.trim().is_empty() {
        return None;
    }
    let content = line[idx + marker.len()..].trim().to_string();
    let prefix = prefix_raw.trim_end().to_string();
    let spacing = if prefix_raw.len() > prefix.len() {
        prefix_raw[prefix.len()..].to_string()
    } else {
        " ".to_string()
    };
    Some(InlineCommentMatch {
        content,
        prefix,
        spacing,
    })
}

fn find_inline_block_comment(line: &str) -> Option<InlineBlockMatch> {
    let chars: Vec<char> = line.chars().collect();
    let mut in_single = false;
    let mut in_double = false;
    let mut in_template = false;
    let mut escaped = false;

    let mut i = 0usize;
    while i + 1 < chars.len() {
        let ch = chars[i];
        let next = chars[i + 1];

        if escaped {
            escaped = false;
            i += 1;
            continue;
        }

        if in_single || in_double || in_template {
            if ch == '\\' {
                escaped = true;
                i += 1;
                continue;
            }
            if in_single && ch == '\'' {
                in_single = false;
                i += 1;
                continue;
            }
            if in_double && ch == '"' {
                in_double = false;
                i += 1;
                continue;
            }
            if in_template && ch == '`' {
                in_template = false;
                i += 1;
                continue;
            }
            i += 1;
            continue;
        }

        if ch == '\'' {
            in_single = true;
            i += 1;
            continue;
        }
        if ch == '"' {
            in_double = true;
            i += 1;
            continue;
        }
        if ch == '`' {
            in_template = true;
            i += 1;
            continue;
        }

        if ch == '/' && next == '*' {
            let start = i;
            if start == 0 {
                return None;
            }
            let end = line[start + 2..].find("*/")? + start + 2;
            let prefix_raw = &line[..start];
            if prefix_raw.trim().is_empty() {
                return None;
            }
            let content = line[start + 2..end].trim().to_string();
            let prefix = prefix_raw.trim_end().to_string();
            let spacing = if prefix_raw.len() > prefix.len() {
                prefix_raw[prefix.len()..].to_string()
            } else {
                " ".to_string()
            };
            return Some(InlineBlockMatch {
                content,
                prefix,
                spacing,
            });
        }

        i += 1;
    }

    None
}

fn clean_block_comment_text(block_lines: &[String]) -> String {
    let raw = block_lines.join("\n");
    let mut cleaned = raw;
    if let Some(stripped) = cleaned.strip_prefix("/*") {
        cleaned = stripped.to_string();
    }
    if let Some(stripped) = cleaned.strip_suffix("*/") {
        cleaned = stripped.to_string();
    }

    let mut lines = cleaned
        .split('\n')
        .map(|l| {
            let ll = l.trim_end();
            if ll.trim_start().starts_with('*') {
                ll.trim_start()
                    .trim_start_matches('*')
                    .trim_start()
                    .to_string()
            } else {
                ll.to_string()
            }
        })
        .collect::<Vec<_>>();

    while !lines.is_empty() && lines[0].trim().is_empty() {
        lines.remove(0);
    }
    while !lines.is_empty() && lines.last().map(|s| s.trim().is_empty()).unwrap_or(false) {
        lines.pop();
    }

    lines.join("\n").trim().to_string()
}

fn collect_comment_blocks(lines: &[String], abs_file: &Path) -> Vec<CommentBlock> {
    let style = comment_style_for(abs_file);
    let mut out = vec![];

    let mut i = 0usize;
    while i < lines.len() {
        let line = &lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        if trimmed.starts_with(style.line) {
            let content = clean_line_comment_text(line, style.line);
            let lower = content.to_lowercase();
            if !lower.starts_with(ANCHOR_TAG)
                && !lower.starts_with(REF_TAG)
                && !trimmed.starts_with("#!")
                && !should_preserve_comment_content(&content)
            {
                out.push(CommentBlock {
                    kind: "line".to_string(),
                    start: i,
                    end: i,
                    indent: line.chars().take_while(|c| c.is_whitespace()).collect(),
                    marker: style.line.to_string(),
                    text: content,
                    replace_mode: "full".to_string(),
                    inline_prefix: String::new(),
                    inline_spacing: " ".to_string(),
                });
            }
            i += 1;
            continue;
        }

        if let Some(inline) = parse_inline_line_comment(line, style.line) {
            let lower = inline.content.to_lowercase();
            if !lower.starts_with(ANCHOR_TAG)
                && !lower.starts_with(REF_TAG)
                && !should_preserve_comment_content(&inline.content)
            {
                out.push(CommentBlock {
                    kind: "inline".to_string(),
                    start: i,
                    end: i,
                    indent: String::new(),
                    marker: style.line.to_string(),
                    text: inline.content,
                    replace_mode: "inline".to_string(),
                    inline_prefix: inline.prefix,
                    inline_spacing: inline.spacing,
                });
                i += 1;
                continue;
            }
        }

        if style.block_start.is_some() {
            if let Some(inline_block) = find_inline_block_comment(line) {
                let lower = inline_block.content.to_lowercase();
                if !lower.starts_with(ANCHOR_TAG)
                    && !lower.starts_with(REF_TAG)
                    && !should_preserve_comment_content(&inline_block.content)
                {
                    out.push(CommentBlock {
                        kind: "inline".to_string(),
                        start: i,
                        end: i,
                        indent: String::new(),
                        marker: style.line.to_string(),
                        text: inline_block.content,
                        replace_mode: "inline".to_string(),
                        inline_prefix: inline_block.prefix,
                        inline_spacing: inline_block.spacing,
                    });
                    i += 1;
                    continue;
                }
            }
        }

        if let (Some(bs), Some(be)) = (style.block_start, style.block_end) {
            if trimmed.starts_with(bs) {
                let start = i;
                let mut end = i;
                let mut block_lines = vec![line.clone()];
                let mut found_end = trimmed.contains(be);
                while !found_end && end + 1 < lines.len() {
                    end += 1;
                    block_lines.push(lines[end].clone());
                    if lines[end].contains(be) {
                        found_end = true;
                    }
                }

                let text = clean_block_comment_text(&block_lines);
                let lower = text.to_lowercase();
                if !lower.starts_with(ANCHOR_TAG)
                    && !lower.starts_with(REF_TAG)
                    && !should_preserve_comment_content(&text)
                {
                    out.push(CommentBlock {
                        kind: if block_lines
                            .first()
                            .map(|l| l.trim().starts_with("/**"))
                            .unwrap_or(false)
                        {
                            "doc".to_string()
                        } else {
                            "block".to_string()
                        },
                        start,
                        end,
                        indent: line.chars().take_while(|c| c.is_whitespace()).collect(),
                        marker: style.line.to_string(),
                        text,
                        replace_mode: "full".to_string(),
                        inline_prefix: String::new(),
                        inline_spacing: " ".to_string(),
                    });
                }
                i = end + 1;
                continue;
            }
        }

        i += 1;
    }

    out
}

fn normalize_covers(args: &ParsedArgs, doc: &LensMapDoc, fallback: &[String]) -> Vec<String> {
    let mut from_args = vec![];
    from_args.extend(split_csv(args.get("covers")));
    from_args.extend(split_csv(args.get("files")));
    from_args.extend(split_csv(args.get("file")));
    if !from_args.is_empty() {
        return from_args;
    }
    if !doc.covers.is_empty() {
        return doc.covers.clone();
    }
    fallback.to_vec()
}

fn should_anchor_function_smart(
    rel: &str,
    fn_hit: &FunctionHit,
    fn_idx: usize,
    ctx: SmartAnchorContext<'_>,
) -> bool {
    if find_anchor_before_function(ctx.lines, fn_hit.line_index, ctx.style).is_some() {
        return true;
    }
    if ctx
        .tracked_anchors
        .iter()
        .any(|anchor| anchor.file == rel && tracked_anchor_matches_function(anchor, fn_hit))
    {
        return true;
    }
    let range_start = compute_anchor_insert_index(ctx.lines, fn_hit.line_index, ctx.style);
    let range_end = ctx
        .functions
        .get(fn_idx + 1)
        .map(|next| next.line_index)
        .unwrap_or(ctx.lines.len());
    comment_block_in_range(ctx.blocks, range_start, range_end)
}

fn cmd_init(root: &Path, args: &ParsedArgs) {
    let project = args
        .positional
        .get(1)
        .map(String::as_str)
        .unwrap_or("project")
        .trim()
        .to_string();
    let project = if project.is_empty() {
        "project".to_string()
    } else {
        project
    };
    let project_dir = normalize_path(&root.join(&project));
    let _ = fs::create_dir_all(&project_dir);
    let private_dir = root.join("local/private-lenses");
    let _ = fs::create_dir_all(&private_dir);

    let mode = if args.get("mode").unwrap_or("group") == "file" {
        "file"
    } else {
        "group"
    };

    let mut covers = split_csv(args.get("covers"));
    if mode == "file" {
        if let Some(f) = args.get("file") {
            covers.push(f.to_string());
        }
    }
    if covers.is_empty() {
        covers.push(project.clone());
    }

    let lens_path = resolve_lensmap_path(root, args, Some(&project_dir));
    let mut doc = make_lensmap_doc(mode, covers);
    if let Some(raw) = args.get("anchor-placement") {
        if let Some(parsed) = parse_anchor_placement(raw) {
            doc.metadata.insert(
                "anchor_placement".to_string(),
                Value::String(match parsed {
                    AnchorPlacement::Inline => "inline".to_string(),
                    AnchorPlacement::Standalone => "standalone".to_string(),
                }),
            );
        } else {
            emit(
                json!({
                    "ok": false,
                    "type": "lensmap",
                    "action": "init",
                    "error": "invalid_anchor_placement",
                    "anchor_placement": raw,
                    "allowed": ["inline", "standalone"],
                }),
                1,
            );
        }
    }
    doc.metadata
        .insert("project".to_string(), Value::String(project.clone()));
    save_doc(&lens_path, doc.clone());

    let safe_name = project.replace(['/', '\\'], "__");
    let private_path = private_dir.join(format!("{}.private.lens.json", safe_name));
    ensure_dir(&private_path);
    let private_json = json!({
        "project": project,
        "hidden": true,
        "entries": [],
        "version": "1.0.0"
    });
    let _ = fs::write(
        &private_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&private_json).unwrap_or_else(|_| "{}".to_string())
        ),
    );

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "init",
        "project": project,
        "mode": mode,
        "lens_file": normalize_relative(root, &lens_path),
        "covers": doc.covers,
        "ts": now_iso(),
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_template_add(root: &Path, args: &ParsedArgs) {
    let t = args
        .positional
        .get(2)
        .map(String::as_str)
        .unwrap_or("default")
        .trim()
        .to_string();
    let template = if t.is_empty() {
        "default".to_string()
    } else {
        t
    };
    let template_path = root
        .join("templates")
        .join(format!("{}.lens.template.json", template));
    ensure_dir(&template_path);
    let payload = json!({
        "type": template,
        "template": true,
        "fields": ["title", "owner", "scope", "anchor", "ref", "text"]
    });
    let _ = fs::write(
        &template_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
        ),
    );

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "template_add",
        "template": normalize_relative(root, &template_path),
        "ts": now_iso(),
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_scan(root: &Path, args: &ParsedArgs) {
    let dry_run = args.has("dry-run");
    let lensmap_path = resolve_lensmap_path(root, args, None);
    let mut doc = load_doc(&lensmap_path, "group");
    let anchor_placement = if let Some(raw) = args.get("anchor-placement") {
        parse_anchor_placement(raw).unwrap_or_else(|| {
            emit(
                json!({
                    "ok": false,
                    "type": "lensmap",
                    "action": "scan",
                    "error": "invalid_anchor_placement",
                    "anchor_placement": raw,
                    "allowed": ["inline", "standalone"],
                }),
                1,
            );
        })
    } else {
        preferred_anchor_placement(&doc, Some(args)).unwrap_or(AnchorPlacement::Inline)
    };
    let anchor_mode = args
        .get("anchor-mode")
        .or_else(|| {
            doc.metadata
                .get("default_anchor_mode")
                .and_then(Value::as_str)
        })
        .unwrap_or("smart")
        .trim()
        .to_lowercase();
    if !["smart", "all"].contains(&anchor_mode.as_str()) {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "scan",
                "error": "invalid_anchor_mode",
                "anchor_mode": anchor_mode,
                "allowed": ["smart", "all"],
            }),
            1,
        );
    }
    let covers = normalize_covers(args, &doc, &[]);
    let files = resolve_covers_to_files(root, &covers);

    if files.is_empty() {
        if !lensmap_path.exists() && covers.is_empty() {
            emit(
                lensmap_missing_payload(root, "scan", &lensmap_path, args),
                1,
            );
        }
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "scan",
                "error": "no_files_resolved",
                "covers": covers,
                "hint": tr(
                    "No source files matched covers/files. Pass a real --covers value or run init first.",
                    "没有源文件匹配 covers/files。请传入真实的 --covers 值，或先运行 init。",
                ),
                "examples": quickstart_examples(),
            }),
            1,
        );
    }

    let mut existing_ids: HashSet<String> = doc
        .anchors
        .iter()
        .map(|a| a.id.to_uppercase())
        .filter(|id| !id.is_empty())
        .collect();

    let mut anchor_by_id: HashMap<String, AnchorRecord> = doc
        .anchors
        .iter()
        .map(|a| (a.id.to_uppercase(), a.clone()))
        .collect();

    let mut file_summaries = vec![];

    for rel in &files {
        let abs = normalize_path(&root.join(rel));
        let style = comment_style_for(&abs);
        let original = fs::read_to_string(&abs).unwrap_or_default();
        let original_lines = split_lines(&original);
        let mut lines = original_lines.clone();
        let comment_blocks = collect_comment_blocks(&original_lines, &abs);
        let tracked_anchors = doc
            .anchors
            .iter()
            .filter(|anchor| anchor.file == *rel)
            .cloned()
            .collect::<Vec<_>>();

        let mut functions = detect_functions(&lines, &abs);
        let mut added = 0usize;
        let mut skipped_by_mode = 0usize;

        let mut i = 0usize;
        while i < functions.len() {
            let fn_hit = functions[i].clone();
            if let Some((id, marker, line_idx, _is_inline)) =
                find_anchor_before_function(&lines, fn_hit.line_index, &style)
            {
                existing_ids.insert(id.clone());
                if marker != style.line {
                    lines[line_idx] = rewrite_anchor_on_line(&lines[line_idx], style.line, &id);
                }
                i += 1;
                continue;
            }

            if anchor_mode == "smart"
                && !should_anchor_function_smart(
                    rel,
                    &fn_hit,
                    i,
                    SmartAnchorContext {
                        functions: &functions,
                        lines: &lines,
                        blocks: &comment_blocks,
                        style: &style,
                        tracked_anchors: &tracked_anchors,
                    },
                )
            {
                skipped_by_mode += 1;
                i += 1;
                continue;
            }

            let id = generate_anchor_id(&existing_ids);
            existing_ids.insert(id.clone());
            let insert_at = compute_anchor_insert_index(&lines, fn_hit.line_index, &style);
            if has_anchor_in_range(&lines, insert_at, fn_hit.line_index.saturating_sub(1)) {
                i += 1;
                continue;
            }
            place_anchor_for_function(&mut lines, &fn_hit, &style, &id, anchor_placement);
            added += 1;
            functions = detect_functions(&lines, &abs);
            i = 0;
        }

        let changed = lines != original_lines;
        if changed && !dry_run {
            let _ = fs::write(&abs, join_lines(&lines));
        }

        let materialized = materialize_anchors_for_file(root, &abs, &lines);
        for anchor in materialized {
            anchor_by_id.insert(anchor.id.to_uppercase(), anchor);
        }

        file_summaries.push(json!({
            "file": rel,
            "functions": functions.len(),
            "anchors_added": added,
            "skipped_by_anchor_mode": skipped_by_mode,
            "changed": changed,
        }));
    }

    let mut merged = anchor_by_id.values().cloned().collect::<Vec<_>>();
    merged.sort_by(|a, b| {
        let fa = a.file.clone();
        let fb = b.file.clone();
        if fa != fb {
            return fa.cmp(&fb);
        }
        a.line_anchor.unwrap_or(0).cmp(&b.line_anchor.unwrap_or(0))
    });
    doc.anchors = merged;

    let mut cover_set = BTreeMap::new();
    for c in &doc.covers {
        cover_set.insert(c.clone(), true);
    }
    for c in &covers {
        cover_set.insert(c.clone(), true);
    }
    doc.covers = cover_set.keys().cloned().collect();

    if !dry_run {
        save_doc(&lensmap_path, doc.clone());
    }

    let added_total: usize = file_summaries
        .iter()
        .filter_map(|v| v.get("anchors_added").and_then(Value::as_u64))
        .map(|v| v as usize)
        .sum();

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "scan",
        "dry_run": dry_run,
        "lensmap": normalize_relative(root, &lensmap_path),
        "anchor_mode": anchor_mode,
        "anchor_placement": match anchor_placement {
            AnchorPlacement::Inline => "inline",
            AnchorPlacement::Standalone => "standalone",
        },
        "files_scanned": files.len(),
        "anchors_added": added_total,
        "anchors_total": doc.anchors.len(),
        "ts": now_iso(),
        "files": file_summaries,
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_extract_comments(root: &Path, args: &ParsedArgs) {
    let dry_run = args.has("dry-run");
    let lensmap_path = resolve_lensmap_path(root, args, None);
    let mut doc = load_doc(&lensmap_path, "group");
    let covers = normalize_covers(args, &doc, &[]);
    let files = resolve_covers_to_files(root, &covers);

    if files.is_empty() {
        if !lensmap_path.exists() && covers.is_empty() {
            emit(
                lensmap_missing_payload(root, "extract_comments", &lensmap_path, args),
                1,
            );
        }
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "extract_comments",
                "error": "no_files_resolved",
                "covers": covers,
                "hint": tr(
                    "No source files matched covers/files. Pass a real --covers value or run init first.",
                    "没有源文件匹配 covers/files。请传入真实的 --covers 值，或先运行 init。",
                ),
                "examples": quickstart_examples(),
            }),
            1,
        );
    }

    let mut existing_entry_keys = HashSet::new();
    for e in &doc.entries {
        let key = format!("{}::{}", e.file, e.ref_id.to_uppercase());
        existing_entry_keys.insert(key);
    }

    let mut new_entries = vec![];
    let mut file_summaries = vec![];

    for rel in &files {
        let abs = normalize_path(&root.join(rel));
        let style = comment_style_for(&abs);
        let text = fs::read_to_string(&abs).unwrap_or_default();
        let mut lines = split_lines(&text);
        let mut anchor_nodes = collect_anchor_nodes(&lines, Some(style.line));
        anchor_nodes.sort_by(|a, b| a.2.cmp(&b.2));
        let anchor_lookup = materialize_anchors_for_file(root, &abs, &lines)
            .into_iter()
            .map(|anchor| (anchor.id.to_uppercase(), anchor))
            .collect::<HashMap<_, _>>();

        let blocks = collect_comment_blocks(&lines, &abs);
        let mut extracted = 0usize;
        let mut changed = false;

        for block in blocks {
            let latest = find_latest_anchor_for_line(&anchor_nodes, block.start);
            if latest.is_none() {
                continue;
            }
            let (anchor_id, _anchor_line) = latest.unwrap();
            let anchor = if let Some(anchor) = anchor_lookup.get(&anchor_id.to_uppercase()) {
                anchor
            } else {
                continue;
            };
            let Some((start_offset, end_offset)) =
                ref_offsets_for_block(anchor, None, block.start, block.end)
            else {
                continue;
            };
            let ref_id = if start_offset == end_offset {
                format!("{}-{}", anchor_id, start_offset)
            } else {
                format!("{}-{}-{}", anchor_id, start_offset, end_offset)
            };

            let entry = EntryRecord {
                ref_id: ref_id.clone(),
                file: rel.clone(),
                anchor_id: Some(anchor_id.clone()),
                kind: Some(if block.kind == "doc" {
                    "doc".to_string()
                } else {
                    "comment".to_string()
                }),
                text: Some(block.text.trim().to_string()),
                created_at: Some(now_iso()),
                source: Some("extract_comments".to_string()),
            };

            let key = format!("{}::{}", entry.file, entry.ref_id.to_uppercase());
            if !existing_entry_keys.contains(&key) {
                existing_entry_keys.insert(key);
                new_entries.push(entry);
            }

            if block.replace_mode == "inline" {
                lines[block.start] = format!(
                    "{}{}{} {} {}",
                    block.inline_prefix, block.inline_spacing, block.marker, REF_TAG, ref_id
                );
            } else {
                lines[block.start] = make_ref_line(&block.indent, &block.marker, &ref_id);
                for idx in (block.start + 1)..=block.end {
                    if idx < lines.len() {
                        lines[idx] = String::new();
                    }
                }
            }

            extracted += 1;
            changed = true;
        }

        if changed && !dry_run {
            let _ = fs::write(&abs, join_lines(&lines));
        }

        file_summaries.push(json!({
            "file": rel,
            "extracted_comments": extracted,
            "changed": changed,
        }));
    }

    doc.entries.extend(new_entries.clone());

    let mut cover_set = BTreeMap::new();
    for c in &doc.covers {
        cover_set.insert(c.clone(), true);
    }
    for c in &covers {
        cover_set.insert(c.clone(), true);
    }
    doc.covers = cover_set.keys().cloned().collect();

    if !dry_run {
        save_doc(&lensmap_path, doc.clone());
    }

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "extract_comments",
        "dry_run": dry_run,
        "lensmap": normalize_relative(root, &lensmap_path),
        "files_scanned": files.len(),
        "entries_added": new_entries.len(),
        "ts": now_iso(),
        "files": file_summaries,
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_annotate(root: &Path, args: &ParsedArgs) {
    let lensmap_path = resolve_lensmap_path(root, args, None);
    if !lensmap_path.exists() {
        emit(
            lensmap_missing_payload(root, "annotate", &lensmap_path, args),
            1,
        );
    }

    let mut doc = load_doc(&lensmap_path, "group");
    let raw_ref = args
        .get("ref")
        .or_else(|| args.positional.get(1).map(String::as_str))
        .unwrap_or("")
        .trim()
        .to_string();
    let text = args
        .get("text")
        .or_else(|| args.positional.get(2).map(String::as_str))
        .unwrap_or("")
        .trim()
        .to_string();
    let requested_file = args.get("file").unwrap_or("").trim().to_string();
    let requested_symbol = args.get("symbol").unwrap_or("").trim().to_string();
    let requested_symbol_path = args.get("symbol-path").unwrap_or("").trim().to_string();

    if text.is_empty() {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "annotate",
                "error": "annotate_requires_text",
                "hint": tr(
                    "Use --text=<comment text> with either --ref=<HEX-start[-end]> or --file + --symbol.",
                    "请配合 --ref=<HEX-start[-end]> 或 --file + --symbol 一起使用 --text=<comment text>。",
                ),
                "example": "lensmap annotate --lensmap=demo/lensmap.json --file=demo/src/app.ts --symbol=run --offset=1 --text=\"why this exists\"",
            }),
            1,
        );
    }

    let (parsed, anchor, mut file, created_anchor) = if raw_ref.is_empty() {
        if requested_file.is_empty() || requested_symbol.is_empty() {
            emit(
                json!({
                    "ok": false,
                    "type": "lensmap",
                    "action": "annotate",
                    "error": "annotate_requires_ref_or_file_symbol",
                    "hint": tr(
                        "Use --ref=<HEX-start[-end]> or --file=<path> --symbol=<name> [--offset=1] [--end-offset=N].",
                        "请使用 --ref=<HEX-start[-end]>，或使用 --file=<path> --symbol=<name> [--offset=1] [--end-offset=N]。",
                    ),
                }),
                1,
            );
        }
        let start = args
            .get("offset")
            .unwrap_or("1")
            .trim()
            .parse::<usize>()
            .unwrap_or_else(|_| {
                emit(
                    json!({
                        "ok": false,
                        "type": "lensmap",
                        "action": "annotate",
                        "error": "invalid_offset",
                        "offset": args.get("offset").unwrap_or(""),
                    }),
                    1,
                );
            });
        let end = args
            .get("end-offset")
            .or_else(|| args.get("end"))
            .unwrap_or("")
            .trim();
        let end = if end.is_empty() {
            start
        } else {
            end.parse::<usize>().unwrap_or_else(|_| {
                emit(
                    json!({
                        "ok": false,
                        "type": "lensmap",
                        "action": "annotate",
                        "error": "invalid_end_offset",
                        "end_offset": end,
                    }),
                    1,
                );
            })
        };
        let (anchor, created) = ensure_anchor_for_symbol(
            root,
            &mut doc,
            &requested_file,
            &requested_symbol,
            if requested_symbol_path.is_empty() {
                None
            } else {
                Some(requested_symbol_path.as_str())
            },
        )
        .unwrap_or_else(|reason| {
            emit(
                json!({
                    "ok": false,
                    "type": "lensmap",
                    "action": "annotate",
                    "error": "symbol_anchor_failed",
                    "reason": reason,
                    "file": requested_file,
                    "symbol": requested_symbol,
                    "symbol_path": requested_symbol_path,
                }),
                1,
            );
        });
        let parsed = RefParts {
            anchor_id: anchor.id.to_uppercase(),
            start,
            end,
        };
        (parsed, anchor.clone(), anchor.file.clone(), created)
    } else {
        let parsed = if let Some(parts) = parse_ref(&raw_ref) {
            parts
        } else {
            emit(
                json!({
                    "ok": false,
                    "type": "lensmap",
                    "action": "annotate",
                    "error": "invalid_ref",
                    "ref": raw_ref,
                }),
                1,
            );
        };
        let anchor = if let Some(a) = doc
            .anchors
            .iter()
            .find(|a| a.id.to_uppercase() == parsed.anchor_id)
        {
            a.clone()
        } else {
            emit(
                json!({
                    "ok": false,
                    "type": "lensmap",
                    "action": "annotate",
                    "error": "entry_anchor_missing",
                    "anchor_id": parsed.anchor_id,
                    "hint": tr(
                        "Run scan first so the anchor exists in lensmap.json, or annotate with --file + --symbol.",
                        "请先运行 scan，让锚点进入 lensmap.json；或者使用 --file + --symbol 进行 annotate。",
                    ),
                }),
                1,
            );
        };
        let file = if requested_file.is_empty() {
            anchor.file.clone()
        } else {
            requested_file.clone()
        };
        (parsed, anchor, file, false)
    };
    let canonical_ref = canonical_ref_id(&parsed);
    if file.is_empty() {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "annotate",
                "error": "file_required",
                "hint": tr(
                    "Pass --file=<path> or annotate against an anchor with a known file.",
                    "请传入 --file=<path>，或者针对已知文件的锚点执行 annotate。",
                ),
            }),
            1,
        );
    }

    let abs_file = resolve_from_root(root, &file);
    if !is_within_root(root, &abs_file) {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "annotate",
                "error": "security_entry_outside_root",
                "file": file,
            }),
            1,
        );
    }
    file = normalize_relative(root, &abs_file);

    let kind_raw = args.get("kind").unwrap_or("comment").trim().to_lowercase();
    let kind = if ["comment", "doc", "todo", "decision"].contains(&kind_raw.as_str()) {
        kind_raw
    } else {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "annotate",
                "error": "invalid_kind",
                "kind": kind_raw,
                "allowed": ["comment", "doc", "todo", "decision"],
            }),
            1,
        );
    };

    let source = args.get("source").unwrap_or("annotate").trim().to_string();
    let ts = now_iso();
    let mut updated = false;

    for entry in &mut doc.entries {
        if entry.file == file && entry.ref_id.eq_ignore_ascii_case(&canonical_ref) {
            entry.anchor_id = Some(parsed.anchor_id.clone());
            entry.kind = Some(kind.clone());
            entry.text = Some(text.clone());
            entry.source = Some(source.clone());
            entry.created_at = Some(ts.clone());
            updated = true;
            break;
        }
    }

    if !updated {
        doc.entries.push(EntryRecord {
            ref_id: canonical_ref.clone(),
            file: file.clone(),
            anchor_id: Some(parsed.anchor_id.clone()),
            kind: Some(kind.clone()),
            text: Some(text.clone()),
            created_at: Some(ts.clone()),
            source: Some(source.clone()),
        });
    }

    if !doc.covers.iter().any(|c| c == &file) {
        doc.covers.push(file.clone());
    }

    save_doc(&lensmap_path, doc.clone());

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "annotate",
        "lensmap": normalize_relative(root, &lensmap_path),
        "ref": canonical_ref,
        "file": file,
        "kind": kind,
        "anchor_id": anchor.id,
        "anchor_created": created_anchor,
        "symbol": anchor.symbol,
        "symbol_path": anchor.symbol_path,
        "updated_existing": updated,
        "ts": ts,
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_merge(root: &Path, args: &ParsedArgs) {
    let dry_run = args.has("dry-run");
    let lensmap_path = resolve_lensmap_path(root, args, None);
    if !lensmap_path.exists() {
        emit(
            lensmap_missing_payload(root, "merge", &lensmap_path, args),
            1,
        );
    }

    let doc = load_doc(&lensmap_path, "group");
    let covers = normalize_covers(args, &doc, &[]);

    let mut file_set = BTreeMap::new();
    for c in &covers {
        for f in resolve_covers_to_files(root, std::slice::from_ref(c)) {
            file_set.insert(f, true);
        }
    }
    for a in &doc.anchors {
        if !a.file.is_empty() {
            file_set.insert(a.file.clone(), true);
        }
    }
    for e in &doc.entries {
        if !e.file.is_empty() {
            file_set.insert(e.file.clone(), true);
        }
    }

    let files = file_set.keys().cloned().collect::<Vec<_>>();
    if files.is_empty() {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "merge",
                "error": "no_files_resolved",
                "covers": covers,
                "hint": tr(
                    "No files to merge. Ensure covers/entries reference real files.",
                    "没有可合并的文件。请确认 covers/entries 引用了真实文件。",
                ),
            }),
            1,
        );
    }

    let mut entry_map = HashMap::new();
    for entry in &doc.entries {
        let key = format!("{}::{}", entry.file, entry.ref_id.to_uppercase());
        entry_map.insert(key, entry.clone());
    }

    let mut merged_total = 0usize;
    let mut missing_entry_refs = vec![];
    let mut file_summaries = vec![];

    for rel in files {
        let abs = resolve_from_root(root, &rel);
        if !is_within_root(root, &abs) || !abs.exists() {
            continue;
        }

        let style = comment_style_for(&abs);
        let mut lines = split_lines(&fs::read_to_string(&abs).unwrap_or_default());
        let mut changed = false;
        let mut merged_in_file = 0usize;
        let mut i = 0usize;

        while i < lines.len() {
            let site = if let Some(site) = locate_ref_site(&lines[i], style.line) {
                site
            } else {
                i += 1;
                continue;
            };

            let entry_key = format!("{}::{}", rel, site.ref_id.to_uppercase());
            let entry = if let Some(entry) = entry_map.get(&entry_key) {
                entry.clone()
            } else {
                missing_entry_refs.push(format!("{}:{}", rel, site.ref_id));
                i += 1;
                continue;
            };

            let entry_text = entry.text.unwrap_or_default();
            if entry_text.trim().is_empty() {
                i += 1;
                continue;
            }

            if site.is_inline {
                let first_line = entry_text.lines().next().unwrap_or("").trim();
                if first_line.is_empty() {
                    i += 1;
                    continue;
                }
                lines[i] = format!(
                    "{}{}{} {}",
                    site.prefix, site.spacing, style.line, first_line
                );
            } else {
                let hydrated = line_comment_lines(&site.indent, style.line, &entry_text);
                lines[i] = hydrated[0].clone();
                if hydrated.len() > 1 {
                    for (offset, extra) in hydrated.iter().skip(1).enumerate() {
                        lines.insert(i + 1 + offset, extra.clone());
                    }
                    i += hydrated.len() - 1;
                }
            }

            changed = true;
            merged_total += 1;
            merged_in_file += 1;
            i += 1;
        }

        if changed && !dry_run {
            let _ = fs::write(&abs, join_lines(&lines));
        }

        file_summaries.push(json!({
            "file": rel,
            "merged_refs": merged_in_file,
            "changed": changed,
        }));
    }

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "merge",
        "dry_run": dry_run,
        "lensmap": normalize_relative(root, &lensmap_path),
        "merged_refs": merged_total,
        "missing_entry_refs": missing_entry_refs,
        "ts": now_iso(),
        "files": file_summaries,
    });
    append_history(root, &out);
    emit(out, 0);
}

fn apply_dir_maps(original_rel: &str, maps: &[(String, String)]) -> Option<String> {
    for (from, to) in maps {
        if original_rel == from {
            return Some(to.clone());
        }
        let prefix = format!("{}/", from);
        if original_rel.starts_with(&prefix) {
            let suffix = &original_rel[from.len()..];
            let candidate = format!("{}{}", to, suffix);
            return Some(candidate.trim_start_matches('/').to_string());
        }
    }
    None
}

fn cmd_package(root: &Path, args: &ParsedArgs) {
    let dry_run = args.has("dry-run");
    let bundle_dir = args
        .get("bundle-dir")
        .unwrap_or(".lenspack")
        .trim()
        .to_string();
    let bundle_dir = if bundle_dir.is_empty() {
        ".lenspack".to_string()
    } else {
        bundle_dir
    };
    let mode = args.get("mode").unwrap_or("move").trim().to_lowercase();
    let keep_source = match mode.as_str() {
        "move" => false,
        "copy" => true,
        _ => {
            emit(
                json!({
                    "ok": false,
                    "type": "lensmap",
                    "action": "package",
                    "error": "invalid_mode",
                    "mode": mode,
                    "allowed": ["move", "copy"],
                }),
                1,
            );
        }
    };

    let bundle_abs = resolve_from_root(root, &bundle_dir);
    if !is_within_root(root, &bundle_abs) {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "package",
                "error": "security_bundle_outside_root",
                "bundle_dir": bundle_dir,
            }),
            1,
        );
    }

    let mut candidate_files = split_csv(args.get("lensmaps"));
    if candidate_files.is_empty() {
        if let Some(single) = args.get("lensmap") {
            candidate_files.push(single.to_string());
        }
    }
    if candidate_files.is_empty() {
        candidate_files = discover_lensmap_files(root, &bundle_dir);
    }
    if candidate_files.is_empty() {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "package",
                "error": "no_lensmap_files_found",
                "hint": tr(
                    "Create a lensmap first or pass --lensmaps=<file1,file2>.",
                    "请先创建 lensmap，或传入 --lensmaps=<file1,file2>。",
                ),
            }),
            1,
        );
    }

    let files_dir = bundle_abs.join("files");
    let manifest_path = bundle_abs.join("manifest.json");
    let mut manifest = load_package_manifest(&manifest_path, root, &bundle_dir);

    let mut by_original = HashMap::new();
    for (idx, item) in manifest.items.iter().enumerate() {
        by_original.insert(item.original_path.clone(), idx);
    }

    let mut packaged_count = 0usize;
    let mut skipped = vec![];
    let mut file_summaries = vec![];

    for raw in candidate_files {
        let abs = resolve_from_root(root, &raw);
        if !is_within_root(root, &abs) {
            skipped.push(format!("security_outside_root:{}", raw));
            continue;
        }
        if !abs.exists() {
            skipped.push(format!("missing:{}", raw));
            continue;
        }
        if abs.starts_with(&bundle_abs) {
            skipped.push(format!("inside_bundle:{}", raw));
            continue;
        }
        if !is_lensmap_filename(&abs) {
            skipped.push(format!("not_lensmap_file:{}", raw));
            continue;
        }

        let rel = normalize_relative(root, &abs);
        let id = hash_text(&rel).to_uppercase();
        let packaged_rel = format!("files/{}.lensmap.json", id);
        let packaged_abs = files_dir.join(format!("{}.lensmap.json", id));

        let mut status = "packaged".to_string();
        let mut err: Option<String> = None;
        if !dry_run {
            if let Err(e) = copy_or_move_file(&abs, &packaged_abs, keep_source) {
                status = "error".to_string();
                err = Some(e);
            }
        }

        if status == "packaged" {
            packaged_count += 1;
        }

        let item = PackageItem {
            id: id.clone(),
            original_path: rel.clone(),
            packaged_path: packaged_rel.clone(),
            status: if status == "packaged" && keep_source {
                "packaged_copy".to_string()
            } else {
                status.clone()
            },
            resolved_path: None,
            last_error: err.clone(),
            updated_at: Some(now_iso()),
        };

        if let Some(idx) = by_original.get(&rel).copied() {
            manifest.items[idx] = item;
        } else {
            by_original.insert(rel.clone(), manifest.items.len());
            manifest.items.push(item);
        }

        file_summaries.push(json!({
            "file": rel,
            "packaged_as": packaged_rel,
            "status": status,
            "error": err,
        }));
    }

    if !dry_run {
        let _ = fs::create_dir_all(&files_dir);
        save_package_manifest(&manifest_path, manifest.clone(), root, &bundle_dir);
    }

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "package",
        "dry_run": dry_run,
        "mode": mode,
        "bundle_dir": normalize_relative(root, &bundle_abs),
        "manifest": normalize_relative(root, &manifest_path),
        "packaged_count": packaged_count,
        "skipped": skipped,
        "ts": now_iso(),
        "files": file_summaries,
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_unpackage(root: &Path, args: &ParsedArgs) {
    let dry_run = args.has("dry-run");
    let overwrite = args.has("overwrite");
    let bundle_dir = args
        .get("bundle-dir")
        .unwrap_or(".lenspack")
        .trim()
        .to_string();
    let bundle_dir = if bundle_dir.is_empty() {
        ".lenspack".to_string()
    } else {
        bundle_dir
    };
    let on_missing = args
        .get("on-missing")
        .unwrap_or("prompt")
        .trim()
        .to_lowercase();
    if !["prompt", "skip", "error"].contains(&on_missing.as_str()) {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "unpackage",
                "error": "invalid_on_missing",
                "on_missing": on_missing,
                "allowed": ["prompt", "skip", "error"],
            }),
            1,
        );
    }

    let bundle_abs = resolve_from_root(root, &bundle_dir);
    let manifest_path = bundle_abs.join("manifest.json");
    if !manifest_path.exists() {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "unpackage",
                "error": "manifest_missing",
                "manifest": normalize_relative(root, &manifest_path),
                "hint": tr(
                    "Run lensmap package first.",
                    "请先运行 lensmap package。",
                ),
            }),
            1,
        );
    }

    let dir_maps = parse_dir_map(args.get("map"));
    let mut manifest = load_package_manifest(&manifest_path, root, &bundle_dir);
    let mut unpacked_count = 0usize;
    let mut skipped_count = 0usize;
    let mut error_count = 0usize;
    let mut non_interactive_prompt_skips = 0usize;
    let mut file_summaries = vec![];

    for item in &mut manifest.items {
        let packaged_abs = bundle_abs.join(&item.packaged_path);
        if !packaged_abs.exists() {
            continue;
        }

        let mut dest_rel = item.original_path.clone();
        if let Some(mapped) = apply_dir_maps(&item.original_path, &dir_maps) {
            dest_rel = mapped;
        }

        let mut dest_abs = resolve_from_root(root, &dest_rel);
        if !is_within_root(root, &dest_abs) {
            item.status = "error".to_string();
            item.last_error = Some("security_target_outside_root".to_string());
            item.updated_at = Some(now_iso());
            error_count += 1;
            file_summaries.push(json!({
                "original_path": item.original_path,
                "status": "error",
                "error": "security_target_outside_root",
            }));
            continue;
        }

        let mut parent_exists = dest_abs.parent().map(|p| p.exists()).unwrap_or(false);
        if !parent_exists {
            match on_missing.as_str() {
                "skip" => {
                    item.status = "packaged".to_string();
                    item.last_error = Some("missing_dir_skipped".to_string());
                    item.updated_at = Some(now_iso());
                    skipped_count += 1;
                    file_summaries.push(json!({
                        "original_path": item.original_path,
                        "status": "skipped",
                        "reason": "missing_dir",
                    }));
                    continue;
                }
                "error" => {
                    item.status = "error".to_string();
                    item.last_error = Some("missing_dir_error".to_string());
                    item.updated_at = Some(now_iso());
                    error_count += 1;
                    file_summaries.push(json!({
                        "original_path": item.original_path,
                        "status": "error",
                        "error": "missing_dir_error",
                    }));
                    continue;
                }
                "prompt" => {
                    let parent_rel = Path::new(&dest_rel)
                        .parent()
                        .map(to_posix_str)
                        .unwrap_or_else(|| ".".to_string());
                    let prompt = tr(
                        &format!(
                            "Missing dir for {} ({}). Enter new directory path, or 'skip': ",
                            item.original_path, parent_rel
                        ),
                        &format!(
                            "{} 缺少目标目录（{}）。请输入新的目录路径，或输入 'skip' 跳过：",
                            item.original_path, parent_rel
                        ),
                    );
                    let response = prompt_line(&prompt);
                    if response.is_none() {
                        item.status = "packaged".to_string();
                        item.last_error =
                            Some("missing_dir_prompt_non_interactive_skipped".to_string());
                        item.updated_at = Some(now_iso());
                        skipped_count += 1;
                        non_interactive_prompt_skips += 1;
                        file_summaries.push(json!({
                            "original_path": item.original_path,
                            "status": "skipped",
                            "reason": "non_interactive_prompt",
                        }));
                        continue;
                    }
                    let response = response.unwrap_or_default();
                    if response.is_empty() || response.eq_ignore_ascii_case("skip") {
                        item.status = "packaged".to_string();
                        item.last_error = Some("missing_dir_prompt_skipped".to_string());
                        item.updated_at = Some(now_iso());
                        skipped_count += 1;
                        file_summaries.push(json!({
                            "original_path": item.original_path,
                            "status": "skipped",
                            "reason": "prompt_skip",
                        }));
                        continue;
                    }
                    let new_parent_abs = resolve_from_root(root, &response);
                    if !is_within_root(root, &new_parent_abs) {
                        item.status = "error".to_string();
                        item.last_error = Some("security_prompt_target_outside_root".to_string());
                        item.updated_at = Some(now_iso());
                        error_count += 1;
                        file_summaries.push(json!({
                            "original_path": item.original_path,
                            "status": "error",
                            "error": "security_prompt_target_outside_root",
                        }));
                        continue;
                    }

                    let filename = Path::new(&item.original_path)
                        .file_name()
                        .and_then(|v| v.to_str())
                        .unwrap_or("lensmap.json");
                    dest_abs = new_parent_abs.join(filename);
                    dest_rel = normalize_relative(root, &dest_abs);
                    parent_exists = true;
                }
                _ => {}
            }
        }

        if !parent_exists && !dry_run {
            if let Some(parent) = dest_abs.parent() {
                let _ = fs::create_dir_all(parent);
            }
        }

        if dest_abs.exists() && !overwrite {
            item.status = "error".to_string();
            item.last_error = Some("target_exists_use_overwrite".to_string());
            item.updated_at = Some(now_iso());
            error_count += 1;
            file_summaries.push(json!({
                "original_path": item.original_path,
                "status": "error",
                "error": "target_exists_use_overwrite",
                "target": dest_rel,
            }));
            continue;
        }

        if !dry_run {
            if let Err(e) = copy_or_move_file(&packaged_abs, &dest_abs, false) {
                item.status = "error".to_string();
                item.last_error = Some(e.clone());
                item.updated_at = Some(now_iso());
                error_count += 1;
                file_summaries.push(json!({
                    "original_path": item.original_path,
                    "status": "error",
                    "error": e,
                    "target": dest_rel,
                }));
                continue;
            }
        }

        item.status = "unpacked".to_string();
        item.resolved_path = Some(dest_rel.clone());
        item.last_error = None;
        item.updated_at = Some(now_iso());
        unpacked_count += 1;
        file_summaries.push(json!({
            "original_path": item.original_path,
            "status": "unpacked",
            "target": dest_rel,
        }));
    }

    if !dry_run {
        save_package_manifest(&manifest_path, manifest.clone(), root, &bundle_dir);
    }

    let remaining_packaged = manifest
        .items
        .iter()
        .filter(|i| bundle_abs.join(&i.packaged_path).exists())
        .count();

    let ok = error_count == 0;
    let out = json!({
        "ok": ok,
        "type": "lensmap",
        "action": "unpackage",
        "dry_run": dry_run,
        "bundle_dir": normalize_relative(root, &bundle_abs),
        "manifest": normalize_relative(root, &manifest_path),
        "on_missing": on_missing,
        "overwrite": overwrite,
        "unpacked_count": unpacked_count,
        "skipped_count": skipped_count,
        "error_count": error_count,
        "remaining_packaged": remaining_packaged,
        "non_interactive_prompt_skips": non_interactive_prompt_skips,
        "ts": now_iso(),
        "files": file_summaries,
    });
    append_history(root, &out);
    emit(out, if ok { 0 } else { 1 });
}

fn cmd_validate(root: &Path, args: &ParsedArgs) {
    let lensmap_path = resolve_lensmap_path(root, args, None);
    if !lensmap_path.exists() {
        emit(
            lensmap_missing_payload(root, "validate", &lensmap_path, args),
            1,
        );
    }

    let doc = load_doc(&lensmap_path, "group");
    let lensmap_rel = normalize_relative(root, &lensmap_path);
    let lensmap_dirty = git_is_dirty(root, &lensmap_rel);
    let mut errors = vec![];
    let mut warnings = vec![];

    if doc.doc_type != "lensmap" {
        errors.push("invalid_type".to_string());
    }

    let mut anchor_by_id: HashMap<String, AnchorRecord> = HashMap::new();
    let mut dup_anchor_ids = HashSet::new();

    let id_re = Regex::new(r"^[A-F0-9]{6,16}$").unwrap();

    for anchor in &doc.anchors {
        let id = anchor.id.to_uppercase();
        if !id_re.is_match(&id) {
            errors.push(format!(
                "invalid_anchor_id:{}",
                if id.is_empty() { "<empty>" } else { &id }
            ));
            continue;
        }
        if anchor_by_id.contains_key(&id) {
            dup_anchor_ids.insert(id.clone());
        }
        anchor_by_id.insert(id.clone(), anchor.clone());

        let abs = resolve_from_root(root, &anchor.file);
        if !is_within_root(root, &abs) {
            errors.push(format!("security_anchor_outside_root:{}", id));
            continue;
        }
        if !abs.exists() {
            warnings.push(format!("anchor_file_missing:{}:{}", id, anchor.file));
            continue;
        }

        let style = comment_style_for(&abs);
        let expected_marker = style.line;
        let content = fs::read_to_string(&abs).unwrap_or_default();
        let lines = split_lines(&content);
        let fns = detect_functions(&lines, &abs);
        let resolution = resolve_anchor_in_lines(anchor, &lines, &fns, &style);
        if resolution.anchor_line_index.is_none() {
            errors.push(format!("anchor_unresolved:{}:{}", id, anchor.file));
            continue;
        }
        let anchor_line = resolution.anchor_line_index.unwrap_or_default();

        if !resolution.anchor_found_in_source {
            warnings.push(format!(
                "anchor_missing_in_source:{}:{}:{}",
                id, anchor.file, resolution.strategy
            ));
        } else if resolution.strategy != "anchor_id" {
            warnings.push(format!(
                "anchor_resolved_via:{}:{}:{}",
                id, anchor.file, resolution.strategy
            ));
        }

        if anchor_line < lines.len() {
            if let Some(resolved) = anchor_match(&lines[anchor_line], None) {
                if resolved.marker != expected_marker {
                    errors.push(format!(
                        "anchor_marker_mismatch:{}:{}:{}->{}",
                        id, anchor.file, resolved.marker, expected_marker
                    ));
                }
            }
        }

        let dirty_ranges = git_dirty_ranges(root, &anchor.file);
        if !dirty_ranges.is_empty() {
            if anchor_overlaps_dirty_ranges(anchor, &dirty_ranges)
                && !resolution.anchor_found_in_source
            {
                warnings.push(format!("git_dirty_overlap:{}:{}", id, anchor.file));
            }
            if lensmap_dirty {
                warnings.push(format!("git_dual_edit_conflict:{}:{}", id, anchor.file));
            }
        }

        if let Some(fn_hit) = resolution
            .function_hit
            .or_else(|| fns.iter().find(|f| f.line_index >= anchor_line).cloned())
        {
            if let Some(symbol_path) = &anchor.symbol_path {
                if symbol_path != &fn_hit.symbol_path {
                    warnings.push(format!("symbol_path_drift:{}", id));
                }
            }
            if let Some(fp) = &anchor.fingerprint {
                if fp != &fn_hit.fingerprint {
                    warnings.push(format!("fingerprint_drift:{}", id));
                }
            }
            if let (Some(span_start), Some(span_end)) = (anchor.span_start, anchor.span_end) {
                if span_start != fn_hit.line_index + 1 || span_end != fn_hit.span_end_index + 1 {
                    warnings.push(format!("span_drift:{}", id));
                }
            }
        } else {
            warnings.push(format!("anchor_without_function:{}:{}", id, anchor.file));
        }
    }

    for id in dup_anchor_ids {
        errors.push(format!("duplicate_anchor_id:{}", id));
    }

    let mut collision_set = HashSet::new();
    for entry in &doc.entries {
        let parsed = parse_ref(&entry.ref_id);
        if parsed.is_none() {
            errors.push(format!("invalid_ref:{}", entry.ref_id));
            continue;
        }
        let parsed = parsed.unwrap();

        let key = format!("{}::{}", entry.file, entry.ref_id);
        if collision_set.contains(&key) {
            errors.push(format!("comment_collision:{}", key));
        }
        collision_set.insert(key);

        let anchor = anchor_by_id.get(&parsed.anchor_id);
        if anchor.is_none() {
            errors.push(format!("entry_anchor_missing:{}", entry.ref_id));
            continue;
        }
        let anchor = anchor.unwrap();

        if !entry.file.is_empty() && !anchor.file.is_empty() && entry.file != anchor.file {
            warnings.push(format!(
                "entry_file_anchor_mismatch:{}:{}:{}",
                entry.ref_id, entry.file, anchor.file
            ));
        }

        if parsed.end < parsed.start {
            errors.push(format!("invalid_ref_range:{}", entry.ref_id));
        }

        let target_file = if !entry.file.is_empty() {
            entry.file.clone()
        } else {
            anchor.file.clone()
        };

        let abs = resolve_from_root(root, &target_file);
        if !is_within_root(root, &abs) {
            errors.push(format!("security_entry_outside_root:{}", entry.ref_id));
            continue;
        }
        if !abs.exists() {
            warnings.push(format!(
                "entry_file_missing:{}:{}",
                entry.ref_id, target_file
            ));
            continue;
        }

        let style = comment_style_for(&abs);
        let expected_marker = style.line;
        let lines = split_lines(&fs::read_to_string(&abs).unwrap_or_default());
        let fns = detect_functions(&lines, &abs);
        let resolution = resolve_anchor_in_lines(anchor, &lines, &fns, &style);
        if resolution.anchor_line_index.is_none() {
            warnings.push(format!("entry_anchor_unresolved:{}", entry.ref_id));
            continue;
        }
        let Some((start_line, end_line)) = entry_line_indexes(anchor, Some(&resolution), &parsed)
        else {
            warnings.push(format!("entry_line_out_of_range:{}", entry.ref_id));
            continue;
        };

        if start_line >= lines.len() || end_line >= lines.len() {
            warnings.push(format!("entry_line_out_of_range:{}", entry.ref_id));
            continue;
        }

        if let Some(rm) = ref_match(&lines[start_line], None) {
            if rm.ref_id == entry.ref_id.to_uppercase() && rm.marker != expected_marker {
                errors.push(format!(
                    "ref_marker_mismatch:{}:{}:{}->{}",
                    entry.ref_id, target_file, rm.marker, expected_marker
                ));
            }
        }
    }

    let ok = errors.is_empty();
    let out = json!({
        "ok": ok,
        "type": "lensmap",
        "action": "validate",
        "lensmap": normalize_relative(root, &lensmap_path),
        "git": {
            "lensmap_dirty": lensmap_dirty,
        },
        "errors": errors,
        "warnings": warnings,
        "stats": {
            "covers": doc.covers.len(),
            "anchors": doc.anchors.len(),
            "entries": doc.entries.len(),
        },
        "ts": now_iso(),
    });
    append_history(root, &out);
    emit(out, if ok { 0 } else { 1 });
}

fn reanchor_doc(
    root: &Path,
    doc: &mut LensMapDoc,
    dry_run: bool,
    lensmap_dirty: bool,
) -> (usize, usize, Vec<Value>) {
    let mut unresolved: Vec<Value> = vec![];
    let mut resolved = 0usize;
    let mut inserted = 0usize;
    let preferred_placement =
        preferred_anchor_placement(doc, None).unwrap_or(AnchorPlacement::Inline);

    let mut file_to_indices: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, anchor) in doc.anchors.iter().enumerate() {
        file_to_indices
            .entry(anchor.file.clone())
            .or_default()
            .push(idx);
    }

    for (file, indices) in file_to_indices {
        let abs = resolve_from_root(root, &file);
        if !is_within_root(root, &abs) || !abs.exists() {
            for idx in indices {
                unresolved.push(json!({
                    "id": doc.anchors[idx].id,
                    "reason": "file_missing",
                    "file": file,
                }));
            }
            continue;
        }

        let style = comment_style_for(&abs);
        let mut lines = split_lines(&fs::read_to_string(&abs).unwrap_or_default());
        let mut file_changed = false;
        let dirty_ranges = git_dirty_ranges(root, &file);
        let source_dirty = !dirty_ranges.is_empty();

        for idx in indices {
            let id = doc.anchors[idx].id.to_uppercase();
            if id.is_empty() {
                unresolved.push(json!({"id": "<empty>", "reason": "invalid_id", "file": file}));
                continue;
            }

            let functions = detect_functions(&lines, &abs);
            let resolution = resolve_anchor_in_lines(&doc.anchors[idx], &lines, &functions, &style);
            if resolution.anchor_found_in_source {
                doc.anchors[idx].line_anchor = resolution.anchor_line_index.map(|v| v + 1);
                doc.anchors[idx].line_symbol =
                    resolution.function_hit.as_ref().map(|f| f.line_index + 1);
                doc.anchors[idx].symbol =
                    resolution.function_hit.as_ref().map(|f| f.symbol.clone());
                doc.anchors[idx].symbol_path = resolution
                    .function_hit
                    .as_ref()
                    .map(|f| f.symbol_path.clone());
                doc.anchors[idx].span_start =
                    resolution.function_hit.as_ref().map(|f| f.line_index + 1);
                doc.anchors[idx].span_end = resolution
                    .function_hit
                    .as_ref()
                    .map(|f| f.span_end_index + 1);
                doc.anchors[idx].fingerprint = resolution
                    .function_hit
                    .as_ref()
                    .map(|f| f.fingerprint.clone());
                doc.anchors[idx].signature_text = resolution
                    .function_hit
                    .as_ref()
                    .map(|f| f.signature_text.clone());
                doc.anchors[idx].updated_at = Some(now_iso());
                resolved += 1;
                continue;
            }

            if source_dirty && anchor_overlaps_dirty_ranges(&doc.anchors[idx], &dirty_ranges) {
                unresolved.push(json!({
                    "id": id,
                    "reason": if lensmap_dirty { "git_dual_edit_conflict" } else { "git_dirty_overlap" },
                    "file": file,
                    "strategy": resolution.strategy,
                }));
                continue;
            }

            let fn_hit = if let Some(fn_hit) = resolution.function_hit.clone() {
                fn_hit
            } else if let Some(projected) =
                git_projected_function_hit(root, &file, &doc.anchors[idx], &functions)
            {
                projected
            } else {
                unresolved.push(json!({
                    "id": id,
                    "reason": format!("{}_not_found", resolution.strategy),
                    "file": file,
                }));
                continue;
            };

            if let Some((existing, marker, line_idx, _is_inline)) =
                find_anchor_before_function(&lines, fn_hit.line_index, &style)
            {
                if existing != id {
                    unresolved.push(json!({
                        "id": id,
                        "reason": format!("anchor_conflict_with_{}", existing),
                        "file": file,
                    }));
                    continue;
                }
                if marker != style.line {
                    lines[line_idx] = rewrite_anchor_on_line(&lines[line_idx], style.line, &id);
                    file_changed = true;
                }
            } else {
                place_anchor_for_function(&mut lines, &fn_hit, &style, &id, preferred_placement);
                file_changed = true;
                inserted += 1;
            }

            let refreshed_functions = detect_functions(&lines, &abs);
            let refreshed =
                resolve_anchor_in_lines(&doc.anchors[idx], &lines, &refreshed_functions, &style);
            if refreshed.anchor_line_index.is_none() {
                unresolved.push(json!({
                    "id": id,
                    "reason": "inserted_but_not_resolved",
                    "file": file,
                }));
                continue;
            }

            doc.anchors[idx].line_anchor = refreshed.anchor_line_index.map(|v| v + 1);
            doc.anchors[idx].line_symbol =
                refreshed.function_hit.as_ref().map(|f| f.line_index + 1);
            doc.anchors[idx].symbol = refreshed.function_hit.as_ref().map(|f| f.symbol.clone());
            doc.anchors[idx].symbol_path = refreshed
                .function_hit
                .as_ref()
                .map(|f| f.symbol_path.clone());
            doc.anchors[idx].span_start = refreshed.function_hit.as_ref().map(|f| f.line_index + 1);
            doc.anchors[idx].span_end = refreshed
                .function_hit
                .as_ref()
                .map(|f| f.span_end_index + 1);
            doc.anchors[idx].fingerprint = refreshed
                .function_hit
                .as_ref()
                .map(|f| f.fingerprint.clone());
            doc.anchors[idx].signature_text = refreshed
                .function_hit
                .as_ref()
                .map(|f| f.signature_text.clone());
            doc.anchors[idx].updated_at = Some(now_iso());
            resolved += 1;
        }

        if file_changed && !dry_run {
            let _ = fs::write(&abs, join_lines(&lines));
        }
    }

    (resolved, inserted, unresolved)
}

fn cmd_reanchor(root: &Path, args: &ParsedArgs) {
    let dry_run = args.has("dry-run");
    let lensmap_path = resolve_lensmap_path(root, args, None);
    if !lensmap_path.exists() {
        emit(
            lensmap_missing_payload(root, "reanchor", &lensmap_path, args),
            1,
        );
    }

    let mut doc = load_doc(&lensmap_path, "group");
    let lensmap_rel = normalize_relative(root, &lensmap_path);
    let lensmap_dirty = git_is_dirty(root, &lensmap_rel);
    let (resolved, inserted, unresolved) = reanchor_doc(root, &mut doc, dry_run, lensmap_dirty);

    if !dry_run {
        save_doc(&lensmap_path, doc.clone());
    }

    let ok = unresolved.is_empty();
    let out = json!({
        "ok": ok,
        "type": "lensmap",
        "action": "reanchor",
        "dry_run": dry_run,
        "lensmap": normalize_relative(root, &lensmap_path),
        "git": {
            "lensmap_dirty": lensmap_dirty,
        },
        "resolved": resolved,
        "inserted": inserted,
        "unresolved": unresolved,
        "ts": now_iso(),
    });
    append_history(root, &out);
    emit(out, if ok { 0 } else { 1 });
}

fn simplify_doc_in_place(doc: &mut LensMapDoc) -> (usize, usize) {
    let before_anchors = doc.anchors.len();
    let before_entries = doc.entries.len();

    let mut anchor_map = BTreeMap::new();
    for anchor in doc.anchors.drain(..) {
        if !anchor.id.trim().is_empty() {
            anchor_map.insert(anchor.id.to_uppercase(), anchor);
        }
    }

    let mut entry_map = BTreeMap::new();
    for entry in doc.entries.drain(..) {
        let key = format!("{}::{}", entry.file, entry.ref_id.to_uppercase());
        entry_map.insert(key, entry);
    }

    let mut anchors = anchor_map.values().cloned().collect::<Vec<_>>();
    anchors.sort_by(|a, b| {
        if a.file != b.file {
            return a.file.cmp(&b.file);
        }
        a.line_anchor.unwrap_or(0).cmp(&b.line_anchor.unwrap_or(0))
    });

    let mut entries = entry_map.values().cloned().collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        if a.file != b.file {
            return a.file.cmp(&b.file);
        }
        a.ref_id.cmp(&b.ref_id)
    });

    doc.anchors = anchors;
    doc.entries = entries;

    (
        before_anchors.saturating_sub(doc.anchors.len()),
        before_entries.saturating_sub(doc.entries.len()),
    )
}

fn default_show_output_path(lensmap_path: &Path) -> PathBuf {
    let parent = lensmap_path
        .parent()
        .map(normalize_path)
        .unwrap_or_else(|| PathBuf::from("."));
    let filename = lensmap_path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("lensmap.json");
    let stem = filename.strip_suffix(".json").unwrap_or(filename);
    parent.join(format!("{}.show.md", stem))
}

fn build_render_lines(
    root: &Path,
    lensmap_path: &Path,
    doc: &LensMapDoc,
    filters: RenderFilters<'_>,
    title: &str,
) -> (Vec<String>, usize, usize) {
    let mut files: BTreeMap<String, bool> = BTreeMap::new();
    for cover in &doc.covers {
        for file in resolve_covers_to_files(root, std::slice::from_ref(cover)) {
            files.insert(file, true);
        }
    }
    for anchor in &doc.anchors {
        if !anchor.file.is_empty() {
            files.insert(anchor.file.clone(), true);
        }
    }
    for entry in &doc.entries {
        if !entry.file.is_empty() {
            files.insert(entry.file.clone(), true);
        }
    }

    let mut anchor_map = HashMap::new();
    for anchor in &doc.anchors {
        anchor_map.insert(anchor.id.to_uppercase(), anchor.clone());
    }

    let file_filter = filters.file.map(str::trim).filter(|v| !v.is_empty());
    let symbol_filter = filters.symbol.map(str::trim).filter(|v| !v.is_empty());
    let ref_filter = filters
        .ref_id
        .map(|v| v.trim().to_uppercase())
        .filter(|v| !v.is_empty());
    let kind_filter = filters.kind.map(str::trim).filter(|v| !v.is_empty());

    let mut lines = vec![];
    lines.push(format!("# {}", title));
    lines.push(String::new());
    lines.push(format!(
        "- {} `{}`",
        tr("Source:", "来源："),
        normalize_relative(root, lensmap_path)
    ));
    lines.push(format!(
        "- {} {}",
        tr("Generated:", "生成时间："),
        now_iso()
    ));
    lines.push(format!(
        "- {} {}",
        tr("Positioning:", "定位："),
        doc.metadata
            .get("positioning")
            .and_then(Value::as_str)
            .unwrap_or("external-doc-layer")
    ));
    if let Some(file) = file_filter {
        lines.push(format!("- {} `{}`", tr("File filter:", "文件过滤："), file));
    }
    if let Some(symbol) = symbol_filter {
        lines.push(format!(
            "- {} `{}`",
            tr("Symbol filter:", "符号过滤："),
            symbol
        ));
    }
    if let Some(ref_id) = &ref_filter {
        lines.push(format!(
            "- {} `{}`",
            tr("Ref filter:", "引用过滤："),
            ref_id
        ));
    }
    if let Some(kind) = kind_filter {
        lines.push(format!("- {} `{}`", tr("Kind filter:", "类型过滤："), kind));
    }
    lines.push(String::new());

    let mut files_rendered = 0usize;
    let mut entries_rendered = 0usize;

    for (rel, _) in files {
        if let Some(filter) = file_filter {
            if rel != filter {
                continue;
            }
        }

        let abs = resolve_from_root(root, &rel);
        if !is_within_root(root, &abs) || !abs.exists() {
            continue;
        }
        let content = fs::read_to_string(&abs).unwrap_or_default();
        let file_lines = split_lines(&content);
        let lang = ext_of(&abs).trim_start_matches('.').to_string();
        let style = comment_style_for(&abs);
        let functions = detect_functions(&file_lines, &abs);

        let mut file_anchors = doc
            .anchors
            .iter()
            .filter(|anchor| anchor.file == rel)
            .filter(|anchor| {
                if let Some(symbol) = symbol_filter {
                    return anchor.symbol.as_deref() == Some(symbol)
                        || anchor.symbol_path.as_deref() == Some(symbol);
                }
                true
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut resolutions = HashMap::new();
        for anchor in &file_anchors {
            resolutions.insert(
                anchor.id.to_uppercase(),
                resolve_anchor_in_lines(anchor, &file_lines, &functions, &style),
            );
        }
        file_anchors.sort_by(|a, b| {
            let left = resolutions
                .get(&a.id.to_uppercase())
                .and_then(|r| r.anchor_line_index)
                .unwrap_or(a.line_anchor.unwrap_or(0).saturating_sub(1));
            let right = resolutions
                .get(&b.id.to_uppercase())
                .and_then(|r| r.anchor_line_index)
                .unwrap_or(b.line_anchor.unwrap_or(0).saturating_sub(1));
            left.cmp(&right)
        });

        let mut file_entries = vec![];
        for entry in doc.entries.iter().filter(|entry| entry.file == rel) {
            if let Some(kind) = kind_filter {
                if entry.kind.as_deref() != Some(kind) {
                    continue;
                }
            }
            if let Some(ref_id) = &ref_filter {
                if entry.ref_id.to_uppercase() != *ref_id {
                    continue;
                }
            }
            let parsed = if let Some(parsed) = parse_ref(&entry.ref_id) {
                parsed
            } else {
                continue;
            };
            let anchor = if let Some(anchor) = anchor_map.get(&parsed.anchor_id) {
                anchor
            } else {
                continue;
            };
            if let Some(symbol) = symbol_filter {
                if anchor.symbol.as_deref() != Some(symbol)
                    && anchor.symbol_path.as_deref() != Some(symbol)
                {
                    continue;
                }
            }
            let resolution = resolutions
                .entry(parsed.anchor_id.to_uppercase())
                .or_insert_with(|| {
                    resolve_anchor_in_lines(anchor, &file_lines, &functions, &style)
                });
            let (start, end) = entry_line_indexes(anchor, Some(&*resolution), &parsed)
                .map(|(start, end)| (Some(start + 1), Some(end + 1)))
                .unwrap_or((None, None));
            file_entries.push((
                entry.clone(),
                anchor.clone(),
                resolution.clone(),
                start,
                end,
            ));
        }
        file_entries.sort_by(|a, b| a.3.unwrap_or(0).cmp(&b.3.unwrap_or(0)));

        if file_anchors.is_empty() && file_entries.is_empty() {
            continue;
        }
        files_rendered += 1;

        lines.push(format!("## {}", rel));
        lines.push(String::new());
        lines.push(format!("### {}", tr("Anchors", "锚点")));
        if file_anchors.is_empty() {
            lines.push(format!("- {}", tr("none", "无")));
        } else {
            for anchor in &file_anchors {
                let resolution = resolutions
                    .get(&anchor.id.to_uppercase())
                    .cloned()
                    .unwrap_or(AnchorResolution {
                        anchor_line_index: anchor.line_anchor.map(|v| v.saturating_sub(1)),
                        function_hit: None,
                        strategy: "stored".to_string(),
                        anchor_found_in_source: false,
                    });
                let line_value = resolution
                    .anchor_line_index
                    .map(|idx| idx + 1)
                    .or(anchor.line_anchor)
                    .unwrap_or(0);
                let mut row = format!("- {} line {}", anchor.id, line_value);
                if let Some(symbol) = &anchor.symbol {
                    row.push_str(&format!(" symbol=`{}`", symbol));
                }
                if let Some(symbol_path) = &anchor.symbol_path {
                    row.push_str(&format!(" path=`{}`", symbol_path));
                }
                if let (Some(span_start), Some(span_end)) = (anchor.span_start, anchor.span_end) {
                    row.push_str(&format!(" span=`{}-{}`", span_start, span_end));
                }
                if let Some(fp) = &anchor.fingerprint {
                    row.push_str(&format!(" fingerprint=`{}`", fp));
                }
                row.push_str(&format!(" resolve=`{}`", resolution.strategy));
                if !resolution.anchor_found_in_source {
                    row.push_str(" source_anchor=missing");
                }
                lines.push(row);
            }
        }
        lines.push(String::new());

        lines.push(format!("### {}", tr("Entries", "条目")));
        if file_entries.is_empty() {
            lines.push(format!("- {}", tr("none", "无")));
            lines.push(String::new());
            continue;
        }

        for (entry, anchor, resolution, start, end) in file_entries {
            entries_rendered += 1;
            let label = if let Some(start_line) = start {
                if ui_lang() == UiLang::ZhCn {
                    if let Some(end_line) = end {
                        if end_line != start_line {
                            format!("第 {}-{} 行", start_line, end_line)
                        } else {
                            format!("第 {} 行", start_line)
                        }
                    } else {
                        format!("第 {} 行", start_line)
                    }
                } else if let Some(end_line) = end {
                    if end_line != start_line {
                        format!("line {}-{}", start_line, end_line)
                    } else {
                        format!("line {}", start_line)
                    }
                } else {
                    format!("line {}", start_line)
                }
            } else {
                tr("line ?", "第 ? 行")
            };

            lines.push(format!(
                "- [{}] ({}) {}: {}",
                entry.ref_id,
                label,
                entry.kind.unwrap_or_else(|| "comment".to_string()),
                entry.text.unwrap_or_default().replace('\n', " ").trim()
            ));
            lines.push(format!(
                "  anchor=`{}` symbol=`{}` path=`{}` resolve=`{}`",
                anchor.id,
                anchor.symbol.clone().unwrap_or_else(|| "?".to_string()),
                anchor
                    .symbol_path
                    .clone()
                    .unwrap_or_else(|| anchor.symbol.clone().unwrap_or_else(|| "?".to_string())),
                resolution.strategy
            ));

            if let Some(start_line) = start {
                let end_line = end.unwrap_or(start_line);
                let start_ctx = start_line.saturating_sub(1).max(1);
                let end_ctx = (end_line + 1).min(file_lines.len());
                lines.push(String::new());
                lines.push(format!(
                    "```{}",
                    if lang.is_empty() { "text" } else { &lang }
                ));
                for line_number in start_ctx..=end_ctx {
                    let body = file_lines.get(line_number - 1).cloned().unwrap_or_default();
                    lines.push(format!("{:>4} | {}", line_number, body));
                }
                lines.push("```".to_string());
                lines.push(String::new());
            }
        }

        lines.push(String::new());
    }

    if files_rendered == 0 {
        lines.push("No matching anchors or entries.".to_string());
        lines.push(String::new());
    }

    (lines, files_rendered, entries_rendered)
}

fn cmd_render(root: &Path, args: &ParsedArgs) {
    let lensmap_path = resolve_lensmap_path(root, args, None);
    if !lensmap_path.exists() {
        emit(
            lensmap_missing_payload(root, "render", &lensmap_path, args),
            1,
        );
    }

    let doc = load_doc(&lensmap_path, "group");
    let out_path = if let Some(out) = args.get("out") {
        resolve_from_root(root, out)
    } else {
        default_render_output_path(&lensmap_path)
    };
    let (lines, files_rendered, entries_rendered) = build_render_lines(
        root,
        &lensmap_path,
        &doc,
        RenderFilters {
            file: None,
            symbol: None,
            ref_id: None,
            kind: None,
        },
        &tr("LensMap Render", "LensMap 渲染视图"),
    );

    ensure_dir(&out_path);
    let _ = fs::write(&out_path, format!("{}\n", lines.join("\n")));

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "render",
        "lensmap": normalize_relative(root, &lensmap_path),
        "output": normalize_relative(root, &out_path),
        "files_rendered": files_rendered,
        "entries_rendered": entries_rendered,
        "ts": now_iso(),
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_simplify(root: &Path, args: &ParsedArgs) {
    let lensmap_path = resolve_lensmap_path(root, args, None);
    if !lensmap_path.exists() {
        emit(
            lensmap_missing_payload(root, "simplify", &lensmap_path, args),
            1,
        );
    }

    let mut doc = load_doc(&lensmap_path, "group");
    let (removed_anchors, removed_entries) = simplify_doc_in_place(&mut doc);
    save_doc(&lensmap_path, doc.clone());

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "simplify",
        "lensmap": normalize_relative(root, &lensmap_path),
        "removed_anchors": removed_anchors,
        "removed_entries": removed_entries,
        "retained_sections": ["covers", "anchors", "entries"],
        "ts": now_iso(),
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_polish(root: &Path) {
    let readme = root.join("README.md");
    let changelog = root.join("CHANGELOG.md");
    ensure_dir(&readme);
    if !readme.exists() {
        let _ = fs::write(
            &readme,
            "# LensMap\n\nInternal lens orchestration utility.\n",
        );
    }
    if !changelog.exists() {
        let _ = fs::write(
            &changelog,
            "# Changelog\n\n## 0.1.0\n- Initial internal release polish artifacts.\n",
        );
    }

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "polish",
        "files": [normalize_relative(root, &readme), normalize_relative(root, &changelog)],
        "ts": now_iso(),
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_import(root: &Path, args: &ParsedArgs) {
    let from = args
        .get("from")
        .or_else(|| args.get("path"))
        .unwrap_or("")
        .trim()
        .to_string();
    if from.is_empty() {
        emit(json!({"ok": false, "error": "from_required"}), 1);
    }
    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "import",
        "from": from,
        "ts": now_iso(),
        "diff_receipt": format!("import_{}", Utc::now().timestamp_millis()),
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_show(root: &Path, args: &ParsedArgs) {
    let lensmap_path = resolve_lensmap_path(root, args, None);
    if !lensmap_path.exists() {
        emit(
            lensmap_missing_payload(root, "show", &lensmap_path, args),
            1,
        );
    }
    let doc = load_doc(&lensmap_path, "group");
    let out_path = if let Some(out) = args.get("out").or_else(|| args.get("to")) {
        resolve_from_root(root, out)
    } else {
        default_show_output_path(&lensmap_path)
    };
    let (lines, files_rendered, entries_rendered) = build_render_lines(
        root,
        &lensmap_path,
        &doc,
        RenderFilters {
            file: args.get("file"),
            symbol: args.get("symbol"),
            ref_id: args.get("ref"),
            kind: args.get("kind"),
        },
        &tr("LensMap View", "LensMap 视图"),
    );
    ensure_dir(&out_path);
    let _ = fs::write(&out_path, format!("{}\n", lines.join("\n")));
    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "show",
        "lensmap": normalize_relative(root, &lensmap_path),
        "output": normalize_relative(root, &out_path),
        "file": args.get("file"),
        "symbol": args.get("symbol"),
        "ref": args.get("ref"),
        "kind": args.get("kind"),
        "files_rendered": files_rendered,
        "entries_rendered": entries_rendered,
        "ts": now_iso(),
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_sync(root: &Path, args: &ParsedArgs) {
    let lensmap_path = resolve_lensmap_path(root, args, None);
    if !lensmap_path.exists() {
        emit(
            lensmap_missing_payload(root, "sync", &lensmap_path, args),
            1,
        );
    }

    let mut doc = load_doc(&lensmap_path, "group");
    let lensmap_rel = normalize_relative(root, &lensmap_path);
    let lensmap_dirty = git_is_dirty(root, &lensmap_rel);
    let (resolved, inserted, unresolved) = reanchor_doc(root, &mut doc, false, lensmap_dirty);
    let (removed_anchors, removed_entries) = simplify_doc_in_place(&mut doc);
    save_doc(&lensmap_path, doc.clone());

    let out_path = if let Some(out) = args.get("to").or_else(|| args.get("out")) {
        resolve_from_root(root, out)
    } else {
        default_render_output_path(&lensmap_path)
    };
    let (lines, files_rendered, entries_rendered) = build_render_lines(
        root,
        &lensmap_path,
        &doc,
        RenderFilters {
            file: None,
            symbol: None,
            ref_id: None,
            kind: None,
        },
        &tr("LensMap Render", "LensMap 渲染视图"),
    );
    ensure_dir(&out_path);
    let _ = fs::write(&out_path, format!("{}\n", lines.join("\n")));

    let ok = unresolved.is_empty();
    let out = json!({
        "ok": ok,
        "type": "lensmap",
        "action": "sync",
        "lensmap": normalize_relative(root, &lensmap_path),
        "output": normalize_relative(root, &out_path),
        "resolved": resolved,
        "inserted": inserted,
        "removed_anchors": removed_anchors,
        "removed_entries": removed_entries,
        "files_rendered": files_rendered,
        "entries_rendered": entries_rendered,
        "unresolved": unresolved,
        "ts": now_iso(),
    });
    append_history(root, &out);
    emit(out, if ok { 0 } else { 1 });
}

fn cmd_index(root: &Path, args: &ParsedArgs) {
    let lensmaps = resolve_search_lensmap_paths(root, args);
    if lensmaps.is_empty() {
        emit(json!({"ok": false, "error": "no_lensmap_files_found"}), 1);
    }

    let index_path = if let Some(out) = args.get("out").or_else(|| args.get("index")) {
        resolve_from_root(root, out)
    } else {
        default_index_path(root)
    };
    if !is_within_root(root, &index_path) {
        emit(
            json!({"ok": false, "error": "security_output_outside_root"}),
            1,
        );
    }

    let entries = collect_repo_search_entries(root, &lensmaps);
    let doc = make_index_doc(root, lensmaps.clone(), entries.clone());
    save_index_doc(&index_path, &doc);

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "index",
        "index": normalize_relative(root, &index_path),
        "lensmaps": lensmaps,
        "lensmap_count": doc.lensmaps.len(),
        "entry_count": doc.entries.len(),
        "ts": now_iso(),
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_search(root: &Path, args: &ParsedArgs) {
    let query = args.get("query").unwrap_or("").trim().to_string();
    if query.is_empty() {
        emit(json!({"ok": false, "error": "query_required"}), 1);
    }

    let limit = args
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(25)
        .clamp(1, 200);
    let file_filter = args
        .get("file")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let symbol_filter = args
        .get("symbol")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let kind_filter = args
        .get("kind")
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let index_path = args.get("index").map(|path| resolve_from_root(root, path));
    let (entries, source_kind, source_path) =
        if let Some(index_path) = index_path.filter(|path| path.exists()) {
            let index = load_index_doc(&index_path);
            (
                index.entries,
                "index".to_string(),
                Some(normalize_relative(root, &index_path)),
            )
        } else {
            let lensmaps = resolve_search_lensmap_paths(root, args);
            if lensmaps.is_empty() {
                emit(json!({"ok": false, "error": "no_lensmap_files_found"}), 1);
            }
            (
                collect_repo_search_entries(root, &lensmaps),
                "live".to_string(),
                None,
            )
        };

    let mut scored = entries
        .into_iter()
        .filter(|entry| {
            search_entry_matches_filters(entry, file_filter, symbol_filter, kind_filter)
        })
        .filter_map(|entry| {
            let score = search_entry_score(&entry, &query);
            if score > 0 {
                Some((score, entry))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.file.cmp(&right.1.file))
            .then_with(|| {
                left.1
                    .start_line
                    .unwrap_or(0)
                    .cmp(&right.1.start_line.unwrap_or(0))
            })
            .then_with(|| left.1.ref_id.cmp(&right.1.ref_id))
    });
    let total_matches = scored.len();
    scored.truncate(limit);

    let results = scored
        .into_iter()
        .map(|(score, entry)| {
            json!({
                "score": score,
                "lensmap": entry.lensmap,
                "file": entry.file,
                "ref": entry.ref_id,
                "anchor_id": entry.anchor_id,
                "kind": entry.kind,
                "text": entry.text,
                "symbol": entry.symbol,
                "symbol_path": entry.symbol_path,
                "start_line": entry.start_line,
                "end_line": entry.end_line,
                "resolve_strategy": entry.resolve_strategy,
            })
        })
        .collect::<Vec<_>>();

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "search",
        "query": query,
        "file": file_filter,
        "symbol": symbol_filter,
        "kind": kind_filter,
        "source_kind": source_kind,
        "source_path": source_path,
        "total_matches": total_matches,
        "returned": results.len(),
        "results": results,
        "ts": now_iso(),
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_expose(root: &Path, args: &ParsedArgs) {
    let lens_name = args
        .get("name")
        .or_else(|| args.positional.get(1).map(String::as_str))
        .unwrap_or("default")
        .trim()
        .to_string();
    let lens_name = if lens_name.is_empty() {
        "default".to_string()
    } else {
        lens_name
    };

    let private_dir = root.join("local/private-lenses");
    let _ = fs::create_dir_all(&private_dir);
    let private_path = private_dir.join(format!("{}.private.lens.json", lens_name));
    if !private_path.exists() {
        let payload = json!({"lens": lens_name, "entries": []});
        let _ = fs::write(
            &private_path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
            ),
        );
    }

    let public_path = root
        .join("public")
        .join(format!("{}.public.lens.json", lens_name));
    ensure_dir(&public_path);
    let source = fs::read_to_string(&private_path).unwrap_or_else(|_| "{}".to_string());
    let source_json =
        serde_json::from_str::<Value>(&source).unwrap_or_else(|_| json!({"entries": []}));
    let entries = source_json
        .get("entries")
        .cloned()
        .unwrap_or_else(|| Value::Array(vec![]));
    let out_json = json!({
        "lens": lens_name,
        "exposed": true,
        "entries": entries,
    });
    let _ = fs::write(
        &public_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&out_json).unwrap_or_else(|_| "{}".to_string())
        ),
    );

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "expose",
        "lens": lens_name,
        "public_path": normalize_relative(root, &public_path),
        "ts": now_iso(),
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_status(root: &Path, args: &ParsedArgs) {
    let lensmap_path = resolve_lensmap_path(root, args, None);
    let history_path = root.join("local/state/ops/lensmap/history.jsonl");
    let total = if history_path.exists() {
        fs::read_to_string(&history_path)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count()
    } else {
        0usize
    };

    let lensmap_stats = if lensmap_path.exists() {
        let doc = load_doc(&lensmap_path, "group");
        let lensmap_rel = normalize_relative(root, &lensmap_path);
        let mut source_files = BTreeMap::new();
        for anchor in &doc.anchors {
            if !anchor.file.is_empty() {
                source_files.insert(anchor.file.clone(), true);
            }
        }
        for entry in &doc.entries {
            if !entry.file.is_empty() {
                source_files.insert(entry.file.clone(), true);
            }
        }
        Some(json!({
            "lensmap": normalize_relative(root, &lensmap_path),
            "mode": doc.mode,
            "covers": doc.covers.len(),
            "anchors": doc.anchors.len(),
            "entries": doc.entries.len(),
            "git": {
                "lensmap_dirty": git_is_dirty(root, &lensmap_rel),
                "dirty_source_files": source_files.keys().filter(|rel| git_is_dirty(root, rel)).count(),
            }
        }))
    } else {
        None
    };

    emit(
        json!({
            "ok": true,
            "type": "lensmap",
            "action": "status",
            "ts": now_iso(),
            "history_events": total,
            "private_store": "local/private-lenses",
            "lensmap": lensmap_stats,
        }),
        0,
    );
}

fn resolve_search_lensmap_paths(root: &Path, args: &ParsedArgs) -> Vec<String> {
    if let Some(raw) = args.get("lensmaps") {
        let mut paths = BTreeMap::new();
        for lensmap in split_csv(Some(raw)) {
            let abs = resolve_from_root(root, &lensmap);
            if is_within_root(root, &abs) && abs.exists() && is_lensmap_filename(&abs) {
                paths.insert(normalize_relative(root, &abs), true);
            }
        }
        return paths.keys().cloned().collect();
    }
    discover_lensmap_files(root, args.get("bundle-dir").unwrap_or(".lenspack"))
}

fn collect_doc_search_entries(
    root: &Path,
    lensmap_path: &Path,
    doc: &LensMapDoc,
) -> Vec<SearchEntryRecord> {
    let lensmap_rel = normalize_relative(root, lensmap_path);
    let mut files = BTreeMap::new();
    for entry in &doc.entries {
        if !entry.file.trim().is_empty() {
            files.insert(entry.file.clone(), true);
        }
    }
    for anchor in &doc.anchors {
        if !anchor.file.trim().is_empty() {
            files.insert(anchor.file.clone(), true);
        }
    }

    let anchor_map = doc
        .anchors
        .iter()
        .map(|anchor| (anchor.id.to_uppercase(), anchor.clone()))
        .collect::<HashMap<_, _>>();
    let mut out = vec![];

    for (rel, _) in files {
        let abs = resolve_from_root(root, &rel);
        let (file_lines, functions, style, resolutions) =
            if is_within_root(root, &abs) && abs.exists() {
                let file_lines = split_lines(&fs::read_to_string(&abs).unwrap_or_default());
                let functions = detect_functions(&file_lines, &abs);
                let style = comment_style_for(&abs);
                let mut resolutions = HashMap::new();
                for anchor in doc.anchors.iter().filter(|anchor| anchor.file == rel) {
                    resolutions.insert(
                        anchor.id.to_uppercase(),
                        resolve_anchor_in_lines(anchor, &file_lines, &functions, &style),
                    );
                }
                (Some(file_lines), Some(functions), Some(style), resolutions)
            } else {
                (None, None, None, HashMap::new())
            };

        for entry in doc.entries.iter().filter(|entry| entry.file == rel) {
            let parsed = parse_ref(&entry.ref_id);
            let anchor_id = entry
                .anchor_id
                .clone()
                .or_else(|| parsed.as_ref().map(|parts| parts.anchor_id.clone()));
            let anchor = anchor_id
                .as_ref()
                .and_then(|id| anchor_map.get(&id.to_uppercase()).cloned());
            let resolution = anchor.as_ref().and_then(|anchor| {
                if let Some(found) = resolutions.get(&anchor.id.to_uppercase()) {
                    return Some(found.clone());
                }
                if let (Some(lines), Some(functions), Some(style)) =
                    (file_lines.as_ref(), functions.as_ref(), style.as_ref())
                {
                    return Some(resolve_anchor_in_lines(anchor, lines, functions, style));
                }
                None
            });
            let start_line = parsed.as_ref().and_then(|parts| {
                resolution.as_ref().and_then(|found| {
                    anchor
                        .as_ref()
                        .and_then(|anchor| entry_line_indexes(anchor, Some(found), parts))
                        .map(|(start, _end)| start + 1)
                })
            });
            let end_line = parsed.as_ref().and_then(|parts| {
                resolution.as_ref().and_then(|found| {
                    anchor
                        .as_ref()
                        .and_then(|anchor| entry_line_indexes(anchor, Some(found), parts))
                        .map(|(_start, end)| end + 1)
                })
            });

            out.push(SearchEntryRecord {
                lensmap: lensmap_rel.clone(),
                file: rel.clone(),
                ref_id: entry.ref_id.to_uppercase(),
                anchor_id,
                kind: entry.kind.clone(),
                text: entry.text.clone(),
                symbol: anchor.as_ref().and_then(|item| item.symbol.clone()),
                symbol_path: anchor.as_ref().and_then(|item| item.symbol_path.clone()),
                start_line,
                end_line,
                resolve_strategy: resolution.as_ref().map(|item| item.strategy.clone()),
            });
        }
    }

    out
}

fn collect_repo_search_entries(root: &Path, lensmaps: &[String]) -> Vec<SearchEntryRecord> {
    let mut out = vec![];
    for lensmap in lensmaps {
        let abs = resolve_from_root(root, lensmap);
        if !is_within_root(root, &abs) || !abs.exists() {
            continue;
        }
        let doc = load_doc(&abs, "group");
        out.extend(collect_doc_search_entries(root, &abs, &doc));
    }
    out
}

fn search_entry_matches_filters(
    entry: &SearchEntryRecord,
    file_filter: Option<&str>,
    symbol_filter: Option<&str>,
    kind_filter: Option<&str>,
) -> bool {
    if let Some(file) = file_filter {
        if entry.file != file {
            return false;
        }
    }
    if let Some(symbol) = symbol_filter {
        if entry.symbol.as_deref() != Some(symbol) && entry.symbol_path.as_deref() != Some(symbol) {
            return false;
        }
    }
    if let Some(kind) = kind_filter {
        if entry.kind.as_deref() != Some(kind) {
            return false;
        }
    }
    true
}

fn search_entry_score(entry: &SearchEntryRecord, query: &str) -> i32 {
    let normalized_query = query.trim().to_lowercase();
    if normalized_query.is_empty() {
        return 0;
    }

    let mut score = 0i32;
    let text = entry.text.as_deref().unwrap_or("").to_lowercase();
    let symbol = entry.symbol.as_deref().unwrap_or("").to_lowercase();
    let symbol_path = entry.symbol_path.as_deref().unwrap_or("").to_lowercase();
    let file = entry.file.to_lowercase();
    let kind = entry.kind.as_deref().unwrap_or("").to_lowercase();
    let ref_id = entry.ref_id.to_lowercase();

    if ref_id == normalized_query {
        score += 180;
    } else if ref_id.contains(&normalized_query) {
        score += 120;
    }
    if symbol_path == normalized_query {
        score += 170;
    } else if symbol_path.contains(&normalized_query) {
        score += 115;
    }
    if symbol == normalized_query {
        score += 140;
    } else if symbol.contains(&normalized_query) {
        score += 90;
    }
    if text.contains(&normalized_query) {
        score += 100;
    }
    if file.contains(&normalized_query) {
        score += 80;
    }
    if kind == normalized_query {
        score += 70;
    }

    for token in normalized_query.split_whitespace() {
        if token.is_empty() {
            continue;
        }
        if text.contains(token) {
            score += 16;
        }
        if symbol_path.contains(token) {
            score += 14;
        }
        if file.contains(token) {
            score += 8;
        }
        if ref_id.contains(token) {
            score += 8;
        }
    }

    score
}

fn usage() {
    println!("{}", tr("LensMap CLI", "LensMap 命令行"));
    println!(
        "{}",
        tr(
            "Use --lang=en or --lang=zh-CN to force the interface language.",
            "使用 --lang=en 或 --lang=zh-CN 可强制指定界面语言。",
        )
    );
    println!();
    println!(
        "lensmap init <project> [--mode=group|file] [--covers=a,b] [--file=path] [--lensmap=path] [--anchor-placement=inline|standalone] [--lang=en|zh-CN]"
    );
    println!("lensmap annotate --lensmap=path (--ref=<HEX-start[-end]> | --file=path --symbol=name [--symbol-path=Outer.inner] [--offset=N] [--end-offset=M]) --text=<text> [--kind=comment|doc|todo|decision]");
    println!("lensmap template add <type>");
    println!("lensmap scan [--lensmap=path] [--covers=a,b] [--anchor-mode=smart|all] [--anchor-placement=inline|standalone] [--dry-run]");
    println!("lensmap extract-comments [--lensmap=path] [--covers=a,b] [--dry-run]");
    println!(
        "lensmap unmerge [--lensmap=path] [--covers=a,b] [--dry-run]  # alias of extract-comments"
    );
    println!("lensmap merge [--lensmap=path] [--covers=a,b] [--dry-run]");
    println!(
        "lensmap package [--bundle-dir=.lenspack] [--mode=move|copy] [--lensmaps=a,b] [--dry-run]"
    );
    println!("lensmap unpackage [--bundle-dir=.lenspack] [--on-missing=prompt|skip|error] [--map=old_dir=new_dir] [--overwrite] [--dry-run]");
    println!("lensmap validate [--lensmap=path]");
    println!("lensmap reanchor [--lensmap=path] [--dry-run]  # git-aware conflict protection on dirty overlaps");
    println!("lensmap render [--lensmap=path] [--out=path]  # defaults to sibling .md");
    println!("lensmap parse [--lensmap=path] [--out=path]  # alias of render");
    println!("lensmap show [--lensmap=path] [--file=path] [--symbol=name|path] [--ref=HEX-start[-end]] [--kind=comment|doc|todo|decision] [--out=path]");
    println!("lensmap simplify [--lensmap=path]");
    println!("lensmap index [--lensmaps=a,b] [--index=path|--out=path]");
    println!("lensmap search --query=<text> [--lensmaps=a,b] [--index=path] [--file=path] [--symbol=name|path] [--kind=comment|doc|todo|decision] [--limit=N]");
    println!("lensmap polish");
    println!("lensmap import --from=<path>");
    println!("lensmap sync [--lensmap=path] [--to=path]  # reanchor + simplify + render");
    println!("lensmap expose --name=<lens_name>");
    println!("lensmap status [--lensmap=path]");
    println!();
    println!("{}", tr("Quickstart:", "快速开始："));
    println!("  lensmap init demo --mode=group --covers=demo/src");
    println!("  lensmap scan --lensmap=demo/lensmap.json --anchor-mode=smart");
    println!("  lensmap extract-comments --lensmap=demo/lensmap.json");
    println!("  lensmap annotate --lensmap=demo/lensmap.json --file=demo/src/app.ts --symbol=run --symbol-path=App.run --offset=1 --text=\"why this exists\"");
    println!("  lensmap show --lensmap=demo/lensmap.json --file=demo/src/app.ts");
    println!("  lensmap index --index=demo/.lensmap-index.json");
    println!("  lensmap search --index=demo/.lensmap-index.json --query=why");
    println!("  lensmap merge --lensmap=demo/lensmap.json");
    println!("  lensmap unmerge --lensmap=demo/lensmap.json");
    println!("  lensmap package --bundle-dir=.lenspack");
    println!("  lensmap unpackage --bundle-dir=.lenspack --on-missing=prompt");
    println!("  lensmap sync --lensmap=demo/lensmap.json");
    println!("  lensmap validate --lensmap=demo/lensmap.json");
    println!();
    println!(
        "{}",
        tr(
            "Supported AST languages: JavaScript, TypeScript, Python, Rust, Go, Java, C, C++, C#, Kotlin.",
            "当前支持的 AST 语言：JavaScript、TypeScript、Python、Rust、Go、Java、C、C++、C#、Kotlin。",
        )
    );
}

fn main() {
    let root = detect_root();
    let argv = env::args().skip(1).collect::<Vec<_>>();
    let args = ParsedArgs::parse(&argv);

    let cmd = args
        .positional
        .first()
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_else(|| "status".to_string());

    if args.has("help") || cmd.is_empty() || cmd == "help" || cmd == "--help" || cmd == "-h" {
        usage();
        std::process::exit(0);
    }

    match cmd.as_str() {
        "init" => cmd_init(&root, &args),
        "annotate" => cmd_annotate(&root, &args),
        "template" if args.positional.get(1).map(String::as_str) == Some("add") => {
            cmd_template_add(&root, &args)
        }
        "scan" => cmd_scan(&root, &args),
        "extract-comments" => cmd_extract_comments(&root, &args),
        "unmerge" => cmd_extract_comments(&root, &args),
        "merge" => cmd_merge(&root, &args),
        "package" => cmd_package(&root, &args),
        "unpackage" => cmd_unpackage(&root, &args),
        "validate" => cmd_validate(&root, &args),
        "reanchor" => cmd_reanchor(&root, &args),
        "render" => cmd_render(&root, &args),
        "parse" => cmd_render(&root, &args),
        "show" => cmd_show(&root, &args),
        "simplify" => cmd_simplify(&root, &args),
        "index" => cmd_index(&root, &args),
        "search" => cmd_search(&root, &args),
        "polish" => cmd_polish(&root),
        "import" => cmd_import(&root, &args),
        "sync" => cmd_sync(&root, &args),
        "expose" => cmd_expose(&root, &args),
        "status" => cmd_status(&root, &args),
        _ => {
            usage();
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn write_temp_source(ext: &str, stem: &str, body: &str) -> (PathBuf, Vec<String>) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = env::temp_dir().join(format!("lensmap_tests_{}_{}", stem, nonce));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}{}", stem, ext));
        fs::write(&path, body).unwrap();
        let lines = split_lines(body);
        (path, lines)
    }

    #[test]
    fn parse_ui_lang_accepts_english_and_chinese() {
        assert_eq!(parse_ui_lang("en_US.UTF-8"), UiLang::En);
        assert_eq!(parse_ui_lang("zh-CN"), UiLang::ZhCn);
        assert_eq!(parse_ui_lang("zh_TW"), UiLang::ZhCn);
    }

    #[test]
    fn detect_functions_ast_supports_go_receivers() {
        let source = r#"package demo

type Runner struct{}

func (r *Runner) Run() {
}

func Top() {
}
"#;
        let (path, lines) = write_temp_source(".go", "sample", source);
        let hits = detect_functions_ast(&lines, &path);
        let paths = hits
            .iter()
            .map(|hit| hit.symbol_path.clone())
            .collect::<Vec<_>>();
        assert!(paths.contains(&"Runner.Run".to_string()));
        assert!(paths.contains(&"Top".to_string()));
        let _ = fs::remove_dir_all(path.parent().unwrap_or_else(|| Path::new(".")));
    }

    #[test]
    fn detect_functions_ast_supports_java_class_paths() {
        let source = r#"class Worker {
    Worker() {
    }

    void run() {
    }

    class Inner {
        void nested() {
        }
    }
}
"#;
        let (path, lines) = write_temp_source(".java", "Worker", source);
        let hits = detect_functions_ast(&lines, &path);
        let paths = hits
            .iter()
            .map(|hit| hit.symbol_path.clone())
            .collect::<Vec<_>>();
        assert!(paths.contains(&"Worker.Worker".to_string()));
        assert!(paths.contains(&"Worker.run".to_string()));
        assert!(paths.contains(&"Worker.Inner.nested".to_string()));
        let _ = fs::remove_dir_all(path.parent().unwrap_or_else(|| Path::new(".")));
    }

    #[test]
    fn detect_functions_ast_supports_c_functions() {
        let source = r#"static int add(int left, int right) {
    return left + right;
}
"#;
        let (path, lines) = write_temp_source(".c", "math", source);
        let hits = detect_functions_ast(&lines, &path);
        let paths = hits
            .iter()
            .map(|hit| hit.symbol_path.clone())
            .collect::<Vec<_>>();
        assert!(paths.contains(&"add".to_string()));
        let _ = fs::remove_dir_all(path.parent().unwrap_or_else(|| Path::new(".")));
    }

    #[test]
    fn detect_functions_ast_supports_cpp_scoped_methods() {
        let source = r#"namespace api {
class Worker {
public:
    void run() {
    }
};
}
"#;
        let (path, lines) = write_temp_source(".cpp", "worker", source);
        let hits = detect_functions_ast(&lines, &path);
        let paths = hits
            .iter()
            .map(|hit| hit.symbol_path.clone())
            .collect::<Vec<_>>();
        assert!(paths.contains(&"api.Worker.run".to_string()));
        let _ = fs::remove_dir_all(path.parent().unwrap_or_else(|| Path::new(".")));
    }

    #[test]
    fn detect_functions_ast_supports_csharp_namespaces() {
        let source = r#"namespace Demo.Tools;

class Worker {
    Worker() {
    }

    public void Run() {
    }
}
"#;
        let (path, lines) = write_temp_source(".cs", "Worker", source);
        let hits = detect_functions_ast(&lines, &path);
        let paths = hits
            .iter()
            .map(|hit| hit.symbol_path.clone())
            .collect::<Vec<_>>();
        assert!(paths.contains(&"Demo.Tools.Worker.Worker".to_string()));
        assert!(paths.contains(&"Demo.Tools.Worker.Run".to_string()));
        let _ = fs::remove_dir_all(path.parent().unwrap_or_else(|| Path::new(".")));
    }

    #[test]
    fn detect_functions_ast_supports_kotlin_members() {
        let source = r#"class Worker {
    fun run() {
    }

    object Inner {
        fun nested() {
        }
    }
}
"#;
        let (path, lines) = write_temp_source(".kt", "Worker", source);
        let hits = detect_functions_ast(&lines, &path);
        let paths = hits
            .iter()
            .map(|hit| hit.symbol_path.clone())
            .collect::<Vec<_>>();
        assert!(paths.contains(&"Worker.run".to_string()));
        assert!(paths.contains(&"Worker.Inner.nested".to_string()));
        let _ = fs::remove_dir_all(path.parent().unwrap_or_else(|| Path::new(".")));
    }

    #[test]
    fn resolve_anchor_in_lines_repairs_refactor_by_signature_and_position() {
        let source = r#"struct Task;
struct Worker;

impl Worker {
    fn execute(&self, task: &Task) -> Result<(), String> {
        let _ = task;
        Ok(())
    }

    fn noop(&self) {}
}
"#;
        let (path, lines) = write_temp_source(".rs", "worker_refactor", source);
        let hits = detect_functions(&lines, &path);
        let style = comment_style_for(&path);
        let anchor = AnchorRecord {
            id: "ABCDEF".to_string(),
            file: "worker_refactor.rs".to_string(),
            symbol: Some("run".to_string()),
            symbol_path: Some("Worker.run".to_string()),
            line_anchor: Some(2),
            line_symbol: Some(3),
            span_start: Some(3),
            span_end: Some(5),
            fingerprint: None,
            signature_text: Some("fn run(&self, task: &Task) -> Result<(), String> {".to_string()),
            placement: Some("standalone".to_string()),
            updated_at: None,
        };

        let resolution = resolve_anchor_in_lines(&anchor, &lines, &hits, &style);
        assert_eq!(resolution.strategy, "fuzzy_refactor");
        let resolved = resolution.function_hit.expect("expected fuzzy match");
        assert_eq!(resolved.symbol, "execute");
        assert_eq!(resolved.symbol_path, "Worker.execute");
        let _ = fs::remove_dir_all(path.parent().unwrap_or_else(|| Path::new(".")));
    }

    #[test]
    fn materialize_anchors_carries_signature_text() {
        let source = r#"// @lensmap-anchor ABCDEF
fn run(task: &str) -> bool {
    !task.is_empty()
}
"#;
        let (path, lines) = write_temp_source(".rs", "worker_anchor", source);
        let root = path.parent().unwrap_or_else(|| Path::new("."));
        let anchors = materialize_anchors_for_file(root, &path, &lines);
        assert_eq!(anchors.len(), 1);
        assert_eq!(
            anchors[0].signature_text.as_deref(),
            Some("fn run(task: &str) -> bool {")
        );
        let _ = fs::remove_dir_all(path.parent().unwrap_or_else(|| Path::new(".")));
    }

    #[test]
    fn materialize_anchors_detects_inline_anchor_placement() {
        let source = r#"fn run(task: &str) -> bool { // @lensmap-anchor ABCDEF
    !task.is_empty()
}
"#;
        let (path, lines) = write_temp_source(".rs", "worker_inline_anchor", source);
        let root = path.parent().unwrap_or_else(|| Path::new("."));
        let anchors = materialize_anchors_for_file(root, &path, &lines);
        assert_eq!(anchors.len(), 1);
        assert_eq!(anchors[0].placement.as_deref(), Some("inline"));
        assert_eq!(anchors[0].line_anchor, Some(1));
        assert_eq!(anchors[0].line_symbol, Some(1));
        let _ = fs::remove_dir_all(path.parent().unwrap_or_else(|| Path::new(".")));
    }

    #[test]
    fn entry_line_indexes_use_symbol_relative_offsets() {
        let anchor = AnchorRecord {
            id: "ABCDEF".to_string(),
            file: "demo.rs".to_string(),
            symbol: Some("run".to_string()),
            symbol_path: Some("Worker.run".to_string()),
            line_anchor: Some(10),
            line_symbol: Some(10),
            span_start: Some(10),
            span_end: Some(12),
            fingerprint: None,
            signature_text: Some("fn run() {".to_string()),
            placement: Some("inline".to_string()),
            updated_at: None,
        };
        let parts = RefParts {
            anchor_id: "ABCDEF".to_string(),
            start: 1,
            end: 3,
        };
        assert_eq!(entry_line_indexes(&anchor, None, &parts), Some((9, 11)));
    }

    #[test]
    fn project_line_from_hunks_tracks_insertions_before_anchor() {
        let hunks = vec![GitHunk {
            old_start: 3,
            old_count: 0,
            new_start: 3,
            new_count: 2,
        }];
        assert_eq!(project_line_from_hunks(&hunks, 8), Some(10));
    }
}
