use super::naming::{clean_title, fold, is_video_ext, natural_cmp, split_ext};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CourseFile {
    pub path: String,
}

impl From<&str> for CourseFile {
    fn from(p: &str) -> Self {
        CourseFile {
            path: p.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentKind {
    Pdf,
    Code,
    Subtitle,
    Archive,
    Image,
    Doc,
    Other,
}

pub fn attachment_kind(path: &str) -> AttachmentKind {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => AttachmentKind::Pdf,
        "srt" | "vtt" | "ass" | "ssa" | "sub" => AttachmentKind::Subtitle,
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" => AttachmentKind::Archive,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" => AttachmentKind::Image,
        "doc" | "docx" | "ppt" | "pptx" | "key" | "xls" | "xlsx" | "csv" | "txt" | "md" | "odt"
        | "odp" | "ods" | "rtf" | "epub" | "pages" | "numbers" => AttachmentKind::Doc,
        "py" | "js" | "mjs" | "ts" | "tsx" | "jsx" | "rs" | "java" | "kt" | "c" | "h" | "cpp"
        | "hpp" | "cc" | "cs" | "go" | "rb" | "php" | "html" | "htm" | "css" | "scss" | "json"
        | "yaml" | "yml" | "toml" | "xml" | "sql" | "sh" | "ps1" | "bat" | "swift" | "dart"
        | "ipynb" | "r" | "lua" | "vue" | "svelte" | "ex" | "exs" | "scala" | "m" => {
            AttachmentKind::Code
        }
        _ => AttachmentKind::Other,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LessonRef {
    pub path: String,
    pub number: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    pub index: u32,
    pub title: String,
    pub lessons: Vec<LessonRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CourseAttachment {
    pub path: String,
    pub name: String,
    pub kind: AttachmentKind,
    pub lesson_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CourseTree {
    pub title: String,
    pub modules: Vec<Module>,
    pub attachments: Vec<CourseAttachment>,
}

pub const LOOSE_MODULE_TITLE: &str = "General";

const JUNK: &[&str] = &["thumbs.db", "desktop.ini", "icon\r", ".ds_store"];

fn is_junk(comps: &[&str]) -> bool {
    comps
        .iter()
        .any(|c| c.starts_with('.') || c.starts_with("__MACOSX"))
        || comps
            .last()
            .is_some_and(|f| JUNK.contains(&f.to_ascii_lowercase().as_str()))
        || comps.last().is_some_and(|f| {
            let l = f.to_ascii_lowercase();
            l.ends_with(".part") || l.ends_with(".crdownload") || l.ends_with(".ytdl")
        })
}

pub fn build(files: &[CourseFile]) -> CourseTree {
    build_titled("", files)
}

struct Entry {
    path: String,
    comps: Vec<String>,
}

pub fn build_titled(root_name: &str, files: &[CourseFile]) -> CourseTree {
    let mut entries: Vec<Entry> = Vec::new();
    for f in files {
        let norm = f.path.replace('\\', "/");
        let comps: Vec<&str> = norm
            .split('/')
            .map(str::trim)
            .filter(|c| !c.is_empty() && *c != ".")
            .collect();
        if comps.is_empty() || is_junk(&comps) {
            continue;
        }
        entries.push(Entry {
            path: f.path.clone(),
            comps: comps.iter().map(|s| s.to_string()).collect(),
        });
    }

    let mut depth = 0;
    let mut wrapper = String::new();
    loop {
        let first = entries.first().and_then(|e| e.comps.get(depth).cloned());
        let Some(first) = first else { break };
        if !looks_like_module(&first)
            && entries
                .iter()
                .all(|e| e.comps.len() > depth + 1 && e.comps[depth] == first)
        {
            wrapper = first;
            depth += 1;
        } else {
            break;
        }
    }
    let title = {
        let src = if root_name.trim().is_empty() {
            wrapper.as_str()
        } else {
            root_name
                .rsplit(['/', '\\'])
                .find(|s| !s.is_empty())
                .unwrap_or(root_name)
        };
        if src.is_empty() {
            String::new()
        } else {
            clean_title(src)
        }
    };

    let mut groups: BTreeMap<String, Vec<&Entry>> = BTreeMap::new();
    let mut loose: Vec<&Entry> = Vec::new();
    let mut attachments_raw: Vec<&Entry> = Vec::new();
    for e in &entries {
        let rel = &e.comps[depth..];
        let file = rel.last().unwrap();
        let (_, ext) = split_ext(file);
        let is_video = ext.as_deref().is_some_and(is_video_ext);
        if !is_video {
            attachments_raw.push(e);
            continue;
        }
        if rel.len() >= 2 {
            groups.entry(rel[0].clone()).or_default().push(e);
        } else {
            loose.push(e);
        }
    }
    let mut dirs: Vec<&String> = groups.keys().collect();
    dirs.sort_by(|a, b| natural_cmp(a, b));
    let has_modules = !dirs.is_empty();

    let rel_of = |e: &Entry| e.comps[depth..].join("/");
    let lesson_title = |e: &Entry| {
        let file = e.comps.last().unwrap();
        let t = clean_title(file);
        if t.is_empty() {
            split_ext(file).0.to_string()
        } else {
            t
        }
    };

    let mut modules = Vec::new();
    if !loose.is_empty() {
        loose.sort_by(|a, b| natural_cmp(&rel_of(a), &rel_of(b)));
        modules.push(Module {
            index: 0,
            title: LOOSE_MODULE_TITLE.to_string(),
            lessons: loose
                .iter()
                .enumerate()
                .map(|(i, e)| LessonRef {
                    path: e.path.clone(),
                    number: if has_modules {
                        format!("0.{}", i + 1)
                    } else {
                        (i + 1).to_string()
                    },
                    title: lesson_title(e),
                })
                .collect(),
        });
    }
    for (mi, d) in dirs.iter().enumerate() {
        let index = mi as u32 + 1;
        let mut list = groups[*d].clone();
        list.sort_by(|a, b| natural_cmp(&rel_of(a), &rel_of(b)));
        let t = clean_title(d);
        modules.push(Module {
            index,
            title: if t.is_empty() { d.to_string() } else { t },
            lessons: list
                .iter()
                .enumerate()
                .map(|(i, e)| LessonRef {
                    path: e.path.clone(),
                    number: format!("{index}.{}", i + 1),
                    title: lesson_title(e),
                })
                .collect(),
        });
    }

    let lessons: Vec<(&Entry, String)> = entries
        .iter()
        .filter(|e| {
            let (_, ext) = split_ext(e.comps.last().unwrap());
            ext.as_deref().is_some_and(is_video_ext)
        })
        .map(|e| (e, e.comps[..e.comps.len() - 1].join("/")))
        .collect();

    let mut attachments: Vec<CourseAttachment> = attachments_raw
        .iter()
        .map(|e| {
            let name = e.comps.last().unwrap().clone();
            CourseAttachment {
                path: e.path.clone(),
                kind: attachment_kind(&name),
                lesson_path: match_lesson(e, &lessons, depth),
                name,
            }
        })
        .collect();
    attachments.sort_by(|a, b| natural_cmp(&a.path, &b.path));

    CourseTree {
        title,
        modules,
        attachments,
    }
}

fn looks_like_module(name: &str) -> bool {
    const WORDS: &[&str] = &[
        "modulo", "module", "mod ", "secao", "section", "parte", "part ", "unidade", "unit ",
        "capitulo", "chapter", "semana", "week", "aula", "lesson",
    ];
    let f = fold(name.trim());
    lead_numbers(name).is_some() || WORDS.iter().any(|w| f.starts_with(w))
}

fn lead_numbers(name: &str) -> Option<Vec<u32>> {
    let s = name.trim_start_matches(['[', '(', ' ']);
    let head: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let nums: Vec<u32> = head
        .split('.')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse().ok())
        .collect();
    (!nums.is_empty()).then_some(nums)
}

fn tokens(name: &str) -> Vec<String> {
    fold(name)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2 && !t.chars().all(|c| c.is_ascii_digit()))
        .map(str::to_string)
        .collect()
}

fn similarity(a: &str, b: &str) -> f64 {
    let ta = tokens(a);
    let tb = tokens(b);
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    let inter = ta.iter().filter(|t| tb.contains(t)).count() as f64;
    let union = (ta.len() + tb.len()) as f64 - inter;
    inter / union
}

fn match_lesson(att: &Entry, lessons: &[(&Entry, String)], depth: usize) -> Option<String> {
    let att_file = att.comps.last()?;
    let att_stem = split_ext(att_file).0;
    let att_base = att_stem.split('.').next().unwrap_or(att_stem);
    let att_nums = lead_numbers(att_stem);
    let mut dir: Vec<String> = att.comps[..att.comps.len() - 1].to_vec();
    loop {
        let here = dir.join("/");
        let cands: Vec<&(&Entry, String)> = lessons.iter().filter(|(_, d)| *d == here).collect();
        if !cands.is_empty() {
            let stem_of = |e: &Entry| split_ext(e.comps.last().unwrap()).0.to_string();
            if let Some((e, _)) = cands.iter().find(|(e, _)| {
                let s = stem_of(e);
                att_stem == s || att_stem.starts_with(&format!("{s}.")) || att_base == s
            }) {
                return Some(e.path.clone());
            }
            if let Some(n) = &att_nums {
                if let Some((e, _)) = cands
                    .iter()
                    .find(|(e, _)| lead_numbers(&stem_of(e)).as_ref() == Some(n))
                {
                    return Some(e.path.clone());
                }
            }
            let best = cands
                .iter()
                .map(|(e, _)| (similarity(att_stem, &stem_of(e)), e))
                .filter(|(s, _)| *s >= 0.5)
                .max_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, e)) = best {
                return Some(e.path.clone());
            }
        }
        if dir.len() <= depth + 1 {
            break;
        }
        dir.pop();
    }
    None
}
