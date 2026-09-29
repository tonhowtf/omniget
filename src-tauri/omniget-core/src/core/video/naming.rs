//! File name parsing for the video library: movies, episodes (SxxEyy, 1x02,
//! season folders, fansub anime), numbered lessons, natural ordering and title
//! cleanup. Everything here is pure and tolerant: bad input degrades to a
//! plain `Video` with a cleaned title.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::sync::LazyLock;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// What a file looks like from its name alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Movie,
    Episode,
    Lesson,
    #[default]
    Video,
}

impl MediaKind {
    /// The lowercase name stored in the database (`movie`, `episode`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            MediaKind::Movie => "movie",
            MediaKind::Episode => "episode",
            MediaKind::Lesson => "lesson",
            MediaKind::Video => "video",
        }
    }
}

/// Result of [`parse`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Parsed {
    pub kind: MediaKind,
    /// Movie title, episode title, lesson title or cleaned file name.
    pub title: String,
    pub show: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    /// Last episode of a multi-episode file (`S01E01-E02` gives 2).
    pub episode_end: Option<u32>,
    pub year: Option<u32>,
    /// Lesson numbering found in the file name (`1.2 Title` gives `[1, 2]`).
    pub lesson_no: Option<Vec<u32>>,
    /// Name of the folder holding the lesson (`Módulo 2`).
    pub module: Option<String>,
    /// Number found in the module folder name (`Módulo 2` gives 2).
    pub module_no: Option<u32>,
    /// Trailer, sample, featurette or anything under an extras folder.
    pub is_extra: bool,
}

/// A piece of a natural sort key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Chunk {
    /// Digits without leading zeros (`"0"` for zero), compared by value.
    Num(String),
    /// Case- and accent-folded text.
    Text(String),
}

impl Ord for Chunk {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Chunk::Num(a), Chunk::Num(b)) => a.len().cmp(&b.len()).then_with(|| a.cmp(b)),
            (Chunk::Text(a), Chunk::Text(b)) => a.cmp(b),
            (Chunk::Num(_), Chunk::Text(_)) => Ordering::Less,
            (Chunk::Text(_), Chunk::Num(_)) => Ordering::Greater,
        }
    }
}

impl PartialOrd for Chunk {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Lowercases and strips diacritics (`Módulo` becomes `modulo`).
pub fn fold(s: &str) -> String {
    s.nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Natural sort key: digit runs compare by value, text ignores case and accents.
pub fn natural_key(s: &str) -> Vec<Chunk> {
    let folded = fold(s);
    let mut out = Vec::new();
    let mut text = String::new();
    let mut num = String::new();
    let mut last_space = false;
    for c in folded.chars() {
        if c.is_ascii_digit() {
            if !text.is_empty() {
                out.push(Chunk::Text(std::mem::take(&mut text)));
            }
            num.push(c);
            last_space = false;
            continue;
        }
        if !num.is_empty() {
            out.push(Chunk::Num(strip_zeros(&num)));
            num.clear();
        }
        let c = if c == '_' || c.is_whitespace() {
            ' '
        } else {
            c
        };
        if c == ' ' {
            if last_space {
                continue;
            }
            last_space = true;
        } else {
            last_space = false;
        }
        text.push(c);
    }
    if !num.is_empty() {
        out.push(Chunk::Num(strip_zeros(&num)));
    }
    if !text.is_empty() {
        out.push(Chunk::Text(text));
    }
    out
}

fn strip_zeros(s: &str) -> String {
    let t = s.trim_start_matches('0');
    if t.is_empty() {
        "0".to_string()
    } else {
        t.to_string()
    }
}

/// Natural order: `2` before `10`, case and accents ignored; ties fall back to
/// the raw strings so the order is total.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    natural_key(a).cmp(&natural_key(b)).then_with(|| a.cmp(b))
}

const VIDEO_EXTS: &[&str] = &[
    "mp4", "m4v", "mkv", "webm", "mov", "avi", "wmv", "flv", "mpg", "mpeg", "m2ts", "mts", "ts",
    "3gp", "ogv", "vob", "divx", "rmvb", "asf", "f4v",
];

const KNOWN_EXTS: &[&str] = &[
    "srt", "vtt", "ass", "ssa", "sub", "idx", "mp3", "m4a", "aac", "flac", "ogg", "opus", "wav",
    "pdf", "zip", "rar", "7z", "txt", "md", "nfo", "jpg", "jpeg", "png", "webp", "gif", "json",
    "html", "part", "strm",
];

/// True for extensions the library treats as video files.
pub fn is_video_ext(ext: &str) -> bool {
    let e = ext.trim_start_matches('.').to_ascii_lowercase();
    VIDEO_EXTS.contains(&e.as_str())
}

/// Splits `name.ext` into stem and lowercase extension when the suffix looks
/// like a real extension (not `2019` or `1080p`).
pub fn split_ext(name: &str) -> (&str, Option<String>) {
    if let Some(i) = name.rfind('.') {
        if i == 0 {
            return (name, None);
        }
        let ext = &name[i + 1..];
        let lower = ext.to_ascii_lowercase();
        let known = VIDEO_EXTS.contains(&lower.as_str()) || KNOWN_EXTS.contains(&lower.as_str());
        let plausible = !ext.is_empty()
            && ext.len() <= 4
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
            && ext.chars().any(|c| c.is_ascii_alphabetic())
            && !RE_QUALITY.is_match(&lower);
        if known || plausible {
            return (&name[..i], Some(lower));
        }
    }
    (name, None)
}

static RE_QUALITY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{3,4}[pi]$").unwrap());

static RE_RELEASE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:^|[\s\[(])(?:2160p|1440p|1080p|1080i|720p|576p|480p|360p|4k|uhd|x264|x265|h\.?264|h\.?265|hevc|av1|xvid|divx|web[-. ]?dl|webrip|web[-. ]?rip|blu[-. ]?ray|bdrip|brrip|bdremux|remux|hdtv|hdrip|dvdrip|dvdscr|hdr10\+?|hdr|dts(?:-hd)?|truehd|atmos|aac(?:2\.0)?|ac3|eac3|ddp?5\.1|10bit|8bit|repack|proper|internal|extended|unrated|remastered|imax|dual[-. ]?audio|dubbed|dublado|legendado|amzn|hmax|dsnp)(?:$|[\s\])\-])",
    )
    .unwrap()
});

