use regex::Regex;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::sync::LazyLock;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

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
    pub fn as_str(self) -> &'static str {
        match self {
            MediaKind::Movie => "movie",
            MediaKind::Episode => "episode",
            MediaKind::Lesson => "lesson",
            MediaKind::Video => "video",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Parsed {
    pub kind: MediaKind,
    pub title: String,
    pub show: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub episode_end: Option<u32>,
    pub year: Option<u32>,
    pub lesson_no: Option<Vec<u32>>,
    pub module: Option<String>,
    pub module_no: Option<u32>,
    pub is_extra: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Chunk {
    Num(String),
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

pub fn fold(s: &str) -> String {
    s.nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(|c| c.to_lowercase())
        .collect()
}

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

pub fn is_video_ext(ext: &str) -> bool {
    let e = ext.trim_start_matches('.').to_ascii_lowercase();
    VIDEO_EXTS.contains(&e.as_str())
}

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

fn is_release_style(stem: &str) -> bool {
    !stem.contains(' ') && (stem.matches('.').count() >= 2 || stem.contains('_'))
}

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

pub fn clean_title(s: &str) -> String {
    let base = s.rsplit(['/', '\\']).next().unwrap_or(s);
    let (stem, _) = split_ext(base);
    let released = strip_release(stem);
    let mut t = released.clone();
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
    if !strong_lesson {
        if let Some((title, year)) = movie_from_name(stem) {
            p.kind = MediaKind::Movie;
            p.title = title;
            p.year = Some(year);
            return p;
        }
    }
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
    let first = n.split('.').next().unwrap_or("");
    if first.len() == 4 && !first.starts_with('0') && !n.contains('.') {
        return None;
    }
    let rest = c.get(3).map(|m| m.as_str()).unwrap_or("").to_string();
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
