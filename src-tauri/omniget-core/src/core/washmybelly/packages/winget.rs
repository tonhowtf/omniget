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

fn clean_line(raw: &str) -> String {
    let raw = raw.trim_end_matches('\r');
    let last = raw.rsplit('\r').next().unwrap_or(raw);
    let mut out = String::with_capacity(last.len());
    let mut chars = last.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    for n in chars.by_ref() {
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                } else {
                    chars.next();
                }
            }
            '\u{feff}' | '\u{8}' => {}
            c => out.push(c),
        }
    }
    out
}

fn looks_like_version(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_digit()) && !s.ends_with('.')
}

fn row_cells(line: &str, cols: &[usize]) -> Option<(String, String, String, String)> {
    let at = |k: usize| cols.get(k).copied();
    let name = cut(line, 0, at(1));
    let id = cut(line, cols[1], at(2));
    let current = cut(line, cols[2], at(3));
    let available = cut(line, cols[3], at(4));
    if !id.is_empty() && !id.contains(char::is_whitespace) && !available.is_empty() {
        return Some((name, id, current, available));
    }
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let tail = if cols.len() > 4 { 4 } else { 3 };
    if tokens.len() <= tail {
        return None;
    }
    let right = &tokens[tokens.len() - tail..];
    let (id, current, available) = (right[0], right[1], right[2]);
    if !super::valid_name("winget", id) || !looks_like_version(available) {
        return None;
    }
    Some((
        tokens[..tokens.len() - tail].join(" "),
        id.to_string(),
        current.to_string(),
        available.to_string(),
    ))
}

pub fn parse_upgrade(text: &str) -> Vec<Update> {
    scan_upgrade(text).0
}

fn scan_upgrade(text: &str) -> (Vec<Update>, bool) {
    let cleaned: Vec<String> = text.lines().map(clean_line).collect();
    let lines: Vec<&str> = cleaned.iter().map(String::as_str).collect();
    let mut out: Vec<Update> = Vec::new();
    let mut saw_table = false;
    let mut i = 0;
    while i < lines.len() {
        let is_rule = lines[i].trim().len() >= 10 && lines[i].trim().chars().all(|c| c == '-');
        if !is_rule || i == 0 {
            i += 1;
            continue;
        }
        let cols = header_columns(lines[i - 1]);
        i += 1;
        saw_table = true;
        if cols.len() < 4 {
            continue;
        }
        while i < lines.len() {
            let l = lines[i];
            if l.trim().is_empty() {
                break;
            }
            let Some((name, id, current, available)) = row_cells(l, &cols) else {
                if l.chars().map(char_width).sum::<usize>() < cols[3] {
                    break;
                }
                i += 1;
                continue;
            };
            i += 1;
            if out.iter().any(|u| u.name == id) {
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
    (out, saw_table)
}

const NO_UPGRADES: &[i32] = &[-1978335189, -1978335212];

pub fn read_upgrade(code: Option<i32>, stdout: &str, stderr: &str) -> Result<Vec<Update>, String> {
    let (updates, saw_table) = scan_upgrade(stdout);
    if !updates.is_empty() {
        return Ok(updates);
    }
    let shown = |code: Option<i32>| match code {
        Some(c) => format!("exit code 0x{:08X}", c as u32),
        None => "stopped without an exit code".to_string(),
    };
    if saw_table {
        return Err(format!(
            "winget printed a table OmniGet could not read ({}). Run winget upgrade in a terminal to see the list",
            shown(code)
        ));
    }
    if code == Some(0) || code.is_some_and(|c| NO_UPGRADES.contains(&c)) {
        return Ok(Vec::new());
    }
    let detail = super::tail(&format!("{stdout}\n{stderr}"), 2);
    Err(if detail.is_empty() {
        format!("winget failed ({})", shown(code))
    } else {
        format!("winget failed ({}): {detail}", shown(code))
    })
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
        read_upgrade(r.code, &r.stdout, &r.stderr)
    }

    fn upgrade_argv(&self, names: &[String]) -> Vec<String> {
        let mut v = vec!["winget".to_string()];
        v.extend(upgrade_args(
            names.first().map(String::as_str).unwrap_or(""),
        ));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIN_26H2: &str = "\r   - \r   \\ \r                                                                 \rName                                       Id                                  Version       Available     Source\r\n--------------------------------------------------------------------------------------------------------------------\r\nPython 3.13.16 (64-bit)                    Python.Python.3.13                  3.13.16       3.13.17       winget\r\nSynology Drive Client                      Synology.DriveClient                3.5.2.16111   4.0.0.17889   winget\r\nTailscale                                  Tailscale.Tailscale                 1.88.3        1.90.1        winget\r\nVisual Studio Community 2022               Microsoft.VisualStudio.2022.Commun\u{2026} 17.14.8       17.14.12      winget\r\n4 upgrades available.\r\n\r\n1 package(s) have version numbers that cannot be determined. Use --include-unknown to see all results.\r\n";

    #[test]
    fn upgrade_table_from_windows_26h2_lists_the_four_updates() {
        let u = read_upgrade(Some(0), WIN_26H2, "").unwrap();
        assert_eq!(u.len(), 4);
        assert_eq!(u[0].name, "Python.Python.3.13");
        assert_eq!(u[0].current, "3.13.16");
        assert_eq!(u[0].available, "3.13.17");
        assert_eq!(u[1].name, "Synology.DriveClient");
        assert_eq!(u[2].name, "Tailscale.Tailscale");
        assert_eq!(u[3].available, "17.14.12");
        assert!(u[3].note.is_some());
    }

    #[test]
    fn long_name_that_fills_its_column_keeps_id_and_versions() {
        let t = "Name                 Id                   Version Available Source\n------------------------------------------------------------------\nA name wider than the column here  Vendor.App  1.2.3 1.2.4 winget\n";
        let u = parse_upgrade(t);
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].name, "Vendor.App");
        assert_eq!(u[0].current, "1.2.3");
        assert_eq!(u[0].available, "1.2.4");
    }

    #[test]
    fn doubled_carriage_returns_and_escape_codes_do_not_hide_the_table() {
        let t = "\u{feff}\u{1b}[2K\u{1b}[1GName   Id          Version Available Source\r\r\n------------------------------------------\r\r\nPython Python.3.13 3.13.16 3.13.17   winget\r\r\n";
        let u = parse_upgrade(t);
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].name, "Python.3.13");
    }

    #[test]
    fn footer_lines_are_never_read_as_updates() {
        let names: Vec<String> = parse_upgrade(WIN_26H2)
            .into_iter()
            .map(|u| u.name)
            .collect();
        assert!(!names.iter().any(|n| n.contains("package") || n == "all"));
    }

    #[test]
    fn winget_failure_is_an_error_and_not_an_empty_list() {
        let e = read_upgrade(
            Some(-1978335157),
            "Failed when searching source: winget\r\n",
            "",
        )
        .unwrap_err();
        assert!(e.contains("0x8A15"), "{e}");
        assert!(e.contains("Failed when searching source"), "{e}");
    }

    #[test]
    fn table_that_cannot_be_read_is_an_error_and_not_an_empty_list() {
        let t = "Nome do pacote\n---------------------\nqualquer coisa\n";
        assert!(read_upgrade(Some(0), t, "").is_err());
    }

    #[test]
    fn nothing_to_upgrade_is_an_empty_list() {
        let none = "No installed package found matching input criteria.\r\n";
        assert!(read_upgrade(Some(0), none, "").unwrap().is_empty());
        assert!(read_upgrade(Some(-1978335189), none, "")
            .unwrap()
            .is_empty());
    }
}
