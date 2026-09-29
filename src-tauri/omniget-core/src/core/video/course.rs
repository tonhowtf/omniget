//! Turns the files of a course folder into modules and numbered lessons in
//! natural order, and ties attachments (PDF, code, subtitles) to lessons.

use super::naming::{clean_title, fold, is_video_ext, natural_cmp, split_ext};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A file found under the course root. `path` is relative to the root.
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

/// Kind of a non-video file.
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

/// Classifies a file by extension.
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

/// A lesson of a module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LessonRef {
    pub path: String,
    /// Display number: `2.10` (module 2, 10th lesson) or `3` for loose files
    /// in a course without modules.
    pub number: String,
    pub title: String,
}

/// A module (first-level folder). Index 0 holds loose files ("Geral").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    pub index: u32,
    pub title: String,
    pub lessons: Vec<LessonRef>,
}

/// A non-video file, tied to a lesson when one matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CourseAttachment {
    pub path: String,
    pub name: String,
    pub kind: AttachmentKind,
    pub lesson_path: Option<String>,
}

/// The whole course.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CourseTree {
    pub title: String,
    pub modules: Vec<Module>,
    pub attachments: Vec<CourseAttachment>,
}

/// Title of the module of loose files.
pub const LOOSE_MODULE_TITLE: &str = "Geral";

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

/// [`build_titled`] without a root name: the title comes from a single
/// wrapping folder, if any.
pub fn build(files: &[CourseFile]) -> CourseTree {
    build_titled("", files)
}

struct Entry {
    path: String,
    comps: Vec<String>,
}

/// Builds the course tree. `root_name` is the course folder name (cleaned
/// into the title).
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

    // A single wrapping folder ("Course/Module 1/...") is the course itself.
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

    // Group videos by first-level folder under the (unwrapped) root.
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

    // Lessons with their folder, for attachment matching.
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

/// True for folder names that are modules, never a wrapping course folder.
fn looks_like_module(name: &str) -> bool {
    const WORDS: &[&str] = &[
        "modulo", "module", "mod ", "secao", "section", "parte", "part ", "unidade", "unit ",
        "capitulo", "chapter", "semana", "week", "aula", "lesson",
    ];
    let f = fold(name.trim());
    lead_numbers(name).is_some() || WORDS.iter().any(|w| f.starts_with(w))
}

