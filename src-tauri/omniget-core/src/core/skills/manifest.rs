use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{
    SkillError, SkillSource, ERR_SKILL_DESCRIPTION, ERR_SKILL_NAME, ERR_SKILL_NOT_FOUND,
    ERR_SKILL_PARSE, ERR_SKILL_TOO_BIG,
};

pub const SKILL_FILE: &str = "SKILL.md";
pub const MAX_SKILL_MD_BYTES: u64 = 256 * 1024;
pub const MAX_NAME_CHARS: usize = 64;
pub const MAX_DESCRIPTION_CHARS: usize = 1024;
pub const MAX_COMPATIBILITY_CHARS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillManifest {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    pub path: PathBuf,
    #[serde(default)]
    pub source: SkillSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub body_bytes: usize,
    #[serde(default)]
    pub scan: super::scan::ScanStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Frontmatter {
    pub name: String,
    pub description: String,
    pub allowed_tools: Vec<String>,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

pub fn parse(dir: &Path) -> Result<SkillManifest, SkillError> {
    let file = dir.join(SKILL_FILE);
    let meta = std::fs::metadata(&file).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SkillError::new(
                ERR_SKILL_NOT_FOUND,
                format!("{} has no {SKILL_FILE}", dir.display()),
            )
        } else {
            SkillError::io(&format!("reading {}", file.display()), &e)
        }
    })?;
    if !meta.is_file() {
        return Err(SkillError::new(
            ERR_SKILL_PARSE,
            format!("{} is not a file", file.display()),
        ));
    }
    if meta.len() > MAX_SKILL_MD_BYTES {
        return Err(SkillError::new(
            ERR_SKILL_TOO_BIG,
            format!(
                "{} is {} bytes, over the {MAX_SKILL_MD_BYTES} byte cap",
                file.display(),
                meta.len()
            ),
        ));
    }
    let bytes = std::fs::read(&file)
        .map_err(|e| SkillError::io(&format!("reading {}", file.display()), &e))?;
    let text = String::from_utf8(bytes).map_err(|_| {
        SkillError::new(
            ERR_SKILL_PARSE,
            format!("{} is not valid UTF-8", file.display()),
        )
    })?;
    let (front, body) = parse_text(&text)?;
    let sidecar = read_sidecar(dir);
    Ok(SkillManifest {
        name: front.name,
        description: front.description,
        allowed_tools: front.allowed_tools,
        path: dir.to_path_buf(),
        scan: sidecar.as_ref().map(|s| s.scan.clone()).unwrap_or_default(),
        source: sidecar.map(|s| s.source).unwrap_or_default(),
        license: front.license,
        compatibility: front.compatibility,
        metadata: front.metadata,
        body_bytes: body.len(),
    })
}

pub fn parse_text(text: &str) -> Result<(Frontmatter, &str), SkillError> {
    let (front_text, body) = split_frontmatter(text)?;
    let front = parse_frontmatter(front_text)?;
    Ok((front, body))
}

fn read_sidecar(dir: &Path) -> Option<super::install::Sidecar> {
    let path = dir.join(super::install::SIDECAR);
    let raw = std::fs::read(&path).ok()?;
    serde_json::from_slice::<super::install::Sidecar>(&raw).ok()
}

