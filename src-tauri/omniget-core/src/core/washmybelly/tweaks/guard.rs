//! What the registry side of washmybelly may touch.
//!
//! Two lists, checked both when the catalog loads and again inside the
//! elevated helper (which trusts nothing from the plan):
//!
//! - roots: the only places an administrator write can land
//!   (`HKLM\SOFTWARE`, `HKLM\SYSTEM\CurrentControlSet\Services|Control`, the
//!   signed-in user's hive by SID, NetworkService's Delivery Optimization key);
//! - forbidden: keys and values washmybelly never changes, from the plan's
//!   "left out" list: Defender, CPU mitigations, VBS (memory integrity is the
//!   one exception, as an `extreme` item), UCPD, Smart App Control, BitLocker,
//!   LSA, Safe Boot, auto-start hijack points, and the placebo tweaks.
//!
//! A whole key is deleted only when the uninstaller's own check
//! (`apps::windows::run::reg_key_ok`) accepts it, when a catalog `.reg`
//! deletes exactly that key, or when it is a key a tweak created (a catalog
//! key or an ancestor of one, deep enough and not a shared parent).

/// Keys with quotes, control characters, empty parts or `..` never pass.
pub fn plain_key(key: &str) -> Result<(), String> {
    let k = key.trim();
    if k.is_empty()
        || k.contains(['"', '\0', '\n', '\r'])
        || k.starts_with('\\')
        || k.ends_with('\\')
        || k.contains("\\\\")
        || k.split('\\')
            .any(|p| p == ".." || p == "." || p.trim().is_empty())
    {
        return Err(format!("PROTECTED: {key:?} is not a plain registry key"));
    }
    Ok(())
}

/// `HKEY_LOCAL_MACHINE\x` → `HKLM\X` (upper case, short hive, no `/reg`).
pub fn normalize(full: &str) -> Result<String, String> {
    plain_key(full)?;
    let (root, rest) = full.split_once('\\').unwrap_or((full, ""));
    let hive = match root.to_ascii_uppercase().as_str() {
        "HKEY_LOCAL_MACHINE" | "HKLM" => "HKLM",
        "HKEY_CURRENT_USER" | "HKCU" => "HKCU",
        "HKEY_USERS" | "HKU" => "HKU",
        "HKEY_CLASSES_ROOT" | "HKCR" => {
            return Ok(format!(
                "HKLM\\SOFTWARE\\CLASSES\\{}",
                rest.to_ascii_uppercase()
            ))
        }
        other => return Err(format!("PROTECTED: hive {other} is not allowed")),
    };
    if rest.is_empty() {
        return Ok(hive.to_string());
    }
    Ok(format!("{hive}\\{}", rest.to_ascii_uppercase()))
}

/// Upper-case component prefix test (`A\B` is under `A`, not under `A\BC`).
fn under(key: &str, prefix: &str) -> bool {
    key == prefix
        || key
            .strip_prefix(prefix)
            .is_some_and(|r| r.starts_with('\\'))
}

/// A user SID (`S-1-5-21-…` local/domain, `S-1-12-1-…` Entra ID).
pub fn user_sid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    let digits = |p: &[&str]| {
        p.iter()
            .all(|x| !x.is_empty() && x.chars().all(|c| c.is_ascii_digit()))
    };
    (s.starts_with("S-1-5-21-") && parts.len() == 8 && digits(&parts[3..]))
        || (s.starts_with("S-1-12-1-") && parts.len() == 8 && digits(&parts[4..]))
}

const USER_ROOTS: &[&str] = &["SOFTWARE", "CONTROL PANEL", "SYSTEM\\GAMECONFIGSTORE"];

const MACHINE_ROOTS: &[&str] = &[
    "HKLM\\SOFTWARE",
    "HKLM\\SYSTEM\\CURRENTCONTROLSET\\SERVICES",
    "HKLM\\SYSTEM\\CURRENTCONTROLSET\\CONTROL",
    "HKU\\S-1-5-20\\SOFTWARE\\MICROSOFT\\WINDOWS\\CURRENTVERSION\\DELIVERYOPTIMIZATION",
];

