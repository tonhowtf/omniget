//! What an app left in `~/Library` and `/Library`, by evidence.
//!
//! Order of confidence (our own rules, written from the idea list of the
//! study; nothing copied from other uninstallers):
//!
//! 1. bundle id lineage (`<id>` or `<id>.<more>`) in the folders macOS names
//!    after it, app groups from the signature, Group Containers of the form
//!    `<TEAM>.<id>`/`group.<id>`, launchd jobs whose label, associated
//!    bundle id or program point at the app — `high`;
//! 2. sandbox containers with a UUID name whose metadata says the owner is
//!    this bundle id — `high`;
//! 3. a Group Container of the same Team ID and nothing else — `medium`,
//!    or `low` when another installed app has the same Team ID;
//! 4. a folder whose name equals the app's name (letters and digits only,
//!    four or more) — `low`, never ticked.
//!
//! Guards: nothing under `com.apple.*`; nothing at all when another
//! installed app has the same bundle id (the sibling guard); an entry that
//! belongs by lineage to a longer bundle id of another installed app stays.
//! Pure over the file system: tests point it at a fake home.

use std::path::{Path, PathBuf};

use super::parse::{self, is_apple, lineage, normalize};
use super::plist;

/// The app we look for.
#[derive(Debug, Clone, Default)]
pub struct Subject {
    pub bundle_id: Option<String>,
    pub team_id: Option<String>,
    /// Display name, bundle file stem, executable (raw; normalized here).
    pub names: Vec<String>,
    pub app_path: Option<PathBuf>,
    /// `com.apple.security.application-groups` of its signature.
    pub app_groups: Vec<String>,
}

/// Another installed app (for the guards).
#[derive(Debug, Clone, Default)]
pub struct Other {
    pub name: String,
    pub path: String,
    pub bundle_id: Option<String>,
    pub team_id: Option<String>,
}

/// Where to look.
#[derive(Debug, Clone)]
pub struct Roots {
    /// `~` (a temporary folder in tests).
    pub home: PathBuf,
    /// `/Library`, or `None` to skip the admin-only places.
    pub system: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launchd {
    /// `~/Library/LaunchAgents`: `gui/<uid>` domain, no admin.
    UserAgent,
    /// `/Library/LaunchAgents`: `gui/<uid>` domain, file owned by root.
    SystemAgent,
    /// `/Library/LaunchDaemons`: `system` domain.
    Daemon,
}

#[derive(Debug, Clone)]
pub struct Found {
    pub path: PathBuf,
    /// `high`, `medium`, `low`.
    pub confidence: &'static str,
    pub evidence: String,
    /// `file`, `folder`, `launchd`.
    pub kind: &'static str,
    /// `data`, `cache`, `preferences`, `launch`, `other`.
    pub group: &'static str,
    pub admin: bool,
    /// The Library folder it was found in (the plan's `root`).
    pub root: PathBuf,
    pub launchd: Option<(Launchd, String)>,
}

/// Why a search came back short, for the notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    Apple,
    SharedBundleId(Vec<String>),
    NoBundleId,
    UnreadableContainers(usize),
}

/// `(subfolder of Library, group)`. Entries are matched by bundle id
/// lineage (and by name, with low confidence, where `by_name` is true).
const USER_DIRS: &[(&str, &str, bool)] = &[
    ("Application Support", "data", true),
    ("Application Scripts", "data", false),
    ("Containers", "data", false),
    ("Caches", "cache", true),
    ("HTTPStorages", "cache", false),
    ("WebKit", "cache", false),
    ("Logs", "cache", true),
    ("Saved Application State", "cache", false),
    ("Cookies", "data", false),
    ("Preferences", "preferences", false),
    ("Preferences/ByHost", "preferences", false),
];

const SYSTEM_DIRS: &[(&str, &str)] = &[
    ("Application Support", "data"),
    ("Caches", "cache"),
    ("Logs", "cache"),
    ("Preferences", "preferences"),
    ("PrivilegedHelperTools", "launch"),
];

fn entries(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<(String, PathBuf)> = rd
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(n, _)| !n.starts_with('.'))
        .collect();
    v.sort();
    v
}

