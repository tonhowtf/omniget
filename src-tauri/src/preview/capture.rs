//! "SnapShot": list the windows of other apps and capture one to a PNG under
//! `<app_data>/snapshots/`, so the composer can attach it. Also captures a
//! rectangle of the screen for the preview window on Windows/Linux.
//!
//! macOS: own FFI to `CGWindowListCopyWindowInfo` (no JXA, no crate) and
//! `/usr/sbin/screencapture -l <windowId> -o -x -t png`. Without the Screen
//! Recording permission the list has no titles for other apps and the
//! capture comes out as wallpaper; `permission` says which case we are in.
//! Windows: PowerShell + user32 (`EnumWindows` through `Get-Process`,
//! `PrintWindow` with PW_RENDERFULLCONTENT). Linux: `wmctrl -lp` + ImageMagick
//! `import -window`, `grim` on Wayland for the whole screen. Best effort.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use omniget_core::core::process;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    /// `screen` for the whole screen, otherwise the OS window id/handle.
    pub id: String,
    pub app: String,
    pub title: String,
    pub pid: Option<u32>,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowList {
    pub windows: Vec<WindowInfo>,
    /// macOS: Screen Recording granted. Other OSes: always true.
    pub permission: bool,
    /// What the list could not do on this platform (e.g. Wayland).
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Captured {
    pub path: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bytes: u64,
}

pub fn snapshots_dir() -> Result<PathBuf, String> {
    let dir = omniget_core::core::paths::app_data_dir()
        .ok_or("SNAPSHOT_DIR: no data dir")?
        .join("snapshots");
    std::fs::create_dir_all(&dir).map_err(|e| format!("SNAPSHOT_DIR: {e}"))?;
    Ok(dir)
}

pub fn new_png_path(prefix: &str) -> Result<PathBuf, String> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let short = uuid::Uuid::new_v4().simple().to_string();
    Ok(snapshots_dir()?.join(format!("{prefix}-{stamp}-{}.png", &short[..6])))
}

/// Width/height from the IHDR chunk; `None` when this is not a PNG.
pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 24 || bytes[..8] != SIG || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some((w, h))
}

fn finish(path: &Path) -> Result<Captured, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("SNAPSHOT_FAILED: no image written ({e})"))?;
    let Some((w, h)) = png_size(&bytes) else {
        let _ = std::fs::remove_file(path);
        return Err("SNAPSHOT_FAILED: the capture is not a PNG".into());
    };
    Ok(Captured {
        path: path.to_string_lossy().into_owned(),
        width: Some(w),
        height: Some(h),
        bytes: bytes.len() as u64,
    })
}

