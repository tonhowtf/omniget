use std::process::Stdio;

use super::devices::Platform;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteError {
    Accessibility(String),
    Automation(String),
    NoTool,
    Failed(String),
}

impl PasteError {
    pub fn code(&self) -> &'static str {
        match self {
            PasteError::Accessibility(_) => "PASTE_ACCESSIBILITY",
            PasteError::Automation(_) => "PASTE_AUTOMATION",
            PasteError::NoTool => "PASTE_NO_TOOL",
            PasteError::Failed(_) => "PASTE_FAILED",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            PasteError::Accessibility(d) | PasteError::Automation(d) | PasteError::Failed(d) => {
                d.clone()
            }
            PasteError::NoTool => String::new(),
        }
    }
}

pub fn classify_osascript(stderr: &str) -> PasteError {
    let lower = stderr.to_lowercase();
    let detail = stderr.trim().to_string();
    if lower.contains("-1743") || lower.contains("not authorized to send apple events") {
        PasteError::Automation(detail)
    } else if lower.contains("1002")
        || lower.contains("-1719")
        || lower.contains("-25211")
        || lower.contains("assistive access")
        || lower.contains("not allowed to send keystrokes")
    {
        PasteError::Accessibility(detail)
    } else {
        PasteError::Failed(detail)
    }
}

pub fn paste_commands(platform: Platform, wayland: bool) -> Vec<(&'static str, Vec<&'static str>)> {
    match platform {
        Platform::Mac => vec![(
            "osascript",
            vec![
                "-e",
                "tell application \"System Events\" to keystroke \"v\" using command down",
            ],
        )],
        Platform::Windows => vec![(
            "powershell",
            vec![
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('^v')",
            ],
        )],
        Platform::Linux => {
            let wtype = ("wtype", vec!["-M", "ctrl", "-k", "v", "-m", "ctrl"]);
            let xdotool = ("xdotool", vec!["key", "--clearmodifiers", "ctrl+v"]);
            if wayland {
                vec![wtype, xdotool]
            } else {
                vec![xdotool, wtype]
            }
        }
    }
}

pub fn is_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

pub async fn send_paste() -> Result<(), PasteError> {
    let platform = Platform::current();
    let mut last: Option<PasteError> = None;
    for (program, args) in paste_commands(platform, is_wayland()) {
        let out = crate::core::process::command(program)
            .args(&args)
            .stdin(Stdio::null())
            .output()
            .await;
        match out {
            Ok(o) if o.status.success() => return Ok(()),
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                let err = if platform == Platform::Mac {
                    classify_osascript(&stderr)
                } else {
                    PasteError::Failed(stderr.trim().to_string())
                };
                last = Some(err);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => last = Some(PasteError::Failed(e.to_string())),
        }
    }
    Err(last.unwrap_or(PasteError::NoTool))
}

pub fn paste_supported() -> bool {
    let platform = Platform::current();
    platform != Platform::Linux
        || paste_commands(platform, is_wayland())
            .iter()
            .any(|(program, _)| which(program))
}

fn which(program: &str) -> bool {
    let path = crate::core::process::enhanced_path()
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();
    std::env::split_paths(&path).any(|d| d.join(program).is_file())
}

pub fn privacy_url(platform: Platform, pane: &str) -> Option<&'static str> {
    match (platform, pane) {
        (Platform::Mac, "accessibility") => {
            Some("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        }
        (Platform::Mac, "automation") => {
            Some("x-apple.systempreferences:com.apple.preference.security?Privacy_Automation")
        }
        (Platform::Mac, "microphone") => {
            Some("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
        }
        (Platform::Windows, "microphone") => Some("ms-settings:privacy-microphone"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osascript_errors_are_classified() {
        let a = classify_osascript(
            "36:83: execution error: System Events got an error: osascript is not allowed to send keystrokes. (1002)",
        );
        assert_eq!(a.code(), "PASTE_ACCESSIBILITY");
        let b = classify_osascript(
            "execution error: Not authorized to send Apple events to System Events. (-1743)",
        );
        assert_eq!(b.code(), "PASTE_AUTOMATION");
        let c = classify_osascript("something odd happened");
        assert_eq!(c.code(), "PASTE_FAILED");
        assert_eq!(c.detail(), "something odd happened");
    }

    #[test]
    fn commands_per_platform() {
        let mac = paste_commands(Platform::Mac, false);
        assert_eq!(mac[0].0, "osascript");
        assert!(mac[0].1[1].contains("command down"));
        let win = paste_commands(Platform::Windows, false);
        assert!(win[0].1.last().unwrap().contains("^v"));
        assert_eq!(paste_commands(Platform::Linux, true)[0].0, "wtype");
        assert_eq!(paste_commands(Platform::Linux, false)[0].0, "xdotool");
    }

    #[test]
    fn privacy_urls() {
        assert!(privacy_url(Platform::Mac, "accessibility")
            .unwrap()
            .ends_with("Privacy_Accessibility"));
        assert_eq!(privacy_url(Platform::Linux, "microphone"), None);
        assert_eq!(privacy_url(Platform::Mac, "bogus"), None);
    }
}
