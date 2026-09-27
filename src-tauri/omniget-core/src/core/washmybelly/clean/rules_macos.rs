//! macOS cleanup rules, ported by hand from two upstream cleaners:
//!
//! - `source: "Mole"`: `lib/clean/{user,dev,app_caches,brew,system}.sh` of
//!   tw93/Mole (GPL-3.0, commit in `data/washmybelly/mole/SOURCE.md`). The
//!   name never reaches the UI.
//! - `source: "PureMac"`: the CLI catalog and `ScanEngine.swift` of
//!   momenbasel/PureMac (MIT, `data/washmybelly/puremac/`).
//!
//! Differences on purpose: one item per app (never `~/Library/Caches` in one
//! go), browsing data separate and unticked, the Trash is screen-only,
//! Archives and simulator runtimes unticked, `simctl delete unavailable`
//! (never `erase all`), Homebrew through `brew cleanup`, Time Machine
//! snapshots only thinned by the admin helper.

use serde_json::{json, Value};

use super::super::plan::Op;
use super::super::{Destination, Os, Risk};
use super::catalog::b;
use super::engine::{Env, Found};
use super::rule::Rule;

const M: &[Os] = &[Os::Macos];
const MOLE: (&str, &str) = ("Mole", "GPL-3.0");
const PUREMAC: (&str, &str) = ("PureMac", "MIT");

/// Chromium-family profile caches under an `Application Support` base.
fn chromium_caches(base: &str) -> Vec<String> {
    let mut v = Vec::new();
    for d in [
        "Code Cache",
        "GPUCache",
        "DawnCache",
        "GrShaderCache",
        "GraphiteDawnCache",
        "Application Cache",
    ] {
        v.push(format!("{base}/*/{d}"));
    }
    for d in [
        "ShaderCache",
        "GrShaderCache",
        "GraphiteDawnCache",
        "component_crx_cache",
        "extensions_crx_cache",
        "Crashpad/completed",
    ] {
        v.push(format!("{base}/{d}"));
    }
    v
}

fn refs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

