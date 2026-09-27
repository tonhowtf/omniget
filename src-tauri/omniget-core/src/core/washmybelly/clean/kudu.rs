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
    let o = segments(outer);
    let i = segments(inner);
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
    let primary_targets: Vec<&String> = primary.iter().flat_map(|r| r.targets.iter()).collect();
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
                !primary_targets
                    .iter()
                    .any(|p| covers(p, s) || (segments(s).len() >= 3 && covers(s, p)))
            });
            (!r.targets.is_empty()).then_some(r)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::rule::{Action, Requirement};
    use super::*;
    use crate::core::washmybelly::{Destination, Os, Risk};

    fn file() -> &'static KuduFile {
        parsed().as_ref().expect("embedded kudu rules.json parses")
    }

    #[test]
    fn embedded_json_parses_with_its_count() {
        let f = file();
        assert_eq!(f.rules.len(), f.count, "count field matches the rules");
        assert!(
            f.count >= 150,
            "expected the ~200 converted Kudu rules, got {}",
            f.count
        );
        assert_eq!(f.license, "MIT");
        assert_eq!(f.commit.len(), 40);
        assert!(
            !f.skipped.is_empty(),
            "the converter records what it left out"
        );
    }

    #[test]
    fn every_rule_is_windows_kudu_mit_and_unique() {
        let mut ids = std::collections::HashSet::new();
        for r in &file().rules {
            assert!(ids.insert(r.id.clone()), "duplicate id {}", r.id);
            assert!(r.id.starts_with("kudu."), "{}", r.id);
            assert_eq!(r.os, vec![Os::Windows], "{}", r.id);
            assert_eq!(r.source, "Kudu");
            assert_eq!(r.license, "MIT");
            assert_eq!(r.destination, Destination::Unlink, "{}", r.id);
            assert_eq!(r.risk, Risk::Destructive, "{}", r.id);
            if !matches!(r.action, Action::Command { .. }) {
                assert!(
                    r.min_age_days >= 1,
                    "{}: Kudu never deletes a cache used yesterday",
                    r.id
                );
                assert!(!r.targets.is_empty(), "{}", r.id);
            }
        }
    }

    #[test]
    fn known_rules_survive_conversion() {
        let f = file();
        let get = |id: &str| {
            f.rules
                .iter()
                .find(|r| r.id == id)
                .unwrap_or_else(|| panic!("missing {id}"))
        };
        let epic = get("kudu.games.epic");
        assert!(epic
            .targets
            .iter()
            .any(|t| t == "%LOCALAPPDATA%\\EpicGamesLauncher\\Saved\\Logs"));
        let jb = get("kudu.apps.jetbrains");
        assert_eq!(
            jb.targets,
            vec!["%LOCALAPPDATA%\\JetBrains\\*\\caches".to_string()]
        );
        let web = get("kudu.games.epic_webcache");
        assert_eq!(web.min_age_days, 7, "upstream minAgeDays carried over");
        let temp = get("kudu.system.user_temp_files");
        assert_eq!(temp.min_age_days, 2, "deepRecencyCheck becomes two days");
        let wu = get("kudu.system.windows_update_cache");
        assert!(wu.requires.contains(&Requirement::Admin));
        let pnpm = get("kudu.apps.pnpm");
        assert!(matches!(&pnpm.action, Action::Command { program, .. } if program == "pnpm"));
    }

    #[test]
    fn nothing_on_the_left_out_list_came_in() {
        let all: Vec<String> = file()
            .rules
            .iter()
            .flat_map(|r| r.targets.iter().map(|t| normalize_target(t)))
            .collect();
        for bad in [
            "%windir%/prefetch",
            "%windir%/winsxs",
            "%systemdrive%/windows.old",
            "%windir%/installer",
            "%windir%/system32",
        ] {
            assert!(
                !all.iter()
                    .any(|t| t == bad || t.starts_with(&format!("{bad}/"))),
                "{bad} leaked in"
            );
        }
        assert!(
            !file()
                .rules
                .iter()
                .any(|r| r.id == "kudu.games.steam_shader_cache"),
            "installed games keep their shader cache"
        );
        assert!(!file().rules.iter().any(|r| r.id.contains("defender")));
    }

    #[test]
    fn normalize_merges_spellings() {
        assert_eq!(
            normalize_target("%USERPROFILE%\\AppData\\Local\\Temp\\x"),
            "%localappdata%/temp/x"
        );
        assert_eq!(normalize_target("%TEMP%"), "%localappdata%/temp");
        assert_eq!(
            normalize_target("${LOCALAPPDATA}/Temp/"),
            "%localappdata%/temp"
        );
        assert_eq!(
            normalize_target("$localappdata\\Google"),
            "%localappdata%/google"
        );
        assert_eq!(normalize_target("%SystemRoot%\\Temp"), "%windir%/temp");
        assert_eq!(
            normalize_target("%ProgramFiles(x86)%\\Steam"),
            "%programfiles_x86%/steam"
        );
        assert_eq!(
            normalize_target("%USERPROFILE%\\AppData\\Roaming\\x"),
            "%appdata%/x"
        );
    }

    #[test]
    fn covers_is_segment_and_wildcard_aware() {
        assert!(covers("%LOCALAPPDATA%\\Temp", "%TEMP%\\foo"));
        assert!(covers(
            "%LOCALAPPDATA%\\Google\\Chrome\\User Data\\*\\Cache",
            "${LOCALAPPDATA}/Google/Chrome/User Data/*/Cache/Cache_Data"
        ));
        assert!(covers(
            "%LOCALAPPDATA%\\Explorer\\*",
            "%LOCALAPPDATA%\\Explorer\\thumbcache_1.db"
        ));
        assert!(!covers(
            "%LOCALAPPDATA%\\Explorer\\thumbcache_*.db",
            "%LOCALAPPDATA%\\Explorer\\*"
        ));
        assert!(!covers("%LOCALAPPDATA%\\Temp", "%LOCALAPPDATA%\\TempFiles"));
        assert!(!covers("%LOCALAPPDATA%\\A\\B", "%LOCALAPPDATA%\\A"));
    }

    fn r(id: &str, targets: &[&str]) -> Rule {
        Rule {
            id: id.into(),
            os: vec![Os::Windows],
            category: "system".into(),
            label_key: None,
            label: id.into(),
            description: String::new(),
            targets: targets.iter().map(|s| s.to_string()).collect(),
            include: vec![],
            exclude: vec![],
            running: vec![],
            min_age_days: 1,
            action: Action::DeleteContents,
            destination: Destination::Unlink,
            risk: Risk::Destructive,
            requires: vec![],
            source: "test".into(),
            license: "MIT".into(),
        }
    }

    #[test]
    fn dedupe_keeps_the_primary_and_the_rest() {
        let primary = vec![
            r("ours.temp", &["%LOCALAPPDATA%\\Temp"]),
            r(
                "ours.thumbs",
                &["%LOCALAPPDATA%\\Microsoft\\Windows\\Explorer\\thumbcache_*.db"],
            ),
        ];
        let secondary = vec![
            r("k.temp", &["${LOCALAPPDATA}/Temp"]),
            r(
                "k.explorer",
                &["%LOCALAPPDATA%\\Microsoft\\Windows\\Explorer"],
            ),
            r("k.mixed", &["%TEMP%\\sub", "%APPDATA%\\Other\\Cache"]),
            r("k.broad", &["%LOCALAPPDATA%\\*\\EBWebView\\*\\Cache"]),
        ];
        let out = dedupe_against(&primary, secondary);
        let ids: Vec<&str> = out.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["k.mixed", "k.broad"]);
        assert_eq!(out[0].targets, vec!["%APPDATA%\\Other\\Cache".to_string()]);
    }

    #[test]
    fn dedupe_drops_the_same_native_command() {
        let mut p = r("ours.docker", &[]);
        p.action = Action::Command {
            program: "docker".into(),
            args: vec!["builder".into(), "prune".into()],
            measure: vec![],
        };
        let mut s = p.clone();
        s.id = "k.docker".into();
        let mut other = p.clone();
        other.id = "k.pnpm".into();
        other.action = Action::Command {
            program: "pnpm".into(),
            args: vec!["store".into(), "prune".into()],
            measure: vec![],
        };
        let out = dedupe_against(&[p], vec![s, other]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "k.pnpm");
    }
}
