use std::collections::BTreeMap;

use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_DWORD, REG_EXPAND_SZ,
    REG_MULTI_SZ, REG_QWORD, REG_SZ,
};

use super::parse::{RegEntry, Root, UNINSTALL};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

impl Key {
    fn open(hive: HKEY, path: &str, view: u32) -> Option<Key> {
        let p = wide(path);
        let mut h: HKEY = std::ptr::null_mut();
        let rc = unsafe { RegOpenKeyExW(hive, p.as_ptr(), 0, KEY_READ | view, &mut h) };
        (rc == ERROR_SUCCESS).then_some(Key(h))
    }

    pub fn subkeys(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut buf = vec![0u16; 512];
        for i in 0u32..100_000 {
            let mut len = buf.len() as u32;
            let rc = unsafe {
                RegEnumKeyExW(
                    self.0,
                    i,
                    buf.as_mut_ptr(),
                    &mut len,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if rc == ERROR_NO_MORE_ITEMS {
                break;
            }
            if rc != ERROR_SUCCESS {
                continue;
            }
            out.push(String::from_utf16_lossy(&buf[..len as usize]));
        }
        out
    }

    pub fn values(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        let mut name = vec![0u16; 16_384];
        let mut data: Vec<u8> = vec![0u8; 4096];
        let mut i = 0u32;
        loop {
            let mut name_len = name.len() as u32;
            let mut data_len = data.len() as u32;
            let mut kind = 0u32;
            let rc = unsafe {
                RegEnumValueW(
                    self.0,
                    i,
                    name.as_mut_ptr(),
                    &mut name_len,
                    std::ptr::null(),
                    &mut kind,
                    data.as_mut_ptr(),
                    &mut data_len,
                )
            };
            if rc == ERROR_NO_MORE_ITEMS {
                break;
            }
            if rc == ERROR_MORE_DATA {
                data.resize((data_len as usize).max(data.len() * 2).min(1 << 20), 0);
                if data.len() >= (1 << 20) && data_len as usize > data.len() {
                    i += 1;
                }
                continue;
            }
            i += 1;
            if rc != ERROR_SUCCESS {
                continue;
            }
            let n = String::from_utf16_lossy(&name[..name_len as usize]);
            let d = &data[..data_len as usize];
            let text = match kind {
                REG_SZ | REG_EXPAND_SZ => utf16(d),
                REG_MULTI_SZ => utf16(d)
                    .split('\0')
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n"),
                REG_DWORD if d.len() >= 4 => {
                    format!("0x{:x}", u32::from_le_bytes([d[0], d[1], d[2], d[3]]))
                }
                REG_QWORD if d.len() >= 8 => format!(
                    "0x{:x}",
                    u64::from_le_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]])
                ),
                _ => continue,
            };
            out.insert(n, text);
        }
        out
    }
}

fn utf16(d: &[u8]) -> String {
    let units: Vec<u16> = d
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
        .trim_end_matches('\0')
        .to_string()
}

fn root_parts(root: Root) -> (HKEY, String, u32) {
    match root {
        Root::Hklm => (
            HKEY_LOCAL_MACHINE,
            format!(r"SOFTWARE\{UNINSTALL}"),
            KEY_WOW64_64KEY,
        ),
        Root::Hklm32 => (
            HKEY_LOCAL_MACHINE,
            format!(r"SOFTWARE\{UNINSTALL}"),
            KEY_WOW64_32KEY,
        ),
        Root::Hkcu => (HKEY_CURRENT_USER, format!(r"Software\{UNINSTALL}"), 0),
        Root::Hkcu32 => (
            HKEY_CURRENT_USER,
            format!(r"Software\WOW6432Node\{UNINSTALL}"),
            0,
        ),
    }
}

pub fn read_entries(root: Root) -> Vec<RegEntry> {
    let (hive, path, view) = root_parts(root);
    let Some(k) = Key::open(hive, &path, view) else {
        return Vec::new();
    };
    k.subkeys()
        .into_iter()
        .filter_map(|name| {
            let sub = Key::open(hive, &format!(r"{path}\{name}"), view)?;
            Some(RegEntry {
                root,
                key_name: name,
                values: sub.values(),
            })
        })
        .collect()
}

pub fn read_entry(root: Root, key_name: &str) -> Option<RegEntry> {
    if key_name.is_empty() || key_name.contains('\\') {
        return None;
    }
    let (hive, path, view) = root_parts(root);
    let sub = Key::open(hive, &format!(r"{path}\{key_name}"), view)?;
    Some(RegEntry {
        root,
        key_name: key_name.to_string(),
        values: sub.values(),
    })
}

pub fn split_full(full: &str) -> Option<(HKEY, String, u32)> {
    let (hive, rest) = full.split_once('\\')?;
    let hive = match hive.to_ascii_uppercase().as_str() {
        "HKLM" | "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
        "HKCU" | "HKEY_CURRENT_USER" => HKEY_CURRENT_USER,
        _ => return None,
    };
    Some((hive, rest.to_string(), KEY_WOW64_64KEY))
}

pub fn exists(full: &str) -> bool {
    split_full(full)
        .and_then(|(h, p, v)| Key::open(h, &p, v))
        .is_some()
}

pub fn children(full: &str) -> Vec<String> {
    split_full(full)
        .and_then(|(h, p, v)| Key::open(h, &p, v))
        .map(|k| k.subkeys())
        .unwrap_or_default()
}

pub fn value(full: &str, name: &str) -> Option<String> {
    let (h, p, v) = split_full(full)?;
    let k = Key::open(h, &p, v)?;
    k.values()
        .into_iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
}

pub fn services() -> Vec<(String, String)> {
    let base = r"SYSTEM\CurrentControlSet\Services";
    let Some(k) = Key::open(HKEY_LOCAL_MACHINE, base, KEY_WOW64_64KEY) else {
        return Vec::new();
    };
    k.subkeys()
        .into_iter()
        .filter_map(|name| {
            let sub = Key::open(
                HKEY_LOCAL_MACHINE,
                &format!(r"{base}\{name}"),
                KEY_WOW64_64KEY,
            )?;
            let image = sub
                .values()
                .into_iter()
                .find(|(n, _)| n.eq_ignore_ascii_case("ImagePath"))
                .map(|(_, v)| v)?;
            Some((name, image))
        })
        .collect()
}
