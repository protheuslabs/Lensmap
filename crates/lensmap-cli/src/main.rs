use chrono::{DateTime, Duration, Utc};
use flate2::write::GzEncoder;
use flate2::Compression;
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
use tar::Builder;
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
const COMMAND_VERSION: &str = env!("CARGO_PKG_VERSION");

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
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    review_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    review_due_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    tags: Vec<String>,
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
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    review_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    review_due_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    tags: Vec<String>,
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
    source_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    packaged_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    bundle_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    envelope_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    redaction_profile: Option<String>,
    #[serde(default)]
    retention_days: i64,
    created_at: String,
    updated_at: String,
    items: Vec<PackageItem>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
struct EvidenceArtifact {
    path: String,
    kind: String,
    hash: String,
    bytes: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
struct EvidenceEnvelope {
    #[serde(rename = "type")]
    doc_type: String,
    version: String,
    command_version: String,
    command: String,
    command_profile: String,
    command_identity: String,
    policy_hash: String,
    execution_fingerprint: String,
    redaction_profile: String,
    retention_window_days: i64,
    started_at: String,
    finished_at: String,
    status: String,
    status_code: i32,
    input_artifacts: Vec<EvidenceArtifact>,
    output_artifacts: Vec<EvidenceArtifact>,
    errors: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct EvidenceCheckpoint {
    #[serde(default)]
    command: String,
    #[serde(default)]
    bundle_dir: String,
    #[serde(default)]
    command_version: String,
    #[serde(default)]
    run_id: String,
    #[serde(default)]
    compression_mode: String,
    #[serde(default)]
    redaction_profile: String,
    #[serde(default)]
    retention_days: i64,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    completed: Vec<String>,
    #[serde(default)]
    skipped: Vec<String>,
    #[serde(default)]
    errors: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EvidenceCompressionMode {
    None,
    Copy,
}

impl EvidenceCompressionMode {
    fn parse(raw: &str) -> Self {
        match raw.trim().to_lowercase().as_str() {
            "copy" => Self::Copy,
            _ => Self::None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Copy => "copy",
        }
    }
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
    owner: Option<&'a str>,
    template: Option<&'a str>,
    review_status: Option<&'a str>,
    scope: Option<&'a str>,
    tag: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Default)]
struct SearchFilters<'a> {
    file: Option<&'a str>,
    symbol: Option<&'a str>,
    kind: Option<&'a str>,
    owner: Option<&'a str>,
    template: Option<&'a str>,
    review_status: Option<&'a str>,
    scope: Option<&'a str>,
    tag: Option<&'a str>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
struct TemplateDefinition {
    #[serde(rename = "type")]
    doc_type: String,
    name: String,
    description: String,
    template: bool,
    kind: String,
    title_prefix: String,
    body: String,
    review_days: i64,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    required_fields: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
struct PolicySettings {
    require_owner: bool,
    require_author: bool,
    require_template: bool,
    require_review_status: bool,
    stale_after_days: i64,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    required_patterns: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, rename_all = "camelCase")]
struct ProductionSettings {
    strip_anchors: bool,
    strip_refs: bool,
    strip_on_package: bool,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    exclude_patterns: Vec<String>,
}

struct PrReportRender<'a> {
    base: Option<&'a str>,
    head: Option<&'a str>,
    source_kind: &'a str,
    changed_files: &'a [String],
    grouped: &'a BTreeMap<String, Vec<SearchEntryRecord>>,
    stale_refs: &'a [String],
    unreviewed_refs: &'a [String],
    uncovered_files: &'a [String],
}

#[derive(Clone, Debug)]
struct LoadedLensMapDoc {
    lensmap: String,
    path: PathBuf,
    doc: LensMapDoc,
}

#[derive(Clone, Debug, Default)]
struct ValidationFindings {
    errors: Vec<String>,
    warnings: Vec<String>,
    lensmap_dirty: bool,
}

#[derive(Clone, Debug, Default)]
struct StripSummary {
    files_scanned: usize,
    files_changed: usize,
    anchors_removed: usize,
    refs_removed: usize,
    marker_hits: usize,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
struct PolicyFinding {
    level: String,
    #[serde(rename = "ref")]
    ref_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    lensmap: Option<String>,
    field: String,
    code: String,
    message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SummaryStats {
    entry_count: usize,
    stale_entries: usize,
    files_with_notes: usize,
    by_file: BTreeMap<String, usize>,
    by_directory: BTreeMap<String, usize>,
    by_owner: BTreeMap<String, usize>,
    by_kind: BTreeMap<String, usize>,
    by_template: BTreeMap<String, usize>,
    by_review_status: BTreeMap<String, usize>,
    by_scope: BTreeMap<String, usize>,
}

#[derive(Serialize, Clone, Debug)]
struct ImportProposal {
    entry: EntryRecord,
    confidence: f64,
    reason: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
struct MetricHistoryPoint {
    period: String,
    generated_at: String,
    metrics: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
struct AutobotReceipt {
    run_id: String,
    action: String,
    profile: String,
    created_at: String,
    updated_at: String,
    source_root: String,
    lensmap: String,
    from: Vec<String>,
    files_scanned: usize,
    proposals_created: usize,
    accepted: usize,
    pending_review: usize,
    conflicts: usize,
    policy_mode: String,
    policy_failures: Vec<String>,
    applied: bool,
    dry_run: bool,
    checkpoint: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum AutobotProfile {
    Conservative,
    Standard,
    Aggressive,
}

#[derive(Clone, Copy, Debug)]
enum EvidenceRedactionProfile {
    Debug,
    Audit,
    Operational,
    Clinical,
    Emergency,
}

impl EvidenceRedactionProfile {
    fn parse(raw: &str) -> Self {
        match raw.trim().to_lowercase().as_str() {
            "audit" => Self::Audit,
            "operational" => Self::Operational,
            "clinical" => Self::Clinical,
            "emergency" => Self::Emergency,
            _ => Self::Debug,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Audit => "audit",
            Self::Operational => "operational",
            Self::Clinical => "clinical",
            Self::Emergency => "emergency",
        }
    }

    fn default_retention_days(self) -> i64 {
        match self {
            Self::Debug => 3650,
            Self::Audit => 1095,
            Self::Operational => 365,
            Self::Clinical => 180,
            Self::Emergency => 90,
        }
    }
}

impl AutobotProfile {
    fn parse(raw: Option<&str>) -> Self {
        match raw.unwrap_or("standard").trim().to_lowercase().as_str() {
            "conservative" => Self::Conservative,
            "aggressive" => Self::Aggressive,
            _ => Self::Standard,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Conservative => "conservative",
            Self::Standard => "standard",
            Self::Aggressive => "aggressive",
        }
    }

    fn acceptance_threshold(self) -> f64 {
        match self {
            Self::Conservative => 0.96,
            Self::Standard => 0.86,
            Self::Aggressive => 0.7,
        }
    }

    fn conflict_tolerance(self) -> usize {
        match self {
            Self::Conservative => 0,
            Self::Standard => 2,
            Self::Aggressive => 4,
        }
    }

