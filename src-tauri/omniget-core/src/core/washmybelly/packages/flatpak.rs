//! flatpak.
//!
//! - list: `flatpak list --app --columns=application,name,version,size,installation`
//!   (tab separated; sizes by GLib's `g_format_size`, SI units with a
//!   no-break space: `1.2 GB`);
//! - removal preview: `flatpak uninstall --user|--system -- <ids>` WITHOUT
//!   `-y`: flatpak prints the transaction table (`  1.  id  [arch]  branch  r`)
//!   and, with stdin/stdout not a terminal, answers its own prompt "n"
//!   (`flatpak_yes_no_prompt`, app/flatpak-tty-utils.c). Nothing is removed;
//! - orphans: `flatpak uninstall --unused` the same way;
//! - updates: `flatpak remote-ls --updates --user|--system --columns=application,version,branch,download-size,ref`;
//! - remove: `flatpak uninstall -y --noninteractive --user -- <ids>` needs no
//!   administrator (Nexis does the same); `--system` goes through the helper;
//! - upgrade: `flatpak update -y --noninteractive --user|--system -- <ref>`.

use std::collections::HashMap;

use serde_json::json;

use super::{parse_size, query, tail, App, PackageBackend, RemovePreview, Update};

pub struct Flatpak;

#[derive(Debug, Clone, PartialEq)]
pub struct FlatpakApp {
    pub id: String,
    pub name: String,
    pub version: String,
    pub bytes: Option<u64>,
    /// `user`, `system` or a custom installation name.
    pub installation: String,
}

/// `flatpak list --app --columns=application,name,version,size,installation`.
pub fn parse_list(text: &str) -> Vec<FlatpakApp> {
    text.lines()
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            let id = c.first()?.trim();
            if c.len() < 5 || !super::valid_name("flatpak", id) {
                return None;
            }
            Some(FlatpakApp {
                id: id.to_string(),
                name: Some(c[1].trim())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(id)
                    .to_string(),
                version: c[2].trim().to_string(),
                bytes: parse_size(c[3]),
                installation: c[4].trim().to_string(),
            })
        })
        .collect()
}

/// Ids of the numbered rows of a flatpak transaction table (uninstall ops).
pub fn parse_transaction(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            let num = w.first()?;
            if w.len() < 4
                || !num.ends_with('.')
                || !num
                    .trim_end_matches('.')
                    .chars()
                    .all(|c| c.is_ascii_digit())
            {
                return None;
            }
            // id [arch] branch op
            let id = w[1];
            let branch = w[w.len() - 2];
            (w[w.len() - 1] == "r" && super::valid_name("flatpak", id))
                .then(|| format!("{id}//{branch}"))
        })
        .collect()
}

/// `flatpak remote-ls --updates --columns=application,version,branch,download-size,ref`.
pub fn parse_remote_updates(
    text: &str,
    scope: &str,
    current: &HashMap<String, String>,
) -> Vec<Update> {
    text.lines()
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            if c.len() < 5 {
                return None;
            }
            let (app, version, reference) = (c[0].trim(), c[1].trim(), c[4].trim());
            if !super::valid_name("flatpak", reference) {
                return None;
            }
            Some(Update {
                id: format!("packages:flatpak:{scope}:{reference}"),
                name: reference.to_string(),
                manager: "flatpak".into(),
                current: current
                    .get(&format!("{scope}:{app}"))
                    .cloned()
                    .unwrap_or_default(),
                available: version.to_string(),
                size: parse_size(c[3]),
                admin: scope != "user",
                scope: Some(scope.to_string()),
                note: None,
            })
        })
        .collect()
}

pub fn installed() -> Result<Vec<FlatpakApp>, String> {
    let r = query(
        "flatpak",
        &[
            "list",
            "--app",
            "--columns=application,name,version,size,installation",
        ],
    )?;
    if r.code != Some(0) {
        return Err(format!("flatpak list: {}", tail(&r.stderr, 3)));
    }
    Ok(parse_list(&r.stdout))
}

