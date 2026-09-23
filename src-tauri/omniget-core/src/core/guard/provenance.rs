//! Integrity (sha256 against expected values) and provenance (where it came
//! from: recognized host, HTTPS, pinned commit, license, semver).

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use super::model::{FileDigest, Finding, Provenance, Sink, Validator};
use super::rules::sev;

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

pub fn digests(files: &BTreeMap<String, Vec<u8>>) -> Vec<FileDigest> {
    files
        .iter()
        .map(|(p, b)| FileDigest {
            path: p.clone(),
            sha256: sha256_hex(b),
            size: b.len() as u64,
        })
        .collect()
}

pub fn check_integrity(
    digests: &[FileDigest],
    expected: Option<&BTreeMap<String, String>>,
    sink: &mut Sink,
) {
    let f = |code: &str, file: &str, d: String| {
        Finding::new(code, Validator::Integrity, sev(code), file, d)
    };
    let Some(expected) = expected else {
        sink.push(f("INT_I001", "", "no expected hashes given".into()));
        return;
    };
    let mut bad = false;
    for (path, want) in expected {
        match digests.iter().find(|d| &d.path == path) {
            None => {
                sink.push(f("INT_W001", path, "listed but not present".into()));
                bad = true;
            }
            Some(d) if !d.sha256.eq_ignore_ascii_case(want.trim()) => {
                sink.push(f(
                    "INT_E001",
                    path,
                    format!(
                        "expected {}…, got {}…",
                        &want[..want.len().min(12)],
                        &d.sha256[..12]
                    ),
                ));
                bad = true;
            }
            _ => {}
        }
    }
    for d in digests {
        if !expected.contains_key(&d.path) {
            sink.push(f("INT_W002", &d.path, "not in the expected list".into()));
            bad = true;
        }
    }
    if !bad {
        sink.push(f(
            "INT_I002",
            "",
            format!("{} files verified", digests.len()),
        ));
    }
}

const HOSTS: &[&str] = &[
    "github.com",
    "gitlab.com",
    "bitbucket.org",
    "codeberg.org",
    "git.sr.ht",
    "huggingface.co",
    "raw.githubusercontent.com",
];

/// Merge caller provenance with what the frontmatter declares.
pub fn check_provenance(
    prov: Option<&Provenance>,
    fm_author: Option<&str>,
    fm_version: Option<&str>,
    fm_license: Option<&str>,
    sink: &mut Sink,
) {
    let Some(p) = prov else {
        return;
    };
    let f = |code: &str, d: String| Finding::new(code, Validator::Provenance, sev(code), "", d);
    let url = p.url.clone().or_else(|| {
        p.repo.clone().map(|r| {
            if r.contains("://") {
                r
            } else {
                format!("https://github.com/{r}")
            }
        })
    });
    match &url {
        None => sink.push(f("PROV_W001", "no repo or url".into())),
        Some(u) => {
            if u.starts_with("http://") {
                sink.push(f("PROV_W006", u.clone()));
            }
            let host = super::reference::host_of(u).unwrap_or_default();
            if !HOSTS
                .iter()
                .any(|h| host == *h || host.ends_with(&format!(".{h}")))
            {
                sink.push(f("PROV_W004", host));
            }
        }
    }
    match p.commit.as_deref() {
        Some(c) if c.len() >= 7 && c.chars().all(|x| x.is_ascii_hexdigit()) => {
            sink.push(f("PROV_I002", c[..c.len().min(12)].to_string()))
        }
        _ => sink.push(f(
            "PROV_W008",
            p.commit.clone().unwrap_or_else(|| "no commit".into()),
        )),
    }
    let license = p
        .license
        .as_deref()
        .or(fm_license)
        .filter(|l| !l.trim().is_empty());
    if license.is_none() {
        sink.push(f("PROV_W009", "no license".into()));
    }
    let _ = p.author.as_deref().or(fm_author);
    if let Some(v) = p.version.as_deref().or(fm_version) {
        let core = v.trim().trim_start_matches('v');
        let main = core.split(['-', '+']).next().unwrap_or("");
        let parts: Vec<&str> = main.split('.').collect();
        if parts.len() != 3
            || !parts
                .iter()
                .all(|x| !x.is_empty() && x.chars().all(|c| c.is_ascii_digit()))
        {
            sink.push(f("PROV_W007", v.to_string()));
        }
    }
}
