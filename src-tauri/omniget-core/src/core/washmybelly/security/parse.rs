use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CodeSign {
    pub signed: bool,
    pub adhoc: bool,
    pub identifier: Option<String>,
    pub authorities: Vec<String>,
    pub team_id: Option<String>,
    pub runtime: bool,
    pub timestamp: Option<String>,
}

impl CodeSign {
    pub fn apple(&self) -> bool {
        self.authorities
            .first()
            .is_some_and(|a| a == "Software Signing" || a.starts_with("Apple "))
            && self.team_id.is_none()
    }

    pub fn app_store(&self) -> bool {
        self.authorities
            .first()
            .is_some_and(|a| a == "Apple Mac OS Application Signing")
    }

    pub fn signer(&self) -> Option<String> {
        let first = self.authorities.first()?;
        if self.apple() {
            return Some("Apple".into());
        }
        let name = first
            .strip_prefix("Developer ID Application: ")
            .or_else(|| first.strip_prefix("Apple Development: "))
            .or_else(|| first.strip_prefix("Apple Distribution: "))
            .or_else(|| first.strip_prefix("3rd Party Mac Developer Application: "))
            .unwrap_or(first);
        let name = match (&self.team_id, name.rfind(" (")) {
            (Some(team), Some(i)) if name[i + 2..].trim_end_matches(')') == team => &name[..i],
            _ => name,
        };
        Some(name.to_string())
    }
}