pub fn rules() -> Vec<Rule> {
    let as_ = "~/Library/Application Support";
    let chrome = format!("{as_}/Google/Chrome");
    let brave = format!("{as_}/BraveSoftware/Brave-Browser");
    let edge = format!("{as_}/Microsoft Edge");
    let arc = format!("{as_}/Arc/User Data");
    let vivaldi = format!("{as_}/Vivaldi");

    let mut v = vec![
        // ── System ──
        b("macos.user_logs", M, "logs", "App logs")
            .t(&["~/Library/Logs"])
            .from(MOLE.0, MOLE.1)
            .desc("Log files apps wrote in your Library; apps start new ones.")
            .done(),
        b("macos.diagnostic_reports", M, "logs", "Diagnostic reports")
            .t(&["~/Library/DiagnosticReports", "~/Library/Application Support/CrashReporter"])
            .from(MOLE.0, MOLE.1)
            .done(),
        b("macos.saved_state", M, "system", "Saved window states")
            .t(&["~/Library/Saved Application State/*"])
            .delete()
            .age(30)
            .from(PUREMAC.0, PUREMAC.1)
            .desc("Windows apps reopen at launch. Only states older than 30 days.")
            .done(),
        b("macos.quicklook", M, "system", "Quick Look thumbnails")
            .t(&["~/Library/Caches/com.apple.QuickLook.thumbnailcache", "~/Library/Caches/Quick Look"])
            .from(MOLE.0, MOLE.1)
            .done(),
        b("macos.apple_caches", M, "system", "macOS caches")
            .t(&[
                "~/Library/Caches/com.apple.WebKit.Networking",
                "~/Library/Caches/com.apple.helpd",
                "~/Library/Caches/GeoServices",
                "~/Library/Caches/com.apple.AppleMediaServices",
                "~/Library/Caches/com.apple.duetexpertd",
                "~/Library/Caches/com.apple.photoanalysisd",
            ])
            .from(MOLE.0, MOLE.1)
            .done(),
        // Other apps' containers: without Full Disk Access macOS asks
        // "allow access to data from other apps?" for each one.
        b("macos.apple_container_caches", M, "system", "macOS app caches")
            .t(&[
                "~/Library/Containers/com.apple.wallpaper.agent/Data/Library/Caches",
                "~/Library/Containers/com.apple.mediaanalysisd/Data/Library/Caches",
                "~/Library/Containers/com.apple.mediaanalysisd/Data/tmp",
                "~/Library/Containers/com.apple.AppStore/Data/Library/Caches",
                "~/Library/Containers/com.apple.stocks/Data/Library/Caches",
                "~/Library/Containers/com.apple.AMPArtworkAgent/Data/Library/Caches",
                "~/Library/Containers/com.apple.AvatarUI.AvatarPickerMemojiPicker/Data/Library/Caches",
                "~/Library/Containers/com.apple.CoreDevice.CoreDeviceService/Data/Library/Caches",
                "~/Library/Containers/com.apple.geod/Data/tmp",
            ])
            .fda()
            .from(MOLE.0, MOLE.1)
            .done(),
        b("macos.app_caches", M, "apps", "App caches")
            .t(&["~/Library/Caches/*"])
            .delete()
            .exclude(&["com.apple.*", ".*"])
            .from(MOLE.0, MOLE.1)
            .desc("One item per app. Apps rebuild their cache the next time they need it.")
            .done(),
        b("macos.container_caches", M, "apps", "Sandboxed app caches")
            .t(&["~/Library/Containers/*/Data/Library/Caches"])
            .exclude(&["path-re:/Library/Containers/com\\.apple\\."])
            .fda()
            .from(MOLE.0, MOLE.1)
            .done(),
        b("macos.mail_downloads", M, "apps", "Mail attachment copies")
            .t(&["~/Library/Containers/com.apple.mail/Data/Library/Mail Downloads", "~/Library/Mail Downloads"])
            .age(30)
            .running(&["Mail"])
            .fda()
            .from(MOLE.0, MOLE.1)
            .desc("Copies Mail made when you opened attachments. The attachments stay in your mail.")
            .done(),
        b("macos.trash", M, "trash", "Trash")
            .t(&["~/.Trash", "{mount}/.Trashes/$UID"])
            .risk(Risk::Extreme)
            .from(PUREMAC.0, PUREMAC.1)
            .desc("Empties the Trash of every disk. Only from the OmniGet screen.")
            .done(),
        b("macos.ios_firmware", M, "user_files", "iPhone and iPad software updates")
            .t(&["~/Library/iTunes/*Software Updates/*.ipsw"])
            .delete()
            .from(MOLE.0, MOLE.1)
            .done(),
        b("macos.tm_snapshots", M, "system", "Local Time Machine snapshots")
            .cmd("tmutil", &["thinlocalsnapshots", "/", "999999999999999", "4"], &[])
            .admin()
            .from(PUREMAC.0, PUREMAC.1)
            .desc("Snapshots Time Machine keeps on this disk until a backup disk is back. macOS also removes them when space runs low.")
            .done(),
        // ── Developer ──
        b("macos.xcode_derived_data", M, "dev", "Xcode build data")
            .t(&["~/Library/Developer/Xcode/DerivedData", "~/Library/Developer/Xcode/Products"])
            .running(&["Xcode", "xcodebuild"])
            .from(MOLE.0, MOLE.1)
            .done(),
        b("macos.xcode_archives", M, "dev", "Xcode archives")
            .t(&["~/Library/Developer/Xcode/Archives/*"])
            .delete()
            .dest(Destination::Trash)
            .risk(Risk::Reversible)
            .from(PUREMAC.0, PUREMAC.1)
            .desc("Builds you archived, with the symbols needed to read crash reports.")
            .done(),
        b("macos.xcode_device_support", M, "dev", "Xcode device support files")
            .t(&[
                "~/Library/Developer/Xcode/iOS DeviceSupport/*",
                "~/Library/Developer/Xcode/watchOS DeviceSupport/*",
                "~/Library/Developer/Xcode/tvOS DeviceSupport/*",
                "~/Library/Developer/Xcode/visionOS DeviceSupport/*",
            ])
            .delete()
            .from(PUREMAC.0, PUREMAC.1)
            .desc("Xcode copies these again the next time a device with that version connects.")
            .done(),
        b("macos.simulator_caches", M, "dev", "Simulator caches")
            .t(&[
                "~/Library/Developer/CoreSimulator/Caches",
                "~/Library/Developer/XCTestDevices",
                "~/Library/Developer/Xcode/UserData/Previews",
                "~/Library/Caches/com.apple.dt.Xcode",
            ])
            .running(&["Simulator", "Xcode"])
            .from(PUREMAC.0, PUREMAC.1)
            .done(),
        b("macos.simctl_unavailable", M, "dev", "Unavailable simulators")
            .cmd("xcrun", &["simctl", "delete", "unavailable"], &[])
            .needs_path("~/Library/Developer/CoreSimulator/Devices")
            .from(MOLE.0, MOLE.1)
            .desc("Simulators whose runtime is gone. Deleted by Xcode's own tool.")
            .done(),
        b("macos.simulator_runtimes", M, "dev", "Older simulator runtimes")
            .cmd("xcrun", &["simctl", "runtime", "delete"], &[])
            .needs_path("/Library/Developer/CoreSimulator")
            .from(PUREMAC.0, PUREMAC.1)
            .desc("Runtimes Xcode marks as deletable, except the newest of each platform.")
            .done(),
        b("macos.homebrew", M, "package_cache", "Homebrew downloads")
            .cmd("brew", &["cleanup", "--prune=all"], &["$BREW_CACHE"])
            .from(MOLE.0, MOLE.1)
            .desc("Old versions and downloaded bottles, removed by brew cleanup.")
            .done(),
        b("macos.swiftpm", M, "dev", "Swift Package Manager cache")
            .t(&["~/Library/Caches/org.swift.swiftpm"])
            .from(PUREMAC.0, PUREMAC.1)
            .done(),
        b("macos.vscode", M, "dev", "VS Code caches")
            .t(&[
                "~/Library/Application Support/Code/Cache",
                "~/Library/Application Support/Code/CachedData",
                "~/Library/Application Support/Code/CachedExtensionVSIXs",
                "~/Library/Application Support/Code/logs",
            ])
            .running(&["Code", "Electron"])
            .from(PUREMAC.0, PUREMAC.1)
            .done(),
        b("macos.cursor", M, "dev", "Cursor caches")
            .t(&[
                "~/Library/Application Support/Cursor/Cache",
                "~/Library/Application Support/Cursor/CachedData",
                "~/Library/Application Support/Cursor/logs",
            ])
            .running(&["Cursor"])
            .from(PUREMAC.0, PUREMAC.1)
            .done(),
        b("macos.ai_logs", M, "logs", "Local AI app logs")
            .t(&["~/.ollama/logs", "~/.lmstudio/server-logs"])
            .from(PUREMAC.0, PUREMAC.1)
            .done(),
        // ── Browsers: caches ──
        b("macos.safari_cache", M, "browser", "Safari: Cache")
            .t(&["~/Library/Caches/com.apple.Safari", "~/Library/Containers/com.apple.Safari/Data/Library/Caches"])
            .running(&["Safari"])
            .fda()
            .from(MOLE.0, MOLE.1)
            .done(),
        b("macos.chrome_cache", M, "browser", "Google Chrome: Cache")
            .t(&["~/Library/Caches/Google/Chrome"])
            .t(&refs(&chromium_caches(&chrome)))
            .running(&["Google Chrome"])
            .from(MOLE.0, MOLE.1)
            .done(),
        b("macos.chrome_ai_models", M, "browser", "Google Chrome: on-device AI models")
            .t(&[
                &format!("{chrome}/OptGuideOnDeviceModel"),
                &format!("{chrome}/OptGuideOnDeviceClassifierModel"),
                &format!("{chrome}/optimization_guide_model_store"),
            ])
            .running(&["Google Chrome"])
            .from(MOLE.0, MOLE.1)
            .desc("Chrome downloads these models again when a feature needs them.")
            .done(),
        b("macos.brave_cache", M, "browser", "Brave: Cache")
            .t(&["~/Library/Caches/BraveSoftware/Brave-Browser"])
            .t(&refs(&chromium_caches(&brave)))
            .running(&["Brave Browser"])
            .from(MOLE.0, MOLE.1)
            .done(),
        b("macos.edge_cache", M, "browser", "Microsoft Edge: Cache")
            .t(&["~/Library/Caches/com.microsoft.edgemac", "~/Library/Caches/Microsoft Edge"])
            .t(&refs(&chromium_caches(&edge)))
            .running(&["Microsoft Edge"])
            .from(MOLE.0, MOLE.1)
            .done(),
        b("macos.arc_cache", M, "browser", "Arc: Cache")
            .t(&["~/Library/Caches/company.thebrowser.Browser", "~/Library/Caches/Arc"])
            .t(&refs(&chromium_caches(&arc)))
            .running(&["Arc"])
            .from(MOLE.0, MOLE.1)
            .done(),
        b("macos.vivaldi_cache", M, "browser", "Vivaldi: Cache")
            .t(&["~/Library/Caches/com.vivaldi.Vivaldi"])
            .t(&refs(&chromium_caches(&vivaldi)))
            .running(&["Vivaldi"])
            .from(MOLE.0, MOLE.1)
            .done(),
        b("macos.opera_cache", M, "browser", "Opera: Cache")
            .t(&["~/Library/Caches/com.operasoftware.Opera"])
            .running(&["Opera"])
            .from(MOLE.0, MOLE.1)
            .done(),
        b("macos.firefox_cache", M, "browser", "Firefox: Cache")
            .t(&["~/Library/Caches/Firefox/Profiles/*", "~/Library/Application Support/Firefox/Profiles/*/cache2"])
            .running(&["firefox"])
            .from(MOLE.0, MOLE.1)
            .done(),
        b("macos.zen_cache", M, "browser", "Zen: Cache")
            .t(&["~/Library/Caches/zen"])
            .running(&["zen"])
            .from(MOLE.0, MOLE.1)
            .done(),
    ];

    // ── Browsers: history and cookies, separate and unticked ──
    for (id, name, base, proc_) in [
        ("chrome", "Google Chrome", chrome.as_str(), "Google Chrome"),
        ("brave", "Brave", brave.as_str(), "Brave Browser"),
        ("edge", "Microsoft Edge", edge.as_str(), "Microsoft Edge"),
        ("arc", "Arc", arc.as_str(), "Arc"),
    ] {
        v.push(
            b(
                &format!("macos.{id}_history"),
                M,
                "privacy",
                &format!("{name}: History"),
            )
            .t(&[
                &format!("{base}/*/History*"),
                &format!("{base}/*/Visited Links"),
                &format!("{base}/*/Top Sites*"),
                &format!("{base}/*/Shortcuts*"),
            ])
            .delete()
            .privacy()
            .running(&[proc_])
            .desc("Visited pages and downloads list. Bookmarks and passwords stay.")
            .done(),
        );
        v.push(
            b(
                &format!("macos.{id}_cookies"),
                M,
                "privacy",
                &format!("{name}: Cookies"),
            )
            .t(&[
                &format!("{base}/*/Cookies*"),
                &format!("{base}/*/Network/Cookies*"),
            ])
            .delete()
            .privacy()
            .running(&[proc_])
            .desc("Signs you out of websites.")
            .done(),
        );
    }
    v.push(
        b("macos.firefox_cookies", M, "privacy", "Firefox: Cookies")
            .t(&["~/Library/Application Support/Firefox/Profiles/*/cookies.sqlite*"])
            .delete()
            .privacy()
            .running(&["firefox"])
            .desc("Signs you out of websites.")
            .done(),
    );
    v.push(
        b("macos.safari_history", M, "privacy", "Safari: History")
            .t(&["~/Library/Safari/History.db*"])
            .delete()
            .privacy()
            .running(&["Safari"])
            .fda()
            .done(),
    );
    v.push(
        b("macos.safari_cookies", M, "privacy", "Safari: Cookies")
            .t(&["~/Library/Cookies/Cookies.binarycookies"])
            .delete()
            .privacy()
            .running(&["Safari"])
            .fda()
            .desc("Signs you out of websites.")
            .done(),
    );
    v
}

