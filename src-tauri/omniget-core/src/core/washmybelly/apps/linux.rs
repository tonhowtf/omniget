use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::super::elevate::ElevatedOp;
use super::super::packages::{self, flatpak, RemovePreview, LINUX_MANAGERS};
use super::super::plan::{Op, Outcome, PlanItem};
use super::super::protect::Guard;
use super::super::receipt::Undo;
use super::super::store::Ctx;
use super::super::{fsops, Destination, Risk};
use super::{App, Leftover};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DesktopEntry {
    pub name: String,
    pub exec: String,
    pub try_exec: String,
    pub icon: String,
    pub kind: String,
    pub no_display: bool,
    pub hidden: bool,
}

impl DesktopEntry {
    pub fn visible(&self) -> bool {
        self.kind == "Application" && !self.no_display && !self.hidden && !self.name.is_empty()
    }
}

pub fn parse_desktop(text: &str) -> DesktopEntry {
    let mut e = DesktopEntry::default();
    let mut in_main = false;
    for l in text.lines() {
        let l = l.trim();
        if l.starts_with('[') {
            in_main = l == "[Desktop Entry]";
            continue;
        }
        if !in_main || l.starts_with('#') {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else {
            continue;
        };
        let v = v.trim().to_string();
        match k.trim() {
            "Name" => e.name = v,
            "Exec" => e.exec = v,
            "TryExec" => e.try_exec = v,
            "Icon" => e.icon = v,
            "Type" => e.kind = v,
            "NoDisplay" => e.no_display = v == "true",
            "Hidden" => e.hidden = v == "true",
            _ => {}
        }
    }
    e
}

pub fn exec_program(exec: &str) -> Option<String> {
    let mut rest = exec.trim();
    loop {
        let (tok, tail) = if let Some(q) = rest.strip_prefix('"') {
            let end = q.find('"')?;
            (&q[..end], &q[end + 1..])
        } else {
            match rest.split_once(char::is_whitespace) {
                Some((a, b)) => (a, b),
                None => (rest, ""),
            }
        };
        let tail = tail.trim_start();
        if tok == "env" || (tok.contains('=') && !tok.starts_with('/')) {
            rest = tail;
            if rest.is_empty() {
                return None;
            }
            continue;
        }
        return (!tok.is_empty()).then(|| tok.to_string());
    }
}

pub fn orphan_target(e: &DesktopEntry, exists: &dyn Fn(&Path) -> bool) -> Option<String> {
    if e.kind != "Application" {
        return None;
    }
    [
        e.try_exec.clone(),
        exec_program(&e.exec).unwrap_or_default(),
    ]
    .into_iter()
    .find(|p| p.starts_with('/') && !exists(Path::new(p)))
}

fn desktop_files(dir: &Path) -> Vec<(PathBuf, DesktopEntry)> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<(PathBuf, DesktopEntry)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "desktop") && p.is_file())
        .filter_map(|p| {
            std::fs::read_to_string(&p)
                .ok()
                .map(|t| (p, parse_desktop(&t)))
        })
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

pub fn is_appimage_header(head: &[u8]) -> bool {
    head.len() >= 11
        && &head[..4] == b"\x7fELF"
        && &head[8..10] == b"AI"
        && matches!(head[10], 1 | 2)
}

fn looks_like_appimage(p: &Path) -> bool {
    if p.extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("appimage"))
    {
        return true;
    }
    use std::io::Read;
    let mut head = [0u8; 11];
    std::fs::File::open(p)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && is_appimage_header(&head)
}

pub fn appimage_name(stem: &str) -> String {
    let arch = [
        "x86_64", "x86-64", "amd64", "aarch64", "arm64", "armhf", "i386", "i686", "linux",
        "appimage",
    ];
    let mut words: Vec<&str> = Vec::new();
    for w in stem.split(['-', '_']) {
        let lw = w.to_ascii_lowercase();
        let digit_at = |i: usize| lw[i..].chars().next().is_some_and(|c| c.is_ascii_digit());
        if digit_at(0) || (lw.starts_with('v') && digit_at(1)) {
            break;
        }
        if !arch.contains(&lw.as_str()) {
            words.push(w);
        }
    }
    let s = words.join(" ");
    if s.trim().is_empty() {
        stem.to_string()
    } else {
        s
    }
}

