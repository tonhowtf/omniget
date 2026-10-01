//! upstream macOS cleaner (`data/washmybelly/mole/protection.txt`, GPL-3.0,

use std::path::Path;
use std::sync::OnceLock;

const UPSTREAM: &str = include_str!("../../../../data/washmybelly/mole/protection.txt");
const OURS: &str = include_str!("../../../../data/washmybelly/ours/protection.txt");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Name,
    Path,
    IPath,
}

#[derive(Debug)]
struct Pattern {
    kind: Kind,
    glob: String,
    origin: &'static str,
    chars: Vec<char>,
    lower: Vec<char>,
}

fn parse(text: &str, origin: &'static str, out: &mut Vec<Pattern>) {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((kind, glob)) = line.split_once(' ') else {
            continue;
        };
        let kind = match kind {
            "name" => Kind::Name,
            "path" => Kind::Path,
            "ipath" => Kind::IPath,
            _ => continue,
        };
        let glob = glob.trim().to_string();
        out.push(Pattern {
            kind,
            chars: glob.chars().collect(),
            lower: glob.to_lowercase().chars().collect(),
            glob,
            origin,
        });
    }
}

fn patterns() -> &'static [Pattern] {
    static P: OnceLock<Vec<Pattern>> = OnceLock::new();
    P.get_or_init(|| {
        let mut v = Vec::new();
        parse(UPSTREAM, "upstream", &mut v);
        parse(OURS, "ours", &mut v);
        v
    })
}

pub fn pattern_count() -> usize {
    patterns().len()
}

pub fn star_match(pattern: &str, text: &str, fold: bool) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = if fold {
        (
            pattern.to_lowercase().chars().collect(),
            text.to_lowercase().chars().collect(),
        )
    } else {
        (pattern.chars().collect(), text.chars().collect())
    };
    star_chars(&p, &t)
}

fn star_chars(p: &[char], t: &[char]) -> bool {
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn home() -> String {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}

fn container_id(path: &str) -> Option<&str> {
    for marker in ["/Library/Containers/", "/Library/Group Containers/"] {
        if let Some(i) = path.find(marker) {
            let rest = &path[i + marker.len()..];
            let id = rest.split('/').next().unwrap_or("");
            let inner = rest.get(id.len()..).unwrap_or("");
            let regenerable = [
                "/Data/Library/Caches",
                "/Data/tmp",
                "/Data/Library/Mail Downloads",
                "/Library/Caches",
            ]
            .iter()
            .any(|p| inner == *p || inner.starts_with(&format!("{p}/")));
            if id.is_empty() || regenerable {
                return None;
            }
            return Some(id);
        }
    }
    None
}

pub fn protection_match(path: &Path) -> Option<String> {
    protection_match_with(path, &home())
}

pub fn protection_match_with(path: &Path, home: &str) -> Option<String> {
    let full = path.to_string_lossy().replace('\\', "/");
    let full = full.trim_end_matches('/');
    let name = full.rsplit('/').next().unwrap_or(full);
    let fold = cfg!(any(windows, target_os = "macos"));
    let name_c: Vec<char> = name.chars().collect();
    let full_c: Vec<char> = full.chars().collect();
    let full_l: Vec<char> = full.to_lowercase().chars().collect();
    let names: Vec<&Pattern> = patterns().iter().filter(|p| p.kind == Kind::Name).collect();

    for p in patterns() {
        let insensitive = p.kind == Kind::IPath || fold;
        let hit = match p.kind {
            Kind::Name => star_chars(&p.chars, &name_c),
            Kind::Path | Kind::IPath => match p.glob.strip_prefix('~') {
                Some(rest) if !home.is_empty() => {
                    star_match(&format!("{home}{rest}"), full, insensitive)
                }
                Some(_) => continue,
                None if insensitive => star_chars(&p.lower, &full_l),
                None => star_chars(&p.chars, &full_c),
            },
        };
        if hit {
            return Some(format!("{} ({} protection list)", p.glob, p.origin));
        }
    }
    if let Some(id) = container_id(full) {
        let id_c: Vec<char> = id.chars().collect();
        if let Some(p) = names.iter().find(|p| star_chars(&p.chars, &id_c)) {
            return Some(format!(
                "{} (container {id}, {} protection list)",
                p.glob, p.origin
            ));
        }
    }
    if Path::new(full)
        .join("com.apple.e5rt.e5bundlecache")
        .is_dir()
    {
        return Some("compiled model cache inside".into());
    }
    None
}