// ── Probes: rules whose size only a system tool knows ───────────────

/// `xcrun` with the Command Line Tools present (the bare shim opens an
/// installer dialog, so never call it without them).
fn xcrun_ready(env: &Env) -> bool {
    env.output("xcode-select", &["-p"])
        .is_some_and(|s| !s.trim().is_empty())
}

/// Unavailable simulators: `simctl list devices unavailable -j`.
pub fn probe_simctl_unavailable(rule: &Rule, env: &Env) -> Vec<Found> {
    if !xcrun_ready(env) {
        return Vec::new();
    }
    let Some(out) = env.output("xcrun", &["simctl", "list", "devices", "unavailable", "-j"]) else {
        return Vec::new();
    };
    let dirs = parse_simctl_unavailable(&out);
    if dirs.is_empty() {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    let (bytes, files) = dirs.iter().fold((0, 0), |(b, f), d| {
        let (x, y) = super::super::fsops::measure_with(std::path::Path::new(d), &mut seen);
        (b + x, f + y)
    });
    let super::rule::Action::Command { program, args, .. } = &rule.action else {
        return Vec::new();
    };
    vec![Found {
        path: "~/Library/Developer/CoreSimulator/Devices".into(),
        label: Some(format!("{} ({})", rule.label, dirs.len())),
        bytes,
        files,
        op: Op::Command {
            program: program.clone(),
            args: args.clone(),
            measure: dirs.iter().map(Into::into).collect(),
        },
        admin: false,
    }]
}

/// Data folders of the unavailable devices in `simctl list -j` output.
pub fn parse_simctl_unavailable(json_text: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(json_text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(map) = v["devices"].as_object() {
        for list in map.values() {
            for d in list.as_array().into_iter().flatten() {
                let unavailable = d["isAvailable"] == json!(false)
                    || d["availability"]
                        .as_str()
                        .is_some_and(|a| a.contains("unavailable"));
                if !unavailable {
                    continue;
                }
                if let Some(p) = d["dataPath"].as_str() {
                    // dataPath is <device>/data; the device folder is its parent.
                    let dev = p.strip_suffix("/data").unwrap_or(p);
                    out.push(dev.to_string());
                }
            }
        }
    }
    out
}

/// Deletable simulator runtimes, minus the newest of each platform.
pub fn probe_simulator_runtimes(rule: &Rule, env: &Env) -> Vec<Found> {
    if !xcrun_ready(env) {
        return Vec::new();
    }
    let Some(out) = env.output("xcrun", &["simctl", "runtime", "list", "-j"]) else {
        return Vec::new();
    };
    parse_simctl_runtimes(&out)
        .into_iter()
        .map(|rt| Found {
            path: format!("simruntime:{}", rt.id),
            label: Some(format!("{}: {} {}", rule.label, rt.platform, rt.version)),
            bytes: rt.bytes,
            files: 1,
            op: Op::Command {
                program: "xcrun".into(),
                args: vec!["simctl".into(), "runtime".into(), "delete".into(), rt.id],
                measure: Vec::new(),
            },
            admin: false,
        })
        .collect()
}

#[derive(Debug, PartialEq)]
pub struct Runtime {
    pub id: String,
    pub platform: String,
    pub version: String,
    pub bytes: u64,
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.chars().enumerate().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

pub fn parse_simctl_runtimes(json_text: &str) -> Vec<Runtime> {
    let Ok(v) = serde_json::from_str::<Value>(json_text) else {
        return Vec::new();
    };
    let Some(map) = v.as_object() else {
        return Vec::new();
    };
    let mut all: Vec<Runtime> = Vec::new();
    let mut newest: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (key, rt) in map {
        let id = rt["identifier"].as_str().unwrap_or(key).to_string();
        let platform = rt["platformIdentifier"]
            .as_str()
            .unwrap_or("")
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_string();
        let version = rt["version"].as_str().unwrap_or("").to_string();
        let e = newest.entry(platform.clone()).or_default();
        if version_key(&version) > version_key(e) {
            *e = version.clone();
        }
        if rt["deletable"] != json!(true) || !is_uuid(&id) {
            continue;
        }
        all.push(Runtime {
            id,
            platform,
            version,
            bytes: rt["sizeBytes"].as_u64().unwrap_or(0),
        });
    }
    all.retain(|r| newest.get(&r.platform) != Some(&r.version));
    all.sort_by(|a, b| a.id.cmp(&b.id));
    all
}

fn version_key(v: &str) -> Vec<u32> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// Local snapshots: `tmutil listlocalsnapshotdates /`. The size is not
/// known without admin; the item says how many there are.
pub fn probe_tm_snapshots(_rule: &Rule, env: &Env) -> Vec<Found> {
    let Some(out) = env.output("tmutil", &["listlocalsnapshotdates", "/"]) else {
        return Vec::new();
    };
    let dates = parse_tm_dates(&out);
    if dates.is_empty() {
        return Vec::new();
    }
    vec![Found {
        path: "tmutil:/".into(),
        label: Some(format!("Local Time Machine snapshots ({})", dates.len())),
        bytes: 0,
        files: dates.len() as u64,
        op: Op::Module {
            module: "clean".into(),
            action: "tmutil_thin".into(),
            args: json!({ "bytes": 999_999_999_999_999u64, "snapshots": dates }),
        },
        admin: true,
    }]
}

/// `2026-09-27-101112` lines after the header.
pub fn parse_tm_dates(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| {
            l.len() >= 17
                && l.as_bytes()[4] == b'-'
                && l.chars().all(|c| c.is_ascii_digit() || c == '-')
        })
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/washmybelly_fixtures/clean")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn unavailable_simulators_are_found_by_data_path() {
        let dirs = parse_simctl_unavailable(&fixture("simctl_unavailable.json"));
        assert_eq!(dirs, vec!["/Users/u/Library/Developer/CoreSimulator/Devices/0A1B2C3D-0000-0000-0000-000000000001"]);
    }

    #[test]
    fn the_newest_runtime_of_each_platform_stays() {
        let rts = parse_simctl_runtimes(&fixture("simctl_runtimes.json"));
        assert_eq!(rts.len(), 1, "{rts:?}");
        assert_eq!(rts[0].version, "17.5");
        assert!(rts[0].bytes > 0);
    }

    #[test]
    fn snapshot_dates_are_parsed() {
        let d =
            parse_tm_dates("Snapshot dates for all disks:\n2026-09-26-101112\n2026-09-27-081500\n");
        assert_eq!(d.len(), 2);
        assert!(parse_tm_dates("Snapshot dates for all disks:\n").is_empty());
    }

    #[test]
    fn no_erase_all_and_no_whole_caches_folder() {
        for r in rules() {
            if let super::super::rule::Action::Command { args, .. } = &r.action {
                assert!(!args.iter().any(|a| a == "erase"), "{}", r.id);
            }
            assert!(
                !r.targets.iter().any(|t| t == "~/Library/Caches"),
                "{}",
                r.id
            );
        }
    }
}
