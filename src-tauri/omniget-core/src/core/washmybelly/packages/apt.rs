use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{package_app, query, tail, App, PackageBackend, RemovePreview, Update};

pub struct Apt;

const FORMAT: &str =
    "-f=${Package}\\t${Version}\\t${Installed-Size}\\t${db:Status-Abbrev}\\t${binary:Summary}\\n";

#[derive(Debug, Clone, PartialEq)]
pub struct DebPkg {
    pub name: String,
    pub version: String,
    pub bytes: Option<u64>,
    pub summary: String,
}

pub fn parse_dpkg_query(text: &str) -> Vec<DebPkg> {
    let mut seen = std::collections::HashSet::new();
    text.lines()
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            if c.len() < 4 || !matches!(c[3].trim(), "ii" | "hi") {
                return None;
            }
            let name = c[0].trim();
            if name.is_empty() || !seen.insert(name.to_string()) {
                return None;
            }
            Some(DebPkg {
                name: name.to_string(),
                version: c[1].trim().to_string(),
                bytes: c[2].trim().parse::<u64>().ok().map(|k| k * 1024),
                summary: c.get(4).map(|s| s.trim().to_string()).unwrap_or_default(),
            })
        })
        .collect()
}

pub fn parse_dpkg_search(text: &str) -> HashMap<PathBuf, String> {
    let mut out = HashMap::new();
    for l in text.lines() {
        if l.starts_with("diversion ") {
            continue;
        }
        let Some((pkgs, path)) = l.split_once(": ") else {
            continue;
        };
        let first = pkgs.split(", ").next().unwrap_or("").trim();
        let first = first.split(':').next().unwrap_or(first);
        if !first.is_empty() && path.starts_with('/') {
            out.insert(PathBuf::from(path.trim()), first.to_string());
        }
    }
    out
}

pub fn parse_simulation(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.strip_prefix("Remv "))
        .filter_map(|r| r.split_whitespace().next())
        .map(|n| n.split(':').next().unwrap_or(n).to_string())
        .collect()
}

pub fn parse_upgradable(text: &str) -> Vec<Update> {
    text.lines()
        .filter_map(|l| {
            let (name, rest) = l.split_once('/')?;
            let mut w = rest.split_whitespace();
            let _suite = w.next()?;
            let new = w.next()?;
            let old = l
                .split("[upgradable from: ")
                .nth(1)?
                .trim_end_matches(']')
                .trim();
            Some(Update {
                id: format!("packages:apt:{name}"),
                name: name.to_string(),
                manager: "apt".into(),
                current: old.to_string(),
                available: new.to_string(),
                admin: true,
                ..Default::default()
            })
        })
        .collect()
}

fn installed() -> Result<Vec<DebPkg>, String> {
    let r = query("dpkg-query", &["-W", FORMAT])?;
    if r.stdout.trim().is_empty() && r.code != Some(0) {
        return Err(format!("dpkg-query: {}", tail(&r.stderr, 3)));
    }
    Ok(parse_dpkg_query(&r.stdout))
}

fn to_app(p: &DebPkg) -> App {
    package_app("apt", &p.name, &p.version, p.bytes, &p.summary, None)
}

impl PackageBackend for Apt {
    fn id(&self) -> &'static str {
        "apt"
    }

    fn available(&self) -> bool {
        cfg!(target_os = "linux")
            && super::find_program("apt-get").is_some()
            && super::find_program("dpkg-query").is_some()
            && Path::new("/var/lib/dpkg/status").is_file()
    }

    fn list(&self) -> Result<Vec<App>, String> {
        Ok(installed()?.iter().map(to_app).collect())
    }

    fn remove_preview(&self, names: &[String]) -> Result<RemovePreview, String> {
        let mut args = vec!["-s", "remove", "--"];
        args.extend(names.iter().map(String::as_str));
        let r = query("apt-get", &args)?;
        let removed = parse_simulation(&r.stdout);
        if r.code != Some(0) || removed.is_empty() {
            return Err(format!(
                "apt refuses this removal: {}",
                tail(&format!("{}\n{}", r.stdout, r.stderr), 6)
            ));
        }
        let sizes: HashMap<String, u64> = installed()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|p| p.bytes.map(|b| (p.name, b)))
            .collect();
        Ok(preview_from(names, removed, &sizes))
    }

    fn remove_argv(&self, names: &[String]) -> Vec<String> {
        let mut v: Vec<String> = ["apt-get", "-y", "remove", "--"].map(String::from).to_vec();
        v.extend(names.iter().cloned());
        v
    }

    fn orphans(&self) -> Result<Vec<App>, String> {
        let r = query("apt-get", &["-s", "autoremove"])?;
        let names = parse_simulation(&r.stdout);
        let all = installed().unwrap_or_default();
        Ok(all
            .iter()
            .filter(|p| names.contains(&p.name))
            .map(to_app)
            .collect())
    }

    fn updates(&self) -> Result<Vec<Update>, String> {
        let r = query("apt", &["list", "--upgradable"])?;
        Ok(parse_upgradable(&r.stdout))
    }

    fn upgrade_argv(&self, names: &[String]) -> Vec<String> {
        let mut v: Vec<String> = ["apt-get", "-y", "install", "--only-upgrade", "--"]
            .map(String::from)
            .to_vec();
        v.extend(names.iter().cloned());
        v
    }

    fn desktop_owners(&self, files: &[PathBuf]) -> HashMap<PathBuf, String> {
        if files.is_empty() {
            return HashMap::new();
        }
        let mut args: Vec<&str> = vec!["-S", "--"];
        args.extend(files.iter().filter_map(|p| p.to_str()));
        query("dpkg-query", &args)
            .map(|r| parse_dpkg_search(&r.stdout))
            .unwrap_or_default()
    }
}

pub fn preview_from(
    asked: &[String],
    removed: Vec<String>,
    sizes: &HashMap<String, u64>,
) -> RemovePreview {
    let frees: u64 = removed.iter().filter_map(|n| sizes.get(n)).sum();
    let known = removed.iter().any(|n| sizes.contains_key(n));
    RemovePreview {
        removes: asked.to_vec(),
        also_removes: removed.into_iter().filter(|n| !asked.contains(n)).collect(),
        frees_bytes: known.then_some(frees),
        exact: true,
    }
}