static RE_YEAR_PAREN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[(\[]((?:19|20)\d{2})[)\]]").unwrap());

static RE_YEAR_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|\s)((?:19[0-9]|20[0-3])\d)(?:\s|$)").unwrap());

static RE_BRACKETS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[[^\]]*\]|\{[^}]*\}|【[^】]*】").unwrap());

static RE_PAREN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(([^)]*)\)").unwrap());

static RE_LEAD_NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^[\[(]?(?:(?:aula|lesson|lecture|li[cç][aã]o|class|v[ií]deo|chapter|cap[ií]tulo|m[oó]dulo|module|parte|part|unit|unidade|episode|epis[oó]dio|ep)[ ._-]*)?(\d{1,4}(?:\.\d{1,4})*)[\])]?(?:\s*[-–—_.:)|\]]+\s*|\s+|$)",
    )
    .unwrap()
});

static RE_LEAD_KEYWORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:aula|lesson|lecture|li[cç][aã]o|class|v[ií]deo|chapter|cap[ií]tulo|m[oó]dulo|module|parte|part|unit|unidade|episode|epis[oó]dio|ep)[ ._-]*\d",
    )
    .unwrap()
});

static RE_SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// True when the name uses dots or underscores instead of spaces
/// (`Name.2019.1080p`).
fn is_release_style(stem: &str) -> bool {
    !stem.contains(' ') && (stem.matches('.').count() >= 2 || stem.contains('_'))
}

/// Applies the separator style of the whole `stem` to a slice of it.
fn despace_as(stem: &str, part: &str) -> String {
    if is_release_style(stem) {
        part.replace(['.', '_'], " ")
    } else {
        part.replace('_', " ")
    }
}

fn despace(stem: &str) -> String {
    if is_release_style(stem) {
        stem.replace(['.', '_'], " ")
    } else {
        stem.replace('_', " ")
    }
}

fn trim_seps(s: &str) -> String {
    let t = s.trim_matches(|c: char| {
        c.is_whitespace() || matches!(c, '-' | '–' | '—' | '_' | '.' | ':' | '|' | ',' | '~')
    });
    RE_SPACES.replace_all(t, " ").into_owned()
}