/// Leading number sequence of a name (`03 - x` gives `[3]`, `1.2 y` gives
/// `[1, 2]`).
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
    // Subtitles keep their language after the video name: "x.pt-BR.srt".
    let att_base = att_stem.split('.').next().unwrap_or(att_stem);
    let att_nums = lead_numbers(att_stem);
    let mut dir: Vec<String> = att.comps[..att.comps.len() - 1].to_vec();
    loop {
        let here = dir.join("/");
        let cands: Vec<&(&Entry, String)> = lessons.iter().filter(|(_, d)| *d == here).collect();
        if !cands.is_empty() {
            let stem_of = |e: &Entry| split_ext(e.comps.last().unwrap()).0.to_string();
            // 1. Same name (or name + language suffix).
            if let Some((e, _)) = cands.iter().find(|(e, _)| {
                let s = stem_of(e);
                att_stem == s || att_stem.starts_with(&format!("{s}.")) || att_base == s
            }) {
                return Some(e.path.clone());
            }
            // 2. Same leading number.
            if let Some(n) = &att_nums {
                if let Some((e, _)) = cands
                    .iter()
                    .find(|(e, _)| lead_numbers(&stem_of(e)).as_ref() == Some(n))
                {
                    return Some(e.path.clone());
                }
            }
            // 3. Similar name.
            let best = cands
                .iter()
                .map(|(e, _)| (similarity(att_stem, &stem_of(e)), e))
                .filter(|(s, _)| *s >= 0.5)
                .max_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, e)) = best {
                return Some(e.path.clone());
            }
        }
        // Attachments in "Materials/" look in the parent folder, down to the
        // module folder.
        if dir.len() <= depth + 1 {
            break;
        }
        dir.pop();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(list: &[&str]) -> Vec<CourseFile> {
        list.iter().map(|p| CourseFile::from(*p)).collect()
    }

    #[test]
    fn modules_and_natural_order() {
        let tree = build_titled(
            "/home/me/Cursos/03 - Rust Avançado",
            &files(&[
                "Módulo 10 - Final/01 - Fim.mp4",
                "Módulo 2 - Base/10 - Dez.mp4",
                "Módulo 2 - Base/2 - Dois.mp4",
                "Módulo 2 - Base/1 - Um.mp4",
                "Módulo 1 - Intro/01 - Olá.mkv",
                "Boas-vindas.mp4",
                ".DS_Store",
                "Módulo 1 - Intro/.hidden.mp4",
                "Módulo 1 - Intro/download.mp4.part",
            ]),
        );
        assert_eq!(tree.title, "Rust Avançado");
        let titles: Vec<(u32, &str)> = tree
            .modules
            .iter()
            .map(|m| (m.index, m.title.as_str()))
            .collect();
        assert_eq!(
            titles,
            vec![(0, "Geral"), (1, "Intro"), (2, "Base"), (3, "Final")]
        );
        let base = &tree.modules[2];
        let got: Vec<(&str, &str)> = base
            .lessons
            .iter()
            .map(|l| (l.number.as_str(), l.title.as_str()))
            .collect();
        assert_eq!(got, vec![("2.1", "Um"), ("2.2", "Dois"), ("2.3", "Dez")]);
        assert_eq!(tree.modules[0].lessons[0].number, "0.1");
        assert_eq!(tree.modules[1].lessons.len(), 1);
    }

    #[test]
    fn flat_course_and_wrapper_folder() {
        let tree = build(&files(&[
            "Curso de Python/10 - Classes.mp4",
            "Curso de Python/2 - Listas.mp4",
            "Curso de Python/1 - Intro.mp4",
            "Curso de Python/1 - Intro.pdf",
        ]));
        assert_eq!(tree.title, "Curso de Python");
        assert_eq!(tree.modules.len(), 1);
        let m = &tree.modules[0];
        assert_eq!(m.index, 0);
        let got: Vec<(&str, &str)> = m
            .lessons
            .iter()
            .map(|l| (l.number.as_str(), l.title.as_str()))
            .collect();
        assert_eq!(got, vec![("1", "Intro"), ("2", "Listas"), ("3", "Classes")]);
        assert_eq!(
            tree.attachments[0].lesson_path.as_deref(),
            Some("Curso de Python/1 - Intro.mp4")
        );
    }

    #[test]
    fn attachments_link_to_lessons() {
        let tree = build(&files(&[
            "M1/01 - Setup do ambiente.mp4",
            "M1/02 - Variáveis.mp4",
            "M1/02 - Variaveis.pt-BR.srt",
            "M1/01 - slides.pdf",
            "M1/Materiais/setup ambiente checklist.docx",
            "M1/codigo.zip",
            "M1/Materiais/02_exercicios.py",
            "leia-me.txt",
        ]));
        let find = |name: &str| {
            tree.attachments
                .iter()
                .find(|a| a.name == name)
                .unwrap()
                .clone()
        };
        assert_eq!(
            find("02 - Variaveis.pt-BR.srt").lesson_path.as_deref(),
            Some("M1/02 - Variáveis.mp4")
        );
        assert_eq!(
            find("02 - Variaveis.pt-BR.srt").kind,
            AttachmentKind::Subtitle
        );
        assert_eq!(
            find("01 - slides.pdf").lesson_path.as_deref(),
            Some("M1/01 - Setup do ambiente.mp4")
        );
        assert_eq!(find("01 - slides.pdf").kind, AttachmentKind::Pdf);
        assert_eq!(
            find("setup ambiente checklist.docx").lesson_path.as_deref(),
            Some("M1/01 - Setup do ambiente.mp4")
        );
        assert_eq!(
            find("02_exercicios.py").lesson_path.as_deref(),
            Some("M1/02 - Variáveis.mp4")
        );
        assert_eq!(find("02_exercicios.py").kind, AttachmentKind::Code);
        assert_eq!(find("codigo.zip").lesson_path, None);
        assert_eq!(find("codigo.zip").kind, AttachmentKind::Archive);
        assert_eq!(find("leia-me.txt").lesson_path, None);
    }

    #[test]
    fn single_module_is_not_unwrapped() {
        let tree = build(&files(&["Módulo 1/a.mp4", "Módulo 1/b.mp4"]));
        assert_eq!(tree.modules.len(), 1);
        assert_eq!(tree.modules[0].index, 1);
        assert_eq!(tree.modules[0].lessons[1].number, "1.2");
    }

    #[test]
    fn empty_and_junk() {
        let tree = build(&[]);
        assert!(tree.modules.is_empty() && tree.attachments.is_empty());
        assert_eq!(tree.title, "");
        let tree = build(&files(&["", "/", "./.git/x.mp4", "a\\b\\c.mp4"]));
        assert_eq!(tree.modules.len(), 1);
        assert_eq!(tree.modules[0].lessons[0].path, "a\\b\\c.mp4");
    }
}