    fn policy_mode(self) -> &'static str {
        match self {
            Self::Conservative => "strict",
            Self::Standard => "standard",
            Self::Aggressive => "permissive",
        }
    }
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
        "strip_in_place_requires_force" => tr(
            "In-place stripping requires --force.",
            "原地清理需要同时传入 --force。",
        ),
        "no_source_files_found" => tr(
            "No source files were found for stripping.",
            "未找到可清理的源码文件。",
        ),
        "strip_sources_no_files" => tr(
            "No source files were resolved for strip-sources packaging.",
            "strip-sources 打包未解析到源码文件。",
        ),
        "invalid_out_format" => tr(
            "Invalid strip archive format. Use tar.gz.",
            "无效的清理归档格式。请使用 tar.gz。",
        ),
        "template_missing" => tr(
            "The requested template could not be found.",
            "未找到请求的模板。",
        ),
        "git_range_unavailable" => tr(
            "The requested git base/head range could not be resolved.",
            "无法解析请求的 Git base/head 范围。",
        ),
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
        "template_list" => tr("Templates listed.", "模板列表已生成。"),
        "scan" => tr("Anchor scan completed.", "锚点扫描已完成。"),
        "extract_comments" => tr("Comments extracted into LensMap.", "注释已提取到 LensMap。"),
        "unmerge" => tr("Comments extracted into LensMap.", "注释已提取到 LensMap。"),
        "merge" => tr(
            "LensMap entries merged back into source files.",
            "LensMap 条目已合并回源码文件。",
        ),
        "annotate" => tr("Annotation saved.", "注释已保存。"),
        "package" => tr("LensMap files packaged.", "LensMap 文件已打包。"),
        "strip" => tr(
            "LensMap source stripping completed.",
            "LensMap 源码清理已完成。",
        ),
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
        "policy_init" => tr("LensMap policy updated.", "LensMap 策略已更新。"),
        "policy_check" => tr(
            "LensMap policy check completed.",
            "LensMap 策略检查已完成。",
        ),
        "summary" => tr("LensMap summary generated.", "LensMap 汇总已生成。"),
        "pr_report" => tr("LensMap PR report generated.", "LensMap PR 报告已生成。"),
        "expose" => tr(
            "Lens exposed to the private store.",
            "镜头已暴露到私有存储。",
        ),
        "status" => tr("Status collected.", "状态已收集。"),
        "metrics" => tr("LensMap metrics generated.", "LensMap 指标已生成。"),
        "scorecard" => tr("LensMap scorecard generated.", "LensMap 评分卡已生成。"),
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

fn parse_redaction_profile(args: &ParsedArgs, key: &str) -> EvidenceRedactionProfile {
    args.get(key).map_or(
        EvidenceRedactionProfile::Debug,
        EvidenceRedactionProfile::parse,
    )
}

fn file_fingerprint(path: &Path) -> Option<String> {
    let mut hasher = Sha256::new();
    let content = fs::read(path).ok()?;
    hasher.update(&content);
    Some(hex::encode(hasher.finalize())[..12].to_string())
}

fn file_size(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|meta| meta.len())
}

fn make_evidence_artifact(root: &Path, path: &Path, kind: &str) -> Option<EvidenceArtifact> {
    let rel = normalize_relative(root, path);
    let bytes = file_size(path)?;
    let hash = file_fingerprint(path)?;
    Some(EvidenceArtifact {
        path: rel,
        kind: kind.to_string(),
        hash,
        bytes,
    })
}

fn command_policy_hash(policy: &PolicySettings) -> String {
    serde_json::to_string(policy)
        .ok()
        .map(|raw| hash_text(&raw))
        .unwrap_or_else(|| "unavailable".to_string())
}

fn command_fingerprint(
    command: &str,
    args: &ParsedArgs,
    inputs: &[String],
    outputs: &[String],
) -> String {
    let mut raw = String::new();
    raw.push_str(command);
    for input in inputs {
        raw.push_str(input);
    }
    for output in outputs {
        raw.push_str(output);
    }
    let mut flag_map: BTreeMap<String, String> = BTreeMap::new();
    for (key, value) in &args.flags {
        flag_map.insert(key.clone(), value.clone());
    }
    for (key, value) in flag_map {
        raw.push_str(&key);
        raw.push('=');
        raw.push_str(&value);
        raw.push('|');
    }
    hash_text(&raw)
}

fn apply_redaction(value: &mut Value, profile: EvidenceRedactionProfile) {
    match profile {
        EvidenceRedactionProfile::Debug => {}
        EvidenceRedactionProfile::Audit => redact_json_fields(
            value,
            &["author", "review_due_at", "source", "signature", "metadata"],
        ),
        EvidenceRedactionProfile::Operational => redact_json_fields(
            value,
            &[
                "author",
                "review_due_at",
                "source",
                "signature",
                "metadata",
                "text",
                "owner",
            ],
        ),
        EvidenceRedactionProfile::Clinical => redact_json_fields(
            value,
            &[
                "author",
                "review_due_at",
                "source",
                "signature",
                "metadata",
                "owner",
                "text",
                "scope",
            ],
        ),
        EvidenceRedactionProfile::Emergency => {
            *value = json!({"redacted": "policy_profile_emergency"});
        }
    }
}

fn redact_json_fields(value: &mut Value, keys: &[&str]) {
    match value {
        Value::Object(map) => {
            for key in keys {
                map.remove(*key);
            }
            for item in map.values_mut() {
                redact_json_fields(item, keys);
            }
        }
        Value::Array(values) => {
            for item in values {
                redact_json_fields(item, keys);
            }
        }
        _ => {}
    }
}

fn to_evidence_artifacts(root: &Path, outputs: &[&Path], kind: &str) -> Vec<EvidenceArtifact> {
    outputs
        .iter()
        .filter_map(|path| make_evidence_artifact(root, path, kind))
        .collect()
}

fn emit_with_envelope(
    root: &Path,
    command: &str,
    action: &str,
    args: &ParsedArgs,
    payload: Value,
    profile: EvidenceRedactionProfile,
    policy: &PolicySettings,
    input_refs: &[String],
    output_paths: &[PathBuf],
    status_code: i32,
    errors: Vec<String>,
) -> ! {
    let started_at = payload
        .get("ts")
        .and_then(Value::as_str)
        .map(|value| value.to_string())
        .unwrap_or_else(now_iso);
    let ts = now_iso();
    let output_refs = output_paths
        .iter()
        .map(|path| normalize_relative(root, path))
        .collect::<Vec<_>>();
    let output_artifacts = output_paths
        .iter()
        .filter_map(|path| make_evidence_artifact(root, path, action))
        .collect::<Vec<_>>();
    let mut out_value = payload;
    apply_redaction(&mut out_value, profile);
    let command_profile = if action.is_empty() { "default" } else { action };
    let receipt = EvidenceEnvelope {
        doc_type: "lensmap_evidence_envelope".to_string(),
        version: "1.0.0".to_string(),
        command_version: COMMAND_VERSION.to_string(),
        command: command.to_string(),
        command_profile: command_profile.to_string(),
        command_identity: action.to_string(),
        policy_hash: command_policy_hash(policy),
        execution_fingerprint: command_fingerprint(command, args, input_refs, &output_refs),
        redaction_profile: profile.label().to_string(),
        retention_window_days: out_value
            .get("retention_days")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| profile.default_retention_days()),
        started_at,
        finished_at: ts.clone(),
        status: if status_code == 0 { "ok" } else { "failed" }.to_string(),
        status_code,
        input_artifacts: input_refs
            .iter()
            .map(|path| EvidenceArtifact {
                path: path.clone(),
                kind: "input".to_string(),
                hash: "".to_string(),
                bytes: 0,
            })
            .collect(),
        output_artifacts,
        errors,
    };
    if let Some(obj) = out_value.as_object_mut() {
        obj.insert(
            "receipt".to_string(),
            serde_json::to_value(receipt).unwrap_or_else(|_| json!({})),
        );
    }
    if let Some(bundle_dir) = out_value
        .get("bundle_dir")
        .and_then(Value::as_str)
        .map(|dir| resolve_from_root(root, dir))
    {
        let envelope_path = bundle_dir.join("envelope.json");
        ensure_dir(&envelope_path);
        let _ = fs::write(
            &envelope_path,
            serde_json::to_string_pretty(&out_value).unwrap_or_else(|_| "{}".to_string()),
        );
    }
    append_history(root, &out_value);
    emit(out_value, status_code);
}

fn metadata_string_array(values: &[&str]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|v| Value::String((*v).to_string()))
            .collect(),
    )
}

fn default_policy_settings() -> PolicySettings {
    PolicySettings {
        require_owner: false,
        require_author: false,
        require_template: false,
        require_review_status: false,
        stale_after_days: 120,
        required_patterns: vec![],
    }
}

fn default_policy_value() -> Value {
    serde_json::to_value(default_policy_settings()).unwrap_or_else(|_| json!({}))
}

fn default_production_settings() -> ProductionSettings {
    ProductionSettings {
        strip_anchors: false,
        strip_refs: false,
        strip_on_package: false,
        exclude_patterns: vec![],
    }
}

fn default_production_value() -> Value {
    serde_json::to_value(default_production_settings()).unwrap_or_else(|_| json!({}))
}

fn metadata_string(metadata: &Map<String, Value>, key: &str, fallback: &str) -> String {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

fn artifact_layers_for_doc(doc: &LensMapDoc) -> Vec<String> {
    doc.metadata
        .get("artifact_layers")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|item| item.to_string())
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| {
            vec![
                "canonical-json".to_string(),
                "readable-markdown".to_string(),
                "search-index".to_string(),
            ]
        })
}

fn policy_for_doc(doc: &LensMapDoc) -> PolicySettings {
    doc.metadata
        .get("policy")
        .cloned()
        .and_then(|value| serde_json::from_value::<PolicySettings>(value).ok())
        .unwrap_or_else(default_policy_settings)
}

fn production_for_doc(doc: &LensMapDoc) -> ProductionSettings {
    doc.metadata
        .get("production")
        .cloned()
        .and_then(|value| serde_json::from_value::<ProductionSettings>(value).ok())
        .unwrap_or_else(default_production_settings)
}

fn aggregate_policy_settings<'a, I>(policies: I) -> PolicySettings
where
    I: IntoIterator<Item = &'a PolicySettings>,
{
    let mut aggregated = PolicySettings::default();
    let mut stale_after_days = None::<i64>;
    let mut required_patterns = BTreeMap::new();

    for policy in policies {
        aggregated.require_owner |= policy.require_owner;
        aggregated.require_author |= policy.require_author;
        aggregated.require_template |= policy.require_template;
        aggregated.require_review_status |= policy.require_review_status;
        if policy.stale_after_days > 0 {
            stale_after_days = Some(match stale_after_days {
                Some(current) => current.min(policy.stale_after_days),
                None => policy.stale_after_days,
            });
        }
        for pattern in &policy.required_patterns {
            let normalized = pattern.trim();
            if !normalized.is_empty() {
                required_patterns.insert(normalized.to_string(), true);
            }
        }
    }

    aggregated.stale_after_days = stale_after_days.unwrap_or(0);
    aggregated.required_patterns = required_patterns.keys().cloned().collect();
    aggregated
}

fn aggregate_production_settings<'a, I>(policies: I) -> ProductionSettings
where
    I: IntoIterator<Item = &'a ProductionSettings>,
{
    let mut aggregated = ProductionSettings::default();
    let mut exclude_patterns = BTreeMap::new();
    for policy in policies {
        aggregated.strip_anchors |= policy.strip_anchors;
        aggregated.strip_refs |= policy.strip_refs;
        aggregated.strip_on_package |= policy.strip_on_package;
        for pattern in &policy.exclude_patterns {
            let normalized = pattern.trim();
            if !normalized.is_empty() {
                exclude_patterns.insert(normalized.to_string(), true);
            }
        }
    }
    aggregated.exclude_patterns = exclude_patterns.keys().cloned().collect();
    aggregated
}

fn store_policy(metadata: &mut Map<String, Value>, policy: &PolicySettings) {
    metadata.insert(
        "policy".to_string(),
        serde_json::to_value(policy).unwrap_or_else(|_| json!({})),
    );
}

fn store_production_policy(metadata: &mut Map<String, Value>, policy: &ProductionSettings) {
    metadata.insert(
        "production".to_string(),
        serde_json::to_value(policy).unwrap_or_else(|_| json!({})),
    );
}

fn bool_from_flag(args: &ParsedArgs, key: &str, fallback: bool) -> bool {
    match args.get(key).map(|raw| raw.trim().to_lowercase()) {
        Some(raw) if ["1", "true", "yes", "on"].contains(&raw.as_str()) => true,
        Some(raw) if ["0", "false", "no", "off"].contains(&raw.as_str()) => false,
        Some(_) => fallback,
        None => fallback,
    }
}

fn parse_i64_flag(args: &ParsedArgs, key: &str) -> Option<i64> {
    args.get(key).and_then(|raw| raw.trim().parse::<i64>().ok())
}

fn parse_f64_flag(args: &ParsedArgs, key: &str) -> Option<f64> {
    args.get(key)
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut uniq = BTreeMap::new();
    for raw in tags {
        let tag = raw.trim().to_lowercase();
        if !tag.is_empty() {
            uniq.insert(tag, true);
        }
    }
    uniq.keys().cloned().collect()
}

fn parse_tags(raw: Option<&str>) -> Vec<String> {
    normalize_tags(&split_csv(raw))
}

fn default_author() -> Option<String> {
    for key in ["LENSMAP_AUTHOR", "GIT_AUTHOR_NAME", "USER", "USERNAME"] {
        if let Ok(value) = env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn default_scope_for_file(file: &str) -> String {
    Path::new(file)
        .parent()
        .map(to_posix_str)
        .filter(|value| !value.is_empty() && value != ".")
        .unwrap_or_else(|| "repo".to_string())
}

fn entry_timestamp(entry: &EntryRecord) -> Option<DateTime<Utc>> {
    for raw in [
        entry.updated_at.as_deref(),
        entry.review_due_at.as_deref(),
        entry.created_at.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if let Ok(parsed) = DateTime::parse_from_rfc3339(raw) {
            return Some(parsed.with_timezone(&Utc));
        }
    }
    None
}

fn entry_review_due(entry: &EntryRecord) -> Option<DateTime<Utc>> {
    entry.review_due_at.as_deref().and_then(|raw| {
        DateTime::parse_from_rfc3339(raw)
            .ok()
            .map(|parsed| parsed.with_timezone(&Utc))
    })
}

fn entry_is_stale(entry: &EntryRecord, stale_after_days: i64) -> bool {
    if let Some(due) = entry_review_due(entry) {
        return due < Utc::now();
    }
    let Some(updated) = entry_timestamp(entry) else {
        return false;
    };
    updated < Utc::now() - Duration::days(stale_after_days.max(1))
}

fn template_library() -> Vec<TemplateDefinition> {
    vec![
        TemplateDefinition {
            doc_type: "lensmap-template".to_string(),
            name: "architecture".to_string(),
            description: "Architecture rationale and consequences.".to_string(),
            template: true,
            kind: "doc".to_string(),
            title_prefix: "Architecture".to_string(),
            body: "Context:\nDecision:\nConsequences:".to_string(),
            review_days: 90,
            tags: vec![
                "architecture".to_string(),
                "knowledge-boilerplate".to_string(),
            ],
            required_fields: vec!["title".to_string(), "owner".to_string(), "text".to_string()],
        },
        TemplateDefinition {
            doc_type: "lensmap-template".to_string(),
            name: "migration".to_string(),
            description: "Migration plan, risk, and rollback notes.".to_string(),
            template: true,
            kind: "decision".to_string(),
            title_prefix: "Migration".to_string(),
            body: "Change:\nRisk:\nRollback:\nVerification:".to_string(),
            review_days: 30,
            tags: vec!["migration".to_string(), "knowledge-boilerplate".to_string()],
            required_fields: vec!["owner".to_string(), "text".to_string()],
        },
        TemplateDefinition {
            doc_type: "lensmap-template".to_string(),
            name: "audit".to_string(),
            description: "Audit and operational compliance note.".to_string(),
            template: true,
            kind: "doc".to_string(),
            title_prefix: "Audit".to_string(),
            body: "Finding:\nImpact:\nControl:\nFollow-up:".to_string(),
            review_days: 30,
            tags: vec!["audit".to_string(), "ops".to_string()],
            required_fields: vec!["owner".to_string(), "text".to_string()],
        },
        TemplateDefinition {
            doc_type: "lensmap-template".to_string(),
            name: "review".to_string(),
            description: "Code review rationale or follow-up note.".to_string(),
            template: true,
            kind: "comment".to_string(),
            title_prefix: "Review".to_string(),
            body: "Observation:\nRisk:\nSuggested follow-up:".to_string(),
            review_days: 14,
            tags: vec!["review".to_string()],
            required_fields: vec!["author".to_string(), "text".to_string()],
        },
        TemplateDefinition {
            doc_type: "lensmap-template".to_string(),
            name: "decision".to_string(),
            description: "Decision record with tradeoffs.".to_string(),
            template: true,
            kind: "decision".to_string(),
            title_prefix: "Decision".to_string(),
            body: "Decision:\nTradeoffs:\nOwner:".to_string(),
            review_days: 60,
            tags: vec!["decision".to_string()],
            required_fields: vec!["title".to_string(), "owner".to_string(), "text".to_string()],
        },
        TemplateDefinition {
            doc_type: "lensmap-template".to_string(),
            name: "todo".to_string(),
            description: "Managed TODO note with owner and due cadence.".to_string(),
            template: true,
            kind: "todo".to_string(),
            title_prefix: "TODO".to_string(),
            body: "Task:\nOwner:\nExit criteria:".to_string(),
            review_days: 14,
            tags: vec!["todo".to_string()],
            required_fields: vec!["owner".to_string(), "text".to_string()],
        },
    ]
}

fn builtin_template(name: &str) -> Option<TemplateDefinition> {
    let normalized = name.trim().to_lowercase();
    template_library()
        .into_iter()
        .find(|template| template.name == normalized)
}

fn template_file_path(root: &Path, name: &str) -> PathBuf {
    root.join("templates")
        .join(format!("{}.lens.template.json", name.trim().to_lowercase()))
}

fn load_template(root: &Path, name: &str) -> Option<TemplateDefinition> {
    if let Some(template) = builtin_template(name) {
        return Some(template);
    }
    let path = template_file_path(root, name);
    if !path.exists() {
        return None;
    }
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str::<TemplateDefinition>(&raw).ok()
}

fn apply_default_metadata(metadata: &mut Map<String, Value>) {
    metadata
        .entry("positioning".to_string())
        .or_insert_with(|| Value::String("external-doc-layer".to_string()));
    metadata
        .entry("boilerplate_scope".to_string())
        .or_insert_with(|| Value::String("knowledge".to_string()));
    metadata
        .entry("default_anchor_mode".to_string())
        .or_insert_with(|| Value::String("smart".to_string()));
    metadata
        .entry("primary_artifact".to_string())
        .or_insert_with(|| Value::String("json+markdown".to_string()));
    metadata
        .entry("artifact_layers".to_string())
        .or_insert_with(|| {
            metadata_string_array(&["canonical-json", "readable-markdown", "search-index"])
        });
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
    metadata
        .entry("policy".to_string())
        .or_insert_with(default_policy_value);
    metadata
        .entry("production".to_string())
        .or_insert_with(default_production_value);
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
        bundle_family: Some("artifact".to_string()),
        envelope_path: None,
        redaction_profile: None,
        retention_days: 0,
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
    if manifest.bundle_family.as_ref().is_none() {
        manifest.bundle_family = Some(if manifest.doc_type == "lensmap_evidence_bundle" {
            "evidence".to_string()
        } else {
            "artifact".to_string()
        });
    }
    if manifest.retention_days < 0 {
        manifest.retention_days = 0;
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
    manifest
        .items
        .sort_by(|left, right| left.original_path.cmp(&right.original_path));
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
    let stdout = String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string();
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

fn git_changed_files(root: &Path, base: &str, head: &str) -> Option<Vec<String>> {
    let range = format!("{}..{}", base, head);
    let output = git_output(root, &["diff", "--name-only", &range])?;
    let mut files = BTreeMap::new();
    for line in output.lines() {
        let rel = line.trim().replace('\\', "/");
        if !rel.is_empty() {
            files.insert(rel, true);
        }
    }
    Some(files.keys().cloned().collect())
}

fn parse_git_status_rel(raw_line: &str) -> Option<String> {
    let line = raw_line.trim_end();
    if line.len() < 4 {
        return None;
    }
    let rel = line[3..]
        .rsplit_once(" -> ")
        .map(|(_, new_path)| new_path)
        .unwrap_or(&line[3..])
        .trim()
        .replace('\\', "/");
    if rel.is_empty() {
        None
    } else {
        Some(rel)
    }
}

fn git_worktree_changed_files(root: &Path) -> Vec<String> {
    let output = git_output(root, &["status", "--porcelain", "--untracked-files=all"]);
    let Some(output) = output.filter(|output| !output.trim().is_empty()) else {
        return vec![];
    };

    let mut files = BTreeMap::new();
    for raw_line in output.lines() {
        if let Some(rel) = parse_git_status_rel(raw_line) {
            files.insert(rel, true);
        }
    }
    files.keys().cloned().collect()
}

fn is_release_ref_context() -> bool {
    let ref_value = env::var("GITHUB_REF")
        .ok()
        .or_else(|| env::var("CI_COMMIT_REF_NAME").ok())
        .unwrap_or_default()
        .to_lowercase();
    ref_value.starts_with("refs/tags/v")
        || ref_value.starts_with("refs/heads/release/")
        || ref_value.starts_with("release/")
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

fn artifact_paths(root: &Path, lensmap_path: &Path) -> (String, String, String) {
    (
        normalize_relative(root, lensmap_path),
        normalize_relative(root, &default_render_output_path(lensmap_path)),
        normalize_relative(root, &default_index_path(root)),
    )
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

fn path_matches_patterns(path: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        let normalized = pattern.trim();
        !normalized.is_empty() && wildcard_match(normalized, path)
    })
}

fn collect_source_files_for_strip(
    root: &Path,
    sources: &[String],
    exclude_patterns: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut files = BTreeMap::new();
    let mut missing = vec![];
    for source in sources {
        let source = source.trim();
        if source.is_empty() {
            continue;
        }
        let abs = resolve_from_root(root, source);
        if !is_within_root(root, &abs) || !abs.exists() {
            missing.push(source.to_string());
            continue;
        }
        if abs.is_file() {
            let rel = normalize_relative(root, &abs);
            if !should_skip_rel(&rel) && !path_matches_patterns(&rel, exclude_patterns) {
                files.insert(rel, true);
            }
            continue;
        }
        for entry in WalkDir::new(&abs)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let file_abs = normalize_path(entry.path());
            let rel = normalize_relative(root, &file_abs);
            if should_skip_rel(&rel) || path_matches_patterns(&rel, exclude_patterns) {
                continue;
            }
            files.insert(rel, true);
        }
    }
    (files.keys().cloned().collect(), missing)
}

fn strip_markers_from_line(
    line: &str,
    clean_anchors: bool,
    clean_refs: bool,
) -> (String, usize, usize) {
    let mut current = line.to_string();
    let mut anchors_removed = 0usize;
    let mut refs_removed = 0usize;

    if clean_anchors {
        if let Some(anchor) = parse_anchor_line(&current) {
            anchors_removed = 1;
            if anchor.is_inline {
                if let Some(idx) = find_line_comment_index_outside_strings(&current, &anchor.marker)
                {
                    current = current[..idx].trim_end().to_string();
                } else {
                    current.clear();
                }
            } else {
                current.clear();
            }
        }
    }

    if clean_refs {
        let mut removed = false;
        for marker in ["//", "#"] {
            if let Some(ref_site) = locate_ref_site(&current, marker) {
                refs_removed = 1;
                removed = true;
                if ref_site.is_inline {
                    current = ref_site.prefix.trim_end().to_string();
                } else {
                    current.clear();
                }
                break;
            }
        }
        if !removed {
            if let Some(ref_match) = parse_ref_in_line(&current) {
                if let Some(idx) =
                    find_line_comment_index_outside_strings(&current, &ref_match.marker)
                {
                    refs_removed = 1;
                    let prefix = current[..idx].trim_end().to_string();
                    current = prefix;
                }
            }
        }
    }

    (current, anchors_removed, refs_removed)
}

fn is_supported_source_file(path: &Path) -> bool {
    let ext = ext_of(path);
    SUPPORTED_EXTS.contains(&ext.as_str())
}

fn strip_file_content(
    content: &str,
    clean_anchors: bool,
    clean_refs: bool,
) -> (String, usize, usize, bool) {
    let lines = split_lines(content);
    let mut out = Vec::with_capacity(lines.len());
    let mut anchors_removed = 0usize;
    let mut refs_removed = 0usize;
    let mut changed = false;
    for line in lines {
        let (stripped, anchors, refs) = strip_markers_from_line(&line, clean_anchors, clean_refs);
        if anchors > 0 || refs > 0 {
            changed = true;
        }
        anchors_removed += anchors;
        refs_removed += refs;
        out.push(stripped);
    }
    (join_lines(&out), anchors_removed, refs_removed, changed)
}

fn to_archive_rel(path: &Path) -> String {
    let mut parts = vec![];
    for component in path.components() {
        if let Component::Normal(part) = component {
            parts.push(part.to_string_lossy().to_string());
        }
    }
    parts.join("/")
}

fn build_deterministic_tar_gz(src_dir: &Path, out_path: &Path) -> Result<usize, String> {
    let mut paths = vec![];
    for entry in WalkDir::new(src_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file() {
            paths.push(normalize_path(entry.path()));
        }
    }
    paths.sort();
    ensure_dir(out_path);
    let file = fs::File::create(out_path).map_err(|error| error.to_string())?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);
    for path in &paths {
        let rel = path
            .strip_prefix(src_dir)
            .map_err(|error| error.to_string())?;
        let rel = to_archive_rel(rel);
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder
            .append_data(&mut header, rel, bytes.as_slice())
            .map_err(|error| error.to_string())?;
    }
    builder.finish().map_err(|error| error.to_string())?;
    Ok(paths.len())
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
    let requested = args
        .positional
        .get(2)
        .map(String::as_str)
        .unwrap_or("architecture")
        .trim()
        .to_lowercase();
    let template = builtin_template(&requested).unwrap_or_else(|| TemplateDefinition {
        doc_type: "lensmap-template".to_string(),
        name: requested.clone(),
        description: "Custom LensMap template.".to_string(),
        template: true,
        kind: "comment".to_string(),
        title_prefix: requested.to_uppercase(),
        body: "Context:\nAction:\nFollow-up:".to_string(),
        review_days: 30,
        tags: vec!["knowledge-boilerplate".to_string()],
        required_fields: vec!["text".to_string()],
    });
    let template_path = template_file_path(root, &template.name);
    ensure_dir(&template_path);
    let _ = fs::write(
        &template_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&template).unwrap_or_else(|_| "{}".to_string())
        ),
    );

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "template_add",
        "template": normalize_relative(root, &template_path),
        "name": template.name,
        "kind": template.kind,
        "tags": template.tags,
        "review_days": template.review_days,
        "ts": now_iso(),
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_template_list(root: &Path) {
    let mut templates = vec![];
    for template in template_library() {
        templates.push(json!({
            "name": template.name,
            "description": template.description,
            "kind": template.kind,
            "review_days": template.review_days,
            "tags": template.tags,
            "source": "builtin",
        }));
    }
    let templates_dir = root.join("templates");
    if templates_dir.exists() {
        for entry in WalkDir::new(&templates_dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
        {
            let path = entry.path();
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.ends_with(".lens.template.json"))
                .unwrap_or(false)
            {
                continue;
            }
            let raw = fs::read_to_string(path).unwrap_or_default();
            if let Ok(template) = serde_json::from_str::<TemplateDefinition>(&raw) {
                templates.push(json!({
                    "name": template.name,
                    "description": template.description,
                    "kind": template.kind,
                    "review_days": template.review_days,
                    "tags": template.tags,
                    "source": normalize_relative(root, path),
                }));
            }
        }
    }
    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "template_list",
        "templates": templates,
        "count": templates.len(),
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
    let strip_requested = bool_from_flag(args, "strip", args.has("strip"));
    let action_name = if args
        .positional
        .first()
        .map(|value| value.eq_ignore_ascii_case("unmerge"))
        .unwrap_or(false)
    {
        "unmerge"
    } else {
        "extract_comments"
    };
    let lensmap_path = resolve_lensmap_path(root, args, None);
    let mut doc = load_doc(&lensmap_path, "group");
    let covers = normalize_covers(args, &doc, &[]);
    let files = resolve_covers_to_files(root, &covers);

    if files.is_empty() {
        if !lensmap_path.exists() && covers.is_empty() {
            emit(
                lensmap_missing_payload(root, action_name, &lensmap_path, args),
                1,
            );
        }
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": action_name,
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
                title: None,
                owner: None,
                author: None,
                scope: None,
                template: None,
                review_status: None,
                review_due_at: None,
                updated_at: None,
                tags: vec![],
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

    let mut strip_details = None::<Value>;
    if strip_requested {
        let clean_anchors = bool_from_flag(args, "clean-anchors", true);
        let clean_refs = bool_from_flag(args, "clean-refs", true);
        let mut strip_summary = StripSummary::default();
        let mut marker_files = vec![];
        let mut changed_files = vec![];

        for rel in &files {
            let abs = resolve_from_root(root, rel);
            if !abs.exists() || !is_supported_source_file(&abs) {
                continue;
            }
            strip_summary.files_scanned += 1;
            let original_content = fs::read_to_string(&abs).unwrap_or_default();
            let (stripped, anchors_removed, refs_removed, changed) =
                strip_file_content(&original_content, clean_anchors, clean_refs);
            strip_summary.anchors_removed += anchors_removed;
            strip_summary.refs_removed += refs_removed;
            if anchors_removed + refs_removed > 0 {
                strip_summary.marker_hits += 1;
                marker_files.push(rel.clone());
            }
            if changed {
                strip_summary.files_changed += 1;
                changed_files.push(rel.clone());
            }
            if changed && !dry_run {
                let _ = fs::write(&abs, stripped);
            }
        }

        strip_details = Some(json!({
            "enabled": true,
            "clean_anchors": clean_anchors,
            "clean_refs": clean_refs,
            "summary": {
                "files_scanned": strip_summary.files_scanned,
                "files_changed": strip_summary.files_changed,
                "anchors_removed": strip_summary.anchors_removed,
                "refs_removed": strip_summary.refs_removed,
                "marker_hits": strip_summary.marker_hits,
            },
            "marker_files": marker_files,
            "changed_files": changed_files,
        }));
    }

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": action_name,
        "dry_run": dry_run,
        "strip_requested": strip_requested,
        "strip": strip_details,
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
    let raw_text = args
        .get("text")
        .or_else(|| args.positional.get(2).map(String::as_str))
        .unwrap_or("")
        .trim()
        .to_string();
    let requested_file = args.get("file").unwrap_or("").trim().to_string();
    let requested_symbol = args.get("symbol").unwrap_or("").trim().to_string();
    let requested_symbol_path = args.get("symbol-path").unwrap_or("").trim().to_string();
    let template_name = args.get("template").unwrap_or("").trim().to_lowercase();
    let template = if template_name.is_empty() {
        None
    } else {
        Some(load_template(root, &template_name).unwrap_or_else(|| {
            emit(
                json!({
                    "ok": false,
                    "type": "lensmap",
                    "action": "annotate",
                    "error": "template_missing",
                    "template": template_name,
                }),
                1,
            );
        }))
    };

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

    let text = if raw_text.is_empty() {
        template
            .as_ref()
            .map(|template| template.body.clone())
            .unwrap_or_default()
    } else {
        raw_text.clone()
    };
    if text.is_empty() {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "annotate",
                "error": "annotate_requires_text",
                "hint": tr(
                    "Use --text=<comment text> with either --ref=<HEX-start[-end]> or --file + --symbol, or pass --template=<name> for a boilerplate note skeleton.",
                    "请配合 --ref=<HEX-start[-end]> 或 --file + --symbol 一起使用 --text=<comment text>，或者通过 --template=<name> 使用结构化注释模板。",
                ),
                "example": "lensmap annotate --lensmap=demo/lensmap.json --file=demo/src/app.ts --symbol=run --template=review",
            }),
            1,
        );
    }

    let kind_raw = args
        .get("kind")
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
        .or_else(|| template.as_ref().map(|template| template.kind.clone()))
        .unwrap_or_else(|| "comment".to_string());
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
    let requested_title = args.get("title").unwrap_or("").trim().to_string();
    let requested_owner = args.get("owner").unwrap_or("").trim().to_string();
    let requested_author = args.get("author").unwrap_or("").trim().to_string();
    let requested_scope = args.get("scope").unwrap_or("").trim().to_string();
    let requested_review_status = args
        .get("review-status")
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let requested_review_due_at = args.get("review-due-at").unwrap_or("").trim().to_string();
    let requested_review_days = parse_i64_flag(args, "review-days");
    let requested_tags = parse_tags(args.get("tags").or_else(|| args.get("tag")));
    let default_title = template.as_ref().map(|template| {
        let subject = anchor
            .symbol_path
            .clone()
            .or(anchor.symbol.clone())
            .unwrap_or_else(|| canonical_ref.clone());
        format!("{}: {}", template.title_prefix, subject)
    });
    let default_review_status = template.as_ref().map(|template| {
        if template.name == "review" {
            "in_review".to_string()
        } else {
            "draft".to_string()
        }
    });
    let template_review_days = template.as_ref().map(|template| template.review_days);
    let merged_tags = normalize_tags(
        &template
            .as_ref()
            .map(|template| {
                let mut combined = template.tags.clone();
                combined.extend(requested_tags.clone());
                combined
            })
            .unwrap_or_else(|| requested_tags.clone()),
    );
    let mut updated = false;

    for entry in &mut doc.entries {
        if entry.file == file && entry.ref_id.eq_ignore_ascii_case(&canonical_ref) {
            entry.anchor_id = Some(parsed.anchor_id.clone());
            entry.kind = Some(kind.clone());
            entry.text = Some(text.clone());
            entry.source = Some(source.clone());
            entry.created_at = Some(ts.clone());
            entry.updated_at = Some(ts.clone());
            if !requested_title.is_empty() {
                entry.title = Some(requested_title.clone());
            } else if entry.title.is_none() {
                entry.title = default_title.clone();
            }
            if !requested_owner.is_empty() {
                entry.owner = Some(requested_owner.clone());
            }
            if !requested_author.is_empty() {
                entry.author = Some(requested_author.clone());
            } else if entry.author.is_none() {
                entry.author = default_author();
            }
            if !requested_scope.is_empty() {
                entry.scope = Some(requested_scope.clone());
            } else if entry.scope.is_none() {
                entry.scope = Some(default_scope_for_file(&file));
            }
            if let Some(template) = &template {
                entry.template = Some(template.name.clone());
            }
            if !requested_review_status.is_empty() {
                entry.review_status = Some(requested_review_status.clone());
            } else if entry.review_status.is_none() {
                entry.review_status = default_review_status.clone();
            }
            if !requested_review_due_at.is_empty() {
                entry.review_due_at = Some(requested_review_due_at.clone());
            } else if let Some(days) = requested_review_days.or(template_review_days) {
                if entry.review_due_at.is_none() || !requested_review_status.is_empty() {
                    entry.review_due_at =
                        Some((Utc::now() + Duration::days(days.max(1))).to_rfc3339());
                }
            }
            if !merged_tags.is_empty() {
                entry.tags = normalize_tags(
                    &entry
                        .tags
                        .iter()
                        .cloned()
                        .chain(merged_tags.clone().into_iter())
                        .collect::<Vec<_>>(),
                );
            }
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
            title: if requested_title.is_empty() {
                default_title.clone()
            } else {
                Some(requested_title.clone())
            },
            owner: if requested_owner.is_empty() {
                None
            } else {
                Some(requested_owner.clone())
            },
            author: if requested_author.is_empty() {
                default_author()
            } else {
                Some(requested_author.clone())
            },
            scope: if requested_scope.is_empty() {
                Some(default_scope_for_file(&file))
            } else {
                Some(requested_scope.clone())
            },
            template: template.as_ref().map(|template| template.name.clone()),
            review_status: if requested_review_status.is_empty() {
                default_review_status.clone()
            } else {
                Some(requested_review_status.clone())
            },
            review_due_at: if !requested_review_due_at.is_empty() {
                Some(requested_review_due_at.clone())
            } else {
                requested_review_days
                    .or(template_review_days)
                    .map(|days| (Utc::now() + Duration::days(days.max(1))).to_rfc3339())
            },
            updated_at: Some(ts.clone()),
            tags: merged_tags.clone(),
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
        "template": template.as_ref().map(|template| template.name.clone()),
        "owner": if requested_owner.is_empty() { None::<String> } else { Some(requested_owner) },
        "author": if requested_author.is_empty() { default_author() } else { Some(requested_author) },
        "scope": if requested_scope.is_empty() { Some(default_scope_for_file(&file)) } else { Some(requested_scope) },
        "review_status": if requested_review_status.is_empty() { default_review_status } else { Some(requested_review_status) },
        "tags": merged_tags,
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

fn cmd_strip(root: &Path, args: &ParsedArgs) {
    let in_place = args.has("in-place");
    let force = args.has("force");
    let check_only = args.has("check");
    let dry_run = args.has("dry-run");
    if in_place && !force {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "strip",
                "error": "strip_in_place_requires_force",
                "hint": tr(
                    "Use --in-place with --force to acknowledge source mutation.",
                    "使用 --in-place 时必须同时传入 --force 以确认会修改源码。",
                ),
            }),
            1,
        );
    }