/// Removes `[tags]`, `(year)`, release tokens and dotted separators, without
/// touching leading numbers.
fn strip_release(stem: &str) -> String {
    let mut s = despace(stem);
    s = RE_BRACKETS.replace_all(&s, " ").into_owned();
    s = RE_PAREN
        .replace_all(&s, |c: &regex::Captures| {
            let inner = c.get(1).map(|m| m.as_str()).unwrap_or("");
            let t = inner.trim();
            let is_year = t.len() == 4
                && t.chars().all(|ch| ch.is_ascii_digit())
                && (t.starts_with("19") || t.starts_with("20"));
            if is_year || t.is_empty() || RE_RELEASE.is_match(&format!(" {t} ")) {
                " ".to_string()
            } else {
                c.get(0).unwrap().as_str().to_string()
            }
        })
        .into_owned();
    if let Some(m) = RE_RELEASE.find(&s) {
        if !trim_seps(&s[..m.start()]).is_empty() {
            s.truncate(m.start());
        }
    }
    trim_seps(&s)
}

/// Cleans a title: drops the extension, leading numbering (`01 - `, `1.2 `,
/// `Aula 3 - `), `[tags]`, release tokens and separators. Never returns an
/// empty string for a non-empty input.
pub fn clean_title(s: &str) -> String {
    let base = s.rsplit(['/', '\\']).next().unwrap_or(s);
    let (stem, _) = split_ext(base);
    let released = strip_release(stem);
    let mut t = released.clone();
    // Trailing year of a release-style name ("Name 2019").
    if let Some(c) = RE_YEAR_TOKEN.captures_iter(&t).last() {
        let m = c.get(1).unwrap();
        if m.start() > 0 && trim_seps(&t[m.end()..]).is_empty() && is_release_style(stem) {
            t = trim_seps(&t[..m.start()]);
        }
    }
    let stripped = strip_lead_number(&t);
    if !stripped.is_empty() {
        return stripped;
    }
    if !t.is_empty() {
        return t;
    }
    let raw = trim_seps(stem);
    if raw.is_empty() {
        trim_seps(base)
    } else {
        raw
    }
}

/// Strips a leading lesson number when it is clearly numbering: a keyword
/// (`Aula 3`), a separator (`3 - `, `3. `), a leading zero or a dotted number.
fn strip_lead_number(t: &str) -> String {
    if let Some(c) = RE_LEAD_NUMBER.captures(t) {
        let whole = c.get(0).unwrap();
        let num = c.get(1).unwrap().as_str();
        let sep = &t[c.get(1).unwrap().end()..whole.end()];
        let strong = RE_LEAD_KEYWORD.is_match(t)
            || num.starts_with('0')
            || num.contains('.')
            || sep.trim().chars().any(|ch| !ch.is_whitespace())
            || t.starts_with(['[', '(']);
        if strong {
            let rest = trim_seps(&t[whole.end()..]);
            if !rest.is_empty() {
                return rest;
            }
        }
    }
    t.to_string()
}

// ---- episode patterns ----

static RE_SXE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:^|[^a-z0-9])s(\d{1,3})[ ._-]*e(\d{1,4})(?:[ ._-]*-?[ ._-]*e(\d{1,4})|-(\d{1,4}))?(?:$|[^0-9a-z])",
    )
    .unwrap()
});

static RE_NXN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[^a-z0-9])(\d{1,2})x(\d{1,3})(?:-(?:\d{1,2}x)?(\d{1,3}))?(?:$|[^0-9a-z])")
        .unwrap()
});

static RE_FANSUB: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*(?:\[[^\]]*\]\s*)+(.+?)\s*-\s*(\d{1,4})(?:v\d)?(?:\s*-\s*(\d{1,4})(?:v\d)?)?(?:\s*[\[(]|\s*$|\s+[^\d\s])",
    )
    .unwrap()
});

static RE_ANIME_TAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^\d\[(].*?)\s+-\s+(\d{1,4})(?:v\d)?\s*[\[(]").unwrap());

static RE_SEASON_DIR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:season|temporada|saison|staffel|stagione|seizoen|sezon|sezona|s[aä]song|s[æe]son|kausi|series|сезон|시즌|シーズン)[ ._-]*(\d{1,3})(?:$|[^0-9])",
    )
    .unwrap()
});

static RE_SEASON_SHORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^s(\d{1,3})$").unwrap());

static RE_SEASON_ORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(\d{1,2})(?:st|nd|rd|th|a|ª|º)?[ ._-]*(?:season|temporada)$").unwrap()
});

static RE_EP_LEAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:e|ep|episode|epis[oó]dio|cap[ií]tulo)?[ ._-]*(\d{1,4})(?:[ ._-]*-[ ._-]*e?(\d{1,4}))?(?:$|[^0-9])",
    )
    .unwrap()
});

static RE_EP_WORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[^a-z0-9])(?:ep|episode|epis[oó]dio)[ ._-]*(\d{1,4})(?:$|[^0-9])")
        .unwrap()
});

