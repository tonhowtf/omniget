//! Renomear em massa (estudo 29, PowerRename): regex + substituição com
//! tokens `{n}`, `{n:3}` (contador com zeros), `{name}`, `{ext}`, `{date}`,
//! mais caixa (upper/lower/title). Sempre com prévia antes de aplicar.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct RenameOptions {
    pub files: Vec<String>,
    #[serde(default)]
    pub pattern: String,
    #[serde(default)]
    pub replacement: String,
    #[serde(default)]
    pub case_insensitive: bool,
    /// "" | "upper" | "lower" | "title"
    #[serde(default)]
    pub case: String,
    #[serde(default = "one")]
    pub counter_start: u32,
    #[serde(default)]
    pub apply_to_extension: bool,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenamePlan {
    pub from: String,
    pub to: String,
    pub changed: bool,
    pub conflict: bool,
}

fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn expand_tokens(template: &str, n: u32, name: &str, ext: &str) -> String {
    let re = regex::Regex::new(r"\{(n(?::(\d+))?|name|ext|date)\}").unwrap();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    re.replace_all(template, |c: &regex::Captures| {
        let key = &c[1];
        if key.starts_with('n') {
            let pad = c
                .get(2)
                .and_then(|m| m.as_str().parse::<usize>().ok())
                .unwrap_or(1);
            format!("{:0width$}", n, width = pad)
        } else if key == "name" {
            name.to_string()
        } else if key == "ext" {
            ext.to_string()
        } else {
            today.clone()
        }
    })
    .to_string()
}

pub fn plan(opts: &RenameOptions) -> Result<Vec<RenamePlan>, String> {
    let re = if opts.pattern.is_empty() {
        None
    } else {
        Some(
            regex::RegexBuilder::new(&opts.pattern)
                .case_insensitive(opts.case_insensitive)
                .build()
                .map_err(|e| format!("expressao invalida: {}", e))?,
        )
    };
    let mut out = Vec::new();
    let mut targets = std::collections::HashSet::new();
    let mut n = opts.counter_start;
    for f in &opts.files {
        let p = Path::new(f);
        let file_name = p
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let (stem, ext) = match (p.file_stem(), p.extension()) {
            (Some(s), Some(e)) => (
                s.to_string_lossy().to_string(),
                e.to_string_lossy().to_string(),
            ),
            _ => (file_name.clone(), String::new()),
        };
        let subject = if opts.apply_to_extension {
            file_name.clone()
        } else {
            stem.clone()
        };
        let replacement = expand_tokens(&opts.replacement, n, &stem, &ext);
        let mut new = match &re {
            Some(re) => re.replace_all(&subject, replacement.as_str()).to_string(),
            None if !opts.replacement.is_empty() => replacement,
            None => subject.clone(),
        };
        new = match opts.case.as_str() {
            "upper" => new.to_uppercase(),
            "lower" => new.to_lowercase(),
            "title" => title_case(&new),
            _ => new,
        };
        let new_name = if opts.apply_to_extension || ext.is_empty() {
            new
        } else {
            format!("{}.{}", new, ext)
        };
        let new_name = super::sanitize_name(&new_name);
        let to = p.with_file_name(&new_name);
        let to_s = to.to_string_lossy().to_string();
        let changed = new_name != file_name;
        let conflict = changed && (to.exists() || !targets.insert(to_s.to_lowercase()));
        out.push(RenamePlan {
            from: f.clone(),
            to: to_s,
            changed,
            conflict,
        });
        if changed {
            n += 1;
        }
    }
    Ok(out)
}

pub fn apply(plans: &[RenamePlan]) -> (usize, Vec<String>) {
    let mut ok = 0;
    let mut failed = Vec::new();
    for p in plans.iter().filter(|p| p.changed && !p.conflict) {
        match std::fs::rename(&p.from, &p.to) {
            Ok(()) => ok += 1,
            Err(e) => failed.push(format!("{}: {}", p.from, e)),
        }
    }
    (ok, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_and_regex() {
        let opts = RenameOptions {
            files: vec![
                "/x/Aula 01 - intro.mp4".into(),
                "/x/Aula 02 - loops.mp4".into(),
            ],
            pattern: r"^Aula (\d+) - ".into(),
            replacement: "{n:2}. ".into(),
            case_insensitive: false,
            case: "title".into(),
            counter_start: 1,
            apply_to_extension: false,
        };
        let p = plan(&opts).unwrap();
        // The plan joins with the platform separator, so compare through Path
        // instead of a literal "/" that Windows would render as "\".
        let expect = |name: &str| {
            std::path::Path::new("/x")
                .join(name)
                .to_string_lossy()
                .to_string()
        };
        assert_eq!(p[0].to, expect("01. Intro.mp4"));
        assert_eq!(p[1].to, expect("02. Loops.mp4"));
        assert!(p.iter().all(|x| x.changed && !x.conflict));
    }
}
