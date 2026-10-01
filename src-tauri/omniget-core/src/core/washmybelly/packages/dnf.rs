use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{package_app, parse_size, query, tail, App, PackageBackend, RemovePreview, Update};

pub struct Dnf;

pub const RPM_QF: &str = "%{NAME}\\t%{VERSION}-%{RELEASE}\\t%{SIZE}\\t%{VENDOR}\\t%{SUMMARY}\\n";

#[derive(Debug, Clone, PartialEq)]
pub struct RpmPkg {
    pub name: String,
    pub version: String,
    pub bytes: Option<u64>,
    pub vendor: String,
    pub summary: String,
}

pub fn parse_rpm_qa(text: &str) -> Vec<RpmPkg> {
    let mut seen = std::collections::HashSet::new();
    text.lines()
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            if c.len() < 3 || c[0].trim().is_empty() || c[0] == "gpg-pubkey" {
                return None;
            }
            if !seen.insert(c[0].to_string()) {
                return None;
            }
            Some(RpmPkg {
                name: c[0].to_string(),
                version: c[1].to_string(),
                bytes: c[2].trim().parse().ok(),
                vendor: c
                    .get(3)
                    .map(|s| s.trim().to_string())
                    .filter(|s| s != "(none)")
                    .unwrap_or_default(),
                summary: c.get(4).map(|s| s.trim().to_string()).unwrap_or_default(),
            })
        })
        .collect()
}

pub fn rpm_installed() -> Result<Vec<RpmPkg>, String> {
    let r = query("rpm", &["-qa", "--qf", RPM_QF])?;
    if r.stdout.trim().is_empty() {
        return Err(format!("rpm: {}", tail(&r.stderr, 3)));
    }
    Ok(parse_rpm_qa(&r.stdout))
}

pub fn rpm_app(manager: &str, p: &RpmPkg) -> App {
    package_app(
        manager,
        &p.name,
        &p.version,
        p.bytes,
        &p.summary,
        (!p.vendor.is_empty()).then(|| p.vendor.clone()),
    )
}

pub fn parse_rpm_owners(files: &[PathBuf], text: &str) -> HashMap<PathBuf, String> {
    let mut out = HashMap::new();
    let mut lines = text.lines();
    for f in files {
        let Some(l) = lines.next() else { break };
        let l = l.trim();
        if !l.is_empty() && !l.contains(' ') {
            out.insert(f.clone(), l.to_string());
        }
    }
    out
}

pub fn rpm_desktop_owners(files: &[PathBuf]) -> HashMap<PathBuf, String> {
    if files.is_empty() {
        return HashMap::new();
    }
    let mut args: Vec<&str> = vec!["-qf", "--qf", "%{NAME}\\n", "--"];
    args.extend(files.iter().filter_map(|p| p.to_str()));
    query("rpm", &args)
        .map(|r| parse_rpm_owners(files, &r.stdout))
        .unwrap_or_default()
}

pub fn parse_transaction(text: &str) -> (Vec<String>, Option<u64>) {
    let mut names = Vec::new();
    let mut freed = None;
    let mut in_section = false;
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with("Removing") && t.ends_with(':') {
            in_section = true;
            continue;
        }
        if t.starts_with("Transaction Summary") || t.starts_with("Transaction summary") {
            in_section = false;
        }
        if let Some(rest) = t.strip_prefix("After this operation, ") {
            freed = rest.split(" will be freed").next().and_then(parse_size);
        }
        if let Some(rest) = t.strip_prefix("Freed space:") {
            freed = parse_size(rest);
        }
        if in_section && l.starts_with(' ') {
            let w: Vec<&str> = t.split_whitespace().collect();
            if w.len() >= 4 {
                names.push(w[0].to_string());
            }
        } else if in_section && !l.starts_with(' ') && !t.is_empty() && !t.starts_with('=') {
            in_section = false;
        }
    }
    (names, freed)
}

