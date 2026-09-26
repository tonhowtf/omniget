//! Capability-rooted external file tools. No shell or ambient workspace lookup.
//! New files: create-only (`fs_write` create_only, `fs_apply_patch` Add File).
//! Existing files: `fs_edit` and `fs_apply_patch` Update File, both bound to the
//! SHA-256 the caller read (`expected_sha256`) and committed through
//! `Root::replace` (private staging, pre-check, atomic exchange, post-exchange
//! verification with rollback). That is optimistic concurrency with documented
//! residual windows, not a kernel CAS; see secure_files.rs.
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
/// `fs_write` without create_only: existing files go through fs_edit.
pub const CREATE_ONLY_REQUIRED: &str = "FS_WRITE_CREATE_ONLY_USE_FS_EDIT";
pub const EXPECTED_SHA256_REQUIRED: &str = "EXPECTED_SHA256_REQUIRED";
fn denied(_: std::io::Error) -> String {
    "FILE_ACCESS_DENIED_OR_CHANGED".into()
}
fn invalid() -> String {
    "INVALID_FILE_ARGUMENTS".into()
}
/// Exact sensitive names, compared on the folded form (see [`fold`]).
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
/// Folders a discovery walk lists but never descends (heavy, generated).
const HEAVY_DIRS: &[&str] = &["node_modules", "target"];
/// Default-ignorable and format code points some filesystems drop when they
/// compare names (HFS+ ignores several of them).
fn ignorable(c: char) -> bool {
    matches!(c as u32, 0x00AD | 0x034F | 0x061C | 0x115F | 0x1160 | 0x17B4 | 0x17B5 | 0x180B..=0x180F | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0x3164 | 0xFE00..=0xFE0F | 0xFEFF | 0xFFA0 | 0xFFF0..=0xFFF8 | 0xE0000..=0xE0FFF)
}
/// Folded comparison form of one name component (audit F-EF1). APFS/HFS+ are
/// case-insensitive with full Unicode folding and normalization-insensitive,
/// so `.\u{17f}\u{17f}h` (long s) or `\u{212a}` (Kelvin) open the same object as
/// `.ssh`/`k`. The fold is deliberately wider than any filesystem: NFKD
/// (compatibility: full-width, ligatures, long s), drop combining marks and
/// ignorable code points, full case fold (upper then lower: ß→ss, ſ→s,
/// K→k, ı→i), then NFKC. It only ever refuses more names.
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
/// Defense in depth for known sensitive names, not a promise to discover every
/// secret. The local grant must select a workspace safe for the recipient.
fn protected(path: &Path) -> bool {
    path.components().any(|part| match part {
        std::path::Component::Normal(name) => protected_name(name),
        _ => false,
    })
}
/// Object-level check (audit F-EF1): whatever the spelling, if any existing
/// component of `path` IS the same object (device+inode, no symlink follow)
/// as a protected name in the same folder, the path is protected. This covers
/// folding rules a string comparison might miss on a given filesystem.
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
/// Up to 50 skipped entries with their reason, plus the total.
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
/// Exactly one of: `content` (whole replacement), `edits` (ordered exact
/// replacements, all-or-nothing) or the single `old_string`/`new_string` form
/// shared with the local fs_edit schema.
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

