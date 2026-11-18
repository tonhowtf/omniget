use regex::Regex;
use std::sync::OnceLock;

pub const DESC_MAX: usize = 300;

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("regex"))
}

fn strip_examples(s: &str) -> String {
    static A: OnceLock<Regex> = OnceLock::new();
    static B: OnceLock<Regex> = OnceLock::new();
    static C: OnceLock<Regex> = OnceLock::new();
    let s = re(&A, r"(?is)<example>.*?</example>").replace_all(s, " ");
    let s = re(&B, r"(?is)<commentary>.*?</commentary>").replace_all(&s, " ");
    re(&C, r"(?is)<example>.*$")
        .replace_all(&s, " ")
        .into_owned()
}

fn strip_trailing_lead(s: &str) -> String {
    static T: OnceLock<Regex> = OnceLock::new();
    re(
        &T,
        r"(?i)\s*(Examples?( include)?|For example|e\.g\.|Specifically|Such as|Including|Use cases?)\s*:?\s*$",
    )
    .replace(s, "")
    .trim()
    .to_string()
}

pub fn short_description(raw: &str) -> (String, Vec<&'static str>) {
    let mut marks = Vec::new();
    let unescaped = raw.replace("\\n", "\n");
    let mut s = strip_examples(&unescaped);
    static TAGS: OnceLock<Regex> = OnceLock::new();
    s = re(
        &TAGS,
        r"(?i)</?(example|commentary|context|user|assistant)>",
    )
    .replace_all(&s, " ")
    .into_owned();
    if s != unescaped {
        marks.push("stripped_example_blocks");
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut s = strip_trailing_lead(&s);
    if s.chars().count() > DESC_MAX {
        let cut: String = s.chars().take(DESC_MAX - 1).collect();
        let base = match cut.rfind(' ') {
            Some(sp) if sp as f64 > (DESC_MAX as f64) * 0.6 => cut[..sp].to_string(),
            _ => cut,
        };
        s = format!(
            "{}…",
            base.trim_end_matches(|c: char| c.is_whitespace() || ",;:.-".contains(c))
        );
        marks.push("truncated_description");
    }
    (s, marks)
}
