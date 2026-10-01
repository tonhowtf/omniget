use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct LcuCredentials {
    pub port: u16,
    pub token: String,
    pub region: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum Source {
    CommandLine,
    Lockfile,
}

pub fn extract_arg(cmdline: &str, key: &str) -> Option<String> {
    let needle = format!("--{}=", key);
    let start = cmdline.find(&needle)? + needle.len();
    let rest = &cmdline[start..];
    let value: String = rest
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
        .collect();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn client_lines(listing: &str) -> impl Iterator<Item = &str> {
    listing
        .lines()
        .filter(|line| line.contains("LeagueClientUx"))
}

pub fn region_from_listing(listing: &str) -> Option<String> {
    client_lines(listing).find_map(|line| {
        extract_arg(line, "region").or_else(|| extract_arg(line, "rso_platform_id"))
    })
}

pub fn credentials_from_listing(listing: &str) -> Option<LcuCredentials> {
    client_lines(listing).find_map(|line| {
        let port = extract_arg(line, "app-port")?.parse::<u16>().ok()?;
        let token = extract_arg(line, "remoting-auth-token")?;
        Some(LcuCredentials {
            port,
            token,
            region: extract_arg(line, "region").or_else(|| extract_arg(line, "rso_platform_id")),
        })
    })
}

pub fn parse_lockfile(contents: &str) -> Option<LcuCredentials> {
    let line = contents.lines().find(|l| !l.trim().is_empty())?;
    let fields: Vec<&str> = line.trim().split(':').collect();
    if fields.len() < 5 {
        return None;
    }
    let port = fields[2].parse::<u16>().ok()?;
    let token = fields[3].to_string();
    if token.is_empty() {
        return None;
    }
    Some(LcuCredentials {
        port,
        token,
        region: None,
    })
}

fn executable_path(line: &str) -> Option<&str> {
    let head = line.split(" --").next()?.trim();
    let head = head.trim_matches(|c| c == '"' || c == '\'');
    if head.is_empty() {
        None
    } else {
        Some(head)
    }
}

fn ancestors_of(path: &str) -> Vec<&str> {
    let mut ancestors = Vec::new();
    let mut current = path;
    while let Some(index) = current.rfind(['/', '\\']) {
        if index == 0 {
            break;
        }
        current = &current[..index];
        ancestors.push(current);
    }
    ancestors
}

fn last_segment(path: &str) -> &str {
    match path.rfind(['/', '\\']) {
        Some(index) => &path[index + 1..],
        None => path,
    }
}

pub fn install_dirs_from_listing(listing: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for line in client_lines(listing) {
        if let Some(dir) = extract_arg(line, "install-directory") {
            push_unique(&mut dirs, PathBuf::from(dir));
        }
        if let Some(exe) = executable_path(line) {
            let ancestors = ancestors_of(exe);
            for ancestor in &ancestors {
                if last_segment(ancestor) == "LoL" {
                    push_unique(&mut dirs, PathBuf::from(*ancestor));
                }
            }
            if let Some(parent) = ancestors.first() {
                push_unique(&mut dirs, PathBuf::from(*parent));
            }
        }
    }
    dirs
}

fn push_unique(dirs: &mut Vec<PathBuf>, dir: PathBuf) {
    if !dirs.contains(&dir) {
        dirs.push(dir);
    }
}

pub fn lockfile_paths_from_listing(listing: &str) -> Vec<PathBuf> {
    install_dirs_from_listing(listing)
        .into_iter()
        .map(|dir| dir.join("lockfile"))
        .collect()
}

pub fn default_lockfile_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let mut paths = vec![PathBuf::from(
            "/Applications/League of Legends.app/Contents/LoL/lockfile",
        )];
        if let Some(home) = std::env::var_os("HOME") {
            paths.push(
                PathBuf::from(home)
                    .join("Library/Application Support/Riot Games/League of Legends/lockfile"),
            );
        }
        paths
    }
    #[cfg(windows)]
    {
        let mut paths = vec![PathBuf::from("C:\\Riot Games\\League of Legends\\lockfile")];
        if let Some(drive) = std::env::var_os("SystemDrive") {
            let mut path = PathBuf::from(drive);
            path.push("\\Riot Games\\League of Legends\\lockfile");
            push_unique(&mut paths, path);
        }
        paths
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Vec::new()
    }
}

async fn read_process_command_lines() -> Result<String, String> {
    #[cfg(windows)]
    {
        let probe = crate::core::process::command("tasklist")
            .args(["/FI", "IMAGENAME eq LeagueClientUx.exe", "/NH"])
            .output()
            .await
            .map_err(|e| format!("failed to probe processes: {}", e))?;
        if !String::from_utf8_lossy(&probe.stdout).contains("LeagueClientUx.exe") {
            return Ok(String::new());
        }
        let output = crate::core::process::command("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Get-CimInstance Win32_Process -Filter \"Name='LeagueClientUx.exe'\" | Select-Object -ExpandProperty CommandLine",
            ])
            .output()
            .await
            .map_err(|e| format!("failed to query processes: {}", e))?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
    #[cfg(not(windows))]
    {
        let output = tokio::process::Command::new("ps")
            .args(["-axo", "command"])
            .output()
            .await
            .map_err(|e| format!("failed to query processes: {}", e))?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

async fn credentials_from_lockfiles(listing: &str) -> Option<(LcuCredentials, Source)> {
    let mut candidates = lockfile_paths_from_listing(listing);
    for path in default_lockfile_paths() {
        push_unique(&mut candidates, path);
    }
    for path in candidates {
        let Ok(contents) = tokio::fs::read_to_string(&path).await else {
            continue;
        };
        if let Some(mut credentials) = parse_lockfile(&contents) {
            credentials.region = region_from_listing(listing);
            return Some((credentials, Source::Lockfile));
        }
    }
    None
}

pub async fn discover() -> Option<(LcuCredentials, Source)> {
    let listing = read_process_command_lines().await.unwrap_or_default();
    if let Some(credentials) = credentials_from_listing(&listing) {
        return Some((credentials, Source::CommandLine));
    }
    credentials_from_lockfiles(&listing).await
}