fn appimages(home: &Path, user_desktop: &[(PathBuf, DesktopEntry)]) -> Vec<App> {
    let mut out = Vec::new();
    for dir in ["Applications", "AppImages", ".local/bin", "Downloads"] {
        let Ok(rd) = std::fs::read_dir(home.join(dir)) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(m) = std::fs::symlink_metadata(&p) else {
                continue;
            };
            if !m.is_file() || fsops::is_link(&m) || !looks_like_appimage(&p) {
                continue;
            }
            let ps = p.to_string_lossy().into_owned();
            let companions: Vec<&(PathBuf, DesktopEntry)> = user_desktop
                .iter()
                .filter(|(_, d)| {
                    d.try_exec == ps || exec_program(&d.exec).as_deref() == Some(ps.as_str())
                })
                .collect();
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let name = companions
                .first()
                .map(|(_, d)| d.name.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| appimage_name(&stem));
            out.push(App {
                id: format!("appimage:{ps}"),
                name,
                source: "appimage".into(),
                path: Some(ps.clone()),
                bytes: Some(fsops::allocated(&m)),
                removable: true,
                extra: json!({
                    "desktop_files": companions.iter().map(|(p, _)| p.to_string_lossy()).collect::<Vec<_>>(),
                    "exec": [p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()],
                }),
                ..Default::default()
            });
        }
    }
    out
}

fn orphan_desktops(user_desktop: &[(PathBuf, DesktopEntry)]) -> Vec<App> {
    user_desktop
        .iter()
        .filter_map(|(p, d)| {
            let missing = orphan_target(d, &|x: &Path| x.exists())?;
            Some(App {
                id: format!("desktop:{}", p.display()),
                name: if d.name.is_empty() {
                    p.file_stem()?.to_string_lossy().into_owned()
                } else {
                    d.name.clone()
                },
                source: "desktop_file".into(),
                path: Some(p.to_string_lossy().into_owned()),
                bytes: std::fs::symlink_metadata(p)
                    .ok()
                    .map(|m| fsops::allocated(&m)),
                removable: true,
                reason: Some(format!("its program {missing} is gone")),
                extra: json!({ "missing": missing }),
                ..Default::default()
            })
        })
        .collect()
}

const SYSTEM_MANAGERS: &[&str] = &["apt", "dnf", "pacman", "zypper"];

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/nonexistent-home"))
}

pub fn apps_from_packages(
    pkgs: Vec<App>,
    owners: &HashMap<PathBuf, String>,
    entries: &HashMap<PathBuf, DesktopEntry>,
) -> Vec<App> {
    let mut by_pkg: HashMap<&str, Vec<(&PathBuf, &DesktopEntry)>> = HashMap::new();
    for (path, pkg) in owners {
        if let Some(e) = entries.get(path).filter(|e| e.visible()) {
            by_pkg.entry(pkg.as_str()).or_default().push((path, e));
        }
    }
    pkgs.into_iter()
        .filter_map(|mut a| {
            let mut files = by_pkg.get(a.native_id.as_deref().unwrap_or(""))?.clone();
            files.sort_by(|x, y| x.0.cmp(y.0));
            a.name = files[0].1.name.clone();
            a.extra["desktop_files"] = json!(files
                .iter()
                .map(|(p, _)| p.to_string_lossy())
                .collect::<Vec<_>>());
            a.extra["exec"] = json!(files
                .iter()
                .filter_map(|(_, e)| exec_program(&e.exec))
                .map(|p| Path::new(&p)
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or(p))
                .collect::<Vec<_>>());
            a.extra["icon"] = json!(files[0].1.icon);
            Some(a)
        })
        .collect()
}

