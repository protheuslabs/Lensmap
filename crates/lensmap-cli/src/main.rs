use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
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
const SUPPORTED_EXTS: &[&str] = &[".js", ".ts", ".tsx", ".jsx", ".mjs", ".cjs", ".py", ".rs"];
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
    line_anchor: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line_symbol: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fingerprint: Option<String>,
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

#[derive(Clone, Debug)]
struct FunctionHit {
    line_index: usize,
    symbol: String,
    indent: String,
    fingerprint: String,
}

#[derive(Clone, Debug)]
struct AnchorLineMatch {
    id: String,
    marker: String,
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

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn emit(payload: Value, code: i32) -> ! {
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

fn make_lensmap_doc(mode: &str, covers: Vec<String>) -> LensMapDoc {
    let ts = now_iso();
    let mut uniq = BTreeMap::new();
    for c in covers {
        let v = c.trim();
        if !v.is_empty() {
            uniq.insert(v.to_string(), true);
        }
    }
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
        metadata: Map::new(),
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
    let mut hint =
        String::from("LensMap file was not found. Run init first or pass a real --lensmap path.");
    if placeholder {
        hint = String::from(
            "You passed a placeholder path literally (path/to/...). Replace it with a real path.",
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

fn detect_functions(lines: &[String], abs_file: &Path) -> Vec<FunctionHit> {
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
        }

        if let Some(symbol) = symbol {
            let indent = line
                .chars()
                .take_while(|c| c.is_whitespace())
                .collect::<String>();
            let normalized = line.split_whitespace().collect::<Vec<_>>().join(" ");
            let fingerprint = hash_text(&normalized);
            out.push(FunctionHit {
                line_index: idx,
                symbol,
                indent,
                fingerprint,
            });
        }
    }

    out
}

fn parse_anchor_line(line: &str) -> Option<AnchorLineMatch> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"^\s*(//|#)\s*@lensmap-anchor\s+([A-Fa-f0-9]{6,16})\b").unwrap()
    });
    let cap = re.captures(line)?;
    Some(AnchorLineMatch {
        marker: cap.get(1)?.as_str().to_string(),
        id: cap.get(2)?.as_str().to_uppercase(),
    })
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
) -> Vec<(String, String, usize)> {
    let mut out = vec![];
    for (i, line) in lines.iter().enumerate() {
        if let Some(m) = anchor_match(line, expected_marker) {
            out.push((m.id, m.marker, i));
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
) -> Option<(String, String, usize)> {
    let mut i = fn_line;
    while i > 0 {
        i -= 1;
        let trimmed = lines[i].trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(m) = anchor_match(&lines[i], None) {
            return Some((m.id, m.marker, i));
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

    for (id, _marker, line_idx) in anchors {
        let fn_hit = functions.iter().find(|f| f.line_index > line_idx);
        out.push(AnchorRecord {
            id,
            file: rel.clone(),
            symbol: fn_hit.map(|f| f.symbol.clone()),
            line_anchor: Some(line_idx + 1),
            line_symbol: fn_hit.map(|f| f.line_index + 1),
            fingerprint: fn_hit.map(|f| f.fingerprint.clone()),
            updated_at: Some(now_iso()),
        });
    }

    out
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

fn find_latest_anchor_for_line(
    nodes: &[(String, String, usize)],
    line_idx: usize,
) -> Option<(String, usize)> {
    let mut out: Option<(String, usize)> = None;
    for (id, _marker, idx) in nodes {
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
                "hint": "No source files matched covers/files. Pass a real --covers value or run init first.",
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

        let mut functions = detect_functions(&lines, &abs);
        let mut added = 0usize;

        let mut i = 0usize;
        while i < functions.len() {
            let fn_hit = functions[i].clone();
            if let Some((id, marker, line_idx)) =
                find_anchor_before_function(&lines, fn_hit.line_index, &style)
            {
                existing_ids.insert(id.clone());
                if marker != style.line {
                    let indent = lines[line_idx]
                        .chars()
                        .take_while(|c| c.is_whitespace())
                        .collect::<String>();
                    lines[line_idx] = make_anchor_line(&indent, style.line, &id);
                }
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
            lines.insert(insert_at, make_anchor_line(&fn_hit.indent, style.line, &id));
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
                "hint": "No source files matched covers/files. Pass a real --covers value or run init first.",
                "examples": quickstart_examples(),
            }),
            1,
        );
    }

    let mut existing_entry_keys = HashSet::new();
    for e in &doc.entries {
        let key = format!(
            "{}::{}::{}::{}",
            e.file,
            e.ref_id,
            e.kind.clone().unwrap_or_default(),
            e.text.clone().unwrap_or_default()
        );
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

        let blocks = collect_comment_blocks(&lines, &abs);
        let mut extracted = 0usize;
        let mut changed = false;

        for block in blocks {
            let latest = find_latest_anchor_for_line(&anchor_nodes, block.start);
            if latest.is_none() {
                continue;
            }
            let (anchor_id, anchor_line) = latest.unwrap();
            if block.start < anchor_line || block.end < block.start {
                continue;
            }
            let start_offset = block.start - anchor_line;
            let end_offset = block.end - anchor_line;
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

            let key = format!(
                "{}::{}::{}::{}",
                entry.file,
                entry.ref_id,
                entry.kind.clone().unwrap_or_default(),
                entry.text.clone().unwrap_or_default()
            );
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

fn cmd_validate(root: &Path, args: &ParsedArgs) {
    let lensmap_path = resolve_lensmap_path(root, args, None);
    if !lensmap_path.exists() {
        emit(
            lensmap_missing_payload(root, "validate", &lensmap_path, args),
            1,
        );
    }

    let doc = load_doc(&lensmap_path, "group");
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
        let mut anchor_line = anchor.line_anchor.unwrap_or(0).saturating_sub(1);

        let found_declared = if anchor_line < lines.len() {
            if let Some(m) = anchor_match(&lines[anchor_line], None) {
                m.id == id
            } else {
                false
            }
        } else {
            false
        };

        if !found_declared {
            let mut found_idx: Option<usize> = None;
            for (idx, line) in lines.iter().enumerate() {
                if let Some(m) = anchor_match(line, None) {
                    if m.id == id {
                        found_idx = Some(idx);
                        break;
                    }
                }
            }
            if let Some(found) = found_idx {
                warnings.push(format!(
                    "anchor_line_drift:{}:{}->{}",
                    id,
                    anchor.line_anchor.unwrap_or(0),
                    found + 1
                ));
                anchor_line = found;
            } else {
                errors.push(format!("anchor_not_found_in_file:{}:{}", id, anchor.file));
                continue;
            }
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

        let fns = detect_functions(&lines, &abs);
        if let Some(fn_hit) = fns.iter().find(|f| f.line_index > anchor_line) {
            if let Some(fp) = &anchor.fingerprint {
                if fp != &fn_hit.fingerprint {
                    warnings.push(format!("fingerprint_drift:{}", id));
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
        let anchor_line = anchor.line_anchor.unwrap_or(0).saturating_sub(1);
        let start_line = anchor_line + parsed.start;
        let end_line = anchor_line + parsed.end;

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
    let mut unresolved: Vec<Value> = vec![];
    let mut resolved = 0usize;
    let mut inserted = 0usize;

    let mut file_to_indices: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, a) in doc.anchors.iter().enumerate() {
        file_to_indices.entry(a.file.clone()).or_default().push(idx);
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

        for idx in indices {
            let id = doc.anchors[idx].id.to_uppercase();
            if id.is_empty() {
                unresolved.push(json!({"id": "<empty>", "reason": "invalid_id", "file": file}));
                continue;
            }

            let nodes = collect_anchor_nodes(&lines, Some(style.line));
            if let Some((_found_id, _marker, line_idx)) =
                nodes.iter().find(|(nid, _, _)| nid == &id)
            {
                let fns = detect_functions(&lines, &abs);
                let fn_hit = fns.iter().find(|f| f.line_index > *line_idx);
                doc.anchors[idx].line_anchor = Some(*line_idx + 1);
                doc.anchors[idx].line_symbol = fn_hit.map(|f| f.line_index + 1);
                doc.anchors[idx].symbol = fn_hit.map(|f| f.symbol.clone());
                doc.anchors[idx].fingerprint = fn_hit.map(|f| f.fingerprint.clone());
                doc.anchors[idx].updated_at = Some(now_iso());
                resolved += 1;
                continue;
            }

            let fns = detect_functions(&lines, &abs);
            let mut candidate: Option<FunctionHit> = None;
            if let Some(symbol) = &doc.anchors[idx].symbol {
                let by_symbol = fns
                    .iter()
                    .filter(|f| &f.symbol == symbol)
                    .cloned()
                    .collect::<Vec<_>>();
                if by_symbol.len() == 1 {
                    candidate = by_symbol.first().cloned();
                } else if by_symbol.len() > 1 {
                    if let Some(fp) = &doc.anchors[idx].fingerprint {
                        let by_fp = by_symbol
                            .into_iter()
                            .filter(|f| &f.fingerprint == fp)
                            .collect::<Vec<_>>();
                        if by_fp.len() == 1 {
                            candidate = by_fp.first().cloned();
                        }
                    }
                }
            }
            if candidate.is_none() {
                if let Some(fp) = &doc.anchors[idx].fingerprint {
                    let by_fp = fns
                        .iter()
                        .filter(|f| &f.fingerprint == fp)
                        .cloned()
                        .collect::<Vec<_>>();
                    if by_fp.len() == 1 {
                        candidate = by_fp.first().cloned();
                    }
                }
            }

            if candidate.is_none() {
                unresolved.push(json!({
                    "id": id,
                    "reason": "symbol_or_fingerprint_not_found",
                    "file": file,
                }));
                continue;
            }

            let fn_hit = candidate.unwrap();
            if let Some((existing, _marker, _line_idx)) =
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
            } else {
                lines.insert(
                    fn_hit.line_index,
                    make_anchor_line(&fn_hit.indent, style.line, &id),
                );
                file_changed = true;
                inserted += 1;
            }

            let refreshed_nodes = collect_anchor_nodes(&lines, Some(style.line));
            if let Some((_fid, _m, line_idx)) =
                refreshed_nodes.iter().find(|(nid, _, _)| nid == &id)
            {
                let refreshed_fns = detect_functions(&lines, &abs);
                let next_fn = refreshed_fns.iter().find(|f| f.line_index > *line_idx);
                doc.anchors[idx].line_anchor = Some(*line_idx + 1);
                doc.anchors[idx].line_symbol = next_fn.map(|f| f.line_index + 1);
                doc.anchors[idx].symbol = next_fn.map(|f| f.symbol.clone());
                doc.anchors[idx].fingerprint = next_fn.map(|f| f.fingerprint.clone());
                doc.anchors[idx].updated_at = Some(now_iso());
                resolved += 1;
            } else {
                unresolved.push(json!({
                    "id": id,
                    "reason": "inserted_but_not_resolved",
                    "file": file,
                }));
            }
        }

        if file_changed && !dry_run {
            let _ = fs::write(&abs, join_lines(&lines));
        }
    }

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
        "resolved": resolved,
        "inserted": inserted,
        "unresolved": unresolved,
        "ts": now_iso(),
    });
    append_history(root, &out);
    emit(out, if ok { 0 } else { 1 });
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
        root.join("local/state/ops/lensmap/rendered.md")
    };

    let mut files: BTreeMap<String, bool> = BTreeMap::new();
    for c in &doc.covers {
        for f in resolve_covers_to_files(root, std::slice::from_ref(c)) {
            files.insert(f, true);
        }
    }
    for a in &doc.anchors {
        if !a.file.is_empty() {
            files.insert(a.file.clone(), true);
        }
    }
    for e in &doc.entries {
        if !e.file.is_empty() {
            files.insert(e.file.clone(), true);
        }
    }
    let files_rendered = files.len();

    let mut anchor_map = HashMap::new();
    for a in &doc.anchors {
        anchor_map.insert(a.id.to_uppercase(), a.clone());
    }

    let mut lines = vec![];
    lines.push("# LensMap Render".to_string());
    lines.push(String::new());
    lines.push(format!(
        "- Source: `{}`",
        normalize_relative(root, &lensmap_path)
    ));
    lines.push(format!("- Generated: {}", now_iso()));
    lines.push(String::new());

    for (rel, _) in files {
        let abs = resolve_from_root(root, &rel);
        if !is_within_root(root, &abs) || !abs.exists() {
            continue;
        }
        let file_content = fs::read_to_string(&abs).unwrap_or_default();
        let file_lines = split_lines(&file_content);
        let lang = ext_of(&abs).trim_start_matches('.').to_string();

        let mut file_anchors = doc
            .anchors
            .iter()
            .filter(|a| a.file == rel)
            .cloned()
            .collect::<Vec<_>>();
        file_anchors.sort_by(|a, b| a.line_anchor.unwrap_or(0).cmp(&b.line_anchor.unwrap_or(0)));

        let mut file_entries = vec![];
        for e in doc.entries.iter().filter(|e| e.file == rel) {
            let parsed = parse_ref(&e.ref_id);
            if parsed.is_none() {
                file_entries.push((e.clone(), None, None));
                continue;
            }
            let parsed = parsed.unwrap();
            if let Some(anchor) = anchor_map.get(&parsed.anchor_id) {
                let anchor_line = anchor.line_anchor.unwrap_or(1).saturating_sub(1);
                let start = anchor_line + parsed.start + 1;
                let end = anchor_line + parsed.end + 1;
                file_entries.push((e.clone(), Some(start), Some(end)));
            } else {
                file_entries.push((e.clone(), None, None));
            }
        }
        file_entries.sort_by(|a, b| a.1.unwrap_or(0).cmp(&b.1.unwrap_or(0)));

        lines.push(format!("## {}", rel));
        lines.push(String::new());
        lines.push("### Anchors".to_string());
        if file_anchors.is_empty() {
            lines.push("- none".to_string());
        } else {
            for a in &file_anchors {
                let mut row = format!("- {} line {}", a.id, a.line_anchor.unwrap_or(0));
                if let Some(symbol) = &a.symbol {
                    row.push_str(&format!(" symbol=`{}`", symbol));
                }
                if let Some(fp) = &a.fingerprint {
                    row.push_str(&format!(" fingerprint=`{}`", fp));
                }
                lines.push(row);
            }
        }
        lines.push(String::new());

        lines.push("### Entries".to_string());
        if file_entries.is_empty() {
            lines.push("- none".to_string());
            lines.push(String::new());
            continue;
        }

        for (entry, start, end) in file_entries {
            let label = if let Some(s) = start {
                if let Some(e) = end {
                    if e != s {
                        format!("line {}-{}", s, e)
                    } else {
                        format!("line {}", s)
                    }
                } else {
                    format!("line {}", s)
                }
            } else {
                "line ?".to_string()
            };

            lines.push(format!(
                "- [{}] ({}) {}: {}",
                entry.ref_id,
                label,
                entry.kind.unwrap_or_else(|| "comment".to_string()),
                entry.text.unwrap_or_default().replace('\n', " ").trim()
            ));

            if let Some(sline) = start {
                let eline = end.unwrap_or(sline);
                let start_ctx = sline.saturating_sub(1).max(1);
                let end_ctx = (eline + 1).min(file_lines.len());
                lines.push(String::new());
                lines.push(format!(
                    "```{}",
                    if lang.is_empty() { "text" } else { &lang }
                ));
                for l in start_ctx..=end_ctx {
                    let body = file_lines.get(l - 1).cloned().unwrap_or_default();
                    lines.push(format!("{:>4} | {}", l, body));
                }
                lines.push("```".to_string());
                lines.push(String::new());
            }
        }

        lines.push(String::new());
    }

    ensure_dir(&out_path);
    let _ = fs::write(&out_path, format!("{}\n", lines.join("\n")));

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "render",
        "lensmap": normalize_relative(root, &lensmap_path),
        "output": normalize_relative(root, &out_path),
        "files_rendered": files_rendered,
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
    let before_anchors = doc.anchors.len();
    let before_entries = doc.entries.len();

    let mut anchor_map = BTreeMap::new();
    for a in doc.anchors {
        if !a.id.trim().is_empty() {
            anchor_map.insert(a.id.to_uppercase(), a);
        }
    }
    let mut entry_map = BTreeMap::new();
    for e in doc.entries {
        let key = format!(
            "{}::{}::{}::{}",
            e.file,
            e.ref_id,
            e.kind.clone().unwrap_or_default(),
            e.text.clone().unwrap_or_default()
        );
        entry_map.insert(key, e);
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
    save_doc(&lensmap_path, doc.clone());

    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "simplify",
        "lensmap": normalize_relative(root, &lensmap_path),
        "removed_anchors": before_anchors.saturating_sub(doc.anchors.len()),
        "removed_entries": before_entries.saturating_sub(doc.entries.len()),
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

fn cmd_sync(root: &Path, args: &ParsedArgs) {
    let to = args
        .get("to")
        .or_else(|| args.get("path"))
        .unwrap_or("")
        .trim()
        .to_string();
    if to.is_empty() {
        emit(json!({"ok": false, "error": "to_required"}), 1);
    }
    let out = json!({
        "ok": true,
        "type": "lensmap",
        "action": "sync",
        "to": to,
        "ts": now_iso(),
        "diff_receipt": format!("sync_{}", Utc::now().timestamp_millis()),
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
        Some(json!({
            "lensmap": normalize_relative(root, &lensmap_path),
            "mode": doc.mode,
            "covers": doc.covers.len(),
            "anchors": doc.anchors.len(),
            "entries": doc.entries.len(),
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

fn usage() {
    println!(
        "lensmap init <project> [--mode=group|file] [--covers=a,b] [--file=path] [--lensmap=path]"
    );
    println!("lensmap template add <type>");
    println!("lensmap scan [--lensmap=path] [--covers=a,b] [--dry-run]");
    println!("lensmap extract-comments [--lensmap=path] [--covers=a,b] [--dry-run]");
    println!("lensmap validate [--lensmap=path]");
    println!("lensmap reanchor [--lensmap=path] [--dry-run]");
    println!("lensmap render [--lensmap=path] [--out=path]");
    println!("lensmap simplify [--lensmap=path]");
    println!("lensmap polish");
    println!("lensmap import --from=<path>");
    println!("lensmap sync --to=<path>");
    println!("lensmap expose --name=<lens_name>");
    println!("lensmap status [--lensmap=path]");
    println!();
    println!("Quickstart:");
    println!("  lensmap init demo --mode=group --covers=demo/src");
    println!("  lensmap scan --lensmap=demo/lensmap.json");
    println!("  lensmap extract-comments --lensmap=demo/lensmap.json");
    println!("  lensmap validate --lensmap=demo/lensmap.json");
    println!("  lensmap render --lensmap=demo/lensmap.json --out=demo/render.md");
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
        "template" if args.positional.get(1).map(String::as_str) == Some("add") => {
            cmd_template_add(&root, &args)
        }
        "scan" => cmd_scan(&root, &args),
        "extract-comments" => cmd_extract_comments(&root, &args),
        "validate" => cmd_validate(&root, &args),
        "reanchor" => cmd_reanchor(&root, &args),
        "render" => cmd_render(&root, &args),
        "simplify" => cmd_simplify(&root, &args),
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
