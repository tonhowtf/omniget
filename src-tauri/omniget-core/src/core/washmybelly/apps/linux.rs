//! Apps on Linux (F3): system packages that ship a `.desktop` file (apps,
//! not libraries), packages nothing needs anymore, snaps, flatpaks,
//! AppImages and `.desktop` files whose program is gone. Leftovers in
//! `~/.config`, `~/.local/share`, `~/.cache` and `~/.var/app` with a
//! confidence grade.
//!
//! Removing a system package (apt, dnf, pacman, zypper, snap, flatpak in the
//! system installation) is an administrator item: `linux.remove`, which the
//! helper runs as `packages::elevated_remove`. Its detail says what else the
//! manager takes along (the preview), and the item is refused at execution if
//! that set grew since the person saw it. Flatpaks of the user installation
//! go without administrator (`linux.flatpak_remove`); AppImages and their
//! `.desktop` go to the Trash.
//!
//! Inside OmniGet's own Flatpak nothing is listed: the host's managers are
//! out of reach (`caps` already says why).

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

// ── .desktop files ───────────────────────────────────────────────────

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
    /// Shown in a menu.
    pub fn visible(&self) -> bool {
        self.kind == "Application" && !self.no_display && !self.hidden && !self.name.is_empty()
    }
}

/// The `[Desktop Entry]` group of a `.desktop` file (untranslated keys).
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

/// The program an `Exec=` line starts (quotes removed, `env VAR=x` skipped).
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

/// When a `.desktop` file points at an absolute program that is gone,
/// the missing path.
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

// ── AppImages ────────────────────────────────────────────────────────

/// ELF with the AppImage magic `AI\x01`/`AI\x02` at offset 8.
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

/// `Obsidian-1.6.7-x86_64` → `Obsidian`.
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

// ── Inventory ────────────────────────────────────────────────────────

const SYSTEM_MANAGERS: &[&str] = &["apt", "dnf", "pacman", "zypper"];

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/nonexistent-home"))
}

/// Attach the `.desktop` files a package owns and keep only the packages
/// that own a visible one (apps). Pure, for tests.
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

// ── Leftovers ────────────────────────────────────────────────────────

/// Folder names many apps share: never offered as anyone's leftover.
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

/// One leftover candidate before measuring.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub path: PathBuf,
    pub base: PathBuf,
    pub confidence: &'static str,
    pub evidence: String,
}

/// What the app can be recognised by, with how sure each clue is.
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
    // Desktop ids: reverse-DNS ones are unique.
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

/// Folders in the usual places whose name matches a clue. Pure over `home`.
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
    // Flatpak's per-app data folder.
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

// ── Uninstall items ──────────────────────────────────────────────────

/// Packages whose removal would take the desktop, the boot or the package
/// manager with it: such a removal is `extreme` (screen only).
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

/// Risk of a package removal from its preview.
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

/// The item that removes a system package (admin) or a user flatpak.
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

/// Whether the app is open now (by its programs' names; flatpaks by
/// `flatpak ps`).
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
    // Linux truncates process names to 15 bytes.
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

/// Asks a package manager what removing a package would take:
/// (manager, installation, names).
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

// ── Plan hooks (`apps.linux.<action>`) ───────────────────────────────

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

/// Without administrator: flatpaks of the user installation.
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

/// Refuse when the manager would now take more than the person saw.
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

/// The administrator half: `linux.remove` becomes `PackageRemove`, after the
/// preview is taken again and compared with what the person approved.
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

