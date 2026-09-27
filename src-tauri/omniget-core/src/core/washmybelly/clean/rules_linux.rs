//! Linux cleanup rules of our own (GPL-3.0). BleachBit covers most app
//! caches; these add what the studies found missing or wrong elsewhere:
//!
//! - `~/.cache` one item per app, never the folder at once (Stacer's bug);
//! - the Trash of every mounted volume (`.Trash-<uid>`, `.Trash/<uid>`);
//! - package caches through the package manager itself, as administrator;
//! - `journalctl --vacuum-size`, core dumps, unused Flatpak runtimes;
//! - old snap revisions read from the snapd REST API, never from the text
//!   of `snap list`.

use serde_json::{json, Value};

use super::super::plan::Op;
use super::super::{Os, Risk};
use super::catalog::b;
use super::engine::{Env, Found};
use super::rule::Rule;

const L: &[Os] = &[Os::Linux];

/// Journal size kept by the vacuum.
pub const JOURNAL_KEEP_MB: u64 = 200;

pub fn rules() -> Vec<Rule> {
    vec![
        b("linux.trash", L, "trash", "Trash")
            .t(&[
                "$XDG_DATA_HOME/Trash/files",
                "$XDG_DATA_HOME/Trash/info",
                "{mount}/.Trash-$UID",
                "{mount}/.Trash/$UID",
            ])
            .risk(Risk::Extreme)
            .desc("Empties the Trash of every disk. Only from the OmniGet screen.")
            .done(),
        b("linux.apt_cache", L, "package_cache", "APT package cache")
            .cmd("apt-get", &["clean"], &["/var/cache/apt/archives"])
            .admin()
            .desc("Downloaded .deb files, removed by apt-get clean.")
            .done(),
        b("linux.dnf_cache", L, "package_cache", "DNF package cache")
            .cmd(
                "dnf",
                &["clean", "packages"],
                &["/var/cache/dnf", "/var/cache/libdnf5"],
            )
            .admin()
            .done(),
        b("linux.paccache", L, "package_cache", "Pacman package cache")
            .cmd("paccache", &["-rk2"], &["/var/cache/pacman/pkg"])
            .admin()
            .desc("Keeps the two newest versions of each package (paccache -rk2).")
            .done(),
        b(
            "linux.pacman_cache",
            L,
            "package_cache",
            "Pacman package cache",
        )
        .cmd(
            "pacman",
            &["-Sc", "--noconfirm"],
            &["/var/cache/pacman/pkg"],
        )
        .admin()
        .desc("Removes packages that are no longer installed (pacman -Sc).")
        .done(),
        b(
            "linux.zypper_cache",
            L,
            "package_cache",
            "Zypper package cache",
        )
        .cmd(
            "zypper",
            &["--non-interactive", "clean"],
            &["/var/cache/zypp/packages"],
        )
        .admin()
        .done(),
        b("linux.journal", L, "logs", "System journal")
            .cmd("journalctl", &["--vacuum-size=200M"], &[])
            .admin()
            .desc("Keeps the newest 200 MB of system logs.")
            .done(),
        b("linux.coredumps", L, "logs", "Crash dumps")
            .t(&["/var/lib/systemd/coredump"])
            .admin()
            .desc("Memory dumps systemd saved when programs crashed.")
            .done(),
        b(
            "linux.flatpak_unused",
            L,
            "package_cache",
            "Unused Flatpak runtimes",
        )
        .cmd("flatpak", &["uninstall", "--unused", "-y", "--user"], &[])
        .needs_path("$XDG_DATA_HOME/flatpak/runtime")
        .desc("Runtimes no installed Flatpak app uses any more (your user only).")
        .done(),
        b(
            "linux.snap_revisions",
            L,
            "package_cache",
            "Old snap revisions",
        )
        .cmd("snap", &["remove"], &[])
        .needs_path("/run/snapd.socket")
        .admin()
        .desc("Disabled revisions snapd keeps after an update.")
        .done(),
        b("linux.app_caches", L, "apps", "App caches")
            .t(&["$XDG_CACHE_HOME/*"])
            .delete()
            .exclude(&["keyring*", "ibus*", "dconf", "flatpak", "thumbnails", ".*"])
            .desc("One item per app. Apps rebuild their cache the next time they need it.")
            .done(),
    ]
}

// ── Probes ───────────────────────────────────────────────────────────

/// `journalctl --disk-usage`: bytes above the size the vacuum keeps.
pub fn probe_journal(_rule: &Rule, env: &Env) -> Vec<Found> {
    let Some(out) = env.output("journalctl", &["--disk-usage"]) else {
        return Vec::new();
    };
    let Some(used) = parse_journal_usage(&out) else {
        return Vec::new();
    };
    let keep = JOURNAL_KEEP_MB * 1024 * 1024;
    if used <= keep {
        return Vec::new();
    }
    vec![Found {
        path: "journal:/var/log/journal".into(),
        label: None,
        bytes: used - keep,
        files: 0,
        op: Op::Module {
            module: "clean".into(),
            action: "journal_vacuum".into(),
            args: json!({ "max_mb": JOURNAL_KEEP_MB }),
        },
        admin: true,
    }]
}

/// "Archived and active journals take up 1.2G in the file system."
pub fn parse_journal_usage(text: &str) -> Option<u64> {
    let i = text.find("take up ")? + "take up ".len();
    let word = text[i..].split_whitespace().next()?;
    parse_size(word)
}

