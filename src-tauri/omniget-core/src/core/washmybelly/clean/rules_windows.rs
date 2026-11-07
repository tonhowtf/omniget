use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::rule::{Action, Requirement, Rule};
use crate::core::washmybelly::{Destination, Os, Risk};

pub const ALLOWED_ROOTS: &[&str] = &[
    "%TEMP%",
    "%LOCALAPPDATA%",
    "%APPDATA%",
    "%USERPROFILE%",
    "%WINDIR%",
    "%ProgramData%",
    "%ProgramFiles%",
    "%ProgramFiles(x86)%",
    "%SystemDrive%",
    "%CARGO_HOME%",
    "%GOCACHE%",
    "%GRADLE_USER_HOME%",
];

pub const PROTECTED: &[&str] = &[
    "%windir%/system32",
    "%windir%/syswow64",
    "%windir%/winsxs",
    "%windir%/prefetch",
    "%windir%/installer",
    "%windir%/appcompat",
    "%windir%/boot",
    "%windir%/servicing",
    "%programdata%/microsoft/windows defender",
    "%systemdrive%/windows.old",
    "%systemdrive%/$windows.~bt",
    "%systemdrive%/recovery",
    "%userprofile%/documents",
    "%userprofile%/desktop",
    "%userprofile%/pictures",
    "%userprofile%/music",
    "%userprofile%/videos",
    "%userprofile%/onedrive",
    "%appdata%/microsoft/credentials",
    "%localappdata%/microsoft/credentials",
    "%appdata%/microsoft/protect",
];

