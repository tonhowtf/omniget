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
    // The needle keeps both leading dashes on purpose: a loose "app-port="
    // would also match the unrelated "--riotclient-app-port=".
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
    // Format: name:pid:port:token:protocol
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
    // Cutting at the first " --" survives install paths that contain spaces,
    // which whitespace splitting would mangle on both platforms.
    let head = line.split(" --").next()?.trim();
    let head = head.trim_matches(|c| c == '"' || c == '\'');
    if head.is_empty() {
        None
    } else {
        Some(head)
    }
}

/// Walks up a path textually, accepting either separator: a Windows listing
/// still has to parse correctly when the tests run on a unix host.
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
            // On macOS the binary sits several bundles deep
            // (…/Contents/LoL/LeagueClientUx.app/Contents/MacOS/LeagueClientUx),
            // so the install root is the ancestor named "LoL".
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
    // The client has no native Linux build, so there is no standard path to
    // guess; discovery there relies on the command line alone.
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Vec::new()
    }
}

async fn read_process_command_lines() -> Result<String, String> {
    #[cfg(windows)]
    {
        // tasklist costs ~50ms; the CIM query costs seconds and stutters the
        // whole app, so it only runs once the cheap probe sees the process.
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

/// Install directory of the game, when it can be told from the running process.
/// Features that need files on disk (replays, the settings lock) depend on this.
pub async fn install_dir() -> Option<PathBuf> {
    let listing = read_process_command_lines().await.ok()?;
    install_dirs_from_listing(&listing).into_iter().next()
}

pub async fn discover() -> Option<(LcuCredentials, Source)> {
    let listing = read_process_command_lines().await.unwrap_or_default();
    if let Some(credentials) = credentials_from_listing(&listing) {
        return Some((credentials, Source::CommandLine));
    }
    // Reading another process' command line can fail (missing privileges on
    // Windows, sandboxed listings), while the lockfile stays readable.
    credentials_from_lockfiles(&listing).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC_LISTING: &str = "/Applications/Safari.app/Contents/MacOS/Safari\n\
/Applications/League of Legends.app/Contents/LoL/LeagueClientUx.app/Contents/MacOS/LeagueClientUx --riotclient-app-port=54321 --app-port=61947 --remoting-auth-token=aBcD-1234_xyz --region=BR --locale=pt_BR\n";

    const WIN_LISTING: &str = "\"C:\\Riot Games\\League of Legends\\LeagueClientUx.exe\" --riotclient-app-port=1111 --app-port=2222 --remoting-auth-token=tok3n --rso_platform_id=NA1\n";

    #[test]
    fn app_port_is_not_confused_with_riotclient_app_port() {
        let creds = credentials_from_listing(MAC_LISTING).expect("credentials");
        assert_eq!(creds.port, 61947);
        assert_eq!(creds.token, "aBcD-1234_xyz");
        assert_eq!(creds.region.as_deref(), Some("BR"));
    }

    #[test]
    fn windows_listing_falls_back_to_rso_platform_id_for_region() {
        let creds = credentials_from_listing(WIN_LISTING).expect("credentials");
        assert_eq!(creds.port, 2222);
        assert_eq!(creds.region.as_deref(), Some("NA1"));
    }

    #[test]
    fn listing_without_the_client_yields_nothing() {
        assert!(credentials_from_listing("/usr/bin/ssh -L 61947:localhost:80").is_none());
    }

    #[test]
    fn lockfile_is_parsed_into_port_and_token() {
        let creds = parse_lockfile("LeagueClient:4242:61947:aBcD-1234_xyz:https\n").expect("creds");
        assert_eq!(creds.port, 61947);
        assert_eq!(creds.token, "aBcD-1234_xyz");
        assert_eq!(creds.region, None);
    }

    #[test]
    fn truncated_or_empty_lockfiles_are_rejected() {
        assert!(parse_lockfile("LeagueClient:4242:61947").is_none());
        assert!(parse_lockfile("").is_none());
        assert!(parse_lockfile("LeagueClient:4242:61947::https").is_none());
        assert!(parse_lockfile("LeagueClient:4242:notaport:tok:https").is_none());
    }

    #[test]
    fn install_dir_is_recovered_from_a_path_containing_spaces() {
        let dirs = install_dirs_from_listing(MAC_LISTING);
        assert!(
            dirs.contains(&PathBuf::from(
                "/Applications/League of Legends.app/Contents/LoL"
            )),
            "got {:?}",
            dirs
        );

        let dirs = install_dirs_from_listing(WIN_LISTING);
        assert!(
            dirs.contains(&PathBuf::from("C:\\Riot Games\\League of Legends")),
            "got {:?}",
            dirs
        );
    }

    #[test]
    fn lockfile_candidates_sit_next_to_the_install_dir() {
        let paths = lockfile_paths_from_listing(MAC_LISTING);
        assert!(
            paths.contains(&PathBuf::from(
                "/Applications/League of Legends.app/Contents/LoL/lockfile"
            )),
            "got {:?}",
            paths
        );
    }

    #[test]
    fn region_survives_a_listing_without_credentials() {
        let listing = "/Applications/League of Legends.app/Contents/LoL/LeagueClientUx.app/Contents/MacOS/LeagueClientUx --region=EUW\n";
        assert!(credentials_from_listing(listing).is_none());
        assert_eq!(region_from_listing(listing).as_deref(), Some("EUW"));
    }
}
