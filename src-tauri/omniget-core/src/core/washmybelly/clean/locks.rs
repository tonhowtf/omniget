//! Who holds a file open, through the Windows Restart Manager
//! (`RmStartSession`, `RmRegisterResources`, `RmGetList`, `RmEndSession`).
//! The cleanup engine uses it to mark an item "in use by <app>" instead of
//! failing half way (idea from Kudu, MIT, `src/main/services/cleaner-blockers.ts`,
//! which does the same through PowerShell; here it is a direct call).
//!
//! Only reads. Outside Windows it returns nothing.

use std::path::PathBuf;

/// Most paths asked about in one call (Kudu uses the same cap).
pub const MAX_PATHS: usize = 512;

/// For each path that some process holds, the display names of those
/// processes (`"Windows Explorer"`, `"Google Chrome"`, or the service short
/// name for a service). Paths nobody holds are left out.
pub fn lockers(paths: &[PathBuf]) -> Vec<(PathBuf, Vec<String>)> {
    let paths = &paths[..paths.len().min(MAX_PATHS)];
    if paths.is_empty() {
        return Vec::new();
    }
    imp::lockers(paths)
}

#[cfg(not(windows))]
mod imp {
    use std::path::PathBuf;

    pub fn lockers(_paths: &[PathBuf]) -> Vec<(PathBuf, Vec<String>)> {
        Vec::new()
    }
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;

    use windows_sys::core::PCWSTR;
    use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
    use windows_sys::Win32::System::RestartManager::{
        RmEndSession, RmGetList, RmRegisterResources, RmStartSession, CCH_RM_SESSION_KEY,
        RM_PROCESS_INFO,
    };

    fn wide(p: &std::path::Path) -> Vec<u16> {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn from_wide(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end]).trim().to_string()
    }

    /// One Restart Manager session over `paths`; the processes holding any
    /// of them. `None` when the API fails (treated as "unknown", not "free").
    fn holders(paths: &[PathBuf]) -> Option<Vec<String>> {
        let wides: Vec<Vec<u16>> = paths.iter().map(|p| wide(p)).collect();
        let ptrs: Vec<PCWSTR> = wides.iter().map(|w| w.as_ptr()).collect();
        let mut handle: u32 = 0;
        let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
        // SAFETY: `handle` and `key` are valid for writes; the key buffer is
        // CCH_RM_SESSION_KEY + 1 wide chars as the API requires.
        let rc = unsafe { RmStartSession(&mut handle, 0, key.as_mut_ptr()) };
        if rc != ERROR_SUCCESS {
            return None;
        }
        let result = (|| {
            // SAFETY: `ptrs` holds `ptrs.len()` NUL-terminated wide strings
            // that outlive the call (`wides` is alive until this closure ends).
            let rc = unsafe {
                RmRegisterResources(
                    handle,
                    ptrs.len() as u32,
                    ptrs.as_ptr(),
                    0,
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                )
            };
            if rc != ERROR_SUCCESS {
                return None;
            }
            let mut needed: u32 = 0;
            let mut count: u32 = 0;
            let mut reasons: u32 = 0;
            // SAFETY: first call with a null buffer and count 0 only reports
            // how many entries are needed.
            let rc = unsafe {
                RmGetList(
                    handle,
                    &mut needed,
                    &mut count,
                    std::ptr::null_mut(),
                    &mut reasons,
                )
            };
            if rc == ERROR_SUCCESS {
                return Some(Vec::new());
            }
            if rc != ERROR_MORE_DATA {
                return None;
            }
            // Processes can start between the two calls: retry a few times.
            for _ in 0..3 {
                let mut buf: Vec<RM_PROCESS_INFO> =
                    vec![RM_PROCESS_INFO::default(); needed as usize + 4];
                count = buf.len() as u32;
                // SAFETY: `buf` has room for `count` entries.
                let rc = unsafe {
                    RmGetList(
                        handle,
                        &mut needed,
                        &mut count,
                        buf.as_mut_ptr(),
                        &mut reasons,
                    )
                };
                if rc == ERROR_MORE_DATA {
                    continue;
                }
                if rc != ERROR_SUCCESS {
                    return None;
                }
                buf.truncate(count as usize);
                let mut names: Vec<String> = buf
                    .iter()
                    .map(|info| {
                        let app = from_wide(&info.strAppName);
                        let svc = from_wide(&info.strServiceShortName);
                        match (app.is_empty(), svc.is_empty()) {
                            (false, _) => app,
                            (true, false) => svc,
                            (true, true) => format!("pid {}", info.Process.dwProcessId),
                        }
                    })
                    .collect();
                names.sort();
                names.dedup();
                return Some(names);
            }
            None
        })();
        // SAFETY: `handle` came from a successful RmStartSession.
        unsafe { RmEndSession(handle) };
        result
    }

    pub fn lockers(paths: &[PathBuf]) -> Vec<(PathBuf, Vec<String>)> {
        // One session for the whole batch first: nearly always nobody holds
        // anything and that is a single round trip. Only when someone does,
        // ask per path to know which file each process holds.
        match holders(paths) {
            Some(v) if v.is_empty() => return Vec::new(),
            None => return Vec::new(),
            Some(v) if paths.len() == 1 => return vec![(paths[0].clone(), v)],
            Some(_) => {}
        }
        paths
            .iter()
            .filter_map(|p| match holders(std::slice::from_ref(p)) {
                Some(v) if !v.is_empty() => Some((p.clone(), v)),
                _ => None,
            })
            .collect()
    }
}