pub fn protected_hit(target: &str) -> Option<&'static str> {
    let t = super::kudu::normalize_target(target);
    PROTECTED
        .iter()
        .copied()
        .find(|p| t == *p || t.starts_with(&format!("{p}/")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Before,
    Instead,
}

pub fn maintenance_for(rule_id: &str) -> Option<(&'static str, Stage)> {
    match rule_id {
        "windows.update.download_cache" => Some(("win_wu_stop", Stage::Before)),
        "windows.delivery_optimization.cache" => {
            Some(("win_delivery_optimization", Stage::Instead))
        }
        _ => None,
    }
}

pub fn maintenance_argv(id: &str, system_root: &str) -> Option<Vec<Vec<String>>> {
    let sys32 = format!("{}\\System32", system_root.trim_end_matches(['\\', '/']));
    let net = format!("{sys32}\\net.exe");
    let ps = format!("{sys32}\\WindowsPowerShell\\v1.0\\powershell.exe");
    let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    match id {
        "win_wu_stop" => Some(vec![
            [vec![net.clone()], v(&["stop", "wuauserv", "/y"])].concat(),
            [vec![net], v(&["stop", "bits", "/y"])].concat(),
        ]),
        "win_delivery_optimization" => Some(vec![[
            vec![ps],
            v(&[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Delete-DeliveryOptimizationCache -Force",
            ]),
        ]
        .concat()]),
        _ => None,
    }
}

struct B(Rule);

impl B {
    fn new(id: &str, category: &str, label: &str, description: &str) -> Self {
        let key = id.trim_start_matches("windows.").replace('.', "_");
        B(Rule {
            id: id.to_string(),
            os: vec![Os::Windows],
            category: category.to_string(),
            label_key: Some(format!("washmybelly.clean.rule.{key}")),
            label: label.to_string(),
            description: description.to_string(),
            targets: Vec::new(),
            include: Vec::new(),
            exclude: Vec::new(),
            running: Vec::new(),
            min_age_days: 1,
            action: Action::DeleteContents,
            destination: Destination::Unlink,
            risk: Risk::Destructive,
            requires: Vec::new(),
            source: "washmybelly".to_string(),
            license: "GPL-3.0".to_string(),
        })
    }
    fn targets(mut self, t: &[&str]) -> Self {
        self.0.targets = t.iter().map(|s| s.to_string()).collect();
        self
    }
    fn target_list(mut self, t: Vec<String>) -> Self {
        self.0.targets = t;
        self
    }
    fn delete(mut self) -> Self {
        self.0.action = Action::Delete;
        self
    }
    fn age(mut self, days: u32) -> Self {
        self.0.min_age_days = days;
        self
    }
    fn admin(mut self) -> Self {
        self.0.requires.push(Requirement::Admin);
        self
    }
    fn running(mut self, names: &[&str]) -> Self {
        self.0.running = names.iter().map(|s| s.to_string()).collect();
        self
    }
    fn exclude(mut self, names: &[&str]) -> Self {
        self.0.exclude = names.iter().map(|s| s.to_string()).collect();
        self
    }
    fn command(mut self, program: &str, args: &[&str]) -> Self {
        self.0.action = Action::Command {
            program: program.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            measure: Vec::new(),
        };
        self.0.requires.push(Requirement::Program {
            name: program.to_string(),
        });
        self.0.min_age_days = 0;
        self
    }
    fn build(self) -> Rule {
        self.0
    }
}

fn chromium_cache(base: &str, profile_is_base: bool) -> Vec<String> {
    let mut v = Vec::new();
    for d in [
        "Cache",
        "Code Cache",
        "GPUCache",
        "DawnWebGPUCache",
        "DawnGraphiteCache",
        "Service Worker\\CacheStorage",
    ] {
        v.push(format!("{base}\\*\\{d}"));
        if profile_is_base {
            v.push(format!("{base}\\{d}"));
        }
    }
    for d in [
        "ShaderCache",
        "GrShaderCache",
        "GraphiteDawnCache",
        "GPUPersistentCache",
    ] {
        v.push(format!("{base}\\{d}"));
    }
    v
}

fn chromium_files(base: &str, profile_is_base: bool, names: &[&str]) -> Vec<String> {
    let mut v = Vec::new();
    for n in names {
        v.push(format!("{base}\\*\\{n}"));
        if profile_is_base {
            v.push(format!("{base}\\{n}"));
        }
    }
    v
}

const HISTORY_FILES: &[&str] = &[
    "History",
    "History-journal",
    "Visited Links",
    "Top Sites",
    "Top Sites-journal",
    "Shortcuts",
    "Shortcuts-journal",
];
const COOKIE_FILES: &[&str] = &[
    "Network\\Cookies",
    "Network\\Cookies-journal",
    "Cookies",
    "Cookies-journal",
];

type Chromium = (
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
    bool,
    &'static str,
);
const CHROMIUMS: &[Chromium] = &[
    (
        "chrome",
        "Google Chrome",
        &["%LOCALAPPDATA%\\Google\\Chrome\\User Data"],
        &[],
        false,
        "chrome.exe",
    ),
    (
        "edge",
        "Microsoft Edge",
        &["%LOCALAPPDATA%\\Microsoft\\Edge\\User Data"],
        &[],
        false,
        "msedge.exe",
    ),
    (
        "brave",
        "Brave",
        &["%LOCALAPPDATA%\\BraveSoftware\\Brave-Browser\\User Data"],
        &[],
        false,
        "brave.exe",
    ),
    (
        "opera",
        "Opera",
        &["%APPDATA%\\Opera Software\\Opera Stable"],
        &["%LOCALAPPDATA%\\Opera Software\\Opera Stable"],
        true,
        "opera.exe",
    ),
    (
        "opera_gx",
        "Opera GX",
        &["%APPDATA%\\Opera Software\\Opera GX Stable"],
        &["%LOCALAPPDATA%\\Opera Software\\Opera GX Stable"],
        true,
        "opera.exe",
    ),
];

pub fn ours() -> Vec<Rule> {
    let mut v = vec![
        B::new("windows.temp.user", "system", "Temporary files", "Files programs left in your temporary folder. Anything used in the last two days stays.")
            .targets(&["%TEMP%", "%LOCALAPPDATA%\\Temp"])
            .age(2)
            .build(),
        B::new("windows.temp.system", "system", "Windows temporary files", "Temporary files of Windows services. Anything used in the last two days stays.")
            .targets(&["%WINDIR%\\Temp"])
            .age(2)
            .admin()
            .build(),
        B::new("windows.update.download_cache", "system", "Windows Update downloads", "Update packages already installed. The update service is stopped first; Windows downloads again anything it still needs.")
            .targets(&["%WINDIR%\\SoftwareDistribution\\Download"])
            .age(0)
            .admin()
            .build(),
        B::new("windows.delivery_optimization.cache", "system", "Delivery Optimization cache", "Update pieces kept to share with other PCs. Cleared by Windows' own tool, which keeps pinned files.")
            .targets(&[
                "%WINDIR%\\ServiceProfiles\\NetworkService\\AppData\\Local\\Microsoft\\Windows\\DeliveryOptimization\\Cache",
                "%WINDIR%\\SoftwareDistribution\\DeliveryOptimization",
            ])
            .age(0)
            .admin()
            .build(),
        B::new("windows.error_reports.user", "logs", "Error reports and crash dumps", "Reports Windows kept about programs that crashed.")
            .targets(&[
                "%LOCALAPPDATA%\\Microsoft\\Windows\\WER\\ReportArchive",
                "%LOCALAPPDATA%\\Microsoft\\Windows\\WER\\ReportQueue",
                "%LOCALAPPDATA%\\CrashDumps",
            ])
            .build(),
        B::new("windows.error_reports.system", "logs", "System error reports", "Crash reports kept for every account on this PC.")
            .targets(&[
                "%ProgramData%\\Microsoft\\Windows\\WER\\ReportArchive",
                "%ProgramData%\\Microsoft\\Windows\\WER\\ReportQueue",
            ])
            .admin()
            .build(),
        B::new("windows.explorer.thumbnail_cache", "system", "Thumbnail cache", "Picture previews File Explorer rebuilds as you browse. Files Explorer has open are skipped.")
            .targets(&["%LOCALAPPDATA%\\Microsoft\\Windows\\Explorer\\thumbcache_*.db"])
            .delete()
            .age(0)
            .build(),
        B::new("windows.explorer.icon_cache", "system", "Icon cache", "Icons File Explorer rebuilds on demand. Files Explorer has open are skipped.")
            .targets(&["%LOCALAPPDATA%\\Microsoft\\Windows\\Explorer\\iconcache_*.db", "%LOCALAPPDATA%\\IconCache.db"])
            .delete()
            .age(0)
            .build(),
        B::new("windows.shader.directx", "shader_cache", "DirectX shader cache", "Compiled shaders; games rebuild them, so the first launch after is slower.")
            .targets(&["%LOCALAPPDATA%\\D3DSCache"])
            .build(),
        B::new("windows.shader.nvidia", "shader_cache", "NVIDIA shader cache", "Compiled shaders; games rebuild them, so the first launch after is slower.")
            .targets(&[
                "%LOCALAPPDATA%\\NVIDIA\\DXCache",
                "%LOCALAPPDATA%\\NVIDIA\\GLCache",
                "%LOCALAPPDATA%\\NVIDIA Corporation\\NV_Cache",
                "%USERPROFILE%\\AppData\\LocalLow\\NVIDIA\\PerDriverVersion\\DXCache",
                "%USERPROFILE%\\AppData\\LocalLow\\NVIDIA\\PerDriverVersion\\GLCache",
            ])
            .build(),
        B::new("windows.shader.nvidia_system", "shader_cache", "NVIDIA shader cache (all accounts)", "Compiled shaders shared by every account.")
            .targets(&["%ProgramData%\\NVIDIA Corporation\\NV_Cache"])
            .admin()
            .build(),
        B::new("windows.shader.amd", "shader_cache", "AMD shader cache", "Compiled shaders; games rebuild them, so the first launch after is slower.")
            .targets(&[
                "%LOCALAPPDATA%\\AMD\\DxCache",
                "%LOCALAPPDATA%\\AMD\\DxcCache",
                "%LOCALAPPDATA%\\AMD\\VkCache",
                "%LOCALAPPDATA%\\AMD\\GLCache",
            ])
            .build(),
        B::new("windows.shader.intel", "shader_cache", "Intel shader cache", "Compiled shaders; games rebuild them, so the first launch after is slower.")
            .targets(&["%LOCALAPPDATA%\\Intel\\ShaderCache", "%USERPROFILE%\\AppData\\LocalLow\\Intel\\ShaderCache"])
            .build(),
        B::new("windows.browser.firefox.cache", "browser", "Firefox cache", "Saved copies of web pages; Firefox downloads them again.")
            .targets(&[
                "%LOCALAPPDATA%\\Mozilla\\Firefox\\Profiles\\*\\cache2",
                "%LOCALAPPDATA%\\Mozilla\\Firefox\\Profiles\\*\\startupCache",
                "%LOCALAPPDATA%\\Mozilla\\Firefox\\Profiles\\*\\thumbnails",
                "%LOCALAPPDATA%\\Mozilla\\Firefox\\Profiles\\*\\jumpListCache",
                "%APPDATA%\\Mozilla\\Firefox\\Profiles\\*\\shader-cache",
            ])
            .running(&["firefox.exe"])
            .build(),
        B::new("windows.browser.firefox.cookies", "privacy", "Firefox cookies", "Signs you out of websites.")
            .targets(&[
                "%APPDATA%\\Mozilla\\Firefox\\Profiles\\*\\cookies.sqlite",
                "%APPDATA%\\Mozilla\\Firefox\\Profiles\\*\\cookies.sqlite-wal",
                "%APPDATA%\\Mozilla\\Firefox\\Profiles\\*\\cookies.sqlite-shm",
            ])
            .delete()
            .age(0)
            .running(&["firefox.exe"])
            .build(),
        B::new("windows.steam.appcache", "games", "Steam cache", "Steam's app info and store page caches; Steam downloads them again. Stats waiting to sync stay.")
            .targets(&[
                "%ProgramFiles(x86)%\\Steam\\appcache",
                "%ProgramFiles%\\Steam\\appcache",
                "%LOCALAPPDATA%\\Steam\\htmlcache",
            ])
            .exclude(&["stats"])
            .running(&["steam.exe", "steamwebhelper.exe"])
            .build(),
        B::new("windows.steam.depotcache", "games", "Steam download manifests", "Manifests of finished downloads; Steam fetches them again when needed.")
            .targets(&["%ProgramFiles(x86)%\\Steam\\depotcache", "%ProgramFiles%\\Steam\\depotcache"])
            .running(&["steam.exe"])
            .build(),
        B::new("windows.steam.logs", "games", "Steam logs and crash dumps", "")
            .targets(&[
                "%ProgramFiles(x86)%\\Steam\\logs",
                "%ProgramFiles(x86)%\\Steam\\dumps",
                "%ProgramFiles%\\Steam\\logs",
                "%ProgramFiles%\\Steam\\dumps",
            ])
            .build(),
        B::new("windows.epic.cache", "games", "Epic Games Launcher cache and logs", "")
            .targets(&[
                "%LOCALAPPDATA%\\EpicGamesLauncher\\Saved\\webcache*",
                "%LOCALAPPDATA%\\EpicGamesLauncher\\Saved\\Logs",
            ])
            .running(&["EpicGamesLauncher.exe"])
            .build(),
        B::new("windows.battlenet.cache", "games", "Battle.net cache and logs", "")
            .targets(&[
                "%LOCALAPPDATA%\\Battle.net\\Cache",
                "%LOCALAPPDATA%\\Battle.net\\BrowserCache",
                "%LOCALAPPDATA%\\Battle.net\\Logs",
                "%LOCALAPPDATA%\\Blizzard Entertainment\\Battle.net\\Logs",
                "%APPDATA%\\Battle.net\\Logs",
            ])
            .running(&["Battle.net.exe"])
            .build(),
        B::new("windows.riot.logs", "games", "Riot Client logs", "")
            .targets(&["%LOCALAPPDATA%\\Riot Games\\Riot Client\\Logs"])
            .running(&["RiotClientServices.exe"])
            .build(),
        B::new("windows.dev.npm", "dev", "npm cache", "Downloaded packages; npm fetches them again.")
            .targets(&["%LOCALAPPDATA%\\npm-cache\\_cacache", "%LOCALAPPDATA%\\npm-cache\\_logs"])
            .build(),
        B::new("windows.dev.pip", "dev", "pip cache", "Downloaded Python packages.")
            .targets(&["%LOCALAPPDATA%\\pip\\Cache"])
            .build(),
        B::new("windows.dev.nuget", "dev", "NuGet cache", "Downloaded .NET packages (the HTTP cache, not your restored packages).")
            .targets(&["%LOCALAPPDATA%\\NuGet\\v3-cache", "%LOCALAPPDATA%\\NuGet\\plugins-cache"])
            .build(),
        B::new("windows.dev.gradle", "dev", "Gradle caches", "Downloaded dependencies and build caches; the next build downloads them again.")
            .targets(&["%GRADLE_USER_HOME%\\caches", "%USERPROFILE%\\.gradle\\caches"])
            .age(2)
            .build(),
        B::new("windows.dev.cargo", "dev", "Cargo registry cache", "Downloaded crate archives.")
            .targets(&["%CARGO_HOME%\\registry\\cache", "%USERPROFILE%\\.cargo\\registry\\cache"])
            .build(),
        B::new("windows.dev.go_build", "dev", "Go build cache", "Compiled Go packages.")
            .targets(&["%GOCACHE%", "%LOCALAPPDATA%\\go-build"])
            .build(),
        B::new("windows.dev.docker_builder", "dev", "Docker build cache", "Build cache unused for a week, through Docker itself. Containers, images and volumes stay.")
            .command("docker", &["builder", "prune", "--force", "--filter", "until=168h"])
            .build(),
    ];

    for (key, label, bases, cache_bases, profile_is_base, exe) in CHROMIUMS {
        let mut cache = Vec::new();
        let mut history = Vec::new();
        let mut cookies = Vec::new();
        for b in bases.iter() {
            cache.extend(chromium_cache(b, *profile_is_base));
            history.extend(chromium_files(b, *profile_is_base, HISTORY_FILES));
            cookies.extend(chromium_files(b, *profile_is_base, COOKIE_FILES));
        }
        for b in cache_bases.iter() {
            cache.extend(chromium_cache(b, true));
        }
        v.push(
            B::new(
                &format!("windows.browser.{key}.cache"),
                "browser",
                &format!("{label} cache"),
                "Saved copies of web pages and compiled scripts; the browser downloads them again.",
            )
            .target_list(cache)
            .running(&[exe])
            .build(),
        );
        v.push(
            B::new(
                &format!("windows.browser.{key}.history"),
                "privacy",
                &format!("{label} history"),
                "Pages you visited and the shortcuts built from them. Bookmarks stay.",
            )
            .target_list(history)
            .delete()
            .age(0)
            .running(&[exe])
            .build(),
        );
        v.push(
            B::new(
                &format!("windows.browser.{key}.cookies"),
                "privacy",
                &format!("{label} cookies"),
                "Signs you out of websites.",
            )
            .target_list(cookies)
            .delete()
            .age(0)
            .running(&[exe])
            .build(),
        );
    }
    v
}

pub fn rules() -> Vec<Rule> {
    let mut v = ours();
    #[cfg(windows)]
    {
        let roots: Vec<PathBuf> = ["ProgramFiles(x86)", "ProgramFiles"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|p| PathBuf::from(p).join("Steam"))
            .collect();
        if let Some(r) = steam_orphan_rule(&roots) {
            v.push(r);
        }
    }
    let kudu = super::kudu::dedupe_against(&v, super::kudu::rules());
    v.extend(kudu);
    v
}

pub fn steam_library_paths(vdf: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in vdf.lines() {
        let parts: Vec<&str> = line.split('"').collect();
        if parts.len() >= 5 && parts[1].eq_ignore_ascii_case("path") && parts[2].trim().is_empty() {
            let p = parts[3].replace("\\\\", "\\");
            if !p.is_empty() && !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

fn app_id(name: &str) -> Option<&str> {
    (!name.is_empty() && name.bytes().all(|b| b.is_ascii_digit())).then_some(name)
}

pub fn steam_orphan_shadercache(steamapps_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut installed: HashSet<String> = HashSet::new();
    for dir in steamapps_dirs {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(id) = name
                .strip_prefix("appmanifest_")
                .and_then(|s| s.strip_suffix(".acf"))
                .and_then(app_id)
            {
                installed.insert(id.to_string());
            }
        }
    }
    if installed.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for dir in steamapps_dirs {
        let sc = dir.join("shadercache");
        let Ok(rd) = std::fs::read_dir(&sc) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(id) = app_id(&name) else { continue };
            let Ok(m) = std::fs::symlink_metadata(e.path()) else {
                continue;
            };
            if m.is_dir()
                && !crate::core::washmybelly::fsops::is_link(&m)
                && !installed.contains(id)
            {
                out.push(e.path());
            }
        }
    }
    out.sort();
    out
}

pub fn steam_steamapps_dirs(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for root in roots {
        let own = root.join("steamapps");
        if !own.is_dir() {
            continue;
        }
        if !dirs.contains(&own) {
            dirs.push(own.clone());
        }
        if let Ok(text) = std::fs::read_to_string(own.join("libraryfolders.vdf")) {
            for lib in steam_library_paths(&text) {
                let d = Path::new(&lib).join("steamapps");
                if d.is_dir() && !dirs.contains(&d) {
                    dirs.push(d);
                }
            }
        }
    }
    dirs
}

pub fn steam_orphan_rule(roots: &[PathBuf]) -> Option<Rule> {
    let orphans = steam_orphan_shadercache(&steam_steamapps_dirs(roots));
    if orphans.is_empty() {
        return None;
    }
    Some(
        B::new(
            "windows.steam.shadercache_uninstalled",
            "games",
            "Shader caches of uninstalled Steam games",
            "Games you removed left their compiled shaders behind.",
        )
        .target_list(
            orphans
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
        )
        .delete()
        .running(&["steam.exe"])
        .build(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::washmybelly::fsops;

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/washmybelly_fixtures/clean_windows"
    );

    fn fake_lookup(name: &str) -> Option<String> {
        let v = match name.to_ascii_uppercase().as_str() {
            "USERPROFILE" | "HOME" => "C:\\Users\\pibble",
            "LOCALAPPDATA" => "C:\\Users\\pibble\\AppData\\Local",
            "APPDATA" => "C:\\Users\\pibble\\AppData\\Roaming",
            "TEMP" | "TMP" => "C:\\Users\\pibble\\AppData\\Local\\Temp",
            "WINDIR" | "SYSTEMROOT" => "C:\\Windows",
            "PROGRAMDATA" => "C:\\ProgramData",
            "PROGRAMFILES" => "C:\\Program Files",
            "PROGRAMFILES(X86)" => "C:\\Program Files (x86)",
            "SYSTEMDRIVE" => "C:",
            "CARGO_HOME" => "C:\\Users\\pibble\\.cargo",
            "GOCACHE" => "C:\\Users\\pibble\\AppData\\Local\\go-build",
            "GRADLE_USER_HOME" => "C:\\Users\\pibble\\.gradle",
            _ => return None,
        };
        Some(v.to_string())
    }

    const RESOLVED_ROOTS: &[&str] = &[
        "c:",
        "c:/windows",
        "c:/windows/temp",
        "c:/program files",
        "c:/program files (x86)",
        "c:/programdata",
        "c:/users",
        "c:/users/pibble",
        "c:/users/pibble/appdata",
        "c:/users/pibble/appdata/local",
        "c:/users/pibble/appdata/roaming",
        "c:/users/pibble/appdata/local/temp",
        "c:/users/pibble/downloads",
    ];
    const RESOLVED_NEVER_ENTER: &[&str] = &[
        "c:/windows/system32",
        "c:/windows/syswow64",
        "c:/windows/winsxs",
        "c:/windows/prefetch",
        "c:/windows/installer",
        "c:/windows/boot",
        "c:/boot",
        "c:/recovery",
        "c:/system volume information",
        "c:/windows.old",
        "c:/programdata/microsoft/windows defender",
        "c:/users/pibble/documents",
        "c:/users/pibble/desktop",
        "c:/users/pibble/pictures",
        "c:/users/pibble/music",
        "c:/users/pibble/videos",
        "c:/users/pibble/onedrive",
        "c:/users/pibble/.ssh",
        "c:/users/pibble/appdata/roaming/microsoft/credentials",
        "c:/users/pibble/appdata/local/microsoft/credentials",
        "c:/users/pibble/appdata/roaming/microsoft/protect",
    ];

    fn resolved(t: &str) -> String {
        let r = fsops::expand_vars(t, &fake_lookup)
            .unwrap_or_else(|| panic!("{t}: a variable did not resolve"));
        r.replace('\\', "/").to_lowercase()
    }

    fn check_rules(rules: &[Rule]) {
        for r in rules {
            for t in &r.targets {
                assert!(
                    ALLOWED_ROOTS
                        .iter()
                        .any(|root| t.to_lowercase().starts_with(&root.to_lowercase())),
                    "{}: {t} does not start at an allowed root",
                    r.id
                );
                assert!(protected_hit(t).is_none(), "{}: {t} is protected", r.id);
                let res = resolved(t);
                assert!(res.starts_with("c:/"), "{}: {t} → {res}", r.id);
                for bad in RESOLVED_NEVER_ENTER {
                    assert!(
                        res != *bad && !res.starts_with(&format!("{bad}/")),
                        "{}: {t} enters {bad}",
                        r.id
                    );
                }
                if r.action == Action::Delete {
                    assert!(
                        !RESOLVED_ROOTS.contains(&res.as_str()),
                        "{}: deletes root {res}",
                        r.id
                    );
                }
                assert!(res.split('/').count() >= 2, "{}: {t} is too shallow", r.id);
            }
        }
    }

    #[test]
    fn our_rules_are_windows_scoped_and_resolve() {
        let rules = ours();
        assert!(rules.len() >= 40, "{}", rules.len());
        let mut ids = HashSet::new();
        for r in &rules {
            assert!(ids.insert(r.id.clone()), "duplicate {}", r.id);
            assert!(r.id.starts_with("windows."), "{}", r.id);
            assert_eq!(r.os, vec![Os::Windows]);
            assert!(r
                .label_key
                .as_deref()
                .is_some_and(|k| k.starts_with("washmybelly.clean.rule.")));
            match &r.action {
                Action::Command { .. } => assert!(r.targets.is_empty()),
                _ => assert!(!r.targets.is_empty(), "{}", r.id),
            }
        }
        check_rules(&rules);
    }

    #[test]
    fn kudu_rules_stay_inside_allowed_roots() {
        check_rules(&super::super::kudu::rules());
    }

    #[test]
    fn no_command_outside_native_programs() {
        for r in rules() {
            if let Action::Command { program, args, .. } = &r.action {
                assert!(
                    crate::core::washmybelly::plan::NATIVE_PROGRAMS.contains(&program.as_str()),
                    "{}: {program}",
                    r.id
                );
                assert!(
                    !args.iter().any(|a| a.eq_ignore_ascii_case("/ResetBase")),
                    "{}",
                    r.id
                );
            }
        }
    }

    #[test]
    fn nothing_from_the_left_out_list() {
        let all = rules();
        let text = serde_json::to_string(&all).unwrap().to_lowercase();
        for bad in [
            "resetbase",
            "cleanmgr",
            "\\\\prefetch",
            "windows.old",
            "winsxs",
        ] {
            assert!(!text.contains(bad), "{bad} appears in a Windows rule");
        }
        for r in &all {
            assert!(!r.id.contains("defender"), "{}", r.id);
        }
    }

    #[test]
    fn requested_categories_are_covered() {
        let all = ours();
        let has = |needle: &str| {
            all.iter()
                .any(|r| r.targets.iter().any(|t| t.contains(needle)))
        };
        for n in [
            "%TEMP%",
            "%WINDIR%\\Temp",
            "SoftwareDistribution\\Download",
            "DeliveryOptimization",
            "WER\\ReportArchive",
            "CrashDumps",
            "thumbcache_*.db",
            "iconcache_*.db",
            "D3DSCache",
            "NVIDIA\\DXCache",
            "NV_Cache",
            "AMD\\DxcCache",
            "Mozilla\\Firefox",
            "Opera Stable",
            "Steam\\appcache",
            "depotcache",
            "EpicGamesLauncher",
            "Battle.net",
            "Riot Client",
            "NuGet\\v3-cache",
            "pip\\Cache",
            ".gradle",
            "registry\\cache",
            "go-build",
        ] {
            assert!(has(n), "no rule reaches {n}");
        }
        let temp = all.iter().find(|r| r.id == "windows.temp.user").unwrap();
        assert_eq!(temp.min_age_days, 2);
        for id in [
            "windows.temp.system",
            "windows.update.download_cache",
            "windows.delivery_optimization.cache",
            "windows.error_reports.system",
        ] {
            let r = all.iter().find(|r| r.id == id).unwrap();
            assert!(r.requires.contains(&Requirement::Admin), "{id} needs admin");
        }
    }

    #[test]
    fn history_and_cookies_start_unchecked_and_caches_do_not() {
        let all = ours();
        for key in ["chrome", "edge", "brave", "opera"] {
            let h = all
                .iter()
                .find(|r| r.id == format!("windows.browser.{key}.history"))
                .unwrap();
            assert_eq!(h.risk, Risk::Destructive);
            assert!(
                !h.targets.iter().any(|t| t.ends_with("Bookmarks")),
                "bookmarks stay"
            );
            assert!(!h.running.is_empty());
        }
        let ff = all
            .iter()
            .find(|r| r.id == "windows.browser.firefox.cookies")
            .unwrap();
        assert!(
            !ff.targets.iter().any(|t| t.contains("places.sqlite")),
            "places.sqlite holds bookmarks"
        );
    }

    #[test]
    fn merged_list_has_no_duplicate_ids_and_dedupes_kudu() {
        let all = rules();
        let mut ids = HashSet::new();
        for r in &all {
            assert!(ids.insert(r.id.clone()), "duplicate {}", r.id);
        }
        assert!(!all.iter().any(|r| r.id == "kudu.system.user_temp_files"));
        assert!(!all
            .iter()
            .any(|r| r.id == "kudu.system.thumbnail_icon_cache"));
        assert!(!all
            .iter()
            .any(|r| r.id == "kudu.system.directx_shader_cache"));
        assert!(all.iter().any(|r| r.id == "kudu.apps.discord"));
        assert!(all.iter().any(|r| r.id == "kudu.browser.vivaldi.cache"));
        assert!(all.len() > ours().len() + 100);
        let admin = all
            .iter()
            .filter(|r| r.requires.contains(&Requirement::Admin))
            .count();
        eprintln!(
            "windows rules: {} total, {} ours, {} admin",
            all.len(),
            ours().len(),
            admin
        );
    }

    #[test]
    fn maintenance_is_fixed_argv_under_system_root() {
        assert_eq!(
            maintenance_for("windows.update.download_cache"),
            Some(("win_wu_stop", Stage::Before))
        );
        assert_eq!(
            maintenance_for("windows.delivery_optimization.cache"),
            Some(("win_delivery_optimization", Stage::Instead))
        );
        assert_eq!(maintenance_for("windows.temp.user"), None);
        let wu = maintenance_argv("win_wu_stop", "C:\\Windows\\").unwrap();
        assert_eq!(
            wu[0],
            vec!["C:\\Windows\\System32\\net.exe", "stop", "wuauserv", "/y"]
        );
        let dosvc = maintenance_argv("win_delivery_optimization", "C:\\Windows").unwrap();
        assert!(dosvc[0][0].ends_with("\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"));
        assert_eq!(
            dosvc[0].last().unwrap(),
            "Delete-DeliveryOptimizationCache -Force"
        );
        assert!(maintenance_argv("anything_else", "C:\\Windows").is_none());
        for id in ["win_wu_stop", "win_delivery_optimization"] {
            for argv in maintenance_argv(id, "C:\\Windows").unwrap() {
                assert!(!argv.iter().any(|a| a.to_lowercase().contains("resetbase")
                    || a.to_lowercase().contains("cleanmgr")));
            }
        }
    }

    #[test]
    fn steam_libraryfolders_fixture_parses() {
        let text = std::fs::read_to_string(format!("{FIXTURES}/libraryfolders.vdf")).unwrap();
        assert_eq!(
            steam_library_paths(&text),
            vec![
                "C:\\Program Files (x86)\\Steam".to_string(),
                "D:\\SteamLibrary".to_string()
            ]
        );
    }

    #[test]
    fn steam_orphans_only_for_uninstalled_games() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Steam");
        let apps = root.join("steamapps");
        let lib2 = dir.path().join("Lib2");
        let apps2 = lib2.join("steamapps");
        for d in [
            apps.join("shadercache/440"),
            apps.join("shadercache/570"),
            apps.join("shadercache/notanid"),
            apps2.join("shadercache/730"),
            apps2.join("shadercache/999"),
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(apps.join("appmanifest_440.acf"), "\"AppState\" {}").unwrap();
        std::fs::write(apps2.join("appmanifest_730.acf"), "\"AppState\" {}").unwrap();
        let vdf = format!(
            "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
            root.to_string_lossy().replace('\\', "\\\\"),
            lib2.to_string_lossy().replace('\\', "\\\\")
        );
        std::fs::write(apps.join("libraryfolders.vdf"), vdf).unwrap();

        let orphans = steam_orphan_shadercache(&steam_steamapps_dirs(std::slice::from_ref(&root)));
        let mut names: Vec<String> = orphans
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["570".to_string(), "999".to_string()]);

        let rule = steam_orphan_rule(std::slice::from_ref(&root)).unwrap();
        assert_eq!(rule.targets.len(), 2);
        assert_eq!(rule.action, Action::Delete);
    }

    #[test]
    fn steam_without_manifests_offers_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("Steam/steamapps");
        std::fs::create_dir_all(apps.join("shadercache/570")).unwrap();
        assert!(steam_orphan_shadercache(&[apps]).is_empty());
        assert!(steam_orphan_rule(&[dir.path().join("Steam")]).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn junction_canary_survives_emptying_a_cache() {
        let dir = tempfile::tempdir().unwrap();
        let precious = dir.path().join("precious");
        std::fs::create_dir_all(&precious).unwrap();
        std::fs::write(precious.join("canary.txt"), b"alive").unwrap();
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(cache.join("sub")).unwrap();
        std::fs::write(cache.join("sub/junk.bin"), b"junk").unwrap();
        std::os::unix::fs::symlink(&precious, cache.join("link")).unwrap();
        std::os::unix::fs::symlink(&precious, cache.join("sub/link2")).unwrap();

        let found = fsops::glob(&format!("{}/*/canary.txt", cache.display()));
        assert!(found.is_empty(), "{found:?}");

        fsops::remove_tree(&cache, true).unwrap();
        assert!(cache.is_dir(), "contents_only keeps the folder");
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
        assert_eq!(
            std::fs::read(precious.join("canary.txt")).unwrap(),
            b"alive"
        );
    }
}