/// Inside one of the roots above (normalized key).
pub fn in_roots(k: &str) -> bool {
    if MACHINE_ROOTS.iter().any(|r| under(k, r)) {
        return true;
    }
    if let Some(rest) = k.strip_prefix("HKCU\\") {
        return USER_ROOTS.iter().any(|r| under(rest, r));
    }
    if let Some(rest) = k.strip_prefix("HKU\\") {
        if let Some((sid, tail)) = rest.split_once('\\') {
            return user_sid(sid) && USER_ROOTS.iter().any(|r| under(tail, r));
        }
    }
    false
}

/// Keys never touched (normalized, component prefix). The user-hive forms
/// are matched after the `HKU\<sid>` → `HKCU` rewrite.
const FORBIDDEN_KEYS: &[(&str, &str)] = &[
    (
        "HKLM\\SOFTWARE\\POLICIES\\MICROSOFT\\WINDOWS DEFENDER",
        "Defender",
    ),
    (
        "HKLM\\SOFTWARE\\POLICIES\\MICROSOFT\\WINDOWS DEFENDER SECURITY CENTER",
        "Defender",
    ),
    (
        "HKLM\\SOFTWARE\\POLICIES\\MICROSOFT\\WINDOWS ADVANCED THREAT PROTECTION",
        "Defender",
    ),
    ("HKLM\\SOFTWARE\\MICROSOFT\\WINDOWS DEFENDER", "Defender"),
    (
        "HKLM\\SOFTWARE\\MICROSOFT\\WINDOWS DEFENDER SECURITY CENTER",
        "Defender",
    ),
    (
        "HKLM\\SOFTWARE\\MICROSOFT\\WINDOWS SECURITY HEALTH",
        "Defender",
    ),
    ("HKLM\\SOFTWARE\\POLICIES\\MICROSOFT\\FVE", "BitLocker"),
    (
        "HKLM\\SOFTWARE\\MICROSOFT\\WINDOWS NT\\CURRENTVERSION\\IMAGE FILE EXECUTION OPTIONS",
        "debugger hijack point",
    ),
    (
        "HKLM\\SOFTWARE\\MICROSOFT\\WINDOWS NT\\CURRENTVERSION\\SILENTPROCESSEXIT",
        "debugger hijack point",
    ),
    (
        "HKLM\\SOFTWARE\\MICROSOFT\\WINDOWS NT\\CURRENTVERSION\\MULTIMEDIA\\SYSTEMPROFILE",
        "MMCSS placebo",
    ),
    (
        "HKLM\\SOFTWARE\\MICROSOFT\\WINDOWS NT\\CURRENTVERSION\\SVCHOST",
        "svchost",
    ),
    (
        "HKLM\\SOFTWARE\\POLICIES\\MICROSOFT\\WINDOWS\\SYSTEM\\SMARTSCREEN",
        "SmartScreen",
    ),
    (
        "HKLM\\SYSTEM\\CURRENTCONTROLSET\\CONTROL\\CI",
        "Smart App Control / code integrity",
    ),
    ("HKLM\\SYSTEM\\CURRENTCONTROLSET\\CONTROL\\LSA", "LSA"),
    (
        "HKLM\\SYSTEM\\CURRENTCONTROLSET\\CONTROL\\SAFEBOOT",
        "Safe Boot",
    ),
    (
        "HKLM\\SYSTEM\\CURRENTCONTROLSET\\CONTROL\\BITLOCKER",
        "BitLocker",
    ),
    (
        "HKLM\\SYSTEM\\CURRENTCONTROLSET\\CONTROL\\SECUREBOOT",
        "Secure Boot",
    ),
    (
        "HKLM\\SYSTEM\\CURRENTCONTROLSET\\CONTROL\\SECURITYPROVIDERS",
        "security providers",
    ),
    (
        "HKLM\\SYSTEM\\CURRENTCONTROLSET\\CONTROL\\SESSION MANAGER\\KERNEL",
        "CPU mitigations",
    ),
    (
        "HKLM\\SYSTEM\\CURRENTCONTROLSET\\CONTROL\\SESSION MANAGER\\MEMORY MANAGEMENT",
        "CPU mitigations",
    ),
    (
        "HKLM\\SYSTEM\\CURRENTCONTROLSET\\CONTROL\\SESSION MANAGER\\ENVIRONMENT",
        "system PATH",
    ),
    (
        "HKLM\\SYSTEM\\CURRENTCONTROLSET\\CONTROL\\PRIORITYCONTROL",
        "placebo",
    ),
    (
        "HKLM\\SYSTEM\\CURRENTCONTROLSET\\CONTROL\\FEATUREMANAGEMENT",
        "feature overrides",
    ),
    (
        "HKLM\\SYSTEM\\CURRENTCONTROLSET\\SERVICES\\TCPIP\\PARAMETERS",
        "netsh/TCP placebo",
    ),
    (
        "HKLM\\SYSTEM\\CURRENTCONTROLSET\\SERVICES\\LANMANSERVER\\PARAMETERS",
        "network placebo",
    ),
];

