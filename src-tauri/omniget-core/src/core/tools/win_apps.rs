//! ⚠️ SOMENTE WINDOWS. Debloat (estudo 25, Sophia Script; 10, Kudu): lista os
//! pacotes Appx do usuário via PowerShell, marca os que o Sophia considera
//! bloat e remove com `Remove-AppxPackage`. Uma lista de protegidos nunca é
//! oferecida para remoção (Store, Calculadora, Fotos, runtimes). Fora do
//! Windows a lista é vazia.

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppxPackage {
    pub name: String,
    pub full_name: String,
    pub version: String,
    pub publisher: String,
    /// Está na lista de bloat sugerida (Sophia)
    pub suggested: bool,
    /// Nome amigável quando conhecido
    pub label: String,
    pub non_removable: bool,
}

/// (prefixo do nome do pacote, rótulo) — Sophia Script `UninstallUWPApps`
/// mais os terceiros que o Windows pré-instala por parceria.
const BLOAT: &[(&str, &str)] = &[
    ("Microsoft.BingNews", "Notícias"),
    ("Microsoft.BingWeather", "Clima"),
    ("Microsoft.BingSearch", "Bing Search"),
    ("Microsoft.GetHelp", "Obter Ajuda"),
    ("Microsoft.Getstarted", "Dicas"),
    ("Microsoft.MicrosoftOfficeHub", "Office Hub"),
    ("Microsoft.MicrosoftSolitaireCollection", "Solitaire Collection"),
    ("Microsoft.People", "Pessoas"),
    ("Microsoft.PowerAutomateDesktop", "Power Automate"),
    ("Microsoft.Todos", "To Do"),
    ("Microsoft.WindowsAlarms", "Alarmes e Relógio"),
    ("Microsoft.WindowsFeedbackHub", "Hub de Comentários"),
    ("Microsoft.WindowsMaps", "Mapas"),
    ("Microsoft.YourPhone", "Vincular ao Celular"),
    ("Microsoft.ZuneMusic", "Media Player / Groove"),
    ("Microsoft.ZuneVideo", "Filmes e TV"),
    ("Microsoft.MixedReality.Portal", "Mixed Reality Portal"),
    ("Microsoft.Microsoft3DViewer", "Visualizador 3D"),
    ("Microsoft.SkypeApp", "Skype"),
    ("Microsoft.XboxApp", "Xbox (antigo)"),
    ("Microsoft.Xbox.TCUI", "Xbox TCUI"),
    ("Microsoft.XboxGamingOverlay", "Xbox Game Bar"),
    ("Microsoft.XboxGameOverlay", "Xbox Game Overlay"),
    ("Microsoft.XboxIdentityProvider", "Xbox Identity Provider"),
    ("Microsoft.XboxSpeechToTextOverlay", "Xbox Speech to Text"),
    ("Microsoft.GamingApp", "Xbox"),
    ("Clipchamp.Clipchamp", "Clipchamp"),
    ("MicrosoftTeams", "Teams (pessoal)"),
    ("MSTeams", "Teams"),
    ("Microsoft.OutlookForWindows", "Outlook (novo)"),
    ("Microsoft.549981C3F5F10", "Cortana"),
    ("Microsoft.Wallet", "Carteira"),
    ("Microsoft.WindowsSoundRecorder", "Gravador de Som"),
    ("Microsoft.Copilot", "Copilot"),
    ("Microsoft.Windows.DevHome", "Dev Home"),
    ("Microsoft.MicrosoftJournal", "Journal"),
    ("Microsoft.Whiteboard", "Whiteboard"),
    ("Microsoft.WindowsCommunicationsApps", "Mail e Calendário"),
    ("Microsoft.OneDriveSync", "OneDrive (Store)"),
    ("Microsoft.QuickAssist", "Assistência Rápida"),
    ("Microsoft.Windows.NarratorQuickStart", "Narrador (tutorial)"),
    ("MicrosoftCorporationII.QuickAssist", "Assistência Rápida"),
    ("MicrosoftCorporationII.MicrosoftFamily", "Microsoft Family"),
    ("MicrosoftWindows.CrossDevice", "Cross Device"),
    ("king.com.CandyCrush", "Candy Crush"),
    ("king.com.BubbleWitch", "Bubble Witch"),
    ("SpotifyAB.SpotifyMusic", "Spotify (Store)"),
    ("Disney.", "Disney+"),
    ("BytedancePte.Ltd.TikTok", "TikTok"),
    ("Facebook.", "Facebook"),
    ("Facebook.InstagramBeta", "Instagram"),
    ("AmazonVideo.PrimeVideo", "Prime Video"),
    ("Netflix.", "Netflix"),
    ("7EE7776C.LinkedInforWindows", "LinkedIn"),
    ("5A894077.McAfeeSecurity", "McAfee"),
    ("4DF9E0F8.Netflix", "Netflix"),
    ("Microsoft.Advertising.Xaml", "Advertising Xaml"),
    ("Microsoft.MicrosoftStickyNotes", "Sticky Notes"),
    ("Microsoft.WindowsCamera", "Câmera"),
    ("Microsoft.Windows.Photos", "Fotos"),
    ("Microsoft.MSPaint", "Paint 3D"),
];

