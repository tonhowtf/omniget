//! (Copyright (c) 2017 Marcin Szeniak, Apache License 2.0, see

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Root {
    Hklm,
    Hklm32,
    Hkcu,
    Hkcu32,
}

pub const UNINSTALL: &str = r"Microsoft\Windows\CurrentVersion\Uninstall";

impl Root {
    pub const ALL: [Root; 4] = [Root::Hklm, Root::Hklm32, Root::Hkcu, Root::Hkcu32];

    pub fn as_str(self) -> &'static str {
        match self {
            Root::Hklm => "hklm",
            Root::Hklm32 => "hklm32",
            Root::Hkcu => "hkcu",
            Root::Hkcu32 => "hkcu32",
        }
    }

    pub fn parse(s: &str) -> Option<Root> {
        Root::ALL.into_iter().find(|r| r.as_str() == s)
    }

    pub fn machine(self) -> bool {
        matches!(self, Root::Hklm | Root::Hklm32)
    }

    pub fn x86(self) -> bool {
        matches!(self, Root::Hklm32 | Root::Hkcu32)
    }

    pub fn uninstall_key(self) -> String {
        match self {
            Root::Hklm => format!(r"HKLM\SOFTWARE\{UNINSTALL}"),
            Root::Hklm32 => format!(r"HKLM\SOFTWARE\WOW6432Node\{UNINSTALL}"),
            Root::Hkcu => format!(r"HKCU\Software\{UNINSTALL}"),
            Root::Hkcu32 => format!(r"HKCU\Software\WOW6432Node\{UNINSTALL}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegEntry {
    pub root: Root,
    pub key_name: String,
    pub values: BTreeMap<String, String>,
}

impl RegEntry {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim())
            .filter(|v| !v.is_empty())
    }

    pub fn get_prefixed(&self, name: &str) -> Option<&str> {
        self.get(name).or_else(|| {
            let lower = name.to_ascii_lowercase();
            self.values
                .iter()
                .find(|(k, v)| k.to_ascii_lowercase().starts_with(&lower) && !v.trim().is_empty())
                .map(|(_, v)| v.trim())
        })
    }

    pub fn flag(&self, name: &str) -> bool {
        self.get(name).and_then(dword) == Some(1)
    }
    pub fn is_update(&self) -> bool {
        if self.get("ParentKeyName").is_some() {
            return true;
        }
        if let Some(t) = self.get("ReleaseType") {
            let t = t.to_ascii_lowercase();
            if t.contains("update") || t.contains("hotfix") {
                return true;
            }
        }
        is_kb(&self.key_name) || self.get("").map(is_kb).unwrap_or(false)
    }

    pub fn product_code(&self) -> Option<String> {
        self.get("BundleProviderKey")
            .and_then(guid_in)
            .or_else(|| is_guid(&self.key_name).then(|| self.key_name.to_ascii_uppercase()))
            .or_else(|| self.get_prefixed("UninstallString").and_then(guid_in))
    }
}

fn is_kb(s: &str) -> bool {
    let s = s.trim();
    s.len() > 2 && s[..2].eq_ignore_ascii_case("kb") && s[2..].chars().all(|c| c.is_ascii_digit())
}

pub fn dword(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(h, 16).ok()
    } else {
        s.parse().ok()
    }
}

pub fn install_date(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() != 8 || !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let y: i64 = s[..4].parse().ok()?;
    let a: u32 = s[4..6].parse().ok()?;
    let b: u32 = s[6..8].parse().ok()?;
    let (m, d) = if (1..=12).contains(&a) {
        (a, b)
    } else {
        (b, a)
    };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || !(1980..=2100).contains(&y) {
        return None;
    }
    Some(days_from_civil(y, m, d) * 86_400)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn is_guid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 38 || b[0] != b'{' || b[37] != b'}' {
        return false;
    }
    b[1..37].iter().enumerate().all(|(i, c)| match i {
        8 | 13 | 18 | 23 => *c == b'-',
        _ => c.is_ascii_hexdigit(),
    })
}

pub fn guid_in(s: &str) -> Option<String> {
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        let cand = rest.get(i..i + 38)?;
        if is_guid(cand) {
            return Some(cand.to_ascii_uppercase());
        }
        rest = &rest[i + 1..];
    }
    None
}

pub fn winget_id_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.starts_with('-')
        && !s.ends_with('\u{2026}')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
}

pub fn appx_name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

pub fn store_id_ok(s: &str) -> bool {
    (12..=14).contains(&s.len())
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

pub fn scoop_name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdLine {
    pub program: String,
    pub args: Vec<String>,
    pub raw_args: String,
}

pub fn argv_split(s: &str) -> Vec<String> {
    let c: Vec<char> = s.chars().collect();
    let n = c.len();
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        while i < n && (c[i] == ' ' || c[i] == '\t') {
            i += 1;
        }
        if i >= n {
            break;
        }
        let mut cur = String::new();
        let mut quoted = false;
        while i < n {
            let ch = c[i];
            if !quoted && (ch == ' ' || ch == '\t') {
                break;
            }
            if ch == '\\' {
                let mut k = 0;
                while i + k < n && c[i + k] == '\\' {
                    k += 1;
                }
                if i + k < n && c[i + k] == '"' {
                    cur.extend(std::iter::repeat_n('\\', k / 2));
                    if k % 2 == 1 {
                        cur.push('"');
                        i += k + 1;
                    } else {
                        i += k;
                    }
                } else {
                    cur.extend(std::iter::repeat_n('\\', k));
                    i += k;
                }
                continue;
            }
            if ch == '"' {
                if quoted && i + 1 < n && c[i + 1] == '"' {
                    cur.push('"');
                    i += 2;
                    continue;
                }
                quoted = !quoted;
                i += 1;
                continue;
            }
            cur.push(ch);
            i += 1;
        }
        out.push(cur);
    }
    out
}

