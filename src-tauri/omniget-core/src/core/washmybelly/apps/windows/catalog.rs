//! Which Appx packages may go, and how to bring them back.
//!
//! Data, converted by `scripts/washmybelly/win-apps-convert.mjs` and embedded:
//! - Win11Debloat `Apps.json` (MIT, Raphire): safe / optional / dangerous;
//! - Sophia Script `$ExcludedAppxPackages` (MIT, farag2): never removed;
//! - winutil `appx.json` (MIT, CT Tech Group LLC): Store id to reinstall.
//!
//! Order: never (Sophia, our own list, System signature, NonRemovable)
//! wins over any recommendation; then Win11Debloat; anything else the
//! person installed is `optional`.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

const WIN11DEBLOAT: &str = include_str!("../../../../../data/washmybelly/win11debloat/apps.json");
const SOPHIA: &str = include_str!("../../../../../data/washmybelly/sophia/excluded_appx.json");
const WINUTIL: &str = include_str!("../../../../../data/washmybelly/winutil/appx.json");

/// Shell hosts and system pieces that are signed as Store apps on some
/// builds but break Windows when removed (OmniGet's own list).
const OURS_NEVER: &[&str] = &[
    "Microsoft.SecHealthUI",
    "Microsoft.AAD.BrokerPlugin",
    "Microsoft.AccountsControl",
    "Microsoft.LockApp",
    "Microsoft.Win32WebViewHost",
    "Microsoft.Windows.ShellExperienceHost",
    "Microsoft.Windows.StartMenuExperienceHost",
    "Microsoft.Windows.Search",
    "Microsoft.Windows.CloudExperienceHost",
    "Microsoft.Windows.ContentDeliveryManager",
    "Microsoft.Windows.OOBENetworkCaptivePortal",
    "Microsoft.Windows.OOBENetworkConnectionFlow",
    "Microsoft.Windows.PeopleExperienceHost",
    "Microsoft.Windows.PinningConfirmationDialog",
    "Microsoft.Windows.XGpuEjectDialog",
    "Microsoft.Windows.Apprep.ChxApp",
    "Microsoft.Windows.AssignedAccessLockApp",
    "Microsoft.Windows.CapturePicker",
    "Microsoft.Windows.ParentalControls",
    "Microsoft.Windows.PrintQueueActionCenter",
    "Microsoft.Windows.CallingShellApp",
    "Microsoft.Windows.NarratorQuickStart",
    "Microsoft.ECApp",
    "Microsoft.CredDialogHost",
    "Microsoft.BioEnrollment",
    "Microsoft.MicrosoftEdge",
    "Microsoft.MicrosoftEdge.Stable",
    "Microsoft.MicrosoftEdgeDevToolsClient",
    "Microsoft.XboxGameCallableUI",
    "Microsoft.Services.Store.Engagement",
    "MicrosoftWindows.Client.CBS",
    "MicrosoftWindows.Client.Core",
    "MicrosoftWindows.Client.FileExp",
    "MicrosoftWindows.UndockedDevKit",
    "NcsiUwpApp",
    "windows.immersivecontrolpanel",
    "Windows.PrintDialog",
];

/// Name prefixes that are always part of Windows.
const OURS_NEVER_PREFIX: &[&str] = &[
    "Microsoft.WindowsAppRuntime",
    "Microsoft.VCLibs",
    "Microsoft.UI.Xaml",
    "Microsoft.NET.",
    "MicrosoftWindows.Client.CBS",
];

#[derive(Debug, Clone, Deserialize)]
struct WdFile {
    apps: Vec<WdApp>,
}

