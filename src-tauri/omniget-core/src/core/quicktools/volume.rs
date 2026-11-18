use serde::Serialize;

use super::QuickError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Apps,
    Master,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Target {
    pub id: String,
    pub name: String,
    pub pid: Option<u32>,
    pub volume: u8,
    pub muted: bool,
}

pub fn kind() -> Kind {
    os::kind()
}

pub async fn list() -> Result<Vec<Target>, QuickError> {
    os::list().await
}

pub async fn set(id: &str, volume: Option<u8>, muted: Option<bool>) -> Result<(), QuickError> {
    if volume.is_none() && muted.is_none() {
        return Err(QuickError::InvalidInput(
            "pass volume (0 to 100), muted, or both".into(),
        ));
    }
    if volume.is_some_and(|v| v > 100) {
        return Err(QuickError::InvalidInput("volume goes from 0 to 100".into()));
    }
    os::set(id, volume, muted).await
}

#[cfg(any(windows, target_os = "macos"))]
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, QuickError> + Send + 'static,
) -> Result<T, QuickError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| QuickError::Os(e.to_string()))?
}

#[cfg(any(target_os = "linux", test))]
fn percent_of(line: &str) -> Option<u8> {
    let end = line.find('%')?;
    let digits = line[..end].trim_end();
    let start = digits
        .rfind(|c: char| !c.is_ascii_digit())
        .map_or(0, |i| i + 1);
    digits[start..]
        .parse::<u32>()
        .ok()
        .map(|p| p.min(100) as u8)
}

#[cfg(any(target_os = "linux", test))]
fn quoted(line: &str) -> Option<&str> {
    let value = line.split_once('=')?.1.trim();
    value.strip_prefix('"')?.strip_suffix('"')
}

#[cfg(any(target_os = "linux", test))]
fn parse_pactl(text: &str) -> Vec<Target> {
    let mut out: Vec<Target> = Vec::new();
    let mut named = 0u8;
    for line in text.lines() {
        let line = line.trim();
        if let Some(index) = line.strip_prefix("Sink Input #") {
            named = 0;
            out.push(Target {
                id: index.trim().to_string(),
                name: format!("Stream {}", index.trim()),
                pid: None,
                volume: 0,
                muted: false,
            });
            continue;
        }
        let Some(current) = out.last_mut() else {
            continue;
        };
        if let Some(value) = line.strip_prefix("Mute:") {
            current.muted = value.trim() == "yes";
        } else if let Some(value) = line.strip_prefix("Volume:") {
            current.volume = percent_of(value).unwrap_or(0);
        } else if line.starts_with("application.process.id =") {
            current.pid = quoted(line).and_then(|v| v.parse().ok());
        } else {
            let rank = if line.starts_with("application.name =") {
                3
            } else if line.starts_with("application.process.binary =") {
                2
            } else if line.starts_with("media.name =") {
                1
            } else {
                0
            };
            if rank > named {
                if let Some(value) = quoted(line).filter(|v| !v.is_empty()) {
                    current.name = value.to_string();
                    named = rank;
                }
            }
        }
    }
    out.retain(|t| t.id.parse::<u32>().is_ok());
    out
}

#[cfg(any(target_os = "linux", test))]
fn parse_wpctl_status(text: &str) -> Vec<(u32, String)> {
    let mut out: Vec<(u32, String, bool, bool)> = Vec::new();
    let mut in_audio = false;
    let mut in_streams = false;
    for line in text.lines() {
        let plain = line.trim_matches(|c: char| c.is_whitespace() || "│├└─*".contains(c));
        if !line.starts_with(char::is_whitespace) && !line.is_empty() {
            in_audio = plain == "Audio";
            in_streams = false;
            continue;
        }
        if !in_audio {
            continue;
        }
        if plain.ends_with(':') {
            in_streams = plain == "Streams:";
            continue;
        }
        if !in_streams {
            continue;
        }
        let Some((id, rest)) = plain.split_once(". ") else {
            continue;
        };
        let Ok(id) = id.parse::<u32>() else {
            continue;
        };
        if rest.contains(" > ") {
            if let Some(stream) = out.last_mut() {
                stream.2 = true;
            }
        } else if rest.contains(" < ") {
            if let Some(stream) = out.last_mut() {
                stream.3 = true;
            }
        } else {
            out.push((id, rest.trim().to_string(), false, false));
        }
    }
    out.into_iter()
        .filter(|(_, _, plays, records)| *plays || !*records)
        .map(|(id, name, _, _)| (id, name))
        .collect()
}