pub fn list() -> Result<Vec<App>, String> {
    if crate::core::dependencies::is_flatpak() {
        return Ok(Vec::new());
    }
    let backends = packages::available();
    let total = backends.len() as u64 + 1;
    let system_dir = Path::new("/usr/share/applications");
    let entries: HashMap<PathBuf, DesktopEntry> = desktop_files(system_dir).into_iter().collect();
    let visible: Vec<PathBuf> = entries
        .iter()
        .filter(|(_, e)| e.visible())
        .map(|(p, _)| p.clone())
        .collect();
    let mut out: Vec<App> = Vec::new();
    for (i, b) in backends.iter().enumerate() {
        super::super::progress("apps_list", "scan", i as u64, total, b.id());
        match b.id() {
            m if SYSTEM_MANAGERS.contains(&m) => {
                let Ok(pkgs) = b.list() else { continue };
                let owners = b.desktop_owners(&visible);
                let mut apps = apps_from_packages(pkgs, &owners, &entries);
                for a in &mut apps {
                    a.extra["orphan"] = json!(false);
                }
                let known: HashSet<String> = apps.iter().map(|a| a.id.clone()).collect();
                for mut o in b.orphans().unwrap_or_default() {
                    if known.contains(&o.id) {
                        if let Some(a) = apps.iter_mut().find(|a| a.id == o.id) {
                            a.extra["orphan"] = json!(true);
                        }
                        continue;
                    }
                    o.extra["orphan"] = json!(true);
                    o.reason = Some("nothing needs it anymore".into());
                    apps.push(o);
                }
                out.extend(apps);
            }
            "flatpak" => {
                let Ok(fl) = flatpak::installed() else {
                    continue;
                };
                let mut seen = HashSet::new();
                for f in &fl {
                    let id = if seen.insert(f.id.clone()) {
                        format!("flatpak:{}", f.id)
                    } else {
                        format!("flatpak:{}:{}", f.installation, f.id)
                    };
                    out.push(flatpak::to_app(f, id));
                }
                if let Ok(orphans) = b.orphans() {
                    out.extend(orphans);
                }
            }
            _ => out.extend(b.list().unwrap_or_default()),
        }
    }
    super::super::progress("apps_list", "scan", total - 1, total, "AppImage");
    let h = home();
    let user_desktop = desktop_files(&h.join(".local/share/applications"));
    out.extend(appimages(&h, &user_desktop));
    out.extend(orphan_desktops(&user_desktop));
    super::super::progress("apps_list", "done", total, total, "");
    Ok(out)
}

const SHARED: &[&str] = &[
    "autostart",
    "gtk-2.0",
    "gtk-3.0",
    "gtk-4.0",
    "dconf",
    "systemd",
    "pulse",
    "pipewire",
    "wireplumber",
    "fontconfig",
    "menus",
    "applications",
    "icons",
    "fonts",
    "trash",
    "keyrings",
    "gnupg",
    "flatpak",
    "snap",
    "containers",
    "environment.d",
    "xdg-desktop-portal",
    "mime",
    "mimeapps.list",
    "user-dirs.dirs",
    "user-dirs.locale",
    "recently-used.xbel",
    "gvfs-metadata",
    "tracker",
    "tracker3",
    "ibus",
    "fcitx",
    "fcitx5",
    "gnome-session",
    "session",
    "sessions",
    "thumbnails",
    "nvim",
    "vim",
    "kdedefaults",
    "kde",
    "kde.org",
    "plasma-workspace",
    "gnome-shell",
    "mozilla",
    "desktop-directories",
    "sounds",
    "themes",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub path: PathBuf,
    pub base: PathBuf,
    pub confidence: &'static str,
    pub evidence: String,
}