#[derive(Debug, Clone, Deserialize)]
struct WdApp {
    ids: Vec<String>,
    name: String,
    #[serde(default)]
    description: String,
    recommendation: String,
    removal: String,
    #[serde(default)]
    default: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct SophiaFile {
    never: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WinutilFile {
    store_ids: HashMap<String, String>,
}

struct Catalog {
    apps: Vec<WdApp>,
    never: Vec<String>,
    store_ids: HashMap<String, String>,
}

fn catalog() -> &'static Catalog {
    static C: OnceLock<Catalog> = OnceLock::new();
    C.get_or_init(|| {
        let wd: WdFile = serde_json::from_str(WIN11DEBLOAT).expect("win11debloat/apps.json");
        let so: SophiaFile = serde_json::from_str(SOPHIA).expect("sophia/excluded_appx.json");
        let wu: WinutilFile = serde_json::from_str(WINUTIL).expect("winutil/appx.json");
        Catalog {
            apps: wd.apps,
            never: so.never,
            store_ids: wu
                .store_ids
                .into_iter()
                .map(|(k, v)| (k.to_ascii_lowercase(), v))
                .collect(),
        }
    })
}

/// How an Appx package is treated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppxClass {
    /// `safe`, `optional`, `dangerous`, `never`.
    pub classification: &'static str,
    /// Where the classification came from: `win11debloat`, `sophia`,
    /// `omniget`, `signature`, `default`.
    pub source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friendly_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Microsoft Store id to reinstall it (`winget install --source msstore`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_id: Option<String>,
    /// Win11Debloat ticks it by default.
    pub suggested: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
}

fn wd_match<'a>(name: &str) -> Option<&'a WdApp> {
    let c = catalog();
    let lower = name.to_ascii_lowercase();
    let appx = || c.apps.iter().filter(|a| a.removal == "appx");
    appx()
        .find(|a| a.ids.iter().any(|id| id.eq_ignore_ascii_case(name)))
        // Win11Debloat matches `*<id>*`; we only allow that for ids without
        // a dot (`Flipboard` → `Flipboard.Flipboard`).
        .or_else(|| {
            appx().find(|a| {
                a.ids.iter().any(|id| {
                    !id.contains('.') && id.len() >= 5 && lower.contains(&id.to_ascii_lowercase())
                })
            })
        })
}

/// Store id for a package name (winutil).
pub fn store_id(name: &str) -> Option<String> {
    catalog().store_ids.get(&name.to_ascii_lowercase()).cloned()
}

/// Classify one package. `signature` is `SignatureKind`.
pub fn classify_appx(name: &str, signature: &str, non_removable: bool) -> AppxClass {
    let c = catalog();
    let wd = wd_match(name);
    let base = AppxClass {
        classification: "optional",
        source: "default",
        friendly_name: wd.map(|a| a.name.clone()),
        description: wd.map(|a| a.description.clone()).filter(|d| !d.is_empty()),
        store_id: store_id(name),
        suggested: false,
        reason: None,
    };
    let never = |source: &'static str, reason: &'static str| AppxClass {
        classification: "never",
        source,
        reason: Some(reason),
        suggested: false,
        ..base.clone()
    };
    if c.never.iter().any(|n| n.eq_ignore_ascii_case(name)) {
        return never(
            "sophia",
            "Windows or a driver needs it (codecs, Store, winget, control panels)",
        );
    }
    if OURS_NEVER.iter().any(|n| n.eq_ignore_ascii_case(name))
        || OURS_NEVER_PREFIX.iter().any(|p| {
            name.to_ascii_lowercase()
                .starts_with(&p.to_ascii_lowercase())
        })
    {
        return never("omniget", "part of Windows");
    }
    if non_removable {
        return never("signature", "Windows marks it as not removable");
    }
    if signature.eq_ignore_ascii_case("System") && wd.is_none() {
        return never("signature", "part of Windows");
    }
    match wd {
        Some(a) => AppxClass {
            classification: match a.recommendation.as_str() {
                "safe" => "safe",
                "dangerous" => "dangerous",
                _ => "optional",
            },
            source: "win11debloat",
            suggested: a.default && a.recommendation != "dangerous",
            ..base
        },
        None => base,
    }
}

/// Win32 apps (registry or winget) that are never uninstalled: removing
/// them breaks Windows or other apps.
pub fn registry_never(
    name: &str,
    publisher: Option<&str>,
    winget_id: Option<&str>,
) -> Option<&'static str> {
    const IDS: &[&str] = &[
        "Microsoft.Edge",
        "Microsoft.EdgeWebView2Runtime",
        "Microsoft.AppInstaller",
        "Microsoft.EdgeWebView2",
    ];
    if winget_id.is_some_and(|id| IDS.iter().any(|x| x.eq_ignore_ascii_case(id))) {
        return Some("part of Windows: other apps need it");
    }
    let microsoft = publisher.is_some_and(|p| p.to_ascii_lowercase().starts_with("microsoft"));
    let n = name.to_ascii_lowercase();
    if microsoft
        && (n == "microsoft edge"
            || n.starts_with("microsoft edge webview2")
            || n == "microsoft edge update")
    {
        return Some("part of Windows: other apps need it");
    }
    // Win11Debloat marks these winget removals as dangerous.
    if let Some(id) = winget_id {
        if catalog().apps.iter().any(|a| {
            a.removal == "winget"
                && a.recommendation == "dangerous"
                && a.ids.iter().any(|x| x.eq_ignore_ascii_case(id))
        }) {
            return Some("removing it breaks parts of Windows");
        }
    }
    None
}
