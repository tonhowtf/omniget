//! Kudu (MIT, AdventDevInc/kudu) Windows cleaner rules, converted once by
//! `scripts/washmybelly/kudu-convert.mjs` into our [`Rule`] shape and
//! embedded here. Attribution and upstream commit in
//! `data/washmybelly/kudu/SOURCE.md`.
//!
//! Also the target comparison every source shares: two rules that reach the
//! same folder are deduplicated by target, the first source winning
//! (BleachBit, then ours, then Kudu).

use std::sync::OnceLock;

use serde::Deserialize;

use super::rule::Rule;

const RULES_JSON: &str = include_str!("../../../../data/washmybelly/kudu/rules.json");

#[derive(Debug, Deserialize)]
pub struct KuduFile {
    pub commit: String,
    pub license: String,
    pub count: usize,
    #[serde(default)]
    pub skipped: Vec<serde_json::Value>,
    pub rules: Vec<Rule>,
}

fn parsed() -> &'static Result<KuduFile, String> {
    static CELL: OnceLock<Result<KuduFile, String>> = OnceLock::new();
    CELL.get_or_init(|| {
        serde_json::from_str::<KuduFile>(RULES_JSON).map_err(|e| format!("kudu rules.json: {e}"))
    })
}

/// The upstream commit the embedded rules were converted from.
pub fn commit() -> &'static str {
    match parsed() {
        Ok(f) => &f.commit,
        Err(_) => "",
    }
}

/// Every Kudu rule, minus any target that falls in a protected Windows tree
/// (the converter already drops those; this is the second fence).
pub fn rules() -> Vec<Rule> {
    let Ok(file) = parsed() else {
        return Vec::new();
    };
    file.rules
        .iter()
        .filter_map(|r| {
            let mut r = r.clone();
            r.targets
                .retain(|t| super::rules_windows::protected_hit(t).is_none());
            let native = matches!(r.action, super::rule::Action::Command { .. });
            (native || !r.targets.is_empty()).then_some(r)
        })
        .collect()
}

// ── Target comparison ──

/// Canonical form of a rule target for comparison: forward slashes, lower
/// case, one spelling per Windows folder (`%USERPROFILE%\AppData\Local` and
/// `$LOCALAPPDATA` both become `%localappdata%`, `%SystemRoot%` becomes
/// `%windir%`, `%TEMP%` becomes `%localappdata%/temp`).
pub fn normalize_target(t: &str) -> String {
    let mut s = t.replace('\\', "/");
    // `${VAR}` and `$VAR` → `%VAR%`.
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' {
            let braced = b.get(i + 1) == Some(&b'{');
            let start = if braced { i + 2 } else { i + 1 };
            let mut j = start;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            if j > start {
                out.push('%');
                out.push_str(&s[start..j]);
                out.push('%');
                i = if braced && b.get(j) == Some(&b'}') {
                    j + 1
                } else {
                    j
                };
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap_or_default();
        out.push(ch);
        i += ch.len_utf8();
    }
    s = out.to_lowercase();
    if let Some(r) = s.strip_prefix('~') {
        s = format!("%userprofile%{r}");
    }
    for (from, to) in [
        ("%home%", "%userprofile%"),
        ("%systemroot%", "%windir%"),
        ("%tmp%", "%localappdata%/temp"),
        ("%temp%", "%localappdata%/temp"),
        ("%userprofile%/appdata/local/temp", "%localappdata%/temp"),
        ("%userprofile%/appdata/local", "%localappdata%"),
        ("%userprofile%/appdata/roaming", "%appdata%"),
        ("%programfiles(x86)%", "%programfiles_x86%"),
        ("%commonprogramfiles%", "%programfiles%/common files"),
        ("%allusersprofile%", "%programdata%"),
    ] {
        if s == from || s.starts_with(&format!("{from}/")) {
            s = format!("{to}{}", &s[from.len()..]);
        }
    }
    while s.contains("//") {
        s = s.replace("//", "/");
    }
    s.trim_end_matches('/').to_string()
}

fn segments(t: &str) -> Vec<String> {
    normalize_target(t)
        .split('/')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether every path `inner` can match is at or below a path `outer` can
/// match. Segment by segment, `outer`'s wildcard must accept `inner`'s
/// segment (a `*` in `inner` is only accepted by a `*` in `outer`).
pub fn covers(outer: &str, inner: &str) -> bool {
    covers_segments(&segments(outer), &segments(inner))
}

/// [`covers`] on targets already split by [`segments`].
fn covers_segments(o: &[String], i: &[String]) -> bool {
    if o.is_empty() || i.len() < o.len() {
        return false;
    }
    o.iter().zip(i.iter()).all(|(po, pi)| {
        if po == pi {
            return true;
        }
        let o_wild = po.contains(['*', '?']);
        let i_wild = pi.contains(['*', '?']);
        o_wild && !i_wild && crate::core::washmybelly::fsops::wildcard(po, pi)
            || o_wild && i_wild && po == "*"
    })
}

/// Drop from `secondary` every target a `primary` rule already reaches:
/// equal or inside a primary target, or a folder that contains a primary
/// target (only when that folder is at least three segments deep, so a broad
/// base like `%LOCALAPPDATA%\*\...` never loses its other targets). Rules
/// left with no target go; native commands go when a primary rule runs the
/// same program with the same arguments.
pub fn dedupe_against(primary: &[Rule], secondary: Vec<Rule>) -> Vec<Rule> {
    use super::rule::Action;
    // Split once: normalizing inside the pairwise loop cost seconds.
    let primary_targets: Vec<Vec<String>> = primary
        .iter()
        .flat_map(|r| r.targets.iter())
        .map(|t| segments(t))
        .collect();
    let primary_cmds: Vec<(&String, &Vec<String>)> = primary
        .iter()
        .filter_map(|r| match &r.action {
            Action::Command { program, args, .. } => Some((program, args)),
            _ => None,
        })
        .collect();
    secondary
        .into_iter()
        .filter_map(|mut r| {
            if let Action::Command { program, args, .. } = &r.action {
                let dup = primary_cmds
                    .iter()
                    .any(|(p, a)| p.eq_ignore_ascii_case(program) && *a == args);
                return (!dup).then_some(r);
            }
            r.targets.retain(|s| {
                let s = segments(s);
                !primary_targets
                    .iter()
                    .any(|p| covers_segments(p, &s) || (s.len() >= 3 && covers_segments(&s, p)))
            });
            (!r.targets.is_empty()).then_some(r)
        })
        .collect()
}