// ---- lesson patterns ----

static RE_LESSON_KW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^[\[(]?(?:aula|lesson|lecture|li[cç][aã]o|class|v[ií]deo|chapter|cap[ií]tulo|parte|part|unit|unidade|tema|topic|t[oó]pico)[ ._-]*(\d{1,4}(?:\.\d{1,3})*)[\])]?(.*)$",
    )
    .unwrap()
});

static RE_LESSON_NUM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[\[(]?(\d{1,4}(?:\.\d{1,4})*)[\])]?(\s*[-–—_.:)|\]]+\s*|\s+|$)(.*)$").unwrap()
});

static RE_MODULE_NO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^[\[(]?(?:m[oó]dulo|module|mod|se[cç][aã]o|section|parte|part|unidade|unit|cap[ií]tulo|chapter|semana|week|bloco|block)?[ ._-]*(\d{1,3})(?:$|[^0-9])",
    )
    .unwrap()
});

const EXTRA_DIRS: &[&str] = &[
    "extras",
    "extra",
    "trailers",
    "trailer",
    "featurettes",
    "featurette",
    "behind the scenes",
    "deleted scenes",
    "interviews",
    "scenes",
    "samples",
    "sample",
    "shorts",
    "other",
    "others",
    "bonus",
    "bonus features",
    "theme-music",
    "backdrops",
];

const EXTRA_SUFFIXES: &[&str] = &[
    "-trailer",
    "-sample",
    "-featurette",
    "-deleted",
    "-deletedscene",
    "-behindthescenes",
    "-interview",
    "-scene",
    "-short",
    "-other",
    "-extra",
    ".sample",
    " sample",
];

fn detect_extra(dirs: &[&str], stem: &str) -> bool {
    if dirs
        .iter()
        .any(|d| EXTRA_DIRS.contains(&fold(d.trim()).as_str()))
    {
        return true;
    }
    let f = fold(stem.trim());
    f == "sample" || f == "trailer" || EXTRA_SUFFIXES.iter().any(|s| f.ends_with(s))
}

fn season_of_dir(d: &str) -> Option<u32> {
    let t = d.trim();
    let f = fold(t);
    if f == "specials" || f == "special" || f == "especiais" || f == "extras especiais" {
        return Some(0);
    }
    RE_SEASON_DIR
        .captures(t)
        .or_else(|| RE_SEASON_SHORT.captures(t))
        .or_else(|| RE_SEASON_ORD.captures(t))
        .and_then(|c| c.get(1)?.as_str().parse().ok())
}

fn year_of(s: &str) -> Option<u32> {
    RE_YEAR_PAREN
        .captures_iter(s)
        .last()
        .and_then(|c| c.get(1)?.as_str().parse().ok())
}

/// Show name and year from the nearest folder that is not a season or extras
/// folder.
fn show_from_dirs(dirs: &[&str]) -> (Option<String>, Option<u32>) {
    for d in dirs.iter().rev() {
        if season_of_dir(d).is_some() || EXTRA_DIRS.contains(&fold(d.trim()).as_str()) {
            continue;
        }
        let t = strip_release(d);
        if !t.is_empty() {
            return (Some(t), year_of(d));
        }
    }
    (None, None)
}

fn num(c: &regex::Captures, i: usize) -> Option<u32> {
    c.get(i).and_then(|m| m.as_str().parse().ok())
}

fn ep_title(rest: &str) -> String {
    let t = strip_release(rest);
    strip_lead_number(&t)
}