pub fn split_command(line: &str, exists: &dyn Fn(&str) -> bool) -> Option<CmdLine> {
    let t = line.trim();
    if t.is_empty() {
        return None;
    }
    let (program, rest) = if let Some(body) = t.strip_prefix('"') {
        let end = body.find('"')?;
        (&body[..end], &body[end + 1..])
    } else {
        let lower = t.to_ascii_lowercase();
        let mut first = None;
        let mut chosen = None;
        for (i, _) in lower.match_indices(".exe") {
            let end = i + 4;
            let at_end = end == t.len() || t[end..].starts_with([' ', '\t']);
            if !at_end {
                continue;
            }
            first.get_or_insert(end);
            if exists(&t[..end]) {
                chosen = Some(end);
                break;
            }
        }
        let end = chosen
            .or(first)
            .unwrap_or_else(|| t.find([' ', '\t']).unwrap_or(t.len()));
        (&t[..end], &t[end..])
    };
    let program = program.trim();
    if program.is_empty() {
        return None;
    }
    let raw_args = rest.trim().to_string();
    Some(CmdLine {
        program: program.to_string(),
        args: argv_split(&raw_args),
        raw_args,
    })
}

pub fn win_file_name(p: &str) -> &str {
    p.rsplit(['\\', '/']).next().unwrap_or(p)
}

pub fn win_parent(p: &str) -> &str {
    match p.rfind(['\\', '/']) {
        Some(i) => &p[..i],
        None => "",
    }
}

pub fn norm_path(p: &str) -> String {
    let s = p.trim().trim_matches('"').replace('/', "\\").to_lowercase();
    let s = s.replace("\\\\", "\\");
    s.trim_end_matches('\\').to_string()
}

pub fn path_inside(child: &str, parent: &str) -> bool {
    let c = norm_path(child);
    let p = norm_path(parent);
    !p.is_empty() && (c == p || c.starts_with(&format!("{p}\\")))
}

