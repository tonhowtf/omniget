use std::collections::HashMap;
use std::path::PathBuf;

use super::dnf::{rpm_app, rpm_desktop_owners, rpm_installed};
use super::{package_app, query, tail, App, PackageBackend, RemovePreview, Update};

pub struct Zypper;

#[derive(Debug, Clone, PartialEq)]
pub struct Solvable {
    pub name: String,
    pub edition: String,
    pub old_edition: String,
    pub summary: String,
}

fn doc(xml: &str) -> Option<roxmltree::Document<'_>> {
    roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).ok()
}

pub fn parse_solvables(xml: &str, tag: &str) -> Vec<Solvable> {
    let Some(d) = doc(xml) else { return Vec::new() };
    d.descendants()
        .filter(|n| n.has_tag_name(tag))
        .filter(|n| {
            n.attribute("kind")
                .or(n.attribute("type"))
                .is_none_or(|k| k == "package")
        })
        .filter_map(|n| {
            Some(Solvable {
                name: n.attribute("name")?.to_string(),
                edition: n.attribute("edition").unwrap_or("").to_string(),
                old_edition: n.attribute("edition-old").unwrap_or("").to_string(),
                summary: n
                    .attribute("summary")
                    .map(String::from)
                    .or_else(|| {
                        n.children()
                            .find(|c| c.has_tag_name("summary"))
                            .and_then(|c| c.text())
                            .map(String::from)
                    })
                    .unwrap_or_default(),
            })
        })
        .collect()
}

pub fn parse_remove_dry_run(xml: &str) -> Result<(Vec<String>, Option<u64>), String> {
    let d = doc(xml).ok_or("zypper: unreadable XML")?;
    if let Some(err) = d
        .descendants()
        .find(|n| n.has_tag_name("message") && n.attribute("type") == Some("error"))
    {
        return Err(err.text().unwrap_or("zypper error").trim().to_string());
    }
    let summary = d.descendants().find(|n| n.has_tag_name("install-summary"));
    let freed = summary
        .and_then(|s| s.attribute("space-usage-removed"))
        .and_then(|v| v.parse().ok());
    let names = d
        .descendants()
        .filter(|n| n.has_tag_name("to-remove"))
        .flat_map(|n| n.children().filter(|c| c.has_tag_name("solvable")))
        .filter_map(|n| n.attribute("name").map(String::from))
        .collect();
    Ok((names, freed))
}

pub fn parse_rpm_needed_by(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in text.lines() {
        let Some(nevra) = l.split("is needed by (installed) ").nth(1) else {
            continue;
        };
        let nevra = nevra.trim();
        let no_arch = nevra.rsplit_once('.').map(|(a, _)| a).unwrap_or(nevra);
        let mut parts: Vec<&str> = no_arch.rsplitn(3, '-').collect();
        if parts.len() == 3 {
            let name = parts.pop().unwrap_or_default().to_string();
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

impl PackageBackend for Zypper {
    fn id(&self) -> &'static str {
        "zypper"
    }

    fn available(&self) -> bool {
        cfg!(target_os = "linux")
            && super::find_program("zypper").is_some()
            && super::find_program("rpm").is_some()
    }

    fn list(&self) -> Result<Vec<App>, String> {
        let r = query(
            "zypper",
            &[
                "--xmlout",
                "--non-interactive",
                "search",
                "--installed-only",
                "--details",
                "--type",
                "package",
            ],
        )?;
        let found = parse_solvables(&r.stdout, "solvable");
        let rpm: HashMap<String, super::dnf::RpmPkg> = rpm_installed()
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.name.clone(), p))
            .collect();
        if found.is_empty() {
            return Ok(rpm.values().map(|p| rpm_app("zypper", p)).collect());
        }
        let mut seen = std::collections::HashSet::new();
        Ok(found
            .into_iter()
            .filter(|s| seen.insert(s.name.clone()))
            .map(|s| match rpm.get(&s.name) {
                Some(p) => rpm_app("zypper", p),
                None => package_app("zypper", &s.name, &s.edition, None, &s.summary, None),
            })
            .collect())
    }

    fn remove_preview(&self, names: &[String]) -> Result<RemovePreview, String> {
        let mut args = vec![
            "--xmlout",
            "--non-interactive",
            "remove",
            "--dry-run",
            "--clean-deps",
            "--",
        ];
        args.extend(names.iter().map(String::as_str));
        let r = query("zypper", &args)?;
        match parse_remove_dry_run(&r.stdout) {
            Ok((removed, freed)) if !removed.is_empty() => Ok(RemovePreview {
                removes: names.to_vec(),
                also_removes: removed.into_iter().filter(|n| !names.contains(n)).collect(),
                frees_bytes: freed,
                exact: true,
            }),
            Ok(_) => Err(format!(
                "zypper would remove nothing: {}",
                tail(&r.stdout, 4)
            )),
            Err(e) if e.contains("Root privileges") => {
                let mut args = vec!["-e", "--test", "--"];
                args.extend(names.iter().map(String::as_str));
                let t = query("rpm", &args)?;
                let needed = parse_rpm_needed_by(&t.stderr);
                let sizes: HashMap<String, u64> = rpm_installed()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|p| p.bytes.map(|b| (p.name, b)))
                    .collect();
                let all: Vec<String> = names.iter().cloned().chain(needed).collect();
                let mut p = super::apt::preview_from(names, all, &sizes);
                p.exact = false;
                Ok(p)
            }
            Err(e) => Err(format!("zypper refuses this removal: {e}")),
        }
    }

    fn remove_argv(&self, names: &[String]) -> Vec<String> {
        let mut v: Vec<String> = [
            "zypper",
            "--non-interactive",
            "remove",
            "--clean-deps",
            "--",
        ]
        .map(String::from)
        .to_vec();
        v.extend(names.iter().cloned());
        v
    }

    fn orphans(&self) -> Result<Vec<App>, String> {
        let r = query(
            "zypper",
            &["--xmlout", "--non-interactive", "packages", "--unneeded"],
        )?;
        let names: Vec<String> = parse_solvables(&r.stdout, "solvable")
            .into_iter()
            .map(|s| s.name)
            .collect();
        if names.is_empty() {
            return Ok(Vec::new());
        }
        Ok(rpm_installed()?
            .iter()
            .filter(|p| names.contains(&p.name))
            .map(|p| rpm_app("zypper", p))
            .collect())
    }

    fn updates(&self) -> Result<Vec<Update>, String> {
        let r = query("zypper", &["--xmlout", "--non-interactive", "list-updates"])?;
        Ok(parse_solvables(&r.stdout, "update")
            .into_iter()
            .map(|s| Update {
                id: format!("packages:zypper:{}", s.name),
                name: s.name,
                manager: "zypper".into(),
                current: s.old_edition,
                available: s.edition,
                admin: true,
                ..Default::default()
            })
            .collect())
    }

    fn upgrade_argv(&self, names: &[String]) -> Vec<String> {
        let mut v: Vec<String> = ["zypper", "--non-interactive", "update", "--"]
            .map(String::from)
            .to_vec();
        v.extend(names.iter().cloned());
        v
    }

    fn desktop_owners(&self, files: &[PathBuf]) -> HashMap<PathBuf, String> {
        rpm_desktop_owners(files)
    }
}
