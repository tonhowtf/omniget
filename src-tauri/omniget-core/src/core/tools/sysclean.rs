//! Limpar caches (estudo 10, Kudu): regras declarativas por sistema, cada uma
//! apontando pastas de cache/temporários/logs de apps, navegadores, jogos e
//! ferramentas de desenvolvimento. Fluxo "varrer → revisar → limpar": nada é
//! apagado sem a lista ter sido mostrada com tamanhos. Só o conteúdo das pastas
//! é removido, nunca a pasta em si (os apps a recriam).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct CleanRule {
    pub id: String,
    /// "system" | "browsers" | "apps" | "dev" | "gaming"
    pub group: String,
    pub name: String,
    /// "safe": cache que o app recria; "review": pode conter algo que o usuário queira.
    pub risk: String,
    pub paths: Vec<String>,
    pub bytes: u64,
    pub files: u64,
}

struct Def {
    id: &'static str,
    group: &'static str,
    name: &'static str,
    risk: &'static str,
    paths: &'static [&'static str],
}

const fn d(
    id: &'static str,
    group: &'static str,
    name: &'static str,
    risk: &'static str,
    paths: &'static [&'static str],
) -> Def {
    Def {
        id,
        group,
        name,
        risk,
        paths,
    }
}

// Caminhos: `~` = home; `%X%` = variável de ambiente; `*` num componente = glob.
#[cfg(target_os = "macos")]
const DEFS: &[Def] = &[
    d("user-caches", "system", "Caches do usuário (~/Library/Caches)", "review", &["~/Library/Caches"]),
    d("user-logs", "system", "Logs do usuário", "safe", &["~/Library/Logs"]),
    d("crash-reports", "system", "Relatórios de erro", "safe", &["~/Library/Logs/DiagnosticReports", "~/Library/Application Support/CrashReporter"]),
    d("quicklook", "system", "Miniaturas do Quick Look", "safe", &["~/Library/Caches/com.apple.QuickLook.thumbnailcache"]),
    d("mail-downloads", "system", "Anexos temporários do Mail", "review", &["~/Library/Containers/com.apple.mail/Data/Library/Mail Downloads"]),
    d("chrome", "browsers", "Google Chrome", "safe", &["~/Library/Caches/Google/Chrome", "~/Library/Application Support/Google/Chrome/*/Cache", "~/Library/Application Support/Google/Chrome/*/Code Cache", "~/Library/Application Support/Google/Chrome/*/GPUCache"]),
    d("brave", "browsers", "Brave", "safe", &["~/Library/Caches/BraveSoftware", "~/Library/Application Support/BraveSoftware/Brave-Browser/*/Cache", "~/Library/Application Support/BraveSoftware/Brave-Browser/*/Code Cache"]),
    d("edge", "browsers", "Microsoft Edge", "safe", &["~/Library/Caches/Microsoft Edge", "~/Library/Application Support/Microsoft Edge/*/Cache", "~/Library/Application Support/Microsoft Edge/*/Code Cache"]),
    d("arc", "browsers", "Arc", "safe", &["~/Library/Caches/Arc", "~/Library/Application Support/Arc/User Data/*/Cache"]),
    d("firefox", "browsers", "Firefox", "safe", &["~/Library/Caches/Firefox/Profiles/*/cache2", "~/Library/Caches/Firefox/Profiles/*/startupCache"]),
    d("safari", "browsers", "Safari", "safe", &["~/Library/Caches/com.apple.Safari", "~/Library/Containers/com.apple.Safari/Data/Library/Caches"]),
    d("spotify", "apps", "Spotify", "safe", &["~/Library/Caches/com.spotify.client", "~/Library/Application Support/Spotify/PersistentCache"]),
    d("discord", "apps", "Discord", "safe", &["~/Library/Application Support/discord/Cache", "~/Library/Application Support/discord/Code Cache", "~/Library/Application Support/discord/GPUCache"]),
    d("slack", "apps", "Slack", "safe", &["~/Library/Application Support/Slack/Cache", "~/Library/Application Support/Slack/Service Worker/CacheStorage", "~/Library/Containers/com.tinyspeck.slackmacgap/Data/Library/Application Support/Slack/Cache"]),
    d("teams", "apps", "Microsoft Teams", "safe", &["~/Library/Containers/com.microsoft.teams2/Data/Library/Caches", "~/Library/Application Support/Microsoft/Teams/Cache"]),
    d("zoom", "apps", "Zoom", "safe", &["~/Library/Application Support/zoom.us/AutoUpdater", "~/Library/Logs/zoom.us"]),
    d("vscode", "apps", "Visual Studio Code", "safe", &["~/Library/Application Support/Code/Cache", "~/Library/Application Support/Code/CachedData", "~/Library/Application Support/Code/CachedExtensionVSIXs", "~/Library/Application Support/Code/logs"]),
    d("adobe", "apps", "Adobe (Media Cache)", "review", &["~/Library/Application Support/Adobe/Common/Media Cache Files", "~/Library/Application Support/Adobe/Common/Media Cache", "~/Library/Application Support/Adobe/Common/Peak Files"]),
    d("telegram", "apps", "Telegram", "review", &["~/Library/Group Containers/6N38VWS5BX.ru.keepcoder.Telegram/appstore/account-*/postbox/media", "~/Library/Application Support/Telegram Desktop/tdata/user_data/cache"]),
    d("whatsapp", "apps", "WhatsApp", "safe", &["~/Library/Group Containers/group.net.whatsapp.WhatsApp.shared/Message/Media/tmp", "~/Library/Containers/net.whatsapp.WhatsApp/Data/Library/Caches"]),
    d("omniget-tmp", "apps", "OmniGet (temporários das tools)", "safe", &["~/Library/Application Support/wtf.tonho.omniget/tools/tmp"]),
    d("xcode-derived", "dev", "Xcode DerivedData", "safe", &["~/Library/Developer/Xcode/DerivedData"]),
    d("xcode-device-support", "dev", "Xcode iOS DeviceSupport", "review", &["~/Library/Developer/Xcode/iOS DeviceSupport", "~/Library/Developer/Xcode/watchOS DeviceSupport", "~/Library/Developer/Xcode/tvOS DeviceSupport"]),
    d("simulator-caches", "dev", "Simulator caches", "safe", &["~/Library/Developer/CoreSimulator/Caches"]),
    d("homebrew", "dev", "Homebrew downloads", "safe", &["~/Library/Caches/Homebrew"]),
    d("npm", "dev", "npm cache", "safe", &["~/.npm/_cacache", "~/.npm/_logs"]),
    d("pnpm", "dev", "pnpm store", "review", &["~/Library/pnpm/store", "~/.local/share/pnpm/store"]),
    d("yarn", "dev", "Yarn cache", "safe", &["~/Library/Caches/Yarn", "~/.yarn/berry/cache"]),
    d("pip", "dev", "pip cache", "safe", &["~/Library/Caches/pip"]),
    d("cargo", "dev", "Cargo registry cache", "safe", &["~/.cargo/registry/cache"]),
    d("gradle", "dev", "Gradle caches", "review", &["~/.gradle/caches"]),
    d("cocoapods", "dev", "CocoaPods cache", "safe", &["~/Library/Caches/CocoaPods"]),
    d("go", "dev", "Go build cache", "safe", &["~/Library/Caches/go-build"]),
    d("composer", "dev", "Composer cache", "safe", &["~/Library/Caches/composer", "~/.composer/cache"]),
    d("docker-desktop-logs", "dev", "Docker Desktop logs", "safe", &["~/Library/Containers/com.docker.docker/Data/log"]),
    d("steam", "gaming", "Steam (http e shader cache)", "safe", &["~/Library/Application Support/Steam/appcache/httpcache", "~/Library/Application Support/Steam/steamapps/shadercache", "~/Library/Application Support/Steam/logs"]),
    d("epic", "gaming", "Epic Games Launcher", "safe", &["~/Library/Caches/com.epicgames.EpicGamesLauncher", "~/Library/Application Support/Epic/EpicGamesLauncher/Saved/webcache"]),
];

