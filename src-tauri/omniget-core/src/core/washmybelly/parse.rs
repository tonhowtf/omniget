//! Text parsers shared by several modules. Pure functions, compiled on every
//! OS so each CI tests the parsers of the other systems.

use std::collections::HashMap;

/// `/etc/os-release` as a map (`ID`, `ID_LIKE`, `VERSION_ID`, `PRETTY_NAME`).
pub fn os_release(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.starts_with('#') {
                return None;
            }
            let (k, v) = l.split_once('=')?;
            let v = v.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|x| x.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
                .unwrap_or(v);
            Some((k.trim().to_string(), v.replace("\\\"", "\"")))
        })
        .collect()
}

/// One value of `reg query` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegValue {
    pub key: String,
    pub name: String,
    pub kind: String,
    pub data: String,
}

/// `reg query <key> [/v name] [/s]` output: key lines start at column 0,
/// value lines are `    Name    REG_TYPE    Data`.
pub fn reg_query(text: &str) -> Vec<RegValue> {
    let mut out = Vec::new();
    let mut key = String::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') {
            key = line.trim().to_string();
            continue;
        }
        let t = line.trim_start();
        // Split on the first run of 4 spaces around the REG_ type.
        let Some(pos) = t.find("    REG_") else {
            continue;
        };
        let name = t[..pos].trim_end().to_string();
        let rest = t[pos..].trim_start();
        let (kind, data) = match rest.find("    ") {
            Some(i) => (rest[..i].to_string(), rest[i..].trim().to_string()),
            None => (rest.trim().to_string(), String::new()),
        };
        out.push(RegValue {
            key: key.clone(),
            name: if name == "(Default)" || name == "(Padrão)" {
                String::new()
            } else {
                name
            },
            kind,
            data,
        });
    }
    out
}

/// Whether `whoami /groups /fo csv` lists a SID.
pub fn whoami_has_sid(csv: &str, sid: &str) -> bool {
    csv.lines()
        .any(|l| l.split(',').any(|c| c.trim_matches('"') == sid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_release_reads_quoted_and_bare_values() {
        let m =
            os_release("NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\nVERSION_ID=\"24.04\"\n# c\n");
        assert_eq!(m["ID"], "ubuntu");
        assert_eq!(m["ID_LIKE"], "debian");
        assert_eq!(m["VERSION_ID"], "24.04");
    }

    #[test]
    fn reg_query_reads_values_under_their_key() {
        let t = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\r\n    ProductName    REG_SZ    Windows 10 Pro\r\n    CurrentBuild    REG_SZ    26100\r\n    UBR    REG_DWORD    0x1234\r\n    (Default)    REG_SZ    \r\n";
        let v = reg_query(t);
        assert_eq!(v.len(), 4);
        assert_eq!(v[0].name, "ProductName");
        assert_eq!(v[0].data, "Windows 10 Pro");
        assert_eq!(v[2].kind, "REG_DWORD");
        assert_eq!(v[2].data, "0x1234");
        assert_eq!(v[3].name, "");
        assert!(v[0].key.ends_with("CurrentVersion"));
    }

    #[test]
    fn whoami_finds_the_performance_log_users_sid() {
        let csv = "\"Group Name\",\"Type\",\"SID\",\"Attributes\"\n\"BUILTIN\\Performance Log Users\",\"Alias\",\"S-1-5-32-559\",\"Mandatory group\"\n";
        assert!(whoami_has_sid(csv, "S-1-5-32-559"));
        assert!(!whoami_has_sid(csv, "S-1-5-32-544"));
    }
}