/// Paths/identities must originate in local grants; never deserialize this
/// capability from tool arguments. Staging must be denied to every worker.
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
        // Reopen by pinned identity for every operation. Replacing the grant
        // pathname never silently redirects an existing executor capability.
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
                    // One unreadable/binary/changing file never fails the search.
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
/// Supported glob subset: *, **, ?, with **/ matching zero or more directories.
/// Braces/character classes are refused rather than given surprising semantics.
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
/// Pure ordered exact edits, all-or-nothing. Each old_string must be non-empty
/// and match exactly once in the text produced by the previous edits unless
/// replace_all is set. Revision binding is done by the caller (Root::replace).
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
/// Target path and kind of a single-file patch, validated before any I/O.
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
/// Pure V4A subset for ONE file: an Add, or an Update with 1..=64 exact hunks.
/// A hunk header is `@@` or `@@ <anchor>`; the anchor must match exactly one
/// whole line after the previous hunk and the hunk is searched after it. Each
/// hunk's context must match exactly once in the remaining text (forward only).
/// No fuzzy matching, move, delete, multi-file partial commit or EOF coercion.
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
/// Descriptions/schemas the external (MCP-granted) projection must expose for
/// the write tools, replacing the local-harness specs of the same name.
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, ExternalFiles, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let workspace = base.join("workspace");
        let staging = base.join("private-staging");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&staging).unwrap();
        let root = Root::open(&workspace, None).unwrap();
        let stage = Root::open(&staging, None).unwrap();
        let executor =
            ExternalFiles::open(&workspace, root.identity(), &staging, stage.identity()).unwrap();
        (dir, executor, workspace)
    }
    #[test]
    fn read_create_search_and_single_file_add_use_real_capabilities() {
        let (_dir, e, ws) = fixture();
        let write = e
            .execute(
                "fs_write",
                &json!({"path":"readme.md","content":"hello\nworld\n","create_only":true}),
            )
            .unwrap();
        assert_eq!(write["digest"], digest(b"hello\nworld\n"));
        assert!(e
            .execute(
                "fs_write",
                &json!({"path":"readme.md","content":"overwrite","create_only":true})
            )
            .is_err());
        assert_eq!(
            std::fs::read_to_string(ws.join("readme.md")).unwrap(),
            "hello\nworld\n"
        );
        let read = e
            .execute("fs_read", &json!({"path":"readme.md","limit":1}))
            .unwrap();
        assert_eq!(read["hasMore"], true);
        assert_eq!(read["nextOffset"], 2);
        assert_eq!(
            e.execute("fs_list", &json!({})).unwrap()["entries"],
            json!(["readme.md"])
        );
        assert_eq!(
            e.execute("fs_glob", &json!({"pattern":"**/*.md"})).unwrap()["files"],
            json!(["readme.md"])
        );
        assert_eq!(
            e.execute("fs_grep", &json!({"pattern":"world","literal_text":true}))
                .unwrap()["matches"][0]["line"],
            2
        );
        e.execute(
            "fs_apply_patch",
            &json!({"patch":"*** Begin Patch\n*** Add File: next.md\n+next\n*** End Patch\n"}),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(ws.join("next.md")).unwrap(),
            "next\n"
        );
    }
    #[test]
    fn traversal_symlink_hardlink_special_files_and_root_swap_are_denied() {
        use std::os::unix::fs::symlink;
        let (dir, e, ws) = fixture();
        std::fs::write(ws.join("valid"), "safe").unwrap();
        for path in ["../outside", "/etc/passwd", "./valid", "folder/../valid"] {
            assert!(e.execute("fs_read", &json!({"path":path})).is_err());
        }
        symlink("valid", ws.join("link")).unwrap();
        assert!(e.execute("fs_read", &json!({"path":"link"})).is_err());
        std::fs::hard_link(ws.join("valid"), ws.join("hard")).unwrap();
        assert!(e.execute("fs_read", &json!({"path":"valid"})).is_err());
        let fifo = std::ffi::CString::new(ws.join("fifo").to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(e.execute("fs_read", &json!({"path":"fifo"})).is_err());
        std::fs::rename(&ws, dir.path().join("moved")).unwrap();
        std::fs::create_dir(&ws).unwrap();
        assert!(e
            .execute(
                "fs_write",
                &json!({"path":"escape","content":"no","create_only":true})
            )
            .is_err());
        assert!(!ws.join("escape").exists());
    }
    #[test]
    fn fs_write_stays_create_only_and_multi_file_patch_is_impossible() {
        let (_dir, e, ws) = fixture();
        std::fs::write(ws.join("existing"), "before\n").unwrap();
        assert_eq!(
            e.execute("fs_write", &json!({"path":"existing","content":"after"}))
                .unwrap_err(),
            CREATE_ONLY_REQUIRED
        );
        assert_eq!(
            e.execute(
                "fs_edit",
                &json!({"path":"existing","old_string":"before","new_string":"after"})
            )
            .unwrap_err(),
            EXPECTED_SHA256_REQUIRED
        );
        assert_eq!(e.execute("fs_apply_patch",&json!({"patch":"*** Begin Patch\n*** Update File: existing\n@@\n-before\n+after\n*** End Patch"})).unwrap_err(),EXPECTED_SHA256_REQUIRED);
        assert!(e.execute("fs_apply_patch",&json!({"patch":"*** Begin Patch\n*** Add File: new\n+first\n*** Add File: second\n+second\n*** End Patch"})).is_err());
        assert!(e
            .execute(
                "fs_apply_patch",
                &json!({"patch":"*** Begin Patch\n*** Delete File: existing\n*** End Patch"})
            )
            .is_err());
        assert!(!ws.join("new").exists());
        assert_eq!(
            std::fs::read_to_string(ws.join("existing")).unwrap(),
            "before\n"
        );
    }
    #[test]
    fn pure_transformations_are_exact_and_bounded() {
        let op = |o: &str, n: &str| EditOp {
            old_string: o.into(),
            new_string: n.into(),
            replace_all: false,
        };
        assert_eq!(
            transform_edits("before\n", &[op("before", "after")]).unwrap(),
            "after\n"
        );
        assert_eq!(
            transform_edits("a b\n", &[op("a", "x"), op("b", "y")]).unwrap(),
            "x y\n"
        );
        assert_eq!(
            transform_edits("a a\n", &[op("a", "x")]).unwrap_err(),
            "EDIT_AMBIGUOUS:0"
        );
        assert_eq!(
            transform_edits("a\n", &[op("a", "x"), op("zz", "y")]).unwrap_err(),
            "EDIT_NO_MATCH:1"
        );
        assert!(transform_edits("a\n", &vec![op("a", "a"); MAX_EDITS + 1]).is_err());
        let patch = "*** Begin Patch\n*** Update File: file\n@@\n-before\n+after\n*** End Patch";
        assert_eq!(
            transform_patch(patch, Some("before\n")).unwrap().content,
            "after\n"
        );
        assert!(transform_patch(patch, Some("before\nbefore\n")).is_err());
        assert!(transform_patch(patch, Some("prefix-before\n")).is_err());
        let multi = "*** Begin Patch\n*** Update File: file\n@@\n title\n-one\n+uno\n@@ ## B\n-x\n+equis\n*** End Patch";
        assert_eq!(
            transform_patch(multi, Some("title\none\n## A\nx\n## B\nx\n"))
                .unwrap()
                .content,
            "title\nuno\n## A\nx\n## B\nequis\n"
        );
        let source = "a".repeat(MAX_FILE as usize);
        let big = EditOp {
            old_string: "a".into(),
            new_string: "b".repeat(MAX_FILE as usize),
            replace_all: true,
        };
        assert_eq!(
            transform_edits(&source, &[big]).unwrap_err(),
            "FILE_SIZE_LIMIT"
        );
    }
    fn read_digest(e: &ExternalFiles, path: &str) -> String {
        e.execute("fs_read", &json!({"path":path})).unwrap()["digest"]
            .as_str()
            .unwrap()
            .to_string()
    }
    fn junk(ws: &Path) -> Vec<String> {
        let mut out = Vec::new();
        for entry in walk(ws) {
            let name = entry.file_name().unwrap().to_string_lossy().into_owned();
            if name.starts_with(".omniget") {
                out.push(entry.to_string_lossy().into_owned());
            }
        }
        out
    }
    fn walk(p: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(p).unwrap() {
            let e = e.unwrap().path();
            if std::fs::symlink_metadata(&e).unwrap().is_dir() {
                out.extend(walk(&e));
            }
            out.push(e);
        }
        out
    }
    use crate::core::secure_files::replace_test_hook as hook;
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            hook::clear();
        }
    }
    #[test]
    fn edit_happy_paths_return_new_digest() {
        let (_dir, e, ws) = fixture();
        std::fs::create_dir(ws.join("docs")).unwrap();
        std::fs::write(
            ws.join("docs/README.md"),
            "# OmniGet\nOmniGet downloads.\n\n## Install\nuse OmniGet\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            ws.join("docs/README.md"),
            std::fs::Permissions::from_mode(0o640),
        )
        .unwrap();
        let d0 = read_digest(&e, "docs/README.md");
        let r = e.execute("fs_edit", &json!({"path":"docs/README.md","expected_sha256":d0,"edits":[{"old_string":"# OmniGet","new_string":"# Loop"},{"old_string":"OmniGet","new_string":"Loop","replace_all":true}]})).unwrap();
        let body = std::fs::read_to_string(ws.join("docs/README.md")).unwrap();
        assert_eq!(body, "# Loop\nLoop downloads.\n\n## Install\nuse Loop\n");
        assert_eq!(r["digest"], digest(body.as_bytes()));
        assert_eq!(r["previousDigest"], d0);
        assert_eq!(r["edits"], 2);
        assert_eq!(
            std::fs::metadata(ws.join("docs/README.md"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
        let d1 = read_digest(&e, "docs/README.md");
        assert_eq!(r["digest"], d1);
        let r = e.execute("fs_edit", &json!({"path":"docs/README.md","expected_sha256":d1.to_uppercase(),"old_string":"use Loop","new_string":"run Loop"})).unwrap();
        let d2 = r["digest"].as_str().unwrap().to_string();
        let r = e.execute("fs_apply_patch", &json!({"expected_sha256":d2,"patch":"*** Begin Patch\n*** Update File: docs/README.md\n@@\n-# Loop\n+# Loop (ex-OmniGet)\n@@ ## Install\n-run Loop\n+brew install loop\n*** End Patch"})).unwrap();
        let body = std::fs::read_to_string(ws.join("docs/README.md")).unwrap();
        assert_eq!(
            body,
            "# Loop (ex-OmniGet)\nLoop downloads.\n\n## Install\nbrew install loop\n"
        );
        assert_eq!(r["digest"], digest(body.as_bytes()));
        let r = e
            .execute(
                "fs_edit",
                &json!({"path":"docs/README.md","expected_sha256":r["digest"],"content":"short\n"}),
            )
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(ws.join("docs/README.md")).unwrap(),
            "short\n"
        );
        assert_eq!(r["digest"], digest(b"short\n"));
        assert_eq!(e.execute("fs_edit", &json!({"path":"docs/README.md","expected_sha256":r["digest"],"content":"x","old_string":"a","new_string":"b"})).unwrap_err(), "EDIT_MODE_AMBIGUOUS");
        assert_eq!(
            e.execute(
                "fs_edit",
                &json!({"path":"docs/README.md","expected_sha256":"nothex","content":"x"})
            )
            .unwrap_err(),
            "EXPECTED_SHA256_INVALID"
        );
        assert_eq!(
            e.execute(
                "fs_edit",
                &json!({"path":"docs/missing.md","expected_sha256":r["digest"],"content":"x"})
            )
            .unwrap_err(),
            "FILE_NOT_FOUND"
        );
        assert!(junk(&ws).is_empty());
    }
    #[test]
    fn stale_digest_is_rejected_without_touching_the_file() {
        let (_dir, e, ws) = fixture();
        std::fs::write(ws.join("a.md"), "v1\n").unwrap();
        let stale = read_digest(&e, "a.md");
        e.execute(
            "fs_edit",
            &json!({"path":"a.md","expected_sha256":stale,"content":"v2\n"}),
        )
        .unwrap();
        assert_eq!(
            e.execute(
                "fs_edit",
                &json!({"path":"a.md","expected_sha256":stale,"content":"v3\n"})
            )
            .unwrap_err(),
            "FILE_REVISION_CONFLICT"
        );
        assert_eq!(e.execute("fs_apply_patch", &json!({"expected_sha256":stale,"patch":"*** Begin Patch\n*** Update File: a.md\n@@\n-v2\n+v3\n*** End Patch"})).unwrap_err(), "FILE_REVISION_CONFLICT");
        assert_eq!(std::fs::read_to_string(ws.join("a.md")).unwrap(), "v2\n");
        // A failing transform never commits.
        let d = read_digest(&e, "a.md");
        assert_eq!(e.execute("fs_edit", &json!({"path":"a.md","expected_sha256":d,"edits":[{"old_string":"v2","new_string":"v3"},{"old_string":"absent","new_string":"x"}]})).unwrap_err(), "EDIT_NO_MATCH:1");
        assert_eq!(std::fs::read_to_string(ws.join("a.md")).unwrap(), "v2\n");
    }
    #[test]
    fn concurrent_writers_at_every_stage_are_detected_and_their_bytes_survive() {
        use std::io::Write;
        for stage in ["after_read", "before_precheck", "before_swap"] {
            let (_dir, e, ws) = fixture();
            let target = ws.join("t.md");
            std::fs::write(&target, "base\n").unwrap();
            let d = read_digest(&e, "t.md");
            // The writer opened the file BEFORE the edit started (worst case:
            // same inode, no rename, writes through an existing descriptor).
            let mut writer = std::fs::OpenOptions::new()
                .write(true)
                .open(&target)
                .unwrap();
            let _c = Clear;
            let wanted = stage;
            let mut done = false;
            hook::set(move |s| {
                if s == wanted && !done {
                    done = true;
                    writer.write_all(b"THEIRS\n").unwrap();
                    writer.sync_all().unwrap();
                }
            });
            let err = e
                .execute(
                    "fs_edit",
                    &json!({"path":"t.md","expected_sha256":d,"content":"ours\n"}),
                )
                .unwrap_err();
            assert_eq!(err, "FILE_CHANGED_DURING_COMMIT", "stage {stage}");
            assert_eq!(
                std::fs::read_to_string(&target).unwrap(),
                "THEIRS\n",
                "stage {stage}"
            );
            assert!(junk(&ws).is_empty(), "stage {stage}");
        }
    }
    #[test]
    fn replacing_the_target_name_during_commit_is_rolled_back() {
        let (dir, e, ws) = fixture();
        let target = ws.join("t.md");
        std::fs::write(&target, "base\n").unwrap();
        let outside = dir.path().canonicalize().unwrap().join("outside-secret");
        std::fs::write(&outside, "secret\n").unwrap();
        let d = read_digest(&e, "t.md");
        for stage in ["before_precheck", "before_swap"] {
            let _c = Clear;
            let (t, o, ws2) = (target.clone(), outside.clone(), ws.clone());
            let wanted = stage;
            hook::set(move |s| {
                if s == wanted && std::fs::symlink_metadata(&t).unwrap().is_file() {
                    std::fs::rename(&t, ws2.join("t.orig")).unwrap();
                    std::os::unix::fs::symlink(&o, &t).unwrap();
                }
            });
            assert_eq!(
                e.execute(
                    "fs_edit",
                    &json!({"path":"t.md","expected_sha256":d,"content":"ours\n"})
                )
                .unwrap_err(),
                "FILE_CHANGED_DURING_COMMIT",
                "{stage}"
            );
            hook::clear();
            assert!(
                std::fs::symlink_metadata(&target)
                    .unwrap()
                    .file_type()
                    .is_symlink(),
                "{stage}"
            );
            assert_eq!(std::fs::read_to_string(&outside).unwrap(), "secret\n");
            assert_eq!(
                std::fs::read_to_string(ws.join("t.orig")).unwrap(),
                "base\n"
            );
            std::fs::remove_file(&target).unwrap();
            std::fs::rename(ws.join("t.orig"), &target).unwrap();
        }
        // Static: a symlink target is refused outright.
        std::os::unix::fs::symlink(&outside, ws.join("link.md")).unwrap();
        assert_eq!(
            e.execute(
                "fs_edit",
                &json!({"path":"link.md","expected_sha256":digest(b"secret\n"),"content":"pwn"})
            )
            .unwrap_err(),
            "FILE_SYMLINK_REFUSED"
        );
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "secret\n");
        assert!(junk(&ws).is_empty());
    }
    #[test]
    fn parent_symlink_static_and_swapped_mid_commit_is_refused() {
        let (dir, e, ws) = fixture();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir(ws.join("sub")).unwrap();
        std::fs::write(ws.join("sub/f.md"), "base\n").unwrap();
        let d = read_digest(&e, "sub/f.md");
        // Static: parent component replaced by a symlink to an outside dir.
        std::fs::create_dir(base.join("out")).unwrap();
        std::fs::write(base.join("out/f.md"), "base\n").unwrap();
        std::os::unix::fs::symlink(base.join("out"), ws.join("alias")).unwrap();
        let err = e
            .execute(
                "fs_edit",
                &json!({"path":"alias/f.md","expected_sha256":d,"content":"pwn\n"}),
            )
            .unwrap_err();
        assert!(
            err == "FILE_SYMLINK_REFUSED" || err == "FILE_PARENT_NOT_DIRECTORY",
            "{err}"
        );
        assert_eq!(
            std::fs::read_to_string(base.join("out/f.md")).unwrap(),
            "base\n"
        );
        // Dynamic: parent moved outside the root and replaced by a symlink
        // after the precheck; the exchange lands in the moved dir and must be
        // reversed there.
        let _c = Clear;
        let (w, b) = (ws.clone(), base.clone());
        let mut done = false;
        hook::set(move |s| {
            if s == "before_swap" && !done {
                done = true;
                std::fs::rename(w.join("sub"), b.join("moved")).unwrap();
                std::os::unix::fs::symlink(b.join("moved"), w.join("sub")).unwrap();
            }
        });
        assert_eq!(
            e.execute(
                "fs_edit",
                &json!({"path":"sub/f.md","expected_sha256":d,"content":"ours\n"})
            )
            .unwrap_err(),
            "FILE_CHANGED_DURING_COMMIT"
        );
        assert_eq!(
            std::fs::read_to_string(base.join("moved/f.md")).unwrap(),
            "base\n"
        );
        assert!(junk(&base.join("moved")).is_empty());
    }
    #[test]
    fn hardlinks_special_files_traversal_and_protected_paths_are_refused() {
        let (dir, e, ws) = fixture();
        std::fs::write(ws.join("a.md"), "x\n").unwrap();
        let d = digest(b"x\n");
        std::fs::hard_link(ws.join("a.md"), dir.path().join("outside-hard")).unwrap();
        assert_eq!(
            e.execute(
                "fs_edit",
                &json!({"path":"a.md","expected_sha256":d,"content":"y"})
            )
            .unwrap_err(),
            "FILE_HARDLINKED"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("outside-hard")).unwrap(),
            "x\n"
        );
        let fifo = std::ffi::CString::new(ws.join("fifo").to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert_eq!(
            e.execute(
                "fs_edit",
                &json!({"path":"fifo","expected_sha256":d,"content":"y"})
            )
            .unwrap_err(),
            "FILE_NOT_REGULAR"
        );
        std::fs::create_dir(ws.join("folder")).unwrap();
        assert_eq!(
            e.execute(
                "fs_edit",
                &json!({"path":"folder","expected_sha256":d,"content":"y"})
            )
            .unwrap_err(),
            "FILE_NOT_REGULAR"
        );
        for path in [
            "../outside-hard",
            "/etc/hosts",
            "./a.md",
            "folder/../a.md",
            "folder//a.md",
            "",
        ] {
            assert_eq!(
                e.execute(
                    "fs_edit",
                    &json!({"path":path,"expected_sha256":d,"content":"y"})
                )
                .unwrap_err(),
                "INVALID_FILE_ARGUMENTS",
                "{path}"
            );
        }
        std::fs::write(ws.join(".env"), "x\n").unwrap();
        for path in [".env", ".git/config", ".omniget-x.replace"] {
            assert_eq!(
                e.execute(
                    "fs_edit",
                    &json!({"path":path,"expected_sha256":d,"content":"y"})
                )
                .unwrap_err(),
                "PROTECTED_WORKSPACE_PATH"
            );
        }
        assert_eq!(std::fs::read_to_string(ws.join(".env")).unwrap(), "x\n");
    }
    #[test]
    fn root_swap_between_read_and_edit_is_refused() {
        let (dir, e, ws) = fixture();
        std::fs::write(ws.join("a.md"), "x\n").unwrap();
        let d = read_digest(&e, "a.md");
        let moved = dir.path().join("moved");
        std::fs::rename(&ws, &moved).unwrap();
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("a.md"), "x\n").unwrap();
        assert!(e
            .execute(
                "fs_edit",
                &json!({"path":"a.md","expected_sha256":d,"content":"y"})
            )
            .is_err());
        assert_eq!(std::fs::read_to_string(ws.join("a.md")).unwrap(), "x\n");
        assert_eq!(std::fs::read_to_string(moved.join("a.md")).unwrap(), "x\n");
    }
    #[test]
    fn content_bounds_apply_to_old_and_new() {
        let (_dir, e, ws) = fixture();
        std::fs::write(ws.join("a.md"), "x\n").unwrap();
        let d = read_digest(&e, "a.md");
        let big = "y".repeat(MAX_FILE as usize + 1);
        let err = e
            .execute(
                "fs_edit",
                &json!({"path":"a.md","expected_sha256":d,"content":big}),
            )
            .unwrap_err();
        assert!(
            err == "FILE_SIZE_LIMIT" || err == "FILE_ARGUMENT_LIMIT",
            "{err}"
        );
        let err = e.execute("fs_edit", &json!({"path":"a.md","expected_sha256":d,"old_string":"x","new_string":"z".repeat(MAX_FILE as usize - 10),"replace_all":true})).unwrap();
        assert_eq!(err["bytes"], MAX_FILE - 10 + 1);
        let huge = vec![b'a'; MAX_FILE as usize + 1];
        std::fs::write(ws.join("huge.md"), &huge).unwrap();
        assert_eq!(
            e.execute(
                "fs_edit",
                &json!({"path":"huge.md","expected_sha256":digest(&huge),"content":"small"})
            )
            .unwrap_err(),
            "FILE_SIZE_LIMIT"
        );
    }
    #[test]
    fn readers_never_observe_partial_content_and_crash_leaves_old_file() {
        let (_dir, e, ws) = fixture();
        let target = ws.join("big.md");
        let a = "A".repeat(700 * 1024);
        let b = "B".repeat(900 * 1024);
        std::fs::write(&target, &a).unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = {
            let (t, stop, a, b) = (target.clone(), stop.clone(), a.clone(), b.clone());
            std::thread::spawn(move || {
                let mut reads = 0u32;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let got = std::fs::read_to_string(&t).unwrap();
                    assert!(
                        got == a || got == b,
                        "partial content of {} bytes observed",
                        got.len()
                    );
                    reads += 1;
                }
                reads
            })
        };
        let mut current = digest(a.as_bytes());
        for i in 0..20 {
            let next = if i % 2 == 0 { &b } else { &a };
            let r = e
                .execute(
                    "fs_edit",
                    &json!({"path":"big.md","expected_sha256":current,"content":next}),
                )
                .unwrap();
            current = r["digest"].as_str().unwrap().to_string();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(reader.join().unwrap() > 0);
        // Simulated crash right before the commit: the name still resolves to
        // the complete old revision and nothing leaks into the workspace.
        let before = std::fs::read_to_string(&target).unwrap();
        let _c = Clear;
        hook::set(|s| {
            if s == "before_swap" {
                panic!("simulated crash")
            }
        });
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            e.execute(
                "fs_edit",
                &json!({"path":"big.md","expected_sha256":current,"content":"never"}),
            )
        }));
        assert!(crashed.is_err());
        hook::clear();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), before);
        assert!(junk(&ws).is_empty());
    }
    #[test]
    fn known_sensitive_paths_are_denied_and_excluded_from_discovery() {
        let (_dir, e, ws) = fixture();
        std::fs::create_dir(ws.join(".git")).unwrap();
        std::fs::write(ws.join(".git/config"), "secret-canary").unwrap();
        std::fs::write(ws.join(".env"), "secret-canary").unwrap();
        std::fs::write(ws.join("public.md"), "safe").unwrap();
        for path in [".git/config", ".env", ".omniget-staging/new", ".ssh/id_rsa"] {
            assert!(e.execute("fs_read", &json!({"path":path})).is_err());
            assert!(e
                .execute(
                    "fs_write",
                    &json!({"path":path,"content":"no","create_only":true})
                )
                .is_err());
        }
        let listing = e.execute("fs_list", &json!({})).unwrap();
        assert_eq!(listing["entries"], json!(["public.md"]));
        let found = e
            .execute("fs_grep", &json!({"pattern":"secret-canary"}))
            .unwrap();
        assert_eq!(found["matches"], json!([]));
        assert_eq!(
            e.execute("fs_glob", &json!({"pattern":"**"})).unwrap()["files"],
            json!(["public.md"])
        );
    }
    #[test]
    fn staging_inside_grant_and_unsupported_globs_are_refused() {
        let (_dir, e, ws) = fixture();
        let staging = ws.join("staging");
        std::fs::create_dir(&staging).unwrap();
        let stage = Root::open(&staging, None).unwrap();
        assert!(ExternalFiles::open(&ws, &e.identity, &staging, stage.identity()).is_err());
        assert!(glob("*.{rs,md}").is_err());
        assert!(glob("[abc]").is_err());
        assert!(e
            .execute("fs_read", &json!({"path":"x","root":"/"}))
            .is_err());
    }

    /// Audit F-EF1 (reproduction turned regression): APFS/HFS+ fold `\u{17f}`
    /// (long s) into `s`, so `.\u{17f}\u{17f}h/config` opened the real `.ssh/config`.
    #[test]
    fn f_ef1_unicode_folded_spellings_of_protected_names_are_refused() {
        let (_dir, e, ws) = fixture();
        std::fs::create_dir(ws.join(".ssh")).unwrap();
        std::fs::write(ws.join(".ssh/config"), "Host prod\n").unwrap();
        std::fs::write(
            ws.join("credentials.json"),
            "{\"client_secret\":\"s3cr3t\"}\n",
        )
        .unwrap();
        for path in [
            ".ssh/config",
            ".\u{17f}\u{17f}h/config", // long s
            "credential\u{17f}.json",  // long s
            ".SSH/config",
            ".\u{ff53}\u{ff53}\u{ff48}/config", // full-width
            ".s\u{301}sh/config",               // combining acute
            ".s\u{200b}sh/config",              // zero-width space
            "ID_RSA",
            "\u{2e}env",
            ".ENV.local",
        ] {
            assert_eq!(
                e.execute("fs_read", &json!({"path":path})).unwrap_err(),
                "PROTECTED_WORKSPACE_PATH",
                "{path:?}"
            );
            assert_eq!(
                validate_path(path).unwrap_err(),
                "PROTECTED_WORKSPACE_PATH",
                "{path:?}"
            );
        }
        assert_eq!(e.execute("fs_write", &json!({"path":".\u{17f}\u{17f}h/authorized_keys","content":"ssh-ed25519 AAAA attacker\n","create_only":true})).unwrap_err(), "PROTECTED_WORKSPACE_PATH");
        assert!(!ws.join(".ssh/authorized_keys").exists());
        assert_eq!(e.execute("fs_edit", &json!({"path":"credential\u{17f}.json","expected_sha256":digest(b"{\"client_secret\":\"s3cr3t\"}\n"),"content":"x"})).unwrap_err(), "PROTECTED_WORKSPACE_PATH");
        // Folding itself.
        assert_eq!(fold("\u{212a}"), "k", "Kelvin sign");
        assert_eq!(fold(".\u{17f}\u{17f}h"), ".ssh");
        assert_eq!(fold(".\u{ff53}\u{ff53}\u{ff48}"), ".ssh");
        assert_eq!(fold(".s\u{301}sh"), ".ssh");
        assert_eq!(fold(".S\u{fe0f}SH"), ".ssh");
        assert_eq!(fold("CREDENTIALS.JSON"), "credentials.json");
        assert_eq!(fold("stra\u{df}e"), "strasse");
        // Ordinary names still work.
        std::fs::write(ws.join("notes.md"), "ok\n").unwrap();
        assert!(e.execute("fs_read", &json!({"path":"notes.md"})).is_ok());
        assert!(validate_path("src/caf\u{e9}.rs").is_ok());
    }
    /// Audit F-EF1, object level: whatever spelling the filesystem accepts
    /// for an existing protected object, the identity check refuses it.
    #[test]
    fn f_ef1_identity_alias_of_a_protected_object_is_refused() {
        let (_dir, e, ws) = fixture();
        std::fs::create_dir(ws.join(".ssh")).unwrap();
        let (root, _) = e.roots().unwrap();
        // The literal spelling is the string check's job; a different case
        // on an insensitive volume is the same inode.
        if ws.join(".SSH").exists() {
            assert!(aliases_protected(&root, Path::new(".SSH/config")));
        }
        // On a case/normalization-insensitive volume (APFS default) the
        // folded spelling resolves to the same inode and is caught by
        // identity too, independently of the string fold.
        if ws.join(".\u{17f}\u{17f}h").exists() {
            assert!(aliases_protected(
                &root,
                Path::new(".\u{17f}\u{17f}h/anything")
            ));
        }
        std::fs::create_dir(ws.join("src")).unwrap();
        assert!(!aliases_protected(&root, Path::new("src/main.rs")));
    }
    /// Folded protected names on disk never show up in discovery either.
    #[test]
    fn f_ef1_folded_protected_names_are_hidden_from_discovery() {
        let (_dir, e, ws) = fixture();
        std::fs::create_dir(ws.join(".\u{17f}\u{17f}h")).unwrap();
        std::fs::write(ws.join(".\u{17f}\u{17f}h/key"), "secret-canary").unwrap();
        std::fs::write(ws.join("credential\u{17f}.json"), "secret-canary").unwrap();
        std::fs::write(ws.join("public.md"), "safe").unwrap();
        assert_eq!(
            e.execute("fs_list", &json!({})).unwrap()["entries"],
            json!(["public.md"])
        );
        assert_eq!(
            e.execute("fs_glob", &json!({"pattern":"**"})).unwrap()["files"],
            json!(["public.md"])
        );
        assert_eq!(
            e.execute("fs_grep", &json!({"pattern":"secret-canary"}))
                .unwrap()["matches"],
            json!([])
        );
    }
    /// Audit F-EF2 (reproduction turned regression): one symlink, a FIFO, a
    /// binary file or a huge protected `.git` no longer fail the whole tree.
    #[test]
    fn f_ef2_discovery_skips_links_and_heavy_dirs_and_reports_them() {
        let (_dir, e, ws) = fixture();
        std::fs::write(ws.join("README.txt"), "hello").unwrap();
        std::fs::create_dir_all(ws.join("node_modules/.bin")).unwrap();
        std::fs::write(ws.join("node_modules/dep.js"), "hello from dep").unwrap();
        std::os::unix::fs::symlink("../x/cli.js", ws.join("node_modules/.bin/x")).unwrap();
        std::os::unix::fs::symlink("README.txt", ws.join("link.txt")).unwrap();
        std::fs::write(
            ws.join("logo.png"),
            [0x89u8, b'P', b'N', b'G', 0, 0, 0xff, 0xfe],
        )
        .unwrap();
        let fifo = std::ffi::CString::new(ws.join("pipe").to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let list = e.execute("fs_list", &json!({})).unwrap();
        assert_eq!(
            list["entries"],
            json!(["README.txt", "logo.png", "node_modules/"])
        );
        let skipped = list["skipped"]["entries"].to_string();
        assert!(
            skipped.contains("link.txt") && skipped.contains("symlink"),
            "{skipped}"
        );
        assert!(
            skipped.contains("pipe") && skipped.contains("special"),
            "{skipped}"
        );
        assert!(
            skipped.contains("\"node_modules\"") && skipped.contains("ignored_dir"),
            "{skipped}"
        );
        let grep = e.execute("fs_grep", &json!({"pattern":"hello"})).unwrap();
        assert_eq!(grep["matches"].as_array().unwrap().len(), 1, "{grep}");
        assert!(
            grep["skipped"]["entries"].to_string().contains("logo.png"),
            "{grep}"
        );
        // Looking inside an ignored folder is explicit.
        let inside = e
            .execute("fs_grep", &json!({"path":"node_modules","pattern":"hello"}))
            .unwrap();
        assert_eq!(inside["matches"][0]["path"], "node_modules/dep.js");
        assert!(inside["skipped"]["entries"]
            .to_string()
            .contains("node_modules/.bin/x"));
        assert_eq!(
            e.execute("fs_glob", &json!({"pattern":"*.txt"})).unwrap()["files"],
            json!(["README.txt"])
        );
        // A protected .git with more entries than the limit is neither opened nor counted.
        std::fs::remove_dir_all(ws.join("node_modules")).unwrap();
        std::fs::create_dir_all(ws.join(".git/objects")).unwrap();
        for i in 0..(MAX_TREE + 1) {
            std::fs::write(ws.join(format!(".git/objects/{i}")), "").unwrap();
        }
        assert!(e.execute("fs_list", &json!({})).is_ok());
        // Real overflow names the limit and the entry.
        std::fs::create_dir(ws.join("many")).unwrap();
        for i in 0..(MAX_TREE + 1) {
            std::fs::write(ws.join(format!("many/{i}")), "").unwrap();
        }
        let err = e.execute("fs_list", &json!({})).unwrap_err();
        assert!(
            err.starts_with("FILE_TREE_LIMIT")
                && err.contains(&format!("more than {MAX_TREE} entries"))
                && err.contains("many/"),
            "{err}"
        );
    }
    /// Audit F-R1 through the tool: the concurrent save wins and the error
    /// names the recovered sibling.
    #[test]
    fn f_r1_fs_edit_reports_where_the_displaced_revision_was_kept() {
        let (_dir, e, ws) = fixture();
        std::fs::write(ws.join("t.md"), "base\n").unwrap();
        let d = read_digest(&e, "t.md");
        let _c = Clear;
        let w = ws.clone();
        hook::set(move |s| {
            if s == "after_swap" {
                std::fs::write(w.join("t.tmp"), "EDITOR\n").unwrap();
                std::fs::rename(w.join("t.tmp"), w.join("t.md")).unwrap();
            }
        });
        let err = e
            .execute(
                "fs_edit",
                &json!({"path":"t.md","expected_sha256":d,"content":"ours\n"}),
            )
            .unwrap_err();
        hook::clear();
        assert!(
            err.starts_with("FILE_CHANGED_DURING_COMMIT; recovered: t.md.omniget-recovered-"),
            "{err}"
        );
        assert_eq!(
            std::fs::read_to_string(ws.join("t.md")).unwrap(),
            "EDITOR\n"
        );
        let rel = err.split("; recovered: ").nth(1).unwrap();
        assert_eq!(std::fs::read_to_string(ws.join(rel)).unwrap(), "base\n");
        // The recovered sibling is an ordinary, readable workspace file.
        assert!(e.execute("fs_read", &json!({"path":rel})).is_ok());
    }
}
