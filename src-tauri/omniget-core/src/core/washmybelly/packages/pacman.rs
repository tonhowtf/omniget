use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{package_app, parse_size, query, tail, App, PackageBackend, RemovePreview, Update};

pub struct Pacman;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PacPkg {
    pub name: String,
    pub version: String,
    pub description: String,
    pub bytes: Option<u64>,
    pub packager: String,
    pub explicit: bool,
}

pub fn parse_qi(text: &str) -> Vec<PacPkg> {
    let mut out = Vec::new();
    let mut cur = PacPkg::default();
    let flush = |cur: &mut PacPkg, out: &mut Vec<PacPkg>| {
        if !cur.name.is_empty() {
            out.push(std::mem::take(cur));
        }
    };
    for l in text.lines() {
        if l.trim().is_empty() {
            flush(&mut cur, &mut out);
            continue;
        }
        let Some((k, v)) = l.split_once(" : ") else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "Name" => cur.name = v.to_string(),
            "Version" => cur.version = v.to_string(),
            "Description" => cur.description = v.to_string(),
            "Installed Size" => cur.bytes = parse_size(v),
            "Packager" => cur.packager = v.split('<').next().unwrap_or(v).trim().to_string(),
            "Install Reason" => cur.explicit = v.starts_with("Explicitly"),
            _ => {}
        }
    }
    flush(&mut cur, &mut out);
    out
}

pub fn parse_print(text: &str) -> Vec<(String, Option<u64>)> {
    text.lines()
        .filter_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            (w.len() == 3 && !l.starts_with("::") && !l.starts_with("error"))
                .then(|| (w[0].to_string(), w[2].parse().ok()))
        })
        .collect()
}

pub fn parse_qu(text: &str) -> Vec<Update> {
    text.lines()
        .filter_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            if w.len() < 4 || w[2] != "->" || l.contains("[ignored]") {
                return None;
            }
            Some(Update {
                id: format!("packages:pacman:{}", w[0]),
                name: w[0].to_string(),
                manager: "pacman".into(),
                current: w[1].to_string(),
                available: w[3].to_string(),
                admin: true,
                ..Default::default()
            })
        })
        .collect()
}

fn installed() -> Result<Vec<PacPkg>, String> {
    let r = query("pacman", &["-Qi"])?;
    if r.stdout.trim().is_empty() {
        return Err(format!("pacman: {}", tail(&r.stderr, 3)));
    }
    Ok(parse_qi(&r.stdout))
}

fn to_app(p: &PacPkg) -> App {
    let mut a = package_app(
        "pacman",
        &p.name,
        &p.version,
        p.bytes,
        &p.description,
        (!p.packager.is_empty()).then(|| p.packager.clone()),
    );
    a.extra["explicit"] = serde_json::json!(p.explicit);
    a
}

impl PackageBackend for Pacman {
    fn id(&self) -> &'static str {
        "pacman"
    }

    fn available(&self) -> bool {
        cfg!(target_os = "linux")
            && super::find_program("pacman").is_some()
            && Path::new("/var/lib/pacman/local").is_dir()
    }

    fn list(&self) -> Result<Vec<App>, String> {
        Ok(installed()?.iter().map(to_app).collect())
    }

    fn remove_preview(&self, names: &[String]) -> Result<RemovePreview, String> {
        let mut args = vec!["-Rs", "--print", "--print-format", "%n %v %s", "--"];
        args.extend(names.iter().map(String::as_str));
        let r = query("pacman", &args)?;
        let rows = parse_print(&r.stdout);
        if r.code != Some(0) || rows.is_empty() {
            return Err(format!(
                "pacman refuses this removal: {}",
                tail(&format!("{}\n{}", r.stdout, r.stderr), 6)
            ));
        }
        let frees = rows.iter().filter_map(|(_, b)| *b).sum();
        Ok(RemovePreview {
            removes: names.to_vec(),
            also_removes: rows
                .into_iter()
                .map(|(n, _)| n)
                .filter(|n| !names.contains(n))
                .collect(),
            frees_bytes: Some(frees),
            exact: true,
        })
    }

    fn remove_argv(&self, names: &[String]) -> Vec<String> {
        let mut v: Vec<String> = ["pacman", "-Rns", "--noconfirm", "--"]
            .map(String::from)
            .to_vec();
        v.extend(names.iter().cloned());
        v
    }

    fn orphans(&self) -> Result<Vec<App>, String> {
        let r = query("pacman", &["-Qdtq"])?;
        let names: Vec<&str> = r
            .stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if names.is_empty() {
            return Ok(Vec::new());
        }
        Ok(installed()?
            .iter()
            .filter(|p| names.contains(&p.name.as_str()))
            .map(to_app)
            .collect())
    }

    fn updates(&self) -> Result<Vec<Update>, String> {
        if super::find_program("checkupdates").is_some() {
            let r = query("checkupdates", &[])?;
            return match r.code {
                Some(0) => Ok(parse_qu(&r.stdout)),
                Some(2) => Ok(Vec::new()),
                _ => Err(format!("checkupdates: {}", tail(&r.stderr, 3))),
            };
        }
        let r = query("pacman", &["-Qu"])?;
        Ok(parse_qu(&r.stdout))
    }

    fn upgrade_argv(&self, _names: &[String]) -> Vec<String> {
        ["pacman", "-Syu", "--noconfirm"].map(String::from).to_vec()
    }

    fn desktop_owners(&self, files: &[PathBuf]) -> HashMap<PathBuf, String> {
        let mut out = HashMap::new();
        if files.is_empty() {
            return out;
        }
        let mut args: Vec<&str> = vec!["-Qqo", "--"];
        args.extend(files.iter().filter_map(|p| p.to_str()));
        if let Ok(r) = query("pacman", &args) {
            let names: Vec<&str> = r
                .stdout
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            if names.len() == files.len() {
                for (f, n) in files.iter().zip(names) {
                    out.insert(f.clone(), n.to_string());
                }
                return out;
            }
        }
        for f in files {
            let Some(p) = f.to_str() else { continue };
            if let Ok(r) = query("pacman", &["-Qqo", "--", p]) {
                if let Some(n) = r.stdout.lines().next().filter(|n| !n.trim().is_empty()) {
                    out.insert(f.clone(), n.trim().to_string());
                }
            }
        }
        out
    }
}
