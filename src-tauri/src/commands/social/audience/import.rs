use std::collections::HashSet;
use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;

use serde_json::{json, Value};

use super::db::Member;
use super::fail;

const DIR: &str = "connections/followers_and_following";
const FILE_CAP: u64 = 64 * 1024 * 1024;
const MAX_FILES: usize = 200;
const WHERE: &str = "In Photo Feed open Accounts Center, Your information and permissions, Download your information, choose Connections with Followers and following, then pass the ZIP it sends or its unpacked folder as path.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Followers,
    Following,
}

impl Kind {
    pub fn id(self) -> &'static str {
        match self {
            Kind::Followers => "followers",
            Kind::Following => "following",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub handle: String,
    pub at: Option<i64>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Export {
    pub files: Vec<(String, Kind, usize)>,
    pub followers: Option<Vec<Entry>>,
    pub following: Option<Vec<Entry>>,
}

impl Export {
    pub fn list(&self, kind: Kind) -> Option<&[Entry]> {
        match kind {
            Kind::Followers => self.followers.as_deref(),
            Kind::Following => self.following.as_deref(),
        }
    }
}

pub fn member(e: &Entry) -> Member {
    Member {
        id: format!("@{}", e.handle),
        handle: e.handle.clone(),
        followed_at: e.at,
        ..Default::default()
    }
}

fn regex(cell: &'static OnceLock<regex::Regex>, pattern: &str) -> &'static regex::Regex {
    cell.get_or_init(|| regex::Regex::new(pattern).expect("static regex"))
}

pub fn kind_of(name: &str) -> Option<Kind> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let name = name.replace('\\', "/");
    let (dir, base) = name.rsplit_once('/')?;
    if dir != DIR && !dir.ends_with(&format!("/{DIR}")) {
        return None;
    }
    let found = regex(&RE, r"^(following|followers(?:_\d+)?)\.(?:html|json)$").captures(base)?;
    Some(if &found[1] == "following" {
        Kind::Following
    } else {
        Kind::Followers
    })
}

fn handle(raw: &str) -> Option<String> {
    let h = raw.trim().trim_start_matches('@').to_lowercase();
    let valid = !h.is_empty()
        && h.len() <= 30
        && h.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.');
    valid.then_some(h)
}

fn handle_of_link(href: &str) -> Option<String> {
    let rest = href.split_once("instagram.com/")?.1;
    let rest = rest.strip_prefix("_u/").unwrap_or(rest);
    handle(rest.split(['/', '?', '#']).next()?)
}

fn date(text: &str) -> Option<i64> {
    let text = text.trim();
    ["%b %d, %Y %I:%M %p", "%b %d, %Y, %I:%M %p"]
        .iter()
        .find_map(|f| chrono::NaiveDateTime::parse_from_str(text, f).ok())
        .map(|d| d.and_utc().timestamp())
}

pub fn parse_html(text: &str) -> Vec<Entry> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    regex(
        &RE,
        r#"<a[^>]*href="(https?://(?:www\.)?instagram\.com/[^"]+)"[^>]*>[^<]*</a>(?:\s*</div>\s*<div>([^<]*)</div>)?"#,
    )
    .captures_iter(text)
    .filter_map(|c| {
        Some(Entry {
            handle: handle_of_link(&c[1])?,
            at: c.get(2).and_then(|d| date(d.as_str())),
        })
    })
    .collect()
}

pub fn parse_json(text: &str) -> Option<Vec<Entry>> {
    let v: Value = serde_json::from_str(text).ok()?;
    let list = v
        .as_array()
        .or_else(|| v["relationships_following"].as_array())
        .or_else(|| v["relationships_followers"].as_array())?;
    Some(
        list.iter()
            .filter_map(|item| {
                let data = &item["string_list_data"][0];
                let name = data["value"]
                    .as_str()
                    .and_then(handle)
                    .or_else(|| data["href"].as_str().and_then(handle_of_link))
                    .or_else(|| item["title"].as_str().and_then(handle))?;
                Some(Entry {
                    handle: name,
                    at: data["timestamp"].as_i64().filter(|t| *t > 0),
                })
            })
            .collect(),
    )
}

pub fn collect(files: Vec<(String, String)>) -> Result<Export, String> {
    let mut out = Export::default();
    let mut seen: [HashSet<String>; 2] = Default::default();
    let mut files = files;
    files.sort();
    for (name, text) in files {
        let Some(kind) = kind_of(&name) else { continue };
        let entries = if name.ends_with(".json") {
            parse_json(&text).ok_or_else(|| {
                fail(
                    "EXPORT_INVALID",
                    &format!("{name} does not have the shape of a Photo Feed export list"),
                    false,
                    WHERE,
                )
            })?
        } else {
            parse_html(&text)
        };
        let (list, seen) = match kind {
            Kind::Followers => (&mut out.followers, &mut seen[0]),
            Kind::Following => (&mut out.following, &mut seen[1]),
        };
        let list = list.get_or_insert_with(Vec::new);
        let before = list.len();
        list.extend(
            entries
                .into_iter()
                .filter(|e| seen.insert(e.handle.clone())),
        );
        out.files.push((name, kind, list.len() - before));
    }
    if out.files.is_empty() {
        return Err(fail(
            "EXPORT_INVALID",
            &format!("no followers or following file was found under {DIR}"),
            false,
            WHERE,
        ));
    }
    Ok(out)
}

fn too_large(name: &str) -> String {
    fail(
        "EXPORT_TOO_LARGE",
        &format!("{name} is larger than 64 MB"),
        false,
        "Pass the export as Photo Feed sent it; a follower list file is far smaller than this.",
    )
}

fn unreadable(what: &str, e: impl std::fmt::Display) -> String {
    fail(
        "EXPORT_INVALID",
        &format!("could not read {what}: {e}"),
        false,
        WHERE,
    )
}

fn capped(mut from: impl Read, name: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    from.by_ref()
        .take(FILE_CAP + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| unreadable(name, e))?;
    if bytes.len() as u64 > FILE_CAP {
        return Err(too_large(name));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_folder(path: &Path) -> Result<Vec<(String, String)>, String> {
    for base in [path.join(DIR), path.to_path_buf()] {
        let Ok(entries) = std::fs::read_dir(&base) else {
            continue;
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let name = format!("{DIR}/{}", entry.file_name().to_string_lossy());
            if kind_of(&name).is_none() || !entry.path().is_file() || out.len() >= MAX_FILES {
                continue;
            }
            let file = std::fs::File::open(entry.path()).map_err(|e| unreadable(&name, e))?;
            out.push((name.clone(), capped(file, &name)?));
        }
        if !out.is_empty() {
            return Ok(out);
        }
    }
    Ok(Vec::new())
}

fn read_zip(path: &Path) -> Result<Vec<(String, String)>, String> {
    let file = std::fs::File::open(path).map_err(|e| unreadable("the file", e))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| unreadable("the ZIP", e))?;
    let names: Vec<String> = zip
        .file_names()
        .filter_map(Result::ok)
        .filter(|n| kind_of(n).is_some())
        .take(MAX_FILES)
        .map(String::from)
        .collect();
    let mut out = Vec::new();
    for name in names {
        let entry = zip.by_name(&name).map_err(|e| unreadable(&name, e))?;
        if entry.size() > FILE_CAP {
            return Err(too_large(&name));
        }
        let text = capped(entry, &name)?;
        out.push((name, text));
    }
    Ok(out)
}

pub fn read(path: &Path) -> Result<Export, String> {
    let files = if path.is_dir() {
        read_folder(path)?
    } else if path.is_file() {
        read_zip(path)?
    } else {
        return Err(fail(
            "EXPORT_NOT_FOUND",
            "nothing exists at that path",
            false,
            "Pass the absolute path of the export ZIP or of its unpacked folder.",
        ));
    };
    collect(files)
}

pub fn account_from_name(path: &Path) -> Option<String> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let name = path.file_name()?.to_str()?.to_lowercase();
    let found = regex(&RE, r"^instagram-([a-z0-9._]{1,30})-\d{4}-\d{2}-\d{2}").captures(&name)?;
    Some(found[1].to_string())
}

pub fn files_json(export: &Export) -> Vec<Value> {
    export
        .files
        .iter()
        .map(|(file, kind, n)| json!({ "file": file, "kind": kind.id(), "accounts": n }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOLLOWING_HTML: &str = r#"<html><head><base href="../../" /><style>a{color:#00f}</style></head><body><div><main class="_a706" role="main"><div class="pam _3-95 _2ph- _a6-g uiBoxWhite noborder"><h2 class="_3-95 _2pim _a6-h _a6-i">ana.dev</h2><div class="_a6-p"><div><div><a target="_blank" href="https://www.instagram.com/_u/ana.dev">https://www.instagram.com/_u/ana.dev</a></div><div>Nov 11, 2026 7:47 am</div></div></div></div><div class="pam _3-95 _2ph- _a6-g uiBoxWhite noborder"><h2 class="_3-95 _2pim _a6-h _a6-i">Bia_01</h2><div class="_a6-p"><div><div><a target="_blank" href="https://www.instagram.com/_u/Bia_01">https://www.instagram.com/_u/Bia_01</a></div><div>Dec 21, 2024 4:33 pm</div></div></div></div></main></div></body></html>"#;
    const FOLLOWERS_HTML: &str = r#"<html><body><main class="_a706" role="main"><div class="pam _3-95 _2ph- _a6-g uiBoxWhite noborder"><div class="_a6-p"><div><div><a target="_blank" href="https://www.instagram.com/ana.dev">ana.dev</a></div><div>Nov 09, 2026 12:39 pm</div></div></div></div><div class="pam _3-95 _2ph- _a6-g uiBoxWhite noborder"><div class="_a6-p"><div><div><a target="_blank" href="https://www.instagram.com/cris">cris</a></div><div>14 de nov. de 2025 09:18</div></div></div></div></main></body></html>"#;

    fn names(list: &[Entry]) -> Vec<&str> {
        list.iter().map(|e| e.handle.as_str()).collect()
    }

    #[test]
    fn reads_the_following_and_followers_pages_of_the_html_export() {
        let following = parse_html(FOLLOWING_HTML);
        assert_eq!(names(&following), vec!["ana.dev", "bia_01"]);
        assert_eq!(following[0].at, Some(1_794_383_220));
        assert_eq!(following[1].at, Some(1_734_798_780));
        let followers = parse_html(FOLLOWERS_HTML);
        assert_eq!(names(&followers), vec!["ana.dev", "cris"]);
        assert_eq!(followers[0].at, Some(1_794_227_940));
        assert_eq!(followers[1].at, None);
        assert!(
            parse_html("<html><body><a href=\"https://example.com/x\">x</a></body></html>")
                .is_empty()
        );
    }

    #[test]
    fn reads_the_json_export_in_its_old_and_new_shapes() {
        let following = r#"{"relationships_following":[
            {"title":"","media_list_data":[],"string_list_data":[{"href":"https://www.instagram.com/ana.dev","value":"ana.dev","timestamp":1700000000}]},
            {"title":"bia_01","string_list_data":[{"href":"https://www.instagram.com/_u/bia_01","timestamp":1700000100}]},
            {"title":"Cris","string_list_data":[{"timestamp":0}]},
            {"title":"not a handle!","string_list_data":[]}]}"#;
        let list = parse_json(following).unwrap();
        assert_eq!(names(&list), vec!["ana.dev", "bia_01", "cris"]);
        assert_eq!(list[0].at, Some(1_700_000_000));
        assert_eq!(list[2].at, None);
        let followers = r#"[{"title":"","media_list_data":[],"string_list_data":[{"href":"https://www.instagram.com/dani","value":"dani","timestamp":1700000200}]}]"#;
        assert_eq!(names(&parse_json(followers).unwrap()), vec!["dani"]);
        assert_eq!(parse_json(r#"{"something_else":[]}"#), None);
        assert_eq!(parse_json("<html>"), None);
    }

    #[test]
    fn only_the_two_lists_inside_the_connections_folder_are_taken() {
        let at = |f: &str| kind_of(&format!("{DIR}/{f}"));
        assert_eq!(at("following.html"), Some(Kind::Following));
        assert_eq!(at("following.json"), Some(Kind::Following));
        assert_eq!(at("followers_1.html"), Some(Kind::Followers));
        assert_eq!(at("followers_12.json"), Some(Kind::Followers));
        assert_eq!(at("followers.json"), Some(Kind::Followers));
        assert_eq!(at("pending_follow_requests.html"), None);
        assert_eq!(at("recently_unfollowed_profiles.html"), None);
        assert_eq!(at("following.html.exe"), None);
        assert_eq!(kind_of("following.html"), None);
        assert_eq!(kind_of("media/following.html"), None);
        assert_eq!(kind_of("../../etc/following.html"), None);
        assert_eq!(
            kind_of("instagram-ana-2026-11-11/connections/followers_and_following/following.html"),
            Some(Kind::Following)
        );
        assert_eq!(
            kind_of("xconnections/followers_and_following/following.html"),
            None
        );
    }

    #[test]
    fn numbered_files_merge_without_repeating_an_account() {
        let f = |n: &str| format!("{DIR}/{n}");
        let second = FOLLOWERS_HTML.replace("ana.dev", "dani");
        let export = collect(vec![
            (f("followers_2.html"), second),
            (f("following.html"), FOLLOWING_HTML.into()),
            (f("followers_1.html"), FOLLOWERS_HTML.into()),
            ("media/photo.html".into(), FOLLOWERS_HTML.into()),
        ])
        .unwrap();
        assert_eq!(
            names(export.followers.as_deref().unwrap()),
            vec!["ana.dev", "cris", "dani"]
        );
        assert_eq!(
            names(export.list(Kind::Following).unwrap()),
            vec!["ana.dev", "bia_01"]
        );
        assert_eq!(
            files_json(&export),
            vec![
                json!({ "file": f("followers_1.html"), "kind": "followers", "accounts": 2 }),
                json!({ "file": f("followers_2.html"), "kind": "followers", "accounts": 1 }),
                json!({ "file": f("following.html"), "kind": "following", "accounts": 2 }),
            ]
        );
        let only = collect(vec![(
            f("following.json"),
            r#"{"relationships_following":[]}"#.into(),
        )])
        .unwrap();
        assert_eq!(only.followers, None);
        assert_eq!(only.following, Some(Vec::new()));
        let m = member(&export.following.as_ref().unwrap()[0]);
        assert_eq!(
            (m.id.as_str(), m.followed_at),
            ("@ana.dev", Some(1_794_383_220))
        );
    }

    #[test]
    fn an_export_without_the_lists_says_where_to_request_one() {
        let err = collect(vec![("media/a.html".into(), String::new())]).unwrap_err();
        assert!(err.starts_with("EXPORT_INVALID: {"));
        assert!(err.contains("Accounts Center"));
        let bad = collect(vec![(format!("{DIR}/following.json"), "{}".into())]).unwrap_err();
        assert!(bad.starts_with("EXPORT_INVALID: {"));
        let missing = read(Path::new("/nonexistent/omniget-export.zip")).unwrap_err();
        assert!(missing.starts_with("EXPORT_NOT_FOUND: {"));
    }

    #[test]
    fn reads_a_zip_without_unpacking_and_a_folder() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("omniget-ig-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lists = dir.join("unpacked").join(DIR);
        std::fs::create_dir_all(&lists).unwrap();
        std::fs::write(lists.join("following.html"), FOLLOWING_HTML).unwrap();
        std::fs::write(lists.join("followers_1.html"), FOLLOWERS_HTML).unwrap();
        std::fs::write(lists.join("pending_follow_requests.html"), FOLLOWERS_HTML).unwrap();
        let folder = read(&dir.join("unpacked")).unwrap();
        assert_eq!(folder.files.len(), 2);
        assert_eq!(folder.following.as_ref().unwrap().len(), 2);
        assert_eq!(read(&lists).unwrap(), folder);

        let zip_path = dir.join("instagram-ana.dev-2026-11-11-AbCd.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for (name, body) in [
            (format!("{DIR}/following.html"), FOLLOWING_HTML),
            (format!("{DIR}/followers_1.html"), FOLLOWERS_HTML),
            ("../outside/following.html".to_string(), FOLLOWERS_HTML),
            ("media/posts/a.html".to_string(), "x"),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        assert_eq!(read(&zip_path).unwrap(), folder);
        assert_eq!(account_from_name(&zip_path).as_deref(), Some("ana.dev"));
        assert_eq!(account_from_name(Path::new("/tmp/export")), None);
        assert!(!dir.join("outside").exists());
        std::fs::write(dir.join("not-a-zip.zip"), "hello").unwrap();
        assert!(read(&dir.join("not-a-zip.zip"))
            .unwrap_err()
            .starts_with("EXPORT_INVALID: {"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "reads a real export: OMNIGET_IG_EXPORT=<zip or folder> [OMNIGET_IG_EXPORT_COUNTS=following,followers]"]
    fn a_real_export_gives_its_counts() {
        let path = std::env::var("OMNIGET_IG_EXPORT").expect("OMNIGET_IG_EXPORT");
        let export = read(Path::new(&path)).unwrap();
        let following = export.following.as_ref().map_or(0, Vec::len);
        let followers = export.followers.as_ref().map_or(0, Vec::len);
        let dated = |l: &Option<Vec<Entry>>| l.iter().flatten().filter(|e| e.at.is_some()).count();
        println!(
            "files {} following {following} (dated {}) followers {followers} (dated {})",
            export.files.len(),
            dated(&export.following),
            dated(&export.followers)
        );
        if let Ok(want) = std::env::var("OMNIGET_IG_EXPORT_COUNTS") {
            assert_eq!(want, format!("{following},{followers}"));
        }
    }
}
