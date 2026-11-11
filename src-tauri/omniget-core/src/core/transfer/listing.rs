use serde::Serialize;
use std::time::SystemTime;

use suppaftp::list::{File, ListParser};

pub const DEFAULT_LIMIT: usize = 200;
pub const MAX_LIMIT: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    File,
    Dir,
    Link,
    Other,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Dir => "directory",
            Self::Link => "link",
            Self::Other => "special file",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    pub size: Option<u64>,
    pub modified: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub path: String,
    pub entries: Vec<Entry>,
    pub total: usize,
    pub truncated: bool,
    pub skipped: usize,
    pub untrusted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Format {
    Mlsd,
    List,
}

pub fn timestamp(time: SystemTime) -> Option<String> {
    if time == SystemTime::UNIX_EPOCH {
        return None;
    }
    let time: chrono::DateTime<chrono::Utc> = time.into();
    Some(time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

fn entry(file: &File) -> Entry {
    let kind = if file.is_directory() {
        Kind::Dir
    } else if file.is_symlink() {
        Kind::Link
    } else {
        Kind::File
    };
    Entry {
        name: file.name().to_string(),
        kind,
        size: (kind == Kind::File).then(|| file.size() as u64),
        modified: timestamp(file.modified()),
    }
}

fn is_self_or_parent(line: &str) -> bool {
    let facts = line.split_once(' ').map_or(line, |(facts, _)| facts);
    let facts = facts.to_ascii_lowercase();
    facts.contains("type=cdir") || facts.contains("type=pdir")
}

pub fn parse_lines(lines: &[String], format: Format) -> (Vec<Entry>, usize) {
    let mut entries = Vec::new();
    let mut skipped = 0;
    for line in lines {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            continue;
        }
        let parsed = match format {
            Format::Mlsd if is_self_or_parent(line) => continue,
            Format::Mlsd => ListParser::parse_mlst(line),
            Format::List => ListParser::parse_posix(line).or_else(|_| ListParser::parse_dos(line)),
        };
        match parsed {
            Ok(file) if matches!(file.name(), "." | "..") => {}
            Ok(file) if file.name().is_empty() => skipped += 1,
            Ok(file) => entries.push(entry(&file)),
            Err(_) => skipped += 1,
        }
    }
    (entries, skipped)
}

pub fn build(path: &str, mut entries: Vec<Entry>, skipped: usize, limit: Option<usize>) -> Listing {
    entries.sort_by_cached_key(|e| (e.kind != Kind::Dir, e.name.to_lowercase(), e.name.clone()));
    let total = entries.len();
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    entries.truncate(limit);
    let reason = match (total, skipped) {
        (0, 0) => Some("empty_directory"),
        (0, _) => Some("no_line_could_be_read"),
        _ => None,
    };
    Listing {
        path: path.to_string(),
        truncated: total > entries.len(),
        entries,
        total,
        skipped,
        untrusted: true,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|l| l.to_string()).collect()
    }

    fn file(name: &str) -> Entry {
        Entry {
            name: name.into(),
            kind: Kind::File,
            size: Some(1),
            modified: None,
        }
    }

    fn dir(name: &str) -> Entry {
        Entry {
            name: name.into(),
            kind: Kind::Dir,
            size: None,
            modified: None,
        }
    }

    #[test]
    fn machine_listings_give_kind_size_and_utc_time() {
        let (entries, skipped) = parse_lines(
            &lines(&[
                "Type=cdir;Modify=20260101120000;Perm=el; .",
                "Type=pdir;Modify=20260101120000;Perm=el; ..",
                "Type=dir;Modify=20260102030405;Perm=el;UNIX.mode=0777; switch",
                "Type=file;Size=1048576;Modify=20261110224500;Perm=r;UNIX.mode=0666; hbmenu.nro",
                "type=file;size=12;modify=20200229000000; name with spaces.txt",
            ]),
            Format::Mlsd,
        );
        assert_eq!(skipped, 0);
        assert_eq!(
            entries,
            vec![
                Entry {
                    name: "switch".into(),
                    kind: Kind::Dir,
                    size: None,
                    modified: Some("2026-01-02T03:04:05Z".into()),
                },
                Entry {
                    name: "hbmenu.nro".into(),
                    kind: Kind::File,
                    size: Some(1_048_576),
                    modified: Some("2026-11-10T22:45:00Z".into()),
                },
                Entry {
                    name: "name with spaces.txt".into(),
                    kind: Kind::File,
                    size: Some(12),
                    modified: Some("2020-02-29T00:00:00Z".into()),
                },
            ]
        );
    }

    #[test]
    fn unix_and_dos_listings_are_both_read() {
        let (entries, skipped) = parse_lines(
            &lines(&[
                "total 12",
                "drwxrwxrwx 1 3DS 3DS 0 Jan 01 2026 atmosphere",
                "-rw-rw-rw- 1 3DS 3DS 524288 Nov 05 2025 payload.bin",
                "lrwxrwxrwx 1 root root 9 Nov 05 2025 current -> /releases",
                "04-08-14  03:09PM       <DIR>          docs",
                "04-08-14  03:09PM                  403 readme.txt",
                "this is not a listing line",
            ]),
            Format::List,
        );
        assert_eq!(skipped, 2);
        let seen: Vec<(&str, Kind, Option<u64>)> = entries
            .iter()
            .map(|e| (e.name.as_str(), e.kind, e.size))
            .collect();
        assert_eq!(
            seen,
            vec![
                ("atmosphere", Kind::Dir, None),
                ("payload.bin", Kind::File, Some(524_288)),
                ("current", Kind::Link, None),
                ("docs", Kind::Dir, None),
                ("readme.txt", Kind::File, Some(403)),
            ]
        );
        assert_eq!(entries[1].modified.as_deref(), Some("2025-11-05T00:00:00Z"));
    }

    #[test]
    fn dot_entries_are_never_listed() {
        let (entries, skipped) = parse_lines(
            &lines(&[
                "drwxr-xr-x 2 u g 0 Jan 01 2026 .",
                "drwxr-xr-x 2 u g 0 Jan 01 2026 ..",
                "-rw-r--r-- 1 u g 3 Jan 01 2026 .hidden",
            ]),
            Format::List,
        );
        assert_eq!(skipped, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, ".hidden");
    }

    #[test]
    fn folders_come_first_then_names_without_regard_to_case() {
        let listing = build(
            "/",
            vec![file("b.txt"), dir("Zeta"), file("A.txt"), dir("alpha")],
            0,
            None,
        );
        let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "Zeta", "A.txt", "b.txt"]);
        assert_eq!(listing.total, 4);
        assert!(!listing.truncated);
        assert!(listing.untrusted);
        assert_eq!(listing.reason, None);
    }

    #[test]
    fn a_long_folder_is_capped_and_says_how_many_exist() {
        let many: Vec<Entry> = (0..250).map(|n| file(&format!("f{n:03}"))).collect();
        let listing = build("/big", many.clone(), 0, None);
        assert_eq!(listing.entries.len(), DEFAULT_LIMIT);
        assert_eq!(listing.total, 250);
        assert!(listing.truncated);
        let listing = build("/big", many.clone(), 0, Some(5));
        assert_eq!(listing.entries.len(), 5);
        assert_eq!(listing.entries[4].name, "f004");
        let listing = build("/big", many, 0, Some(50_000));
        assert_eq!(listing.entries.len(), 250);
        assert!(!listing.truncated);
    }

    #[test]
    fn an_empty_folder_explains_itself() {
        let listing = build("/empty", vec![], 0, None);
        assert_eq!(listing.reason, Some("empty_directory"));
        let listing = build("/odd", vec![], 3, None);
        assert_eq!(listing.reason, Some("no_line_could_be_read"));
    }

    #[test]
    fn times_are_rfc3339_in_utc_and_the_epoch_means_unknown() {
        let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        assert_eq!(timestamp(time).as_deref(), Some("2023-11-14T22:13:20Z"));
        assert_eq!(timestamp(SystemTime::UNIX_EPOCH), None);
    }
}