#[cfg(target_os = "windows")]
const DEFS: &[Def] = &[
    d(
        "temp-user",
        "system",
        "Temporários do usuário (%TEMP%)",
        "safe",
        &["%TEMP%"],
    ),
    d(
        "temp-windows",
        "system",
        "Temporários do Windows (precisa de administrador)",
        "safe",
        &["%SystemRoot%\\Temp"],
    ),
    d(
        "inetcache",
        "system",
        "Cache do Internet Explorer/WebView",
        "safe",
        &["%LOCALAPPDATA%\\Microsoft\\Windows\\INetCache"],
    ),
    d(
        "thumbcache",
        "system",
        "Cache de miniaturas do Explorer",
        "review",
        &["%LOCALAPPDATA%\\Microsoft\\Windows\\Explorer"],
    ),
    d(
        "crashdumps",
        "system",
        "Despejos de falhas",
        "safe",
        &[
            "%LOCALAPPDATA%\\CrashDumps",
            "%LOCALAPPDATA%\\Microsoft\\Windows\\WER",
        ],
    ),
    d(
        "windows-update",
        "system",
        "Downloads do Windows Update (precisa de administrador)",
        "review",
        &["%SystemRoot%\\SoftwareDistribution\\Download"],
    ),
    d(
        "prefetch",
        "system",
        "Prefetch (precisa de administrador)",
        "review",
        &["%SystemRoot%\\Prefetch"],
    ),
    d(
        "delivery-opt",
        "system",
        "Otimização de Entrega",
        "safe",
        &["%SystemRoot%\\SoftwareDistribution\\DeliveryOptimization"],
    ),
    d(
        "d3d-cache",
        "system",
        "Cache de shaders DirectX",
        "safe",
        &["%LOCALAPPDATA%\\D3DSCache"],
    ),
    d(
        "nvidia-cache",
        "system",
        "Cache de shaders NVIDIA",
        "safe",
        &[
            "%LOCALAPPDATA%\\NVIDIA\\DXCache",
            "%LOCALAPPDATA%\\NVIDIA\\GLCache",
            "%LOCALAPPDATA%\\NVIDIA Corporation\\NV_Cache",
        ],
    ),
    d(
        "amd-cache",
        "system",
        "Cache de shaders AMD",
        "safe",
        &[
            "%LOCALAPPDATA%\\AMD\\DxCache",
            "%LOCALAPPDATA%\\AMD\\GLCache",
            "%LOCALAPPDATA%\\AMD\\VkCache",
        ],
    ),
    d(
        "intel-cache",
        "system",
        "Cache de shaders Intel",
        "safe",
        &["%LOCALAPPDATA%\\Intel\\ShaderCache"],
    ),
    d(
        "chrome",
        "browsers",
        "Google Chrome",
        "safe",
        &[
            "%LOCALAPPDATA%\\Google\\Chrome\\User Data\\*\\Cache",
            "%LOCALAPPDATA%\\Google\\Chrome\\User Data\\*\\Code Cache",
            "%LOCALAPPDATA%\\Google\\Chrome\\User Data\\*\\GPUCache",
            "%LOCALAPPDATA%\\Google\\Chrome\\User Data\\ShaderCache",
        ],
    ),
    d(
        "edge",
        "browsers",
        "Microsoft Edge",
        "safe",
        &[
            "%LOCALAPPDATA%\\Microsoft\\Edge\\User Data\\*\\Cache",
            "%LOCALAPPDATA%\\Microsoft\\Edge\\User Data\\*\\Code Cache",
            "%LOCALAPPDATA%\\Microsoft\\Edge\\User Data\\*\\GPUCache",
        ],
    ),
    d(
        "brave",
        "browsers",
        "Brave",
        "safe",
        &[
            "%LOCALAPPDATA%\\BraveSoftware\\Brave-Browser\\User Data\\*\\Cache",
            "%LOCALAPPDATA%\\BraveSoftware\\Brave-Browser\\User Data\\*\\Code Cache",
        ],
    ),
    d(
        "firefox",
        "browsers",
        "Firefox",
        "safe",
        &[
            "%LOCALAPPDATA%\\Mozilla\\Firefox\\Profiles\\*\\cache2",
            "%LOCALAPPDATA%\\Mozilla\\Firefox\\Profiles\\*\\startupCache",
        ],
    ),
    d(
        "opera",
        "browsers",
        "Opera / Opera GX",
        "safe",
        &[
            "%APPDATA%\\Opera Software\\Opera Stable\\Cache",
            "%APPDATA%\\Opera Software\\Opera GX Stable\\Cache",
            "%LOCALAPPDATA%\\Opera Software\\Opera Stable\\Cache",
        ],
    ),
    d(
        "vivaldi",
        "browsers",
        "Vivaldi",
        "safe",
        &[
            "%LOCALAPPDATA%\\Vivaldi\\User Data\\*\\Cache",
            "%LOCALAPPDATA%\\Vivaldi\\User Data\\*\\Code Cache",
        ],
    ),
    d(
        "spotify",
        "apps",
        "Spotify",
        "safe",
        &[
            "%LOCALAPPDATA%\\Spotify\\Storage",
            "%LOCALAPPDATA%\\Spotify\\Data",
            "%APPDATA%\\Spotify\\Storage",
        ],
    ),
    d(
        "discord",
        "apps",
        "Discord",
        "safe",
        &[
            "%APPDATA%\\discord\\Cache",
            "%APPDATA%\\discord\\Code Cache",
            "%APPDATA%\\discord\\GPUCache",
        ],
    ),
    d(
        "slack",
        "apps",
        "Slack",
        "safe",
        &[
            "%APPDATA%\\Slack\\Cache",
            "%APPDATA%\\Slack\\Code Cache",
            "%APPDATA%\\Slack\\GPUCache",
            "%APPDATA%\\Slack\\logs",
        ],
    ),
    d(
        "teams",
        "apps",
        "Microsoft Teams",
        "safe",
        &[
            "%APPDATA%\\Microsoft\\Teams\\Cache",
            "%APPDATA%\\Microsoft\\Teams\\Code Cache",
            "%APPDATA%\\Microsoft\\Teams\\GPUCache",
            "%LOCALAPPDATA%\\Packages\\MSTeams_8wekyb3d8bbwe\\LocalCache\\Microsoft\\MSTeams\\Logs",
        ],
    ),
    d("zoom", "apps", "Zoom", "safe", &["%APPDATA%\\Zoom\\logs"]),
    d(
        "vscode",
        "apps",
        "Visual Studio Code",
        "safe",
        &[
            "%APPDATA%\\Code\\Cache",
            "%APPDATA%\\Code\\CachedData",
            "%APPDATA%\\Code\\CachedExtensionVSIXs",
            "%APPDATA%\\Code\\logs",
        ],
    ),
    d(
        "adobe",
        "apps",
        "Adobe (Media Cache)",
        "review",
        &[
            "%APPDATA%\\Adobe\\Common\\Media Cache Files",
            "%APPDATA%\\Adobe\\Common\\Media Cache",
            "%APPDATA%\\Adobe\\Common\\Peak Files",
        ],
    ),
    d(
        "telegram",
        "apps",
        "Telegram Desktop",
        "review",
        &[
            "%APPDATA%\\Telegram Desktop\\tdata\\user_data\\cache",
            "%APPDATA%\\Telegram Desktop\\tdata\\user_data\\media_cache",
        ],
    ),
    d(
        "whatsapp",
        "apps",
        "WhatsApp",
        "safe",
        &[
            "%LOCALAPPDATA%\\WhatsApp\\Cache",
            "%LOCALAPPDATA%\\WhatsApp\\Code Cache",
        ],
    ),
    d(
        "omniget-tmp",
        "apps",
        "OmniGet (temporários das tools)",
        "safe",
        &["%APPDATA%\\wtf.tonho.omniget\\tools\\tmp"],
    ),
    d(
        "npm",
        "dev",
        "npm cache",
        "safe",
        &["%LOCALAPPDATA%\\npm-cache"],
    ),
    d(
        "pnpm",
        "dev",
        "pnpm store",
        "review",
        &["%LOCALAPPDATA%\\pnpm\\store"],
    ),
    d(
        "yarn",
        "dev",
        "Yarn cache",
        "safe",
        &["%LOCALAPPDATA%\\Yarn\\Cache"],
    ),
    d(
        "pip",
        "dev",
        "pip cache",
        "safe",
        &["%LOCALAPPDATA%\\pip\\cache"],
    ),
    d(
        "cargo",
        "dev",
        "Cargo registry cache",
        "safe",
        &["%USERPROFILE%\\.cargo\\registry\\cache"],
    ),
    d(
        "gradle",
        "dev",
        "Gradle caches",
        "review",
        &["%USERPROFILE%\\.gradle\\caches"],
    ),
    d(
        "nuget",
        "dev",
        "NuGet cache",
        "safe",
        &[
            "%LOCALAPPDATA%\\NuGet\\v3-cache",
            "%LOCALAPPDATA%\\NuGet\\Cache",
        ],
    ),
    d(
        "go",
        "dev",
        "Go build cache",
        "safe",
        &["%LOCALAPPDATA%\\go-build"],
    ),
    d(
        "composer",
        "dev",
        "Composer cache",
        "safe",
        &["%LOCALAPPDATA%\\Composer"],
    ),
    d(
        "steam",
        "gaming",
        "Steam (http e shader cache)",
        "safe",
        &[
            "%ProgramFiles(x86)%\\Steam\\appcache\\httpcache",
            "%ProgramFiles(x86)%\\Steam\\steamapps\\shadercache",
            "%ProgramFiles(x86)%\\Steam\\logs",
            "%ProgramFiles(x86)%\\Steam\\dumps",
        ],
    ),
    d(
        "epic",
        "gaming",
        "Epic Games Launcher",
        "safe",
        &[
            "%LOCALAPPDATA%\\EpicGamesLauncher\\Saved\\webcache",
            "%LOCALAPPDATA%\\EpicGamesLauncher\\Saved\\Logs",
        ],
    ),
    d(
        "battlenet",
        "gaming",
        "Battle.net",
        "safe",
        &[
            "%ProgramData%\\Battle.net\\Agent\\data\\cache",
            "%LOCALAPPDATA%\\Battle.net\\Cache",
        ],
    ),
    d(
        "riot",
        "gaming",
        "Riot Client",
        "safe",
        &["%LOCALAPPDATA%\\Riot Games\\Riot Client\\Logs"],
    ),
    d(
        "ubisoft",
        "gaming",
        "Ubisoft Connect",
        "safe",
        &[
            "%LOCALAPPDATA%\\Ubisoft Game Launcher\\cache",
            "%LOCALAPPDATA%\\Ubisoft Game Launcher\\logs",
        ],
    ),
];