/// DeviceGuard is off limits (VBS) except the memory integrity switch.
const DEVICEGUARD: &str = "HKLM\\SYSTEM\\CURRENTCONTROLSET\\CONTROL\\DEVICEGUARD";
const HVCI: &str = "HKLM\\SYSTEM\\CURRENTCONTROLSET\\CONTROL\\DEVICEGUARD\\SCENARIOS\\HYPERVISORENFORCEDCODEINTEGRITY";

/// Services whose key (and start type) is never touched.
const FORBIDDEN_SERVICES: &[&str] = &[
    "windefend",
    "wdnissvc",
    "wdfilter",
    "wdboot",
    "wdnisdrv",
    "sense",
    "securityhealthservice",
    "wscsvc",
    "mpssvc",
    "mpsdrv",
    "bfe",
    "ucpd",
    "trustedinstaller",
    "sgrmbroker",
    "sgrmagent",
    "wuauserv",
    "usosvc",
    "waasmedicsvc",
    "bits",
    "cryptsvc",
    "rpcss",
    "eventlog",
    "appidsvc",
    "applockerfltr",
    "hvsics",
    "vmcompute",
];

/// Value names never written, wherever they live.
const FORBIDDEN_VALUES: &[(&str, &str)] = &[
    ("FEATURESETTINGSOVERRIDE", "CPU mitigations"),
    ("FEATURESETTINGSOVERRIDEMASK", "CPU mitigations"),
    ("DISABLEEXCEPTIONCHAINVALIDATION", "SEHOP"),
    ("MITIGATIONOPTIONS", "CPU mitigations"),
    ("MITIGATIONAUDITOPTIONS", "CPU mitigations"),
    ("ENABLEVIRTUALIZATIONBASEDSECURITY", "VBS"),
    ("REQUIREPLATFORMSECURITYFEATURES", "VBS"),
    ("LSACFGFLAGS", "Credential Guard"),
    ("VERIFIEDANDREPUTABLEPOLICYSTATE", "Smart App Control"),
    ("SYSTEMRESTOREPOINTCREATIONFREQUENCY", "restore point limit"),
    ("DISABLEANTISPYWARE", "Defender"),
    ("DISABLEANTIVIRUS", "Defender"),
    ("DISABLEREALTIMEMONITORING", "Defender"),
    ("SUBMITSAMPLESCONSENT", "Defender"),
    ("SPYNETREPORTING", "Defender"),
    ("PREVENTDEVICEENCRYPTION", "BitLocker"),
    ("ENABLELUA", "UAC off"),
    ("SVCHOSTSPLITTHRESHOLDINKB", "svchost placebo"),
    ("GLOBALTIMERRESOLUTIONREQUESTS", "timer placebo"),
    ("NETWORKTHROTTLINGINDEX", "MMCSS placebo"),
    ("SYSTEMRESPONSIVENESS", "MMCSS placebo"),
    ("WIN32PRIORITYSEPARATION", "placebo"),
    ("ENABLEPREFETCHER", "Prefetch placebo"),
    ("ENABLESUPERFETCH", "Prefetch placebo"),
    ("NOAUTOUPDATE", "Windows Update off"),
    ("DISABLEWINDOWSUPDATEACCESS", "Windows Update off"),
];

/// Auto-start points: deleting a value there is fine (removing leftovers),
/// writing one is not.
const NO_WRITE_KEYS: &[&str] = &[
    "HKLM\\SOFTWARE\\MICROSOFT\\WINDOWS\\CURRENTVERSION\\RUN",
    "HKLM\\SOFTWARE\\MICROSOFT\\WINDOWS\\CURRENTVERSION\\RUNONCE",
    "HKLM\\SOFTWARE\\WOW6432NODE\\MICROSOFT\\WINDOWS\\CURRENTVERSION\\RUN",
    "HKLM\\SOFTWARE\\WOW6432NODE\\MICROSOFT\\WINDOWS\\CURRENTVERSION\\RUNONCE",
    "HKLM\\SOFTWARE\\MICROSOFT\\WINDOWS NT\\CURRENTVERSION\\WINLOGON",
    "HKCU\\SOFTWARE\\MICROSOFT\\WINDOWS\\CURRENTVERSION\\RUN",
    "HKCU\\SOFTWARE\\MICROSOFT\\WINDOWS\\CURRENTVERSION\\RUNONCE",
];