fn kind_of(p: &Path) -> &'static str {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => "folder",
        _ => "file",
    }
}

/// True when `name` belongs to a longer bundle id of another installed app
/// (`com.foo.app.pro` is not a leftover of `com.foo.app`).
fn belongs_to_other(name: &str, id: &str, others: &[Other]) -> Option<String> {
    let id_l = id.to_ascii_lowercase();
    others.iter().find_map(|o| {
        let ob = o.bundle_id.as_deref()?;
        let ob_l = ob.to_ascii_lowercase();
        (ob_l != id_l && lineage(ob, id) && lineage(name, ob)).then(|| o.name.clone())
    })
}

pub fn find(roots: &Roots, subj: &Subject, others: &[Other]) -> (Vec<Found>, Vec<Note>) {
    let mut notes = Vec::new();
    let mut out: Vec<Found> = Vec::new();
    let id = subj.bundle_id.clone().filter(|b| parse::valid_bundle_id(b));
    if let Some(b) = &id {
        if is_apple(b) {
            return (out, vec![Note::Apple]);
        }
        let twins: Vec<String> = others
            .iter()
            .filter(|o| {
                o.bundle_id
                    .as_deref()
                    .is_some_and(|x| x.eq_ignore_ascii_case(b))
            })
            .map(|o| o.path.clone())
            .collect();
        if !twins.is_empty() {
            return (out, vec![Note::SharedBundleId(twins)]);
        }
    } else {
        notes.push(Note::NoBundleId);
    }
    let lib = roots.home.join("Library");
    let names: Vec<String> = subj
        .names
        .iter()
        .map(|n| normalize(n.trim_end_matches(".app")))
        .filter(|n| n.chars().count() >= 4)
        .collect();
    let other_names: Vec<String> = others.iter().map(|o| normalize(&o.name)).collect();
    let mut unreadable = 0usize;

    let push = |out: &mut Vec<Found>, f: Found| {
        if !out.iter().any(|x| x.path == f.path) {
            out.push(f);
        }
    };

    // 1 + 4: bundle id lineage and name, user Library.
    for &(sub, group, by_name) in USER_DIRS {
        let root = lib.join(sub);
        for (name, path) in entries(&root) {
            if is_apple(&name) {
                continue;
            }
            if let Some(b) = &id {
                if lineage(&name, b) && belongs_to_other(&name, b, others).is_none() {
                    push(
                        &mut out,
                        Found {
                            kind: kind_of(&path),
                            path,
                            confidence: "high",
                            evidence: format!("named after the bundle id {b} in {sub}"),
                            group,
                            admin: false,
                            root: root.clone(),
                            launchd: None,
                        },
                    );
                    continue;
                }
                // 2: sandbox containers with a UUID name.
                if sub == "Containers" && parse::is_uuid(&name) {
                    let meta = path.join(".com.apple.containermanagerd.metadata.plist");
                    match plist::read(&meta) {
                        Ok(v) => {
                            if let Some(owner) = parse::container_owner(&v) {
                                if lineage(&owner, b)
                                    && belongs_to_other(&owner, b, others).is_none()
                                {
                                    push(
                                        &mut out,
                                        Found {
                                            kind: "folder",
                                            path,
                                            confidence: "high",
                                            evidence: format!("container metadata names {owner}"),
                                            group,
                                            admin: false,
                                            root: root.clone(),
                                            launchd: None,
                                        },
                                    );
                                }
                            }
                        }
                        Err(_) => unreadable += 1,
                    }
                    continue;
                }
            }
            if by_name && !names.is_empty() {
                let stem = Path::new(&name)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let n = normalize(if kind_of(&path) == "folder" {
                    &name
                } else {
                    &stem
                });
                if n.chars().count() >= 4 && names.contains(&n) && !other_names.contains(&n) {
                    push(
                        &mut out,
                        Found {
                            kind: kind_of(&path),
                            path,
                            confidence: "low",
                            evidence: format!("same name as the app in {sub} (name only)"),
                            group,
                            admin: false,
                            root: root.clone(),
                            launchd: None,
                        },
                    );
                }
            }
        }
    }

    // Group Containers: app groups, `<TEAM>.<id>`, `group.<id>`, Team ID.
    let gc = lib.join("Group Containers");
    for (name, path) in entries(&gc) {
        if is_apple(&name) || name.to_ascii_lowercase().starts_with("group.com.apple.") {
            continue;
        }
        let rest_after_team = subj
            .team_id
            .as_deref()
            .and_then(|t| name.strip_prefix(t).and_then(|r| r.strip_prefix('.')));
        let (confidence, evidence) = if subj
            .app_groups
            .iter()
            .any(|g| g.eq_ignore_ascii_case(&name))
        {
            ("high", "an app group in the app's signature".to_string())
        } else if let (Some(b), Some(rest)) = (&id, rest_after_team) {
            if lineage(rest, b) {
                ("high", format!("Team ID and bundle id {b}"))
            } else {
                let siblings: Vec<&str> = others
                    .iter()
                    .filter(|o| o.team_id.as_deref() == subj.team_id.as_deref())
                    .map(|o| o.name.as_str())
                    .collect();
                if siblings.is_empty() {
                    ("medium", "same developer (Team ID) as the app".to_string())
                } else {
                    (
                        "low",
                        format!(
                            "same developer (Team ID) as the app, also used by {}",
                            siblings.join(", ")
                        ),
                    )
                }
            }
        } else if let Some(b) = &id {
            match name.strip_prefix("group.") {
                Some(rest) if lineage(rest, b) => ("high", format!("app group named after {b}")),
                _ => continue,
            }
        } else {
            continue;
        };
        push(
            &mut out,
            Found {
                kind: "folder",
                path,
                confidence,
                evidence,
                group: "data",
                admin: false,
                root: gc.clone(),
                launchd: None,
            },
        );
    }

    // launchd jobs.
    let mut launch_dirs = vec![(lib.join("LaunchAgents"), Launchd::UserAgent, false)];
    if let Some(sys) = &roots.system {
        launch_dirs.push((sys.join("LaunchAgents"), Launchd::SystemAgent, true));
        launch_dirs.push((sys.join("LaunchDaemons"), Launchd::Daemon, true));
    }
    for (dir, kind, admin) in launch_dirs {
        for (name, path) in entries(&dir) {
            if !name.ends_with(".plist") || is_apple(&name) {
                continue;
            }
            let job = plist::read(&path)
                .map(|v| parse::launch_job(&v))
                .unwrap_or_default();
            let label = job
                .label
                .clone()
                .unwrap_or_else(|| name.trim_end_matches(".plist").to_string());
            if is_apple(&label) {
                continue;
            }
            let evidence = if let Some(b) = id
                .as_deref()
                .filter(|b| lineage(&label, b) && belongs_to_other(&label, b, others).is_none())
            {
                Some(format!(
                    "launch job label {label} carries the bundle id {b}"
                ))
            } else if let Some(b) = id
                .as_deref()
                .filter(|b| job.associated.iter().any(|a| a.eq_ignore_ascii_case(b)))
            {
                Some(format!("launch job {label} is associated with {b}"))
            } else if let (Some(app), Some(prog)) = (&subj.app_path, &job.program) {
                Path::new(prog)
                    .starts_with(app)
                    .then(|| format!("launch job {label} starts a program inside the app"))
            } else {
                None
            };
            if let Some(evidence) = evidence {
                push(
                    &mut out,
                    Found {
                        path,
                        confidence: "high",
                        evidence,
                        kind: "launchd",
                        group: "launch",
                        admin,
                        root: dir.clone(),
                        launchd: parse::valid_label(&label).then(|| (kind, label)),
                    },
                );
            }
        }
    }

    // System Library, by bundle id only (admin).
    if let (Some(sys), Some(b)) = (&roots.system, &id) {
        for &(sub, group) in SYSTEM_DIRS {
            let root = sys.join(sub);
            for (name, path) in entries(&root) {
                if lineage(&name, b) && belongs_to_other(&name, b, others).is_none() {
                    push(
                        &mut out,
                        Found {
                            kind: kind_of(&path),
                            path,
                            confidence: "high",
                            evidence: format!("named after the bundle id {b} in /Library/{sub}"),
                            group,
                            admin: true,
                            root: root.clone(),
                            launchd: None,
                        },
                    );
                }
            }
        }
    }

    if unreadable > 0 {
        notes.push(Note::UnreadableContainers(unreadable));
    }
    // A path inside another found path goes with its parent.
    let all: Vec<PathBuf> = out.iter().map(|f| f.path.clone()).collect();
    out.retain(|f| !all.iter().any(|p| p != &f.path && f.path.starts_with(p)));
    (out, notes)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    const FX: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/washmybelly_fixtures/apps_macos/"
    );

    fn mk(p: &Path) {
        std::fs::create_dir_all(p).unwrap();
        std::fs::write(p.join("data.bin"), vec![1u8; 4096]).unwrap();
    }

    fn file(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    /// A fake home with the usual places, a UUID container, a sibling app
    /// with a longer bundle id, and a shared Team ID.
    pub fn fake_home(root: &Path) -> Roots {
        let home = root.join("home");
        let lib = home.join("Library");
        mk(&lib.join("Application Support/com.example.editor"));
        mk(&lib.join("Application Support/Example Editor"));
        mk(&lib.join("Application Support/com.example.editor.pro"));
        mk(&lib.join("Application Support/com.apple.something"));
        mk(&lib.join("Caches/com.example.editor"));
        mk(&lib.join("Caches/com.example.editor.ShipIt"));
        file(&lib.join("Preferences/com.example.editor.plist"));
        file(&lib.join(
            "Preferences/ByHost/com.example.editor.0A1B2C3D-0000-4000-8000-00000000ABCD.plist",
        ));
        mk(&lib.join("Saved Application State/com.example.editor.savedState"));
        mk(&lib.join("HTTPStorages/com.example.editor"));
        mk(&lib.join("Containers/com.example.editor.Share"));
        let uuid = lib.join("Containers/6F1C0A52-8C57-4E0B-9D6E-2F7C1B9E4A10");
        mk(&uuid);
        std::fs::copy(
            format!("{FX}container_metadata.bplist"),
            uuid.join(".com.apple.containermanagerd.metadata.plist"),
        )
        .unwrap();
        mk(&lib.join("Group Containers/TEAM123456.com.example.editor"));
        mk(&lib.join("Group Containers/TEAM123456.shared"));
        mk(&lib.join("Group Containers/group.com.example.editor.widgets"));
        std::fs::create_dir_all(lib.join("LaunchAgents")).unwrap();
        std::fs::write(
            lib.join("LaunchAgents/com.example.updater.plist"),
            r#"<?xml version="1.0"?><plist version="1.0"><dict><key>Label</key><string>com.example.updater</string><key>AssociatedBundleIdentifiers</key><array><string>com.example.editor</string></array></dict></plist>"#,
        )
        .unwrap();
        std::fs::write(
            lib.join("LaunchAgents/org.other.agent.plist"),
            r#"<?xml version="1.0"?><plist version="1.0"><dict><key>Label</key><string>org.other.agent</string><key>Program</key><string>/usr/bin/true</string></dict></plist>"#,
        )
        .unwrap();
        let sys = root.join("SysLibrary");
        std::fs::create_dir_all(sys.join("LaunchDaemons")).unwrap();
        std::fs::write(
            sys.join("LaunchDaemons/com.example.editor.helper.plist"),
            r#"<?xml version="1.0"?><plist version="1.0"><dict><key>Label</key><string>com.example.editor.helper</string></dict></plist>"#,
        )
        .unwrap();
        mk(&sys.join("Application Support/com.example.editor"));
        Roots {
            home,
            system: Some(sys),
        }
    }

    fn subject() -> Subject {
        Subject {
            bundle_id: Some("com.example.editor".into()),
            team_id: Some("TEAM123456".into()),
            names: vec!["Example Editor".into()],
            app_path: Some(PathBuf::from("/Applications/Example Editor.app")),
            app_groups: vec![],
        }
    }

    fn others() -> Vec<Other> {
        vec![Other {
            name: "Example Editor Pro".into(),
            path: "/Applications/Example Editor Pro.app".into(),
            bundle_id: Some("com.example.editor.pro".into()),
            team_id: Some("TEAM123456".into()),
        }]
    }

    #[test]
    fn evidence_tiers_on_a_fake_home() {
        let dir = tempfile::tempdir().unwrap();
        let roots = fake_home(dir.path());
        let mut subj = subject();
        // The UUID container's metadata names com.example.sandboxed.
        subj.bundle_id = Some("com.example.editor".into());
        let (found, notes) = find(&roots, &subj, &others());
        let get = |suffix: &str| {
            found
                .iter()
                .find(|f| f.path.to_string_lossy().ends_with(suffix))
        };
        assert_eq!(
            get("Application Support/com.example.editor")
                .unwrap()
                .confidence,
            "high"
        );
        assert!(
            get("Application Support/com.example.editor.pro").is_none(),
            "belongs to the Pro app"
        );
        assert!(get("com.apple.something").is_none());
        assert_eq!(
            get("Application Support/Example Editor")
                .unwrap()
                .confidence,
            "low"
        );
        assert!(get("Caches/com.example.editor.ShipIt").is_some());
        assert_eq!(
            get("Preferences/com.example.editor.plist").unwrap().group,
            "preferences"
        );
        assert!(
            get("ByHost/com.example.editor.0A1B2C3D-0000-4000-8000-00000000ABCD.plist").is_some()
        );
        assert!(get("com.example.editor.savedState").is_some());
        assert!(get("Containers/com.example.editor.Share").is_some());
        assert!(
            get("6F1C0A52-8C57-4E0B-9D6E-2F7C1B9E4A10").is_none(),
            "owned by another id"
        );
        assert_eq!(
            get("TEAM123456.com.example.editor").unwrap().confidence,
            "high"
        );
        assert_eq!(
            get("TEAM123456.shared").unwrap().confidence,
            "low",
            "the Pro app has the same team"
        );
        assert!(get("group.com.example.editor.widgets").is_some());
        let agent = get("LaunchAgents/com.example.updater.plist").unwrap();
        assert_eq!(
            agent.launchd,
            Some((Launchd::UserAgent, "com.example.updater".into()))
        );
        assert!(get("org.other.agent.plist").is_none());
        let daemon = get("LaunchDaemons/com.example.editor.helper.plist").unwrap();
        assert!(daemon.admin);
        assert_eq!(daemon.launchd.as_ref().unwrap().0, Launchd::Daemon);
        assert!(
            get("SysLibrary/Application Support/com.example.editor")
                .unwrap()
                .admin
        );
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn uuid_container_found_by_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let roots = fake_home(dir.path());
        let subj = Subject {
            bundle_id: Some("com.example.sandboxed".into()),
            ..Default::default()
        };
        let (found, _) = find(&roots, &subj, &[]);
        let c = found
            .iter()
            .find(|f| f.path.ends_with("6F1C0A52-8C57-4E0B-9D6E-2F7C1B9E4A10"))
            .unwrap();
        assert_eq!(c.confidence, "high");
        assert!(c.evidence.contains("metadata"));
    }

    #[test]
    fn a_shared_bundle_id_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let roots = fake_home(dir.path());
        let twin = Other {
            name: "Example Editor copy".into(),
            path: "/Users/x/Applications/Example Editor.app".into(),
            bundle_id: Some("com.example.editor".into()),
            team_id: None,
        };
        let (found, notes) = find(&roots, &subject(), &[twin]);
        assert!(found.is_empty());
        assert!(matches!(&notes[0], Note::SharedBundleId(p) if p.len() == 1));
    }

    #[test]
    fn apple_ids_are_never_searched() {
        let dir = tempfile::tempdir().unwrap();
        let roots = fake_home(dir.path());
        let subj = Subject {
            bundle_id: Some("com.apple.Keynote".into()),
            ..Default::default()
        };
        let (found, notes) = find(&roots, &subj, &[]);
        assert!(found.is_empty());
        assert_eq!(notes, vec![Note::Apple]);
    }
}