/// App ids per installation, for the stable ids apps/linux builds.
pub fn to_app(f: &FlatpakApp, id: String) -> App {
    let user = f.installation == "user";
    App {
        id,
        name: f.name.clone(),
        version: (!f.version.is_empty()).then(|| f.version.clone()),
        publisher: None,
        source: "flatpak".into(),
        path: dirs::home_dir().map(|h| {
            h.join(".var/app")
                .join(&f.id)
                .to_string_lossy()
                .into_owned()
        }),
        bytes: f.bytes,
        native_id: Some(f.id.clone()),
        removable: true,
        needs_admin: !user,
        extra: json!({ "installation": f.installation }),
        ..Default::default()
    }
}

fn scope_flag(installation: &str) -> Result<&'static str, String> {
    match installation {
        "user" => Ok("--user"),
        "system" => Ok("--system"),
        other => Err(format!("flatpak installation {other} is not supported")),
    }
}

/// Preview a removal in one installation.
pub fn preview_in(installation: &str, names: &[String]) -> Result<RemovePreview, String> {
    let mut args = vec!["uninstall", scope_flag(installation)?, "--"];
    args.extend(names.iter().map(String::as_str));
    let r = query("flatpak", &args)?;
    let rows = parse_transaction(&r.stdout);
    if rows.is_empty() {
        return Err(format!(
            "flatpak refuses this removal: {}",
            tail(&format!("{}\n{}", r.stdout, r.stderr), 4)
        ));
    }
    let asked = |row: &String| {
        names
            .iter()
            .any(|n| row == n || row.starts_with(&format!("{n}//")))
    };
    let sizes: HashMap<String, u64> = installed()
        .unwrap_or_default()
        .into_iter()
        .filter(|f| f.installation == installation)
        .filter_map(|f| f.bytes.map(|b| (f.id, b)))
        .collect();
    Ok(RemovePreview {
        removes: names.to_vec(),
        also_removes: rows.iter().filter(|r| !asked(r)).cloned().collect(),
        frees_bytes: Some(names.iter().filter_map(|n| sizes.get(n)).sum()),
        exact: true,
    })
}

/// argv for a removal in the user installation (no administrator).
pub fn user_remove_argv(names: &[String]) -> Vec<String> {
    let mut v: Vec<String> = [
        "flatpak",
        "uninstall",
        "-y",
        "--noninteractive",
        "--user",
        "--",
    ]
    .map(String::from)
    .to_vec();
    v.extend(names.iter().cloned());
    v
}

/// Remove from the user installation, in the app's own process.
pub fn remove_user(names: &[String]) -> Result<String, String> {
    super::validate("flatpak", names, &["flatpak"])?;
    let argv = user_remove_argv(names);
    let args: Vec<&str> = argv[1..].iter().map(String::as_str).collect();
    let r = query("flatpak", &args)?;
    if r.code == Some(0) {
        Ok(format!("removed {}", names.join(", ")))
    } else {
        Err(format!(
            "flatpak: {}",
            tail(&format!("{}\n{}", r.stdout, r.stderr), 4)
        ))
    }
}

/// Update one ref in the user installation.
pub fn upgrade_user(reference: &str) -> Result<String, String> {
    if !super::valid_name("flatpak", reference) {
        return Err("BAD_NAME".into());
    }
    let r = query(
        "flatpak",
        &[
            "update",
            "-y",
            "--noninteractive",
            "--user",
            "--",
            reference,
        ],
    )?;
    if r.code == Some(0) {
        Ok("updated".into())
    } else {
        Err(format!(
            "flatpak: {}",
            tail(&format!("{}\n{}", r.stdout, r.stderr), 4)
        ))
    }
}