async fn run_status(program: &str, args: &[String]) -> Result<String, String> {
    let mut cmd = process::command(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let child = process::spawn_retrying_busy(|| cmd.spawn())
        .map_err(|e| format!("SNAPSHOT_TOOL: {program}: {e}"))?;
    let out = tokio::time::timeout(Duration::from_secs(15), child.wait_with_output())
        .await
        .map_err(|_| format!("SNAPSHOT_TIMEOUT: {program}"))?
        .map_err(|e| format!("SNAPSHOT_TOOL: {program}: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(format!(
            "SNAPSHOT_FAILED: {program} exited with {} {err}",
            out.status
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

// ── macOS ─────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
#[allow(non_upper_case_globals, non_snake_case)]
mod mac {
    use std::ffi::{c_char, c_void, CStr};

    type CFTypeRef = *const c_void;
    type CFArrayRef = *const c_void;
    type CFDictionaryRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFNumberRef = *const c_void;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct CGPoint {
        pub x: f64,
        pub y: f64,
    }
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct CGSize {
        pub width: f64,
        pub height: f64,
    }
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct CGRect {
        pub origin: CGPoint,
        pub size: CGSize,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFArrayGetCount(a: CFArrayRef) -> isize;
        fn CFArrayGetValueAtIndex(a: CFArrayRef, i: isize) -> *const c_void;
        fn CFDictionaryGetValue(d: CFDictionaryRef, key: *const c_void) -> *const c_void;
        fn CFNumberGetValue(n: CFNumberRef, the_type: isize, out: *mut c_void) -> u8;
        fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, size: isize, enc: u32) -> u8;
        fn CFStringGetLength(s: CFStringRef) -> isize;
        fn CFRelease(r: CFTypeRef);
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFArrayRef;
        fn CGRectMakeWithDictionaryRepresentation(d: CFDictionaryRef, rect: *mut CGRect) -> u8;
        fn CGPreflightScreenCaptureAccess() -> u8;
        fn CGRequestScreenCaptureAccess() -> u8;
        static kCGWindowNumber: CFStringRef;
        static kCGWindowOwnerName: CFStringRef;
        static kCGWindowOwnerPID: CFStringRef;
        static kCGWindowName: CFStringRef;
        static kCGWindowLayer: CFStringRef;
        static kCGWindowBounds: CFStringRef;
        static kCGWindowAlpha: CFStringRef;
    }

    const kCGWindowListOptionOnScreenOnly: u32 = 1 << 0;
    const kCGWindowListOptionAll: u32 = 0;
    const kCGWindowListExcludeDesktopElements: u32 = 1 << 4;
    const kCFNumberSInt64Type: isize = 4;
    const kCFNumberFloat64Type: isize = 6;
    const kCFStringEncodingUTF8: u32 = 0x0800_0100;

    pub struct RawWindow {
        pub number: u32,
        pub owner: String,
        pub pid: u32,
        pub name: String,
        pub layer: i64,
        pub alpha: f64,
        pub bounds: CGRect,
    }

    unsafe fn num_i64(d: CFDictionaryRef, key: CFStringRef) -> Option<i64> {
        let v = CFDictionaryGetValue(d, key);
        if v.is_null() {
            return None;
        }
        let mut out: i64 = 0;
        (CFNumberGetValue(v, kCFNumberSInt64Type, &mut out as *mut i64 as *mut c_void) != 0)
            .then_some(out)
    }

    unsafe fn num_f64(d: CFDictionaryRef, key: CFStringRef) -> Option<f64> {
        let v = CFDictionaryGetValue(d, key);
        if v.is_null() {
            return None;
        }
        let mut out: f64 = 0.0;
        (CFNumberGetValue(v, kCFNumberFloat64Type, &mut out as *mut f64 as *mut c_void) != 0)
            .then_some(out)
    }

    unsafe fn string(d: CFDictionaryRef, key: CFStringRef) -> String {
        let v = CFDictionaryGetValue(d, key);
        if v.is_null() {
            return String::new();
        }
        let len = CFStringGetLength(v);
        let cap = (len * 4 + 1).max(1) as usize;
        let mut buf = vec![0 as c_char; cap];
        if CFStringGetCString(v, buf.as_mut_ptr(), cap as isize, kCFStringEncodingUTF8) == 0 {
            return String::new();
        }
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }

    /// Every window, on-screen only unless `all`.
    pub fn windows(all: bool) -> Vec<RawWindow> {
        let mut res = Vec::new();
        unsafe {
            let opts = if all {
                kCGWindowListOptionAll
            } else {
                kCGWindowListOptionOnScreenOnly
            } | kCGWindowListExcludeDesktopElements;
            let arr = CGWindowListCopyWindowInfo(opts, 0);
            if arr.is_null() {
                return res;
            }
            let n = CFArrayGetCount(arr);
            for i in 0..n {
                let d = CFArrayGetValueAtIndex(arr, i);
                if d.is_null() {
                    continue;
                }
                let mut bounds = CGRect::default();
                let b = CFDictionaryGetValue(d, kCGWindowBounds);
                if !b.is_null() {
                    CGRectMakeWithDictionaryRepresentation(b, &mut bounds);
                }
                res.push(RawWindow {
                    number: num_i64(d, kCGWindowNumber).unwrap_or(0) as u32,
                    owner: string(d, kCGWindowOwnerName),
                    pid: num_i64(d, kCGWindowOwnerPID).unwrap_or(0) as u32,
                    name: string(d, kCGWindowName),
                    layer: num_i64(d, kCGWindowLayer).unwrap_or(0),
                    alpha: num_f64(d, kCGWindowAlpha).unwrap_or(1.0),
                    bounds,
                });
            }
            CFRelease(arr);
        }
        res
    }

    pub fn has_permission() -> bool {
        unsafe { CGPreflightScreenCaptureAccess() != 0 }
    }

    pub fn request_permission() -> bool {
        unsafe { CGRequestScreenCaptureAccess() != 0 }
    }
}

/// macOS: the CGWindowID of one of our own windows, by exact title.
#[cfg(target_os = "macos")]
pub fn own_window_number(title: &str) -> Option<u32> {
    let pid = std::process::id();
    mac::windows(true)
        .into_iter()
        .find(|w| w.pid == pid && w.name == title)
        .map(|w| w.number)
}

/// Asks the OS for the Screen Recording permission (macOS). Elsewhere a no-op.
pub fn request_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        mac::request_permission()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// The windows the user can pick, biggest apps first, our own left out.
pub async fn list_windows() -> Result<WindowList, String> {
    let screen = WindowInfo {
        id: "screen".into(),
        app: String::new(),
        title: String::new(),
        pid: None,
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    };
    #[cfg(target_os = "macos")]
    {
        let own = std::process::id();
        let permission = mac::has_permission();
        let mut windows: Vec<WindowInfo> =
            tauri::async_runtime::spawn_blocking(|| mac::windows(false))
                .await
                .map_err(|e| format!("SNAPSHOT_LIST: {e}"))?
                .into_iter()
                .filter(|w| {
                    w.pid != own
                        && w.layer == 0
                        && w.alpha > 0.0
                        && w.bounds.size.width >= 60.0
                        && w.bounds.size.height >= 40.0
                })
                .filter(|w| {
                    !matches!(
                        w.owner.as_str(),
                        "Window Server" | "Dock" | "Control Center" | "Notification Center"
                    )
                })
                .map(|w| WindowInfo {
                    id: w.number.to_string(),
                    app: w.owner,
                    title: w.name,
                    pid: Some(w.pid),
                    x: w.bounds.origin.x,
                    y: w.bounds.origin.y,
                    width: w.bounds.size.width,
                    height: w.bounds.size.height,
                })
                .collect();
        windows.insert(0, screen);
        return Ok(WindowList {
            windows,
            permission,
            note: (!permission).then(|| "macOS Screen Recording permission is off: titles are hidden and captures of other apps come out blank.".to_string()),
        });
    }
    #[cfg(windows)]
    {
        let script = "Get-Process | Where-Object { $_.MainWindowHandle -ne 0 -and $_.MainWindowTitle } | Select-Object Id,ProcessName,MainWindowTitle,@{n='Handle';e={[int64]$_.MainWindowHandle}} | ConvertTo-Json -Compress";
        let out = run_status(
            "powershell",
            &[
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                script.into(),
            ],
        )
        .await?;
        let v: serde_json::Value =
            serde_json::from_str(out.trim()).unwrap_or(serde_json::Value::Null);
        let items = match v {
            serde_json::Value::Array(a) => a,
            serde_json::Value::Object(_) => vec![v],
            _ => vec![],
        };
        let own = std::process::id();
        let mut windows = vec![screen];
        for it in items {
            let pid = it.get("Id").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            if pid == own {
                continue;
            }
            windows.push(WindowInfo {
                id: it
                    .get("Handle")
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0)
                    .to_string(),
                app: it
                    .get("ProcessName")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                title: it
                    .get("MainWindowTitle")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                pid: Some(pid),
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            });
        }
        return Ok(WindowList {
            windows,
            permission: true,
            note: None,
        });
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let mut windows = vec![screen];
        let own = std::process::id();
        if let Ok(out) = run_status("wmctrl", &["-lpG".into()]).await {
            for line in out.lines() {
                // 0x03a00003  0 4242   10 20 800 600 host Title words
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() < 8 {
                    continue;
                }
                let pid: u32 = cols[2].parse().unwrap_or(0);
                if pid == own {
                    continue;
                }
                let title = cols[8..].join(" ");
                windows.push(WindowInfo {
                    id: cols[0].to_string(),
                    app: String::new(),
                    title,
                    pid: (pid != 0).then_some(pid),
                    x: cols[3].parse().unwrap_or(0.0),
                    y: cols[4].parse().unwrap_or(0.0),
                    width: cols[5].parse().unwrap_or(0.0),
                    height: cols[6].parse().unwrap_or(0.0),
                });
            }
        }
        let note = if windows.len() == 1 {
            Some(if wayland {
                "Wayland does not let apps list other windows; only the whole screen can be captured (grim).".to_string()
            } else {
                "Install wmctrl to list windows; the whole screen can still be captured."
                    .to_string()
            })
        } else {
            None
        };
        return Ok(WindowList {
            windows,
            permission: true,
            note,
        });
    }
    #[allow(unreachable_code)]
    Ok(WindowList {
        windows: vec![screen],
        permission: true,
        note: None,
    })
}

/// Captures one window (id from [`list_windows`], or `screen`) to a new PNG.
pub async fn capture_window(id: &str) -> Result<Captured, String> {
    let path = new_png_path("snapshot")?;
    let p = path.to_string_lossy().into_owned();
    #[cfg(target_os = "macos")]
    {
        let mut args: Vec<String> = vec!["-x".into(), "-t".into(), "png".into()];
        if id != "screen" {
            let n: u32 = id
                .parse()
                .map_err(|_| "SNAPSHOT_INVALID: bad window id".to_string())?;
            args.extend(["-o".into(), "-l".into(), n.to_string()]);
        }
        args.push(p.clone());
        match run_status("/usr/sbin/screencapture", &args).await {
            Ok(_) => return finish(&path),
            Err(e) if id != "screen" => {
                // `-l` refuses some windows ("could not create image from
                // window"); fall back to the window rectangle on screen.
                let n: u32 = id.parse().unwrap_or(0);
                let bounds = tauri::async_runtime::spawn_blocking(move || {
                    mac::windows(false)
                        .into_iter()
                        .find(|w| w.number == n)
                        .map(|w| w.bounds)
                })
                .await
                .ok()
                .flatten()
                .ok_or(e)?;
                let r = format!(
                    "{},{},{},{}",
                    bounds.origin.x.round(),
                    bounds.origin.y.round(),
                    bounds.size.width.round(),
                    bounds.size.height.round()
                );
                let args: Vec<String> = vec![
                    "-x".into(),
                    "-t".into(),
                    "png".into(),
                    "-R".into(),
                    r,
                    p.clone(),
                ];
                run_status("/usr/sbin/screencapture", &args).await?;
                return finish(&path);
            }
            Err(e) => return Err(e),
        }
    }
    #[cfg(windows)]
    {
        let script = windows_capture_script(id, &p);
        run_status(
            "powershell",
            &[
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                script,
            ],
        )
        .await?;
        return finish(&path);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if id == "screen" {
            let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
            let attempts: Vec<(&str, Vec<String>)> = if wayland {
                vec![
                    ("grim", vec![p.clone()]),
                    ("gnome-screenshot", vec!["-f".into(), p.clone()]),
                    (
                        "spectacle",
                        vec![
                            "-b".into(),
                            "-n".into(),
                            "-f".into(),
                            "-o".into(),
                            p.clone(),
                        ],
                    ),
                ]
            } else {
                vec![
                    ("import", vec!["-window".into(), "root".into(), p.clone()]),
                    ("gnome-screenshot", vec!["-f".into(), p.clone()]),
                    ("scrot", vec![p.clone()]),
                ]
            };
            let mut last = String::from(
                "SNAPSHOT_UNSUPPORTED: no screenshot tool (grim, import, gnome-screenshot, scrot)",
            );
            for (prog, args) in attempts {
                match run_status(prog, &args).await {
                    Ok(_) => return finish(&path),
                    Err(e) => last = e,
                }
            }
            return Err(last);
        }
        run_status("import", &["-window".into(), id.to_string(), p.clone()]).await?;
        return finish(&path);
    }
    #[allow(unreachable_code)]
    Err(format!("SNAPSHOT_UNSUPPORTED: {id} {p}"))
}

/// Captures a rectangle of the screen in physical pixels (preview window on
/// Windows/Linux, where there is no per-window capture of our own webview).
pub async fn capture_rect(
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    prefix: &str,
) -> Result<Captured, String> {
    let path = new_png_path(prefix)?;
    let p = path.to_string_lossy().into_owned();
    #[cfg(target_os = "macos")]
    {
        // screencapture -R takes points; callers on macOS use the window id.
        let args: Vec<String> = vec![
            "-x".into(),
            "-t".into(),
            "png".into(),
            "-R".into(),
            format!("{x},{y},{w},{h}"),
            p.clone(),
        ];
        run_status("/usr/sbin/screencapture", &args).await?;
        return finish(&path);
    }
    #[cfg(windows)]
    {
        let script = format!(
            "Add-Type -AssemblyName System.Drawing; $b = New-Object System.Drawing.Bitmap {w},{h}; $g = [System.Drawing.Graphics]::FromImage($b); $g.CopyFromScreen({x},{y},0,0,$b.Size); $b.Save('{}', [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $b.Dispose()",
            p.replace('\'', "''")
        );
        run_status(
            "powershell",
            &[
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                script,
            ],
        )
        .await?;
        return finish(&path);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let geometry = format!("{x},{y} {w}x{h}");
        if run_status("grim", &["-g".into(), geometry, p.clone()])
            .await
            .is_ok()
        {
            return finish(&path);
        }
        run_status(
            "import",
            &[
                "-window".into(),
                "root".into(),
                "-crop".into(),
                format!("{w}x{h}+{x}+{y}"),
                p.clone(),
            ],
        )
        .await?;
        return finish(&path);
    }
    #[allow(unreachable_code)]
    Err(format!("SNAPSHOT_UNSUPPORTED: {x} {y} {w} {h} {p}"))
}

/// macOS: captures one of our own windows by its CGWindowID.
#[cfg(target_os = "macos")]
pub async fn capture_own_window(number: u32, prefix: &str) -> Result<Captured, String> {
    let path = new_png_path(prefix)?;
    let p = path.to_string_lossy().into_owned();
    let args: Vec<String> = vec![
        "-x".into(),
        "-o".into(),
        "-t".into(),
        "png".into(),
        "-l".into(),
        number.to_string(),
        p,
    ];
    run_status("/usr/sbin/screencapture", &args).await?;
    finish(&path)
}

#[cfg(windows)]
fn windows_capture_script(id: &str, path: &str) -> String {
    let path = path.replace('\'', "''");
    if id == "screen" {
        return format!(
            "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; $s = [System.Windows.Forms.SystemInformation]::VirtualScreen; $b = New-Object System.Drawing.Bitmap $s.Width,$s.Height; $g = [System.Drawing.Graphics]::FromImage($b); $g.CopyFromScreen($s.Left,$s.Top,0,0,$b.Size); $b.Save('{path}', [System.Drawing.Imaging.ImageFormat]::Png)"
        );
    }
    let handle: i64 = id.parse().unwrap_or(0);
    format!(
        r#"Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public struct OGRECT {{ public int Left, Top, Right, Bottom; }}
public static class OGWin {{
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out OGRECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
}}
"@
$h = [IntPtr]{handle}
$r = New-Object OGRECT
[void][OGWin]::GetWindowRect($h, [ref]$r)
$w = $r.Right - $r.Left; $hh = $r.Bottom - $r.Top
if ($w -le 0 -or $hh -le 0) {{ exit 3 }}
$b = New-Object System.Drawing.Bitmap $w,$hh
$g = [System.Drawing.Graphics]::FromImage($b)
$dc = $g.GetHdc()
[void][OGWin]::PrintWindow($h, $dc, 2)
$g.ReleaseHdc($dc)
$b.Save('{path}', [System.Drawing.Imaging.ImageFormat]::Png)
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_size_reads_ihdr() {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&640u32.to_be_bytes());
        b.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(png_size(&b), Some((640, 480)));
        assert_eq!(png_size(b"GIF89a....................."), None);
    }

    /// Live: `cargo test -p omniget --lib live_list_windows -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live_list_windows() {
        let list = list_windows().await.unwrap();
        println!(
            "permission={} note={:?} count={}",
            list.permission,
            list.note,
            list.windows.len()
        );
        for w in list.windows.iter().take(8) {
            println!(
                "{} | {} | {} | {}x{}",
                w.id, w.app, w.title, w.width, w.height
            );
        }
        assert_eq!(list.windows[0].id, "screen");
        if let Ok(dir) = std::env::var("SNAPSHOT_TEST_CAPTURE") {
            std::env::set_var("OMNIGET_DATA_DIR", dir);
            let shot = capture_window(
                &list
                    .windows
                    .get(1)
                    .map(|w| w.id.clone())
                    .unwrap_or("screen".into()),
            )
            .await;
            println!("{shot:?}");
        }
    }
}