pub fn expand_env(s: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find('%') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        match after.find('%') {
            Some(b) if b > 0 => {
                let name = &after[..b];
                match lookup(name) {
                    Some(v) => out.push_str(&v),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[b + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn program_refused(program: &str) -> Option<&'static str> {
    let name = win_file_name(program).to_ascii_lowercase();
    let name = name.trim_end_matches(".exe");
    match name {
        "cmd" | "powershell" | "pwsh" | "wscript" | "cscript" | "mshta" | "bash" | "wsl"
        | "conhost" | "explorer" => {
            Some("the uninstall entry runs a shell or script host, which OmniGet does not start")
        }
        _ if !program.to_ascii_lowercase().ends_with(".exe") && name != "msiexec" => {
            Some("the uninstall entry does not point to an .exe")
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Msi,
    Inno,
    Nsis,
    Wix,
    Squirrel,
    Other,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Msi => "msi",
            Kind::Inno => "inno",
            Kind::Nsis => "nsis",
            Kind::Wix => "wix",
            Kind::Squirrel => "squirrel",
            Kind::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        [
            Kind::Msi,
            Kind::Inno,
            Kind::Nsis,
            Kind::Wix,
            Kind::Squirrel,
            Kind::Other,
        ]
        .into_iter()
        .find(|k| k.as_str() == s)
    }

    pub fn msi_like(self) -> bool {
        matches!(self, Kind::Msi | Kind::Wix | Kind::Other)
    }
}

pub trait Probe {
    fn exists(&self, path: &str) -> bool;
    fn nullsoft(&self, path: &str) -> bool;
}

pub fn detect_kind(e: &RegEntry, probe: &dyn Probe) -> Kind {
    if e.flag("WindowsInstaller") {
        return Kind::Msi;
    }
    if e.values
        .keys()
        .any(|k| k.to_ascii_lowercase().starts_with("inno setup:"))
    {
        return Kind::Inno;
    }
    let us = e.get_prefixed("UninstallString").unwrap_or("");
    let lower = us.to_ascii_lowercase();
    if lower.contains("msiexec") {
        return Kind::Msi;
    }
    if lower.contains(r"\package cache\{")
        || e.get("BundleProviderKey").is_some()
        || e.get("BundleCachePath").is_some()
    {
        return Kind::Wix;
    }
    let Some(cmd) = split_command(us, &|p| probe.exists(p)) else {
        return Kind::Other;
    };
    let file = win_file_name(&cmd.program).to_ascii_lowercase();
    if is_inno_uninstaller(&file) {
        let dat = format!("{}.dat", &cmd.program[..cmd.program.len() - 4]);
        if probe.exists(&dat) {
            return Kind::Inno;
        }
    }
    if file == "update.exe" && lower.contains("--uninstall") {
        return Kind::Squirrel;
    }
    if file.ends_with(".exe") && probe.exists(&cmd.program) && probe.nullsoft(&cmd.program) {
        return Kind::Nsis;
    }
    Kind::Other
}

pub fn is_inno_uninstaller(file: &str) -> bool {
    let f = file.to_ascii_lowercase();
    f.len() == 12
        && f.starts_with("unins")
        && f.ends_with(".exe")
        && f[5..8].chars().all(|c| c.is_ascii_digit())
}

pub fn has_nullsoft(bytes: &[u8]) -> bool {
    bytes.windows(8).any(|w| w == b"Nullsoft")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WingetRow {
    pub name: String,
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub available: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub truncated: bool,
}

impl WingetRow {
    pub fn arp(&self) -> Option<(Root, String)> {
        let rest = self.id.strip_prefix(r"ARP\")?;
        let mut parts = rest.splitn(3, '\\');
        let scope = parts.next()?.to_ascii_lowercase();
        let arch = parts.next()?.to_ascii_lowercase();
        let key = parts.next()?.to_string();
        let root = match (scope.as_str(), arch.as_str()) {
            ("machine", "x86") => Root::Hklm32,
            ("machine", _) => Root::Hklm,
            ("user", "x86") => Root::Hkcu32,
            ("user", _) => Root::Hkcu,
            _ => return None,
        };
        Some((root, key))
    }

    pub fn package_id(&self) -> Option<&str> {
        (self.source.is_some() && winget_id_ok(&self.id)).then_some(self.id.as_str())
    }
}

fn char_width(c: char) -> usize {
    let u = c as u32;
    let wide = (0x1100..=0x115F).contains(&u)
        || (0x2E80..=0xA4CF).contains(&u)
        || (0xAC00..=0xD7A3).contains(&u)
        || (0xF900..=0xFAFF).contains(&u)
        || (0xFE30..=0xFE4F).contains(&u)
        || (0xFF00..=0xFF60).contains(&u)
        || (0xFFE0..=0xFFE6).contains(&u)
        || (0x1F300..=0x1FAFF).contains(&u)
        || (0x20000..=0x3FFFD).contains(&u);
    if wide {
        2
    } else {
        1
    }
}

fn cut(line: &str, a: usize, b: Option<usize>) -> String {
    let mut col = 0;
    let mut out = String::new();
    for c in line.chars() {
        let w = char_width(c);
        if col >= a && b.is_none_or(|b| col < b) {
            out.push(c);
        }
        col += w;
    }
    out.trim().to_string()
}

pub fn parse_winget_list(text: &str) -> Vec<WingetRow> {
    let lines: Vec<&str> = text
        .split('\n')
        .map(|l| {
            let l = l.trim_end_matches('\r');
            l.rsplit('\r').next().unwrap_or(l)
        })
        .collect();
    let Some(sep) = lines.iter().position(|l| {
        let t = l.trim();
        t.len() >= 10 && t.chars().all(|c| c == '-')
    }) else {
        return Vec::new();
    };
    if sep == 0 {
        return Vec::new();
    }
    let header = lines[sep - 1];
    let mut starts: Vec<(usize, String)> = Vec::new();
    let mut col = 0;
    let mut prev_space = true;
    for c in header.chars() {
        let space = c == ' ';
        if !space && prev_space {
            starts.push((col, String::new()));
        }
        if !space {
            if let Some(last) = starts.last_mut() {
                last.1.push(c);
            }
        }
        prev_space = space;
        col += char_width(c);
    }
    if starts.len() < 3 {
        return Vec::new();
    }
    let (id_c, ver_c) = (starts[1].0, starts[2].0);
    let mut avail_c = None;
    let mut src_c = None;
    match starts.len() {
        3 => {}
        4 => {
            let w = starts[3].1.to_lowercase();
            if [
                "source",
                "origem",
                "quelle",
                "origen",
                "origine",
                "источник",
                "来源",
                "源",
            ]
            .contains(&w.as_str())
            {
                src_c = Some(starts[3].0);
            } else if [
                "available",
                "disponível",
                "verfügbar",
                "disponible",
                "disponibile",
                "доступно",
                "可用",
            ]
            .contains(&w.as_str())
            {
                avail_c = Some(starts[3].0);
            } else {
                let values: Vec<String> = lines[sep + 1..]
                    .iter()
                    .map(|l| cut(l, starts[3].0, None))
                    .filter(|v| !v.is_empty())
                    .collect();
                if !values.is_empty() && values.iter().all(|v| v == "winget" || v == "msstore") {
                    src_c = Some(starts[3].0);
                } else {
                    avail_c = Some(starts[3].0);
                }
            }
        }
        _ => {
            avail_c = Some(starts[3].0);
            src_c = Some(starts[4].0);
        }
    }
    let ver_end = avail_c.or(src_c);
    let mut out = Vec::new();
    for l in &lines[sep + 1..] {
        if l.trim().is_empty() {
            continue;
        }
        let width: usize = l.chars().map(char_width).sum();
        if width <= ver_c {
            continue;
        }
        let name = cut(l, 0, Some(id_c));
        let id = cut(l, id_c, Some(ver_c));
        let version = cut(l, ver_c, ver_end);
        if id.is_empty() || (id.contains(' ') && !id.starts_with(r"ARP\")) || name.is_empty() {
            continue;
        }
        let available = avail_c.map(|a| cut(l, a, src_c)).filter(|s| !s.is_empty());
        let source = src_c.map(|s| cut(l, s, None)).filter(|s| !s.is_empty());
        let truncated = name.ends_with('\u{2026}') || id.ends_with('\u{2026}');
        out.push(WingetRow {
            name,
            id,
            version,
            available,
            source,
            truncated,
        });
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExitMeaning {
    pub code: i32,
    pub ok: bool,
    pub skipped: bool,
    pub restart: bool,
    pub text: String,
}

fn meaning(
    code: i32,
    ok: bool,
    skipped: bool,
    restart: bool,
    text: impl Into<String>,
) -> ExitMeaning {
    ExitMeaning {
        code,
        ok,
        skipped,
        restart,
        text: text.into(),
    }
}

pub fn winget_exit(code: i32) -> ExitMeaning {
    match code {
        0 => meaning(code, true, false, false, "uninstalled"),
        3010 | -1978334967 => meaning(
            code,
            true,
            false,
            true,
            "uninstalled, a restart is needed to finish",
        ),
        1641 | -1978334965 => meaning(
            code,
            true,
            false,
            true,
            "uninstalled, the uninstaller started a restart",
        ),
        -1978334966 => meaning(
            code,
            false,
            false,
            true,
            "Windows needs a restart before this can run",
        ),
        -1978335107 => meaning(
            code,
            false,
            false,
            false,
            "winget will not touch an app installed for one user while it runs as administrator",
        ),
        -1978335135 => meaning(code, false, true, false, "already installed"),
        -1978335189 => meaning(code, false, true, false, "no applicable update"),
        -1978335212 => meaning(
            code,
            false,
            true,
            false,
            "winget found no installed package with this id",
        ),
        _ => meaning(
            code,
            false,
            false,
            false,
            format!("winget failed (0x{:08X})", code as u32),
        ),
    }
}

pub fn installer_exit(kind: Kind, code: i32) -> ExitMeaning {
    match code {
        0 => meaning(code, true, false, false, "uninstalled"),
        3010 => meaning(
            code,
            true,
            false,
            true,
            "uninstalled, a restart is needed to finish",
        ),
        1641 => meaning(
            code,
            true,
            false,
            true,
            "uninstalled, the uninstaller started a restart",
        ),
        1605 if kind == Kind::Msi => meaning(
            code,
            false,
            true,
            false,
            "this product is not installed anymore",
        ),
        1602 => meaning(code, false, true, false, "cancelled in the uninstaller"),
        1 | 2 if kind == Kind::Nsis => {
            meaning(code, false, true, false, "cancelled in the uninstaller")
        }
        1618 => meaning(
            code,
            false,
            false,
            false,
            "another installation is running; try again when it finishes",
        ),
        1603 => meaning(
            code,
            false,
            false,
            false,
            "the uninstaller hit a fatal error (1603)",
        ),
        740 => meaning(
            code,
            false,
            false,
            false,
            "the uninstaller needs administrator rights",
        ),
        5 => meaning(
            code,
            false,
            false,
            false,
            "access denied: the uninstaller needs administrator rights",
        ),
        2 | 3 => meaning(
            code,
            false,
            false,
            false,
            "a file the uninstaller needs is missing",
        ),
        9009 => meaning(code, false, false, false, "the uninstaller was not found"),
        -1073741510 => meaning(code, false, false, false, "the uninstaller was terminated"),
        -2147024846 => meaning(
            code,
            false,
            false,
            false,
            "this is part of Windows and cannot be removed for one user",
        ),
        _ => meaning(
            code,
            false,
            false,
            false,
            format!("the uninstaller failed with code {code}"),
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Vdf {
    Str(String),
    Map(Vec<(String, Vdf)>),
}

impl Vdf {
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Map(m) => m
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Vdf::Str(s) => Some(s),
            Vdf::Map(_) => None,
        }
    }

    pub fn entries(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Map(m) => m,
            Vdf::Str(_) => &[],
        }
    }
}

pub fn parse_vdf(text: &str) -> Vdf {
    let c: Vec<char> = text.chars().collect();
    let mut i = 0;
    Vdf::Map(vdf_map(&c, &mut i))
}

fn vdf_token(c: &[char], i: &mut usize) -> Option<String> {
    loop {
        while *i < c.len() && c[*i].is_whitespace() {
            *i += 1;
        }
        if *i + 1 < c.len() && c[*i] == '/' && c[*i + 1] == '/' {
            while *i < c.len() && c[*i] != '\n' {
                *i += 1;
            }
            continue;
        }
        break;
    }
    if *i >= c.len() {
        return None;
    }
    match c[*i] {
        '{' | '}' => {
            *i += 1;
            Some(c[*i - 1].to_string())
        }
        '"' => {
            *i += 1;
            let mut s = String::new();
            while *i < c.len() && c[*i] != '"' {
                if c[*i] == '\\' && *i + 1 < c.len() {
                    *i += 1;
                    s.push(match c[*i] {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                } else {
                    s.push(c[*i]);
                }
                *i += 1;
            }
            *i += 1;
            Some(format!("\u{0}{s}"))
        }
        _ => {
            let mut s = String::new();
            while *i < c.len()
                && !c[*i].is_whitespace()
                && c[*i] != '{'
                && c[*i] != '}'
                && c[*i] != '"'
            {
                s.push(c[*i]);
                *i += 1;
            }
            Some(format!("\u{0}{s}"))
        }
    }
}

fn vdf_map(c: &[char], i: &mut usize) -> Vec<(String, Vdf)> {
    let mut out = Vec::new();
    while let Some(tok) = vdf_token(c, i) {
        if tok == "}" {
            break;
        }
        if tok == "{" {
            let _ = vdf_map(c, i);
            continue;
        }
        let key = tok.trim_start_matches('\u{0}').to_string();
        match vdf_token(c, i) {
            Some(t) if t == "{" => out.push((key, Vdf::Map(vdf_map(c, i)))),
            Some(t) if t == "}" => break,
            Some(t) => out.push((key, Vdf::Str(t.trim_start_matches('\u{0}').to_string()))),
            None => break,
        }
    }
    out
}

pub fn steam_libraries(text: &str) -> Vec<String> {
    let root = parse_vdf(text);
    let Some(lf) = root
        .get("libraryfolders")
        .or_else(|| root.get("LibraryFolders"))
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (k, v) in lf.entries() {
        if !k.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let path = match v {
            Vdf::Str(s) => Some(s.clone()),
            Vdf::Map(_) => v.str("path").map(str::to_string),
        };
        if let Some(p) = path.filter(|p| !p.is_empty()) {
            if !out.iter().any(|o: &String| norm_path(o) == norm_path(&p)) {
                out.push(p);
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SteamApp {
    pub appid: String,
    pub name: String,
    pub installdir: String,
    pub size: Option<u64>,
    pub last_updated: Option<i64>,
}

pub fn steam_manifest(text: &str) -> Option<SteamApp> {
    let root = parse_vdf(text);
    let st = root.get("AppState")?;
    let appid = st.str("appid")?.to_string();
    if appid.is_empty() || !appid.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(SteamApp {
        name: st.str("name").unwrap_or(&appid).to_string(),
        installdir: st.str("installdir").unwrap_or("").to_string(),
        size: st.str("SizeOnDisk").and_then(|s| s.parse().ok()),
        last_updated: st.str("LastUpdated").and_then(|s| s.parse().ok()),
        appid,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppxPkg {
    pub name: String,
    pub full_name: String,
    pub family: String,
    pub install_location: Option<String>,
    pub signature: String,
    pub version: Option<String>,
    pub publisher: Option<String>,
    pub non_removable: bool,
}

pub fn parse_appx_json(text: &str) -> Vec<AppxPkg> {
    let v: serde_json::Value =
        match serde_json::from_str(text.trim().trim_start_matches('\u{feff}')) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
    let items = match v {
        serde_json::Value::Array(a) => a,
        o @ serde_json::Value::Object(_) => vec![o],
        _ => Vec::new(),
    };
    let s = |o: &serde_json::Value, k: &str| {
        o.get(k)
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(str::to_string)
    };
    let b = |o: &serde_json::Value, k: &str| o.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
    items
        .iter()
        .filter(|o| !b(o, "IsFramework") && !b(o, "IsResourcePackage"))
        .filter_map(|o| {
            let name = s(o, "Name")?;
            let full_name = s(o, "PackageFullName").unwrap_or_else(|| name.clone());
            let family = s(o, "PackageFamilyName").unwrap_or_else(|| name.clone());
            let signature = match o.get("SignatureKind") {
                Some(serde_json::Value::Number(n)) => match n.as_u64() {
                    Some(0) => "None",
                    Some(1) => "Developer",
                    Some(2) => "Enterprise",
                    Some(3) => "Store",
                    Some(4) => "System",
                    _ => "None",
                }
                .to_string(),
                Some(serde_json::Value::String(x)) => x.clone(),
                _ => "None".to_string(),
            };
            let version = match o.get("Version") {
                Some(serde_json::Value::String(x)) => Some(x.clone()),
                Some(obj @ serde_json::Value::Object(_)) => {
                    let n = |k: &str| obj.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
                    Some(format!(
                        "{}.{}.{}.{}",
                        n("Major"),
                        n("Minor"),
                        n("Build"),
                        n("Revision")
                    ))
                }
                _ => None,
            };
            let publisher = s(o, "Publisher").map(|p| publisher_of_dn(&p));
            Some(AppxPkg {
                install_location: s(o, "InstallLocation"),
                non_removable: b(o, "NonRemovable"),
                name,
                full_name,
                family,
                signature,
                version,
                publisher,
            })
        })
        .collect()
}

pub fn publisher_of_dn(dn: &str) -> String {
    let part = |key: &str| {
        dn.split(',')
            .map(str::trim)
            .find_map(|p| p.strip_prefix(key))
            .map(|v| v.trim_matches('"').to_string())
    };
    part("O=")
        .or_else(|| part("CN="))
        .unwrap_or_else(|| dn.to_string())
}

pub fn csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut it = line.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' if quoted && it.peek() == Some(&'"') => {
                cur.push('"');
                it.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

pub fn schtasks_csv(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for l in text.lines() {
        let f = csv_line(l.trim_end_matches('\r'));
        if f.len() < 9 {
            continue;
        }
        let (task, run) = (f[1].trim(), f[8].trim());
        if !task.starts_with('\\') || out.iter().any(|(t, _)| t == task) {
            continue;
        }
        out.push((task.to_string(), run.to_string()));
    }
    out
}

pub fn lnk_mentions(bytes: &[u8], needle: &str) -> bool {
    let n = needle.trim_end_matches(['\\', '/']).to_lowercase();
    if n.len() < 4 {
        return false;
    }
    let ascii: String = bytes
        .iter()
        .map(|b| (*b as char).to_ascii_lowercase())
        .collect();
    if ascii.contains(&n) {
        return true;
    }
    for off in 0..2 {
        let units: Vec<u16> = bytes[off.min(bytes.len())..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let s = String::from_utf16_lossy(&units).to_lowercase();
        if s.contains(&n) {
            return true;
        }
    }
    false
}

pub fn normalize_name(s: &str) -> String {
    let mut t = String::new();
    let mut depth = 0;
    for c in s.to_lowercase().chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth > 0 => {}
            '_' => t.push(' '),
            _ => t.push(c),
        }
    }
    let words: Vec<&str> = t
        .split_whitespace()
        .filter(|w| {
            let w = w.trim_matches(|c: char| c == ',' || c == '-');
            !w.is_empty()
                && !matches!(
                    w,
                    "x64"
                        | "x86"
                        | "amd64"
                        | "arm64"
                        | "64-bit"
                        | "32-bit"
                        | "64bit"
                        | "32bit"
                        | "win64"
                        | "win32"
                )
                && !(w.starts_with(|c: char| c.is_ascii_digit())
                    && w.chars().all(|c| c.is_ascii_digit() || c == '.'))
                && !(w.starts_with('v')
                    && w.len() > 1
                    && w[1..].chars().all(|c| c.is_ascii_digit() || c == '.'))
        })
        .collect();
    words
        .join(" ")
        .trim_matches(|c: char| c == '-' || c == ' ' || c == ',')
        .to_string()
}

fn compact(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NameMatch {
    Perfect,
    Partial,
    None,
}

pub fn name_match(candidate: &str, product: &str, publisher: Option<&str>) -> NameMatch {
    let c = normalize_name(candidate);
    let p = normalize_name(product);
    if c.chars().count() < 2 || p.chars().count() < 2 {
        return NameMatch::None;
    }
    if c == p || (compact(&c) == compact(&p) && compact(&c).len() >= 3) {
        return NameMatch::Perfect;
    }
    if let Some(publ) = publisher.map(normalize_name).filter(|x| !x.is_empty()) {
        let first = publ.split_whitespace().next().unwrap_or("");
        for prefix in [publ.as_str(), first] {
            if let Some(rest) = p.strip_prefix(prefix).map(str::trim) {
                if !rest.is_empty()
                    && rest.len() >= 3
                    && (rest == c || compact(rest) == compact(&c))
                {
                    return NameMatch::Perfect;
                }
            }
        }
    }
    let (cl, pl) = (c.chars().count(), p.chars().count());
    if cl <= 4 || pl <= 4 {
        return NameMatch::None;
    }
    let d = levenshtein(&c, &p);
    if d <= 1 {
        return NameMatch::Perfect;
    }
    if p.contains(&c) || c.contains(&p) {
        return NameMatch::Partial;
    }
    if d < cl.min(pl) / 3 {
        return NameMatch::Partial;
    }
    NameMatch::None
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIX: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/washmybelly_fixtures/apps_windows/"
    );

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FIX}{name}")).unwrap()
    }

    struct FakeDisk(Vec<&'static str>, Vec<&'static str>);
    impl Probe for FakeDisk {
        fn exists(&self, p: &str) -> bool {
            self.0.iter().any(|x| x.eq_ignore_ascii_case(p))
        }
        fn nullsoft(&self, p: &str) -> bool {
            self.1.iter().any(|x| x.eq_ignore_ascii_case(p))
        }
    }

    #[test]
    fn install_date_reads_both_orders() {
        assert_eq!(install_date("20240131"), Some(1_706_659_200));
        assert_eq!(install_date("20243101"), Some(1_706_659_200));
        assert_eq!(install_date("2024"), None);
    }

    #[test]
    fn guids_are_strict() {
        assert!(is_guid("{23170F69-40C1-2702-2301-000001000000}"));
        assert!(!is_guid("{23170F69-40C1-2702-2301-00000100000}"));
        assert!(!is_guid("23170F69-40C1-2702-2301-000001000000"));
        assert!(!is_guid("{23170F69-40C1-2702-2301-00000100000G}"));
        assert_eq!(
            guid_in("MsiExec.exe /X{ac76ba86-7ad7-1033-7b44-ac0f074e4100}").as_deref(),
            Some("{AC76BA86-7AD7-1033-7B44-AC0F074E4100}")
        );
        assert_eq!(guid_in("no guid {here"), None);
    }

    #[test]
    fn tokenizer_follows_windows_rules() {
        assert_eq!(argv_split(r#"/S "a b" c"#), vec!["/S", "a b", "c"]);
        assert_eq!(argv_split(r#"a\\\"b"#), vec![r#"a\"b"#]);
        assert_eq!(argv_split(r#""a\\" b"#), vec![r"a\", "b"]);
        assert_eq!(argv_split(r#""say ""hi""""#), vec![r#"say "hi""#]);
        assert_eq!(argv_split(r"C:\x\y.exe"), vec![r"C:\x\y.exe"]);
        assert!(argv_split("   ").is_empty());
    }

    #[test]
    fn split_command_handles_quotes_and_unquoted_spaces() {
        let none = |_: &str| false;
        let c =
            split_command(r#""C:\Program Files\VideoLAN\VLC\uninstall.exe" /S"#, &none).unwrap();
        assert_eq!(c.program, r"C:\Program Files\VideoLAN\VLC\uninstall.exe");
        assert_eq!(c.args, vec!["/S"]);
        assert_eq!(c.raw_args, "/S");
        let disk = |p: &str| p == r"C:\Program Files\Foo Bar\unins.exe";
        let c = split_command(r"C:\Program Files\Foo Bar\unins.exe /x C:\a.exe", &disk).unwrap();
        assert_eq!(c.program, r"C:\Program Files\Foo Bar\unins.exe");
        assert_eq!(c.raw_args, r"/x C:\a.exe");
        let c = split_command(r"C:\Program Files\Foo\setup.exe --uninstall", &none).unwrap();
        assert_eq!(c.program, r"C:\Program Files\Foo\setup.exe");
        assert_eq!(c.args, vec!["--uninstall"]);
        let c = split_command(
            "MsiExec.exe /X{AC76BA86-7AD7-1033-7B44-AC0F074E4100}",
            &none,
        )
        .unwrap();
        assert_eq!(c.program, "MsiExec.exe");
        assert!(split_command("  ", &none).is_none());
        assert!(split_command("\"unterminated", &none).is_none());
    }

    #[test]
    fn shells_are_refused_as_uninstallers() {
        assert!(program_refused(r"C:\Windows\System32\cmd.exe").is_some());
        assert!(program_refused("powershell.exe").is_some());
        assert!(program_refused(r"C:\x\uninstall.bat").is_some());
        assert!(program_refused("MsiExec").is_none());
        assert!(program_refused(r"C:\x\unins000.exe").is_none());
    }

    #[test]
    fn installer_kind_cascade() {
        let e = |pairs: &[(&str, &str)]| RegEntry {
            root: Root::Hklm,
            key_name: "X".into(),
            values: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        };
        let disk = FakeDisk(
            vec![
                r"C:\Inno\unins000.exe",
                r"C:\Inno\unins000.dat",
                r"C:\Nsis\uninst.exe",
                r"C:\Plain\remove.exe",
            ],
            vec![r"C:\Nsis\uninst.exe"],
        );
        assert_eq!(
            detect_kind(
                &e(&[("WindowsInstaller", "0x1"), ("UninstallString", "x")]),
                &disk
            ),
            Kind::Msi
        );
        assert_eq!(
            detect_kind(
                &e(&[(
                    "UninstallString",
                    "MsiExec.exe /X{AC76BA86-7AD7-1033-7B44-AC0F074E4100}"
                )]),
                &disk
            ),
            Kind::Msi
        );
        assert_eq!(
            detect_kind(
                &e(&[
                    ("Inno Setup: App Path", "C:\\Inno"),
                    ("UninstallString", "x")
                ]),
                &disk
            ),
            Kind::Inno
        );
        assert_eq!(
            detect_kind(
                &e(&[("UninstallString", r#""C:\Inno\unins000.exe""#)]),
                &disk
            ),
            Kind::Inno
        );
        assert_eq!(
            detect_kind(
                &e(&[("UninstallString", r#""C:\Other\unins000.exe""#)]),
                &disk
            ),
            Kind::Other,
            "no .dat, no Inno"
        );
        assert_eq!(
            detect_kind(
                &e(&[(
                    "UninstallString",
                    r#""C:\ProgramData\Package Cache\{AC76BA86-7AD7-1033-7B44-AC0F074E4100}\setup.exe" /uninstall"#
                )]),
                &disk
            ),
            Kind::Wix
        );
        assert_eq!(
            detect_kind(
                &e(&[(
                    "UninstallString",
                    r#""C:\Users\u\AppData\Local\Slack\Update.exe" --uninstall -s"#
                )]),
                &disk
            ),
            Kind::Squirrel
        );
        assert_eq!(
            detect_kind(&e(&[("UninstallString", r"C:\Nsis\uninst.exe")]), &disk),
            Kind::Nsis
        );
        assert_eq!(
            detect_kind(&e(&[("UninstallString", r"C:\Plain\remove.exe")]), &disk),
            Kind::Other
        );
        assert_eq!(
            detect_kind(
                &e(&[("UninstallString_hidden", r"C:\Nsis\uninst.exe")]),
                &disk
            ),
            Kind::Nsis,
            "hidden value"
        );
        assert!(is_inno_uninstaller("unins001.exe"));
        assert!(!is_inno_uninstaller("uninstall.exe"));
        assert!(has_nullsoft(b"xx Nullsoft Install System v3"));
    }

    #[test]
    fn winget_list_fixture() {
        let rows = parse_winget_list(&fixture("winget_list.txt"));
        let ff = rows.iter().find(|r| r.id == "Mozilla.Firefox").unwrap();
        assert_eq!(ff.name, "Mozilla Firefox (x64 en-US)");
        assert_eq!(ff.version, "120.0");
        assert_eq!(ff.available.as_deref(), Some("121.0"));
        assert_eq!(ff.source.as_deref(), Some("winget"));
        assert_eq!(ff.package_id(), Some("Mozilla.Firefox"));
        let arp = rows.iter().find(|r| r.id.starts_with(r"ARP\")).unwrap();
        assert_eq!(arp.source, None);
        assert_eq!(arp.package_id(), None);
        let (root, key) = arp.arp().unwrap();
        assert_eq!(root, Root::Hklm32);
        assert_eq!(key, "Foo Tool_is1");
        let cut = rows
            .iter()
            .find(|r| r.id.starts_with("SomeVendor"))
            .unwrap();
        assert!(cut.truncated);
        assert_eq!(cut.package_id(), None, "a cut id is not usable");
        let vc = rows
            .iter()
            .find(|r| r.id == "Microsoft.VCRedist.2015+.x64")
            .unwrap();
        assert!(vc.truncated, "the name was cut");
        assert_eq!(
            vc.package_id(),
            Some("Microsoft.VCRedist.2015+.x64"),
            "a whole id still works"
        );
        let store = rows
            .iter()
            .find(|r| r.source.as_deref() == Some("msstore"))
            .unwrap();
        assert_eq!(store.id, "9NBLGGH4R32N");
        assert!(rows.iter().all(|r| !r.name.contains('\r')));
    }

    #[test]
    fn winget_list_without_available_column() {
        let t = "Name       Id              Version  Source\n------------------------------------------\nGit        Git.Git         2.44.0   winget\nOld Thing  ARP\\User\\X64\\ot 1.0\n";
        let rows = parse_winget_list(t);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].source.as_deref(), Some("winget"));
        assert_eq!(rows[0].available, None);
        assert_eq!(rows[1].arp().unwrap().0, Root::Hkcu);
    }

    #[test]
    fn winget_list_in_portuguese() {
        let t = "Nome       ID              Versão   Disponível Origem\n-----------------------------------------------------\nGit        Git.Git         2.44.0   2.45.0     winget\n";
        let rows = parse_winget_list(t);
        assert_eq!(rows[0].available.as_deref(), Some("2.45.0"));
        assert_eq!(rows[0].source.as_deref(), Some("winget"));
    }

    #[test]
    fn winget_exit_codes() {
        assert!(winget_exit(0).ok);
        let r = winget_exit(-1978334967);
        assert!(r.ok && r.restart);
        assert!(winget_exit(1641).restart);
        assert!(!winget_exit(-1978335107).ok);
        assert!(winget_exit(-1978335212).skipped);
        assert!(winget_exit(-1978335135).skipped);
        assert_eq!(winget_exit(-1978335226).text, "winget failed (0x8A150006)");
        assert!(installer_exit(Kind::Msi, 1605).skipped);
        assert!(installer_exit(Kind::Nsis, 1).skipped);
        assert!(!installer_exit(Kind::Inno, 1).skipped);
        assert!(installer_exit(Kind::Msi, 3010).ok);
        assert!(!installer_exit(Kind::Msi, 1618).ok);
    }

    #[test]
    fn steam_library_and_manifest() {
        let libs = steam_libraries(&fixture("libraryfolders.vdf"));
        assert_eq!(
            libs,
            vec![
                r"C:\Program Files (x86)\Steam".to_string(),
                r"D:\SteamLibrary".to_string()
            ]
        );
        let old = steam_libraries(
            "\"LibraryFolders\"\n{\n \"TimeNextStatsReport\" \"1\"\n \"1\" \"E:\\\\Games\"\n}\n",
        );
        assert_eq!(old, vec![r"E:\Games".to_string()]);
        let app = steam_manifest(&fixture("appmanifest_730.acf")).unwrap();
        assert_eq!(app.appid, "730");
        assert_eq!(app.name, "Counter-Strike 2");
        assert_eq!(app.installdir, "Counter-Strike Global Offensive");
        assert_eq!(app.size, Some(35_000_000_000));
        assert!(steam_manifest("\"AppState\" { \"name\" \"x\" }").is_none());
    }

    #[test]
    fn appx_json_in_both_powershells() {
        let pkgs = parse_appx_json(&fixture("appx.json"));
        assert!(
            pkgs.iter().all(|p| p.name != "Microsoft.VCLibs.140.00"),
            "frameworks dropped"
        );
        let news = pkgs
            .iter()
            .find(|p| p.name == "Microsoft.BingNews")
            .unwrap();
        assert_eq!(news.signature, "Store");
        assert_eq!(news.version.as_deref(), Some("4.55.62231.0"));
        assert_eq!(news.publisher.as_deref(), Some("Microsoft Corporation"));
        let sys = pkgs
            .iter()
            .find(|p| p.name == "Microsoft.Windows.ShellExperienceHost")
            .unwrap();
        assert_eq!(sys.signature, "System");
        let one = parse_appx_json(
            r#"{"Name":"A.B","PackageFullName":"A.B_1","SignatureKind":"Store","Version":"1.0"}"#,
        );
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].version.as_deref(), Some("1.0"));
        assert!(parse_appx_json("not json").is_empty());
    }

    #[test]
    fn schtasks_csv_pairs() {
        let t = "\"PC\",\"\\Foo\\Updater\",\"N/A\",\"Ready\",\"Interactive\",\"N/A\",\"0\",\"Foo\",\"\"\"C:\\Program Files\\Foo\\upd.exe\"\" /check\",\"N/A\"\n\"PC\",\"\\Foo\\Updater\",\"N/A\",\"Ready\",\"Interactive\",\"N/A\",\"0\",\"Foo\",\"x\",\"N/A\"\nbad line\n";
        let v = schtasks_csv(t);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].0, r"\Foo\Updater");
        assert_eq!(v[0].1, r#""C:\Program Files\Foo\upd.exe" /check"#);
    }

    #[test]
    fn lnk_mentions_ascii_and_utf16() {
        let mut b = b"L\0\0\0 junk C:\\Program Files\\Foo\\foo.exe junk".to_vec();
        assert!(lnk_mentions(&b, r"C:\Program Files\Foo"));
        b.clear();
        for u in r"x C:\Apps\Bär\run.exe".encode_utf16() {
            b.extend_from_slice(&u.to_le_bytes());
        }
        assert!(lnk_mentions(&b, r"c:\apps\bär"));
        assert!(!lnk_mentions(&b, r"C:\Apps\Other"));
    }

    #[test]
    fn names_normalize_and_match() {
        assert_eq!(
            normalize_name("Mozilla Firefox 120.0 (x64 en-US)"),
            "mozilla firefox"
        );
        assert_eq!(normalize_name("7-Zip 23.01 (x64)"), "7-zip");
        assert_eq!(normalize_name("Notepad++ v8.6.2 64-bit"), "notepad++");
        assert_eq!(
            name_match("Mozilla Firefox", "Mozilla Firefox 120.0 (x64 en-US)", None),
            NameMatch::Perfect
        );
        assert_eq!(
            name_match("Firefox", "Mozilla Firefox", Some("Mozilla")),
            NameMatch::Perfect
        );
        assert_eq!(
            name_match("Reader", "Adobe Reader", Some("Adobe Inc.")),
            NameMatch::Perfect
        );
        assert_eq!(
            name_match("VideoLAN", "VLC media player", Some("VideoLAN")),
            NameMatch::None
        );
        assert_eq!(
            name_match("Obsidian Data", "Obsidian", None),
            NameMatch::Partial
        );
        assert_eq!(name_match("Git", "Git", None), NameMatch::Perfect);
        assert_eq!(
            name_match("Gitx", "Git", None),
            NameMatch::None,
            "short names only match exactly"
        );
        assert_eq!(
            name_match("Spotify", "Spotifx", None),
            NameMatch::Perfect,
            "one edit"
        );
        assert_eq!(name_match("Microsoft", "Discord", None), NameMatch::None);
    }

    #[test]
    fn env_expansion_and_paths() {
        let look = |k: &str| {
            (k.eq_ignore_ascii_case("ProgramFiles")).then(|| r"C:\Program Files".to_string())
        };
        assert_eq!(
            expand_env(r"%ProgramFiles%\Foo", &look),
            r"C:\Program Files\Foo"
        );
        assert_eq!(expand_env(r"%Nope%\x 100%", &look), r"%Nope%\x 100%");
        assert!(path_inside(
            r"C:\Program Files\Foo\bin",
            r"c:/program files/foo/"
        ));
        assert!(!path_inside(
            r"C:\Program Files\Foobar",
            r"C:\Program Files\Foo"
        ));
        assert_eq!(win_file_name(r"C:\a\b.exe"), "b.exe");
        assert_eq!(win_parent(r"C:\a\b.exe"), r"C:\a");
    }

    #[test]
    fn id_validators() {
        assert!(winget_id_ok("Mozilla.Firefox"));
        assert!(winget_id_ok("Notepad++.Notepad++"));
        assert!(!winget_id_ok("--id"));
        assert!(!winget_id_ok("A B"));
        assert!(!winget_id_ok("Mozilla.Fire\u{2026}"));
        assert!(appx_name_ok("Microsoft.BingNews"));
        assert!(!appx_name_ok("a';b"));
        assert!(store_id_ok("9NBLGGH4R32N"));
        assert!(store_id_ok("XP8BT8DW290MPQ"));
        assert!(!store_id_ok("9nbl"));
    }
}