/// Nunca oferecidos para remoção: quebram o sistema ou são runtimes.
const PROTECTED: &[&str] = &[
    "Microsoft.WindowsStore",
    "Microsoft.StorePurchaseApp",
    "Microsoft.DesktopAppInstaller",
    "Microsoft.WindowsCalculator",
    "Microsoft.WindowsNotepad",
    "Microsoft.WindowsTerminal",
    "Microsoft.Paint",
    "Microsoft.ScreenSketch",
    "Microsoft.SecHealthUI",
    "Microsoft.VCLibs",
    "Microsoft.UI.Xaml",
    "Microsoft.NET.",
    "Microsoft.WindowsAppRuntime",
    "Microsoft.HEVCVideoExtension",
    "Microsoft.VP9VideoExtensions",
    "Microsoft.AV1VideoExtension",
    "Microsoft.WebMediaExtensions",
    "Microsoft.WebpImageExtension",
    "Microsoft.RawImageExtension",
    "Microsoft.HEIFImageExtension",
    "Microsoft.MPEG2VideoExtension",
    "Microsoft.Windows.ShellExperienceHost",
    "Microsoft.Windows.StartMenuExperienceHost",
    "Microsoft.Windows.Search",
    "Microsoft.AAD.BrokerPlugin",
    "Microsoft.AccountsControl",
    "Microsoft.LockApp",
    "Microsoft.Win32WebViewHost",
    "Microsoft.Windows.CloudExperienceHost",
    "Microsoft.Windows.ContentDeliveryManager",
    "Microsoft.Windows.OOBENetworkCaptivePortal",
    "Microsoft.Windows.OOBENetworkConnectionFlow",
    "Microsoft.Windows.PeopleExperienceHost",
    "Microsoft.Windows.PinningConfirmationDialog",
    "Microsoft.Windows.SecureAssessmentBrowser",
    "Microsoft.Windows.XGpuEjectDialog",
    "Microsoft.XboxGameCallableUI",
    "Microsoft.ECApp",
    "Microsoft.CredDialogHost",
    "Microsoft.BioEnrollment",
    "Microsoft.MicrosoftEdge",
    "Microsoft.Services.Store.Engagement",
    "MicrosoftWindows.Client",
    "Microsoft.Windows.Apprep",
    "Microsoft.Windows.AssignedAccessLockApp",
    "Microsoft.Windows.CapturePicker",
    "Microsoft.Windows.ParentalControls",
    "Microsoft.Windows.PrintQueueActionCenter",
    "Microsoft.Windows.CallingShellApp",
    "Microsoft.Windows.NarratorQuickStart",
    "NcsiUwpApp",
    "windows.immersivecontrolpanel",
    "Windows.PrintDialog",
    "Microsoft.PPIProjection",
    "Microsoft.Windows.Cortana",
    "MicrosoftWindows.UndockedDevKit",
    "Microsoft.WidgetsPlatformRuntime",
    "Microsoft.StartExperiencesApp",
    "Microsoft.SecureAssessmentBrowser",
];

fn is_protected(name: &str) -> bool {
    PROTECTED.iter().any(|p| name.starts_with(p))
}

fn suggestion(name: &str) -> Option<&'static str> {
    BLOAT.iter().find(|(prefix, _)| name.starts_with(prefix)).map(|(_, label)| *label)
}

