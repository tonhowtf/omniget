use super::{query, App, PackageBackend, RemovePreview, Update};

pub struct Winget;

pub fn char_width(c: char) -> usize {
    let u = c as u32;
    let wide = matches!(u,
        0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF |
        0xA000..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 |
        0xFFE0..=0xFFE6 | 0x1F300..=0x1F64F | 0x1F900..=0x1F9FF | 0x20000..=0x3FFFD);
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
        if col >= a && b.is_none_or(|b| col < b) {
            out.push(c);
        }
        col += char_width(c);
    }
    out.trim().to_string()
}

fn header_columns(header: &str) -> Vec<usize> {
    let mut cols = Vec::new();
    let mut col = 0;
    let mut prev_space = true;
    for c in header.chars() {
        if !c.is_whitespace() && prev_space {
            cols.push(col);
        }
        prev_space = c.is_whitespace();
        col += char_width(c);
    }
    cols
}

pub fn parse_upgrade(text: &str) -> Vec<Update> {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.rsplit('\r').next().unwrap_or(l))
        .collect();
    let mut out: Vec<Update> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let is_rule = lines[i].trim().len() >= 10 && lines[i].trim().chars().all(|c| c == '-');
        if !is_rule || i == 0 {
            i += 1;
            continue;
        }
        let cols = header_columns(lines[i - 1]);
        i += 1;
        if cols.len() < 4 {
            continue;
        }
        let bound = |k: usize| cols.get(k).copied();
        while i < lines.len() {
            let l = lines[i];
            if l.trim().is_empty() || l.chars().map(char_width).sum::<usize>() < cols[3] {
                break;
            }
            let name = cut(l, 0, bound(1));
            let id = cut(l, cols[1], bound(2));
            let current = cut(l, cols[2], bound(3));
            let available = cut(l, cols[3], bound(4));
            i += 1;
            if id.is_empty() || available.is_empty() || out.iter().any(|u| u.name == id) {
                continue;
            }
            let truncated = id.ends_with('…') || !super::valid_name("winget", &id);
            out.push(Update {
                id: format!("packages:winget:{id}"),
                note: truncated.then(|| {
                    format!("{name}: winget cut the id short; update it from winget directly")
                }),
                name: id,
                manager: "winget".into(),
                current,
                available,
                admin: false,
                scope: None,
                size: None,
            });
        }
    }
    out
}

pub fn outcome(code: i32) -> (&'static str, String) {
    match code {
        0 => ("succeeded", "done".into()),
        3010 | -1978334967 => ("succeeded", "a restart is needed to finish".into()),
        1641 | -1978334965 => ("succeeded", "the installer started a restart".into()),
        -1978335135 => ("skipped", "already installed".into()),
        -1978335189 => ("skipped", "no applicable update".into()),
        -1978335107 => ("skipped", "installed for another user or scope; winget cannot change it from here".into()),
        -1978335212 => ("failed", "winget did not find this package".into()),
        -1978334975 => ("failed", "the app is open; close it and try again".into()),
        other => ("failed", format!("winget reported 0x{:08X} (see https://learn.microsoft.com/windows/package-manager/winget/returnCodes)", other as u32)),
    }
}

pub fn upgrade_args(id: &str) -> Vec<String> {
    [
        "upgrade",
        "--id",
        id,
        "--exact",
        "--silent",
        "--include-unknown",
        "--accept-package-agreements",
        "--accept-source-agreements",
        "--disable-interactivity",
    ]
    .map(String::from)
    .to_vec()
}

pub fn upgrade_one(id: &str) -> Result<String, String> {
    if !super::valid_name("winget", id) {
        return Err("BAD_NAME".into());
    }
    let args = upgrade_args(id);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let r = query("winget", &args)?;
    let code = r.code.unwrap_or(-1);
    let (kind, detail) = outcome(code);
    let report = serde_json::json!({ "package": id, "action": "upgrade", "exit_code": code, "outcome": kind, "detail": detail });
    if kind == "failed" {
        Err(report.to_string())
    } else {
        Ok(report.to_string())
    }
}

impl PackageBackend for Winget {
    fn id(&self) -> &'static str {
        "winget"
    }

    fn available(&self) -> bool {
        cfg!(windows) && super::find_program("winget").is_some()
    }

    fn list(&self) -> Result<Vec<App>, String> {
        Ok(Vec::new())
    }

    fn remove_preview(&self, names: &[String]) -> Result<RemovePreview, String> {
        Ok(RemovePreview {
            removes: names.to_vec(),
            also_removes: Vec::new(),
            frees_bytes: None,
            exact: false,
        })
    }

    fn remove_argv(&self, names: &[String]) -> Vec<String> {
        let mut v: Vec<String> = [
            "winget",
            "uninstall",
            "--exact",
            "--silent",
            "--disable-interactivity",
            "--id",
        ]
        .map(String::from)
        .to_vec();
        v.extend(names.iter().take(1).cloned());
        v
    }

    fn orphans(&self) -> Result<Vec<App>, String> {
        Ok(Vec::new())
    }

    fn updates(&self) -> Result<Vec<Update>, String> {
        let r = query(
            "winget",
            &[
                "upgrade",
                "--include-unknown",
                "--accept-source-agreements",
                "--disable-interactivity",
            ],
        )?;
        Ok(parse_upgrade(&r.stdout))
    }

    fn upgrade_argv(&self, names: &[String]) -> Vec<String> {
        let mut v = vec!["winget".to_string()];
        v.extend(upgrade_args(
            names.first().map(String::as_str).unwrap_or(""),
        ));
        v
    }
}