pub async fn ui_action(action: &str, a: Value) -> Result<Value, String> {
    let _ = a;
    Err(format!("apps.linux: unknown screen action {action}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx(name: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/washmybelly_fixtures/packages")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn desktop_entries() {
        let g = parse_desktop(&fx("desktop/org.gimp.GIMP.desktop"));
        assert_eq!(g.name, "GNU Image Manipulation Program");
        assert_eq!(exec_program(&g.exec).as_deref(), Some("gimp-2.10"));
        assert!(g.visible());
        let h = parse_desktop(&fx("desktop/hidden.desktop"));
        assert!(!h.visible());
        let o = parse_desktop(&fx("desktop/appimagekit_0a1b2c-Obsidian.desktop"));
        assert_eq!(
            exec_program(&o.exec).as_deref(),
            Some("/home/u/Applications/Obsidian-1.6.7.AppImage")
        );
        assert_eq!(
            orphan_target(&o, &|_| false).as_deref(),
            Some("/home/u/Applications/Obsidian-1.6.7.AppImage")
        );
        assert_eq!(orphan_target(&o, &|_| true), None);
        assert_eq!(
            orphan_target(&g, &|_| false),
            None,
            "a bare command is not judged"
        );
        assert_eq!(
            exec_program("env GDK_BACKEND=x11 /opt/app/run %U").as_deref(),
            Some("/opt/app/run")
        );
    }

    #[test]
    fn appimage_magic_and_names() {
        let mut h = b"\x7fELF\x02\x01\x01\x00AI\x02".to_vec();
        assert!(is_appimage_header(&h));
        h[10] = 9;
        assert!(!is_appimage_header(&h));
        assert!(!is_appimage_header(b"\x7fELF\x02\x01\x01\x00\x00\x00\x00"));
        assert_eq!(appimage_name("Obsidian-1.6.7"), "Obsidian");
        assert_eq!(appimage_name("Joplin-3.0.15-x86_64"), "Joplin");
        assert_eq!(appimage_name("balenaEtcher-1.19.21-x64"), "balenaEtcher");
    }

    #[test]
    fn only_packages_with_a_menu_entry_are_apps() {
        let pkgs = vec![
            packages::package_app("apt", "gimp", "2.10", Some(10), "editor", None),
            packages::package_app("apt", "libfoo1", "1", Some(1), "lib", None),
            packages::package_app("apt", "helper", "1", Some(1), "", None),
        ];
        let gimp = PathBuf::from("/usr/share/applications/gimp.desktop");
        let hidden = PathBuf::from("/usr/share/applications/helper.desktop");
        let owners = HashMap::from([
            (gimp.clone(), "gimp".to_string()),
            (hidden.clone(), "helper".to_string()),
        ]);
        let entries = HashMap::from([
            (gimp, parse_desktop(&fx("desktop/org.gimp.GIMP.desktop"))),
            (hidden, parse_desktop(&fx("desktop/hidden.desktop"))),
        ]);
        let apps = apps_from_packages(pkgs, &owners, &entries);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "GNU Image Manipulation Program");
        assert_eq!(apps[0].id, "apt:gimp");
        assert_eq!(apps[0].extra["exec"], json!(["gimp-2.10"]));
        assert!(apps[0].needs_admin);
    }

    fn fake_home() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for p in [
            ".config/vlc",
            ".config/VLC media player",
            ".config/autostart",
            ".local/share/vlc",
            ".cache/vlc",
            ".var/app/org.videolan.VLC/config",
            ".config/org.gimp.GIMP",
            ".config/GIMP",
        ] {
            std::fs::create_dir_all(d.path().join(p)).unwrap();
            std::fs::write(d.path().join(p).join("x"), b"1234").unwrap();
        }
        d
    }

    fn vlc_apt() -> App {
        let mut a = packages::package_app("apt", "vlc", "3", Some(100), "", None);
        a.name = "VLC media player".into();
        a.extra["exec"] = json!(["vlc"]);
        a.extra["desktop_files"] = json!(["/usr/share/applications/vlc.desktop"]);
        a
    }

    #[test]
    fn leftovers_have_a_confidence() {
        let home = fake_home();
        let c = find_candidates(home.path(), &vlc_apt());
        let conf: HashMap<String, &str> = c
            .iter()
            .map(|c| {
                (
                    c.path
                        .strip_prefix(home.path())
                        .unwrap()
                        .display()
                        .to_string(),
                    c.confidence,
                )
            })
            .collect();
        assert_eq!(
            conf[".config/vlc"], "high",
            "package name = its program name"
        );
        assert_eq!(conf[".local/share/vlc"], "high");
        assert_eq!(conf[".cache/vlc"], "high");
        assert!(!conf.contains_key(".config/autostart"));
        assert!(
            !conf.contains_key(".var/app/org.videolan.VLC"),
            "a deb's leftovers are not the flatpak's"
        );

        let fl = App {
            id: "flatpak:org.videolan.VLC".into(),
            name: "VLC".into(),
            source: "flatpak".into(),
            native_id: Some("org.videolan.VLC".into()),
            removable: true,
            extra: json!({ "installation": "user" }),
            ..Default::default()
        };
        let c = find_candidates(home.path(), &fl);
        assert!(c
            .iter()
            .any(|c| c.path.ends_with(".var/app/org.videolan.VLC") && c.confidence == "high"));
        let gimp = App {
            id: "apt:gimp".into(),
            name: "GNU Image Manipulation Program".into(),
            source: "apt".into(),
            native_id: Some("gimp".into()),
            extra: json!({ "desktop_files": ["/usr/share/applications/org.gimp.GIMP.desktop"], "exec": ["gimp-2.10"] }),
            ..Default::default()
        };
        let c = find_candidates(home.path(), &gimp);
        let g: HashMap<String, &str> = c
            .iter()
            .map(|c| {
                (
                    c.path.file_name().unwrap().to_string_lossy().into_owned(),
                    c.confidence,
                )
            })
            .collect();
        assert_eq!(g["org.gimp.GIMP"], "high");
        assert_eq!(
            g["GIMP"], "medium",
            "package name without a matching program"
        );
    }

    #[test]
    fn a_removed_app_is_searched_by_name() {
        let home = fake_home();
        let gone = App {
            id: "removed:vlc".into(),
            name: "vlc".into(),
            source: "removed".into(),
            ..Default::default()
        };
        let c = find_candidates(home.path(), &gone);
        assert!(c
            .iter()
            .any(|c| c.path.ends_with(".config/vlc") && c.confidence == "medium"));
        let b = build(&gone, home.path(), &|_, _, _| {
            panic!("no preview for a removed app")
        });
        assert!(b.ticked.is_empty());
        assert_eq!(b.unticked.len(), 3);
        assert!(b
            .unticked
            .iter()
            .all(|(it, _)| it.detail.as_deref() == Some(it_path(it).as_str())));
    }

    fn it_path(it: &PlanItem) -> String {
        match &it.op {
            Op::RemovePath { path, .. } => path.display().to_string(),
            _ => String::new(),
        }
    }

    #[test]
    fn system_package_item_carries_the_preview() {
        let home = fake_home();
        let preview = |m: &str, _: &str, names: &[String]| -> Result<RemovePreview, String> {
            assert_eq!(m, "apt");
            Ok(RemovePreview {
                removes: names.to_vec(),
                also_removes: vec!["vlc-plugin-base".into(), "libvlc5".into()],
                frees_bytes: Some(5000),
                exact: true,
            })
        };
        let b = build(&vlc_apt(), home.path(), &preview);
        let pkg = &b.ticked[0];
        assert_eq!(pkg.group, "app");
        assert!(pkg.admin);
        assert_eq!(pkg.risk, Risk::Destructive);
        assert_eq!(pkg.bytes, 5000);
        assert!(pkg.detail.as_deref().unwrap().contains("vlc-plugin-base"));
        let Op::Module {
            module,
            action,
            args,
        } = &pkg.op
        else {
            panic!()
        };
        assert_eq!((module.as_str(), action.as_str()), ("apps", "linux.remove"));
        assert_eq!(args["manager"], "apt");
        assert_eq!(args["names"], json!(["vlc"]));
        // Leftovers: the three high ones ticked, detail = path, group leftovers.
        let lo: Vec<&PlanItem> = b.ticked.iter().filter(|i| i.group == "leftovers").collect();
        assert_eq!(lo.len(), 3);
        assert!(lo.iter().all(
            |i| i.detail.as_deref() == Some(it_path(i).as_str()) && i.risk == Risk::Reversible
        ));
        assert_eq!(b.leftovers.len(), 3 + b.unticked.len());
        let Op::RemovePath {
            root,
            destination,
            guard,
            ..
        } = &lo[0].op
        else {
            panic!()
        };
        assert!(root.as_ref().unwrap().starts_with(home.path()));
        assert_eq!((*destination, *guard), (Destination::Trash, Guard::Picked));
    }

    #[test]
    fn user_flatpak_needs_no_admin_and_system_does() {
        let home = fake_home();
        let preview = |m: &str, inst: &str, names: &[String]| -> Result<RemovePreview, String> {
            assert_eq!((m, inst), ("flatpak", "user"));
            Ok(RemovePreview {
                removes: names.to_vec(),
                also_removes: vec!["org.videolan.VLC.Locale//stable".into()],
                frees_bytes: None,
                exact: true,
            })
        };
        let mut fl = App {
            id: "flatpak:org.videolan.VLC".into(),
            name: "VLC".into(),
            source: "flatpak".into(),
            native_id: Some("org.videolan.VLC".into()),
            bytes: Some(9),
            removable: true,
            extra: json!({ "installation": "user" }),
            ..Default::default()
        };
        let b = build(&fl, home.path(), &preview);
        assert!(!b.ticked[0].admin);
        let Op::Module { action, .. } = &b.ticked[0].op else {
            panic!()
        };
        assert_eq!(action, "linux.flatpak_remove");
        fl.extra["installation"] = json!("system");
        let b = build(&fl, home.path(), &|_, inst, names| {
            assert_eq!(inst, "system");
            Ok(RemovePreview {
                removes: names.to_vec(),
                ..Default::default()
            })
        });
        assert!(b.ticked[0].admin);
    }

    #[test]
    fn critical_removals_are_screen_only() {
        let p = RemovePreview {
            removes: vec!["foo".into()],
            also_removes: vec!["gnome-shell".into()],
            frees_bytes: None,
            exact: true,
        };
        assert_eq!(removal_risk(&p), Risk::Extreme);
        let p = RemovePreview {
            removes: vec!["foo".into()],
            also_removes: (0..30).map(|i| format!("p{i}")).collect(),
            frees_bytes: None,
            exact: true,
        };
        assert_eq!(removal_risk(&p), Risk::Extreme);
        assert!(is_critical("linux-image-6.8.0-45-generic"));
        assert!(!is_critical("vlc"));
    }

    #[test]
    fn a_grown_removal_is_refused() {
        let now = RemovePreview {
            removes: vec!["vlc".into()],
            also_removes: vec!["libvlc5".into(), "surprise".into()],
            frees_bytes: None,
            exact: true,
        };
        assert!(check_same(&now, &json!({ "also": ["libvlc5", "surprise"] })).is_ok());
        assert!(check_same(&now, &json!({ "also": ["libvlc5"] }))
            .unwrap_err()
            .starts_with("CHANGED"));
    }

    #[test]
    fn elevated_hook_validates_first() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        assert!(plan_elevated(
            &ctx,
            "r",
            "remove",
            &json!({ "manager": "apt", "names": ["-y"] })
        )
        .unwrap_err()
        .starts_with("BAD_NAME"));
        assert!(plan_elevated(
            &ctx,
            "r",
            "remove",
            &json!({ "manager": "winget", "names": ["Git.Git"] })
        )
        .unwrap_err()
        .starts_with("BAD_MANAGER"));
        assert!(plan_elevated(&ctx, "r", "flatpak_remove", &json!({})).is_err());
        assert!(
            plan_execute(&ctx, "r", "flatpak_remove", &json!({ "names": ["x;y"] }))
                .unwrap_err()
                .starts_with("BAD_NAME")
        );
    }

    #[test]
    fn appimage_and_its_desktop_go_to_the_trash() {
        let home = tempfile::tempdir().unwrap();
        let apps_dir = home.path().join("Apps");
        std::fs::create_dir_all(&apps_dir).unwrap();
        let img = apps_dir.join("Tool-1.0.AppImage");
        std::fs::write(&img, b"\x7fELF\x02\x01\x01\x00AI\x02rest").unwrap();
        let desk_dir = home.path().join("desk");
        std::fs::create_dir_all(&desk_dir).unwrap();
        let desk = desk_dir.join("tool.desktop");
        std::fs::write(
            &desk,
            format!(
                "[Desktop Entry]\nType=Application\nName=Tool\nExec={} %U\n",
                img.display()
            ),
        )
        .unwrap();
        let user_desktop = desktop_files(&desk_dir);
        let found = {
            // appimages() looks in fixed folders; point one at ours.
            let fake = home.path().join("Applications");
            std::fs::rename(&apps_dir, &fake).unwrap();
            let img2 = fake.join("Tool-1.0.AppImage");
            std::fs::write(
                &desk,
                format!(
                    "[Desktop Entry]\nType=Application\nName=Tool\nExec={} %U\n",
                    img2.display()
                ),
            )
            .unwrap();
            appimages(home.path(), &desktop_files(&desk_dir))
        };
        let _ = user_desktop;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Tool");
        let b = build(&found[0], home.path(), &|_, _, _| panic!());
        assert_eq!(b.ticked.len(), 2);
        assert!(b.ticked.iter().all(|i| i.group == "app"
            && matches!(
                i.op,
                Op::RemovePath {
                    destination: Destination::Trash,
                    ..
                }
            )));
    }
}
