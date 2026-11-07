use super::Platform;

pub const NO_FFMPEG: &str = "CAPTURE_NO_FFMPEG";
pub const NO_SCREENS: &str = "CAPTURE_NO_SCREENS";
pub const NO_DISPLAY: &str = "CAPTURE_NO_DISPLAY";
pub const PERMISSION_SCREEN: &str = "CAPTURE_PERMISSION_SCREEN";
pub const PERMISSION_MIC: &str = "CAPTURE_PERMISSION_MIC";
pub const DEVICE_BUSY: &str = "CAPTURE_DEVICE_BUSY";
pub const RECORD_FAILED: &str = "CAPTURE_RECORD_FAILED";
pub const EMPTY_OUTPUT: &str = "CAPTURE_EMPTY_OUTPUT";
pub const ALREADY_RECORDING: &str = "CAPTURE_ALREADY_RECORDING";
pub const NOT_RECORDING: &str = "CAPTURE_NOT_RECORDING";
pub const BAD_OPTIONS: &str = "CAPTURE_BAD_OPTIONS";
pub const NO_SEGMENTS: &str = "CAPTURE_NO_SEGMENTS";
pub const CANCELLED: &str = "CAPTURE_CANCELLED";
pub const EXPORT_FAILED: &str = "CAPTURE_EXPORT_FAILED";
pub const PROBE_FAILED: &str = "CAPTURE_PROBE_FAILED";
pub const NOT_FOUND: &str = "CAPTURE_NOT_FOUND";
pub const OUT_DIR: &str = "CAPTURE_OUT_DIR";

pub fn code_message(code: &str, detail: &str) -> String {
    let detail = detail.trim();
    if detail.is_empty() {
        code.to_string()
    } else {
        format!("{code}: {detail}")
    }
}

pub fn tail(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("Press [q]"))
        .collect();
    let start = lines.len().saturating_sub(4);
    let mut out = lines[start..].join(" ");
    if out.chars().count() > 600 {
        out = out.chars().take(600).collect();
    }
    out
}

fn output_failure(low: &str) -> bool {
    low.contains("error opening output")
        || low.contains("could not open file")
        || low.contains("could not write header")
        || low.contains("read-only file system")
        || low.contains("no space left on device")
        || low.contains("disk quota exceeded")
}

pub fn classify_record_failure(stderr: &str, platform: Platform, with_mic: bool) -> &'static str {
    let low = stderr.to_ascii_lowercase();
    if output_failure(&low) {
        return OUT_DIR;
    }
    if low.contains("device or resource busy") || low.contains("already in use") {
        return DEVICE_BUSY;
    }
    if low.contains("cannot open display") || low.contains("can't open display") {
        return NO_DISPLAY;
    }
    if platform != Platform::Macos {
        return RECORD_FAILED;
    }
    let denied = [
        "not authorized",
        "not permitted",
        "permission",
        "denied",
        "tcc",
        "privacy",
    ]
    .iter()
    .any(|p| low.contains(p));
    let mentions_audio = low.contains("microphone") || low.contains("audio device");
    if denied {
        if with_mic && mentions_audio && !low.contains("screen") {
            return PERMISSION_MIC;
        }
        return PERMISSION_SCREEN;
    }
    if low.contains("failed to create av capture input device")
        || low.contains("could not create capture session")
        || (low.contains("error opening input") && low.contains("input/output error"))
    {
        return PERMISSION_SCREEN;
    }
    RECORD_FAILED
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_io_error_means_screen_permission() {
        let err =
            "[in#0 @ 0x1] Error opening input: Input/output error\nError opening input file 2:1.";
        assert_eq!(
            classify_record_failure(err, Platform::Macos, true),
            PERMISSION_SCREEN
        );
        assert_eq!(
            classify_record_failure(err, Platform::Linux, true),
            RECORD_FAILED
        );
    }

    #[test]
    fn explicit_denials() {
        assert_eq!(
            classify_record_failure(
                "[avfoundation @ 0x1] Failed to create AV capture input device: Cannot use Capture screen 0",
                Platform::Macos,
                false
            ),
            PERMISSION_SCREEN
        );
        assert_eq!(
            classify_record_failure(
                "Microphone access is not authorized for this app",
                Platform::Macos,
                true
            ),
            PERMISSION_MIC
        );
        assert_eq!(
            classify_record_failure("Screen capture is not authorized", Platform::Macos, true),
            PERMISSION_SCREEN
        );
    }

    #[test]
    fn busy_display_and_generic() {
        assert_eq!(
            classify_record_failure("Device or resource busy", Platform::Linux, true),
            DEVICE_BUSY
        );
        assert_eq!(
            classify_record_failure("[x11grab] Cannot open display :0", Platform::Linux, false),
            NO_DISPLAY
        );
        assert_eq!(
            classify_record_failure("Unknown encoder 'libx264'", Platform::Windows, false),
            RECORD_FAILED
        );
    }

    #[test]
    fn output_errors_are_not_permission_errors() {
        let err = "[out#0/mp4 @ 0x1] Error opening output /ro/x.mp4: Permission denied\nError opening output file /ro/x.mp4.";
        for platform in [Platform::Macos, Platform::Linux, Platform::Windows] {
            assert_eq!(classify_record_failure(err, platform, true), OUT_DIR);
        }
        assert_eq!(
            classify_record_failure(
                "/full/a.mkv: No space left on device",
                Platform::Linux,
                false
            ),
            OUT_DIR
        );
    }

    #[test]
    fn permission_codes_only_on_macos() {
        let err = "Screen capture is not authorized";
        assert_eq!(
            classify_record_failure(err, Platform::Windows, false),
            RECORD_FAILED
        );
        assert_eq!(
            classify_record_failure("Permission denied", Platform::Linux, true),
            RECORD_FAILED
        );
    }

    #[test]
    fn tail_keeps_last_lines() {
        let t = tail("a\n\nb\nPress [q] to stop\nc\nd\ne\nf");
        assert_eq!(t, "c d e f");
        assert_eq!(code_message(NOT_FOUND, " "), NOT_FOUND);
        assert_eq!(code_message(NOT_FOUND, "x"), "CAPTURE_NOT_FOUND: x");
    }
}