pub fn parse_check_update(text: &str) -> Vec<Update> {
    let mut out = Vec::new();
    for l in text.lines() {
        if l.starts_with("Obsoleting") {
            break;
        }
        if l.starts_with(' ') {
            continue;
        }
        let w: Vec<&str> = l.split_whitespace().collect();
        if w.len() != 3 || !w[1].chars().next().is_some_and(|c| c.is_ascii_digit()) {
            continue;
        }
        let Some((name, _arch)) = w[0].rsplit_once('.') else {
            continue;
        };
        if out.iter().any(|u: &Update| u.name == name) {
            continue;
        }
        out.push(Update {
            id: format!("packages:dnf:{name}"),
            name: name.to_string(),
            manager: "dnf".into(),
            current: String::new(),
            available: w[1].to_string(),
            admin: true,
            ..Default::default()
        });
    }
    out
}

pub fn parse_names(text: &str) -> Vec<String> {
    text.lines()
        .flat_map(|s| s.split("\\n"))
        .map(str::trim)
        .filter(|s| !s.is_empty() && super::valid_name("dnf", s))
        .map(String::from)
        .collect()
}

impl PackageBackend for Dnf {
    fn id(&self) -> &'static str {
        "dnf"
    }

    fn available(&self) -> bool {
        cfg!(target_os = "linux")
            && super::find_program("dnf").is_some()
            && super::find_program("rpm").is_some()
            && !Path::new("/run/ostree-booted").exists()
    }

    fn list(&self) -> Result<Vec<App>, String> {
        Ok(rpm_installed()?.iter().map(|p| rpm_app("dnf", p)).collect())
    }

    fn remove_preview(&self, names: &[String]) -> Result<RemovePreview, String> {
        let mut args = vec!["remove", "--assumeno", "--"];
        args.extend(names.iter().map(String::as_str));
        let r = query("dnf", &args)?;
        let (removed, freed) = parse_transaction(&r.stdout);
        if removed.is_empty() {
            return Err(format!(
                "dnf refuses this removal: {}",
                tail(&format!("{}\n{}", r.stdout, r.stderr), 6)
            ));
        }
        Ok(RemovePreview {
            removes: names.to_vec(),
            also_removes: removed.into_iter().filter(|n| !names.contains(n)).collect(),
            frees_bytes: freed,
            exact: true,
        })
    }

    fn remove_argv(&self, names: &[String]) -> Vec<String> {
        let mut v: Vec<String> = ["dnf", "-y", "remove", "--"].map(String::from).to_vec();
        v.extend(names.iter().cloned());
        v
    }

    fn orphans(&self) -> Result<Vec<App>, String> {
        let r = query("dnf", &["repoquery", "--unneeded", "--qf", "%{name}\\n"])?;
        let names = parse_names(&r.stdout);
        Ok(rpm_installed()?
            .iter()
            .filter(|p| names.contains(&p.name))
            .map(|p| rpm_app("dnf", p))
            .collect())
    }

    fn updates(&self) -> Result<Vec<Update>, String> {
        let r = query("dnf", &["check-update"])?;
        match r.code {
            Some(0) => Ok(Vec::new()),
            Some(100) => {
                let mut v = parse_check_update(&r.stdout);
                let current: HashMap<String, String> = rpm_installed()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|p| (p.name, p.version))
                    .collect();
                for u in &mut v {
                    u.current = current.get(&u.name).cloned().unwrap_or_default();
                }
                Ok(v)
            }
            _ => Err(format!("dnf check-update: {}", tail(&r.stderr, 4))),
        }
    }

    fn upgrade_argv(&self, names: &[String]) -> Vec<String> {
        let mut v: Vec<String> = ["dnf", "-y", "upgrade", "--"].map(String::from).to_vec();
        v.extend(names.iter().cloned());
        v
    }

    fn desktop_owners(&self, files: &[PathBuf]) -> HashMap<PathBuf, String> {
        rpm_desktop_owners(files)
    }
}