pub fn clues(app: &App) -> Vec<(String, &'static str, String)> {
    let mut v: Vec<(String, &'static str, String)> = Vec::new();
    let native = app.native_id.clone().unwrap_or_default();
    let exec: Vec<String> = app.extra["exec"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    for f in app.extra["desktop_files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if let Some(stem) = Path::new(f)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
        {
            let sure = if stem.matches('.').count() >= 2 {
                "high"
            } else {
                "medium"
            };
            v.push((
                stem.clone(),
                sure,
                format!("same name as its menu entry {stem}.desktop"),
            ));
        }
    }
    if app.source == "flatpak" && !native.is_empty() {
        let id = native.split("//").next().unwrap_or(&native).to_string();
        v.push((id.clone(), "high", format!("the Flatpak id {id}")));
    }
    if !native.is_empty() && app.source != "flatpak" && native.len() >= 3 {
        let sure = if exec.iter().any(|e| e.eq_ignore_ascii_case(&native)) {
            "high"
        } else {
            "medium"
        };
        v.push((
            native.clone(),
            sure,
            format!("same name as the package {native}"),
        ));
    }
    for e in &exec {
        if e.len() >= 3 && !e.contains(".AppImage") && !e.contains(".appimage") {
            v.push((e.clone(), "medium", format!("same name as its program {e}")));
        }
    }
    let norm: String = app
        .name
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '.')
        .collect();
    if norm.len() >= 3 {
        let sure = if app.source == "removed" {
            "medium"
        } else {
            "low"
        };
        v.push((norm, sure, format!("same name as {}", app.name)));
    }
    v
}

pub fn find_candidates(home: &Path, app: &App) -> Vec<Candidate> {
    let bases = [".config", ".local/share", ".cache"];
    let clues = clues(app);
    let rank = |c: &str| match c {
        "high" => 3,
        "medium" => 2,
        _ => 1,
    };
    let mut best: HashMap<PathBuf, Candidate> = HashMap::new();
    let mut offer = |path: PathBuf, base: PathBuf, confidence: &'static str, evidence: String| {
        let keep = best
            .get(&path)
            .is_none_or(|c| rank(c.confidence) < rank(confidence));
        if keep {
            best.insert(
                path.clone(),
                Candidate {
                    path,
                    base,
                    confidence,
                    evidence,
                },
            );
        }
    };
    for b in bases {
        let base = home.join(b);
        let Ok(rd) = std::fs::read_dir(&base) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let lower = name.to_lowercase();
            if SHARED.contains(&lower.as_str()) {
                continue;
            }
            for (clue, sure, why) in &clues {
                if lower == clue.to_lowercase() {
                    offer(e.path(), base.clone(), sure, why.clone());
                }
            }
        }
    }
    let var = home.join(".var/app");
    for (clue, _, why) in clues.iter().filter(|(c, _, _)| c.matches('.').count() >= 2) {
        let p = var.join(clue);
        if p.is_dir() && (app.source == "flatpak" || app.source == "removed") {
            offer(p, var.clone(), "high", format!("{why} (its sandbox data)"));
        }
    }
    let mut v: Vec<Candidate> = best.into_values().collect();
    v.sort_by(|a, b| a.path.cmp(&b.path));
    v
}

const CRITICAL: &[&str] = &[
    "base",
    "filesystem",
    "linux",
    "kernel",
    "kernel-core",
    "systemd",
    "glibc",
    "libc6",
    "bash",
    "coreutils",
    "sudo",
    "polkit",
    "dbus",
    "dbus-broker",
    "network-manager",
    "NetworkManager",
    "networkmanager",
    "gnome-shell",
    "mutter",
    "plasma-workspace",
    "plasma-desktop",
    "kwin",
    "kwin-wayland",
    "kwin-x11",
    "xorg-server",
    "xserver-xorg-core",
    "gdm",
    "gdm3",
    "sddm",
    "lightdm",
    "shim",
    "apt",
    "dpkg",
    "dnf",
    "dnf5",
    "rpm",
    "pacman",
    "zypper",
    "snapd",
    "flatpak",
    "ubuntu-desktop",
    "ubuntu-minimal",
    "ubuntu-standard",
    "cinnamon",
    "xfce4-session",
    "gnome-session",
];

