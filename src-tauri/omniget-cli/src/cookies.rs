use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;

use omniget_core::core::paths::app_data_dir;
use omniget_core::core::cookie_parser::parse_cookie_input;

const COOKIES_DIR: &str = "cookies";
const DEFAULT_COOKIE_FILE: &str = "cookies.txt";

/// Path to the default cookie file shared with the GUI
pub fn default_cookie_file() -> Option<PathBuf> {
    let dir = app_data_dir()?.join(COOKIES_DIR);
    Some(dir.join(DEFAULT_COOKIE_FILE))
}

/// Parse a Netscape-format cookies.txt and return (domain, cookie_string) tuples
pub fn parse_netscape_cookies(content: &str) -> Vec<(String, String)> {
    let mut results: Vec<(String, String)> = Vec::new();
    let mut current_domain: Option<String> = None;
    let mut cookies_for_domain: Vec<(String, String)> = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Netscape format: domain  flag  path  secure  expiration  name  value
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() >= 7 {
            let domain = fields[0].to_string();
            let name = fields[5].trim();
            let value = fields[6].trim();

            if !name.is_empty() && !value.is_empty() {
                if current_domain.as_deref() != Some(&domain) {
                    // Flush previous domain
                    if let Some(d) = &current_domain {
                        if !cookies_for_domain.is_empty() {
                            let cookie_str = cookies_for_domain
                                .iter()
                                .map(|(n, v)| format!("{}={}", n, v))
                                .collect::<Vec<_>>()
                                .join("; ");
                            results.push((d.clone(), cookie_str));
                        }
                    }
                    current_domain = Some(domain.clone());
                    cookies_for_domain.clear();
                }
                cookies_for_domain.push((name.to_string(), value.to_string()));
            }
        }
    }

    // Flush last domain
    if let Some(d) = current_domain {
        if !cookies_for_domain.is_empty() {
            let cookie_str = cookies_for_domain
                .iter()
                .map(|(n, v)| format!("{}={}", n, v))
                .collect::<Vec<_>>()
                .join("; ");
            results.push((d, cookie_str));
        }
    }

    results
}

/// Import a cookies.txt file into the app's cookie storage
pub fn import_cookies_file(src_path: &PathBuf, name: Option<&str>) -> Result<usize> {
    let content = std::fs::read_to_string(src_path)
        .with_context(|| format!("Failed to read {}", src_path.display()))?;

    let cookies_dir = app_data_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine app data directory"))?
        .join(COOKIES_DIR);

    std::fs::create_dir_all(&cookies_dir)?;

    let dest_file = if let Some(n) = name {
        cookies_dir.join(format!("cookies_{}.txt", n))
    } else {
        cookies_dir.join(DEFAULT_COOKIE_FILE)
    };

    // Parse to validate and count
    let parsed = parse_netscape_cookies(&content);
    if parsed.is_empty() {
        anyhow::bail!("No valid cookies found in file (expected Netscape format)");
    }

    // Copy the raw file (preserving original format for yt-dlp consumption)
    // Normalize line endings but otherwise keep as-is
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    std::fs::write(&dest_file, normalized)
        .with_context(|| format!("Failed to write {}", dest_file.display()))?;

    Ok(parsed.iter().map(|(_, s)| s.split(';').count()).sum())
}

/// List showing what the cookie file would look like when consumed
pub fn preview_import(src_path: &PathBuf) -> Result<Vec<(String, usize)>> {
    let content = std::fs::read_to_string(src_path)
        .with_context(|| format!("Failed to read {}", src_path.display()))?;

    let parsed = parse_netscape_cookies(&content);
    Ok(parsed
        .into_iter()
        .map(|(domain, cookies)| {
            let count = cookies.split(';').count();
            (domain, count)
        })
        .collect())
}

pub fn import_cookies_raw(input: &str, target_cookie: &str) -> Result<ParsedCookies> {
    let parsed = parse_cookie_input(input, target_cookie);
    Ok(ParsedCookies {
        token: parsed.token,
        cookie_string: parsed.cookie_string,
    })
}

pub struct ParsedCookies {
    pub token: String,
    pub cookie_string: String,
}