pub fn parse_codesign(text: &str) -> CodeSign {
    let mut s = CodeSign::default();
    if text.contains("code object is not signed at all")
        || text.contains("No such file or directory")
    {
        return s;
    }
    for line in text.lines() {
        let line = line.trim_end();
        if let Some(v) = line.strip_prefix("Authority=") {
            s.authorities.push(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("TeamIdentifier=") {
            let v = v.trim();
            if v != "not set" && !v.is_empty() {
                s.team_id = Some(v.to_string());
            }
        } else if let Some(v) = line.strip_prefix("Identifier=") {
            s.signed = true;
            s.identifier = Some(v.trim().to_string());
        } else if line.starts_with("CodeDirectory ") {
            s.signed = true;
            if let Some(flags) = line
                .split_whitespace()
                .find_map(|w| w.strip_prefix("flags="))
            {
                let inside = flags
                    .split_once('(')
                    .map(|(_, r)| r.trim_end_matches(')'))
                    .unwrap_or("");
                let words: Vec<&str> = inside.split(',').collect();
                s.runtime = words.contains(&"runtime");
                if words.contains(&"adhoc") {
                    s.adhoc = true;
                }
            }
        } else if line == "Signature=adhoc" {
            s.adhoc = true;
        } else if let Some(v) = line.strip_prefix("Timestamp=") {
            s.timestamp = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("Signed Time=") {
            s.timestamp.get_or_insert_with(|| v.trim().to_string());
        }
    }
    if !s.authorities.is_empty() {
        s.signed = true;
    }
    s
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verify {
    Notarized,
    NotNotarized,
    Invalid(String),
}

pub fn parse_verify(code: Option<i32>, stderr: &str) -> Verify {
    let msg = stderr.trim();
    match code {
        Some(0) => Verify::Notarized,
        _ if msg.contains("failed to satisfy specified code requirement") => Verify::NotNotarized,
        _ => {
            let reason = msg
                .lines()
                .find(|l| !l.trim().is_empty())
                .map(|l| match l.rsplit_once(": ") {
                    Some((_, r)) => r.trim().to_string(),
                    None => l.trim().to_string(),
                })
                .unwrap_or_else(|| "the signature could not be checked".into());
            let reason = match msg
                .lines()
                .find_map(|l| l.trim().strip_prefix("In subcomponent: "))
            {
                Some(sub) => format!("{reason} (in {sub})"),
                None => reason,
            };
            Verify::Invalid(reason)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Assess {
    pub accepted: bool,
    pub source: Option<String>,
    pub origin: Option<String>,
    pub reason: Option<String>,
}

impl Assess {
    pub fn notarized(&self) -> Option<bool> {
        match self.source.as_deref()? {
            "Notarized Developer ID" => Some(true),
            "Developer ID" | "Unnotarized Developer ID" => Some(false),
            _ => None,
        }
    }
}

pub fn parse_spctl(text: &str) -> Assess {
    let mut a = Assess::default();
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("source=") {
            a.source = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("origin=") {
            a.origin = Some(v.trim().to_string());
        } else if let Some((_, verdict)) = line.rsplit_once(": ") {
            let verdict = verdict.trim();
            if verdict == "accepted" {
                a.accepted = true;
            } else if let Some(rest) = verdict.strip_prefix("rejected") {
                a.accepted = false;
                let r = rest
                    .trim()
                    .trim_start_matches('(')
                    .trim_end_matches(')')
                    .trim();
                if !r.is_empty() {
                    a.reason = Some(r.to_string());
                }
            }
        }
    }
    a
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SysExt {
    pub category: String,
    pub enabled: bool,
    pub active: bool,
    pub team_id: Option<String>,
    pub bundle_id: String,
    pub version: Option<String>,
    pub name: String,
    pub state: String,
}

pub fn parse_systemextensions(text: &str) -> Vec<SysExt> {
    let mut out = Vec::new();
    let mut category = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("--- ") {
            let id = rest.split_whitespace().next().unwrap_or("");
            category = id.rsplit('.').next().unwrap_or(id).to_string();
            continue;
        }
        if line.starts_with("enabled\t") || !line.contains('\t') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 6 {
            continue;
        }
        let (bundle, version) = match cols[3].split_once(" (") {
            Some((b, v)) => (
                b.trim().to_string(),
                Some(v.trim_end_matches(')').to_string()),
            ),
            None => (cols[3].trim().to_string(), None),
        };
        if bundle.is_empty() {
            continue;
        }
        let team = cols[2].trim();
        out.push(SysExt {
            category: category.clone(),
            enabled: cols[0].trim() == "*",
            active: cols[1].trim() == "*",
            team_id: (!team.is_empty() && team != "-").then(|| team.to_string()),
            bundle_id: bundle,
            version,
            name: cols[4].trim().to_string(),
            state: cols[5]
                .trim()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_string(),
        });
    }
    out
}

pub fn parse_kmutil(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.len() < 7 || words[0].parse::<u32>().is_err() {
            continue;
        }
        for i in 1..words.len() - 1 {
            let v = words[i + 1];
            if v.starts_with('(') && v.ends_with(')') && words[i].contains('.') {
                out.push((
                    words[i].to_string(),
                    v.trim_matches(|c| c == '(' || c == ')').to_string(),
                ));
                break;
            }
        }
    }
    out
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Profile {
    pub scope: String,
    pub name: String,
    pub identifier: String,
    pub organization: Option<String>,
    pub description: Option<String>,
    pub installed: Option<String>,
    pub verification: Option<String>,
    pub removal_disallowed: bool,
    pub payloads: Vec<String>,
}

pub fn parse_profiles(text: &str) -> Vec<Profile> {
    let mut out: Vec<Profile> = Vec::new();
    let mut key: Option<(String, String)> = None;
    for line in text.lines() {
        let Some(open) = line.find('[') else { continue };
        let Some(close) = line[open..].find(']').map(|c| open + c) else {
            continue;
        };
        let scope = line[..open].trim().to_string();
        let index = line[open + 1..close].to_string();
        if scope.is_empty() || scope.contains(' ') || index.parse::<u32>().is_err() {
            continue;
        }
        let rest = line[close + 1..].trim();
        if key.as_ref() != Some(&(scope.clone(), index.clone())) {
            key = Some((scope.clone(), index.clone()));
            out.push(Profile {
                scope: scope.clone(),
                ..Default::default()
            });
        }
        let Some(p) = out.last_mut() else { continue };
        if let Some(attr) = rest.strip_prefix("attribute: ") {
            let (k, v) = attr
                .split_once(':')
                .map(|(k, v)| (k.trim(), v.trim()))
                .unwrap_or((attr.trim(), ""));
            let v = v.to_string();
            match k {
                "name" => p.name = v,
                "profileIdentifier" => p.identifier = v,
                "organization" => p.organization = (!v.is_empty()).then_some(v),
                "configurationDescription" => p.description = (!v.is_empty()).then_some(v),
                "installationDate" => p.installed = (!v.is_empty()).then_some(v),
                "profileVerificationState" => p.verification = (!v.is_empty()).then_some(v),
                "removalDisallowed" => p.removal_disallowed = v.eq_ignore_ascii_case("true"),
                _ => {}
            }
        } else if rest.starts_with("payload[") {
            if let Some((_, t)) = rest.split_once('=') {
                p.payloads.push(t.trim().to_string());
            }
        }
    }
    out.retain(|p| !p.identifier.is_empty() || !p.name.is_empty());
    out
}

pub fn parse_profile_date(s: &str) -> Option<i64> {
    let mut parts = s.split_whitespace();
    let date = parts.next()?;
    let time = parts.next().unwrap_or("00:00:00");
    let offset = parts.next().unwrap_or("+0000");
    let d: Vec<i64> = date.split('-').filter_map(|x| x.parse().ok()).collect();
    let t: Vec<i64> = time.split(':').filter_map(|x| x.parse().ok()).collect();
    if d.len() != 3 || t.len() < 2 {
        return None;
    }
    let days = days_from_civil(d[0], d[1], d[2]);
    let mut secs = days * 86_400 + t[0] * 3600 + t[1] * 60 + t.get(2).copied().unwrap_or(0);
    if offset.len() == 5 {
        let sign = if offset.starts_with('-') { -1 } else { 1 };
        let h: i64 = offset[1..3].parse().ok()?;
        let m: i64 = offset[3..5].parse().ok()?;
        secs -= sign * (h * 3600 + m * 60);
    }
    Some(secs)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Quarantine {
    pub flags: String,
    pub at: Option<i64>,
    pub agent: Option<String>,
    pub approved: bool,
}

pub fn parse_quarantine(value: &str) -> Option<Quarantine> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let parts: Vec<&str> = value.split(';').collect();
    let flags = parts.first()?.to_string();
    let bits = u32::from_str_radix(&flags, 16).unwrap_or(0);
    Some(Quarantine {
        at: parts
            .get(1)
            .and_then(|h| i64::from_str_radix(h, 16).ok())
            .filter(|t| *t > 0),
        agent: parts
            .get(2)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        approved: bits & 0x40 != 0,
        flags,
    })
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CronLine {
    pub schedule: String,
    pub command: String,
}

pub fn parse_crontab(text: &str) -> Vec<CronLine> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let first = t.split_whitespace().next().unwrap_or("");
        if !first.starts_with('@') && first.contains('=') {
            continue;
        }
        let fields = if first.starts_with('@') { 1 } else { 5 };
        let words: Vec<&str> = t.split_whitespace().collect();
        if words.len() <= fields {
            continue;
        }
        out.push(CronLine {
            schedule: words[..fields].join(" "),
            command: words[fields..].join(" "),
        });
    }
    out
}

pub fn is_macho(head: &[u8]) -> bool {
    matches!(
        head.get(..4),
        Some([0xfe, 0xed, 0xfa, 0xce])
            | Some([0xfe, 0xed, 0xfa, 0xcf])
            | Some([0xce, 0xfa, 0xed, 0xfe])
            | Some([0xcf, 0xfa, 0xed, 0xfe])
            | Some([0xca, 0xfe, 0xba, 0xbe])
            | Some([0xbe, 0xba, 0xfe, 0xca])
    )
}

pub fn enclosing_app(path: &str) -> Option<&str> {
    let i = path
        .find(".app/")
        .map(|i| i + 4)
        .or_else(|| path.ends_with(".app").then_some(path.len()))?;
    Some(&path[..i])
}

const INTERPRETERS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "dash",
    "ksh",
    "tcsh",
    "csh",
    "fish",
    "python",
    "python2",
    "python3",
    "perl",
    "ruby",
    "node",
    "bun",
    "deno",
    "osascript",
    "php",
    "java",
    "env",
];

pub fn is_interpreter(program: &str) -> bool {
    let name = program.rsplit('/').next().unwrap_or(program);
    let base = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    INTERPRETERS.contains(&name) || INTERPRETERS.contains(&base)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Script {
    None,
    File(String),
    Inline,
}

pub fn script_of(arguments: &[String]) -> Script {
    let Some(program) = arguments.first() else {
        return Script::None;
    };
    if !is_interpreter(program) {
        return Script::None;
    }
    for a in &arguments[1..] {
        if a == "-c" || a == "-e" || a == "--eval" {
            return Script::Inline;
        }
        if a.starts_with('/') || a.starts_with("~/") {
            return Script::File(a.clone());
        }
    }
    Script::None
}

pub fn parse_yara(text: &str) -> Vec<String> {
    let mut rules: Vec<String> = text
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|r| !r.is_empty() && !r.starts_with("warning") && !r.starts_with("error"))
        .map(str::to_string)
        .collect();
    rules.sort();
    rules.dedup();
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx(name: &str) -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/washmybelly_fixtures/security")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn codesign_developer_id_with_runtime_and_timestamp() {
        let s = parse_codesign(&fx("codesign_v4_devid.txt"));
        assert!(s.signed && !s.adhoc && s.runtime && !s.apple());
        assert_eq!(s.team_id.as_deref(), Some("HUAQ24HBR6"));
        assert_eq!(s.signer().as_deref(), Some("Orbital Labs, LLC (U.S.)"));
        assert_eq!(
            s.identifier.as_deref(),
            Some("dev.orbstack.OrbStack.privhelper")
        );
        assert_eq!(s.timestamp.as_deref(), Some("4 Jun 2026 at 14:09:33"));
        assert_eq!(s.authorities.len(), 3);
    }

    #[test]
    fn codesign_apple_adhoc_and_unsigned() {
        let a = parse_codesign(&fx("codesign_v4_apple.txt"));
        assert!(a.signed && a.apple() && !a.runtime);
        assert_eq!(a.signer().as_deref(), Some("Apple"));
        let h = parse_codesign(&fx("codesign_v4_adhoc.txt"));
        assert!(h.signed && h.adhoc && h.authorities.is_empty() && h.team_id.is_none());
        let u = parse_codesign(&fx("codesign_unsigned.txt"));
        assert!(!u.signed && u.signer().is_none());
    }

    #[test]
    fn verify_tells_notarized_from_not_and_broken() {
        assert_eq!(
            parse_verify(Some(0), &fx("codesign_verify_ok.txt")),
            Verify::Notarized
        );
        assert_eq!(
            parse_verify(Some(3), &fx("codesign_notarized_req_fail.txt")),
            Verify::NotNotarized
        );
        assert_eq!(
            parse_verify(Some(1), &fx("codesign_verify_subcomponent.txt")),
            Verify::Invalid(
                "code object is not signed at all (in /Applications/Example.app/Contents/Frameworks/Plugin/x64/model.kef)".into()
            )
        );
        assert_eq!(
            parse_verify(Some(1), &fx("codesign_verify_invalid.txt")),
            Verify::Invalid("a sealed resource is missing or invalid".into())
        );
    }

    #[test]
    fn spctl_sources() {
        let n = parse_spctl(&fx("spctl_notarized.txt"));
        assert!(n.accepted);
        assert_eq!(n.notarized(), Some(true));
        assert_eq!(
            n.origin.as_deref(),
            Some("Developer ID Application: Orbital Labs, LLC (U.S.) (HUAQ24HBR6)")
        );
        let a = parse_spctl(&fx("spctl_apple.txt"));
        assert!(a.accepted && a.notarized().is_none());
        assert_eq!(a.source.as_deref(), Some("Apple System"));
        let bare = parse_spctl(&fx("spctl_not_an_app.txt"));
        assert!(!bare.accepted);
        let open = parse_spctl(&fx("spctl_open_unsigned.txt"));
        assert!(!open.accepted);
        assert_eq!(open.source.as_deref(), Some("no usable signature"));
        let un = parse_spctl("/A/X.app: rejected\nsource=Unnotarized Developer ID\norigin=Developer ID Application: X (ABC)\n");
        assert_eq!(un.notarized(), Some(false));
    }

    #[test]
    fn system_extensions_by_category() {
        let v = parse_systemextensions(&fx("systemextensionsctl_list.txt"));
        assert_eq!(v.len(), 4);
        assert_eq!(v[0].category, "network_extension");
        assert!(v[0].enabled && v[0].active);
        assert_eq!(v[0].bundle_id, "net.mullvad.MullvadVPN.PacketTunnel");
        assert_eq!(v[0].version.as_deref(), Some("2025.14/2025.14"));
        assert_eq!(v[0].state, "activated enabled");
        assert!(!v[1].enabled && !v[1].active);
        assert_eq!(v[2].category, "endpoint_security");
        assert_eq!(v[3].category, "driver_extension");
        assert!(!v[3].enabled && v[3].active);
        assert_eq!(v[3].team_id.as_deref(), Some("QED4VVPZWA"));
        let live = parse_systemextensions(&fx("systemextensionsctl_list_live.txt"));
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].name, "Logitech G HUB HID Driver Extension");
        assert!(parse_systemextensions("0 extension(s)\n").is_empty());
    }

    #[test]
    fn kmutil_lists_bundles() {
        let v = parse_kmutil(&fx("kmutil_showloaded.txt"));
        assert!(v.iter().any(|(b, _)| b == "com.apple.kpi.bsd"));
        let third: Vec<_> = v
            .iter()
            .filter(|(b, _)| !b.starts_with("com.apple."))
            .collect();
        assert_eq!(third.len(), 2);
        assert_eq!(
            third[0],
            &(
                "com.highpoint-tech.kext.HighPointRR".to_string(),
                "5.2.4".to_string()
            )
        );
    }

    #[test]
    fn profiles_grouped_by_scope_and_index() {
        let v = parse_profiles(&fx("profiles_show.txt"));
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].scope, "_computerlevel");
        assert_eq!(v[0].name, "Acme Device Management");
        assert_eq!(v[0].organization.as_deref(), Some("Acme Corp"));
        assert!(v[0].removal_disallowed);
        assert_eq!(
            v[0].payloads,
            vec!["com.apple.wifi.managed", "com.apple.vpn.managed"]
        );
        assert_eq!(v[0].verification.as_deref(), Some("verified"));
        assert_eq!(v[1].scope, "user");
        assert_eq!(v[1].verification.as_deref(), Some("unsigned"));
        assert!(parse_profiles(&fx("profiles_none.txt")).is_empty());
        assert_eq!(
            parse_profile_date("2026-03-02 14:23:11 +0000"),
            Some(1_772_461_391)
        );
        assert_eq!(parse_profile_date("1970-01-01 01:00:00 +0100"), Some(0));
    }

    #[test]
    fn quarantine_value() {
        let q =
            parse_quarantine("0083;6523a1b2;Safari;6C2F1E0A-0000-4000-8000-000000000000").unwrap();
        assert_eq!(q.agent.as_deref(), Some("Safari"));
        assert_eq!(q.at, Some(0x6523a1b2));
        assert!(!q.approved);
        assert!(parse_quarantine("01c1;6523a1b2;Chrome;").unwrap().approved);
        assert!(parse_quarantine("").is_none());
    }

    #[test]
    fn crontab_lines() {
        let v = parse_crontab(&fx("crontab.txt"));
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].schedule, "*/15 * * * *");
        assert_eq!(v[0].command, "/Users/someone/bin/backup.sh --quiet");
        assert_eq!(v[1].schedule, "@reboot");
    }

    #[test]
    fn helpers() {
        assert!(is_macho(&[0xcf, 0xfa, 0xed, 0xfe, 7]));
        assert!(is_macho(&[0xca, 0xfe, 0xba, 0xbe]));
        assert!(!is_macho(b"#!/bin/sh"));
        assert_eq!(
            enclosing_app("/Applications/lghub.app/Contents/MacOS/lghub"),
            Some("/Applications/lghub.app")
        );
        assert_eq!(
            enclosing_app("/Applications/Foo.app"),
            Some("/Applications/Foo.app")
        );
        assert_eq!(enclosing_app("/usr/bin/x"), None);
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            script_of(&args(&["/usr/local/bin/node", "/Users/a/x.mjs", "--dev"])),
            Script::File("/Users/a/x.mjs".into())
        );
        assert_eq!(
            script_of(&args(&["/bin/sh", "-c", "curl x | sh"])),
            Script::Inline
        );
        assert_eq!(
            script_of(&args(&["/usr/bin/python3.12", "/tmp/a.py"])),
            Script::File("/tmp/a.py".into())
        );
        assert_eq!(
            script_of(&args(&["/Applications/X.app/Contents/MacOS/x", "/y"])),
            Script::None
        );
        assert_eq!(
            parse_yara("MACOS_ADLOAD_A /tmp/x\nMACOS_ADLOAD_A /tmp/x\nOTHER /tmp/x\n"),
            vec!["MACOS_ADLOAD_A", "OTHER"]
        );
    }
}