async fn powershell(script: &str) -> anyhow::Result<String> {
    let o = crate::core::process::command("powershell").args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script]).output().await?;
    if !o.status.success() {
        return Err(anyhow!("PowerShell: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&o.stdout).to_string())
}

pub async fn list() -> anyhow::Result<Vec<AppxPackage>> {
    if !cfg!(target_os = "windows") {
        return Ok(vec![]);
    }
    let out = powershell("Get-AppxPackage | Where-Object { -not $_.IsFramework -and -not $_.IsResourcePackage } | Select-Object Name,PackageFullName,Version,Publisher,NonRemovable | ConvertTo-Json -Compress").await?;
    let json: serde_json::Value = serde_json::from_str(out.trim()).unwrap_or(serde_json::Value::Null);
    let items: Vec<serde_json::Value> = match json {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Object(_) => vec![json],
        _ => vec![],
    };
    let mut pkgs = Vec::new();
    for it in items {
        let name = it.get("Name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if name.is_empty() || is_protected(&name) {
            continue;
        }
        let sug = suggestion(&name);
        pkgs.push(AppxPackage {
            label: sug.map(String::from).unwrap_or_else(|| name.rsplit('.').next().unwrap_or(&name).to_string()),
            suggested: sug.is_some(),
            full_name: it.get("PackageFullName").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            version: it.get("Version").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            publisher: it.get("Publisher").and_then(|v| v.as_str()).map(|p| p.split("O=").nth(1).unwrap_or(p).split(',').next().unwrap_or(p).to_string()).unwrap_or_default(),
            non_removable: it.get("NonRemovable").and_then(|v| v.as_bool()).unwrap_or(false),
            name,
        });
    }
    pkgs.sort_by(|a, b| b.suggested.cmp(&a.suggested).then(a.label.to_lowercase().cmp(&b.label.to_lowercase())));
    Ok(pkgs)
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RemoveResult {
    pub removed: Vec<String>,
    pub failed: Vec<String>,
}

/// Remove os pacotes do usuário atual; com `provisioned`, também impede que
/// voltem para contas novas (precisa de administrador).
pub async fn remove(names: &[String], provisioned: bool, progress: &super::ProgressFn) -> RemoveResult {
    let mut r = RemoveResult::default();
    let total = names.len() as u64;
    for (i, name) in names.iter().enumerate() {
        super::report(progress, "debloat", "progress", i as u64, Some(total), Some(name.clone()));
        if is_protected(name) {
            r.failed.push(format!("{}: protegido", name));
            continue;
        }
        let safe = name.replace('\'', "");
        let mut script = format!("Get-AppxPackage -Name '{}' | Remove-AppxPackage -ErrorAction Stop", safe);
        if provisioned {
            script.push_str(&format!("; Get-AppxProvisionedPackage -Online | Where-Object {{ $_.DisplayName -eq '{}' }} | Remove-AppxProvisionedPackage -Online -ErrorAction SilentlyContinue | Out-Null", safe));
        }
        match powershell(&script).await {
            Ok(_) => r.removed.push(name.clone()),
            Err(e) => r.failed.push(format!("{}: {}", name, e)),
        }
    }
    super::report(progress, "debloat", "done", total, Some(total), None);
    r
}

/// Tenta registrar de novo um pacote removido (só funciona enquanto os
/// arquivos ainda existem em WindowsApps); senão, o caminho é a Store.
pub async fn restore(name: &str) -> anyhow::Result<()> {
    let safe = name.replace('\'', "");
    powershell(&format!(
        "Get-AppxPackage -AllUsers -Name '{}' | ForEach-Object {{ Add-AppxPackage -DisableDevelopmentMode -Register \"$($_.InstallLocation)\\AppXManifest.xml\" -ErrorAction Stop }}",
        safe
    ))
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists() {
        assert!(is_protected("Microsoft.WindowsStore"));
        assert!(is_protected("Microsoft.VCLibs.140.00"));
        assert!(!is_protected("Microsoft.BingNews"));
        assert_eq!(suggestion("Microsoft.XboxGamingOverlay"), Some("Xbox Game Bar"));
        assert_eq!(suggestion("king.com.CandyCrushSaga"), Some("Candy Crush"));
        assert!(suggestion("Contoso.App").is_none());
    }
}