/// `HKU\<user sid>\x` → `HKCU\x`, so one list covers both forms.
fn as_user(k: &str) -> String {
    if let Some(rest) = k.strip_prefix("HKU\\") {
        if let Some((sid, tail)) = rest.split_once('\\') {
            if user_sid(sid) {
                return format!("HKCU\\{tail}");
            }
        }
    }
    k.to_string()
}

pub fn forbidden_service(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    FORBIDDEN_SERVICES
        .contains(&n.as_str())
        .then_some("a protected Windows service")
}

/// Why this key/value is never changed, if it is not.
pub fn forbidden(full: &str, name: Option<&str>) -> Option<&'static str> {
    let Ok(k) = normalize(full) else {
        return Some("not a plain key");
    };
    let k = as_user(&k);
    if let Some((_, why)) = FORBIDDEN_KEYS.iter().find(|(p, _)| under(&k, p)) {
        return Some(why);
    }
    if under(&k, DEVICEGUARD) {
        let hvci = k == HVCI && name.is_some_and(|n| n.eq_ignore_ascii_case("Enabled"));
        if !hvci {
            return Some("VBS");
        }
    }
    if let Some(rest) = k.strip_prefix("HKLM\\SYSTEM\\CURRENTCONTROLSET\\SERVICES\\") {
        let svc = rest.split('\\').next().unwrap_or_default();
        if forbidden_service(svc).is_some() {
            return Some("a protected Windows service");
        }
    }
    if let Some(n) = name {
        let n = n.to_ascii_uppercase();
        if let Some((_, why)) = FORBIDDEN_VALUES.iter().find(|(v, _)| *v == n) {
            return Some(why);
        }
    }
    None
}

/// Words in a tweak id or label that belong to the left-out list.
pub fn forbidden_words(id: &str, label: &str) -> Option<&'static str> {
    let text = format!("{id} {label}").to_ascii_lowercase();
    const WORDS: &[(&str, &str)] = &[
        ("defender", "Defender"),
        ("bitlocker", "BitLocker"),
        ("hpet", "placebo"),
        ("dynamictick", "placebo"),
        ("timer resolution", "placebo"),
        ("prefetch", "placebo"),
        ("svchost", "placebo"),
        ("mmcss", "placebo"),
        ("optimize fps", "placebo"),
        ("remove edge", "Edge removal"),
        ("edge - remove", "Edge removal"),
        ("trustedinstaller", "TrustedInstaller"),
        ("smart app control", "Smart App Control"),
        ("spectre", "CPU mitigations"),
        ("meltdown", "CPU mitigations"),
    ];
    WORDS
        .iter()
        .find(|(w, _)| text.contains(w))
        .map(|(_, why)| *why)
}

/// May the helper set, or delete one value of, this key?
pub fn value_write_ok(full: &str, name: &str, deleting: bool) -> Result<String, String> {
    let k = normalize(full)?;
    if !in_roots(&k) {
        return Err(format!(
            "PROTECTED: {full} is outside the allowed registry roots"
        ));
    }
    if let Some(why) = forbidden(full, Some(name)) {
        return Err(format!("PROTECTED: {full} ({why}) is never changed"));
    }
    if !deleting && NO_WRITE_KEYS.iter().any(|p| k == *p || as_user(&k) == *p) {
        return Err(format!(
            "PROTECTED: {full} starts programs; washmybelly does not add to it"
        ));
    }
    Ok(k)
}