/// Parses a path relative to a library root into kind, title and numbering.
pub fn parse(path_relative: &str) -> Parsed {
    let norm = path_relative.replace('\\', "/");
    let comps: Vec<&str> = norm
        .split('/')
        .map(str::trim)
        .filter(|c| !c.is_empty() && *c != ".")
        .collect();
    let Some((file, dirs)) = comps.split_last() else {
        return Parsed::default();
    };
    let (stem, _) = split_ext(file);
    let mut p = Parsed {
        is_extra: detect_extra(dirs, stem),
        ..Parsed::default()
    };
    let spaced = despace(stem);
    let season_dir = dirs
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, d)| season_of_dir(d).map(|s| (i, s)));

    // SxxEyy (and S01E01-E02 / S01E01E02).
    if let Some(c) = RE_SXE.captures(stem) {
        let whole = c.get(0).unwrap();
        p.kind = MediaKind::Episode;
        p.season = num(&c, 1);
        p.episode = num(&c, 2);
        p.episode_end = num(&c, 3).or_else(|| num(&c, 4));
        fill_show(&mut p, &despace_as(stem, &stem[..whole.start()]), dirs);
        p.title = ep_title(&despace_as(stem, &stem[whole.end()..]));
        finish_episode(&mut p, stem);
        return p;
    }
    // 1x02
    if let Some(c) = RE_NXN.captures(stem) {
        let whole = c.get(0).unwrap();
        p.kind = MediaKind::Episode;
        p.season = num(&c, 1);
        p.episode = num(&c, 2);
        p.episode_end = num(&c, 3);
        fill_show(&mut p, &despace_as(stem, &stem[..whole.start()]), dirs);
        p.title = ep_title(&despace_as(stem, &stem[whole.end()..]));
        finish_episode(&mut p, stem);
        return p;
    }
    // Fansub anime: [Group] Name - 12 [1080p] (absolute numbering).
    let under = stem.replace('_', " ");
    if let Some(c) = RE_FANSUB
        .captures(&under)
        .or_else(|| RE_ANIME_TAIL.captures(&under))
    {
        let name = strip_release(c.get(1).unwrap().as_str());
        if name.chars().any(char::is_alphabetic) && !RE_LEAD_KEYWORD.is_match(&name) {
            p.kind = MediaKind::Episode;
            p.episode = num(&c, 2);
            p.episode_end = num(&c, 3);
            p.show = Some(name);
            p.season = season_dir.map(|(_, s)| s);
            let whole = c.get(0).unwrap();
            let rest = &under[whole.end()..];
            // The terminator may have eaten the first letter of the title.
            let tail =
                under[c.get(2).unwrap().end()..whole.end()].trim_start_matches(|ch: char| {
                    ch.is_whitespace() || ch == '-' || ch == 'v' || ch.is_ascii_digit()
                });
            let tail = if tail.starts_with(['[', '(']) {
                ""
            } else {
                tail
            };
            p.title = ep_title(&format!("{tail}{rest}"));
            finish_episode(&mut p, stem);
            return p;
        }
    }
    // Season folder: Show/Season 1/05 - Title.mkv
    if let Some((si, season)) = season_dir {
        let lead = RE_EP_LEAD.captures(&spaced).map(|c| (c, true));
        let word = || RE_EP_WORD.captures(&spaced).map(|c| (c, false));
        if let Some((c, at_start)) = lead.or_else(word) {
            let whole = c.get(0).unwrap();
            p.kind = MediaKind::Episode;
            p.season = Some(season);
            p.episode = num(&c, 1);
            p.episode_end = if at_start { num(&c, 2) } else { None };
            let (show, year) = show_from_dirs(&dirs[..si]);
            let prefix = if at_start {
                String::new()
            } else {
                strip_release(&spaced[..whole.start()])
            };
            p.show = show.or((!prefix.is_empty()).then_some(prefix));
            p.year = year;
            p.title = ep_title(&spaced[whole.end()..]);
            finish_episode(&mut p, stem);
            return p;
        }
    }
    // Episode keyword anywhere: "Show Ep 05", "Episode 5".
    if let Some(c) = RE_EP_WORD.captures(&spaced) {
        let whole = c.get(0).unwrap();
        p.kind = MediaKind::Episode;
        p.episode = num(&c, 1);
        fill_show(&mut p, &spaced[..whole.start()], dirs);
        p.title = ep_title(&spaced[whole.end()..]);
        finish_episode(&mut p, stem);
        return p;
    }

    let strong_lesson = is_strong_lesson(stem);
    // Movie with the year in the name: "Name (2019)" or "Name.2019.1080p".
    if !strong_lesson {
        if let Some((title, year)) = movie_from_name(stem) {
            p.kind = MediaKind::Movie;
            p.title = title;
            p.year = Some(year);
            return p;
        }
    }
    // Numbered lesson.
    if let Some((nums, rest)) = lesson_numbers(stem) {
        p.kind = MediaKind::Lesson;
        p.lesson_no = Some(nums);
        let t = strip_release(&rest);
        p.title = if t.is_empty() { clean_title(stem) } else { t };
        if let Some(d) = dirs.last() {
            p.module = Some(d.to_string());
            p.module_no = RE_MODULE_NO.captures(d).and_then(|c| num(&c, 1));
        }
        return p;
    }
    // Movie folder: "Inception (2010)/movie.mkv".
    if let Some(d) = dirs.last() {
        if let Some(year) = year_of(d) {
            let t = strip_release(d);
            if !t.is_empty() && !p.is_extra {
                p.kind = MediaKind::Movie;
                p.title = t;
                p.year = Some(year);
                return p;
            }
        }
    }
    p.kind = MediaKind::Video;
    p.title = clean_title(stem);
    p
}