#[cfg(any(target_os = "linux", test))]
fn parse_wpctl_volume(line: &str) -> Option<(u8, bool)> {
    let rest = line.trim().strip_prefix("Volume:")?.trim();
    let level: f32 = rest.split_whitespace().next()?.parse().ok()?;
    Some((
        (level.clamp(0.0, 1.0) * 100.0).round() as u8,
        rest.contains("[MUTED]"),
    ))
}

#[cfg(target_os = "linux")]
mod os {
    use std::process::Stdio;
    use std::sync::OnceLock;
    use std::time::Duration;

    use tokio::io::AsyncReadExt;

    use super::{Kind, QuickError, Target};

    const OUTPUT_CAP: u64 = 512 * 1024;
    const TIMEOUT: Duration = Duration::from_secs(3);

    #[derive(Clone, Copy, PartialEq)]
    enum Tool {
        Pactl,
        Wpctl,
    }

    fn on_path(name: &str) -> bool {
        std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
    }

    fn tool() -> Option<Tool> {
        static TOOL: OnceLock<Option<Tool>> = OnceLock::new();
        *TOOL.get_or_init(|| {
            if on_path("pactl") {
                Some(Tool::Pactl)
            } else if on_path("wpctl") {
                Some(Tool::Wpctl)
            } else {
                None
            }
        })
    }

    fn need_tool() -> Result<Tool, QuickError> {
        tool().ok_or_else(|| QuickError::Unavailable("neither pactl nor wpctl is installed".into()))
    }

    pub fn kind() -> Kind {
        if tool().is_some() {
            Kind::Apps
        } else {
            Kind::None
        }
    }

    async fn run(program: &str, args: &[&str]) -> Result<String, QuickError> {
        let mut cmd = crate::core::process::command(program);
        cmd.args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let work = async {
            let mut child = cmd.spawn()?;
            let mut bytes = Vec::new();
            if let Some(stdout) = child.stdout.take() {
                stdout.take(OUTPUT_CAP).read_to_end(&mut bytes).await?;
            }
            let status = child.wait().await?;
            Ok::<_, std::io::Error>((status.success(), bytes))
        };
        match tokio::time::timeout(TIMEOUT, work).await {
            Ok(Ok((true, bytes))) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
            Ok(Ok((false, _))) => Err(QuickError::Os(format!("{program} refused {args:?}"))),
            Ok(Err(e)) => Err(QuickError::Os(format!("{program}: {e}"))),
            Err(_) => Err(QuickError::Os(format!("{program} did not answer in time"))),
        }
    }

    pub async fn list() -> Result<Vec<Target>, QuickError> {
        match need_tool()? {
            Tool::Pactl => Ok(super::parse_pactl(
                &run("pactl", &["list", "sink-inputs"]).await?,
            )),
            Tool::Wpctl => {
                let streams = super::parse_wpctl_status(&run("wpctl", &["status"]).await?);
                let mut out = Vec::with_capacity(streams.len());
                for (id, name) in streams {
                    let id = id.to_string();
                    let Ok(line) = run("wpctl", &["get-volume", &id]).await else {
                        continue;
                    };
                    let Some((volume, muted)) = super::parse_wpctl_volume(&line) else {
                        continue;
                    };
                    out.push(Target {
                        id,
                        name,
                        pid: None,
                        volume,
                        muted,
                    });
                }
                Ok(out)
            }
        }
    }

    pub async fn set(id: &str, volume: Option<u8>, muted: Option<bool>) -> Result<(), QuickError> {
        let tool = need_tool()?;
        if id.parse::<u32>().is_err() {
            return Err(QuickError::InvalidInput(
                "id must be one returned by volume_list".into(),
            ));
        }
        if let Some(volume) = volume {
            match tool {
                Tool::Pactl => {
                    run(
                        "pactl",
                        &["set-sink-input-volume", id, &format!("{volume}%")],
                    )
                    .await?
                }
                Tool::Wpctl => {
                    let level = format!("{:.2}", f32::from(volume) / 100.0);
                    run("wpctl", &["set-volume", id, &level]).await?
                }
            };
        }
        if let Some(muted) = muted {
            let flag = if muted { "1" } else { "0" };
            match tool {
                Tool::Pactl => run("pactl", &["set-sink-input-mute", id, flag]).await?,
                Tool::Wpctl => run("wpctl", &["set-mute", id, flag]).await?,
            };
        }
        Ok(())
    }
}

#[cfg(windows)]
mod os {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{
        eMultimedia, eRender, AudioSessionStateExpired, IAudioSessionControl2,
        IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    use super::{blocking, Kind, QuickError, Target};