/// Shared parents a created-key cleanup must never remove.
const SHARED_PARENTS: &[&str] = &[
    "HKLM\\SOFTWARE\\POLICIES\\MICROSOFT\\WINDOWS",
    "HKLM\\SOFTWARE\\POLICIES\\MICROSOFT\\WINDOWS NT",
    "HKLM\\SOFTWARE\\POLICIES\\MICROSOFT\\WINDOWS\\CURRENTVERSION",
    "HKLM\\SOFTWARE\\MICROSOFT\\WINDOWS\\CURRENTVERSION",
    "HKLM\\SOFTWARE\\MICROSOFT\\WINDOWS NT\\CURRENTVERSION",
    "HKLM\\SOFTWARE\\MICROSOFT\\WINDOWS\\CURRENTVERSION\\EXPLORER",
    "HKLM\\SOFTWARE\\MICROSOFT\\WINDOWS\\CURRENTVERSION\\POLICIES",
    "HKLM\\SOFTWARE\\WOW6432NODE\\MICROSOFT\\WINDOWS\\CURRENTVERSION",
    "HKLM\\SOFTWARE\\CLASSES\\CLSID",
    "HKCU\\SOFTWARE\\POLICIES\\MICROSOFT\\WINDOWS",
    "HKCU\\SOFTWARE\\MICROSOFT\\WINDOWS\\CURRENTVERSION",
    "HKCU\\SOFTWARE\\MICROSOFT\\WINDOWS\\CURRENTVERSION\\EXPLORER",
    "HKCU\\SOFTWARE\\MICROSOFT\\WINDOWS\\CURRENTVERSION\\POLICIES",
    "HKCU\\SOFTWARE\\MICROSOFT\\WINDOWS NT\\CURRENTVERSION",
    "HKCU\\SOFTWARE\\CLASSES\\CLSID",
];

/// A key a tweak may have created: at or above a catalog key, deep enough,
/// never a shared parent, and nowhere under `SYSTEM` (drivers and services
/// keys are never removed whole).
pub fn created_key_ok(k: &str, catalog_keys: &std::collections::HashSet<String>) -> bool {
    let u = as_user(k);
    let depth = u.split('\\').count();
    if depth < 5 || u.starts_with("HKLM\\SYSTEM\\") || SHARED_PARENTS.contains(&u.as_str()) {
        return false;
    }
    catalog_keys.iter().any(|c| under(c, &u))
}