#[cfg(all(unix, not(target_os = "macos")))]
const DEFS: &[Def] = &[
    d(
        "user-cache",
        "system",
        "Cache do usuário (~/.cache)",
        "review",
        &["~/.cache"],
    ),
    d(
        "thumbnails",
        "system",
        "Miniaturas",
        "safe",
        &["~/.cache/thumbnails", "~/.thumbnails"],
    ),
    d(
        "shader-cache",
        "system",
        "Cache de shaders (Mesa/NVIDIA)",
        "safe",
        &[
            "~/.cache/mesa_shader_cache",
            "~/.cache/mesa_shader_cache_db",
            "~/.cache/nvidia",
            "~/.nv/GLCache",
        ],
    ),
    d(
        "crash",
        "system",
        "Relatórios de erro",
        "safe",
        &["~/.cache/abrt", "~/.local/share/apport"],
    ),
    d(
        "chrome",
        "browsers",
        "Google Chrome",
        "safe",
        &[
            "~/.cache/google-chrome",
            "~/.var/app/com.google.Chrome/cache",
        ],
    ),
    d(
        "chromium",
        "browsers",
        "Chromium",
        "safe",
        &[
            "~/.cache/chromium",
            "~/.var/app/org.chromium.Chromium/cache",
            "~/snap/chromium/common/.cache",
        ],
    ),
    d(
        "brave",
        "browsers",
        "Brave",
        "safe",
        &[
            "~/.cache/BraveSoftware",
            "~/.var/app/com.brave.Browser/cache",
        ],
    ),
    d(
        "edge",
        "browsers",
        "Microsoft Edge",
        "safe",
        &[
            "~/.cache/microsoft-edge",
            "~/.var/app/com.microsoft.Edge/cache",
        ],
    ),
    d(
        "firefox",
        "browsers",
        "Firefox",
        "safe",
        &[
            "~/.cache/mozilla/firefox/*/cache2",
            "~/.var/app/org.mozilla.firefox/cache/mozilla/firefox/*/cache2",
            "~/snap/firefox/common/.cache/mozilla/firefox/*/cache2",
        ],
    ),
    d(
        "vivaldi",
        "browsers",
        "Vivaldi",
        "safe",
        &["~/.cache/vivaldi"],
    ),
    d("opera", "browsers", "Opera", "safe", &["~/.cache/opera"]),
    d(
        "spotify",
        "apps",
        "Spotify",
        "safe",
        &[
            "~/.cache/spotify",
            "~/.var/app/com.spotify.Client/cache",
            "~/snap/spotify/common/.cache",
        ],
    ),
    d(
        "discord",
        "apps",
        "Discord",
        "safe",
        &[
            "~/.config/discord/Cache",
            "~/.config/discord/Code Cache",
            "~/.config/discord/GPUCache",
            "~/.var/app/com.discordapp.Discord/config/discord/Cache",
            "~/.var/app/com.discordapp.Discord/config/discord/Code Cache",
        ],
    ),
    d(
        "slack",
        "apps",
        "Slack",
        "safe",
        &[
            "~/.config/Slack/Cache",
            "~/.config/Slack/Code Cache",
            "~/.var/app/com.slack.Slack/config/Slack/Cache",
        ],
    ),
    d(
        "vscode",
        "apps",
        "Visual Studio Code",
        "safe",
        &[
            "~/.config/Code/Cache",
            "~/.config/Code/CachedData",
            "~/.config/Code/CachedExtensionVSIXs",
            "~/.config/Code/logs",
            "~/.var/app/com.visualstudio.code/config/Code/Cache",
        ],
    ),
    d(
        "telegram",
        "apps",
        "Telegram Desktop",
        "review",
        &[
            "~/.local/share/TelegramDesktop/tdata/user_data/cache",
            "~/.var/app/org.telegram.desktop/data/TelegramDesktop/tdata/user_data/cache",
        ],
    ),
    d(
        "omniget-tmp",
        "apps",
        "OmniGet (temporários das tools)",
        "safe",
        &[
            "~/.local/share/wtf.tonho.omniget/tools/tmp",
            "~/.var/app/wtf.tonho.omniget/data/wtf.tonho.omniget/tools/tmp",
        ],
    ),
    d(
        "flatpak-unused",
        "apps",
        "Flatpak (.removed)",
        "safe",
        &["~/.local/share/flatpak/.removed"],
    ),
    d(
        "npm",
        "dev",
        "npm cache",
        "safe",
        &["~/.npm/_cacache", "~/.npm/_logs"],
    ),
    d(
        "pnpm",
        "dev",
        "pnpm store",
        "review",
        &["~/.local/share/pnpm/store", "~/.cache/pnpm"],
    ),
    d(
        "yarn",
        "dev",
        "Yarn cache",
        "safe",
        &["~/.cache/yarn", "~/.yarn/berry/cache"],
    ),
    d("pip", "dev", "pip cache", "safe", &["~/.cache/pip"]),
    d(
        "cargo",
        "dev",
        "Cargo registry cache",
        "safe",
        &["~/.cargo/registry/cache"],
    ),
    d(
        "gradle",
        "dev",
        "Gradle caches",
        "review",
        &["~/.gradle/caches"],
    ),
    d(
        "go",
        "dev",
        "Go build cache",
        "safe",
        &["~/.cache/go-build"],
    ),
    d(
        "composer",
        "dev",
        "Composer cache",
        "safe",
        &["~/.cache/composer"],
    ),
    d(
        "steam",
        "gaming",
        "Steam (http e shader cache)",
        "safe",
        &[
            "~/.local/share/Steam/appcache/httpcache",
            "~/.local/share/Steam/steamapps/shadercache",
            "~/.local/share/Steam/logs",
            "~/.steam/steam/appcache/httpcache",
            "~/.steam/steam/steamapps/shadercache",
            "~/.var/app/com.valvesoftware.Steam/data/Steam/appcache/httpcache",
            "~/.var/app/com.valvesoftware.Steam/data/Steam/steamapps/shadercache",
        ],
    ),
    d("lutris", "gaming", "Lutris", "safe", &["~/.cache/lutris"]),
    d(
        "heroic",
        "gaming",
        "Heroic Games Launcher",
        "safe",
        &[
            "~/.config/heroic/Cache",
            "~/.var/app/com.heroicgameslauncher.hgl/config/heroic/Cache",
        ],
    ),
];