    struct Com(bool);

    impl Com {
        fn enter() -> Com {
            // SAFETY: called on a worker thread that owns no COM state; a failure (another
            // apartment model already set) is recorded so the matching uninitialize is skipped.
            Com(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: pairs with the successful CoInitializeEx of this thread, after every
                // interface created under it has been released.
                unsafe { CoUninitialize() }
            }
        }
    }

    fn os_error(e: windows::core::Error) -> QuickError {
        QuickError::Os(e.message())
    }

    fn sessions() -> windows::core::Result<Vec<(u32, ISimpleAudioVolume)>> {
        // SAFETY: COM is initialized on this thread by the caller and every interface pointer
        // comes from a successful call, so each method runs on a live object.
        unsafe {
            let devices: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = devices.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
            let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
            let list = manager.GetSessionEnumerator()?;
            let mut out = Vec::new();
            for index in 0..list.GetCount()? {
                let Ok(control) = list.GetSession(index) else {
                    continue;
                };
                if matches!(control.GetState(), Ok(state) if state == AudioSessionStateExpired) {
                    continue;
                }
                let pid = control
                    .cast::<IAudioSessionControl2>()
                    .and_then(|c| c.GetProcessId())
                    .unwrap_or(0);
                if let Ok(volume) = control.cast::<ISimpleAudioVolume>() {
                    out.push((pid, volume));
                }
            }
            Ok(out)
        }
    }

    pub fn kind() -> Kind {
        Kind::Apps
    }

    pub async fn list() -> Result<Vec<Target>, QuickError> {
        blocking(|| {
            let _com = Com::enter();
            let sessions = sessions().map_err(os_error)?;
            let pids: Vec<Pid> = sessions
                .iter()
                .filter(|(pid, _)| *pid != 0)
                .map(|(pid, _)| Pid::from_u32(*pid))
                .collect();
            let mut sys = System::new();
            sys.refresh_processes_specifics(
                ProcessesToUpdate::Some(&pids),
                true,
                ProcessRefreshKind::nothing(),
            );
            let mut out: Vec<Target> = Vec::new();
            for (pid, control) in &sessions {
                if out.iter().any(|t| t.pid == Some(*pid)) {
                    continue;
                }
                // SAFETY: `control` is a live interface returned by `sessions` on this thread.
                let (level, muted) = unsafe {
                    (
                        control.GetMasterVolume().unwrap_or(0.0),
                        control.GetMute().is_ok_and(|m| m.as_bool()),
                    )
                };
                let name = match sys.process(Pid::from_u32(*pid)) {
                    Some(p) => {
                        let name = p.name().to_string_lossy();
                        name.strip_suffix(".exe").unwrap_or(&name).to_string()
                    }
                    None if *pid == 0 => "System sounds".to_string(),
                    None => format!("PID {pid}"),
                };
                out.push(Target {
                    id: pid.to_string(),
                    name,
                    pid: Some(*pid),
                    volume: (level.clamp(0.0, 1.0) * 100.0).round() as u8,
                    muted,
                });
            }
            Ok(out)
        })
        .await
    }

    pub async fn set(id: &str, volume: Option<u8>, muted: Option<bool>) -> Result<(), QuickError> {
        let pid: u32 = id.parse().map_err(|_| {
            QuickError::InvalidInput("id must be one returned by volume_list".into())
        })?;
        blocking(move || {
            let _com = Com::enter();
            let mut found = false;
            for (_, control) in sessions()
                .map_err(os_error)?
                .iter()
                .filter(|(owner, _)| *owner == pid)
            {
                found = true;
                // SAFETY: `control` is a live interface returned by `sessions` on this thread;
                // a null event context is allowed by both setters.
                unsafe {
                    if let Some(volume) = volume {
                        control
                            .SetMasterVolume(f32::from(volume) / 100.0, std::ptr::null())
                            .map_err(os_error)?;
                    }
                    if let Some(muted) = muted {
                        control.SetMute(muted, std::ptr::null()).map_err(os_error)?;
                    }
                }
            }
            if found {
                Ok(())
            } else {
                Err(QuickError::NotFound(format!(
                    "no app with id {pid} is playing audio"
                )))
            }
        })
        .await
    }
}

#[cfg(target_os = "macos")]
mod os {
    use std::ffi::c_void;

    use super::{blocking, Kind, QuickError, Target};

    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }

    const fn code(tag: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*tag)
    }

    const SYSTEM_OBJECT: u32 = 1;
    const DEFAULT_OUTPUT: Address = Address {
        selector: code(b"dOut"),
        scope: code(b"glob"),
        element: 0,
    };
    const VOLUME: Address = Address {
        selector: code(b"vmvc"),
        scope: code(b"outp"),
        element: 0,
    };
    const MUTE: Address = Address {
        selector: code(b"mute"),
        scope: code(b"outp"),
        element: 0,
    };

    #[link(name = "CoreAudio", kind = "framework")]
    extern "C" {
        fn AudioObjectGetPropertyData(
            object: u32,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: *mut u32,
            data: *mut c_void,
        ) -> i32;
        fn AudioObjectSetPropertyData(
            object: u32,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: u32,
            data: *const c_void,
        ) -> i32;
    }

    fn get<T: Copy + Default>(object: u32, address: &Address) -> Option<T> {
        let mut value = T::default();
        let mut size = std::mem::size_of::<T>() as u32;
        // SAFETY: `value` is a plain 4-byte value and `size` is its exact length, which is what
        // each property read here returns; the address struct matches AudioObjectPropertyAddress.
        let status = unsafe {
            AudioObjectGetPropertyData(
                object,
                address,
                0,
                std::ptr::null(),
                &mut size,
                (&mut value as *mut T).cast(),
            )
        };
        (status == 0).then_some(value)
    }

    fn put<T: Copy>(object: u32, address: &Address, value: T) -> bool {
        // SAFETY: `value` lives for the whole call and its size is passed along with it.
        let status = unsafe {
            AudioObjectSetPropertyData(
                object,
                address,
                0,
                std::ptr::null(),
                std::mem::size_of::<T>() as u32,
                (&value as *const T).cast(),
            )
        };
        status == 0
    }

    fn device() -> Result<u32, QuickError> {
        get::<u32>(SYSTEM_OBJECT, &DEFAULT_OUTPUT)
            .filter(|id| *id != 0)
            .ok_or_else(|| QuickError::Unavailable("no audio output device".into()))
    }

    pub fn kind() -> Kind {
        Kind::Master
    }

    pub async fn list() -> Result<Vec<Target>, QuickError> {
        blocking(|| {
            let device = device()?;
            let Some(level) = get::<f32>(device, &VOLUME) else {
                return Ok(Vec::new());
            };
            Ok(vec![Target {
                id: "master".into(),
                name: String::new(),
                pid: None,
                volume: (level.clamp(0.0, 1.0) * 100.0).round() as u8,
                muted: get::<u32>(device, &MUTE).is_some_and(|m| m != 0),
            }])
        })
        .await
    }

    pub async fn set(id: &str, volume: Option<u8>, muted: Option<bool>) -> Result<(), QuickError> {
        if id != "master" {
            return Err(QuickError::NotFound(
                "this computer only has the master output, id \"master\"".into(),
            ));
        }
        blocking(move || {
            let device = device()?;
            let refused = || QuickError::Os("the output device refused the change".into());
            if let Some(volume) = volume {
                if !put(device, &VOLUME, f32::from(volume) / 100.0) {
                    return Err(refused());
                }
            }
            if let Some(muted) = muted {
                if !put(device, &MUTE, u32::from(muted)) {
                    return Err(refused());
                }
            }
            Ok(())
        })
        .await
    }
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
mod os {
    use super::{Kind, QuickError, Target};

    pub fn kind() -> Kind {
        Kind::None
    }

    pub async fn list() -> Result<Vec<Target>, QuickError> {
        Err(QuickError::Unavailable("no volume control here".into()))
    }