fn fill_show(p: &mut Parsed, before: &str, dirs: &[&str]) {
    let name = strip_release(before);
    let (dir_show, dir_year) = show_from_dirs(dirs);
    if name.is_empty() {
        p.show = dir_show;
        p.year = dir_year;
    } else {
        p.year = year_of(before).or(dir_year);
        p.show = Some(name);
    }
}

fn finish_episode(p: &mut Parsed, stem: &str) {
    if p.episode_end.is_some() && p.episode_end <= p.episode {
        p.episode_end = None;
    }
    if p.title.is_empty() {
        p.title = clean_title(stem);
    }
}

fn is_strong_lesson(stem: &str) -> bool {
    if RE_LESSON_KW.is_match(stem) {
        return true;
    }
    match RE_LESSON_NUM.captures(stem) {
        Some(c) => {
            let n = c.get(1).unwrap().as_str();
            let sep = c.get(2).map(|m| m.as_str()).unwrap_or("");
            n.starts_with('0') || n.contains('.') || !sep.trim().is_empty()
        }
        None => false,
    }
}

fn lesson_numbers(stem: &str) -> Option<(Vec<u32>, String)> {
    let parse_nums = |s: &str| -> Option<Vec<u32>> {
        s.split('.')
            .map(|x| x.parse::<u32>().ok())
            .collect::<Option<Vec<u32>>>()
    };
    if let Some(c) = RE_LESSON_KW.captures(stem) {
        let nums = parse_nums(c.get(1)?.as_str())?;
        return Some((nums, c.get(2).map(|m| m.as_str()).unwrap_or("").to_string()));
    }
    let c = RE_LESSON_NUM.captures(stem)?;
    let n = c.get(1)?.as_str();
    // Four bare digits look like a year ("1917"): only numbering with a zero.
    let first = n.split('.').next().unwrap_or("");
    if first.len() == 4 && !first.starts_with('0') && !n.contains('.') {
        return None;
    }
    let rest = c.get(3).map(|m| m.as_str()).unwrap_or("").to_string();
    // "1080p" or "2x" are not lesson numbers.
    if rest.starts_with(|ch: char| ch.is_ascii_alphanumeric())
        && c.get(2).map(|m| m.as_str().is_empty()).unwrap_or(true)
    {
        return None;
    }
    Some((parse_nums(n)?, rest))
}