    let lensmap_path = resolve_lensmap_path(root, args, None);
    let mut production = default_production_settings();
    let mut sources = split_csv(args.get("source"));
    if lensmap_path.exists() {
        let doc = load_doc(&lensmap_path, "group");
        production = production_for_doc(&doc);
        if sources.is_empty() && !doc.covers.is_empty() {
            sources = doc.covers.clone();
        }
    }
    if sources.is_empty() {
        sources.push(".".to_string());
    }

    let mut exclude_patterns = if let Some(raw) = args
        .get("exclude-patterns")
        .or_else(|| args.get("exclude-pattern"))
    {
        split_csv(Some(raw))
    } else {
        production.exclude_patterns.clone()
    };
    exclude_patterns.sort();
    exclude_patterns.dedup();

    let clean_anchors = bool_from_flag(
        args,
        "clean-anchors",
        if production.strip_anchors || production.strip_refs {
            production.strip_anchors
        } else {
            true
        },
    );
    let clean_refs = bool_from_flag(
        args,
        "clean-refs",
        if production.strip_anchors || production.strip_refs {
            production.strip_refs
        } else {
            true
        },
    );
    let out_dir = args.get("out-dir").unwrap_or("dist/prod");
    let out_root = resolve_from_root(root, out_dir);
    if !in_place && !is_within_root(root, &out_root) {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "strip",
                "error": "security_output_outside_root",
                "out_dir": out_dir,
            }),
            1,
        );
    }

    let (files, missing_sources) =
        collect_source_files_for_strip(root, &sources, &exclude_patterns);
    if files.is_empty() {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "strip",
                "error": "no_source_files_found",
                "sources": sources,
                "missing_sources": missing_sources,
            }),
            1,
        );
    }
    if !in_place && !check_only && !dry_run && out_root.exists() {
        let _ = fs::remove_dir_all(&out_root);
    }

    let mut summary = StripSummary::default();
    let mut marker_files = vec![];
    let mut file_summaries = vec![];

    for rel in &files {
        let abs = resolve_from_root(root, rel);
        if !abs.exists() {
            continue;
        }
        summary.files_scanned += 1;
        let is_supported = is_supported_source_file(&abs);

        if is_supported {
            let original_content = fs::read_to_string(&abs).unwrap_or_default();
            let (stripped, anchors_removed, refs_removed, changed) =
                strip_file_content(&original_content, clean_anchors, clean_refs);
            summary.anchors_removed += anchors_removed;
            summary.refs_removed += refs_removed;
            if anchors_removed + refs_removed > 0 {
                summary.marker_hits += 1;
                marker_files.push(rel.clone());
            }
            if changed {
                summary.files_changed += 1;
            }

            if !check_only && !dry_run {
                if in_place {
                    if changed {
                        let _ = fs::write(&abs, stripped);
                    }
                } else {
                    let out_path = out_root.join(rel);
                    ensure_dir(&out_path);
                    if changed {
                        let _ = fs::write(&out_path, stripped);
                    } else {
                        let _ = fs::copy(&abs, &out_path);
                    }
                }
            }

            file_summaries.push(json!({
                "file": rel,
                "supported": true,
                "anchors_removed": anchors_removed,
                "refs_removed": refs_removed,
                "changed": changed,
            }));
            continue;
        }

        if !in_place && !check_only && !dry_run {
            let out_path = out_root.join(rel);
            ensure_dir(&out_path);
            let _ = fs::copy(&abs, &out_path);
        }

        file_summaries.push(json!({
            "file": rel,
            "supported": false,
            "anchors_removed": 0,
            "refs_removed": 0,
            "changed": false,
        }));
    }

    let out = json!({
        "ok": !(check_only && summary.marker_hits > 0),
        "type": "lensmap",
        "action": "strip",
        "command_version": COMMAND_VERSION,
        "check_only": check_only,
        "in_place": in_place,
        "dry_run": dry_run,
        "force": force,
        "sources": sources,
        "missing_sources": missing_sources,
        "exclude_patterns": exclude_patterns,
        "out_dir": if in_place { None::<String> } else { Some(normalize_relative(root, &out_root)) },
        "clean_anchors": clean_anchors,
        "clean_refs": clean_refs,
        "summary": {
            "files_scanned": summary.files_scanned,
            "files_changed": summary.files_changed,
            "anchors_removed": summary.anchors_removed,
            "refs_removed": summary.refs_removed,
            "marker_hits": summary.marker_hits,
        },
        "marker_files": marker_files,
        "files": file_summaries,
        "ts": now_iso(),
    });
    append_history(root, &out);
    emit(
        out,
        if check_only && summary.marker_hits > 0 {
            1
        } else {
            0
        },
    );
}

fn package_stripped_sources(
    root: &Path,
    args: &ParsedArgs,
    bundle_abs: &Path,
    dry_run: bool,
    source_hints: &[String],
    production: &ProductionSettings,
) -> Option<Value> {
    if !args.has("strip-sources") && !args.has("production") && !production.strip_on_package {
        return None;
    }

    let out_format = args
        .get("out-format")
        .unwrap_or("tar.gz")
        .trim()
        .to_lowercase();
    if out_format != "tar.gz" {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "package",
                "error": "invalid_out_format",
                "out_format": out_format,
                "allowed": ["tar.gz"],
            }),
            1,
        );
    }

    let mut sources = split_csv(args.get("source"));
    if sources.is_empty() {
        sources = source_hints.to_vec();
    }
    if sources.is_empty() {
        sources.push(".".to_string());
    }

    let mut exclude_patterns = if let Some(raw) = args
        .get("strip-exclude-patterns")
        .or_else(|| args.get("exclude-patterns"))
    {
        split_csv(Some(raw))
    } else {
        production.exclude_patterns.clone()
    };
    exclude_patterns.sort();
    exclude_patterns.dedup();

    let clean_anchors = bool_from_flag(
        args,
        "clean-anchors",
        if args.has("strip-sources") || args.has("production") {
            true
        } else {
            production.strip_anchors
        },
    );
    let clean_refs = bool_from_flag(
        args,
        "clean-refs",
        if args.has("strip-sources") || args.has("production") {
            true
        } else {
            production.strip_refs
        },
    );
    let (files, missing_sources) =
        collect_source_files_for_strip(root, &sources, &exclude_patterns);
    if files.is_empty() {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "package",
                "error": "strip_sources_no_files",
                "sources": sources,
                "missing_sources": missing_sources,
            }),
            1,
        );
    }

    let stage_dir = bundle_abs.join("prod-sources");
    let archive_path = bundle_abs.join("prod-sources.tar.gz");
    if !is_within_root(root, &stage_dir) || !is_within_root(root, &archive_path) {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "package",
                "error": "security_output_outside_root",
            }),
            1,
        );
    }
    if !dry_run && stage_dir.exists() {
        let _ = fs::remove_dir_all(&stage_dir);
    }

    let mut summary = StripSummary::default();
    let mut marker_files = vec![];
    for rel in &files {
        let abs = resolve_from_root(root, rel);
        if !abs.exists() {
            continue;
        }
        summary.files_scanned += 1;
        if is_supported_source_file(&abs) {
            let original_content = fs::read_to_string(&abs).unwrap_or_default();
            let (stripped, anchors_removed, refs_removed, changed) =
                strip_file_content(&original_content, clean_anchors, clean_refs);
            summary.anchors_removed += anchors_removed;
            summary.refs_removed += refs_removed;
            if anchors_removed + refs_removed > 0 {
                summary.marker_hits += 1;
                marker_files.push(rel.clone());
            }
            if changed {
                summary.files_changed += 1;
            }
            if !dry_run {
                let out_path = stage_dir.join(rel);
                ensure_dir(&out_path);
                if changed {
                    let _ = fs::write(&out_path, stripped);
                } else {
                    let _ = fs::copy(&abs, &out_path);
                }
            }
            continue;
        }
        if !dry_run {
            let out_path = stage_dir.join(rel);
            ensure_dir(&out_path);
            let _ = fs::copy(&abs, &out_path);
        }
    }

    let archived_files = if dry_run {
        0usize
    } else {
        build_deterministic_tar_gz(&stage_dir, &archive_path).unwrap_or_else(|error| {
            emit(
                json!({
                    "ok": false,
                    "type": "lensmap",
                    "action": "package",
                    "error": "strip_archive_failed",
                    "message": error,
                }),
                1,
            );
        })
    };

    Some(json!({
        "enabled": true,
        "out_format": out_format,
        "sources": sources,
        "missing_sources": missing_sources,
        "exclude_patterns": exclude_patterns,
        "stage_dir": normalize_relative(root, &stage_dir),
        "archive": normalize_relative(root, &archive_path),
        "clean_anchors": clean_anchors,
        "clean_refs": clean_refs,
        "summary": {
            "files_scanned": summary.files_scanned,
            "files_changed": summary.files_changed,
            "anchors_removed": summary.anchors_removed,
            "refs_removed": summary.refs_removed,
            "marker_hits": summary.marker_hits,
            "archived_files": archived_files,
        },
        "marker_files": marker_files,
    }))
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
    let package_lensmaps = candidate_files.clone();
    let loaded_docs = load_repo_lensmap_docs(root, &package_lensmaps);
    let production = aggregate_production_settings(
        loaded_docs
            .iter()
            .map(|loaded| production_for_doc(&loaded.doc))
            .collect::<Vec<_>>()
            .iter(),
    );
    let mut source_hints = BTreeMap::new();
    for loaded in &loaded_docs {
        for cover in &loaded.doc.covers {
            let normalized = cover.trim();
            if !normalized.is_empty() {
                source_hints.insert(normalized.to_string(), true);
            }
        }
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

        let source_hash = file_fingerprint(&abs);
        let packaged_hash = if dry_run {
            None
        } else {
            file_fingerprint(&packaged_abs)
        };
        let bytes = fs::metadata(&abs).ok().map(|m| m.len());

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
            source_hash,
            packaged_hash,
            bytes,
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
    let strip_sources = package_stripped_sources(
        root,
        args,
        &bundle_abs,
        dry_run,
        &source_hints.keys().cloned().collect::<Vec<_>>(),
        &production,
    );

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
        "strip_sources": strip_sources,
        "ts": now_iso(),
        "files": file_summaries,
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_package_evidence(root: &Path, args: &ParsedArgs) {
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
    let compression_mode = args
        .get("compression-mode")
        .unwrap_or("copy")
        .trim()
        .to_lowercase();
    if !["copy", "none"].contains(&compression_mode.as_str()) {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "package_evidence",
                "error": "invalid_compression_mode",
                "compression_mode": compression_mode,
                "allowed": ["copy", "none"],
            }),
            1,
        );
    }
    let keep_source = true; // evidence packaging is copy-only
    let profile = parse_redaction_profile(args, "redaction-profile");
    let retention_days = args
        .get("retention-days")
        .and_then(|raw| raw.parse::<i64>().ok())
        .unwrap_or_else(|| profile.default_retention_days())
        .max(0);

    let bundle_abs = resolve_from_root(root, &bundle_dir);
    if !is_within_root(root, &bundle_abs) {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "package_evidence",
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

    // Optional evidence inclusions
    if args.has("include-metrics") {
        let path = default_metric_output_path(root);
        if path.exists() {
            candidate_files.push(normalize_relative(root, &path));
        }
    }
    if args.has("include-scorecard") {
        let path = default_scorecard_output_path(root);
        if path.exists() {
            candidate_files.push(normalize_relative(root, &path));
        }
    }
    if args.has("include-pr-report") {
        let path = default_pr_report_output_path(root);
        if path.exists() {
            candidate_files.push(normalize_relative(root, &path));
        }
    }
    if args.has("include-policy") {
        let path = default_policy_output_path(root);
        if path.exists() {
            candidate_files.push(normalize_relative(root, &path));
        }
    }

    if candidate_files.is_empty() {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "package_evidence",
                "error": "no_input_files_found",
                "hint": tr(
                    "Create lensmaps or run metrics/scorecard first.",
                    "请先生成 lensmap 或运行 metrics/scorecard。",
                ),
            }),
            1,
        );
    }
    let evidence_lensmaps = candidate_files
        .iter()
        .map(|raw| resolve_from_root(root, raw))
        .filter(|path| is_within_root(root, path) && path.exists() && is_lensmap_filename(path))
        .map(|path| normalize_relative(root, &path))
        .collect::<Vec<_>>();
    let evidence_docs = load_repo_lensmap_docs(root, &evidence_lensmaps);
    let production = aggregate_production_settings(
        evidence_docs
            .iter()
            .map(|loaded| production_for_doc(&loaded.doc))
            .collect::<Vec<_>>()
            .iter(),
    );
    let mut source_hints = BTreeMap::new();
    for loaded in &evidence_docs {
        for cover in &loaded.doc.covers {
            let normalized = cover.trim();
            if !normalized.is_empty() {
                source_hints.insert(normalized.to_string(), true);
            }
        }
    }

    let files_dir = bundle_abs.join("files");
    let manifest_path = bundle_abs.join("manifest.json");
    let mut manifest = load_package_manifest(&manifest_path, root, &bundle_dir);
    manifest.doc_type = "lensmap_evidence_bundle".to_string();
    manifest.bundle_family = Some("evidence".to_string());
    manifest.redaction_profile = Some(profile.label().to_string());
    manifest.retention_days = retention_days;
    manifest.envelope_path = Some("envelope.json".to_string());

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

        let rel = normalize_relative(root, &abs);
        let id = hash_text(&rel).to_uppercase();
        let ext = Path::new(&rel)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("dat");
        let packaged_rel = format!("files/{}.{}", id, ext);
        let packaged_abs = files_dir.join(format!("{}.{}", id, ext));

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

        let source_hash = file_fingerprint(&abs);
        let packaged_hash = if dry_run {
            None
        } else {
            file_fingerprint(&packaged_abs)
        };
        let bytes = fs::metadata(&abs).ok().map(|m| m.len());

        let item = PackageItem {
            id: id.clone(),
            original_path: rel.clone(),
            packaged_path: packaged_rel.clone(),
            status: status.clone(),
            resolved_path: None,
            last_error: err.clone(),
            updated_at: Some(now_iso()),
            source_hash,
            packaged_hash,
            bytes,
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
    let strip_sources = package_stripped_sources(
        root,
        args,
        &bundle_abs,
        dry_run,
        &source_hints.keys().cloned().collect::<Vec<_>>(),
        &production,
    );

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "package_evidence",
        "bundle_dir": normalize_relative(root, &bundle_abs),
        "manifest": normalize_relative(root, &manifest_path),
        "packaged_count": packaged_count,
        "retention_days": retention_days,
        "redaction_profile": profile.label(),
        "compression_mode": compression_mode,
        "strip_sources": strip_sources,
        "skipped": skipped,
        "ts": now_iso(),
        "files": file_summaries,
    });

    emit_with_envelope(
        root,
        "lensmap",
        "package_evidence",
        args,
        out,
        profile,
        &default_policy_settings(),
        &[],
        &[manifest_path],
        0,
        vec![],
    );
}

fn cmd_verify(root: &Path, args: &ParsedArgs) {
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
    let bundle_abs = resolve_from_root(root, &bundle_dir);
    if !is_within_root(root, &bundle_abs) {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "verify",
                "error": "security_bundle_outside_root",
                "bundle_dir": bundle_dir,
            }),
            1,
        );
    }
    let manifest_path = bundle_abs.join("manifest.json");
    if !manifest_path.exists() {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "verify",
                "error": "manifest_missing",
                "manifest": normalize_relative(root, &manifest_path),
            }),
            1,
        );
    }
    let mut manifest = load_package_manifest(&manifest_path, root, &bundle_dir);
    manifest = normalize_package_manifest(manifest, root, &bundle_dir);

    let mut errors = vec![];
    if let Some(envelope_rel) = manifest.envelope_path.clone() {
        let envelope_abs = bundle_abs.join(envelope_rel);
        if !envelope_abs.exists() {
            errors.push("envelope_missing".to_string());
        }
    }
    let mut verified = 0usize;
    for item in &manifest.items {
        let packaged_abs = bundle_abs.join(&item.packaged_path);
        if !packaged_abs.exists() {
            errors.push(format!("missing_packaged:{}", item.packaged_path));
            continue;
        }
        if let Some(expected) = &item.packaged_hash {
            if let Some(actual) = file_fingerprint(&packaged_abs) {
                if &actual != expected {
                    errors.push(format!(
                        "hash_mismatch:{} expected={} actual={}",
                        item.packaged_path, expected, actual
                    ));
                    continue;
                }
            }
        }
        if let Some(original) = &item.source_hash {
            let original_abs = resolve_from_root(root, &item.original_path);
            if original_abs.exists() {
                if let Some(actual) = file_fingerprint(&original_abs) {
                    if &actual != original {
                        errors.push(format!(
                            "source_hash_mismatch:{} expected={} actual={}",
                            item.original_path, original, actual
                        ));
                        continue;
                    }
                }
            }
        }
        verified += 1;
    }

    let ok = errors.is_empty();
    let out = json!({
        "ok": ok,
        "type": "lensmap",
        "action": "verify",
        "bundle_dir": normalize_relative(root, &bundle_abs),
        "manifest": normalize_relative(root, &manifest_path),
        "verified": verified,
        "items": manifest.items.len(),
        "errors": errors,
        "ts": now_iso(),
    });

    emit_with_envelope(
        root,
        "lensmap",
        "verify",
        args,
        out,
        parse_redaction_profile(args, "redaction-profile"),
        &default_policy_settings(),
        &[],
        &[manifest_path],
        if ok { 0 } else { 1 },
        vec![],
    );
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

