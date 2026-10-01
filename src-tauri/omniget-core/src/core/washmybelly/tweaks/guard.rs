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

fn under(key: &str, prefix: &str) -> bool {
    key == prefix
        || key
            .strip_prefix(prefix)
            .is_some_and(|r| r.starts_with('\\'))
}

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

const DEVICEGUARD: &str = "HKLM\\SYSTEM\\CURRENTCONTROLSET\\CONTROL\\DEVICEGUARD";
const HVCI: &str = "HKLM\\SYSTEM\\CURRENTCONTROLSET\\CONTROL\\DEVICEGUARD\\SCENARIOS\\HYPERVISORENFORCEDCODEINTEGRITY";

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

const NO_WRITE_KEYS: &[&str] = &[
    "HKLM\\SOFTWARE\\MICROSOFT\\WINDOWS\\CURRENTVERSION\\RUN",
    "HKLM\\SOFTWARE\\MICROSOFT\\WINDOWS\\CURRENTVERSION\\RUNONCE",
    "HKLM\\SOFTWARE\\WOW6432NODE\\MICROSOFT\\WINDOWS\\CURRENTVERSION\\RUN",
    "HKLM\\SOFTWARE\\WOW6432NODE\\MICROSOFT\\WINDOWS\\CURRENTVERSION\\RUNONCE",
    "HKLM\\SOFTWARE\\MICROSOFT\\WINDOWS NT\\CURRENTVERSION\\WINLOGON",
    "HKCU\\SOFTWARE\\MICROSOFT\\WINDOWS\\CURRENTVERSION\\RUN",
    "HKCU\\SOFTWARE\\MICROSOFT\\WINDOWS\\CURRENTVERSION\\RUNONCE",
];

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

pub fn created_key_ok(k: &str, catalog_keys: &std::collections::HashSet<String>) -> bool {
    let u = as_user(k);
    let depth = u.split('\\').count();
    if depth < 5 || u.starts_with("HKLM\\SYSTEM\\") || SHARED_PARENTS.contains(&u.as_str()) {
        return false;
    }
    catalog_keys.iter().any(|c| under(c, &u))
}

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
    super::super::apps::windows::run::reg_key_ok(full).map(|_| k)
}