fn movie_from_name(stem: &str) -> Option<(String, u32)> {
    if let Some(c) = RE_YEAR_PAREN.captures_iter(stem).last() {
        let m = c.get(0).unwrap();
        let title = strip_release(&stem[..m.start()]);
        if !title.is_empty() {
            return Some((title, c.get(1)?.as_str().parse().ok()?));
        }
    }
    let spaced = despace(stem);
    let release = is_release_style(stem);
    for c in RE_YEAR_TOKEN
        .captures_iter(&spaced)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let m = c.get(1).unwrap();
        if m.start() == 0 {
            continue;
        }
        let after = &spaced[m.end()..];
        let tail_ok = trim_seps(after).is_empty()
            || RE_RELEASE
                .find(after)
                .is_some_and(|r| trim_seps(&after[..r.start()]).is_empty())
            || after.trim_start().starts_with('[');
        if release || tail_ok {
            let title = strip_release(&spaced[..m.start()]);
            if !title.is_empty() {
                return Some((title, m.as_str().parse().ok()?));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut v: Vec<&str>) -> Vec<&str> {
        v.sort_by(|a, b| natural_cmp(a, b));
        v
    }

    #[test]
    fn natural_order_numbers() {
        assert_eq!(
            sorted(vec!["10 - c", "2 - b", "1 - a"]),
            vec!["1 - a", "2 - b", "10 - c"]
        );
        assert_eq!(
            sorted(vec!["Aula 10", "aula 2", "Aula 1"]),
            vec!["Aula 1", "aula 2", "Aula 10"]
        );
        assert_eq!(
            sorted(vec!["1.10 x", "1.2 x", "1.1 x"]),
            vec!["1.1 x", "1.2 x", "1.10 x"]
        );
        assert!(natural_cmp("01", "1").is_ne());
        assert_eq!(natural_key("01"), natural_key("1"));
        // Very long digit runs do not overflow.
        assert_eq!(
            natural_cmp("99999999999999999999999", "100000000000000000000000"),
            Ordering::Less
        );
    }

    #[test]
    fn natural_order_accents_and_case() {
        assert_eq!(natural_key("Módulo"), natural_key("modulo"));
        assert_eq!(
            sorted(vec!["Égua", "abacate", "Zebra", "elefante"]),
            vec!["abacate", "Égua", "elefante", "Zebra"]
        );
        assert!(natural_cmp("Introdução", "introducao").is_ne());
        assert_eq!(natural_key("Introdução"), natural_key("INTRODUCAO"));
    }

    #[test]
    fn clean_titles() {
        assert_eq!(clean_title("01 - Intro.mp4"), "Intro");
        assert_eq!(
            clean_title("1.2 Variáveis e tipos.mkv"),
            "Variáveis e tipos"
        );
        assert_eq!(
            clean_title("Aula 3 - Setup do ambiente"),
            "Setup do ambiente"
        );
        assert_eq!(clean_title("Aula 3"), "Aula 3");
        assert_eq!(clean_title("03. Setup"), "Setup");
        assert_eq!(
            clean_title("300 Rise of an Empire"),
            "300 Rise of an Empire"
        );
        assert_eq!(clean_title("Name.2019.1080p.BluRay.x264-GRP.mkv"), "Name");
        assert_eq!(clean_title("[Grupo] Nome [1080p]"), "Nome");
        assert_eq!(clean_title("Video (Parte 1)"), "Video (Parte 1)");
        assert_eq!(clean_title("folder/sub/05_intro_final.mp4"), "intro final");
        assert_eq!(clean_title("01"), "01");
        assert_eq!(clean_title(""), "");
    }

    #[test]
    fn movies() {
        let p = parse("Movies/Inception (2010).mkv");
        assert_eq!(p.kind, MediaKind::Movie);
        assert_eq!(p.title, "Inception");
        assert_eq!(p.year, Some(2010));

        let p = parse("Nome.Do.Filme.2019.1080p.WEB-DL.x264.mkv");
        assert_eq!(p.kind, MediaKind::Movie);
        assert_eq!(p.title, "Nome Do Filme");
        assert_eq!(p.year, Some(2019));

        let p = parse("1917 (2019).mp4");
        assert_eq!(
            (p.kind, p.title.as_str(), p.year),
            (MediaKind::Movie, "1917", Some(2019))
        );

        let p = parse("300 (2006).mkv");
        assert_eq!((p.kind, p.title.as_str()), (MediaKind::Movie, "300"));

        let p = parse("Blade Runner 2049 (2017).mkv");
        assert_eq!(
            (p.title.as_str(), p.year),
            ("Blade Runner 2049", Some(2017))
        );

        let p = parse("Inception (2010)/movie.mkv");
        assert_eq!((p.kind, p.title.as_str()), (MediaKind::Movie, "Inception"));

        let p = parse("Inception (2010)/Inception-trailer.mp4");
        assert!(p.is_extra);
        let p = parse("Inception (2010)/Extras/Making of.mp4");
        assert!(p.is_extra);
    }

    #[test]
    fn series() {
        let p = parse("Shows/Breaking Bad/Season 1/Breaking.Bad.S01E02.Cat's.in.the.Bag.720p.mkv");
        assert_eq!(p.kind, MediaKind::Episode);
        assert_eq!(p.show.as_deref(), Some("Breaking Bad"));
        assert_eq!(
            (p.season, p.episode, p.episode_end),
            (Some(1), Some(2), None)
        );
        assert_eq!(p.title, "Cat's in the Bag");

        let p = parse("Show/Show - S01E01-E02 - Pilot.mkv");
        assert_eq!(
            (p.season, p.episode, p.episode_end),
            (Some(1), Some(1), Some(2))
        );
        assert_eq!(p.show.as_deref(), Some("Show"));
        assert_eq!(p.title, "Pilot");

        let p = parse("Show.S02E03E04.mkv");
        assert_eq!(
            (p.season, p.episode, p.episode_end),
            (Some(2), Some(3), Some(4))
        );

        let p = parse("Friends 1x02 The One.avi");
        assert_eq!(p.kind, MediaKind::Episode);
        assert_eq!((p.season, p.episode), (Some(1), Some(2)));
        assert_eq!(p.show.as_deref(), Some("Friends"));
        assert_eq!(p.title, "The One");

        let p = parse("Friends/1x02.avi");
        assert_eq!(p.show.as_deref(), Some("Friends"));

        let p = parse("Video 1920x1080.mp4");
        assert_ne!(p.kind, MediaKind::Episode);

        let p = parse("Dark/Temporada 2/03 - Ghosts.mkv");
        assert_eq!(p.kind, MediaKind::Episode);
        assert_eq!((p.season, p.episode), (Some(2), Some(3)));
        assert_eq!(p.show.as_deref(), Some("Dark"));
        assert_eq!(p.title, "Ghosts");

        let p = parse("The Office (2005)/Season 1/Episode 4.mkv");
        assert_eq!((p.season, p.episode), (Some(1), Some(4)));
        assert_eq!(p.show.as_deref(), Some("The Office"));
        assert_eq!(p.year, Some(2005));

        let p = parse("Show/Specials/E01.mkv");
        assert_eq!(p.season, Some(0));
    }

    #[test]
    fn anime() {
        let p = parse("Anime/[SubsPlease] Sousou no Frieren - 12 [1080p].mkv");
        assert_eq!(p.kind, MediaKind::Episode);
        assert_eq!(p.show.as_deref(), Some("Sousou no Frieren"));
        assert_eq!(p.episode, Some(12));
        assert_eq!(p.season, None);

        let p = parse("[Grupo] Nome - 12 [1080p].mkv");
        assert_eq!((p.show.as_deref(), p.episode), (Some("Nome"), Some(12)));

        let p = parse("[Grp]_Some_Show_-_05v2_[720p][ABCD1234].mkv");
        assert_eq!((p.show.as_deref(), p.episode), (Some("Some Show"), Some(5)));

        let p = parse("[Grp] One Piece - 1085 (1080p).mkv");
        assert_eq!(p.episode, Some(1085));

        let p = parse("Mushishi - 03 [BD 1080p].mkv");
        assert_eq!((p.show.as_deref(), p.episode), (Some("Mushishi"), Some(3)));
    }

    #[test]
    fn lessons() {
        let p = parse("01 - Intro.mp4");
        assert_eq!(p.kind, MediaKind::Lesson);
        assert_eq!(p.lesson_no, Some(vec![1]));
        assert_eq!(p.title, "Intro");
        assert_eq!(p.module, None);

        let p = parse("Módulo 2/03 - Tal.mp4");
        assert_eq!(p.kind, MediaKind::Lesson);
        assert_eq!(p.lesson_no, Some(vec![3]));
        assert_eq!(p.title, "Tal");
        assert_eq!(p.module.as_deref(), Some("Módulo 2"));
        assert_eq!(p.module_no, Some(2));

        let p = parse("Curso/Aula 3.mp4");
        assert_eq!(
            (p.kind, p.lesson_no.clone()),
            (MediaKind::Lesson, Some(vec![3]))
        );
        assert_eq!(p.title, "Aula 3");

        let p = parse("Lesson 10 - Closures.mkv");
        assert_eq!(p.lesson_no, Some(vec![10]));
        assert_eq!(p.title, "Closures");

        let p = parse("1.2 Title.mp4");
        assert_eq!(p.lesson_no, Some(vec![1, 2]));
        assert_eq!(p.title, "Title");

        let p = parse("02 - Fundamentos/1.10 Loops.mp4");
        assert_eq!(p.lesson_no, Some(vec![1, 10]));
        assert_eq!(p.module_no, Some(2));

        let p = parse("[05] Deploy.mp4");
        assert_eq!(p.lesson_no, Some(vec![5]));
        assert_eq!(p.title, "Deploy");
    }

    #[test]
    fn plain_and_garbage() {
        let p = parse("Downloads/my holiday clip.mp4");
        assert_eq!(p.kind, MediaKind::Video);
        assert_eq!(p.title, "my holiday clip");

        for junk in [
            "", "/", "///", ".mp4", "   ", "\\\\", "[", "(", "-", "S01E", "x",
        ] {
            let _ = parse(junk);
        }
        assert_eq!(parse("").kind, MediaKind::Video);
        assert_eq!(parse("clip.1080p.mp4").kind, MediaKind::Video);
        let p = parse("C:\\Videos\\Show\\S01E05.mkv");
        assert_eq!(p.show.as_deref(), Some("Show"));
    }

    #[test]
    fn split_ext_rules() {
        assert_eq!(split_ext("a.mkv"), ("a", Some("mkv".into())));
        assert_eq!(split_ext("Name.2019"), ("Name.2019", None));
        assert_eq!(split_ext("Name.1080p"), ("Name.1080p", None));
        assert_eq!(split_ext(".hidden"), (".hidden", None));
        assert!(is_video_ext(".MKV"));
        assert!(!is_video_ext("pdf"));
    }
}