    pub async fn set(_: &str, _: Option<u8>, _: Option<bool>) -> Result<(), QuickError> {
        Err(QuickError::Unavailable("no volume control here".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACTL: &str = r#"Sink Input #71
	Driver: PipeWire
	Owner Module: n/a
	Client: 70
	Sink: 52
	Sample Specification: float32le 2ch 48000Hz
	Channel Map: front-left,front-right
	Format: pcm, format.sample_format = "\"float32le\""  format.rate = "48000"  format.channels = "2"
	Corked: no
	Mute: no
	Volume: front-left: 45875 /  70% / -9.29 dB,   front-right: 45875 /  70% / -9.29 dB
	        balance 0.00
	Buffer Latency: 0 usec
	Sink Latency: 0 usec
	Resample method: PipeWire
	Properties:
		client.api = "pipewire-pulse"
		media.name = "AudioStream"
		application.name = "Firefox"
		application.process.id = "4242"
		application.process.binary = "firefox"
		node.name = "Firefox"

Sink Input #105
	Driver: PipeWire
	Sink: 52
	Corked: no
	Mute: yes
	Volume: mono: 98304 / 150% / 10.57 dB
	        balance 0.00
	Properties:
		application.process.binary = "mpv"
		media.name = "song.flac - mpv"

Sink Input #110
	Sink: 52
	Mute: no
	Volume: front-left: 0 /   0% / -inf dB,   front-right: 0 /   0% / -inf dB
	Properties:
		node.name = "alsa_playback.aplay"
"#;

    const WPCTL: &str = r#"PipeWire 'pipewire-0' [1.0.5, ana@host, cookie:2291236093]
 └─ Clients:
        33. WirePlumber                         [1.0.5, ana@host, pid:1500]
        71. Firefox                             [1.0.5, ana@host, pid:4242]

Audio
 ├─ Devices:
 │      46. Built-in Audio                      [alsa]
 │
 ├─ Sinks:
 │  *   52. Built-in Audio Analog Stereo        [vol: 0.74]
 │
 ├─ Sources:
 │  *   53. Built-in Audio Analog Stereo        [vol: 1.00]
 │
 ├─ Filters:
 │
 └─ Streams:
        71. Firefox
             78. output_FL       > ALC295 Analog:playback_FL	[active]
             79. output_FR       > ALC295 Analog:playback_FR	[active]
        90. Chromium input
             91. input_FL        < ALC295 Analog:capture_FL	[active]
        95. mpv

Video
 ├─ Devices:
 │      60. Integrated Camera                   [v4l2]
 │
 └─ Streams:
        99. Camera viewer
             100. input_0        < Integrated Camera:capture_0	[active]

Settings
 └─ Default Configured Devices:
         0. Audio/Sink    alsa_output.pci-0000_00_1f.3.analog-stereo
"#;

    #[test]
    fn pactl_sink_inputs_become_one_row_per_stream() {
        let rows = parse_pactl(PACTL);
        assert_eq!(
            rows,
            vec![
                Target {
                    id: "71".into(),
                    name: "Firefox".into(),
                    pid: Some(4242),
                    volume: 70,
                    muted: false
                },
                Target {
                    id: "105".into(),
                    name: "mpv".into(),
                    pid: None,
                    volume: 100,
                    muted: true
                },
                Target {
                    id: "110".into(),
                    name: "Stream 110".into(),
                    pid: None,
                    volume: 0,
                    muted: false
                },
            ]
        );
    }

    #[test]
    fn pactl_with_nothing_playing_or_garbage_gives_no_rows() {
        assert!(parse_pactl("").is_empty());
        assert!(parse_pactl("Connection failure: Connection refused\n").is_empty());
        assert!(parse_pactl("Sink Input #abc; rm -rf\n\tMute: no\n").is_empty());
    }

    #[test]
    fn wpctl_status_lists_only_the_audio_streams_that_play() {
        assert_eq!(
            parse_wpctl_status(WPCTL),
            vec![(71, "Firefox".to_string()), (95, "mpv".to_string())]
        );
        assert!(parse_wpctl_status("").is_empty());
        assert!(parse_wpctl_status("Audio\n └─ Streams:\n\nVideo\n").is_empty());
    }

    #[test]
    fn wpctl_volume_lines_give_percent_and_mute() {
        assert_eq!(parse_wpctl_volume("Volume: 0.70\n"), Some((70, false)));
        assert_eq!(parse_wpctl_volume("Volume: 0.35 [MUTED]"), Some((35, true)));
        assert_eq!(parse_wpctl_volume("Volume: 1.50"), Some((100, false)));
        assert_eq!(parse_wpctl_volume("Node '71' not found"), None);
    }

    #[tokio::test]
    async fn a_change_with_nothing_to_change_or_out_of_range_is_refused_before_the_system() {
        assert!(matches!(
            set("1", None, None).await,
            Err(QuickError::InvalidInput(_))
        ));
        assert!(matches!(
            set("1", Some(101), None).await,
            Err(QuickError::InvalidInput(_))
        ));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "reads and rewrites the real output volume of this machine"]
    async fn the_master_output_reads_and_takes_back_its_own_level() {
        assert_eq!(kind(), Kind::Master);
        let rows = list().await.unwrap();
        let Some(master) = rows.first() else {
            return;
        };
        assert_eq!(master.id, "master");
        set("master", Some(master.volume), Some(master.muted))
            .await
            .unwrap();
        assert_eq!(list().await.unwrap()[0].volume, master.volume);
        assert!(matches!(
            set("other", Some(10), None).await,
            Err(QuickError::NotFound(_))
        ));
    }
}
