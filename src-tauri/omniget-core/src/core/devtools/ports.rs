use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, Signal, System, UpdateKind};

use super::{cap_chars, DevError};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listener {
    pub port: u16,
    pub pid: u32,
    pub process: String,
    pub command: Option<String>,
    pub user: Option<String>,
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub pid: u32,
    pub process: String,
    pub command: Option<String>,
    pub ports: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KillFailure {
    pub pid: u32,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KillResult {
    pub dry_run: bool,
    pub targets: Vec<Target>,
    pub killed: Vec<u32>,
    pub failed: Vec<KillFailure>,
}

fn unescape(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while let Some(at) = rest.find("\\x") {
        out.push_str(&rest[..at]);
        let code = rest
            .get(at + 2..at + 4)
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match code {
            Some(byte) => {
                out.push(byte as char);
                rest = &rest[at + 4..];
            }
            None => {
                out.push_str("\\x");
                rest = &rest[at + 2..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn merged(rows: Vec<(u16, u32, String, Option<String>, String)>) -> Vec<Listener> {
    let mut out: Vec<Listener> = Vec::new();
    for (port, pid, process, user, address) in rows {
        match out.iter_mut().find(|l| l.port == port && l.pid == pid) {
            Some(found) => {
                if !found.addresses.contains(&address) {
                    found.addresses.push(address);
                }
            }
            None => out.push(Listener {
                port,
                pid,
                process,
                command: None,
                user,
                addresses: vec![address],
            }),
        }
    }
    out.sort_by_key(|l| (l.port, l.pid));
    out
}

pub fn parse_lsof(text: &str) -> Vec<Listener> {
    let rows = text
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let name = fields.iter().position(|f| *f == "TCP")? + 1;
            let (address, port) = fields.get(name)?.rsplit_once(':')?;
            Some((
                port.parse().ok()?,
                fields.get(1)?.parse().ok()?,
                unescape(fields.first()?),
                Some(fields.get(2)?.to_string()),
                address.to_string(),
            ))
        })
        .collect();
    merged(rows)
}

pub fn parse_netstat(text: &str) -> Vec<Listener> {
    let rows = text
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() != 5 || fields[0] != "TCP" || !fields[2].ends_with(":0") {
                return None;
            }
            let (address, port) = fields[1].rsplit_once(':')?;
            Some((
                port.parse().ok()?,
                fields[4].parse().ok()?,
                String::new(),
                None,
                address.to_string(),
            ))
        })
        .collect();
    merged(rows)
}

pub fn targets(
    listeners: &[Listener],
    port: Option<u16>,
    pid: Option<u32>,
    own_pid: u32,
) -> Result<Vec<Target>, DevError> {
    let pids: Vec<u32> = match (port, pid) {
        (None, None) => return Err(DevError::InvalidInput("pass port or pid".into())),
        (Some(port), _) => {
            let found: Vec<u32> = listeners
                .iter()
                .filter(|l| l.port == port && pid.is_none_or(|p| p == l.pid))
                .map(|l| l.pid)
                .collect();
            if found.is_empty() {
                return Err(DevError::NotFound(format!(
                    "nothing is listening on TCP port {port}"
                )));
            }
            found
        }
        (None, Some(pid)) => {
            if !listeners.iter().any(|l| l.pid == pid) {
                return Err(DevError::NotFound(format!(
                    "pid {pid} is not listening on any TCP port"
                )));
            }
            vec![pid]
        }
    };
    let mut out: Vec<Target> = Vec::new();
    for pid in pids {
        if out.iter().any(|t| t.pid == pid) {
            continue;
        }
        if pid <= 1 || pid == own_pid {
            return Err(DevError::Refused(format!(
                "pid {pid} is the system or OmniGet itself"
            )));
        }
        let own: Vec<&Listener> = listeners.iter().filter(|l| l.pid == pid).collect();
        out.push(Target {
            pid,
            process: own[0].process.clone(),
            command: own[0].command.clone(),
            ports: own.iter().map(|l| l.port).collect(),
        });
    }
    Ok(out)
}

fn output_of(program: &str, args: &[&str]) -> Result<String, DevError> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command
        .output()
        .map_err(|e| DevError::Unavailable(format!("cannot run {program}: {e}")))?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn processes(pids: &[u32]) -> System {
    let pids: Vec<Pid> = pids.iter().map(|pid| Pid::from_u32(*pid)).collect();
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );
    system
}

pub const COMMAND_CHARS: usize = 200;

fn command_line(parts: &[String]) -> Option<String> {
    if parts.is_empty() {
        return None;
    }
    let (shown, cut, _) = cap_chars(&parts.join(" "), COMMAND_CHARS);
    Some(if cut { format!("{shown}…") } else { shown })
}

pub fn list() -> Result<Vec<Listener>, DevError> {
    let mut found = if cfg!(windows) {
        parse_netstat(&output_of("netstat", &["-ano"])?)
    } else {
        parse_lsof(&output_of("lsof", &["-nP", "-iTCP", "-sTCP:LISTEN"])?)
    };
    let pids: Vec<u32> = found.iter().map(|l| l.pid).collect();
    let system = processes(&pids);
    for listener in &mut found {
        let Some(process) = system.process(Pid::from_u32(listener.pid)) else {
            continue;
        };
        listener.process = process.name().to_string_lossy().into_owned();
        let parts: Vec<String> = process
            .cmd()
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        listener.command = command_line(&parts);
    }
    Ok(found)
}

pub fn kill(port: Option<u16>, pid: Option<u32>, confirm: bool) -> Result<KillResult, DevError> {
    if port.is_none() && pid.is_none() {
        return Err(DevError::InvalidInput("pass port or pid".into()));
    }
    let targets = targets(&list()?, port, pid, std::process::id())?;
    let mut result = KillResult {
        dry_run: !confirm,
        targets,
        killed: Vec::new(),
        failed: Vec::new(),
    };
    if !confirm {
        return Ok(result);
    }
    let pids: Vec<u32> = result.targets.iter().map(|t| t.pid).collect();
    let system = processes(&pids);
    for pid in pids {
        let sent = system.process(Pid::from_u32(pid)).map(|process| {
            process
                .kill_with(Signal::Term)
                .unwrap_or_else(|| process.kill())
        });
        match sent {
            Some(true) => result.killed.push(pid),
            Some(false) => result.failed.push(KillFailure {
                pid,
                reason: "the system refused; the process may belong to another user".into(),
            }),
            None => result.failed.push(KillFailure {
                pid,
                reason: "the process is gone or not visible to this user".into(),
            }),
        }
    }
    if result.killed.is_empty() {
        let reasons: Vec<String> = result
            .failed
            .iter()
            .map(|f| format!("pid {}: {}", f.pid, f.reason))
            .collect();
        return Err(DevError::Refused(reasons.join("; ")));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LSOF: &str = "\
COMMAND     PID  USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME
Code\\x20H  3308 tonho   49u  IPv4 0x5053034a198b268e      0t0  TCP 127.0.0.1:32882 (LISTEN)
OrbStack  30717 tonho   86u  IPv4 0x4e44684a0c6e0fd6      0t0  TCP 127.0.0.1:32222 (LISTEN)
OrbStack  30717 tonho   87u  IPv6 0xe8cb09874004b5bd      0t0  TCP [::1]:32222 (LISTEN)
OrbStack  30717 tonho  127u  IPv4 0x67fd8aa2aebc6817      0t0  TCP 127.0.0.1:8090 (LISTEN)
node      41200 root    23u  IPv6 0xbbac279ab2bd9b38      0t0  TCP *:5173 (LISTEN)
";

    const NETSTAT: &str = "\r
Active Connections\r
\r
  Proto  Local Address          Foreign Address        State           PID\r
  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1180\r
  TCP    127.0.0.1:5173         0.0.0.0:0              ESCUTANDO       9012\r
  TCP    192.168.0.10:51000     140.82.112.25:443      ESTABLISHED     7044\r
  TCP    [::]:135               [::]:0                 LISTENING       1180\r
  TCP    [::1]:5173             [::]:0                 LISTENING       9012\r
  UDP    0.0.0.0:5353           *:*                                    2300\r
";

    #[test]
    fn lsof_output_captured_on_linux_parses_the_same() {
        let linux = "COMMAND  PID USER FD   TYPE  DEVICE SIZE/OFF NODE NAME\npython3 3058 root 3u  IPv4 1759623      0t0  TCP *:8123 (LISTEN)\n";
        let found = parse_lsof(linux);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].port, 8123);
        assert_eq!(found[0].pid, 3058);
        assert_eq!(found[0].process, "python3");
        assert_eq!(found[0].user.as_deref(), Some("root"));
    }

    #[test]
    fn lsof_rows_become_listeners_sorted_by_port() {
        let found = parse_lsof(LSOF);
        let ports: Vec<u16> = found.iter().map(|l| l.port).collect();
        assert_eq!(ports, vec![5173, 8090, 32222, 32882]);
        assert_eq!(
            found[0],
            Listener {
                port: 5173,
                pid: 41200,
                process: "node".into(),
                command: None,
                user: Some("root".into()),
                addresses: vec!["*".into()],
            }
        );
    }

    #[test]
    fn lsof_merges_ipv4_and_ipv6_of_one_process_and_unescapes_names() {
        let found = parse_lsof(LSOF);
        assert_eq!(found[2].pid, 30717);
        assert_eq!(found[2].addresses, vec!["127.0.0.1", "[::1]"]);
        assert_eq!(found[3].process, "Code H");
    }

    #[test]
    fn a_long_command_line_is_cut_to_a_readable_size() {
        let parts: Vec<String> = (0..100).map(|n| format!("--flag-{n}=value")).collect();
        let shown = command_line(&parts).unwrap();
        assert_eq!(shown.chars().count(), COMMAND_CHARS + 1);
        assert!(shown.starts_with("--flag-0=value --flag-1=value"));
        assert!(shown.ends_with('…'));
        assert_eq!(
            command_line(&["node".to_string(), "vite".to_string()]).as_deref(),
            Some("node vite")
        );
        assert_eq!(command_line(&[]), None);
    }

    #[test]
    fn lsof_noise_yields_nothing() {
        assert!(parse_lsof("").is_empty());
        assert!(parse_lsof("lsof: no process found\n").is_empty());
    }

    #[test]
    fn netstat_keeps_only_listening_tcp_rows_in_any_language() {
        let found = parse_netstat(NETSTAT);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].port, 135);
        assert_eq!(found[0].pid, 1180);
        assert_eq!(found[0].addresses, vec!["0.0.0.0", "[::]"]);
        assert_eq!(found[0].user, None);
        assert_eq!(found[1].port, 5173);
        assert_eq!(found[1].pid, 9012);
    }

    #[test]
    fn a_port_resolves_to_the_process_holding_it() {
        let found = parse_lsof(LSOF);
        let hit = targets(&found, Some(8090), None, 1).unwrap();
        assert_eq!(
            hit,
            vec![Target {
                pid: 30717,
                process: "OrbStack".into(),
                command: None,
                ports: vec![8090, 32222],
            }]
        );
    }

    #[test]
    fn a_pid_must_be_listening() {
        let found = parse_lsof(LSOF);
        assert_eq!(
            targets(&found, None, Some(41200), 1).unwrap()[0].ports,
            vec![5173]
        );
        assert!(matches!(
            targets(&found, None, Some(99), 1),
            Err(DevError::NotFound(_))
        ));
        assert!(matches!(
            targets(&found, Some(9), None, 1),
            Err(DevError::NotFound(_))
        ));
    }

    #[test]
    fn kill_needs_a_target_and_never_takes_this_process() {
        let found = parse_lsof(LSOF);
        assert!(matches!(
            targets(&found, None, None, 1),
            Err(DevError::InvalidInput(_))
        ));
        assert!(matches!(
            targets(&found, Some(5173), None, 41200),
            Err(DevError::Refused(_))
        ));
    }

    #[test]
    #[ignore = "smoke: reads the live listeners of this machine, run by hand with --ignored"]
    fn live_listeners_are_parsed_and_enriched() {
        let found = list().unwrap();
        for listener in &found {
            println!("{}", serde_json::to_string(listener).unwrap());
        }
        assert!(found
            .iter()
            .all(|l| l.port > 0 && l.pid > 0 && !l.process.is_empty()));
        assert!(found.windows(2).all(|pair| pair[0].port <= pair[1].port));
    }
}
