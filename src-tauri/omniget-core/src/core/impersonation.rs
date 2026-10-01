use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImpersonateTarget {
    pub client: String,
    pub os: Option<String>,
}

impl ImpersonateTarget {
    pub fn as_flag_value(&self) -> String {
        match &self.os {
            Some(os) => format!("{}:{}", self.client, os),
            None => self.client.clone(),
        }
    }
}

pub fn parse_targets(stdout: &str) -> Vec<ImpersonateTarget> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| !l.starts_with('['))
        .filter(|l| !l.starts_with('-'))
        .filter(|l| !l.starts_with("Client"))
        .filter(|l| !l.contains("(unavailable)"))
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let client = cols.next()?.to_string();
            let os = cols.next().map(|s| s.to_string()).filter(|s| s != "-");
            Some(ImpersonateTarget { client, os })
        })
        .collect()
}

pub fn pick_ytdlp(
    onedir: Option<std::path::PathBuf>,
    zipapp: Option<std::path::PathBuf>,
    onefile: Option<std::path::PathBuf>,
    zipapp_impersonates: bool,
) -> Option<std::path::PathBuf> {
    if onedir.is_some() {
        return onedir;
    }
    match (zipapp, onefile) {
        (Some(zip), Some(_)) if zipapp_impersonates => Some(zip),
        (_, Some(onefile)) => Some(onefile),
        (zip, None) => zip,
    }
}

pub fn strip_default_user_agent(args: &mut Vec<String>, default_ua: &str) {
    if !args.iter().any(|a| a == "--impersonate") {
        return;
    }
    let mut i = 0;
    while i + 1 < args.len() {
        if args[i] == "--user-agent" && args[i + 1] == default_ua {
            args.drain(i..i + 2);
        } else {
            i += 1;
        }
    }
}

pub fn stderr_indicates_fingerprint_block(stderr_lower: &str) -> bool {
    const SIGNS: &[&str] = &[
        "unable to handshake",
        "ssl: sslv3_alert_handshake_failure",
        "the read operation timed out",
        "cloudflare",
        "just a moment...",
        "enable javascript and cookies to continue",
        "http error 403: forbidden",
    ];
    SIGNS.iter().any(|s| stderr_lower.contains(s))
}

pub fn preferred_target(targets: &[ImpersonateTarget]) -> Option<&ImpersonateTarget> {
    let desktop_chrome = targets.iter().find(|t| {
        t.client.starts_with("Chrome")
            && t.os
                .as_deref()
                .is_some_and(|os| !os.starts_with("Android") && !os.starts_with("Ios"))
    });
    desktop_chrome
        .or_else(|| targets.iter().find(|t| t.client.starts_with("Chrome")))
        .or_else(|| targets.first())
}