/// May the helper delete this whole key?
pub fn key_delete_ok(
    full: &str,
    catalog_keys: &std::collections::HashSet<String>,
    whole_deletes: &std::collections::HashSet<String>,
) -> Result<String, String> {
    let k = normalize(full)?;
    if !in_roots(&k) {
        return Err(format!(
            "PROTECTED: {full} is outside the allowed registry roots"
        ));
    }
    if let Some(why) = forbidden(full, None) {
        return Err(format!("PROTECTED: {full} ({why}) is never changed"));
    }
    let u = as_user(&k);
    if whole_deletes.contains(&u) || created_key_ok(&k, catalog_keys) {
        return Ok(k);
    }
    // The uninstaller's leftovers: its own check, which also refuses
    // shared vendor keys.
    super::super::apps::windows::run::reg_key_ok(full).map(|_| k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn roots_accept_the_allowed_places_only() {
        for ok in [
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\GameDVR",
            r"HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\GraphicsDrivers",
            r"HKLM\SYSTEM\CurrentControlSet\Services\Tcpip6\Parameters",
            r"HKCU\Software\Microsoft\GameBar",
            r"HKU\S-1-5-21-1111111111-2222222222-3333333333-1001\Control Panel\Mouse",
            r"HKCR\*\shellex\ContextMenuHandlers\Sharing",
        ] {
            assert!(value_write_ok(ok, "X", false).is_ok(), "{ok}");
        }
        for bad in [
            r"HKLM\SAM\SAM",
            r"HKLM\SECURITY\Policy",
            r"HKLM\SYSTEM\Setup",
            r"HKLM\SYSTEM\ControlSet001\Control\FeatureManagement",
            r"HKU\.DEFAULT\Control Panel\Keyboard",
            r"HKU\S-1-5-18\Software\X",
            r"HKCU\Environment",
            r"HKLM\SOFTWARE\..\SAM",
            "HKLM\\SOFTWARE\\\"quoted\"",
            r"HKLM\SOFTWARE\x\",
        ] {
            assert!(value_write_ok(bad, "X", false).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_left_out_list_is_refused_even_inside_the_roots() {
        let deny = [
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows Defender",
                "DisableAntiSpyware",
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows Defender\Real-Time Protection",
                "DisableRealtimeMonitoring",
            ),
            (r"HKLM\SYSTEM\CurrentControlSet\Services\WinDefend", "Start"),
            (r"HKLM\SYSTEM\CurrentControlSet\Services\UCPD", "Start"),
            (
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management",
                "FeatureSettingsOverride",
            ),
            (
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\kernel",
                "DisableExceptionChainValidation",
            ),
            (
                r"HKLM\SYSTEM\CurrentControlSet\Control\DeviceGuard",
                "EnableVirtualizationBasedSecurity",
            ),
            (
                r"HKLM\SYSTEM\CurrentControlSet\Control\DeviceGuard\Scenarios\CredentialGuard",
                "Enabled",
            ),
            (
                r"HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy",
                "VerifiedAndReputablePolicyState",
            ),
            (
                r"HKLM\SYSTEM\CurrentControlSet\Control\BitLocker",
                "PreventDeviceEncryption",
            ),
            (
                r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\SystemRestore",
                "SystemRestorePointCreationFrequency",
            ),
            (
                r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System",
                "EnableLUA",
            ),
            (
                r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile\Tasks\Games",
                "GPU Priority",
            ),
            (
                r"HKLM\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters",
                "TcpAckFrequency",
            ),
            (
                r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Run",
                "Evil",
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU",
                "NoAutoUpdate",
            ),
        ];
        for (k, n) in deny {
            assert!(value_write_ok(k, n, false).is_err(), "{k} {n}");
        }
        // Memory integrity is the one DeviceGuard switch allowed.
        assert!(value_write_ok(r"HKLM\SYSTEM\CurrentControlSet\Control\DeviceGuard\Scenarios\HypervisorEnforcedCodeIntegrity", "Enabled", false).is_ok());
        // Removing a leftover from Run is fine; adding one is not.
        assert!(value_write_ok(
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Run",
            "OldApp",
            true
        )
        .is_ok());
    }

    #[test]
    fn whole_keys_need_a_reason() {
        let cat: HashSet<String> = [
            r"HKLM\SOFTWARE\POLICIES\MICROSOFT\WINDOWS\WINDOWSCOPILOT",
            r"HKCU\SOFTWARE\CLASSES\CLSID\{86CA1AA0-34AA-4E8B-A509-50C905BAE2A2}\INPROCSERVER32",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let whole: HashSet<String> =
            [r"HKLM\SOFTWARE\CLASSES\*\SHELLEX\CONTEXTMENUHANDLERS\SHARING".to_string()]
                .into_iter()
                .collect();
        // A catalog [-KEY], written through HKCR.
        assert!(key_delete_ok(r"HKCR\*\shellex\ContextMenuHandlers\Sharing", &cat, &whole).is_ok());
        // A key a tweak created, and its created parent.
        assert!(key_delete_ok(
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsCopilot",
            &cat,
            &whole
        )
        .is_ok());
        assert!(key_delete_ok(r"HKU\S-1-5-21-1-2-3-1001\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}", &cat, &whole).is_ok());
        // Shared parents never go.
        assert!(key_delete_ok(r"HKLM\SOFTWARE\Policies\Microsoft\Windows", &cat, &whole).is_err());
        assert!(key_delete_ok(r"HKCU\Software\Classes\CLSID", &cat, &whole).is_err());
        // The uninstaller's app keys (apps::windows::run::reg_key_ok).
        assert!(key_delete_ok(r"HKLM\SOFTWARE\Foo", &cat, &whole).is_ok());
        assert!(key_delete_ok(
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\FooApp",
            &cat,
            &whole
        )
        .is_ok());
        assert!(key_delete_ok(r"HKLM\SOFTWARE\Microsoft", &cat, &whole).is_err());
        assert!(key_delete_ok(
            r"HKLM\SYSTEM\CurrentControlSet\Services\Tcpip6",
            &cat,
            &whole
        )
        .is_err());
        assert!(key_delete_ok(
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows Defender",
            &cat,
            &whole
        )
        .is_err());
    }

    #[test]
    fn sids_are_checked() {
        assert!(user_sid("S-1-5-21-1111111111-2222222222-3333333333-1001"));
        assert!(user_sid("S-1-12-1-1-2-3-4"));
        assert!(!user_sid("S-1-5-18"));
        assert!(!user_sid("S-1-5-21-1-2-3-x"));
    }
}
