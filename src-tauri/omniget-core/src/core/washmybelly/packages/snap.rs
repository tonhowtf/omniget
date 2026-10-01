use serde_json::Value;

use super::{package_app, App, PackageBackend, RemovePreview, Update};

pub struct Snap;

#[derive(Debug, Clone, PartialEq)]
pub struct SnapInfo {
    pub name: String,
    pub title: String,
    pub version: String,
    pub kind: String,
    pub bytes: Option<u64>,
    pub publisher: String,
    pub summary: String,
}

fn results(body: &str) -> Vec<Value> {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("result").and_then(Value::as_array).cloned())
        .unwrap_or_default()
}

pub fn parse_snaps(body: &str) -> Vec<SnapInfo> {
    results(body)
        .into_iter()
        .filter_map(|s| {
            let name = s["name"].as_str()?.to_string();
            if !super::valid_name("snap", &name) {
                return None;
            }
            Some(SnapInfo {
                title: s["title"]
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .unwrap_or(&name)
                    .to_string(),
                version: s["version"].as_str().unwrap_or("").to_string(),
                kind: s["type"].as_str().unwrap_or("app").to_string(),
                bytes: s["installed-size"]
                    .as_u64()
                    .or_else(|| s["download-size"].as_u64()),
                publisher: s["publisher"]["display-name"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                summary: s["summary"].as_str().unwrap_or("").to_string(),
                name,
            })
        })
        .collect()
}

pub fn parse_refresh(body: &str, installed: &[SnapInfo]) -> Vec<Update> {
    parse_snaps(body)
        .into_iter()
        .map(|s| Update {
            id: format!("packages:snap:{}", s.name),
            current: installed
                .iter()
                .find(|i| i.name == s.name)
                .map(|i| i.version.clone())
                .unwrap_or_default(),
            available: s.version,
            size: s.bytes,
            manager: "snap".into(),
            admin: true,
            name: s.name,
            ..Default::default()
        })
        .collect()
}

fn get(path: &str) -> Result<String, String> {
    super::super::clean::rules_linux::snapd_get(path)
        .ok_or_else(|| format!("snapd did not answer {path}"))
}

fn installed() -> Result<Vec<SnapInfo>, String> {
    Ok(parse_snaps(&get("/v2/snaps")?))
}

pub fn to_app(s: &SnapInfo) -> App {
    let mut a = package_app(
        "snap",
        &s.name,
        &s.version,
        s.bytes,
        &s.summary,
        (!s.publisher.is_empty()).then(|| s.publisher.clone()),
    );
    a.name = s.title.clone();
    a.path = Some(format!("/snap/{}", s.name));
    a
}

impl PackageBackend for Snap {
    fn id(&self) -> &'static str {
        "snap"
    }

    fn available(&self) -> bool {
        cfg!(target_os = "linux")
            && std::path::Path::new("/run/snapd.socket").exists()
            && super::find_program("snap").is_some()
    }

    fn list(&self) -> Result<Vec<App>, String> {
        Ok(installed()?
            .iter()
            .filter(|s| s.kind == "app")
            .map(to_app)
            .collect())
    }

    fn remove_preview(&self, names: &[String]) -> Result<RemovePreview, String> {
        let all = installed()?;
        let mut frees = 0;
        for n in names {
            let s = all
                .iter()
                .find(|s| &s.name == n)
                .ok_or_else(|| format!("{n} is not installed"))?;
            if s.kind != "app" {
                return Err(format!("{n} is a {} snap other snaps need", s.kind));
            }
            frees += s.bytes.unwrap_or(0);
        }
        Ok(RemovePreview {
            removes: names.to_vec(),
            also_removes: Vec::new(),
            frees_bytes: Some(frees),
            exact: true,
        })
    }

    fn remove_argv(&self, names: &[String]) -> Vec<String> {
        let mut v: Vec<String> = ["snap", "remove", "--"].map(String::from).to_vec();
        v.extend(names.iter().cloned());
        v
    }

    fn orphans(&self) -> Result<Vec<App>, String> {
        Ok(Vec::new())
    }

    fn updates(&self) -> Result<Vec<Update>, String> {
        let inst = installed()?;
        Ok(parse_refresh(&get("/v2/find?select=refresh")?, &inst))
    }

    fn upgrade_argv(&self, names: &[String]) -> Vec<String> {
        let mut v: Vec<String> = ["snap", "refresh", "--"].map(String::from).to_vec();
        v.extend(names.iter().cloned());
        v
    }
}