fn validate_doc(root: &Path, lensmap_path: &Path, doc: &LensMapDoc) -> ValidationFindings {
    let lensmap_rel = normalize_relative(root, lensmap_path);
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

    ValidationFindings {
        errors,
        warnings,
        lensmap_dirty,
    }
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
    let findings = validate_doc(root, &lensmap_path, &doc);
    let ok = findings.errors.is_empty();
    let out = json!({
        "ok": ok,
        "type": "lensmap",
        "action": "validate",
        "lensmap": normalize_relative(root, &lensmap_path),
        "git": {
            "lensmap_dirty": findings.lensmap_dirty,
        },
        "errors": findings.errors,
        "warnings": findings.warnings,
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

fn increment_counter(map: &mut BTreeMap<String, usize>, raw: &str, fallback: &str) {
    let value = if raw.trim().is_empty() {
        fallback.trim()
    } else {
        raw.trim()
    };
    if value.is_empty() {
        return;
    }
    *map.entry(value.to_string()).or_insert(0) += 1;
}

fn counter_rows(map: &BTreeMap<String, usize>, limit: usize) -> Vec<Value> {
    let mut rows = map
        .iter()
        .map(|(name, count)| (name.clone(), *count))
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    if limit > 0 && rows.len() > limit {
        rows.truncate(limit);
    }
    rows.into_iter()
        .map(|(name, count)| json!({"name": name, "count": count}))
        .collect()
}

fn render_counter_section(
    lines: &mut Vec<String>,
    title: &str,
    map: &BTreeMap<String, usize>,
    limit: usize,
) {
    lines.push(format!("## {}", title));
    if map.is_empty() {
        lines.push("- none".to_string());
        lines.push(String::new());
        return;
    }
    for row in counter_rows(map, limit) {
        let name = row.get("name").and_then(Value::as_str).unwrap_or("unknown");
        let count = row.get("count").and_then(Value::as_u64).unwrap_or(0);
        lines.push(format!("- `{}`: {}", name, count));
    }
    lines.push(String::new());
}

#[cfg(test)]
fn collect_policy_findings(
    root: &Path,
    doc: &LensMapDoc,
) -> (PolicySettings, Vec<PolicyFinding>, Vec<PolicyFinding>) {
    let policy = policy_for_doc(doc);
    let mut errors = vec![];
    let mut warnings = vec![];

    for entry in &doc.entries {
        if policy.require_owner && entry.owner.as_deref().unwrap_or("").trim().is_empty() {
            errors.push(PolicyFinding {
                level: "error".to_string(),
                ref_id: entry.ref_id.clone(),
                lensmap: None,
                field: "owner".to_string(),
                code: "missing_owner".to_string(),
                message: format!("Entry {} is missing an owner.", entry.ref_id),
            });
        }
        if policy.require_author && entry.author.as_deref().unwrap_or("").trim().is_empty() {
            errors.push(PolicyFinding {
                level: "error".to_string(),
                ref_id: entry.ref_id.clone(),
                lensmap: None,
                field: "author".to_string(),
                code: "missing_author".to_string(),
                message: format!("Entry {} is missing an author.", entry.ref_id),
            });
        }
        if policy.require_template && entry.template.as_deref().unwrap_or("").trim().is_empty() {
            errors.push(PolicyFinding {
                level: "error".to_string(),
                ref_id: entry.ref_id.clone(),
                lensmap: None,
                field: "template".to_string(),
                code: "missing_template".to_string(),
                message: format!("Entry {} is missing a template.", entry.ref_id),
            });
        }
        if policy.require_review_status
            && entry
                .review_status
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
        {
            errors.push(PolicyFinding {
                level: "error".to_string(),
                ref_id: entry.ref_id.clone(),
                lensmap: None,
                field: "review_status".to_string(),
                code: "missing_review_status".to_string(),
                message: format!("Entry {} is missing a review status.", entry.ref_id),
            });
        }
        if policy.stale_after_days > 0 && entry_is_stale(entry, policy.stale_after_days) {
            warnings.push(PolicyFinding {
                level: "warning".to_string(),
                ref_id: entry.ref_id.clone(),
                lensmap: None,
                field: "review_due_at".to_string(),
                code: "stale_entry".to_string(),
                message: format!("Entry {} is stale and should be reviewed.", entry.ref_id),
            });
        }
    }

    if !policy.required_patterns.is_empty() {
        let all_files = resolve_covers_to_files(root, &doc.covers);
        let noted_files = doc
            .entries
            .iter()
            .filter_map(|entry| {
                let file = entry.file.trim();
                if file.is_empty() {
                    None
                } else {
                    Some(file.to_string())
                }
            })
            .collect::<HashSet<_>>();

        for pattern in &policy.required_patterns {
            let matched = all_files
                .iter()
                .filter(|file| wildcard_match(pattern, file))
                .cloned()
                .collect::<Vec<_>>();
            if matched.is_empty() {
                warnings.push(PolicyFinding {
                    level: "warning".to_string(),
                    ref_id: format!("policy:{}", pattern),
                    lensmap: None,
                    field: "required_patterns".to_string(),
                    code: "pattern_matches_no_files".to_string(),
                    message: format!("Policy pattern '{}' matched no covered files.", pattern),
                });
                continue;
            }
            let uncovered = matched
                .iter()
                .filter(|file| !noted_files.contains(*file))
                .cloned()
                .collect::<Vec<_>>();
            if !uncovered.is_empty() {
                let preview = uncovered
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                errors.push(PolicyFinding {
                    level: "error".to_string(),
                    ref_id: format!("policy:{}", pattern),
                    lensmap: None,
                    field: "required_patterns".to_string(),
                    code: "required_pattern_missing_notes".to_string(),
                    message: format!(
                        "Policy pattern '{}' matched files without LensMap notes: {}{}",
                        pattern,
                        preview,
                        if uncovered.len() > 3 { ", ..." } else { "" }
                    ),
                });
            }
        }
    }

    (policy, errors, warnings)
}

fn collect_aggregated_policy_findings(
    root: &Path,
    docs: &[LoadedLensMapDoc],
) -> (
    PolicySettings,
    Vec<PolicyFinding>,
    Vec<PolicyFinding>,
    Vec<Value>,
) {
    let policy_pairs = docs
        .iter()
        .map(|loaded| (loaded.lensmap.clone(), policy_for_doc(&loaded.doc)))
        .collect::<Vec<_>>();
    let aggregated_policy =
        aggregate_policy_settings(policy_pairs.iter().map(|(_, policy)| policy));
    let policy_sources = policy_pairs
        .iter()
        .map(|(lensmap, policy)| {
            json!({
                "lensmap": lensmap,
                "policy": policy,
            })
        })
        .collect::<Vec<_>>();

    let mut errors = vec![];
    let mut warnings = vec![];
    let mut all_files = BTreeMap::new();
    let mut noted_files = HashSet::new();

    for loaded in docs {
        for file in resolve_covers_to_files(root, &loaded.doc.covers) {
            all_files.insert(file, true);
        }
        for entry in &loaded.doc.entries {
            let lensmap = Some(loaded.lensmap.clone());
            let file = entry.file.trim();
            if !file.is_empty() {
                noted_files.insert(file.to_string());
            }

            if aggregated_policy.require_owner
                && entry.owner.as_deref().unwrap_or("").trim().is_empty()
            {
                errors.push(PolicyFinding {
                    level: "error".to_string(),
                    ref_id: entry.ref_id.clone(),
                    lensmap: lensmap.clone(),
                    field: "owner".to_string(),
                    code: "missing_owner".to_string(),
                    message: format!("Entry {} is missing an owner.", entry.ref_id),
                });
            }
            if aggregated_policy.require_author
                && entry.author.as_deref().unwrap_or("").trim().is_empty()
            {
                errors.push(PolicyFinding {
                    level: "error".to_string(),
                    ref_id: entry.ref_id.clone(),
                    lensmap: lensmap.clone(),
                    field: "author".to_string(),
                    code: "missing_author".to_string(),
                    message: format!("Entry {} is missing an author.", entry.ref_id),
                });
            }
            if aggregated_policy.require_template
                && entry.template.as_deref().unwrap_or("").trim().is_empty()
            {
                errors.push(PolicyFinding {
                    level: "error".to_string(),
                    ref_id: entry.ref_id.clone(),
                    lensmap: lensmap.clone(),
                    field: "template".to_string(),
                    code: "missing_template".to_string(),
                    message: format!("Entry {} is missing a template.", entry.ref_id),
                });
            }
            if aggregated_policy.require_review_status
                && entry
                    .review_status
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
            {
                errors.push(PolicyFinding {
                    level: "error".to_string(),
                    ref_id: entry.ref_id.clone(),
                    lensmap: lensmap.clone(),
                    field: "review_status".to_string(),
                    code: "missing_review_status".to_string(),
                    message: format!("Entry {} is missing a review status.", entry.ref_id),
                });
            }
            if aggregated_policy.stale_after_days > 0
                && entry_is_stale(entry, aggregated_policy.stale_after_days)
            {
                warnings.push(PolicyFinding {
                    level: "warning".to_string(),
                    ref_id: entry.ref_id.clone(),
                    lensmap,
                    field: "review_due_at".to_string(),
                    code: "stale_entry".to_string(),
                    message: format!("Entry {} is stale and should be reviewed.", entry.ref_id),
                });
            }
        }
    }

    if !aggregated_policy.required_patterns.is_empty() {
        let covered_files = all_files.keys().cloned().collect::<Vec<_>>();
        for pattern in &aggregated_policy.required_patterns {
            let matched = covered_files
                .iter()
                .filter(|file| wildcard_match(pattern, file))
                .cloned()
                .collect::<Vec<_>>();
            if matched.is_empty() {
                warnings.push(PolicyFinding {
                    level: "warning".to_string(),
                    ref_id: format!("policy:{}", pattern),
                    lensmap: None,
                    field: "required_patterns".to_string(),
                    code: "pattern_matches_no_files".to_string(),
                    message: format!("Policy pattern '{}' matched no covered files.", pattern),
                });
                continue;
            }
            let uncovered = matched
                .iter()
                .filter(|file| !noted_files.contains(*file))
                .cloned()
                .collect::<Vec<_>>();
            if !uncovered.is_empty() {
                let preview = uncovered
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                errors.push(PolicyFinding {
                    level: "error".to_string(),
                    ref_id: format!("policy:{}", pattern),
                    lensmap: None,
                    field: "required_patterns".to_string(),
                    code: "required_pattern_missing_notes".to_string(),
                    message: format!(
                        "Policy pattern '{}' matched files without LensMap notes: {}{}",
                        pattern,
                        preview,
                        if uncovered.len() > 3 { ", ..." } else { "" }
                    ),
                });
            }
        }
    }

    (aggregated_policy, errors, warnings, policy_sources)
}

fn collect_production_marker_hits(
    root: &Path,
    docs: &[LoadedLensMapDoc],
    production: &ProductionSettings,
) -> Vec<String> {
    if !production.strip_anchors && !production.strip_refs {
        return vec![];
    }
    let mut candidate_files = BTreeMap::new();
    for loaded in docs {
        for file in resolve_covers_to_files(root, &loaded.doc.covers) {
            if !path_matches_patterns(&file, &production.exclude_patterns) {
                candidate_files.insert(file, true);
            }
        }
    }

    let mut hits = vec![];
    for rel in candidate_files.keys() {
        let abs = resolve_from_root(root, rel);
        if !is_within_root(root, &abs) || !abs.exists() || !is_supported_source_file(&abs) {
            continue;
        }
        let content = fs::read_to_string(&abs).unwrap_or_default();
        let lines = split_lines(&content);
        let mut has_marker = false;
        for line in lines {
            let (_stripped, anchors_removed, refs_removed) =
                strip_markers_from_line(&line, production.strip_anchors, production.strip_refs);
            if anchors_removed + refs_removed > 0 {
                has_marker = true;
                break;
            }
        }
        if has_marker {
            hits.push(rel.clone());
        }
    }
    hits
}

fn summary_stats(entries: &[SearchEntryRecord], stale_after_days: i64) -> SummaryStats {
    let mut stats = SummaryStats {
        entry_count: entries.len(),
        ..SummaryStats::default()
    };

    for entry in entries {
        increment_counter(&mut stats.by_file, &entry.file, "unknown");
        let directory = Path::new(&entry.file)
            .parent()
            .map(to_posix_str)
            .filter(|value| !value.is_empty() && value != ".")
            .unwrap_or_else(|| "repo".to_string());
        increment_counter(&mut stats.by_directory, &directory, "repo");
        increment_counter(
            &mut stats.by_owner,
            entry.owner.as_deref().unwrap_or(""),
            "unassigned",
        );
        increment_counter(
            &mut stats.by_kind,
            entry.kind.as_deref().unwrap_or(""),
            "comment",
        );
        increment_counter(
            &mut stats.by_template,
            entry.template.as_deref().unwrap_or(""),
            "ad-hoc",
        );
        increment_counter(
            &mut stats.by_review_status,
            entry.review_status.as_deref().unwrap_or(""),
            "unspecified",
        );
        increment_counter(
            &mut stats.by_scope,
            entry.scope.as_deref().unwrap_or(""),
            "repo",
        );

        let entry_record = EntryRecord {
            ref_id: entry.ref_id.clone(),
            file: entry.file.clone(),
            anchor_id: entry.anchor_id.clone(),
            kind: entry.kind.clone(),
            text: entry.text.clone(),
            title: entry.title.clone(),
            owner: entry.owner.clone(),
            author: entry.author.clone(),
            scope: entry.scope.clone(),
            template: entry.template.clone(),
            review_status: entry.review_status.clone(),
            review_due_at: entry.review_due_at.clone(),
            updated_at: entry.updated_at.clone(),
            tags: entry.tags.clone(),
            created_at: None,
            source: None,
        };
        if stale_after_days > 0 && entry_is_stale(&entry_record, stale_after_days) {
            stats.stale_entries += 1;
        }
    }

    stats.files_with_notes = stats.by_file.len();
    stats
}

fn summary_stats_to_value(stats: &SummaryStats, top: usize) -> Value {
    json!({
        "entry_count": stats.entry_count,
        "files_with_notes": stats.files_with_notes,
        "stale_entries": stats.stale_entries,
        "by_file": counter_rows(&stats.by_file, top),
        "by_directory": counter_rows(&stats.by_directory, top),
        "by_owner": counter_rows(&stats.by_owner, top),
        "by_kind": counter_rows(&stats.by_kind, top),
        "by_template": counter_rows(&stats.by_template, top),
        "by_review_status": counter_rows(&stats.by_review_status, top),
        "by_scope": counter_rows(&stats.by_scope, top),
    })
}

fn render_summary_markdown(
    title: &str,
    stats: &SummaryStats,
    lensmaps: &[String],
    filters: &Map<String, Value>,
    top: usize,
) -> String {
    let mut lines = vec![
        format!("# {}", title),
        String::new(),
        format!("- LensMaps: {}", lensmaps.len()),
        format!("- Entries: {}", stats.entry_count),
        format!("- Files with notes: {}", stats.files_with_notes),
        format!("- Stale entries: {}", stats.stale_entries),
        String::new(),
    ];

    if !filters.is_empty() {
        lines.push("## Filters".to_string());
        for (key, value) in filters {
            if value.is_null() {
                continue;
            }
            lines.push(format!("- `{}`: {}", key, value));
        }
        lines.push(String::new());
    }

    render_counter_section(&mut lines, "Files", &stats.by_file, top);
    render_counter_section(&mut lines, "Directories", &stats.by_directory, top);
    render_counter_section(&mut lines, "Owners", &stats.by_owner, top);
    render_counter_section(&mut lines, "Kinds", &stats.by_kind, top);
    render_counter_section(&mut lines, "Templates", &stats.by_template, top);
    render_counter_section(&mut lines, "Review Status", &stats.by_review_status, top);
    render_counter_section(&mut lines, "Scopes", &stats.by_scope, top);

    format!("{}\n", lines.join("\n"))
}

fn render_policy_markdown(payload: &Value) -> String {
    let lensmaps = payload
        .get("lensmaps")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let policy = payload.get("policy").cloned().unwrap_or_else(|| json!({}));
    let policy_sources = payload
        .get("policy_sources")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let production_policy = payload
        .get("production_policy")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let findings = payload
        .get("findings")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let validate = payload
        .get("validate")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let stats = payload.get("stats").cloned().unwrap_or_else(|| json!({}));

    let mut lines = vec![
        format!("# {}", tr("LensMap Policy Check", "LensMap 策略检查")),
        String::new(),
        format!("- {}: {}", tr("LensMaps", "LensMap 数量"), lensmaps.len()),
        format!(
            "- {}: {}",
            tr("Aggregation", "聚合策略"),
            payload
                .get("aggregation")
                .and_then(Value::as_str)
                .unwrap_or("strictest_union")
        ),
        format!(
            "- {}: {}",
            tr("Policy errors", "策略错误"),
            findings
                .get("summary")
                .and_then(|value| value.get("error_count"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        ),
        format!(
            "- {}: {}",
            tr("Policy warnings", "策略警告"),
            findings
                .get("summary")
                .and_then(|value| value.get("warning_count"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        ),
        format!(
            "- {}: {}",
            tr("Structural errors", "结构错误"),
            validate
                .get("summary")
                .and_then(|value| value.get("error_count"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        ),
        String::new(),
        "## Policy".to_string(),
        format!(
            "- require_owner: {}",
            policy
                .get("require_owner")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "- require_author: {}",
            policy
                .get("require_author")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "- require_template: {}",
            policy
                .get("require_template")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "- require_review_status: {}",
            policy
                .get("require_review_status")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "- stale_after_days: {}",
            policy
                .get("stale_after_days")
                .and_then(Value::as_i64)
                .unwrap_or(0)
        ),
    ];

    let required_patterns = policy
        .get("required_patterns")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if required_patterns.is_empty() {
        lines.push("- required_patterns: []".to_string());
    } else {
        lines.push("- required_patterns:".to_string());
        for pattern in required_patterns {
            if let Some(pattern) = pattern.as_str() {
                lines.push(format!("  - `{}`", pattern));
            }
        }
    }
    lines.push(String::new());

    lines.push("## Production".to_string());
    lines.push(format!(
        "- strip_anchors: {}",
        production_policy
            .get("stripAnchors")
            .or_else(|| production_policy.get("strip_anchors"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    ));
    lines.push(format!(
        "- strip_refs: {}",
        production_policy
            .get("stripRefs")
            .or_else(|| production_policy.get("strip_refs"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    ));
    lines.push(format!(
        "- strip_on_package: {}",
        production_policy
            .get("stripOnPackage")
            .or_else(|| production_policy.get("strip_on_package"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    ));
    let production_markers = payload
        .get("production_marker_files")
        .and_then(Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0);
    lines.push(format!("- marker_files: {}", production_markers));
    lines.push(String::new());

    lines.push("## Policy Sources".to_string());
    if policy_sources.is_empty() {
        lines.push("- none".to_string());
    } else {
        for source in policy_sources {
            let lensmap = source
                .get("lensmap")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let source_policy = source.get("policy").cloned().unwrap_or_else(|| json!({}));
            let stale = source_policy
                .get("stale_after_days")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            lines.push(format!(
                "- `{}` owner={} author={} template={} review={} stale={}",
                lensmap,
                source_policy
                    .get("require_owner")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                source_policy
                    .get("require_author")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                source_policy
                    .get("require_template")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                source_policy
                    .get("require_review_status")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                stale,
            ));
        }
    }
    lines.push(String::new());

    let render_findings = |lines: &mut Vec<String>, title: &str, items: &[Value]| {
        lines.push(format!("## {}", title));
        if items.is_empty() {
            lines.push("- none".to_string());
            lines.push(String::new());
            return;
        }
        for item in items {
            let code = item
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let ref_id = item.get("ref").and_then(Value::as_str).unwrap_or("?");
            let lensmap = item
                .get("lensmap")
                .and_then(Value::as_str)
                .map(|value| format!(" [{}]", value))
                .unwrap_or_default();
            let message = item
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no message");
            lines.push(format!("- `{}` `{}`{} {}", code, ref_id, lensmap, message));
        }
        lines.push(String::new());
    };

    let policy_errors = findings
        .get("errors")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let policy_warnings = findings
        .get("warnings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    render_findings(&mut lines, &tr("Policy Errors", "策略错误"), &policy_errors);
    render_findings(
        &mut lines,
        &tr("Policy Warnings", "策略警告"),
        &policy_warnings,
    );

    let structural_errors = validate
        .get("errors")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let structural_warnings = validate
        .get("warnings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    lines.push(format!(
        "## {}",
        tr("Structural Validate Findings", "结构校验结果")
    ));
    lines.push(format!(
        "- {}: {}",
        tr("Errors", "错误"),
        structural_errors.len()
    ));
    lines.push(format!(
        "- {}: {}",
        tr("Warnings", "警告"),
        structural_warnings.len()
    ));
    lines.push(String::new());

    lines.push("## Stats".to_string());
    lines.push(format!(
        "- lensmap_count: {}",
        stats
            .get("lensmap_count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    ));
    lines.push(format!(
        "- cover_roots: {}",
        stats
            .get("cover_roots")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    ));
    lines.push(format!(
        "- anchors: {}",
        stats.get("anchors").and_then(Value::as_u64).unwrap_or(0)
    ));
    lines.push(format!(
        "- entries: {}",
        stats.get("entries").and_then(Value::as_u64).unwrap_or(0)
    ));
    lines.push(String::new());

    format!("{}\n", lines.join("\n"))
}

fn default_pr_report_output_path(root: &Path) -> PathBuf {
    root.join("lensmap-pr-report.md")
}

fn render_pr_report_markdown(report: PrReportRender<'_>) -> String {
    let mut lines = vec![
        format!("# {}", tr("LensMap PR Report", "LensMap PR 报告")),
        String::new(),
        format!(
            "- {}: {}",
            tr("Change source", "变更来源"),
            report.source_kind
        ),
        format!("- {}: {}", tr("Base", "基线"), report.base.unwrap_or("-")),
        format!("- {}: {}", tr("Head", "目标"), report.head.unwrap_or("-")),
        format!(
            "- {}: {}",
            tr("Changed files", "变更文件"),
            report.changed_files.len()
        ),
        format!(
            "- {}: {}",
            tr("Files with notes", "带注释文件"),
            report.grouped.len()
        ),
        format!(
            "- {}: {}",
            tr("Stale notes touching change", "涉及变更的过期注释"),
            report.stale_refs.len()
        ),
        format!(
            "- {}: {}",
            tr("Unreviewed notes touching change", "涉及变更的未评审注释"),
            report.unreviewed_refs.len()
        ),
        format!(
            "- {}: {}",
            tr("Changed files without notes", "无注释的变更文件"),
            report.uncovered_files.len()
        ),
        String::new(),
    ];

    lines.push(format!(
        "## {}",
        tr("Changed Files With Notes", "带注释的变更文件")
    ));
    if report.grouped.is_empty() {
        lines.push(format!("- {}", tr("none", "无")));
        lines.push(String::new());
    } else {
        for (file, entries) in report.grouped {
            lines.push(format!("### `{}`", file));
            for entry in entries {
                let summary = entry
                    .title
                    .clone()
                    .or_else(|| entry.text.clone())
                    .unwrap_or_else(|| tr("Untitled note", "未命名注释"));
                let mut detail = vec![
                    format!("`{}`", entry.ref_id),
                    entry.kind.clone().unwrap_or_else(|| "comment".to_string()),
                    summary.replace('\n', " "),
                ];
                if let Some(owner) = entry
                    .owner
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                {
                    detail.push(format!("owner={}", owner));
                }
                if let Some(status) = entry
                    .review_status
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                {
                    detail.push(format!("review={}", status));
                }
                lines.push(format!("- {}", detail.join(" | ")));
            }
            lines.push(String::new());
        }
    }

    lines.push(format!(
        "## {}",
        tr("Changed Files Without Notes", "无注释的变更文件")
    ));
    if report.uncovered_files.is_empty() {
        lines.push(format!("- {}", tr("none", "无")));
    } else {
        for file in report.uncovered_files {
            lines.push(format!("- `{}`", file));
        }
    }
    lines.push(String::new());

    format!("{}\n", lines.join("\n"))
}

fn cmd_policy_init(root: &Path, args: &ParsedArgs) {
    let lensmap_path = resolve_lensmap_path(root, args, None);
    if !lensmap_path.exists() {
        emit(
            lensmap_missing_payload(root, "policy_init", &lensmap_path, args),
            1,
        );
    }
    if !is_within_root(root, &lensmap_path) {
        emit(
            json!({"ok": false, "error": "security_output_outside_root"}),
            1,
        );
    }

    let mut doc = load_doc(&lensmap_path, "group");
    let mut policy = policy_for_doc(&doc);
    let mut production = production_for_doc(&doc);
    policy.require_owner = bool_from_flag(args, "require-owner", policy.require_owner);
    policy.require_author = bool_from_flag(args, "require-author", policy.require_author);
    policy.require_template = bool_from_flag(args, "require-template", policy.require_template);
    policy.require_review_status =
        bool_from_flag(args, "require-review-status", policy.require_review_status);
    if let Some(days) = parse_i64_flag(args, "stale-after-days") {
        policy.stale_after_days = days.max(0);
    }
    if let Some(raw) = args
        .get("required-patterns")
        .or_else(|| args.get("required-pattern"))
    {
        policy.required_patterns = split_csv(Some(raw));
    }
    production.strip_anchors =
        bool_from_flag(args, "production-strip-anchors", production.strip_anchors);
    production.strip_refs = bool_from_flag(args, "production-strip-refs", production.strip_refs);
    production.strip_on_package = bool_from_flag(
        args,
        "production-strip-on-package",
        production.strip_on_package,
    );
    if let Some(raw) = args
        .get("production-exclude-patterns")
        .or_else(|| args.get("production-exclude-pattern"))
    {
        production.exclude_patterns = split_csv(Some(raw));
    }
    store_policy(&mut doc.metadata, &policy);
    store_production_policy(&mut doc.metadata, &production);
    save_doc(&lensmap_path, doc.clone());

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "policy_init",
        "lensmap": normalize_relative(root, &lensmap_path),
        "policy": policy,
        "production": production,
        "ts": now_iso(),
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_policy_check(root: &Path, args: &ParsedArgs) {
    let docs = resolve_policy_lensmap_docs(root, args);
    let lensmaps = docs
        .iter()
        .map(|loaded| loaded.lensmap.clone())
        .collect::<Vec<_>>();
    let mut structural_errors = vec![];
    let mut structural_warnings = vec![];
    let mut dirty_lensmaps = vec![];
    let mut total_covers = BTreeMap::new();
    let mut total_anchors = 0usize;
    let mut total_entries = 0usize;

    for loaded in &docs {
        total_anchors += loaded.doc.anchors.len();
        total_entries += loaded.doc.entries.len();
        for cover in &loaded.doc.covers {
            total_covers.insert(cover.clone(), true);
        }
        let findings = validate_doc(root, &loaded.path, &loaded.doc);
        if findings.lensmap_dirty {
            dirty_lensmaps.push(loaded.lensmap.clone());
        }
        structural_errors.extend(findings.errors.into_iter().map(|finding| {
            json!({
                "lensmap": loaded.lensmap,
                "finding": finding,
            })
        }));
        structural_warnings.extend(findings.warnings.into_iter().map(|finding| {
            json!({
                "lensmap": loaded.lensmap,
                "finding": finding,
            })
        }));
    }

    let (policy, mut errors, mut warnings, policy_sources) =
        collect_aggregated_policy_findings(root, &docs);
    let production_policy = aggregate_production_settings(
        docs.iter()
            .map(|loaded| production_for_doc(&loaded.doc))
            .collect::<Vec<_>>()
            .iter(),
    );
    let production_marker_hits = collect_production_marker_hits(root, &docs, &production_policy);
    let production_enforced = args.has("production") || is_release_ref_context();
    if !production_marker_hits.is_empty() {
        let preview = production_marker_hits
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let finding = PolicyFinding {
            level: if production_enforced {
                "error".to_string()
            } else {
                "warning".to_string()
            },
            ref_id: "policy:production".to_string(),
            lensmap: None,
            field: "production".to_string(),
            code: if production_enforced {
                "production_strip_required".to_string()
            } else {
                "production_strip_pending".to_string()
            },
            message: format!(
                "Production strip policy found remaining markers in source files: {}{}",
                preview,
                if production_marker_hits.len() > 3 {
                    ", ..."
                } else {
                    ""
                }
            ),
        };
        if production_enforced {
            errors.push(finding);
        } else {
            warnings.push(finding);
        }
    }
    let output_path = args.get("out").map(|out| resolve_from_root(root, out));
    if let Some(output_path) = output_path.as_ref() {
        if !is_within_root(root, output_path) {
            emit(
                json!({"ok": false, "error": "security_output_outside_root"}),
                1,
            );
        }
    }
    let fail_on_warnings = args.has("fail-on-warnings");
    let report_only = args.has("report-only");
    let ok = structural_errors.is_empty()
        && errors.is_empty()
        && (!fail_on_warnings || warnings.is_empty());
    let mut out = json!({
        "ok": ok,
        "type": "lensmap",
        "action": "policy_check",
        "lensmap": if docs.len() == 1 { Some(lensmaps[0].clone()) } else { None::<String> },
        "lensmaps": lensmaps,
        "aggregation": "strictest_union",
        "policy": policy,
        "production_policy": production_policy,
        "production_enforced": production_enforced,
        "production_marker_files": production_marker_hits,
        "policy_sources": policy_sources,
        "findings": {
            "errors": errors,
            "warnings": warnings,
            "summary": {
                "error_count": errors.len(),
                "warning_count": warnings.len(),
            }
        },
        "validate": {
            "errors": structural_errors,
            "warnings": structural_warnings,
            "dirty_lensmaps": dirty_lensmaps,
            "summary": {
                "error_count": structural_errors.len(),
                "warning_count": structural_warnings.len(),
            }
        },
        "stats": {
            "lensmap_count": docs.len(),
            "cover_roots": total_covers.len(),
            "anchors": total_anchors,
            "entries": total_entries,
        },
        "report_only": report_only,
        "ts": now_iso(),
    });
    if let Some(output_path) = output_path.as_ref() {
        ensure_dir(output_path);
        let markdown = render_policy_markdown(&out);
        let _ = fs::write(output_path, markdown);
        if let Some(obj) = out.as_object_mut() {
            obj.insert(
                "output".to_string(),
                Value::String(normalize_relative(root, output_path)),
            );
        }
    }
    let profile = parse_redaction_profile(args, "redaction-profile");
    let output_paths: Vec<PathBuf> = output_path
        .as_ref()
        .map(|p| vec![p.clone()])
        .unwrap_or_default();
    emit_with_envelope(
        root,
        "lensmap",
        "policy_check",
        args,
        out,
        profile,
        &policy,
        &lensmaps,
        &output_paths,
        if ok || report_only { 0 } else { 1 },
        vec![],
    );
}

fn cmd_summary(root: &Path, args: &ParsedArgs) {
    let lensmaps = resolve_search_lensmap_paths(root, args);
    if lensmaps.is_empty() {
        emit(json!({"ok": false, "error": "no_lensmap_files_found"}), 1);
    }
    let loaded_docs = load_repo_lensmap_docs(root, &lensmaps);
    let policy_sources = loaded_docs
        .iter()
        .map(|loaded| (loaded.lensmap.clone(), policy_for_doc(&loaded.doc)))
        .collect::<Vec<_>>();
    let aggregated_policy =
        aggregate_policy_settings(policy_sources.iter().map(|(_, policy)| policy));

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
    let owner_filter = args
        .get("owner")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let template_filter = args
        .get("template")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let review_filter = args
        .get("review-status")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let scope_filter = args
        .get("scope")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let tag_filter = args
        .get("tag")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let search_filters = search_filters_from_args(args);
    let top = args
        .get("top")
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(10)
        .clamp(1, 100);

    let explicit_range = args.get("base").is_some() || args.get("head").is_some();
    let base = args.get("base");
    let head = args.get("head").or(Some("HEAD"));
    let changed_files = if explicit_range {
        git_changed_files(root, base.unwrap_or("HEAD~1"), head.unwrap_or("HEAD")).unwrap_or_else(
            || {
                emit(
                    json!({
                        "ok": false,
                        "type": "lensmap",
                        "action": "summary",
                        "error": "git_range_unavailable",
                        "base": base,
                        "head": head,
                    }),
                    1,
                );
            },
        )
    } else {
        vec![]
    };
    let changed_filter = if explicit_range {
        Some(changed_files.iter().cloned().collect::<HashSet<_>>())
    } else {
        None
    };

    let entries = collect_repo_search_entries(root, &lensmaps)
        .into_iter()
        .filter(|entry| search_entry_matches_filters(entry, search_filters))
        .filter(|entry| {
            changed_filter
                .as_ref()
                .map(|changed| changed.contains(&entry.file))
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();

    let stats = summary_stats(&entries, aggregated_policy.stale_after_days);
    let summary = summary_stats_to_value(&stats, top);

    let mut filters = Map::new();
    filters.insert("file".to_string(), json!(file_filter));
    filters.insert("symbol".to_string(), json!(symbol_filter));
    filters.insert("kind".to_string(), json!(kind_filter));
    filters.insert("owner".to_string(), json!(owner_filter));
    filters.insert("template".to_string(), json!(template_filter));
    filters.insert("review_status".to_string(), json!(review_filter));
    filters.insert("scope".to_string(), json!(scope_filter));
    filters.insert("tag".to_string(), json!(tag_filter));
    if explicit_range {
        filters.insert("base".to_string(), json!(base));
        filters.insert("head".to_string(), json!(head));
    }

    let output_path = args.get("out").map(|out| resolve_from_root(root, out));
    if let Some(output_path) = output_path.as_ref() {
        if !is_within_root(root, output_path) {
            emit(
                json!({"ok": false, "error": "security_output_outside_root"}),
                1,
            );
        }
        ensure_dir(output_path);
        let markdown = render_summary_markdown(
            &tr("LensMap Summary", "LensMap 汇总"),
            &stats,
            &lensmaps,
            &filters,
            top,
        );
        let _ = fs::write(output_path, markdown);
    }

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "summary",
        "lensmaps": lensmaps,
        "aggregation": "strictest_union",
        "policy": aggregated_policy,
        "policy_sources": policy_sources
            .iter()
            .map(|(lensmap, policy)| json!({"lensmap": lensmap, "policy": policy}))
            .collect::<Vec<_>>(),
        "changed_files": changed_files,
        "filters": filters,
        "summary": summary,
        "output": output_path.as_ref().map(|path| normalize_relative(root, path)),
        "ts": now_iso(),
    });
    append_history(root, &out);
    emit(out, 0);
}

fn cmd_pr_report(root: &Path, args: &ParsedArgs) {
    let lensmaps = resolve_search_lensmap_paths(root, args);
    if lensmaps.is_empty() {
        emit(json!({"ok": false, "error": "no_lensmap_files_found"}), 1);
    }
    let loaded_docs = load_repo_lensmap_docs(root, &lensmaps);
    let policy_sources = loaded_docs
        .iter()
        .map(|loaded| (loaded.lensmap.clone(), policy_for_doc(&loaded.doc)))
        .collect::<Vec<_>>();
    let aggregated_policy =
        aggregate_policy_settings(policy_sources.iter().map(|(_, policy)| policy));
    let production_policy = aggregate_production_settings(
        loaded_docs
            .iter()
            .map(|loaded| production_for_doc(&loaded.doc))
            .collect::<Vec<_>>()
            .iter(),
    );
    let production_enforced = args.has("production");
    let production_marker_hits = if production_enforced {
        collect_production_marker_hits(root, &loaded_docs, &production_policy)
    } else {
        vec![]
    };

    let explicit_range = args.get("base").is_some() || args.get("head").is_some();
    let base = args.get("base").unwrap_or("HEAD~1");
    let head = args.get("head").unwrap_or("HEAD");
    let (source_kind, changed_files) = if explicit_range {
        (
            "git_range".to_string(),
            git_changed_files(root, base, head).unwrap_or_else(|| {
                emit(
                    json!({
                        "ok": false,
                        "type": "lensmap",
                        "action": "pr_report",
                        "error": "git_range_unavailable",
                        "base": base,
                        "head": head,
                    }),
                    1,
                );
            }),
        )
    } else {
        let ranged = git_changed_files(root, base, head).unwrap_or_default();
        if ranged.is_empty() {
            ("worktree".to_string(), git_worktree_changed_files(root))
        } else {
            ("git_range".to_string(), ranged)
        }
    };

    let candidate_changed_files = changed_files
        .into_iter()
        .filter(|file| SUPPORTED_EXTS.contains(&ext_of(Path::new(file)).as_str()))
        .collect::<Vec<_>>();
    let changed_set = candidate_changed_files
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let lensmap_entries = collect_repo_search_entries(root, &lensmaps)
        .into_iter()
        .filter(|entry| changed_set.contains(&entry.file))
        .collect::<Vec<_>>();

    let mut grouped = BTreeMap::<String, Vec<SearchEntryRecord>>::new();
    let mut stale_refs = vec![];
    let mut unreviewed_refs = vec![];
    for entry in lensmap_entries {
        let review_status = entry
            .review_status
            .clone()
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        let entry_record = EntryRecord {
            ref_id: entry.ref_id.clone(),
            file: entry.file.clone(),
            anchor_id: entry.anchor_id.clone(),
            kind: entry.kind.clone(),
            text: entry.text.clone(),
            title: entry.title.clone(),
            owner: entry.owner.clone(),
            author: entry.author.clone(),
            scope: entry.scope.clone(),
            template: entry.template.clone(),
            review_status: entry.review_status.clone(),
            review_due_at: entry.review_due_at.clone(),
            updated_at: entry.updated_at.clone(),
            tags: entry.tags.clone(),
            created_at: None,
            source: None,
        };
        if aggregated_policy.stale_after_days > 0
            && entry_is_stale(&entry_record, aggregated_policy.stale_after_days)
        {
            stale_refs.push(entry.ref_id.clone());
        }
        if review_status.is_empty()
            || ["draft", "todo", "pending"].contains(&review_status.as_str())
        {
            unreviewed_refs.push(entry.ref_id.clone());
        }
        grouped.entry(entry.file.clone()).or_default().push(entry);
    }
    for entries in grouped.values_mut() {
        entries.sort_by(|left, right| {
            left.start_line
                .unwrap_or(0)
                .cmp(&right.start_line.unwrap_or(0))
                .then_with(|| left.ref_id.cmp(&right.ref_id))
        });
    }

    let uncovered_files = candidate_changed_files
        .iter()
        .filter(|file| !grouped.contains_key(*file))
        .cloned()
        .collect::<Vec<_>>();
    let strict = args.has("strict");
    let strict_failures = {
        let mut failures = vec![];
        if !uncovered_files.is_empty() {
            failures.push("changed_files_without_notes".to_string());
        }
        if !stale_refs.is_empty() {
            failures.push("stale_notes".to_string());
        }
        if !unreviewed_refs.is_empty() {
            failures.push("unreviewed_notes".to_string());
        }
        if production_enforced && !production_marker_hits.is_empty() {
            failures.push("production_strip_required".to_string());
        }
        failures
    };

    let output_path = if let Some(out) = args.get("out") {
        resolve_from_root(root, out)
    } else {
        default_pr_report_output_path(root)
    };
    if !is_within_root(root, &output_path) {
        emit(
            json!({"ok": false, "error": "security_output_outside_root"}),
            1,
        );
    }
    ensure_dir(&output_path);
    let markdown = render_pr_report_markdown(PrReportRender {
        base: Some(base),
        head: Some(head),
        source_kind: &source_kind,
        changed_files: &candidate_changed_files,
        grouped: &grouped,
        stale_refs: &stale_refs,
        unreviewed_refs: &unreviewed_refs,
        uncovered_files: &uncovered_files,
    });
    let _ = fs::write(&output_path, markdown);

    let ok = (!strict || strict_failures.is_empty())
        && (!production_enforced || production_marker_hits.is_empty());
    let out = json!({
        "ok": ok,
        "type": "lensmap",
        "action": "pr_report",
        "lensmaps": lensmaps,
        "aggregation": "strictest_union",
        "policy": aggregated_policy,
        "production_policy": production_policy,
        "production_enforced": production_enforced,
        "production_marker_files": production_marker_hits,
        "policy_sources": policy_sources
            .iter()
            .map(|(lensmap, policy)| json!({"lensmap": lensmap, "policy": policy}))
            .collect::<Vec<_>>(),
        "source_kind": source_kind,
        "base": base,
        "head": head,
        "changed_files": candidate_changed_files,
        "files_with_notes": grouped.len(),
        "entry_count": grouped.values().map(|entries| entries.len()).sum::<usize>(),
        "stale_refs": stale_refs,
        "unreviewed_refs": unreviewed_refs,
        "uncovered_files": uncovered_files,
        "strict": strict,
        "strict_failures": strict_failures,
        "output": normalize_relative(root, &output_path),
        "ts": now_iso(),
    });
    let profile = parse_redaction_profile(args, "redaction-profile");
    emit_with_envelope(
        root,
        "lensmap",
        "pr_report",
        args,
        out,
        profile,
        &aggregated_policy,
        &lensmaps,
        &[output_path],
        if ok { 0 } else { 1 },
        strict_failures,
    );
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

fn default_autobot_output_path(root: &Path) -> PathBuf {
    root.join("local/state/ops/lensmap/autobot.json")
}

fn default_metric_output_path(root: &Path) -> PathBuf {
    root.join("local/state/ops/lensmap/metrics.json")
}

fn default_scorecard_output_path(root: &Path) -> PathBuf {
    root.join("local/state/ops/lensmap/scorecard.md")
}

fn default_policy_output_path(root: &Path) -> PathBuf {
    root.join("local/state/ops/lensmap/policy.md")
}

fn default_metric_history_path(root: &Path) -> PathBuf {
    root.join("local/state/ops/lensmap/metric-history.jsonl")
}

fn render_filters_from_args<'a>(args: &'a ParsedArgs) -> RenderFilters<'a> {
    RenderFilters {
        file: args.get("file"),
        symbol: args.get("symbol"),
        ref_id: args.get("ref"),
        kind: args.get("kind"),
        owner: args.get("owner"),
        template: args.get("template"),
        review_status: args.get("review-status"),
        scope: args.get("scope"),
        tag: args.get("tag"),
    }
}

fn search_filters_from_args<'a>(args: &'a ParsedArgs) -> SearchFilters<'a> {
    SearchFilters {
        file: args
            .get("file")
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        symbol: args
            .get("symbol")
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        kind: args
            .get("kind")
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        owner: args
            .get("owner")
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        template: args
            .get("template")
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        review_status: args
            .get("review-status")
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        scope: args
            .get("scope")
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        tag: args
            .get("tag")
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    }
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
    let owner_filter = filters.owner.map(str::trim).filter(|v| !v.is_empty());
    let template_filter = filters.template.map(str::trim).filter(|v| !v.is_empty());
    let review_filter = filters
        .review_status
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let scope_filter = filters.scope.map(str::trim).filter(|v| !v.is_empty());
    let tag_filter = filters.tag.map(str::trim).filter(|v| !v.is_empty());

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
    lines.push(format!(
        "- {} {}",
        tr("Boilerplate scope:", "样板范围："),
        doc.metadata
            .get("boilerplate_scope")
            .and_then(Value::as_str)
            .unwrap_or("knowledge")
    ));
    lines.push(format!(
        "- {} `{}`",
        tr("Artifact layers:", "产物层："),
        artifact_layers_for_doc(doc).join(", ")
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
    if let Some(owner) = owner_filter {
        lines.push(format!(
            "- {} `{}`",
            tr("Owner filter:", "负责人过滤："),
            owner
        ));
    }
    if let Some(template) = template_filter {
        lines.push(format!(
            "- {} `{}`",
            tr("Template filter:", "模板过滤："),
            template
        ));
    }
    if let Some(review) = review_filter {
        lines.push(format!(
            "- {} `{}`",
            tr("Review filter:", "审核状态过滤："),
            review
        ));
    }
    if let Some(scope) = scope_filter {
        lines.push(format!(
            "- {} `{}`",
            tr("Scope filter:", "范围过滤："),
            scope
        ));
    }
    if let Some(tag) = tag_filter {
        lines.push(format!("- {} `{}`", tr("Tag filter:", "标签过滤："), tag));
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
            if let Some(owner) = owner_filter {
                if entry.owner.as_deref() != Some(owner) {
                    continue;
                }
            }
            if let Some(template) = template_filter {
                if entry.template.as_deref() != Some(template) {
                    continue;
                }
            }
            if let Some(review) = review_filter {
                if entry.review_status.as_deref() != Some(review) {
                    continue;
                }
            }
            if let Some(scope) = scope_filter {
                if entry.scope.as_deref() != Some(scope) {
                    continue;
                }
            }
            if let Some(tag) = tag_filter {
                if !entry.tags.iter().any(|candidate| candidate == tag) {
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
                entry.title.clone().unwrap_or_else(|| entry
                    .text
                    .clone()
                    .unwrap_or_default()
                    .replace('\n', " ")
                    .trim()
                    .to_string())
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
            let mut metadata_parts = vec![];
            if let Some(owner) = &entry.owner {
                metadata_parts.push(format!("owner=`{}`", owner));
            }
            if let Some(author) = &entry.author {
                metadata_parts.push(format!("author=`{}`", author));
            }
            if let Some(scope) = &entry.scope {
                metadata_parts.push(format!("scope=`{}`", scope));
            }
            if let Some(template) = &entry.template {
                metadata_parts.push(format!("template=`{}`", template));
            }
            if let Some(review_status) = &entry.review_status {
                metadata_parts.push(format!("review=`{}`", review_status));
            }
            if let Some(review_due_at) = &entry.review_due_at {
                metadata_parts.push(format!("review_due=`{}`", review_due_at));
            }
            if let Some(updated_at) = &entry.updated_at {
                metadata_parts.push(format!("updated=`{}`", updated_at));
            }
            if !entry.tags.is_empty() {
                metadata_parts.push(format!("tags=`{}`", entry.tags.join(",")));
            }
            if !metadata_parts.is_empty() {
                lines.push(format!("  {}", metadata_parts.join(" ")));
            }
            if let Some(text) = &entry.text {
                lines.push(format!("  {}", text.replace('\n', " ").trim()));
            }

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
    if !is_within_root(root, &out_path) {
        emit(
            json!({"ok": false, "error": "security_output_outside_root"}),
            1,
        );
    }
    let (lines, files_rendered, entries_rendered) = build_render_lines(
        root,
        &lensmap_path,
        &doc,
        render_filters_from_args(args),
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
        "filters": {
            "file": args.get("file"),
            "symbol": args.get("symbol"),
            "ref": args.get("ref"),
            "kind": args.get("kind"),
            "owner": args.get("owner"),
            "template": args.get("template"),
            "review_status": args.get("review-status"),
            "scope": args.get("scope"),
            "tag": args.get("tag"),
        },
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

fn annotation_has_candidate_signal(text: &str) -> bool {
    let lower = text.to_lowercase();
    if lower.len() < 12 {
        return false;
    }
    [
        "todo", "fixme", "warning", "risk", "hack", "issue", "review", "TODO", "FIXME", "NOTE",
        "NOTE:",
    ]
    .iter()
    .any(|term| lower.contains(&term.to_lowercase()))
}

fn autobot_confidence_for_block(text: &str, kind: &str, profile: AutobotProfile) -> f64 {
    let lower = text.to_lowercase();
    let mut score: f64 = 0.36;
    if lower.contains("todo") || lower.contains("fixme") {
        score += 0.30;
    }
    if lower.contains("warning") || lower.contains("risk") {
        score += 0.16;
    }
    if lower.contains("note:") || lower.contains("hack") {
        score += 0.12;
    }
    if kind == "doc" {
        score += 0.05;
    }
    if text.len() > 180 {
        score += 0.05;
    }
    score = score.clamp(0.0, 1.0);
    match profile {
        AutobotProfile::Conservative => (score + 0.03).clamp(0.0, 1.0),
        AutobotProfile::Standard => score,
        AutobotProfile::Aggressive => (score - 0.05).clamp(0.0, 1.0),
    }
}

fn synthetic_ref_id(file: &str, start: usize, end: usize) -> String {
    let safe = file
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect::<String>();
    let prefix = if safe.is_empty() { "file" } else { &safe };
    format!("AUTO-{}-{}-{}", prefix, start + 1, end + 1)
}

fn collect_autobot_proposals(
    root: &Path,
    source_files: &[String],
    _doc: &LensMapDoc,
    existing_keys: &mut HashSet<String>,
    profile: AutobotProfile,
) -> (Vec<ImportProposal>, Vec<Value>, usize, usize, usize) {
    let mut proposals = vec![];
    let mut conflicts = vec![];
    let mut files_scanned = 0usize;
    let mut filtered = 0usize;
    let mut duplicates = 0usize;
    let mut seen = HashSet::new();

    for rel in source_files {
        let abs = resolve_from_root(root, rel);
        if !is_within_root(root, &abs) || !abs.exists() || abs.is_dir() {
            continue;
        }
        let style = comment_style_for(&abs);
        let lines = split_lines(&fs::read_to_string(&abs).unwrap_or_default());
        if lines.is_empty() {
            continue;
        }
        let anchor_nodes = collect_anchor_nodes(&lines, Some(style.line));
        let anchor_lookup = materialize_anchors_for_file(root, &abs, &lines)
            .into_iter()
            .map(|anchor| (anchor.id.to_uppercase(), anchor))
            .collect::<HashMap<_, _>>();
        let blocks = collect_comment_blocks(&lines, &abs);
        files_scanned = files_scanned.saturating_add(1);

        for block in blocks {
            let candidate = block.text.trim().to_string();
            if !annotation_has_candidate_signal(&candidate) {
                filtered = filtered.saturating_add(1);
                continue;
            }
            let candidate_reason = if candidate.to_lowercase().contains("todo") {
                "explicit_todo_marker"
            } else if candidate.to_lowercase().contains("fixme") {
                "explicit_fixme_marker"
            } else {
                "doc_annotation_signal"
            };

            let mut anchor_id = None::<String>;
            let mut ref_id = String::new();
            let maybe_latest = find_latest_anchor_for_line(&anchor_nodes, block.start);
            if let Some((raw_anchor_id, _)) = maybe_latest {
                let anchor_key = raw_anchor_id.to_uppercase();
                if let Some(anchor) = anchor_lookup.get(&anchor_key) {
                    let offsets = ref_offsets_for_block(anchor, None, block.start, block.end);
                    if let Some((start, end)) = offsets {
                        anchor_id = Some(anchor.id.clone());
                        ref_id = if start == end {
                            format!("{}-{}", anchor.id.to_uppercase(), start)
                        } else {
                            format!("{}-{}-{}", anchor.id.to_uppercase(), start, end)
                        };
                    }
                }
            }
            if ref_id.is_empty() {
                ref_id = synthetic_ref_id(rel, block.start, block.end);
            }

            let key = format!("{}::{}", rel, ref_id.to_uppercase());
            if existing_keys.contains(&key) {
                duplicates = duplicates.saturating_add(1);
                conflicts.push(json!({
                    "file": rel,
                    "ref": ref_id,
                    "reason": "already_implemented",
                    "source": "autobot",
                }));
                continue;
            }
            if !seen.insert(key.clone()) {
                duplicates = duplicates.saturating_add(1);
                conflicts.push(json!({
                    "file": rel,
                    "ref": ref_id,
                    "reason": "duplicate_candidate_in_run",
                    "source": "autobot",
                }));
                continue;
            }
            existing_keys.insert(key.clone());

            let confidence = autobot_confidence_for_block(&candidate, &block.kind, profile);
            proposals.push(ImportProposal {
                confidence,
                reason: candidate_reason.to_string(),
                entry: EntryRecord {
                    ref_id,
                    file: rel.clone(),
                    anchor_id,
                    kind: Some(if block.kind == "doc" {
                        "doc".to_string()
                    } else {
                        "comment".to_string()
                    }),
                    text: Some(candidate),
                    title: None,
                    owner: None,
                    author: None,
                    scope: None,
                    template: Some("knowledge-boilerplate".to_string()),
                    review_status: None,
                    review_due_at: None,
                    updated_at: Some(now_iso()),
                    tags: vec!["autobot".to_string(), profile.label().to_string()],
                    created_at: None,
                    source: Some("autobot".to_string()),
                },
            });
        }
    }

    (proposals, conflicts, files_scanned, filtered, duplicates)
}

fn cmd_metrics(root: &Path, args: &ParsedArgs) {
    let lensmaps = resolve_search_lensmap_paths(root, args);
    if lensmaps.is_empty() {
        emit(json!({"ok": false, "error": "no_lensmap_files_found"}), 1);
    }

    let period = args.get("period").unwrap_or("run").trim();
    let top = args
        .get("top")
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(10)
        .clamp(1, 100);
    let generated_at = now_iso();
    let snapshot = build_metric_snapshot(root, &lensmaps, top, period, &generated_at);

    let out_path = args
        .get("out")
        .map(|path| resolve_from_root(root, path))
        .unwrap_or_else(|| default_metric_output_path(root));
    if !is_within_root(root, &out_path) {
        emit(
            json!({"ok": false, "error": "security_output_outside_root"}),
            1,
        );
    }
    ensure_dir(&out_path);
    let _ = fs::write(
        &out_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&snapshot).unwrap_or_else(|_| "{}".to_string())
        ),
    );
    append_metric_history(root, period, &generated_at, &snapshot, &out_path);

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "metrics",
        "period": period,
        "top": top,
        "lensmaps": lensmaps,
        "output": normalize_relative(root, &out_path),
        "snapshot": snapshot,
        "ts": now_iso(),
    });
    let profile = parse_redaction_profile(args, "redaction-profile");
    emit_with_envelope(
        root,
        "lensmap",
        "metrics",
        args,
        out,
        profile,
        &default_policy_settings(),
        &lensmaps,
        &[out_path],
        0,
        vec![],
    );
}

fn cmd_scorecard(root: &Path, args: &ParsedArgs) {
    let lensmaps = resolve_search_lensmap_paths(root, args);
    if lensmaps.is_empty() {
        emit(json!({"ok": false, "error": "no_lensmap_files_found"}), 1);
    }

    let period = args.get("period").unwrap_or("run").trim();
    let top = args
        .get("top")
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(10)
        .clamp(1, 100);
    let generated_at = now_iso();
    let snapshot = build_metric_snapshot(root, &lensmaps, top, period, &generated_at);
    let out_path = args
        .get("out")
        .map(|path| resolve_from_root(root, path))
        .unwrap_or_else(|| default_scorecard_output_path(root));
    if !is_within_root(root, &out_path) {
        emit(
            json!({"ok": false, "error": "security_output_outside_root"}),
            1,
        );
    }
    ensure_dir(&out_path);
    let health = snapshot.get("health").cloned().unwrap_or_else(|| json!({}));
    let markdown = render_scorecard_markdown(
        &tr("LensMap Scorecard", "LensMap 评分卡"),
        &snapshot,
        &health,
        top,
    );
    let _ = fs::write(&out_path, markdown);
    append_metric_history(root, period, &generated_at, &snapshot, &out_path);

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "scorecard",
        "period": period,
        "top": top,
        "lensmaps": lensmaps,
        "output": normalize_relative(root, &out_path),
        "snapshot": snapshot,
        "ts": now_iso(),
    });
    let profile = parse_redaction_profile(args, "redaction-profile");
    emit_with_envelope(
        root,
        "lensmap",
        "scorecard",
        args,
        out,
        profile,
        &default_policy_settings(),
        &lensmaps,
        &[out_path],
        0,
        vec![],
    );
}

fn resolve_import_source_files(root: &Path, raw: &str, fallback_covers: &[String]) -> Vec<String> {
    if !raw.trim().is_empty() {
        return resolve_covers_to_files(root, &[raw.to_string()]);
    }
    resolve_covers_to_files(root, fallback_covers)
}

fn render_scorecard_markdown(title: &str, snapshot: &Value, health: &Value, rows: usize) -> String {
    let period = snapshot
        .get("period")
        .and_then(Value::as_str)
        .unwrap_or("run");
    let generated_at = snapshot
        .get("generated_at")
        .and_then(Value::as_str)
        .unwrap_or("");
    let lensmaps = snapshot
        .get("lensmaps")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let rates = snapshot
        .get("rates")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let totals = snapshot.get("totals").cloned().unwrap_or_else(|| json!({}));
    let stats = snapshot.get("stats").cloned().unwrap_or_else(|| json!({}));
    let policy = snapshot.get("policy").cloned().unwrap_or_else(|| json!({}));
    let policy_findings = snapshot
        .get("policy_findings")
        .cloned()
        .unwrap_or_else(|| json!({"errors": [], "warnings": []}));
    let policy_errors = policy_findings
        .get("errors")
        .and_then(Value::as_array)
        .map(|value| value.len())
        .unwrap_or(0);
    let policy_warnings = policy_findings
        .get("warnings")
        .and_then(Value::as_array)
        .map(|value| value.len())
        .unwrap_or(0);

    let metric_labels = |name: &str| match name {
        "note_coverage_rate" => "Note coverage".to_string(),
        "stale_note_ratio" => "Stale notes".to_string(),
        "orphan_notes_rate" => "Orphan notes".to_string(),
        "no_owner_notes_rate" => "No owner notes".to_string(),
        "reviewed_rate" => "Reviewed".to_string(),
        "anchor_fidelity_rate" => "Anchor fidelity".to_string(),
        "policy_pass_rate" => "Policy pass".to_string(),
        _ => name.to_string(),
    };

    let mut lines = vec![
        format!("# {}", title),
        String::new(),
        format!("- {}", tr("Period:", "周期：")),
        format!("- {}", period),
        format!("- Generated: {}", generated_at),
        format!("- LensMaps: {}", lensmaps.len()),
        String::new(),
    ];

    lines.push("## Health and Trends".to_string());
    if rates.is_empty() {
        lines.push("- no_rates".to_string());
    } else {
        for row in rates.iter().take(rows.max(1)) {
            let name = row.get("name").and_then(Value::as_str).unwrap_or("unknown");
            let value = row.get("value").and_then(Value::as_f64).unwrap_or(0.0);
            let trend = row.get("trend").and_then(Value::as_str).unwrap_or("n/a");
            let status = health
                .get(name)
                .and_then(|value| value.get("status"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            lines.push(format!(
                "- {}: {:.2} [{}] [{}]",
                metric_labels(name),
                value,
                status,
                trend
            ));
        }
    }
    lines.push(String::new());

    lines.push("## Totals".to_string());
    let total_fields = [
        ("entry_count", "Entries"),
        ("files_with_notes", "Files with notes"),
        ("source_files", "Source files"),
        ("stale_entries", "Stale entries"),
        ("orphan_notes", "Orphan notes"),
        ("unowned_notes", "No owner notes"),
        ("reviewed_notes", "Reviewed notes"),
        ("policy_checks", "Policy checks"),
        ("policy_failures", "Policy failures"),
        ("policy_pass_count", "Policy pass"),
        ("lensmap_count", "LensMap docs"),
        ("anchor_count", "Anchors"),
    ];
    for (key, label) in total_fields {
        let value = totals.get(key).and_then(Value::as_u64).unwrap_or(0);
        lines.push(format!("- {}: {}", label, value));
    }
    lines.push(String::new());

    lines.push("## Policy".to_string());
    lines.push(format!(
        "- Policy checks: {} checks | {} errors | {} warnings",
        totals
            .get("policy_checks")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        policy_errors,
        policy_warnings,
    ));
    if !policy.is_null() {
        let stale_after_days = policy
            .get("stale_after_days")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let require_owner = policy
            .get("require_owner")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let require_author = policy
            .get("require_author")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let require_template = policy
            .get("require_template")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let require_review_status = policy
            .get("require_review_status")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        lines.push(format!("- Stale after days: {}", stale_after_days));
        lines.push(format!("- Require owner: {}", require_owner));
        lines.push(format!("- Require author: {}", require_author));
        lines.push(format!("- Require template: {}", require_template));
        lines.push(format!(
            "- Require review status: {}",
            require_review_status
        ));
    }
    lines.push(String::new());

    lines.push("## Stats".to_string());
    let counter_sections = [
        ("Top files", "by_file"),
        ("Top directories", "by_directory"),
        ("Top owners", "by_owner"),
        ("Top kinds", "by_kind"),
        ("Top templates", "by_template"),
        ("Top review_status", "by_review_status"),
        ("Top scopes", "by_scope"),
    ];
    for (section, key) in counter_sections {
        lines.push(format!("### {}", section));
        let rows_value: &[Value] = match stats.get(key).and_then(Value::as_array) {
            Some(rows) => rows,
            None => &[],
        };
        if rows_value.is_empty() {
            lines.push("- none".to_string());
            lines.push(String::new());
            continue;
        }
        for item in rows_value.iter().take(rows.max(1)) {
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let count = item.get("count").and_then(Value::as_u64).unwrap_or(0);
            lines.push(format!("- `{}`: {}", name, count));
        }
        lines.push(String::new());
    }

    lines.push("## Age Distribution".to_string());
    if let Some(distribution) = snapshot.get("age_distribution").and_then(Value::as_array) {
        if distribution.is_empty() {
            lines.push("- none".to_string());
        } else {
            for item in distribution.iter() {
                let bucket = item
                    .get("bucket")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let count = item.get("count").and_then(Value::as_u64).unwrap_or(0);
                let share = item.get("share").and_then(Value::as_f64).unwrap_or(0.0);
                lines.push(format!("- {}: {} ({:.2}%)", bucket, count, share));
            }
        }
    } else {
        lines.push("- none".to_string());
    }

    lines.push("## Health".to_string());
    if let Some(obj) = health.as_object() {
        let row_limit = rows.max(1).min(obj.len());
        for (key, item) in obj.iter().take(row_limit) {
            let status = item
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let value = item.get("value").and_then(Value::as_f64).unwrap_or(0.0);
            let green = item.get("green").and_then(Value::as_f64).unwrap_or(0.0);
            let yellow = item.get("yellow").and_then(Value::as_f64).unwrap_or(0.0);
            let higher_is_better = item
                .get("higher_is_better")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            lines.push(format!(
                "- {}: {} ({:.2}, green>= {:.2}, yellow>= {:.2}, higher_is_better={})",
                key, status, value, green, yellow, higher_is_better
            ));
        }
    }

    lines.join("\n") + "\n"
}

fn build_metric_rate_row(name: &str, value: f64, trend: &str) -> Value {
    json!({
        "name": name,
        "value": clamp0_100(value),
        "trend": trend,
    })
}

fn cmd_autobot(root: &Path, args: &ParsedArgs) {
    let profile = AutobotProfile::parse(args.get("profile"));
    let lensmap_path = resolve_lensmap_path(root, args, None);
    if !lensmap_path.exists() {
        emit(
            lensmap_missing_payload(root, "autobot", &lensmap_path, args),
            1,
        );
    }
    let from = args.get("from").unwrap_or("").trim().to_string();
    let fallback_covers = vec![".".to_string()];
    let source_files = if from.is_empty() {
        let doc = load_doc(&lensmap_path, "group");
        resolve_import_source_files(root, "", &doc.covers)
    } else {
        resolve_import_source_files(root, &from, &fallback_covers)
    };
    if source_files.is_empty() {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "autobot",
                "error": "no_files_resolved",
                "from": from,
                "resume": args.has("resume"),
            }),
            1,
        );
    }

    let dry_run = args.has("dry-run");
    let apply = args.has("apply");
    let strict_policy = args.has("strict-policy") || profile == AutobotProfile::Conservative;
    let run_id = if let Some(raw) = args.get("run-id") {
        raw.to_string()
    } else {
        format!("autobot_{}", Utc::now().timestamp_millis())
    };
    let out_path = args
        .get("out")
        .map(|path| resolve_from_root(root, path))
        .unwrap_or_else(|| default_autobot_output_path(root));
    if !is_within_root(root, &out_path) {
        emit(
            json!({"ok": false, "error": "security_output_outside_root"}),
            1,
        );
    }
    let checkpoint_path = args
        .get("checkpoint")
        .map(|path| resolve_from_root(root, path))
        .unwrap_or_else(|| default_autobot_output_path(root));
    if !is_within_root(root, &checkpoint_path) {
        emit(
            json!({"ok": false, "error": "security_output_outside_root"}),
            1,
        );
    }

    let mut doc = load_doc(&lensmap_path, "group");
    let mut existing_keys = doc
        .entries
        .iter()
        .map(|entry| format!("{}::{}", entry.file, entry.ref_id.to_uppercase()))
        .collect::<HashSet<_>>();

    let (proposals, conflicts, files_scanned, filtered, duplicates) =
        collect_autobot_proposals(root, &source_files, &doc, &mut existing_keys, profile);

    let mut accepted = vec![];
    let mut pending_review = vec![];
    for proposal in proposals {
        if proposal.confidence >= profile.acceptance_threshold() {
            accepted.push(proposal);
        } else {
            pending_review.push(proposal);
        }
    }

    let mut simulation = doc.clone();
    for item in &accepted {
        simulation.entries.push(item.entry.clone());
    }
    let (policy, errors, warnings, _) = collect_aggregated_policy_findings(
        root,
        &[LoadedLensMapDoc {
            lensmap: normalize_relative(root, &lensmap_path),
            path: lensmap_path.clone(),
            doc: simulation.clone(),
        }],
    );
    let policy_failure_count = errors.len() + warnings.len();
    let mut policy_failures = vec![];
    for item in errors.iter().chain(warnings.iter()) {
        policy_failures.push(item.message.clone());
    }

    let mut applied = false;
    let mut applied_count = 0usize;
    let mut pending_count = pending_review.len();
    if apply && !dry_run {
        if strict_policy && policy_failure_count > 0 {
            applied = false;
        } else {
            for item in accepted.iter() {
                if conflicts.len() <= profile.conflict_tolerance() {
                    doc.entries.push(item.entry.clone());
                    applied_count = applied_count.saturating_add(1);
                    applied = true;
                } else {
                    pending_count = pending_count.saturating_add(1);
                }
            }
            if applied {
                save_doc(&lensmap_path, doc.clone());
            }
        }
    } else {
        pending_count = pending_count.saturating_add(accepted.len());
        applied_count = 0;
    }

    let receipt = AutobotReceipt {
        run_id: run_id.clone(),
        action: "autobot".to_string(),
        profile: profile.label().to_string(),
        created_at: now_iso(),
        updated_at: now_iso(),
        source_root: from.clone(),
        lensmap: normalize_relative(root, &lensmap_path),
        from: source_files.clone(),
        files_scanned,
        proposals_created: accepted.len() + pending_review.len(),
        accepted: applied_count,
        pending_review: pending_count,
        conflicts: conflicts.len(),
        policy_mode: profile.policy_mode().to_string(),
        policy_failures,
        applied,
        dry_run,
        checkpoint: Some(normalize_relative(root, &checkpoint_path)),
    };

    let _ = fs::write(
        &checkpoint_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({
                "run_id": run_id.clone(),
                "action": "autobot",
                "status": if dry_run { "dry_run" } else if applied { "applied" } else { "planned" },
                "profile": profile.label(),
                "source_files": source_files,
                "applied_count": applied_count,
                "pending_review": pending_count,
            }))
            .unwrap_or_else(|_| "{}".to_string())
        ),
    );

    let output_metrics = json!({
        "policy": policy,
        "policy_file_count": files_scanned,
        "policy_confidence": if profile == AutobotProfile::Conservative { "strict" } else if profile == AutobotProfile::Standard { "standard" } else { "permissive" },
        "policy_failures": receipt.policy_failures,
        "policy_sources": source_files,
    });

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "autobot",
        "run_id": run_id,
        "profile": profile.label(),
        "source": from,
        "lensmap": normalize_relative(root, &lensmap_path),
        "files_scanned": files_scanned,
        "filtered_out": filtered,
        "duplicates": duplicates,
        "proposals": accepted.len() + pending_review.len(),
        "accepted": accepted.len(),
        "applied": applied_count,
        "pending_review": pending_count,
        "conflicts": conflicts,
        "conflict_tolerance": profile.conflict_tolerance(),
        "apply_requested": apply,
        "dry_run": dry_run,
        "strict_policy": strict_policy,
        "snapshot": output_metrics,
        "receipt": serde_json::to_value(&receipt).unwrap_or_else(|_| json!({})),
        "out": normalize_relative(root, &out_path),
        "checkpoint": normalize_relative(root, &checkpoint_path),
        "ts": now_iso(),
    });
    let _ = fs::write(
        &out_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&out).unwrap_or_else(|_| "{}".to_string())
        ),
    );
    append_history(root, &out);
    emit(
        out,
        if applied {
            0
        } else if strict_policy && policy_failure_count > 0 {
            1
        } else {
            0
        },
    );
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
    let lensmap_path = resolve_lensmap_path(root, args, None);
    if !lensmap_path.exists() {
        emit(
            lensmap_missing_payload(root, "import", &lensmap_path, args),
            1,
        );
    }
    let source_files = resolve_import_source_files(root, &from, &[".".to_string()]);
    if source_files.is_empty() {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "import",
                "error": "no_files_resolved",
                "from": from,
            }),
            1,
        );
    }
    let dry_run = args.has("dry-run");
    let apply = args.has("apply");
    let profile = AutobotProfile::parse(args.get("profile"));
    let mut doc = load_doc(&lensmap_path, "group");
    let mut existing_keys = doc
        .entries
        .iter()
        .map(|entry| format!("{}::{}", entry.file, entry.ref_id.to_uppercase()))
        .collect::<HashSet<_>>();
    let (proposals, conflicts, files_scanned, filtered, duplicates) =
        collect_autobot_proposals(root, &source_files, &doc, &mut existing_keys, profile);
    let mut accepted = vec![];
    let mut pending_review = vec![];
    for proposal in proposals {
        if proposal.confidence >= profile.acceptance_threshold() {
            accepted.push(proposal);
        } else {
            pending_review.push(proposal);
        }
    }

    let mut imported_count = 0usize;
    if apply && !dry_run {
        for item in accepted {
            doc.entries.push(item.entry);
            imported_count = imported_count.saturating_add(1);
        }
        save_doc(&lensmap_path, doc.clone());
    }

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "import",
        "from": from,
        "lensmap": normalize_relative(root, &lensmap_path),
        "files_scanned": files_scanned,
        "filtered_out": filtered,
        "duplicates": duplicates,
        "conflicts": conflicts.len(),
        "imported": imported_count,
        "pending_review": pending_review.len(),
        "profile": profile.label(),
        "dry_run": dry_run,
        "apply_requested": apply,
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
    if !is_within_root(root, &out_path) {
        emit(
            json!({"ok": false, "error": "security_output_outside_root"}),
            1,
        );
    }
    let (lines, files_rendered, entries_rendered) = build_render_lines(
        root,
        &lensmap_path,
        &doc,
        render_filters_from_args(args),
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
        "owner": args.get("owner"),
        "template": args.get("template"),
        "review_status": args.get("review-status"),
        "scope": args.get("scope"),
        "tag": args.get("tag"),
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

    let artifact_layers = artifact_layers_for_doc(&doc);
    let write_markdown = artifact_layers
        .iter()
        .any(|layer| layer == "readable-markdown")
        || args.get("to").is_some()
        || args.get("out").is_some();
    let write_index =
        artifact_layers.iter().any(|layer| layer == "search-index") || args.get("index").is_some();
    let (canonical_path, default_markdown_path, default_index_path_rel) =
        artifact_paths(root, &lensmap_path);

    let mut markdown_output = None::<String>;
    let mut files_rendered = 0usize;
    let mut entries_rendered = 0usize;
    if write_markdown {
        let out_path = if let Some(out) = args.get("to").or_else(|| args.get("out")) {
            resolve_from_root(root, out)
        } else {
            default_render_output_path(&lensmap_path)
        };
        if !is_within_root(root, &out_path) {
            emit(
                json!({"ok": false, "error": "security_output_outside_root"}),
                1,
            );
        }
        let (lines, rendered_files, rendered_entries) = build_render_lines(
            root,
            &lensmap_path,
            &doc,
            render_filters_from_args(args),
            &tr("LensMap Render", "LensMap 渲染视图"),
        );
        ensure_dir(&out_path);
        let _ = fs::write(&out_path, format!("{}\n", lines.join("\n")));
        markdown_output = Some(normalize_relative(root, &out_path));
        files_rendered = rendered_files;
        entries_rendered = rendered_entries;
    }

    let mut index_output = None::<String>;
    let mut index_entries = None::<usize>;
    if write_index {
        let index_path = if let Some(index) = args.get("index") {
            resolve_from_root(root, index)
        } else {
            default_index_path(root)
        };
        if !is_within_root(root, &index_path) {
            emit(
                json!({"ok": false, "error": "security_output_outside_root"}),
                1,
            );
        }
        let entries = collect_repo_search_entries(root, &[normalize_relative(root, &lensmap_path)]);
        let index_doc = make_index_doc(
            root,
            vec![normalize_relative(root, &lensmap_path)],
            entries.clone(),
        );
        save_index_doc(&index_path, &index_doc);
        index_output = Some(normalize_relative(root, &index_path));
        index_entries = Some(entries.len());
    }

    let production_policy = production_for_doc(&doc);
    let production_enforced = args.has("production");
    let production_marker_hits = if production_enforced {
        collect_production_marker_hits(
            root,
            &[LoadedLensMapDoc {
                lensmap: lensmap_rel.clone(),
                path: lensmap_path.clone(),
                doc: doc.clone(),
            }],
            &production_policy,
        )
    } else {
        vec![]
    };

    let ok = unresolved.is_empty() && (!production_enforced || production_marker_hits.is_empty());
    let out = json!({
        "ok": ok,
        "type": "lensmap",
        "action": "sync",
        "lensmap": normalize_relative(root, &lensmap_path),
        "production_policy": production_policy,
        "production_enforced": production_enforced,
        "production_marker_files": production_marker_hits,
        "artifacts": {
            "canonical_json": canonical_path,
            "readable_markdown": {
                "path": default_markdown_path,
                "output": markdown_output,
            },
            "search_index": {
                "path": default_index_path_rel,
                "output": index_output,
                "entry_count": index_entries,
            }
        },
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
    let owner_filter = args
        .get("owner")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let template_filter = args
        .get("template")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let review_filter = args
        .get("review-status")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let scope_filter = args
        .get("scope")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let tag_filter = args
        .get("tag")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let search_filters = search_filters_from_args(args);

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
        .filter(|entry| search_entry_matches_filters(entry, search_filters))
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
                "title": entry.title,
                "owner": entry.owner,
                "author": entry.author,
                "scope": entry.scope,
                "template": entry.template,
                "review_status": entry.review_status,
                "review_due_at": entry.review_due_at,
                "updated_at": entry.updated_at,
                "tags": entry.tags,
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
        "owner": owner_filter,
        "template": template_filter,
        "review_status": review_filter,
        "scope": scope_filter,
        "tag": tag_filter,
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
            },
            "positioning": metadata_string(&doc.metadata, "positioning", "external-doc-layer"),
            "boilerplate_scope": metadata_string(&doc.metadata, "boilerplate_scope", "knowledge"),
            "anchor_visibility": metadata_string(&doc.metadata, "editor_anchor_visibility", "dimmed"),
            "artifact_layers": artifact_layers_for_doc(&doc),
            "artifacts": {
                "canonical_json": {
                    "path": artifact_paths(root, &lensmap_path).0,
                    "exists": lensmap_path.exists(),
                },
                "readable_markdown": {
                    "path": artifact_paths(root, &lensmap_path).1,
                    "exists": resolve_from_root(root, &artifact_paths(root, &lensmap_path).1).exists(),
                },
                "search_index": {
                    "path": artifact_paths(root, &lensmap_path).2,
                    "exists": resolve_from_root(root, &artifact_paths(root, &lensmap_path).2).exists(),
                }
            },
            "policy": policy_for_doc(&doc),
            "summary": summary_stats_to_value(
                &summary_stats(
                    &collect_doc_search_entries(root, &lensmap_path, &doc),
                    policy_for_doc(&doc).stale_after_days,
                ),
                5,
            ),
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

fn load_repo_lensmap_docs(root: &Path, lensmaps: &[String]) -> Vec<LoadedLensMapDoc> {
    lensmaps
        .iter()
        .filter_map(|lensmap| {
            let path = resolve_from_root(root, lensmap);
            if !is_within_root(root, &path) || !path.exists() {
                return None;
            }
            Some(LoadedLensMapDoc {
                lensmap: normalize_relative(root, &path),
                path: path.clone(),
                doc: load_doc(&path, "group"),
            })
        })
        .collect()
}

fn read_history_rows(root: &Path, path: &Path) -> Vec<Value> {
    if !path.exists() {
        return vec![];
    }
    let mut out = vec![];
    for raw in fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if let Ok(row) = serde_json::from_str::<Value>(raw) {
            if let Some(path_value) = row.get("path").and_then(Value::as_str) {
                if !is_within_root(root, &resolve_from_root(root, path_value)) {
                    continue;
                }
            }
            out.push(row);
        }
    }
    out
}

fn as_f64_or_default(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_u64().map(|v| v as f64))
        .or_else(|| {
            value
                .as_i64()
                .and_then(|v| if v >= 0 { Some(v as f64) } else { None })
        })
}

fn entry_record_from_search_entry(entry: &SearchEntryRecord) -> EntryRecord {
    EntryRecord {
        ref_id: entry.ref_id.clone(),
        file: entry.file.clone(),
        anchor_id: entry.anchor_id.clone(),
        kind: entry.kind.clone(),
        text: entry.text.clone(),
        title: entry.title.clone(),
        owner: entry.owner.clone(),
        author: entry.author.clone(),
        scope: entry.scope.clone(),
        template: entry.template.clone(),
        review_status: entry.review_status.clone(),
        review_due_at: entry.review_due_at.clone(),
        updated_at: entry.updated_at.clone(),
        tags: entry.tags.clone(),
        created_at: None,
        source: entry.lensmap.clone().into(),
    }
}

fn metric_rate_trend(current: f64, previous: Option<f64>) -> String {
    let Some(previous) = previous else {
        return "n/a".to_string();
    };

    let delta = current - previous;
    if delta.abs() < 0.01 {
        return "flat".to_string();
    }
    if delta > 0.0 {
        format!("up +{:.2}", delta)
    } else {
        format!("down {:.2}", delta.abs())
    }
}

fn extract_metric_rates(metrics: &Value) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    let Some(rows) = metrics.get("rates").and_then(Value::as_array) else {
        return out;
    };
    for row in rows {
        let Some(name) = row.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(value) = row.get("value").and_then(as_f64_or_default) else {
            continue;
        };
        out.insert(name.to_string(), clamp0_100(value));
    }
    out
}

fn latest_metric_rates(root: &Path, period: &str) -> BTreeMap<String, f64> {
    let history_path = default_metric_history_path(root);
    let history = read_history_rows(root, &history_path);

    let mut latest: Option<(DateTime<Utc>, Value)> = None;
    let mut latest_rates = BTreeMap::new();
    for row in history {
        if row.get("period").and_then(Value::as_str).unwrap_or("run") != period {
            continue;
        }
        let Some(timestamp) = row
            .get("generated_at")
            .and_then(Value::as_str)
            .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
            .map(|dt| dt.with_timezone(&Utc))
        else {
            continue;
        };

        let should_update = if let Some((prev, _)) = latest.as_ref() {
            timestamp > *prev
        } else {
            true
        };
        if should_update {
            let snapshot = row
                .get("snapshot")
                .unwrap_or_else(|| row.get("metrics").unwrap_or(&row));
            latest = Some((timestamp, snapshot.clone()));
            latest_rates = extract_metric_rates(snapshot);
        }
    }

    if latest.is_none() {
        return BTreeMap::new();
    }
    let _ = latest;
    latest_rates
}

fn append_metric_history(
    root: &Path,
    period: &str,
    generated_at: &str,
    metrics: &Value,
    out_path: &Path,
) {
    let history_path = default_metric_history_path(root);
    ensure_dir(&history_path);
    let row = json!({
        "period": period,
        "generated_at": generated_at,
        "output": normalize_relative(root, out_path),
        "snapshot": metrics,
    });
    let mut existing = fs::read_to_string(&history_path).unwrap_or_default();
    existing.push_str(&serde_json::to_string(&row).unwrap_or_else(|_| "{}".to_string()));
    existing.push('\n');
    let _ = fs::write(&history_path, existing);
}

fn metric_health_row(value: f64, green: f64, yellow: f64, higher_is_better: bool) -> Value {
    json!({
        "status": metric_band(value, green, yellow, higher_is_better),
        "value": clamp0_100(value),
        "green": green,
        "yellow": yellow,
        "higher_is_better": higher_is_better,
    })
}

fn build_metric_snapshot(
    root: &Path,
    lensmaps: &[String],
    top: usize,
    period: &str,
    generated_at: &str,
) -> Value {
    let loaded_docs = load_repo_lensmap_docs(root, lensmaps);
    let policy_sources = loaded_docs
        .iter()
        .map(|loaded| (loaded.lensmap.clone(), policy_for_doc(&loaded.doc)))
        .collect::<Vec<_>>();
    let (policy, errors, warnings, _) = collect_aggregated_policy_findings(root, &loaded_docs);
    let entries = collect_repo_search_entries(root, lensmaps);
    let stats = summary_stats(&entries, policy.stale_after_days);

    let now = Utc::now();
    let mut orphan_notes = 0usize;
    let mut unowned_notes = 0usize;
    let mut unreviewed_notes = 0usize;
    let mut by_age_bucket: BTreeMap<String, usize> = BTreeMap::new();

    for entry in &entries {
        let entry_record = entry_record_from_search_entry(entry);
        if entry_record
            .anchor_id
            .as_ref()
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
        {
            orphan_notes = orphan_notes.saturating_add(1);
        }
        if entry_record
            .owner
            .as_ref()
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
        {
            unowned_notes = unowned_notes.saturating_add(1);
        }

        let review_status = entry_record
            .review_status
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if review_status.is_empty()
            || review_status == "draft"
            || review_status == "todo"
            || review_status == "pending"
            || review_status == "in_review"
        {
            unreviewed_notes = unreviewed_notes.saturating_add(1);
        }

        let bucket = entry_age_bucket(&entry_record, &now);
        increment_counter(&mut by_age_bucket, bucket, "untracked");
    }

    let entry_count = entries.len();
    let source_files = all_supported_files(root);
    let total_source_files = source_files.len().max(1);
    let reviewed_notes = entry_count.saturating_sub(unreviewed_notes);

    let note_coverage_rate = percentage(stats.files_with_notes, total_source_files);
    let stale_note_ratio = percentage(stats.stale_entries, entry_count);
    let orphan_notes_rate = percentage(orphan_notes, entry_count);
    let no_owner_notes_rate = percentage(unowned_notes, entry_count);
    let reviewed_rate = percentage(reviewed_notes, entry_count);
    let anchor_fidelity_rate = percentage(entry_count.saturating_sub(orphan_notes), entry_count);

    let policy_failures = errors.len() + warnings.len();
    let policy_checks = entry_count.max(loaded_docs.len()).max(1);
    let policy_pass_count = policy_checks.saturating_sub(policy_failures.min(policy_checks));
    let policy_pass_rate = percentage(policy_pass_count, policy_checks);

    let previous_rates = latest_metric_rates(root, period);

    let rates = vec![
        build_metric_rate_row(
            "note_coverage_rate",
            note_coverage_rate,
            &metric_rate_trend(
                note_coverage_rate,
                previous_rates.get("note_coverage_rate").copied(),
            ),
        ),
        build_metric_rate_row(
            "stale_note_ratio",
            stale_note_ratio,
            &metric_rate_trend(
                stale_note_ratio,
                previous_rates.get("stale_note_ratio").copied(),
            ),
        ),
        build_metric_rate_row(
            "orphan_notes_rate",
            orphan_notes_rate,
            &metric_rate_trend(
                orphan_notes_rate,
                previous_rates.get("orphan_notes_rate").copied(),
            ),
        ),
        build_metric_rate_row(
            "no_owner_notes_rate",
            no_owner_notes_rate,
            &metric_rate_trend(
                no_owner_notes_rate,
                previous_rates.get("no_owner_notes_rate").copied(),
            ),
        ),
        build_metric_rate_row(
            "reviewed_rate",
            reviewed_rate,
            &metric_rate_trend(reviewed_rate, previous_rates.get("reviewed_rate").copied()),
        ),
        build_metric_rate_row(
            "anchor_fidelity_rate",
            anchor_fidelity_rate,
            &metric_rate_trend(
                anchor_fidelity_rate,
                previous_rates.get("anchor_fidelity_rate").copied(),
            ),
        ),
        build_metric_rate_row(
            "policy_pass_rate",
            policy_pass_rate,
            &metric_rate_trend(
                policy_pass_rate,
                previous_rates.get("policy_pass_rate").copied(),
            ),
        ),
    ];

    let health = json!({
        "note_coverage_rate": metric_health_row(note_coverage_rate, 75.0, 55.0, true),
        "stale_note_ratio": metric_health_row(stale_note_ratio, 10.0, 20.0, false),
        "orphan_notes_rate": metric_health_row(orphan_notes_rate, 5.0, 12.0, false),
        "no_owner_notes_rate": metric_health_row(no_owner_notes_rate, 5.0, 12.0, false),
        "reviewed_rate": metric_health_row(reviewed_rate, 80.0, 65.0, true),
        "anchor_fidelity_rate": metric_health_row(anchor_fidelity_rate, 90.0, 80.0, true),
        "policy_pass_rate": metric_health_row(policy_pass_rate, 95.0, 90.0, true),
    });

    let totals = json!({
        "entry_count": entry_count,
        "files_with_notes": stats.files_with_notes,
        "source_files": total_source_files,
        "stale_entries": stats.stale_entries,
        "orphan_notes": orphan_notes,
        "unowned_notes": unowned_notes,
        "reviewed_notes": reviewed_notes,
        "policy_checks": policy_checks,
        "policy_failures": policy_failures,
        "policy_pass_count": policy_pass_count,
        "lensmap_count": lensmaps.len(),
        "anchor_count": loaded_docs.iter().map(|loaded| loaded.doc.anchors.len()).sum::<usize>(),
    });

    let mut age_distribution = vec![];
    for (bucket, count) in by_age_bucket {
        age_distribution.push(json!({
            "bucket": bucket,
            "count": count,
            "share": percentage(count, entry_count),
        }));
    }

    let policy_sources_out = policy_sources
        .iter()
        .map(|(lensmap, policy)| json!({"lensmap": lensmap, "policy": policy}))
        .collect::<Vec<_>>();

    json!({
        "period": period,
        "generated_at": generated_at,
        "lensmaps": lensmaps,
        "aggregation": "strictest_union",
        "policy": policy,
        "policy_sources": policy_sources_out,
        "policy_findings": {
            "errors": errors,
            "warnings": warnings,
        },
        "totals": totals,
        "stats": summary_stats_to_value(&stats, top),
        "rates": rates,
        "health": health,
        "age_distribution": age_distribution,
    })
}

fn percentage(value: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        (value as f64 / total as f64) * 100.0
    }
}

fn metric_band(value: f64, green: f64, yellow: f64, higher_is_better: bool) -> &'static str {
    if higher_is_better {
        if value >= green {
            "green"
        } else if value >= yellow {
            "yellow"
        } else {
            "red"
        }
    } else {
        if value <= green {
            "green"
        } else if value <= yellow {
            "yellow"
        } else {
            "red"
        }
    }
}

fn entry_age_bucket(entry: &EntryRecord, now: &DateTime<Utc>) -> &'static str {
    let Some(updated) = entry_timestamp(entry) else {
        return "untracked";
    };
    let delta = now.signed_duration_since(updated).num_days();
    if delta < 0 {
        return "future";
    }
    if delta <= 7 {
        "0-7d"
    } else if delta <= 30 {
        "8-30d"
    } else if delta <= 90 {
        "31-90d"
    } else if delta <= 365 {
        "91-365d"
    } else {
        "365d+"
    }
}

fn clamp0_100(value: f64) -> f64 {
    value.clamp(0.0, 100.0)
}

fn resolve_policy_lensmap_docs(root: &Path, args: &ParsedArgs) -> Vec<LoadedLensMapDoc> {
    if args.get("lensmaps").is_some() {
        let lensmaps = resolve_search_lensmap_paths(root, args);
        if lensmaps.is_empty() {
            emit(json!({"ok": false, "error": "no_lensmap_files_found"}), 1);
        }
        return load_repo_lensmap_docs(root, &lensmaps);
    }
    if args.get("lensmap").is_none() {
        let lensmaps = discover_lensmap_files(root, args.get("bundle-dir").unwrap_or(".lenspack"));
        if lensmaps.is_empty() {
            emit(json!({"ok": false, "error": "no_lensmap_files_found"}), 1);
        }
        return load_repo_lensmap_docs(root, &lensmaps);
    }

    let lensmap_path = resolve_lensmap_path(root, args, None);
    if !lensmap_path.exists() {
        emit(
            lensmap_missing_payload(root, "policy_check", &lensmap_path, args),
            1,
        );
    }
    vec![LoadedLensMapDoc {
        lensmap: normalize_relative(root, &lensmap_path),
        path: lensmap_path.clone(),
        doc: load_doc(&lensmap_path, "group"),
    }]
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
                title: entry.title.clone(),
                owner: entry.owner.clone(),
                author: entry.author.clone(),
                scope: entry.scope.clone(),
                template: entry.template.clone(),
                review_status: entry.review_status.clone(),
                review_due_at: entry.review_due_at.clone(),
                updated_at: entry.updated_at.clone(),
                tags: entry.tags.clone(),
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

fn search_entry_matches_filters(entry: &SearchEntryRecord, filters: SearchFilters<'_>) -> bool {
    if let Some(file) = filters.file {
        if entry.file != file {
            return false;
        }
    }
    if let Some(symbol) = filters.symbol {
        if entry.symbol.as_deref() != Some(symbol) && entry.symbol_path.as_deref() != Some(symbol) {
            return false;
        }
    }
    if let Some(kind) = filters.kind {
        if entry.kind.as_deref() != Some(kind) {
            return false;
        }
    }
    if let Some(owner) = filters.owner {
        if entry.owner.as_deref() != Some(owner) {
            return false;
        }
    }
    if let Some(template) = filters.template {
        if entry.template.as_deref() != Some(template) {
            return false;
        }
    }
    if let Some(review) = filters.review_status {
        if entry.review_status.as_deref() != Some(review) {
            return false;
        }
    }
    if let Some(scope) = filters.scope {
        if entry.scope.as_deref() != Some(scope) {
            return false;
        }
    }
    if let Some(tag) = filters.tag {
        if !entry.tags.iter().any(|candidate| candidate == tag) {
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
    let title = entry.title.as_deref().unwrap_or("").to_lowercase();
    let symbol = entry.symbol.as_deref().unwrap_or("").to_lowercase();
    let symbol_path = entry.symbol_path.as_deref().unwrap_or("").to_lowercase();
    let file = entry.file.to_lowercase();
    let kind = entry.kind.as_deref().unwrap_or("").to_lowercase();
    let owner = entry.owner.as_deref().unwrap_or("").to_lowercase();
    let author = entry.author.as_deref().unwrap_or("").to_lowercase();
    let scope = entry.scope.as_deref().unwrap_or("").to_lowercase();
    let template = entry.template.as_deref().unwrap_or("").to_lowercase();
    let review_status = entry.review_status.as_deref().unwrap_or("").to_lowercase();
    let tags = entry.tags.join(" ").to_lowercase();
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
    if title.contains(&normalized_query) {
        score += 95;
    }
    if file.contains(&normalized_query) {
        score += 80;
    }
    if kind == normalized_query {
        score += 70;
    }
    if owner.contains(&normalized_query) {
        score += 60;
    }
    if author.contains(&normalized_query) {
        score += 50;
    }
    if scope.contains(&normalized_query) {
        score += 50;
    }
    if template == normalized_query {
        score += 65;
    }
    if review_status == normalized_query {
        score += 55;
    }
    if tags.contains(&normalized_query) {
        score += 45;
    }

    for token in normalized_query.split_whitespace() {
        if token.is_empty() {
            continue;
        }
        if text.contains(token) {
            score += 16;
        }
        if title.contains(token) {
            score += 14;
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
        if owner.contains(token) || author.contains(token) || scope.contains(token) {
            score += 8;
        }
        if template.contains(token) || review_status.contains(token) || tags.contains(token) {
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
    println!("lensmap annotate --lensmap=path (--ref=<HEX-start[-end]> | --file=path --symbol=name [--symbol-path=Outer.inner] [--offset=N] [--end-offset=M]) (--text=<text> | --template=name) [--kind=comment|doc|todo|decision] [--owner=name] [--author=name] [--review-status=draft|in_review|approved] [--tags=a,b]");
    println!("lensmap template add <type>");
    println!("lensmap template list");
    println!("lensmap scan [--lensmap=path] [--covers=a,b] [--anchor-mode=smart|all] [--anchor-placement=inline|standalone] [--dry-run]");
    println!("lensmap extract-comments [--lensmap=path] [--covers=a,b] [--dry-run]");
    println!(
        "lensmap unmerge [--lensmap=path] [--covers=a,b] [--strip] [--clean-anchors=true|false] [--clean-refs=true|false] [--dry-run]  # alias of extract-comments"
    );
    println!("lensmap merge [--lensmap=path] [--covers=a,b] [--dry-run]");
    println!(
        "lensmap package [--bundle-dir=.lenspack] [--mode=move|copy] [--lensmaps=a,b] [--strip-sources] [--production] [--out-format=tar.gz] [--dry-run]"
    );
    println!(
        "lensmap package evidence [--bundle-dir=.lenspack] [--compression-mode=none|copy] [--redaction-profile=debug|audit|operational|clinical|emergency] [--retention-days=N] [--include-metrics] [--include-scorecard] [--include-policy] [--include-pr-report] [--checkpoint=<path>] [--resume]"
    );
    println!("lensmap unpackage [--bundle-dir=.lenspack] [--on-missing=prompt|skip|error] [--map=old_dir=new_dir] [--overwrite] [--dry-run]");
    println!("lensmap strip [--source=src,api] [--out-dir=dist/prod] [--clean-anchors=true|false] [--clean-refs=true|false] [--exclude-patterns=glob,glob] [--check] [--in-place --force] [--dry-run]");
    println!(
        "lensmap verify [--bundle-dir=.lenspack] [--envelope=<path>]  # validate bundle signatures and replay chain"
    );
    println!(
        "lensmap restore [--bundle-dir=.lenspack] [--on-missing=prompt|skip|error] [--overwrite] [--dry-run]  # alias of evidence restore"
    );
    println!("lensmap validate [--lensmap=path]");
    println!("lensmap policy init [--lensmap=path] [--require-owner=true|false] [--require-author=true|false] [--require-template=true|false] [--require-review-status=true|false] [--stale-after-days=N] [--required-patterns=glob,glob] [--production-strip-anchors=true|false] [--production-strip-refs=true|false] [--production-strip-on-package=true|false] [--production-exclude-patterns=glob,glob]");
    println!("lensmap policy check [--lensmap=path | --lensmaps=a,b] [--fail-on-warnings] [--report-only] [--production] [--out=path]  # aggregates all discovered LensMaps by default");
    println!("lensmap reanchor [--lensmap=path] [--dry-run]  # git-aware conflict protection on dirty overlaps");
    println!("lensmap render [--lensmap=path] [--file=path] [--symbol=name|path] [--ref=HEX-start[-end]] [--kind=comment|doc|todo|decision] [--owner=name] [--template=name] [--review-status=status] [--scope=path] [--tag=tag] [--out=path]");
    println!("lensmap parse [--lensmap=path] [--out=path]  # alias of render");
    println!("lensmap show [--lensmap=path] [--file=path] [--symbol=name|path] [--ref=HEX-start[-end]] [--kind=comment|doc|todo|decision] [--owner=name] [--template=name] [--review-status=status] [--scope=path] [--tag=tag] [--out=path]");
    println!("lensmap simplify [--lensmap=path]");
    println!("lensmap index [--lensmaps=a,b] [--index=path|--out=path]");
    println!("lensmap search --query=<text> [--lensmaps=a,b] [--index=path] [--file=path] [--symbol=name|path] [--kind=comment|doc|todo|decision] [--owner=name] [--template=name] [--review-status=status] [--scope=path] [--tag=tag] [--limit=N]");
    println!("lensmap summary [--lensmaps=a,b] [--file=path] [--kind=comment|doc|todo|decision] [--owner=name] [--template=name] [--review-status=status] [--scope=path] [--tag=tag] [--base=rev --head=rev] [--top=N] [--out=path]  # strictest policy aggregation across lensmaps");
    println!("lensmap metrics [--lensmaps=a,b] [--bundle-dir=.lenspack] [--period=run] [--top=N] [--out=path]");
    println!("lensmap scorecard [--lensmaps=a,b] [--bundle-dir=.lenspack] [--period=run] [--top=N] [--out=path]");
    println!("lensmap pr report [--lensmaps=a,b] [--base=rev --head=rev] [--strict] [--production] [--out=path]  # strictest policy aggregation across lensmaps");
    println!("lensmap polish");
    println!("lensmap import --from=<path>");
    println!("lensmap sync [--lensmap=path] [--to=path] [--index=path] [--production]  # reanchor + simplify + artifact refresh");
    println!("lensmap expose --name=<lens_name>");
    println!("lensmap status [--lensmap=path]");
    println!();
    println!("{}", tr("Quickstart:", "快速开始："));
    println!("  lensmap init demo --mode=group --covers=demo/src");
    println!("  lensmap scan --lensmap=demo/lensmap.json --anchor-mode=smart");
    println!("  lensmap extract-comments --lensmap=demo/lensmap.json");
    println!("  lensmap template list");
    println!("  lensmap annotate --lensmap=demo/lensmap.json --file=demo/src/app.ts --symbol=run --symbol-path=App.run --offset=1 --template=architecture --owner=platform");
    println!("  lensmap policy init --lensmap=demo/lensmap.json --require-owner=true --require-template=true --stale-after-days=30");
    println!("  lensmap policy check --lensmap=demo/lensmap.json");
    println!("  lensmap show --lensmap=demo/lensmap.json --file=demo/src/app.ts");
    println!("  lensmap index --index=demo/.lensmap-index.json");
    println!("  lensmap search --index=demo/.lensmap-index.json --query=why");
    println!("  lensmap summary --lensmaps=demo/lensmap.json --owner=platform --out=demo/lensmap-summary.md");
    println!(
        "  lensmap metrics --lensmaps=demo/lensmap.json --period=run --out=demo/lensmap-metrics.json"
    );
    println!(
        "  lensmap scorecard --lensmaps=demo/lensmap.json --period=run --out=demo/lensmap-scorecard.md"
    );
    println!(
        "  lensmap pr report --lensmaps=demo/lensmap.json --base=origin/main --head=HEAD --strict"
    );
    println!("  lensmap merge --lensmap=demo/lensmap.json");
    println!("  lensmap unmerge --lensmap=demo/lensmap.json --strip");
    println!("  lensmap package --bundle-dir=.lenspack");
    println!("  lensmap package --bundle-dir=.lenspack --strip-sources --out-format=tar.gz");
    println!("  lensmap strip --source=demo/src --out-dir=demo/dist/prod");
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
        "template" if args.positional.get(1).map(String::as_str) == Some("list") => {
            cmd_template_list(&root)
        }
        "scan" => cmd_scan(&root, &args),
        "extract-comments" => cmd_extract_comments(&root, &args),
        "unmerge" => cmd_extract_comments(&root, &args),
        "merge" => cmd_merge(&root, &args),
        "package" if args.positional.get(1).map(String::as_str) == Some("evidence") => {
            cmd_package_evidence(&root, &args)
        }
        "package" => cmd_package(&root, &args),
        "strip" => cmd_strip(&root, &args),
        "unpackage" => cmd_unpackage(&root, &args),
        "restore" => cmd_unpackage(&root, &args),
        "verify" => cmd_verify(&root, &args),
        "validate" => cmd_validate(&root, &args),
        "policy" if args.positional.get(1).map(String::as_str) == Some("init") => {
            cmd_policy_init(&root, &args)
        }
        "policy" if args.positional.get(1).map(String::as_str) == Some("check") => {
            cmd_policy_check(&root, &args)
        }
        "reanchor" => cmd_reanchor(&root, &args),
        "render" => cmd_render(&root, &args),
        "parse" => cmd_render(&root, &args),
        "show" => cmd_show(&root, &args),
        "simplify" => cmd_simplify(&root, &args),
        "index" => cmd_index(&root, &args),
        "search" => cmd_search(&root, &args),
        "summary" => cmd_summary(&root, &args),
        "metrics" => cmd_metrics(&root, &args),
        "scorecard" => cmd_scorecard(&root, &args),
        "pr" if args.positional.get(1).map(String::as_str) == Some("report") => {
            cmd_pr_report(&root, &args)
        }
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

    #[test]
    fn parse_git_status_rel_preserves_modified_and_renamed_paths() {
        assert_eq!(
            parse_git_status_rel(" M api/src/lib.rs"),
            Some("api/src/lib.rs".to_string())
        );
        assert_eq!(
            parse_git_status_rel("R  old/path.rs -> api/src/new.rs"),
            Some("api/src/new.rs".to_string())
        );
        assert_eq!(
            parse_git_status_rel("?? ui/src/view.ts"),
            Some("ui/src/view.ts".to_string())
        );
    }

    #[test]
    fn collect_policy_findings_requires_metadata_and_flags_stale_entries() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = env::temp_dir().join(format!("lensmap_policy_{}", nonce));
        let src_dir = root.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("app.rs"), "fn run() {}\n").unwrap();

        let mut doc = make_lensmap_doc("group", vec!["src".to_string()]);
        let policy = PolicySettings {
            require_owner: true,
            require_author: true,
            require_template: true,
            require_review_status: true,
            stale_after_days: 1,
            required_patterns: vec!["src/*.rs".to_string()],
        };
        store_policy(&mut doc.metadata, &policy);
        doc.entries.push(EntryRecord {
            ref_id: "ABCDEF-1".to_string(),
            file: "src/app.rs".to_string(),
            text: Some("Context:\nDecision:".to_string()),
            updated_at: Some(
                (Utc::now() - Duration::days(5)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            ),
            ..EntryRecord::default()
        });

        let (_policy, errors, warnings) = collect_policy_findings(&root, &doc);
        let error_codes = errors
            .iter()
            .map(|finding| finding.code.clone())
            .collect::<HashSet<_>>();
        let warning_codes = warnings
            .iter()
            .map(|finding| finding.code.clone())
            .collect::<HashSet<_>>();
        assert!(error_codes.contains("missing_owner"));
        assert!(error_codes.contains("missing_author"));
        assert!(error_codes.contains("missing_template"));
        assert!(error_codes.contains("missing_review_status"));
        assert!(warning_codes.contains("stale_entry"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn collect_policy_findings_flags_required_patterns_without_notes() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = env::temp_dir().join(format!("lensmap_policy_pattern_{}", nonce));
        let src_dir = root.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("app.rs"), "fn run() {}\n").unwrap();

        let mut doc = make_lensmap_doc("group", vec!["src".to_string()]);
        let policy = PolicySettings {
            required_patterns: vec!["src/*.rs".to_string()],
            ..default_policy_settings()
        };
        store_policy(&mut doc.metadata, &policy);

        let (_policy, errors, warnings) = collect_policy_findings(&root, &doc);
        assert!(warnings.is_empty());
        assert!(errors
            .iter()
            .any(|finding| finding.code == "required_pattern_missing_notes"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn aggregate_policy_settings_uses_union_and_strictest_staleness() {
        let first = PolicySettings {
            require_owner: true,
            require_author: false,
            require_template: false,
            require_review_status: false,
            stale_after_days: 14,
            required_patterns: vec!["src/*.rs".to_string()],
        };
        let second = PolicySettings {
            require_owner: false,
            require_author: true,
            require_template: true,
            require_review_status: false,
            stale_after_days: 7,
            required_patterns: vec!["src/*.rs".to_string(), "src/**/*.ts".to_string()],
        };
        let third = PolicySettings {
            require_owner: false,
            require_author: false,
            require_template: false,
            require_review_status: true,
            stale_after_days: 0,
            required_patterns: vec!["src/**/*.kt".to_string()],
        };

        let aggregated = aggregate_policy_settings([&first, &second, &third]);
        let patterns = aggregated
            .required_patterns
            .iter()
            .cloned()
            .collect::<HashSet<_>>();

        assert!(aggregated.require_owner);
        assert!(aggregated.require_author);
        assert!(aggregated.require_template);
        assert!(aggregated.require_review_status);
        assert_eq!(aggregated.stale_after_days, 7);
        assert_eq!(patterns.len(), 3);
        assert!(patterns.contains("src/*.rs"));
        assert!(patterns.contains("src/**/*.ts"));
        assert!(patterns.contains("src/**/*.kt"));
    }

    #[test]
    fn aggregated_policy_findings_apply_union_across_multiple_lensmaps() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = env::temp_dir().join(format!("lensmap_policy_aggregate_{}", nonce));
        let api_dir = root.join("api");
        let ui_dir = root.join("ui");
        fs::create_dir_all(&api_dir).unwrap();
        fs::create_dir_all(&ui_dir).unwrap();
        fs::write(api_dir.join("service.rs"), "fn run() {}\n").unwrap();
        fs::write(ui_dir.join("view.ts"), "function render() {}\n").unwrap();

        let mut api_doc = make_lensmap_doc("group", vec!["api".to_string()]);
        store_policy(
            &mut api_doc.metadata,
            &PolicySettings {
                require_owner: true,
                stale_after_days: 30,
                required_patterns: vec!["api/*.rs".to_string()],
                ..default_policy_settings()
            },
        );
        api_doc.entries.push(EntryRecord {
            ref_id: "ABCDEF-1".to_string(),
            file: "api/service.rs".to_string(),
            text: Some("Runtime contract".to_string()),
            ..EntryRecord::default()
        });

        let mut ui_doc = make_lensmap_doc("group", vec!["ui".to_string()]);
        store_policy(
            &mut ui_doc.metadata,
            &PolicySettings {
                require_template: true,
                stale_after_days: 5,
                required_patterns: vec!["ui/*.ts".to_string()],
                ..default_policy_settings()
            },
        );
        ui_doc.entries.push(EntryRecord {
            ref_id: "FEDCBA-1".to_string(),
            file: "ui/view.ts".to_string(),
            text: Some("UX note".to_string()),
            owner: Some("frontend".to_string()),
            updated_at: Some(
                (Utc::now() - Duration::days(12))
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            ),
            ..EntryRecord::default()
        });

        let docs = vec![
            LoadedLensMapDoc {
                lensmap: "api/lensmap.json".to_string(),
                path: root.join("api/lensmap.json"),
                doc: api_doc,
            },
            LoadedLensMapDoc {
                lensmap: "ui/lensmap.json".to_string(),
                path: root.join("ui/lensmap.json"),
                doc: ui_doc,
            },
        ];

        let (policy, errors, warnings, sources) = collect_aggregated_policy_findings(&root, &docs);
        let error_codes = errors
            .iter()
            .map(|finding| {
                (
                    finding.ref_id.clone(),
                    finding.code.clone(),
                    finding.lensmap.clone(),
                )
            })
            .collect::<HashSet<_>>();
        let warning_codes = warnings
            .iter()
            .map(|finding| {
                (
                    finding.ref_id.clone(),
                    finding.code.clone(),
                    finding.lensmap.clone(),
                )
            })
            .collect::<HashSet<_>>();

        assert!(policy.require_owner);
        assert!(policy.require_template);
        assert_eq!(policy.stale_after_days, 5);
        assert_eq!(sources.len(), 2);
        assert!(error_codes.contains(&(
            "ABCDEF-1".to_string(),
            "missing_owner".to_string(),
            Some("api/lensmap.json".to_string())
        )));
        assert!(error_codes.contains(&(
            "ABCDEF-1".to_string(),
            "missing_template".to_string(),
            Some("api/lensmap.json".to_string())
        )));
        assert!(error_codes.contains(&(
            "FEDCBA-1".to_string(),
            "missing_template".to_string(),
            Some("ui/lensmap.json".to_string())
        )));
        assert!(warning_codes.contains(&(
            "FEDCBA-1".to_string(),
            "stale_entry".to_string(),
            Some("ui/lensmap.json".to_string())
        )));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn summary_stats_group_owner_template_scope_and_staleness() {
        let stale_due =
            (Utc::now() - Duration::days(1)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let fresh_due =
            (Utc::now() + Duration::days(7)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let entries = vec![
            SearchEntryRecord {
                lensmap: "demo/lensmap.json".to_string(),
                file: "src/api/app.rs".to_string(),
                ref_id: "ABCDEF-1".to_string(),
                kind: Some("doc".to_string()),
                title: Some("Architecture note".to_string()),
                owner: Some("platform".to_string()),
                author: Some("jay".to_string()),
                scope: Some("src/api".to_string()),
                template: Some("architecture".to_string()),
                review_status: Some("approved".to_string()),
                review_due_at: Some(stale_due),
                updated_at: Some(now_iso()),
                tags: vec!["architecture".to_string()],
                ..SearchEntryRecord::default()
            },
            SearchEntryRecord {
                lensmap: "demo/lensmap.json".to_string(),
                file: "src/ui/view.rs".to_string(),
                ref_id: "FEDCBA-2".to_string(),
                kind: Some("todo".to_string()),
                text: Some("Follow up".to_string()),
                scope: Some("src/ui".to_string()),
                template: Some("todo".to_string()),
                review_status: Some("in_review".to_string()),
                review_due_at: Some(fresh_due),
                updated_at: Some(now_iso()),
                tags: vec!["todo".to_string()],
                ..SearchEntryRecord::default()
            },
        ];

        let stats = summary_stats(&entries, 30);
        assert_eq!(stats.entry_count, 2);
        assert_eq!(stats.files_with_notes, 2);
        assert_eq!(stats.stale_entries, 1);
        assert_eq!(stats.by_owner.get("platform"), Some(&1));
        assert_eq!(stats.by_owner.get("unassigned"), Some(&1));
        assert_eq!(stats.by_template.get("architecture"), Some(&1));
        assert_eq!(stats.by_template.get("todo"), Some(&1));
        assert_eq!(stats.by_scope.get("src/api"), Some(&1));
        assert_eq!(stats.by_scope.get("src/ui"), Some(&1));
        assert_eq!(stats.by_directory.get("src/api"), Some(&1));
        assert_eq!(stats.by_directory.get("src/ui"), Some(&1));
    }
}