fn expand_vars(p: &str) -> String {
    let mut out = String::new();
    let mut rest = p;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('%') {
            let var = &after[..end];
            match std::env::var(var) {
                Ok(v) => out.push_str(&v),
                Err(_) => {
                    out.push('%');
                    out.push_str(var);
                    out.push('%');
                }
            }
            rest = &after[end + 1..];
        } else {
            out.push('%');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Expande `~`, `%VAR%` e `*` (um nível) em caminhos existentes.
pub fn expand(pattern: &str) -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let p = if let Some(rest) = pattern.strip_prefix("~/") {
        home.join(rest).to_string_lossy().to_string()
    } else if pattern == "~" {
        home.to_string_lossy().to_string()
    } else {
        expand_vars(pattern)
    };
    if p.contains('%') {
        return vec![];
    }
    let mut current: Vec<PathBuf> = vec![PathBuf::new()];
    for comp in Path::new(&p).components() {
        let s = comp.as_os_str().to_string_lossy().to_string();
        if s.contains('*') {
            let mut next = Vec::new();
            for base in &current {
                if let Ok(rd) = std::fs::read_dir(if base.as_os_str().is_empty() {
                    Path::new("/")
                } else {
                    base.as_path()
                }) {
                    for e in rd.flatten() {
                        let name = e.file_name().to_string_lossy().to_string();
                        if glob_match(&s, &name) {
                            next.push(base.join(&name));
                        }
                    }
                }
            }
            current = next;
        } else {
            for c in current.iter_mut() {
                c.push(comp);
            }
        }
        if current.is_empty() {
            break;
        }
    }
    current.into_iter().filter(|c| c.exists()).collect()
}

fn glob_match(pat: &str, name: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == name;
    }
    let mut pos = 0usize;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !name.starts_with(part) {
                return false;
            }
            pos = part.len();
        } else if i == parts.len() - 1 {
            return name[pos..].ends_with(part);
        } else if let Some(idx) = name[pos..].find(part) {
            pos += idx + part.len();
        } else {
            return false;
        }
    }
    true
}