/// `1.2G`, `512.0M`, `8K`, `100B` (binary units, as journalctl prints).
pub fn parse_size(word: &str) -> Option<u64> {
    let w = word.trim().trim_end_matches('B');
    let (num, mult) = match w.chars().last()? {
        'K' | 'k' => (&w[..w.len() - 1], 1u64 << 10),
        'M' => (&w[..w.len() - 1], 1 << 20),
        'G' => (&w[..w.len() - 1], 1 << 30),
        'T' => (&w[..w.len() - 1], 1 << 40),
        _ => (w, 1),
    };
    let n: f64 = num.parse().ok()?;
    Some((n * mult as f64) as u64)
}

/// Old snap revisions from `GET /v2/snaps?select=all` on the snapd socket.
pub fn probe_snaps(rule: &Rule, env: &Env) -> Vec<Found> {
    let Some(body) = env.snapd_get("/v2/snaps?select=all") else {
        return Vec::new();
    };
    parse_snap_revisions(&body)
        .into_iter()
        .map(|s| Found {
            path: format!("snap:{}:{}", s.name, s.revision),
            label: Some(format!(
                "{}: {} (revision {})",
                rule.label, s.name, s.revision
            )),
            bytes: s.bytes,
            files: 1,
            op: Op::Module {
                module: "clean".into(),
                action: "snap_revision".into(),
                args: json!({ "name": s.name, "revision": s.revision }),
            },
            admin: true,
        })
        .collect()
}

#[derive(Debug, PartialEq)]
pub struct SnapRevision {
    pub name: String,
    pub revision: String,
    pub bytes: u64,
}

/// Snap names are `[a-z0-9-]`, revisions digits (or `x<digits>` for local installs).
pub fn valid_snap(name: &str, revision: &str) -> bool {
    let digits = revision.strip_prefix('x').unwrap_or(revision);
    !name.is_empty()
        && name.len() <= 40
        && name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        && !digits.is_empty()
        && digits.bytes().all(|c| c.is_ascii_digit())
}

pub fn parse_snap_revisions(body: &str) -> Vec<SnapRevision> {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for s in v["result"].as_array().into_iter().flatten() {
        let name = s["name"].as_str().unwrap_or("");
        let revision = match &s["revision"] {
            Value::String(r) => r.clone(),
            Value::Number(n) => n.to_string(),
            _ => continue,
        };
        // "active" is the revision in use; anything else installed is a
        // disabled older revision snapd keeps for rollback.
        if s["status"].as_str() == Some("active") || !valid_snap(name, &revision) {
            continue;
        }
        out.push(SnapRevision {
            name: name.to_string(),
            revision,
            bytes: s["installed-size"].as_u64().unwrap_or(0),
        });
    }
    out
}

/// Minimal HTTP/1.1 GET over the snapd unix socket (no extra crate).
#[cfg(target_os = "linux")]
pub fn snapd_get(path: &str) -> Option<String> {
    use std::io::{Read, Write};
    let mut s = std::os::unix::net::UnixStream::connect("/run/snapd.socket").ok()?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok()?;
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).ok()?;
    http_body(&String::from_utf8_lossy(&raw))
}

#[cfg(not(target_os = "linux"))]
pub fn snapd_get(_path: &str) -> Option<String> {
    None
}

/// Body of a raw HTTP/1.1 response, de-chunked when needed.
pub fn http_body(raw: &str) -> Option<String> {
    let (head, body) = raw.split_once("\r\n\r\n")?;
    if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
        return None;
    }
    if !head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        return Some(body.to_string());
    }
    let mut out = String::new();
    let mut rest = body;
    loop {
        let (size, tail) = rest.split_once("\r\n")?;
        let n = usize::from_str_radix(size.trim(), 16).ok()?;
        if n == 0 {
            break;
        }
        out.push_str(tail.get(..n)?);
        rest = tail.get(n..)?.strip_prefix("\r\n")?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/washmybelly_fixtures/clean")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn journal_usage_is_read_in_binary_units() {
        assert_eq!(
            parse_journal_usage("Archived and active journals take up 1.5G in the file system."),
            Some(1_610_612_736)
        );
        assert_eq!(
            parse_journal_usage("Journals take up 96.0M on disk."),
            Some(100_663_296)
        );
        assert_eq!(parse_journal_usage("nothing"), None);
    }

    #[test]
    fn only_disabled_snap_revisions_are_offered() {
        let revs = parse_snap_revisions(&fixture("snapd_snaps.json"));
        assert_eq!(
            revs,
            vec![SnapRevision {
                name: "firefox".into(),
                revision: "4793".into(),
                bytes: 262_144_000
            }]
        );
    }

    #[test]
    fn snap_names_are_validated() {
        assert!(valid_snap("core22", "1380"));
        assert!(valid_snap("my-app", "x1"));
        assert!(!valid_snap("a;rm", "1"));
        assert!(!valid_snap("a", "1 2"));
        assert!(!valid_snap("a", ""));
    }

    #[test]
    fn chunked_http_is_decoded() {
        let raw = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        assert_eq!(http_body(raw).unwrap(), "hello world");
        assert!(http_body("HTTP/1.1 401 Unauthorized\r\n\r\n{}").is_none());
    }
}
