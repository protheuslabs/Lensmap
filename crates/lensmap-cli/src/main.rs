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
) -> Result<(AnchorRecord, bool), String> {
    let abs = resolve_from_root(root, file);
    if !is_within_root(root, &abs) {
        return Err(format!("security_entry_outside_root:{}", file));
    }
    if !abs.exists() {
        return Err(format!("file_missing:{}", file));
    }

    let style = comment_style_for(&abs);
    let mut lines = split_lines(&fs::read_to_string(&abs).unwrap_or_default());
    let functions = detect_functions(&lines, &abs);
    let rel = normalize_relative(root, &abs);
    let mut candidates = functions
        .iter()
        .filter(|f| f.symbol == symbol)
        .cloned()
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Err(format!("symbol_not_found:{}:{}", rel, symbol));
    }
    if candidates.len() > 1 {
        let tracked = doc
            .anchors
            .iter()
            .find(|anchor| anchor.file == rel && anchor.symbol.as_deref() == Some(symbol));
        if let Some(anchor) = tracked {
            if let Some(fp) = &anchor.fingerprint {
                candidates = candidates
                    .into_iter()
                    .filter(|f| &f.fingerprint == fp)
                    .collect::<Vec<_>>();
            }
        }
    }
    if candidates.len() != 1 {
        return Err(format!("symbol_ambiguous:{}:{}", rel, symbol));
    }
    let fn_hit = candidates.remove(0);

    if let Some((id, marker, line_idx)) =
        find_anchor_before_function(&lines, fn_hit.line_index, &style)
    {
        if marker != style.line {
            let indent = lines[line_idx]
                .chars()
                .take_while(|c| c.is_whitespace())
                .collect::<String>();
            lines[line_idx] = make_anchor_line(&indent, style.line, &id);
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
                .map(|(id, _, _)| id),
        )
        .collect();
    let id = generate_anchor_id(&existing_ids);
    let insert_at = compute_anchor_insert_index(&lines, fn_hit.line_index, &style);
    lines.insert(insert_at, make_anchor_line(&fn_hit.indent, style.line, &id));
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
    if let Some((_nid, _marker, line_idx)) = nodes.iter().find(|(nid, _, _)| nid == &id) {
        let fn_hit = fns.iter().find(|f| f.line_index > *line_idx).cloned();
        return AnchorResolution {
            anchor_line_index: Some(*line_idx),
            function_hit: fn_hit,
            strategy: "anchor_id".to_string(),
            anchor_found_in_source: true,
        };
    }

    let mut candidate: Option<FunctionHit> = None;
    let mut strategy = "unresolved".to_string();

    if let Some(symbol) = &anchor.symbol {
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
        if let Some((_existing, _marker, line_idx)) =
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

    if text.is_empty() {
        emit(
            json!({
                "ok": false,
                "type": "lensmap",
                "action": "annotate",
                "error": "annotate_requires_text",
                "hint": "Use --text=<comment text> with either --ref=<HEX-start[-end]> or --file + --symbol.",
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
                    "hint": "Use --ref=<HEX-start[-end]> or --file=<path> --symbol=<name> [--offset=1] [--end-offset=N].",
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
        let (anchor, created) =
            ensure_anchor_for_symbol(root, &mut doc, &requested_file, &requested_symbol)
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
                    "hint": "Run scan first so the anchor exists in lensmap.json, or annotate with --file + --symbol.",
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
                "hint": "Pass --file=<path> or annotate against an anchor with a known file.",
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
                "hint": "No files to merge. Ensure covers/entries reference real files.",
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
                "hint": "Create a lensmap first or pass --lensmaps=<file1,file2>.",
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
                "hint": "Run lensmap package first.",
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
                    let prompt = format!(
                        "Missing dir for {} ({}). Enter new directory path, or 'skip': ",
                        item.original_path, parent_rel
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
        let anchor_line = if let Some(anchor_line) = resolution.anchor_line_index {
            anchor_line
        } else {
            errors.push(format!("anchor_unresolved:{}:{}", id, anchor.file));
            continue;
        };

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

        if let Some(fn_hit) = resolution
            .function_hit
            .or_else(|| fns.iter().find(|f| f.line_index > anchor_line).cloned())
        {
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
        let fns = detect_functions(&lines, &abs);
        let resolution = resolve_anchor_in_lines(anchor, &lines, &fns, &style);
        let anchor_line = if let Some(anchor_line) = resolution.anchor_line_index {
            anchor_line
        } else {
            warnings.push(format!("entry_anchor_unresolved:{}", entry.ref_id));
            continue;
        };
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

fn reanchor_doc(root: &Path, doc: &mut LensMapDoc, dry_run: bool) -> (usize, usize, Vec<Value>) {
    let mut unresolved: Vec<Value> = vec![];
    let mut resolved = 0usize;
    let mut inserted = 0usize;

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
                doc.anchors[idx].fingerprint = resolution
                    .function_hit
                    .as_ref()
                    .map(|f| f.fingerprint.clone());
                doc.anchors[idx].updated_at = Some(now_iso());
                resolved += 1;
                continue;
            }

            let fn_hit = if let Some(fn_hit) = resolution.function_hit.clone() {
                fn_hit
            } else {
                unresolved.push(json!({
                    "id": id,
                    "reason": format!("{}_not_found", resolution.strategy),
                    "file": file,
                }));
                continue;
            };

            if let Some((existing, marker, line_idx)) =
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
                    let indent = lines[line_idx]
                        .chars()
                        .take_while(|c| c.is_whitespace())
                        .collect::<String>();
                    lines[line_idx] = make_anchor_line(&indent, style.line, &id);
                    file_changed = true;
                }
            } else {
                let insert_at = resolution.anchor_line_index.unwrap_or_else(|| {
                    compute_anchor_insert_index(&lines, fn_hit.line_index, &style)
                });
                lines.insert(insert_at, make_anchor_line(&fn_hit.indent, style.line, &id));
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
            doc.anchors[idx].fingerprint = refreshed
                .function_hit
                .as_ref()
                .map(|f| f.fingerprint.clone());
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
    let (resolved, inserted, unresolved) = reanchor_doc(root, &mut doc, dry_run);

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
        "- Source: `{}`",
        normalize_relative(root, lensmap_path)
    ));
    lines.push(format!("- Generated: {}", now_iso()));
    lines.push(format!(
        "- Positioning: {}",
        doc.metadata
            .get("positioning")
            .and_then(Value::as_str)
            .unwrap_or("external-doc-layer")
    ));
    if let Some(file) = file_filter {
        lines.push(format!("- File filter: `{}`", file));
    }
    if let Some(symbol) = symbol_filter {
        lines.push(format!("- Symbol filter: `{}`", symbol));
    }
    if let Some(ref_id) = &ref_filter {
        lines.push(format!("- Ref filter: `{}`", ref_id));
    }
    if let Some(kind) = kind_filter {
        lines.push(format!("- Kind filter: `{}`", kind));
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
                    return anchor.symbol.as_deref() == Some(symbol);
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
                if anchor.symbol.as_deref() != Some(symbol) {
                    continue;
                }
            }
            let resolution = resolutions
                .entry(parsed.anchor_id.to_uppercase())
                .or_insert_with(|| {
                    resolve_anchor_in_lines(anchor, &file_lines, &functions, &style)
                });
            let start = resolution
                .anchor_line_index
                .map(|idx| idx + parsed.start + 1);
            let end = resolution.anchor_line_index.map(|idx| idx + parsed.end + 1);
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
        lines.push("### Anchors".to_string());
        if file_anchors.is_empty() {
            lines.push("- none".to_string());
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

        lines.push("### Entries".to_string());
        if file_entries.is_empty() {
            lines.push("- none".to_string());
            lines.push(String::new());
            continue;
        }

        for (entry, anchor, resolution, start, end) in file_entries {
            entries_rendered += 1;
            let label = if let Some(start_line) = start {
                if let Some(end_line) = end {
                    if end_line != start_line {
                        format!("line {}-{}", start_line, end_line)
                    } else {
                        format!("line {}", start_line)
                    }
                } else {
                    format!("line {}", start_line)
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
            lines.push(format!(
                "  anchor=`{}` symbol=`{}` resolve=`{}`",
                anchor.id,
                anchor.symbol.unwrap_or_else(|| "?".to_string()),
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
        "LensMap Render",
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
        "LensMap View",
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
    let (resolved, inserted, unresolved) = reanchor_doc(root, &mut doc, false);
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
        "LensMap Render",
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
    println!("lensmap annotate --lensmap=path (--ref=<HEX-start[-end]> | --file=path --symbol=name [--offset=N] [--end-offset=M]) --text=<text> [--kind=comment|doc|todo|decision]");
    println!("lensmap template add <type>");
    println!("lensmap scan [--lensmap=path] [--covers=a,b] [--anchor-mode=smart|all] [--dry-run]");
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
    println!("lensmap reanchor [--lensmap=path] [--dry-run]");
    println!("lensmap render [--lensmap=path] [--out=path]  # defaults to sibling .md");
    println!("lensmap parse [--lensmap=path] [--out=path]  # alias of render");
    println!("lensmap show [--lensmap=path] [--file=path] [--symbol=name] [--ref=HEX-start[-end]] [--kind=comment|doc|todo|decision] [--out=path]");
    println!("lensmap simplify [--lensmap=path]");
    println!("lensmap polish");
    println!("lensmap import --from=<path>");
    println!("lensmap sync [--lensmap=path] [--to=path]  # reanchor + simplify + render");
    println!("lensmap expose --name=<lens_name>");
    println!("lensmap status [--lensmap=path]");
    println!();
    println!("Quickstart:");
    println!("  lensmap init demo --mode=group --covers=demo/src");
    println!("  lensmap scan --lensmap=demo/lensmap.json --anchor-mode=smart");
    println!("  lensmap extract-comments --lensmap=demo/lensmap.json");
    println!("  lensmap annotate --lensmap=demo/lensmap.json --file=demo/src/app.ts --symbol=run --offset=1 --text=\"why this exists\"");
    println!("  lensmap show --lensmap=demo/lensmap.json --file=demo/src/app.ts");
    println!("  lensmap merge --lensmap=demo/lensmap.json");
    println!("  lensmap unmerge --lensmap=demo/lensmap.json");
    println!("  lensmap package --bundle-dir=.lenspack");
    println!("  lensmap unpackage --bundle-dir=.lenspack --on-missing=prompt");
    println!("  lensmap sync --lensmap=demo/lensmap.json");
    println!("  lensmap validate --lensmap=demo/lensmap.json");
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