pub fn is_critical(name: &str) -> bool {
    CRITICAL.contains(&name)
        || [
            "linux-image-",
            "kernel-",
            "grub",
            "systemd-",
            "linux-firmware",
        ]
        .iter()
        .any(|p| name.starts_with(p))
}

pub fn removal_risk(p: &RemovePreview) -> Risk {
    let all = p.removes.iter().chain(p.also_removes.iter());
    if p.also_removes.len() > 25
        || all
            .clone()
            .any(|n| is_critical(n.split("//").next().unwrap_or(n)))
    {
        Risk::Extreme
    } else {
        Risk::Destructive
    }
}

pub fn preview_detail(p: &RemovePreview) -> String {
    let mut s = if p.also_removes.is_empty() {
        "Nothing else is removed with it.".to_string()
    } else {
        let shown: Vec<&str> = p.also_removes.iter().take(15).map(String::as_str).collect();
        let more = p.also_removes.len() - shown.len();
        format!(
            "Also removes {} package{}: {}{}",
            p.also_removes.len(),
            if p.also_removes.len() == 1 { "" } else { "s" },
            shown.join(", "),
            if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            }
        )
    };
    if !p.exact {
        s.push_str(" (the manager could only give a partial answer without administrator rights)");
    }
    s.push_str(" No undo.");
    s
}

pub fn package_item(
    app: &App,
    manager: &str,
    name: &str,
    preview: &RemovePreview,
    user_flatpak: bool,
) -> PlanItem {
    let (action, args) = if user_flatpak {
        (
            "linux.flatpak_remove",
            json!({ "names": [name], "also": preview.also_removes }),
        )
    } else {
        (
            "linux.remove",
            json!({ "manager": manager, "names": [name], "also": preview.also_removes }),
        )
    };
    PlanItem {
        id: format!("uninstall:{}:package", app.id),
        label: format!("Remove {} ({manager})", app.name),
        detail: Some(preview_detail(preview)),
        group: "app".into(),
        bytes: preview.frees_bytes.or(app.bytes).unwrap_or(0),
        risk: removal_risk(preview),
        admin: !user_flatpak,
        op: Op::Module {
            module: "apps".into(),
            action: action.into(),
            args,
        },
    }
}

fn trash_item(
    app: &App,
    path: &Path,
    what: &str,
    group: &str,
    detail: Option<String>,
) -> Result<PlanItem, String> {
    let root = path.parent().map(Path::to_path_buf);
    super::super::protect::check(path, Guard::Picked, root.as_deref())?;
    Ok(PlanItem {
        id: format!("uninstall:{}:{what}:{}", app.id, path.display()),
        label: format!(
            "Move {} to the Trash",
            path.file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default()
        ),
        detail,
        group: group.into(),
        bytes: fsops::measure(path).0,
        risk: Risk::Reversible,
        admin: false,
        op: Op::RemovePath {
            path: path.to_path_buf(),
            destination: Destination::Trash,
            fp: fsops::fingerprint(path)?,
            root,
            guard: Guard::Picked,
            contents_only: false,
        },
    })
}

fn process_names() -> HashSet<String> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    let mut out = HashSet::new();
    for p in sys.processes().values() {
        out.insert(p.name().to_string_lossy().to_lowercase());
        if let Some(exe) = p.exe().and_then(|e| e.file_name()) {
            out.insert(exe.to_string_lossy().to_lowercase());
        }
    }
    out
}