pub fn split_frontmatter(text: &str) -> Result<(&str, &str), SkillError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
        .ok_or_else(|| {
            SkillError::new(
                ERR_SKILL_PARSE,
                format!("{SKILL_FILE} must open with a `---` frontmatter fence"),
            )
        })?;
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" || trimmed == "..." {
            return Ok((&rest[..offset], &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    Err(SkillError::new(
        ERR_SKILL_PARSE,
        "frontmatter is never closed by a `---` line",
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Scalar(String),
    Seq(Vec<String>),
    Map(BTreeMap<String, String>),
}

pub fn parse_frontmatter(front: &str) -> Result<Frontmatter, SkillError> {
    let fields = parse_fields(front)?;
    let get = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v);

    let name = match get("name") {
        Some(Node::Scalar(s)) => validate_name(s.trim())?,
        Some(_) => {
            return Err(SkillError::new(
                ERR_SKILL_NAME,
                "`name` must be a plain string",
            ))
        }
        None => {
            return Err(SkillError::new(
                ERR_SKILL_NAME,
                "frontmatter has no `name` field",
            ))
        }
    };

    let description = match get("description") {
        Some(Node::Scalar(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err(SkillError::new(
                    ERR_SKILL_DESCRIPTION,
                    "`description` is empty",
                ));
            }
            truncate_chars(trimmed, MAX_DESCRIPTION_CHARS)
        }
        Some(_) => {
            return Err(SkillError::new(
                ERR_SKILL_DESCRIPTION,
                "`description` must be a plain string",
            ))
        }
        None => {
            return Err(SkillError::new(
                ERR_SKILL_DESCRIPTION,
                "frontmatter has no `description` field",
            ))
        }
    };

    let allowed_tools = match get("allowed-tools").or_else(|| get("allowed_tools")) {
        Some(Node::Scalar(s)) => split_tool_list(s),
        Some(Node::Seq(items)) => items
            .iter()
            .flat_map(|item| split_tool_list(item))
            .collect(),
        Some(Node::Map(_)) | None => Vec::new(),
    };

    let license = match get("license") {
        Some(Node::Scalar(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    };
    let compatibility = match get("compatibility") {
        Some(Node::Scalar(s)) if !s.trim().is_empty() => {
            Some(truncate_chars(s.trim(), MAX_COMPATIBILITY_CHARS))
        }
        _ => None,
    };
    let metadata = match get("metadata") {
        Some(Node::Map(map)) => map.clone(),
        _ => BTreeMap::new(),
    };

    Ok(Frontmatter {
        name,
        description,
        allowed_tools,
        license,
        compatibility,
        metadata,
    })
}

pub fn validate_name(name: &str) -> Result<String, SkillError> {
    let bad = |why: &str| {
        SkillError::new(
            ERR_SKILL_NAME,
            format!("`{name}` is not a skill name: {why}"),
        )
    };
    if name.is_empty() {
        return Err(bad("empty"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(bad("over 64 characters"));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err(bad("starts or ends with a hyphen"));
    }
    if name.contains("--") {
        return Err(bad("has consecutive hyphens"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(bad(
            "only lowercase letters, digits and hyphens are allowed",
        ));
    }
    Ok(name.to_string())
}

fn split_tool_list(raw: &str) -> Vec<String> {
    raw.split([',', ' ', '\t', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn truncate_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        None => text.to_string(),
        Some((idx, _)) => text[..idx].to_string(),
    }
}

fn parse_fields(front: &str) -> Result<Vec<(String, Node)>, SkillError> {
    let lines: Vec<&str> = front.lines().collect();
    let mut out: Vec<(String, Node)> = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        if line.starts_with([' ', '\t']) {
            return Err(SkillError::new(
                ERR_SKILL_PARSE,
                format!(
                    "frontmatter line {} is indented with no key above it",
                    i + 1
                ),
            ));
        }
        let Some((key, rest)) = line.split_once(':') else {
            return Err(SkillError::new(
                ERR_SKILL_PARSE,
                format!("frontmatter line {} is not `key: value`", i + 1),
            ));
        };
        let key = key.trim().to_string();
        if key.is_empty() {
            return Err(SkillError::new(
                ERR_SKILL_PARSE,
                format!("frontmatter line {} has an empty key", i + 1),
            ));
        }
        let rest = rest.trim();
        i += 1;
        let node = if let Some(style) = rest.strip_prefix(['|', '>']) {
            let folded = rest.starts_with('>');
            let keep = style.contains('+');
            let block = take_indented(&lines, &mut i);
            Node::Scalar(block_scalar(&block, folded, keep))
        } else if rest.is_empty() {
            let block = take_indented(&lines, &mut i);
            nested_node(&block, i)?
        } else if rest.starts_with('[') {
            Node::Seq(flow_seq(rest))
        } else {
            let more = take_indented(&lines, &mut i);
            if more.iter().all(|l| l.trim().is_empty()) {
                Node::Scalar(unquote(rest))
            } else {
                let mut block = vec![rest.to_string()];
                block.extend(more);
                Node::Scalar(unquote(&block_scalar(&block, true, false)))
            }
        };
        out.push((key, node));
    }
    Ok(out)
}

fn take_indented(lines: &[&str], i: &mut usize) -> Vec<String> {
    let mut raw: Vec<&str> = Vec::new();
    while *i < lines.len() {
        let line = lines[*i];
        if line.trim().is_empty() || line.starts_with([' ', '\t']) {
            raw.push(line);
            *i += 1;
        } else {
            break;
        }
    }
    while raw.last().is_some_and(|l| l.trim().is_empty()) {
        raw.pop();
    }
    let indent = raw
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    raw.iter()
        .map(|l| {
            if l.len() >= indent {
                l[indent..].to_string()
            } else {
                String::new()
            }
        })
        .collect()
}

fn block_scalar(block: &[String], folded: bool, keep: bool) -> String {
    let mut text = if folded {
        let mut out = String::new();
        for line in block {
            if line.trim().is_empty() {
                out.push('\n');
            } else {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push(' ');
                }
                out.push_str(line.trim_end());
            }
        }
        out
    } else {
        block.join("\n")
    };
    if keep {
        text.push('\n');
    } else {
        text = text.trim_end().to_string();
    }
    text
}

fn nested_node(block: &[String], line_no: usize) -> Result<Node, SkillError> {
    let first = block.iter().find(|l| !l.trim().is_empty());
    let Some(first) = first else {
        return Ok(Node::Scalar(String::new()));
    };
    if first.trim_start().starts_with("- ") || first.trim() == "-" {
        let items = block
            .iter()
            .filter_map(|l| l.trim().strip_prefix('-'))
            .map(|v| unquote(v.trim()))
            .filter(|v| !v.is_empty())
            .collect();
        return Ok(Node::Seq(items));
    }
    let prose = block
        .iter()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .any(|l| !l.contains(": ") && !l.trim_end().ends_with(':'));
    if prose {
        return Ok(Node::Scalar(block_scalar(block, true, false)));
    }
    let mut map = BTreeMap::new();
    for line in block {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            return Err(SkillError::new(
                ERR_SKILL_PARSE,
                format!("nested frontmatter near line {line_no} is not `key: value`"),
            ));
        };
        map.insert(k.trim().to_string(), unquote(v.trim()));
    }
    Ok(Node::Map(map))
}

fn flow_seq(raw: &str) -> Vec<String> {
    let inner = raw
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim();
    inner
        .split(',')
        .map(|s| unquote(s.trim()))
        .filter(|s| !s.is_empty())
        .collect()
}

fn unquote(raw: &str) -> String {
    let raw = raw.trim();
    if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        let inner = &raw[1..raw.len() - 1];
        return inner
            .replace("\\\"", "\"")
            .replace("\\n", "\n")
            .replace("\\\\", "\\");
    }
    if raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'') {
        return raw[1..raw.len() - 1].replace("''", "'");
    }
    raw.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_description_wrapped_over_indented_lines_is_one_scalar() {
        let src = "---\nname: composition-patterns\ndescription:\n  React composition patterns that scale. Use when refactoring\n  designing reusable APIs. Includes React 19\n  API changes.\nlicense: MIT\nmetadata:\n  author: vercel\n---\nbody\n";
        let (m, _) = parse_text(src).expect("parses");
        assert_eq!(m.name, "composition-patterns");
        assert!(m.description.starts_with("React composition patterns"));
        assert!(m.description.ends_with("API changes."));
        let inline = "---\nname: x\ndescription: first line\n  second line\n---\nbody\n";
        assert_eq!(
            parse_text(inline).expect("parses").0.description,
            "first line second line"
        );
    }

    const GOOD: &str = "---\nname: pdf-forms\ndescription: Fills PDF forms. Use when the user has a form to fill.\nallowed-tools: Read Bash(pdftk:*)\nlicense: MIT\n---\n\n# PDF forms\n\nBody.\n";

    #[test]
    fn parses_a_minimal_skill() {
        let (front, body) = parse_text(GOOD).unwrap();
        assert_eq!(front.name, "pdf-forms");
        assert!(front.description.starts_with("Fills PDF forms."));
        assert_eq!(front.allowed_tools, vec!["Read", "Bash(pdftk:*)"]);
        assert_eq!(front.license.as_deref(), Some("MIT"));
        assert!(body.trim_start().starts_with("# PDF forms"));
    }

    #[test]
    fn accepts_crlf_and_a_bom() {
        let text = "\u{feff}---\r\nname: crlf-skill\r\ndescription: Works on Windows checkouts.\r\n---\r\nBody\r\n";
        let (front, body) = parse_text(text).unwrap();
        assert_eq!(front.name, "crlf-skill");
        assert!(body.contains("Body"));
    }

    #[test]
    fn rejects_a_file_with_no_fence() {
        let err = parse_text("# just markdown\n").unwrap_err();
        assert_eq!(err.code(), ERR_SKILL_PARSE);
    }

    #[test]
    fn rejects_unterminated_frontmatter() {
        let err = parse_text("---\nname: x\ndescription: y\n").unwrap_err();
        assert_eq!(err.code(), ERR_SKILL_PARSE);
        assert!(err.message.contains("never closed"), "{}", err.message);
    }

    #[test]
    fn rejects_a_line_that_is_not_key_value() {
        let err = parse_text("---\nname pdf\ndescription: y\n---\n").unwrap_err();
        assert_eq!(err.code(), ERR_SKILL_PARSE);
    }

    #[test]
    fn rejects_a_missing_name() {
        let err = parse_text("---\ndescription: y\n---\n").unwrap_err();
        assert_eq!(err.code(), ERR_SKILL_NAME);
    }

    #[test]
    fn rejects_a_missing_or_empty_description() {
        let err = parse_text("---\nname: ok-skill\n---\n").unwrap_err();
        assert_eq!(err.code(), ERR_SKILL_DESCRIPTION);
        let err = parse_text("---\nname: ok-skill\ndescription: \"\"\n---\n").unwrap_err();
        assert_eq!(err.code(), ERR_SKILL_DESCRIPTION);
    }

    #[test]
    fn name_rules_follow_the_spec() {
        assert!(validate_name("pdf-processing").is_ok());
        assert!(validate_name("a1").is_ok());
        for bad in [
            "PDF-Processing",
            "-pdf",
            "pdf-",
            "pdf--processing",
            "",
            "pdf_proc",
            "pdf/../x",
        ] {
            let err = validate_name(bad).unwrap_err();
            assert_eq!(err.code(), ERR_SKILL_NAME, "{bad} should be rejected");
        }
        let long = "a".repeat(65);
        assert!(validate_name(&long).is_err());
        assert!(validate_name(&"a".repeat(64)).is_ok());
    }

    #[test]
    fn truncates_an_over_long_description() {
        let long = "x".repeat(2000);
        let text = format!("---\nname: long-skill\ndescription: {long}\n---\nbody\n");
        let (front, _) = parse_text(&text).unwrap();
        assert_eq!(front.description.chars().count(), MAX_DESCRIPTION_CHARS);
    }

    #[test]
    fn reads_block_scalars_and_nested_metadata() {
        let text = "---\nname: block-skill\ndescription: >-\n  A folded description\n  over two lines.\nmetadata:\n  author: tonho\n  version: \"1.0\"\nallowed-tools:\n  - Read\n  - Bash(jq:*)\n---\nbody\n";
        let (front, _) = parse_text(text).unwrap();
        assert_eq!(front.description, "A folded description over two lines.");
        assert_eq!(
            front.metadata.get("author").map(String::as_str),
            Some("tonho")
        );
        assert_eq!(
            front.metadata.get("version").map(String::as_str),
            Some("1.0")
        );
        assert_eq!(front.allowed_tools, vec!["Read", "Bash(jq:*)"]);
    }

    #[test]
    fn reads_a_comma_separated_and_a_flow_tool_list() {
        let text =
            "---\nname: tools-skill\ndescription: d\nallowed-tools: Agent, WebSearch, Read\n---\n";
        let (front, _) = parse_text(text).unwrap();
        assert_eq!(front.allowed_tools, vec!["Agent", "WebSearch", "Read"]);
        let text =
            "---\nname: tools-skill\ndescription: d\nallowed-tools: [Read, \"Bash(git:*)\"]\n---\n";
        let (front, _) = parse_text(text).unwrap();
        assert_eq!(front.allowed_tools, vec!["Read", "Bash(git:*)"]);
    }

    #[test]
    fn keeps_a_hash_inside_a_description() {
        let text = "---\nname: hash-skill\ndescription: Use for C# and # headings.\n---\n";
        let (front, _) = parse_text(text).unwrap();
        assert_eq!(front.description, "Use for C# and # headings.");
    }

    #[test]
    fn parse_reads_a_directory_and_reports_body_size() {
        let dir = crate::core::skills::install::tests_support::temp_dir("manifest-parse");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(SKILL_FILE), GOOD).unwrap();
        let manifest = parse(&dir).unwrap();
        assert_eq!(manifest.name, "pdf-forms");
        assert_eq!(manifest.path, dir);
        assert_eq!(manifest.source, SkillSource::Unknown);
        assert!(manifest.body_bytes > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_says_not_found_when_there_is_no_skill_md() {
        let dir = crate::core::skills::install::tests_support::temp_dir("manifest-missing");
        std::fs::create_dir_all(&dir).unwrap();
        let err = parse(&dir).unwrap_err();
        assert_eq!(err.code(), ERR_SKILL_NOT_FOUND);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_rejects_an_oversized_skill_md() {
        let dir = crate::core::skills::install::tests_support::temp_dir("manifest-big");
        std::fs::create_dir_all(&dir).unwrap();
        let mut text = String::from("---\nname: big-skill\ndescription: d\n---\n");
        text.push_str(&"x".repeat(MAX_SKILL_MD_BYTES as usize + 1));
        std::fs::write(dir.join(SKILL_FILE), text).unwrap();
        let err = parse(&dir).unwrap_err();
        assert_eq!(err.code(), ERR_SKILL_TOO_BIG);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_every_bundled_fixture_that_should_be_valid() {
        let root = fixtures();
        for name in ["good-skill", "metadata-skill", "windows-skill"] {
            let manifest = parse(&root.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!manifest.description.is_empty(), "{name}");
        }
    }

    #[test]
    fn rejects_every_bundled_bad_fixture() {
        let root = fixtures();
        for (name, code) in [
            ("bad-fence", ERR_SKILL_PARSE),
            ("bad-name", ERR_SKILL_NAME),
            ("no-description", ERR_SKILL_DESCRIPTION),
        ] {
            let err = parse(&root.join(name)).unwrap_err();
            assert_eq!(err.code(), code, "{name}");
        }
    }

    #[test]
    fn parse_stays_under_one_millisecond() {
        let root = fixtures().join("good-skill");
        let _ = parse(&root).unwrap();
        let runs = 200;
        let started = std::time::Instant::now();
        for _ in 0..runs {
            let _ = parse(&root).unwrap();
        }
        let per = started.elapsed() / runs;
        println!("skills::manifest::parse = {per:?} per call over {runs} runs");
        assert!(
            per < std::time::Duration::from_millis(1),
            "parse took {per:?}"
        );
    }

    fn fixtures() -> PathBuf {
        static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        ROOT.get_or_init(|| {
            let source = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("skills_fixtures");
            let target = std::env::temp_dir()
                .join(format!("omniget-skills-fixtures-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&target);
            copy_fixtures(&source, &target);
            target
        })
        .clone()
    }

    fn copy_fixtures(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                copy_fixtures(&path, &to.join(&name));
            } else if let Some(real) = name.strip_suffix(".fixture") {
                std::fs::copy(&path, to.join(real)).unwrap();
            } else if !name.ends_with(".md") {
                std::fs::copy(&path, to.join(&name)).unwrap();
            }
        }
    }
}
