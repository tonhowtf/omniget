use crate::core::secure_files::{Identity, Root, ScanFilter};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_FILE: u64 = 1024 * 1024;
const MAX_TREE: usize = 5000;
const MAX_REPLY: usize = 64 * 1024;
const SEARCH_BYTES: u64 = 16 * 1024 * 1024;
const MAX_EDITS: usize = 64;
const MAX_HUNKS: usize = 64;
pub const CREATE_ONLY_REQUIRED: &str = "FS_WRITE_CREATE_ONLY_USE_FS_EDIT";
pub const EXPECTED_SHA256_REQUIRED: &str = "EXPECTED_SHA256_REQUIRED";
fn denied(_: std::io::Error) -> String {
    "FILE_ACCESS_DENIED_OR_CHANGED".into()
}
fn invalid() -> String {
    "INVALID_FILE_ARGUMENTS".into()
}
const PROTECTED_NAMES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".ssh",
    ".aws",
    ".azure",
    ".config",
    ".codex",
    ".claude",
    ".omniget",
    ".env",
    "credentials",
    "credentials.json",
    "credentials.toml",
    "id_rsa",
    "id_ed25519",
];
const HEAVY_DIRS: &[&str] = &["node_modules", "target"];
fn ignorable(c: char) -> bool {
    matches!(c as u32, 0x00AD | 0x034F | 0x061C | 0x115F | 0x1160 | 0x17B4 | 0x17B5 | 0x180B..=0x180F | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0x3164 | 0xFE00..=0xFE0F | 0xFEFF | 0xFFA0 | 0xFFF0..=0xFFF8 | 0xE0000..=0xE0FFF)
}
pub fn fold(name: &str) -> String {
    use unicode_normalization::char::is_combining_mark;
    use unicode_normalization::UnicodeNormalization;
    let base: String = name
        .nfkd()
        .filter(|c| !is_combining_mark(*c) && !ignorable(*c))
        .collect();
    let folded: String = base
        .chars()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .collect();
    folded
        .nfkc()
        .filter(|c| !is_combining_mark(*c) && !ignorable(*c))
        .collect()
}
fn protected_name(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return true;
    };
    let name = fold(name);
    PROTECTED_NAMES.contains(&name.as_str())
        || name.starts_with(".env.")
        || name.starts_with(".omniget-")
}
fn protected(path: &Path) -> bool {
    path.components().any(|part| match part {
        std::path::Component::Normal(name) => protected_name(name),
        _ => false,
    })
}
fn aliases_protected(root: &Root, path: &Path) -> bool {
    let mut prefix = PathBuf::new();
    for part in path.components() {
        let std::path::Component::Normal(name) = part else {
            return false;
        };
        let parent = prefix.clone();
        prefix.push(name);
        let Ok(id) = root.identity_of(&prefix) else {
            return false;
        };
        for literal in PROTECTED_NAMES {
            if parent.join(literal) == prefix {
                continue;
            }
            if root
                .identity_of(&parent.join(literal))
                .is_ok_and(|p| p == id)
            {
                return true;
            }
        }
    }
    false
}
fn guard(root: &Root, path: &str) -> Result<(), String> {
    if path != "." && aliases_protected(root, Path::new(path)) {
        return Err("PROTECTED_WORKSPACE_PATH".into());
    }
    Ok(())
}
fn scan_filter(path: &Path) -> ScanFilter {
    let Some(name) = path.file_name() else {
        return ScanFilter::Walk;
    };
    if protected_name(name) {
        ScanFilter::Hide
    } else if name.to_str().is_some_and(|n| HEAVY_DIRS.contains(&n)) {
        ScanFilter::ListOnly
    } else {
        ScanFilter::Walk
    }
}
fn tree_error(e: std::io::Error) -> String {
    let text = e.to_string();
    if text.starts_with("FILE_TREE_") {
        text
    } else {
        denied(e)
    }
}
fn skipped_json(skipped: &[(PathBuf, &'static str)]) -> Value {
    let list: Vec<Value> = skipped
        .iter()
        .take(50)
        .map(|(p, why)| json!({"path": p.to_string_lossy(), "reason": why}))
        .collect();
    json!({"entries": list, "count": skipped.len(), "hint": "symlinks, hard links and special files are not followed; ignored folders (node_modules, target) are listed but not descended: pass their path to look inside"})
}
fn relative(path: &str, directory: bool) -> Result<&Path, String> {
    if directory && path == "." {
        return Ok(Path::new(path));
    }
    if path.is_empty()
        || path.len() > 1024
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || path
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == "..")
    {
        return Err(invalid());
    }
    if protected(Path::new(path)) {
        return Err("PROTECTED_WORKSPACE_PATH".into());
    }
    Ok(Path::new(path))
}
pub fn validate_path(path: &str) -> Result<(), String> {
    relative(path, false).map(|_| ())
}

fn parse<T: serde::de::DeserializeOwned>(a: &Value) -> Result<T, String> {
    if serde_json::to_vec(a).map_err(|_| invalid())?.len() > MAX_FILE as usize + 8192 {
        return Err("FILE_ARGUMENT_LIMIT".into());
    }
    serde_json::from_value(a.clone()).map_err(|_| invalid())
}
fn bounded(v: Value) -> Result<Value, String> {
    if serde_json::to_vec(&v).map_err(|_| invalid())?.len() > MAX_REPLY {
        Err("FILE_RESPONSE_LIMIT".into())
    } else {
        Ok(v)
    }
}
fn dot() -> String {
    ".".into()
}
fn one() -> usize {
    1
}
fn lines() -> usize {
    200
}
fn two() -> usize {
    2
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    path: String,
    #[serde(default = "one")]
    offset: usize,
    #[serde(default = "lines")]
    limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    #[serde(default = "dot")]
    path: String,
    #[serde(default = "two")]
    depth: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GlobArgs {
    #[serde(default = "dot")]
    path: String,
    pattern: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GrepArgs {
    #[serde(default = "dot")]
    path: String,
    pattern: String,
    #[serde(default)]
    literal_text: bool,
    #[serde(default)]
    include: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteArgs {
    path: String,
    content: String,
    #[serde(default)]
    create_only: bool,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditOp {
    pub old_string: String,
    pub new_string: String,
    #[serde(default)]
    pub replace_all: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditArgs {
    path: String,
    #[serde(default, alias = "expected_digest")]
    expected_sha256: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    edits: Option<Vec<EditOp>>,
    #[serde(default)]
    old_string: Option<String>,
    #[serde(default)]
    new_string: Option<String>,
    #[serde(default)]
    replace_all: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchArgs {
    patch: String,
    #[serde(default, alias = "expected_digest")]
    expected_sha256: Option<String>,
}
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PatchKind {
    Add,
    Update,
}
#[derive(Debug, Serialize)]
pub struct PreparedPatch {
    pub path: String,
    pub kind: PatchKind,
    pub content: String,
}

pub struct ExternalFiles {
    workspace: PathBuf,
    identity: Identity,
    staging: PathBuf,
    staging_identity: Identity,
}
impl ExternalFiles {
    pub fn open(
        workspace: &Path,
        expected: &Identity,
        staging: &Path,
        staging_identity: &Identity,
    ) -> Result<Self, String> {
        if staging.starts_with(workspace) || workspace.starts_with(staging) {
            return Err("STAGING_MUST_BE_OUTSIDE_WORKSPACE".into());
        }
        let root = Root::open(workspace, Some(expected)).map_err(denied)?;
        let stage = Root::open(staging, Some(staging_identity)).map_err(denied)?;
        if root.identity().device != stage.identity().device {
            return Err("STAGING_FILESYSTEM_MISMATCH".into());
        }
        Ok(Self {
            workspace: workspace.into(),
            identity: expected.clone(),
            staging: staging.into(),
            staging_identity: staging_identity.clone(),
        })
    }
    fn roots(&self) -> Result<(Root, Root), String> {
        Ok((
            Root::open(&self.workspace, Some(&self.identity)).map_err(denied)?,
            Root::open(&self.staging, Some(&self.staging_identity)).map_err(denied)?,
        ))
    }
    pub fn execute(&self, tool: &str, args: &Value) -> Result<Value, String> {
        let (root, staging) = self.roots()?;
        let result = match tool {
            "fs_read" => {
                let a: ReadArgs = parse(args)?;
                relative(&a.path, false)?;
                guard(&root, &a.path)?;
                if a.offset == 0 || a.limit == 0 || a.limit > 1000 {
                    return Err(invalid());
                }
                let (body, digest, bytes) = read_text(&root, Path::new(&a.path))?;
                let total = body.lines().count();
                let mut content = String::new();
                let mut next = a.offset;
                let mut truncated = false;
                for (index, line) in body.lines().enumerate().skip(a.offset - 1).take(a.limit) {
                    let shown: String = line.chars().take(2000).collect();
                    if shown.len() != line.len() {
                        truncated = true;
                    }
                    let row = format!("{:>6}\t{}\n", index + 1, shown);
                    if content.len() + row.len() > 48 * 1024 {
                        truncated = true;
                        break;
                    }
                    content.push_str(&row);
                    next = index + 2;
                }
                json!({"path":a.path,"content":content,"lines":total,"nextOffset":next,"hasMore":next<=total,"truncated":truncated,"digest":digest,"bytes":bytes,"externalContent":true})
            }
            "fs_list" => {
                let a: ListArgs = parse(args)?;
                let base = relative(&a.path, true)?;
                if !(1..=8).contains(&a.depth) {
                    return Err(invalid());
                }
                guard(&root, &a.path)?;
                let scan = root
                    .scan(base, MAX_TREE, &scan_filter)
                    .map_err(tree_error)?;
                let entries = scan.entries;
                let mut out = Vec::new();
                let mut more = false;
                let base_depth = if a.path == "." {
                    0
                } else {
                    base.components().count()
                };
                for (path, directory) in entries {
                    if protected(&path) {
                        continue;
                    }
                    if path.components().count() - base_depth > a.depth {
                        continue;
                    }
                    if out.len() == 500 {
                        more = true;
                        break;
                    }
                    let path = path.to_str().ok_or("NON_UTF8_PATH_UNSUPPORTED")?;
                    out.push(if directory {
                        format!("{path}/")
                    } else {
                        path.to_string()
                    });
                }
                json!({"path":a.path,"entries":out,"truncated":more,"protectedEntriesExcluded":true,"skipped":skipped_json(&scan.skipped)})
            }
            "fs_glob" => {
                let a: GlobArgs = parse(args)?;
                let base = relative(&a.path, true)?;
                let matcher = glob(&a.pattern)?;
                guard(&root, &a.path)?;
                let scan = root
                    .scan(base, MAX_TREE, &scan_filter)
                    .map_err(tree_error)?;
                let entries = scan
                    .entries
                    .iter()
                    .filter(|(_, directory)| !directory)
                    .map(|(p, _)| p.clone());
                let mut files = Vec::new();
                let mut more = false;
                for path in entries {
                    if protected(&path) {
                        continue;
                    }
                    let name = path.to_str().ok_or("NON_UTF8_PATH_UNSUPPORTED")?;
                    let candidate = if a.path == "." {
                        name
                    } else {
                        path.strip_prefix(base)
                            .map_err(|_| invalid())?
                            .to_str()
                            .ok_or("NON_UTF8_PATH_UNSUPPORTED")?
                    };
                    if matcher.is_match(candidate)
                        || (!a.pattern.contains('/')
                            && path
                                .file_name()
                                .and_then(|v| v.to_str())
                                .is_some_and(|v| matcher.is_match(v)))
                    {
                        if files.len() == 500 {
                            more = true;
                            break;
                        }
                        files.push(name.to_string());
                    }
                }
                json!({"pattern":a.pattern,"files":files,"truncated":more,"protectedEntriesExcluded":true,"skipped":skipped_json(&scan.skipped)})
            }
            "fs_grep" => {
                let a: GrepArgs = parse(args)?;
                let base = relative(&a.path, true)?;
                if a.pattern.is_empty() || a.pattern.len() > 512 {
                    return Err(invalid());
                }
                let pattern = if a.literal_text {
                    regex::escape(&a.pattern)
                } else {
                    a.pattern.clone()
                };
                let matcher = regex::RegexBuilder::new(&pattern)
                    .size_limit(256 * 1024)
                    .build()
                    .map_err(|_| "INVALID_SEARCH_PATTERN")?;
                let include = a
                    .include
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(glob)
                    .transpose()?;
                guard(&root, &a.path)?;
                let scan = root
                    .scan(base, MAX_TREE, &scan_filter)
                    .map_err(tree_error)?;
                let mut skipped = scan.skipped.clone();
                let files = scan
                    .entries
                    .into_iter()
                    .filter(|(_, directory)| !directory)
                    .map(|(p, _)| p);
                let mut matches = Vec::new();
                let mut bytes = 0u64;
                let mut truncated = false;
                let mut stopped = None;
                for path in files {
                    if protected(&path) {
                        continue;
                    }
                    let name = path.to_str().ok_or("NON_UTF8_PATH_UNSUPPORTED")?;
                    if include.as_ref().is_some_and(|m| {
                        !m.is_match(name)
                            && !path
                                .file_name()
                                .and_then(|v| v.to_str())
                                .is_some_and(|v| m.is_match(v))
                    }) {
                        continue;
                    }
                    let (body, digest, size) = match read_text(&root, &path) {
                        Ok(v) => v,
                        Err(e) => {
                            skipped.push((path.clone(), skip_reason(&e)));
                            continue;
                        }
                    };
                    bytes = bytes.saturating_add(size);
                    if bytes > SEARCH_BYTES {
                        truncated = true;
                        stopped = Some(format!("SEARCH_BYTE_LIMIT: searched {SEARCH_BYTES} bytes; stopped before `{name}`. Pass a narrower `path` or `include`"));
                        break;
                    }
                    for (line_no, line) in body.lines().enumerate() {
                        if matcher.is_match(line) {
                            if matches.len() == 100 {
                                truncated = true;
                                break;
                            }
                            matches.push(json!({"path":name,"line":line_no+1,"text":line.chars().take(240).collect::<String>(),"digest":digest}));
                        }
                    }
                    if truncated {
                        break;
                    }
                }
                json!({"matches":matches,"truncated":truncated,"stopped":stopped,"externalContent":true,"protectedEntriesExcluded":true,"skipped":skipped_json(&skipped)})
            }
            "fs_write" => {
                let a: WriteArgs = parse(args)?;
                relative(&a.path, false)?;
                guard(&root, &a.path)?;
                if !a.create_only {
                    return Err(CREATE_ONLY_REQUIRED.into());
                }
                if a.content.len() > MAX_FILE as usize {
                    return Err("FILE_SIZE_LIMIT".into());
                }
                root.create_new(&staging, Path::new(&a.path), a.content.as_bytes())
                    .map_err(denied)?;
                json!({"path":a.path,"created":true,"bytes":a.content.len(),"digest":digest(a.content.as_bytes())})
            }
            "fs_edit" => {
                let a: EditArgs = parse(args)?;
                relative(&a.path, false)?;
                guard(&root, &a.path)?;
                let expected = a
                    .expected_sha256
                    .as_deref()
                    .ok_or(EXPECTED_SHA256_REQUIRED)?;
                let ops: Option<Vec<EditOp>> =
                    match (a.content.is_some(), a.edits, a.old_string, a.new_string) {
                        (true, None, None, None) => None,
                        (false, Some(e), None, None) if !a.replace_all => Some(e),
                        (false, None, Some(old_string), Some(new_string)) => Some(vec![EditOp {
                            old_string,
                            new_string,
                            replace_all: a.replace_all,
                        }]),
                        _ => return Err("EDIT_MODE_AMBIGUOUS".into()),
                    };
                if a.content
                    .as_ref()
                    .is_some_and(|c| c.len() > MAX_FILE as usize)
                {
                    return Err("FILE_SIZE_LIMIT".into());
                }
                let count = ops.as_ref().map_or(0, Vec::len);
                let content = a.content;
                let replaced = root.replace(
                    &staging,
                    Path::new(&a.path),
                    expected,
                    MAX_FILE,
                    |old| match (&ops, content) {
                        (Some(ops), _) => transform_edits(text(old)?, ops).map(String::into_bytes),
                        (None, Some(content)) => Ok(content.into_bytes()),
                        (None, None) => Err(invalid()),
                    },
                )?;
                json!({"path":a.path,"replaced":true,"edits":count,"previousDigest":replaced.previous_digest,"digest":replaced.digest,"bytes":replaced.bytes})
            }
            "fs_apply_patch" => {
                let a: PatchArgs = parse(args)?;
                let (path, kind) = patch_target(&a.patch)?;
                guard(&root, &path)?;
                match kind {
                    PatchKind::Add => {
                        if a.expected_sha256.is_some() {
                            return Err("EXPECTED_SHA256_NOT_ALLOWED_FOR_ADD".into());
                        }
                        let prepared = transform_patch(&a.patch, None)?;
                        root.create_new(
                            &staging,
                            Path::new(&prepared.path),
                            prepared.content.as_bytes(),
                        )
                        .map_err(denied)?;
                        json!({"applied":[format!("A {}",prepared.path)],"digest":digest(prepared.content.as_bytes()),"bytes":prepared.content.len()})
                    }
                    PatchKind::Update => {
                        let expected = a
                            .expected_sha256
                            .as_deref()
                            .ok_or(EXPECTED_SHA256_REQUIRED)?;
                        let patch = a.patch;
                        let replaced =
                            root.replace(&staging, Path::new(&path), expected, MAX_FILE, |old| {
                                transform_patch(&patch, Some(text(old)?))
                                    .map(|p| p.content.into_bytes())
                            })?;
                        json!({"applied":[format!("M {path}")],"previousDigest":replaced.previous_digest,"digest":replaced.digest,"bytes":replaced.bytes})
                    }
                }
            }
            _ => return Err("EXTERNAL_FILE_TOOL_UNSUPPORTED".into()),
        };
        bounded(result)
    }
}
fn skip_reason(code: &str) -> &'static str {
    match code {
        "FILE_UTF8_REQUIRED" => "not_utf8",
        "BINARY_FILE_UNSUPPORTED" => "binary",
        _ => "unreadable_or_changed",
    }
}
fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}
fn read_text(root: &Root, path: &Path) -> Result<(String, String, u64), String> {
    let mut snap = root.snapshot(path, MAX_FILE).map_err(denied)?;
    let mut body = String::new();
    snap.file
        .read_to_string(&mut body)
        .map_err(|_| "FILE_UTF8_REQUIRED")?;
    if body.contains('\0') {
        return Err("BINARY_FILE_UNSUPPORTED".into());
    }
    Ok((body, snap.digest, snap.bytes))
}
fn glob(pattern: &str) -> Result<regex::Regex, String> {
    if pattern.is_empty()
        || pattern.len() > 512
        || pattern.contains(['[', ']', '{', '}', '\\', '\0'])
    {
        return Err("GLOB_SUBSET_UNSUPPORTED".into());
    }
    let mut re = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    chars.next();
                    re.push_str("(?:.*/)?");
                } else {
                    re.push_str(".*");
                }
            }
            '*' => re.push_str("[^/]*"),
            '?' => re.push_str("[^/]"),
            c => re.push_str(&regex::escape(&c.to_string())),
        }
    }
    re.push('$');
    regex::RegexBuilder::new(&re)
        .size_limit(256 * 1024)
        .build()
        .map_err(|_| "INVALID_GLOB".into())
}
fn text(bytes: &[u8]) -> Result<&str, String> {
    let body = std::str::from_utf8(bytes).map_err(|_| "FILE_UTF8_REQUIRED")?;
    if body.contains('\0') {
        return Err("BINARY_FILE_UNSUPPORTED".into());
    }
    Ok(body)
}
pub fn transform_edits(source: &str, edits: &[EditOp]) -> Result<String, String> {
    if edits.is_empty() || edits.len() > MAX_EDITS {
        return Err("EDIT_COUNT_LIMIT".into());
    }
    if source.len() > MAX_FILE as usize {
        return Err("FILE_SIZE_LIMIT".into());
    }
    let mut current = source.to_owned();
    for (index, edit) in edits.iter().enumerate() {
        if edit.old_string.is_empty() {
            return Err(format!("EDIT_EMPTY_OLD_STRING:{index}"));
        }
        if edit.new_string.len() > MAX_FILE as usize {
            return Err("FILE_SIZE_LIMIT".into());
        }
        let count = current.matches(&edit.old_string).count();
        if count == 0 {
            return Err(format!("EDIT_NO_MATCH:{index}"));
        }
        if count > 1 && !edit.replace_all {
            return Err(format!("EDIT_AMBIGUOUS:{index}"));
        }
        let replacements = if edit.replace_all { count } else { 1 };
        let size = replacements
            .checked_mul(edit.new_string.len())
            .and_then(|added| {
                current
                    .len()
                    .checked_sub(replacements * edit.old_string.len())?
                    .checked_add(added)
            })
            .ok_or("FILE_SIZE_LIMIT")?;
        if size > MAX_FILE as usize {
            return Err("FILE_SIZE_LIMIT".into());
        }
        current = if edit.replace_all {
            current.replace(&edit.old_string, &edit.new_string)
        } else {
            current.replacen(&edit.old_string, &edit.new_string, 1)
        };
    }
    Ok(current)
}
fn envelope(patch: &str) -> Result<(&str, Vec<&str>), String> {
    if patch.len() > MAX_FILE as usize || patch.contains('\r') {
        return Err("PATCH_SUBSET_UNSUPPORTED".into());
    }
    let lines: Vec<&str> = patch.lines().collect();
    if lines.first() != Some(&"*** Begin Patch")
        || lines.last() != Some(&"*** End Patch")
        || lines.len() < 4
    {
        return Err("INVALID_PATCH_ENVELOPE".into());
    }
    Ok((lines[1], lines[2..lines.len() - 1].to_vec()))
}
pub fn patch_target(patch: &str) -> Result<(String, PatchKind), String> {
    let (header, _) = envelope(patch)?;
    let (path, kind) = if let Some(p) = header.strip_prefix("*** Add File: ") {
        (p, PatchKind::Add)
    } else if let Some(p) = header.strip_prefix("*** Update File: ") {
        (p, PatchKind::Update)
    } else {
        return Err("PATCH_SUBSET_UNSUPPORTED".into());
    };
    relative(path, false)?;
    Ok((path.into(), kind))
}
pub fn transform_patch(patch: &str, source: Option<&str>) -> Result<PreparedPatch, String> {
    let (path, kind) = patch_target(patch)?;
    let (_, body) = envelope(patch)?;
    if kind == PatchKind::Add {
        let mut content = String::new();
        for line in &body {
            let added = line.strip_prefix('+').ok_or("PATCH_SUBSET_UNSUPPORTED")?;
            content.push_str(added);
            content.push('\n');
        }
        if content.len() > MAX_FILE as usize {
            return Err("FILE_SIZE_LIMIT".into());
        }
        return Ok(PreparedPatch {
            path,
            kind,
            content,
        });
    }
    let source = source.ok_or("PATCH_SOURCE_REQUIRED")?;
    if source.len() > MAX_FILE as usize || source.contains('\r') {
        return Err("PATCH_SUBSET_UNSUPPORTED".into());
    }
    struct Hunk<'a> {
        anchor: Option<&'a str>,
        old: Vec<&'a str>,
        new: Vec<&'a str>,
    }
    let mut hunks: Vec<Hunk> = Vec::new();
    for line in &body {
        if *line == "@@" || line.starts_with("@@ ") {
            if hunks.len() == MAX_HUNKS {
                return Err("PATCH_HUNK_LIMIT".into());
            }
            let anchor = line.strip_prefix("@@ ").filter(|a| !a.is_empty());
            hunks.push(Hunk {
                anchor,
                old: Vec::new(),
                new: Vec::new(),
            });
            continue;
        }
        let hunk = hunks.last_mut().ok_or("PATCH_SUBSET_UNSUPPORTED")?;
        match line.as_bytes().first() {
            Some(b' ') => {
                hunk.old.push(&line[1..]);
                hunk.new.push(&line[1..]);
            }
            Some(b'-') => hunk.old.push(&line[1..]),
            Some(b'+') => hunk.new.push(&line[1..]),
            _ => return Err("PATCH_SUBSET_UNSUPPORTED".into()),
        }
    }
    if hunks.is_empty() {
        return Err("PATCH_SUBSET_UNSUPPORTED".into());
    }
    let mut lines: Vec<&str> = source.lines().collect();
    let mut cursor = 0usize;
    for (index, hunk) in hunks.into_iter().enumerate() {
        if let Some(anchor) = hunk.anchor {
            let found: Vec<usize> = (cursor..lines.len())
                .filter(|i| lines[*i] == anchor)
                .collect();
            if found.len() != 1 {
                return Err(format!("PATCH_ANCHOR_NOT_UNIQUE:{index}"));
            }
            cursor = found[0] + 1;
        }
        if hunk.old.is_empty() {
            return Err(format!("PATCH_EMPTY_CONTEXT:{index}"));
        }
        let positions: Vec<usize> = lines[cursor.min(lines.len())..]
            .windows(hunk.old.len())
            .enumerate()
            .filter(|(_, w)| *w == hunk.old.as_slice())
            .map(|(i, _)| i + cursor)
            .collect();
        match positions.len() {
            0 => return Err(format!("PATCH_CONTEXT_NOT_FOUND:{index}")),
            1 => {}
            _ => return Err(format!("PATCH_CONTEXT_NOT_UNIQUE:{index}")),
        }
        let at = positions[0];
        let added = hunk.new.len();
        lines.splice(at..at + hunk.old.len(), hunk.new);
        cursor = at + added;
    }
    let mut content = lines.join("\n");
    if source.ends_with('\n') && !content.is_empty() {
        content.push('\n');
    }
    if content.len() > MAX_FILE as usize {
        return Err("FILE_SIZE_LIMIT".into());
    }
    Ok(PreparedPatch {
        path,
        kind,
        content,
    })
}
pub fn external_spec(name: &str) -> Option<(&'static str, Value)> {
    let sha = json!({"type":"string","pattern":"^[0-9a-fA-F]{64}$","description":"SHA-256 `digest` returned by fs_read (or a previous write) for the exact current content"});
    match name {
        "fs_write" => Some((
            "Create a new file inside the granted workspace. Existing files are never overwritten; create_only must be true. Use fs_edit to change an existing file.",
            json!({"type":"object","additionalProperties":false,"properties":{"path":{"type":"string"},"content":{"type":"string"},"create_only":{"type":"boolean","const":true}},"required":["path","content","create_only"]}),
        )),
        "fs_edit" => Some((
            "Edit an existing regular file inside the granted workspace. Pass expected_sha256 = the digest from your last fs_read of that file, plus exactly one of: content (whole new content), edits (1-64 ordered exact replacements, all-or-nothing) or old_string/new_string. Fails with FILE_REVISION_CONFLICT or FILE_CHANGED_DURING_COMMIT if the file changed: re-read and retry. Returns the new digest. Max 1 MiB; symlinks, hard links and special files are refused.",
            json!({"type":"object","additionalProperties":false,"properties":{
                "path":{"type":"string"},
                "expected_sha256":sha,
                "content":{"type":"string","description":"Whole replacement content"},
                "edits":{"type":"array","minItems":1,"maxItems":MAX_EDITS,"items":{"type":"object","additionalProperties":false,"properties":{"old_string":{"type":"string","minLength":1},"new_string":{"type":"string"},"replace_all":{"type":"boolean"}},"required":["old_string","new_string"]}},
                "old_string":{"type":"string","minLength":1},
                "new_string":{"type":"string"},
                "replace_all":{"type":"boolean"}
            },"required":["path","expected_sha256"]}),
        )),
        "fs_apply_patch" => Some((
            "Apply a single-file patch in the '*** Begin Patch' envelope. '*** Add File: <path>' creates a new file (never overwrites). '*** Update File: <path>' with 1-64 '@@' hunks of ' ', '-', '+' lines edits an existing file and requires expected_sha256 (digest from fs_read); each hunk's context must match exactly once after the previous hunk. Delete, move and multi-file patches are denied.",
            json!({"type":"object","additionalProperties":false,"properties":{"patch":{"type":"string"},"expected_sha256":sha},"required":["patch"]}),
        )),
        _ => None,
    }
}