impl PackageBackend for Flatpak {
    fn id(&self) -> &'static str {
        "flatpak"
    }

    fn available(&self) -> bool {
        cfg!(target_os = "linux") && super::find_program("flatpak").is_some()
    }

    fn list(&self) -> Result<Vec<App>, String> {
        Ok(installed()?
            .iter()
            .map(|f| to_app(f, format!("flatpak:{}", f.id)))
            .collect())
    }

    /// The system installation (the one the helper removes from).
    fn remove_preview(&self, names: &[String]) -> Result<RemovePreview, String> {
        preview_in("system", names)
    }

    fn remove_argv(&self, names: &[String]) -> Vec<String> {
        let mut v: Vec<String> = [
            "flatpak",
            "uninstall",
            "-y",
            "--noninteractive",
            "--system",
            "--",
        ]
        .map(String::from)
        .to_vec();
        v.extend(names.iter().cloned());
        v
    }

    fn orphans(&self) -> Result<Vec<App>, String> {
        let mut out = Vec::new();
        for inst in ["user", "system"] {
            let r = query("flatpak", &["uninstall", "--unused", scope_flag(inst)?])?;
            for reference in parse_transaction(&r.stdout) {
                let id = reference
                    .split("//")
                    .next()
                    .unwrap_or(&reference)
                    .to_string();
                out.push(App {
                    id: format!("flatpak:{inst}:{reference}"),
                    name: id.clone(),
                    source: "flatpak".into(),
                    native_id: Some(reference.clone()),
                    removable: true,
                    needs_admin: inst != "user",
                    extra: json!({ "installation": inst, "orphan": true, "runtime": true }),
                    ..Default::default()
                });
            }
        }
        Ok(out)
    }

    fn updates(&self) -> Result<Vec<Update>, String> {
        let all = query(
            "flatpak",
            &["list", "--columns=application,version,installation"],
        )?;
        let current: HashMap<String, String> = all
            .stdout
            .lines()
            .filter_map(|l| {
                let c: Vec<&str> = l.split('\t').collect();
                (c.len() >= 3).then(|| {
                    (
                        format!("{}:{}", c[2].trim(), c[0].trim()),
                        c[1].trim().to_string(),
                    )
                })
            })
            .collect();
        let mut out = Vec::new();
        for inst in ["user", "system"] {
            let r = query(
                "flatpak",
                &[
                    "remote-ls",
                    "--updates",
                    scope_flag(inst)?,
                    "--columns=application,version,branch,download-size,ref",
                ],
            )?;
            out.extend(parse_remote_updates(&r.stdout, inst, &current));
        }
        Ok(out)
    }

    fn upgrade_argv(&self, names: &[String]) -> Vec<String> {
        let mut v: Vec<String> = [
            "flatpak",
            "update",
            "-y",
            "--noninteractive",
            "--system",
            "--",
        ]
        .map(String::from)
        .to_vec();
        v.extend(names.iter().cloned());
        v
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixture;
    use super::*;

    #[test]
    fn list_columns() {
        let v = parse_list(&fixture("flatpak_list.txt"));
        assert_eq!(v.len(), 2, "the bad id is dropped");
        assert_eq!(v[0].id, "org.gimp.GIMP");
        assert_eq!(v[0].bytes, Some(1_200_000_000));
        assert_eq!(v[0].installation, "system");
        assert_eq!(v[1].installation, "user");
        assert!(!to_app(&v[1], "flatpak:org.videolan.VLC".into()).needs_admin);
        assert!(to_app(&v[0], "flatpak:org.gimp.GIMP".into()).needs_admin);
    }

    #[test]
    fn uninstall_table() {
        let v = parse_transaction(&fixture("flatpak_uninstall_preview.txt"));
        assert_eq!(
            v,
            vec!["org.gimp.GIMP.Locale//stable", "org.gimp.GIMP//stable"]
        );
        // With the arch column (non-default arch).
        assert_eq!(
            parse_transaction(" 1.     org.x.Y   aarch64   stable   r\n"),
            vec!["org.x.Y//stable"]
        );
    }

    #[test]
    fn remote_updates() {
        let cur = HashMap::from([("user:org.videolan.VLC".to_string(), "3.0.20".to_string())]);
        let v = parse_remote_updates(&fixture("flatpak_remote_ls_updates.txt"), "user", &cur);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "app/org.videolan.VLC/x86_64/stable");
        assert_eq!(v[0].current, "3.0.20");
        assert_eq!(v[0].size, Some(24_100_000));
        assert!(!v[0].admin);
        assert_eq!(
            v[0].id,
            "packages:flatpak:user:app/org.videolan.VLC/x86_64/stable"
        );
        let v = parse_remote_updates(&fixture("flatpak_remote_ls_updates.txt"), "system", &cur);
        assert!(v.iter().all(|u| u.admin));
    }

    #[test]
    fn user_removal_argv() {
        assert_eq!(
            user_remove_argv(&["org.x.Y".into()]),
            vec![
                "flatpak",
                "uninstall",
                "-y",
                "--noninteractive",
                "--user",
                "--",
                "org.x.Y"
            ]
        );
        assert!(remove_user(&["--all".into()])
            .unwrap_err()
            .starts_with("BAD_NAME"));
    }
}