fn running(app: &App) -> bool {
    if app.source == "flatpak" {
        let id = app.native_id.clone().unwrap_or_default();
        return packages::query("flatpak", &["ps", "--columns=application"])
            .is_ok_and(|r| r.stdout.lines().any(|l| l.trim() == id));
    }
    let mut names: Vec<String> = app.extra["exec"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_lowercase))
                .collect()
        })
        .unwrap_or_default();
    if let Some(n) = &app.native_id {
        names.push(n.to_lowercase());
    }
    if names.is_empty() {
        return false;
    }
    let procs = process_names();
    names
        .iter()
        .any(|n| procs.contains(n) || (n.len() > 15 && procs.contains(&n[..15])))
}

struct Built {
    ticked: Vec<PlanItem>,
    unticked: Vec<(PlanItem, &'static str)>,
    leftovers: Vec<Leftover>,
    notes: Vec<String>,
    preview: Option<RemovePreview>,
}

type PreviewFn<'a> = dyn Fn(&str, &str, &[String]) -> Result<RemovePreview, String> + 'a;

fn build(app: &App, home: &Path, preview: &PreviewFn<'_>) -> Built {
    let mut b = Built {
        ticked: Vec::new(),
        unticked: Vec::new(),
        leftovers: Vec::new(),
        notes: Vec::new(),
        preview: None,
    };
    let native = app.native_id.clone().unwrap_or_default();
    let installation = app.extra["installation"]
        .as_str()
        .unwrap_or("system")
        .to_string();
    match app.source.as_str() {
        m if SYSTEM_MANAGERS.contains(&m) || m == "snap" || m == "flatpak" => {
            let user_flatpak = m == "flatpak" && installation == "user";
            if m == "flatpak" && !matches!(installation.as_str(), "user" | "system") {
                b.notes.push(format!("{} is in the Flatpak installation {installation}; remove it with flatpak directly.", app.name));
            } else if !packages::valid_name(m, &native) {
                b.notes.push(format!(
                    "{native:?} is not a package name OmniGet can hand to {m}."
                ));
            } else {
                match preview(m, &installation, std::slice::from_ref(&native)) {
                    Ok(p) => {
                        b.ticked
                            .push(package_item(app, m, &native, &p, user_flatpak));
                        b.preview = Some(p);
                    }
                    Err(e) => b
                        .notes
                        .push(format!("{m} will not remove {}: {e}", app.name)),
                }
            }
        }
        "appimage" => {
            if let Some(p) = app.path.as_deref().map(PathBuf::from) {
                match trash_item(app, &p, "file", "app", Some(p.display().to_string())) {
                    Ok(it) => b.ticked.push(it),
                    Err(e) => b.notes.push(e),
                }
            }
            for d in app.extra["desktop_files"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if let Ok(it) = trash_item(app, Path::new(d), "desktop", "app", Some(d.to_string()))
                {
                    b.ticked.push(it);
                }
            }
        }
        "desktop_file" => {
            if let Some(p) = app.path.as_deref() {
                match trash_item(app, Path::new(p), "desktop", "app", Some(p.to_string())) {
                    Ok(it) => b.ticked.push(it),
                    Err(e) => b.notes.push(e),
                }
            }
        }
        _ => {}
    }
    for c in find_candidates(home, app) {
        let Ok(mut it) = trash_item(
            app,
            &c.path,
            "leftover",
            "leftovers",
            Some(c.path.display().to_string()),
        ) else {
            continue;
        };
        it.op = match it.op {
            Op::RemovePath {
                path,
                destination,
                fp,
                guard,
                contents_only,
                ..
            } => Op::RemovePath {
                path,
                destination,
                fp,
                root: Some(c.base.clone()),
                guard,
                contents_only,
            },
            other => other,
        };
        it.label = format!(
            "{} ({})",
            c.path
                .strip_prefix(home)
                .map(|p| format!("~/{}", p.display()))
                .unwrap_or_else(|_| c.path.display().to_string()),
            c.confidence
        );
        b.leftovers.push(Leftover {
            path: c.path.display().to_string(),
            bytes: it.bytes,
            confidence: c.confidence.into(),
            evidence: c.evidence.clone(),
            kind: if c.path.is_dir() {
                "folder".into()
            } else {
                "file".into()
            },
        });
        match c.confidence {
            "high" => b.ticked.push(it),
            "medium" => b.unticked.push((it, "medium")),
            _ => b.unticked.push((it, "low")),
        }
    }
    b
}

fn real_preview(
    manager: &str,
    installation: &str,
    names: &[String],
) -> Result<RemovePreview, String> {
    if manager == "flatpak" {
        return flatpak::preview_in(installation, names);
    }
    packages::backend(manager)
        .ok_or("unknown manager")?
        .remove_preview(names)
}

pub fn uninstall_items(ctx: &Ctx, app: &App) -> Result<(Vec<PlanItem>, Value), String> {
    let _ = ctx;
    if crate::core::dependencies::is_flatpak() {
        return Err("washmybelly.reason.flatpak_sandbox".into());
    }
    let is_running = app.source != "removed" && running(app);
    let mut b = build(app, &home(), &real_preview);
    if is_running {
        b.notes.push(format!(
            "{} is open. Quit it first; its items stay unticked until then.",
            app.name
        ));
        let moved: Vec<PlanItem> = std::mem::take(&mut b.ticked);
        b.unticked
            .extend(moved.into_iter().map(|it| (it, "running")));
    }
    let unticked: Vec<PlanItem> = b.unticked.iter().map(|(it, _)| it.clone()).collect();
    super::super::plan::register("uninstall", &unticked);
    let extra = json!({
        "leftovers": b.leftovers,
        "notes": b.notes,
        "running": is_running,
        "preview": b.preview,
        "unticked": b.unticked.iter().map(|(x, why)| json!({
            "id": x.id, "label": x.label, "detail": x.detail, "group": x.group,
            "bytes": x.bytes, "risk": x.risk, "admin": x.admin, "why": why,
        })).collect::<Vec<_>>(),
    });
    Ok((b.ticked, extra))
}

fn names_of(args: &Value) -> Vec<String> {
    args.get("names")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

pub fn plan_execute(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<Outcome, String> {
    let _ = (ctx, receipt_id);
    match action {
        "flatpak_remove" => {
            let names = names_of(args);
            packages::validate("flatpak", &names, &["flatpak"])?;
            let now = flatpak::preview_in("user", &names)?;
            check_same(&now, args)?;
            Ok(Outcome {
                detail: flatpak::remove_user(&names)?,
                bytes: now.frees_bytes.unwrap_or(0),
                undo: None,
            })
        }
        "remove" => Err("apps.linux: remove runs as administrator".into()),
        _ => Err(format!("apps.linux: unknown action {action}")),
    }
}

fn check_same(now: &RemovePreview, args: &Value) -> Result<(), String> {
    let approved: HashSet<&str> = args
        .get("also")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let extra: Vec<&String> = now
        .also_removes
        .iter()
        .filter(|n| !approved.contains(n.as_str()))
        .collect();
    if extra.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "CHANGED: the removal would now also take {}; scan again",
            extra
                .iter()
                .take(8)
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

pub fn plan_elevated(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    let _ = (ctx, receipt_id);
    if action != "remove" {
        return Err(format!(
            "apps.linux: action {action} has no administrator form"
        ));
    }
    let manager = args.get("manager").and_then(Value::as_str).unwrap_or("");
    let names = names_of(args);
    packages::validate(manager, &names, LINUX_MANAGERS)?;
    let now = real_preview(manager, "system", &names)?;
    check_same(&now, args)?;
    Ok((
        vec![ElevatedOp::PackageRemove {
            manager: manager.to_string(),
            names,
        }],
        None,
    ))
}

pub fn plan_undo(ctx: &Ctx, action: &str, args: &Value) -> Result<String, String> {
    let _ = (ctx, args);
    Err(format!(
        "apps.linux: {action} cannot be undone (reinstall it from the package manager)"
    ))
}