/// Tamanho e número de arquivos abaixo de um caminho (sem seguir links).
pub fn measure(path: &Path) -> (u64, u64) {
    let mut bytes = 0u64;
    let mut files = 0u64;
    if path.is_file() {
        return (std::fs::metadata(path).map(|m| m.len()).unwrap_or(0), 1);
    }
    for entry in walkdir::WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .flatten()
    {
        if entry.file_type().is_file() {
            files += 1;
            bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    (bytes, files)
}

pub fn scan(progress: &super::ProgressFn) -> Vec<CleanRule> {
    let total = DEFS.len() as u64;
    let mut out = Vec::new();
    for (i, def) in DEFS.iter().enumerate() {
        super::report(
            progress,
            "sysclean",
            "progress",
            i as u64,
            Some(total),
            Some(def.name.to_string()),
        );
        let mut paths = Vec::new();
        let mut bytes = 0u64;
        let mut files = 0u64;
        for pat in def.paths {
            for p in expand(pat) {
                let (b, f) = measure(&p);
                if f == 0 {
                    continue;
                }
                bytes += b;
                files += f;
                paths.push(p.to_string_lossy().to_string());
            }
        }
        if paths.is_empty() {
            continue;
        }
        out.push(CleanRule {
            id: def.id.into(),
            group: def.group.into(),
            name: def.name.into(),
            risk: def.risk.into(),
            paths,
            bytes,
            files,
        });
    }
    super::report(progress, "sysclean", "done", total, Some(total), None);
    out.sort_by_key(|b| std::cmp::Reverse(b.bytes));
    out
}

#[derive(Debug, Clone, Deserialize)]
pub struct CleanRequest {
    pub ids: Vec<String>,
    #[serde(default)]
    pub to_trash: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct CleanResult {
    pub freed: u64,
    pub removed: u64,
    pub failed: Vec<String>,
}

fn remove_entry(path: &Path, to_trash: bool, result: &mut CleanResult) {
    let (bytes, files) = measure(path);
    let ok = if to_trash {
        trash::delete(path).is_ok()
    } else if path.is_dir() && !path.is_symlink() {
        std::fs::remove_dir_all(path).is_ok()
    } else {
        std::fs::remove_file(path).is_ok()
    };
    if ok {
        result.freed += bytes;
        result.removed += files.max(1);
    } else {
        result.failed.push(path.to_string_lossy().to_string());
    }
}

/// Apaga o conteúdo das pastas das regras escolhidas (não as pastas).
pub fn clean(req: &CleanRequest, progress: &super::ProgressFn) -> CleanResult {
    let mut result = CleanResult::default();
    let defs: Vec<&Def> = DEFS
        .iter()
        .filter(|d| req.ids.iter().any(|id| id == d.id))
        .collect();
    let total = defs.len() as u64;
    for (i, def) in defs.iter().enumerate() {
        super::report(
            progress,
            "sysclean",
            "clean",
            i as u64,
            Some(total),
            Some(def.name.to_string()),
        );
        for pat in def.paths {
            for root in expand(pat) {
                if root.is_file() {
                    remove_entry(&root, req.to_trash, &mut result);
                    continue;
                }
                let Ok(rd) = std::fs::read_dir(&root) else {
                    continue;
                };
                for e in rd.flatten() {
                    remove_entry(&e.path(), req.to_trash, &mut result);
                }
            }
        }
    }
    super::report(progress, "sysclean", "done", total, Some(total), None);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob() {
        assert!(glob_match("*", "abc"));
        assert!(glob_match("a*", "abc"));
        assert!(glob_match("*.default", "xyz.default"));
        assert!(glob_match("account-*", "account-123"));
        assert!(!glob_match("a*c", "abd"));
        assert!(glob_match("a*c", "abc"));
    }

    #[test]
    fn vars() {
        std::env::set_var("OMNIGET_T", "/x");
        assert_eq!(expand_vars("%OMNIGET_T%/y"), "/x/y");
        assert_eq!(expand_vars("%NOPE_NOPE%/y"), "%NOPE_NOPE%/y");
        assert!(expand("%NOPE_NOPE%/y").is_empty());
    }

    #[test]
    fn measure_and_clean() {
        let dir = std::env::temp_dir().join(format!("omniget-clean-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.bin"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.join("sub/b.bin"), vec![0u8; 50]).unwrap();
        assert_eq!(measure(&dir), (150, 2));
        let mut r = CleanResult::default();
        remove_entry(&dir.join("sub"), false, &mut r);
        assert_eq!(r.freed, 50);
        assert!(!dir.join("sub").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod live {
    /// `cargo test -p omniget-core --lib tools::sysclean::live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn scan_here() {
        let rules = super::scan(&super::super::noop_progress());
        for r in &rules {
            println!(
                "{:<9} {:<45} {:>12} {:>7} {}",
                r.group,
                r.name,
                r.bytes,
                r.files,
                r.paths.len()
            );
        }
        println!("{} regras com conteudo", rules.len());
    }
}
