use std::path::Path;

use super::TransferError;

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub host: String,
    pub port: u16,
    pub kind: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Known,
    New,
    Changed { expected: String },
}

pub fn parse(text: &str) -> Vec<Record> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [host, port, kind, fingerprint] = fields[..] else {
                return None;
            };
            Some(Record {
                host: host.to_string(),
                port: port.parse().ok()?,
                kind: kind.to_string(),
                fingerprint: fingerprint.to_string(),
            })
        })
        .collect()
}

pub fn render(records: &[Record]) -> String {
    records
        .iter()
        .map(|r| format!("{} {} {} {}\n", r.host, r.port, r.kind, r.fingerprint))
        .collect()
}

fn same_place(record: &Record, host: &str, port: u16) -> bool {
    record.port == port && record.host.eq_ignore_ascii_case(host)
}

pub fn judge(records: &[Record], host: &str, port: u16, kind: &str, fingerprint: &str) -> Verdict {
    let mut saved = records
        .iter()
        .filter(|r| same_place(r, host, port) && r.kind == kind)
        .peekable();
    let Some(first) = saved.peek() else {
        return Verdict::New;
    };
    let expected = first.fingerprint.clone();
    if saved.any(|r| r.fingerprint == fingerprint) {
        Verdict::Known
    } else {
        Verdict::Changed { expected }
    }
}

fn read(file: &Path) -> Result<Vec<Record>, TransferError> {
    match std::fs::read_to_string(file) {
        Ok(text) => Ok(parse(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(TransferError::Storage(e.to_string())),
    }
}

fn write(file: &Path, records: &[Record]) -> Result<(), TransferError> {
    let failed = |e: std::io::Error| TransferError::Storage(e.to_string());
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(failed)?;
    }
    std::fs::write(file, render(records)).map_err(failed)
}

pub fn verify(
    file: &Path,
    host: &str,
    port: u16,
    kind: &str,
    fingerprint: &str,
) -> Result<Verdict, TransferError> {
    let mut records = read(file)?;
    let verdict = judge(&records, host, port, kind, fingerprint);
    if verdict == Verdict::New {
        records.push(Record {
            host: host.to_string(),
            port,
            kind: kind.to_string(),
            fingerprint: fingerprint.to_string(),
        });
        write(file, &records)?;
    }
    Ok(verdict)
}

pub fn forget(file: &Path, host: &str, port: u16) -> Result<usize, TransferError> {
    let records = read(file)?;
    let kept: Vec<Record> = records
        .iter()
        .filter(|r| !same_place(r, host, port))
        .cloned()
        .collect();
    let removed = records.len() - kept.len();
    if removed > 0 {
        write(file, &kept)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(host: &str, port: u16, kind: &str, fingerprint: &str) -> Record {
        Record {
            host: host.into(),
            port,
            kind: kind.into(),
            fingerprint: fingerprint.into(),
        }
    }

    #[test]
    fn the_file_round_trips_and_skips_broken_lines() {
        let text = "nas.local 22 ssh SHA256:abc\n\nbroken line\n[::1] 2121 tls SHA256:def\nhost notaport ssh SHA256:x\n";
        let records = parse(text);
        assert_eq!(
            records,
            vec![
                record("nas.local", 22, "ssh", "SHA256:abc"),
                record("[::1]", 2121, "tls", "SHA256:def"),
            ]
        );
        assert_eq!(parse(&render(&records)), records);
    }

    #[test]
    fn the_first_key_of_a_host_is_new_and_the_same_key_is_known() {
        let records = vec![record("nas.local", 22, "ssh", "SHA256:abc")];
        assert_eq!(
            judge(&[], "nas.local", 22, "ssh", "SHA256:abc"),
            Verdict::New
        );
        assert_eq!(
            judge(&records, "nas.local", 22, "ssh", "SHA256:abc"),
            Verdict::Known
        );
        assert_eq!(
            judge(&records, "NAS.local", 22, "ssh", "SHA256:abc"),
            Verdict::Known
        );
    }

    #[test]
    fn another_key_for_the_same_host_and_port_is_a_change() {
        let records = vec![record("nas.local", 22, "ssh", "SHA256:abc")];
        assert_eq!(
            judge(&records, "nas.local", 22, "ssh", "SHA256:evil"),
            Verdict::Changed {
                expected: "SHA256:abc".into()
            }
        );
    }

    #[test]
    fn another_port_or_another_kind_is_a_separate_identity() {
        let records = vec![record("nas.local", 22, "ssh", "SHA256:abc")];
        assert_eq!(
            judge(&records, "nas.local", 2222, "ssh", "SHA256:zzz"),
            Verdict::New
        );
        assert_eq!(
            judge(&records, "nas.local", 22, "tls", "SHA256:zzz"),
            Verdict::New
        );
    }

    #[test]
    fn verifying_saves_on_first_use_and_refuses_a_change_without_saving_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nested/known_hosts");
        assert_eq!(
            verify(&file, "h", 22, "ssh", "SHA256:one").unwrap(),
            Verdict::New
        );
        assert_eq!(
            verify(&file, "h", 22, "ssh", "SHA256:one").unwrap(),
            Verdict::Known
        );
        assert_eq!(
            verify(&file, "h", 22, "ssh", "SHA256:two").unwrap(),
            Verdict::Changed {
                expected: "SHA256:one".into()
            }
        );
        assert_eq!(
            parse(&std::fs::read_to_string(&file).unwrap()),
            vec![record("h", 22, "ssh", "SHA256:one")]
        );
    }

    #[test]
    fn forgetting_removes_every_kind_for_that_host_and_port_only() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        verify(&file, "h", 22, "ssh", "SHA256:one").unwrap();
        verify(&file, "h", 22, "tls", "SHA256:cert").unwrap();
        verify(&file, "h", 2222, "ssh", "SHA256:other").unwrap();
        assert_eq!(forget(&file, "h", 22).unwrap(), 2);
        assert_eq!(forget(&file, "h", 22).unwrap(), 0);
        assert_eq!(
            verify(&file, "h", 22, "ssh", "SHA256:two").unwrap(),
            Verdict::New
        );
        assert_eq!(
            verify(&file, "h", 2222, "ssh", "SHA256:other").unwrap(),
            Verdict::Known
        );
    }

    #[test]
    fn forgetting_with_no_file_is_zero() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(forget(&dir.path().join("missing"), "h", 22).unwrap(), 0);
    }
}
