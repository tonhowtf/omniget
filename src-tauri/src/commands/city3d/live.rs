//! Live expression in the 3D city (estudo 13, sessão M7).
//!
//! - On-demand MediaPipe files for the webview path (macOS and Windows): the
//!   wasm and the `.task` models are downloaded once into the app data dir,
//!   sha256-checked, and served to the webview as bytes. They never go into
//!   the bundle.
//! - The native camera path for Linux, where WebKitGTK has neither a usable
//!   `getUserMedia` nor WebGL in workers: `nokhwa` (V4L2) + the managed ONNX
//!   Runtime run the pose landmark model, and only the landmarks (about 530
//!   bytes a frame) go to the webview over a `Channel`. The solver in
//!   `src/lib/city3d/live/solver.ts` turns them into bones, same as on the
//!   other platforms. Pixels never leave this process.
//! - `validate_gesture`: the server-side twin of `src/lib/city3d/live/gesture.ts`.
//!
//! Linux model: MediaPipe ships `.task` (TFLite) only. The pose landmark model
//! (BlazePose GHUM lite, Apache-2.0) is expected as ONNX at
//! `<app data>/city3d/live/onnx/pose_landmark_lite.onnx`. To produce it:
//! unzip `pose_landmarker_lite.task`, then
//! `python -m tf2onnx.convert --tflite pose_landmarks_detector.tflite --opset 17 --output pose_landmark_lite.onnx`.
//! Inputs 1×256×256×3 (NHWC) or 1×3×256×256 in [0, 1]; the outputs are picked
//! by size (195 = 39 landmarks × 5, 117 = 39 world landmarks × 3, 1 = presence).
//! Face blendshapes are not on the native path yet (the blendshape model
//! needs the 146-landmark subset from the face mesh; see the handoff).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::ipc::{Channel, Response};

// ------------------------------------------------------------------ assets

pub const MEDIAPIPE_VERSION: &str = "1.0.1";

pub struct LiveAsset {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

/// Same table as `LIVE_ASSETS` in `src/lib/city3d/live/assets.ts`. The
/// hashes are of the npm package `@mediapipe/tasks-vision@1.0.1` and of the
/// official model files (float16, version 1), checked on 2026-09-29.
pub const LIVE_ASSETS: &[LiveAsset] = &[
    LiveAsset {
        name: "vision_wasm_module_internal.js",
        url: "https://cdn.jsdelivr.net/npm/@mediapipe/tasks-vision@1.0.1/wasm/vision_wasm_module_internal.js",
        sha256: "da8934057f147b622e82cfb4c0dbd85461c598e268588b5a8ba9ca963a8ff82d",
        bytes: 323_415,
    },
    LiveAsset {
        name: "vision_wasm_module_internal.wasm",
        url: "https://cdn.jsdelivr.net/npm/@mediapipe/tasks-vision@1.0.1/wasm/vision_wasm_module_internal.wasm",
        sha256: "2dabd8e23c60984628beb7bb338764c81a08e6837145273f59578684b5d53c1b",
        bytes: 11_756_972,
    },
    LiveAsset {
        name: "face_landmarker.task",
        url: "https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task",
        sha256: "64184e229b263107bc2b804c6625db1341ff2bb731874b0bcc2fe6544e0bc9ff",
        bytes: 3_758_596,
    },
    LiveAsset {
        name: "pose_landmarker_lite.task",
        url: "https://storage.googleapis.com/mediapipe-models/pose_landmarker/pose_landmarker_lite/float16/1/pose_landmarker_lite.task",
        sha256: "59929e1d1ee95287735ddd833b19cf4ac46d29bc7afddbbf6753c459690d574a",
        bytes: 5_777_746,
    },
    LiveAsset {
        name: "hand_landmarker.task",
        url: "https://storage.googleapis.com/mediapipe-models/hand_landmarker/hand_landmarker/float16/1/hand_landmarker.task",
        sha256: "fbc2a30080c3c557093b5ddfc334698132eb341044ccee322ccf8bcf3607cde1",
        bytes: 7_819_105,
    },
];

fn live_dir() -> Result<PathBuf, String> {
    omniget_core::core::paths::app_data_dir()
        .map(|d| d.join("city3d").join("live"))
        .ok_or_else(|| "could not find the app data directory".to_string())
}

fn asset_path(a: &LiveAsset) -> Result<PathBuf, String> {
    Ok(live_dir()?
        .join(format!("mediapipe-{MEDIAPIPE_VERSION}"))
        .join(a.name))
}

fn installed(a: &LiveAsset) -> bool {
    asset_path(a)
        .ok()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.is_file() && m.len() == a.bytes)
        .unwrap_or(false)
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetStatus {
    pub name: &'static str,
    pub bytes: u64,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetProgress {
    pub name: String,
    pub received: u64,
    pub total: u64,
    pub cached: bool,
}

/// What is on disk already.
#[tauri::command]
pub fn city3d_camera_assets() -> Vec<AssetStatus> {
    LIVE_ASSETS
        .iter()
        .map(|a| AssetStatus {
            name: a.name,
            bytes: a.bytes,
            installed: installed(a),
        })
        .collect()
}

static DOWNLOAD_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Bytes of one asset, downloading and checking it the first time.
#[tauri::command]
pub async fn city3d_camera_asset(
    name: String,
    on_progress: Channel<AssetProgress>,
) -> Result<Response, String> {
    let asset = LIVE_ASSETS
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| format!("unknown live asset {name}"))?;
    let path = asset_path(asset)?;
    let _guard = DOWNLOAD_LOCK.lock().await;
    if !installed(asset) {
        download(asset, &path, &on_progress).await?;
    } else {
        let _ = on_progress.send(AssetProgress {
            name: name.clone(),
            received: asset.bytes,
            total: asset.bytes,
            cached: true,
        });
    }
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| format!("reading {}: {e}", path.display()))?;
    Ok(Response::new(bytes))
}

async fn download(
    asset: &LiveAsset,
    path: &std::path::Path,
    on_progress: &Channel<AssetProgress>,
) -> Result<(), String> {
    use futures::StreamExt;
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncWriteExt;

    let dir = path.parent().ok_or_else(|| "bad asset path".to_string())?;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let part = path.with_extension("part");
    let client = reqwest::Client::builder()
        .user_agent(concat!("omniget/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let res = client
        .get(asset.url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("downloading {}: {e}", asset.name))?;
    let total = res.content_length().unwrap_or(asset.bytes);
    let mut file = tokio::fs::File::create(&part)
        .await
        .map_err(|e| format!("creating {}: {e}", part.display()))?;
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    let mut last_report = 0u64;
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("downloading {}: {e}", asset.name))?;
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("writing {}: {e}", part.display()))?;
        received += chunk.len() as u64;
        if received - last_report >= 256 * 1024 || received == total {
            last_report = received;
            let _ = on_progress.send(AssetProgress {
                name: asset.name.to_string(),
                received,
                total,
                cached: false,
            });
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    let got = hex::encode(hasher.finalize());
    if got != asset.sha256 || received != asset.bytes {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(format!(
            "{} failed the integrity check (sha256 {got}, {received} bytes)",
            asset.name
        ));
    }
    tokio::fs::rename(&part, path)
        .await
        .map_err(|e| format!("saving {}: {e}", path.display()))?;
    Ok(())
}

// ------------------------------------------------------------- native camera

/// Where the Linux path looks for the ONNX pose model (see the module docs).
pub fn pose_onnx_path() -> Result<PathBuf, String> {
    Ok(live_dir()?.join("onnx").join("pose_landmark_lite.onnx"))
}

#[derive(Debug, Clone, Serialize)]
pub struct CameraEntry {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// Pose landmarks for the webview solver, in MediaPipe's world convention
/// (metres, hip-centred, x right, y down, z away from the camera).
#[derive(Debug, Clone, Serialize)]
pub struct BodyFrame {
    pub world: Vec<f32>,
    pub visibility: Vec<f32>,
}

/// Messages on the `city3d_camera_start` channel. Mirrors `NativeMessage` in
/// `src/lib/city3d/live/live.ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NativeMessage {
    Status {
        state: &'static str,
        message: Option<String>,
    },
    Frame {
        /// Milliseconds since the stream started.
        t: f64,
        infer_ms: f64,
        body: Option<BodyFrame>,
    },
}

/// Cameras the native path can open (empty outside Linux, where the webview
/// lists them itself).
#[tauri::command]
pub async fn city3d_camera_list() -> Result<Vec<CameraEntry>, String> {
    #[cfg(target_os = "linux")]
    {
        tokio::task::spawn_blocking(native::list)
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(Vec::new())
    }
}

/// Opens the camera and streams pose landmarks on `channel` until
/// `city3d_camera_stop`. `device` is an id from `city3d_camera_list`.
#[tauri::command]
pub async fn city3d_camera_start(
    device: Option<String>,
    channel: Channel<NativeMessage>,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let model = pose_onnx_path()?;
        if !model.is_file() {
            return Err(format!(
                "the pose model is not installed (expected at {}); see commands/city3d/live.rs",
                model.display()
            ));
        }
        tokio::task::spawn_blocking(move || native::start(device, model, channel))
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (device, channel);
        Err("the native camera path is Linux only; the webview handles the camera here".into())
    }
}

#[tauri::command]
pub async fn city3d_camera_stop() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        tokio::task::spawn_blocking(native::stop)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Square region of interest in frame pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Roi {
    pub cx: f32,
    pub cy: f32,
    pub size: f32,
}

impl Roi {
    pub fn full(w: u32, h: u32) -> Self {
        Roi {
            cx: w as f32 / 2.0,
            cy: h as f32 / 2.0,
            size: w.max(h) as f32,
        }
    }
}

/// Crops `roi` out of an RGB8 frame into a `side`×`side` float tensor in
/// [0, 1] (bilinear, black outside the frame), NHWC or NCHW.
pub fn crop_to_tensor(rgb: &[u8], w: u32, h: u32, roi: Roi, side: usize, nchw: bool) -> Vec<f32> {
    let mut out = vec![0f32; side * side * 3];
    let (w, h) = (w as i64, h as i64);
    let x0 = roi.cx - roi.size / 2.0;
    let y0 = roi.cy - roi.size / 2.0;
    let step = roi.size / side as f32;
    let px = |x: i64, y: i64, c: usize| -> f32 {
        if x < 0 || y < 0 || x >= w || y >= h {
            0.0
        } else {
            rgb[((y * w + x) * 3) as usize + c] as f32
        }
    };
    let plane = side * side;
    for v in 0..side {
        let sy = y0 + (v as f32 + 0.5) * step - 0.5;
        let yi = sy.floor();
        let fy = sy - yi;
        let yi = yi as i64;
        for u in 0..side {
            let sx = x0 + (u as f32 + 0.5) * step - 0.5;
            let xi = sx.floor();
            let fx = sx - xi;
            let xi = xi as i64;
            for c in 0..3 {
                let a = px(xi, yi, c) * (1.0 - fx) + px(xi + 1, yi, c) * fx;
                let b = px(xi, yi + 1, c) * (1.0 - fx) + px(xi + 1, yi + 1, c) * fx;
                let val = (a * (1.0 - fy) + b * fy) / 255.0;
                let i = if nchw {
                    c * plane + v * side + u
                } else {
                    (v * side + u) * 3 + c
                };
                out[i] = val;
            }
        }
    }
    out
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Turns the raw landmark outputs into a `BodyFrame` plus the next ROI.
/// `lm` is 39 × (x, y, z, visibility, presence) in crop pixels, `world` is
/// 39 × (x, y, z) in metres.
pub fn decode_pose(
    lm: &[f32],
    world: &[f32],
    roi: Roi,
    side: f32,
    w: u32,
    h: u32,
) -> (BodyFrame, Roi) {
    let mut out_world = Vec::with_capacity(99);
    let mut vis = Vec::with_capacity(33);
    let scale = roi.size / side;
    let x0 = roi.cx - roi.size / 2.0;
    let y0 = roi.cy - roi.size / 2.0;
    let to_frame = |i: usize| (x0 + lm[i * 5] * scale, y0 + lm[i * 5 + 1] * scale);
    for i in 0..33 {
        out_world.extend_from_slice(&world[i * 3..i * 3 + 3]);
        let (fx, fy) = to_frame(i);
        let inside = fx >= -0.05 * w as f32
            && fx <= 1.05 * w as f32
            && fy >= -0.05 * h as f32
            && fy <= 1.05 * h as f32;
        vis.push(if inside { sigmoid(lm[i * 5 + 3]) } else { 0.0 });
    }
    // Landmarks 33 and 34 are the ROI helpers: hip centre and a point on the
    // body-size circle. MediaPipe sizes the next crop at 2 × radius × 1.25.
    let (cx, cy) = to_frame(33);
    let (sx, sy) = to_frame(34);
    let radius = ((sx - cx).powi(2) + (sy - cy).powi(2)).sqrt();
    let next = if radius.is_finite() && radius > 8.0 {
        Roi {
            cx,
            cy,
            size: (radius * 2.0 * 1.25).min(w.max(h) as f32 * 2.0),
        }
    } else {
        Roi::full(w, h)
    };
    (
        BodyFrame {
            world: out_world,
            visibility: vis,
        },
        next,
    )
}

#[cfg(target_os = "linux")]
mod native {
    //! nokhwa capture thread + ONNX pose model (Linux only).

    use super::{crop_to_tensor, decode_pose, BodyFrame, CameraEntry, NativeMessage, Roi};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};
    use tauri::ipc::Channel;

    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{
        ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType,
        Resolution,
    };
    use nokhwa::Camera;

    const SIDE: usize = 256;
    /// Pose at 15 Hz, as in the webview path.
    const PERIOD: Duration = Duration::from_millis(66);

    struct Running {
        stop: Arc<AtomicBool>,
        handle: JoinHandle<()>,
    }

    static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

    fn index_of(id: &str) -> CameraIndex {
        match id.parse::<u32>() {
            Ok(i) => CameraIndex::Index(i),
            Err(_) => CameraIndex::String(id.to_string()),
        }
    }

    pub fn list() -> Result<Vec<CameraEntry>, String> {
        let cams = nokhwa::query(ApiBackend::Auto).map_err(|e| e.to_string())?;
        Ok(cams
            .into_iter()
            .map(|c| CameraEntry {
                id: c.index().to_string(),
                name: c.human_name(),
                description: c.description().to_string(),
            })
            .collect())
    }

    pub fn stop() {
        let running = RUNNING.lock().ok().and_then(|mut g| g.take());
        if let Some(r) = running {
            r.stop.store(true, Ordering::SeqCst);
            let _ = r.handle.join();
        }
    }

    pub fn start(
        device: Option<String>,
        model: PathBuf,
        channel: Channel<NativeMessage>,
    ) -> Result<(), String> {
        stop();
        omniget_core::core::onnxrt::init().map_err(|e| e.to_string())?;
        let session = omniget_core::core::tools::onnx::session_from_file(&model)
            .map_err(|e| e.to_string())?;
        let mut pose = PoseModel::new(session)?;
        let stop_flag = Arc::new(AtomicBool::new(false));
        let flag = stop_flag.clone();
        let index = device
            .as_deref()
            .map(index_of)
            .unwrap_or(CameraIndex::Index(0));
        let handle = std::thread::Builder::new()
            .name("city3d-camera".into())
            .spawn(move || {
                let send = |m: NativeMessage| {
                    let _ = channel.send(m);
                };
                send(NativeMessage::Status {
                    state: "opening",
                    message: None,
                });
                if let Err(e) = run(index, &mut pose, &flag, &send) {
                    send(NativeMessage::Status {
                        state: "error",
                        message: Some(e),
                    });
                    return;
                }
                send(NativeMessage::Status {
                    state: "stopped",
                    message: None,
                });
            })
            .map_err(|e| e.to_string())?;
        if let Ok(mut g) = RUNNING.lock() {
            *g = Some(Running {
                stop: stop_flag,
                handle,
            });
        }
        Ok(())
    }

    fn run(
        index: CameraIndex,
        pose: &mut PoseModel,
        stop: &AtomicBool,
        send: &dyn Fn(NativeMessage),
    ) -> Result<(), String> {
        let wanted = CameraFormat::new(Resolution::new(640, 480), FrameFormat::YUYV, 30);
        let mut camera = Camera::new(
            index,
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(wanted)),
        )
        .map_err(|e| format!("opening the camera: {e}"))?;
        camera
            .open_stream()
            .map_err(|e| format!("starting the camera: {e}"))?;
        send(NativeMessage::Status {
            state: "running",
            message: None,
        });
        let t0 = Instant::now();
        let mut last = Instant::now() - PERIOD;
        let mut roi: Option<Roi> = None;
        while !stop.load(Ordering::SeqCst) {
            let frame = match camera.frame() {
                Ok(f) => f,
                Err(e) => {
                    let _ = camera.stop_stream();
                    return Err(format!("reading the camera: {e}"));
                }
            };
            if last.elapsed() < PERIOD {
                continue;
            }
            last = Instant::now();
            let (rgb, w, h) = match frame.decode_image::<RgbFormat>() {
                Ok(img) => {
                    let (w, h) = img.dimensions();
                    (img.into_raw(), w, h)
                }
                // Built without mozjpeg: MJPEG frames go through `image`.
                Err(_) if frame.source_frame_format() == FrameFormat::MJPEG => {
                    match image::load_from_memory_with_format(
                        frame.buffer(),
                        image::ImageFormat::Jpeg,
                    ) {
                        Ok(img) => {
                            let img = img.to_rgb8();
                            let (w, h) = img.dimensions();
                            (img.into_raw(), w, h)
                        }
                        Err(_) => continue,
                    }
                }
                Err(_) => continue,
            };
            let r = roi.unwrap_or_else(|| Roi::full(w, h));
            let started = Instant::now();
            let result = pose.infer(&rgb, w, h, r);
            let infer_ms = started.elapsed().as_secs_f64() * 1000.0;
            let body = match result {
                Ok(Some((body, next))) => {
                    roi = Some(next);
                    Some(body)
                }
                Ok(None) => {
                    roi = None;
                    None
                }
                Err(e) => {
                    let _ = camera.stop_stream();
                    return Err(e);
                }
            };
            send(NativeMessage::Frame {
                t: t0.elapsed().as_secs_f64() * 1000.0,
                infer_ms,
                body,
            });
        }
        let _ = camera.stop_stream();
        Ok(())
    }

    struct PoseModel {
        session: ort::session::Session,
        input: String,
        nchw: bool,
    }

    impl PoseModel {
        fn new(session: ort::session::Session) -> Result<Self, String> {
            let first = session
                .inputs()
                .first()
                .ok_or_else(|| "the pose model has no input".to_string())?;
            let input = first.name().to_string();
            let nchw = first
                .dtype()
                .tensor_shape()
                .map(|s| s.len() == 4 && s[1] == 3)
                .unwrap_or(false);
            Ok(PoseModel {
                session,
                input,
                nchw,
            })
        }

        /// `Ok(None)` when no person is in the crop.
        fn infer(
            &mut self,
            rgb: &[u8],
            w: u32,
            h: u32,
            roi: Roi,
        ) -> Result<Option<(BodyFrame, Roi)>, String> {
            let data = crop_to_tensor(rgb, w, h, roi, SIDE, self.nchw);
            let shape: Vec<i64> = if self.nchw {
                vec![1, 3, SIDE as i64, SIDE as i64]
            } else {
                vec![1, SIDE as i64, SIDE as i64, 3]
            };
            let tensor =
                ort::value::Tensor::from_array((shape, data)).map_err(|e| e.to_string())?;
            let inputs: Vec<(
                std::borrow::Cow<'static, str>,
                ort::session::SessionInputValue<'_>,
            )> = vec![(std::borrow::Cow::Owned(self.input.clone()), tensor.into())];
            let outputs = self
                .session
                .run(inputs)
                .map_err(|e| format!("pose inference: {e}"))?;
            let mut lm: Option<Vec<f32>> = None;
            let mut world: Option<Vec<f32>> = None;
            let mut flag = 1.0f32;
            for i in 0..outputs.len() {
                let Ok((_, raw)) = outputs[i].try_extract_tensor::<f32>() else {
                    continue;
                };
                match raw.len() {
                    195 => lm = Some(raw.to_vec()),
                    117 => world = Some(raw.to_vec()),
                    1 => {
                        let v = raw[0];
                        flag = if (0.0..=1.0).contains(&v) {
                            v
                        } else {
                            1.0 / (1.0 + (-v).exp())
                        };
                    }
                    _ => {}
                }
            }
            let (Some(lm), Some(world)) = (lm, world) else {
                return Err(
                    "the pose model does not have the expected outputs (195 and 117 values)".into(),
                );
            };
            if flag < 0.5 {
                return Ok(None);
            }
            Ok(Some(decode_pose(&lm, &world, roi, SIDE as f32, w, h)))
        }
    }
}

// ------------------------------------------------------------ gesture check

/// Clip ids, as in `CLIP_CATALOG` (gesture.ts).
pub const GESTURE_CLIPS: &[&str] = &[
    "wave",
    "nod",
    "shake_head",
    "shrug",
    "point_at",
    "clap",
    "bow",
    "cheer",
    "think",
    "facepalm",
    "thumbs_up",
    "cross_arms",
    "stretch",
    "yawn",
    "laugh",
    "sit_tired",
];

const BONES: &[&str] = &[
    "pelvis",
    "spine",
    "chest",
    "neck",
    "head",
    "shoulder_left",
    "shoulder_right",
    "elbow_left",
    "elbow_right",
    "wrist_left",
    "wrist_right",
    "hip_left",
    "hip_right",
    "knee_left",
    "knee_right",
    "ankle_left",
    "ankle_right",
    "thumb_left",
    "index_left",
    "middle_left",
    "ring_left",
    "pinky_left",
    "thumb_right",
    "index_right",
    "middle_right",
    "ring_right",
    "pinky_right",
];

const GROUPS: &[(&str, &[&str])] = &[
    ("shoulders", &["shoulder_left", "shoulder_right"]),
    ("elbows", &["elbow_left", "elbow_right"]),
    ("forearms", &["elbow_left", "elbow_right"]),
    ("wrists", &["wrist_left", "wrist_right"]),
    ("hips", &["hip_left", "hip_right"]),
    ("knees", &["knee_left", "knee_right"]),
    ("ankles", &["ankle_left", "ankle_right"]),
    (
        "fingers_left",
        &[
            "thumb_left",
            "index_left",
            "middle_left",
            "ring_left",
            "pinky_left",
        ],
    ),
    (
        "fingers_right",
        &[
            "thumb_right",
            "index_right",
            "middle_right",
            "ring_right",
            "pinky_right",
        ],
    ),
    (
        "fingers",
        &[
            "thumb_left",
            "index_left",
            "middle_left",
            "ring_left",
            "pinky_left",
            "thumb_right",
            "index_right",
            "middle_right",
            "ring_right",
            "pinky_right",
        ],
    ),
];

const ACTIONS: &[&str] = &[
    "flex",
    "extend",
    "abduct",
    "adduct",
    "rotate-in",
    "rotate-out",
    "twist-left",
    "twist-right",
    "supinate",
    "pronate",
    "dorsiflex",
    "plantarflex",
    "hinge",
];

/// Max degrees per joint type and action (posecode's ROM, Apache-2.0).
const ROM: &[(&str, &[(&str, f64)])] = &[
    (
        "shoulder",
        &[
            ("flex", 180.0),
            ("extend", 60.0),
            ("abduct", 180.0),
            ("adduct", 50.0),
            ("rotate-in", 70.0),
            ("rotate-out", 90.0),
        ],
    ),
    (
        "elbow",
        &[
            ("flex", 154.0),
            ("extend", 10.0),
            ("supinate", 92.0),
            ("pronate", 84.0),
        ],
    ),
    (
        "wrist",
        &[
            ("flex", 80.0),
            ("extend", 70.0),
            ("abduct", 20.0),
            ("adduct", 30.0),
        ],
    ),
    (
        "hip",
        &[
            ("flex", 135.0),
            ("extend", 20.0),
            ("abduct", 45.0),
            ("adduct", 30.0),
            ("rotate-in", 40.0),
            ("rotate-out", 45.0),
        ],
    ),
    ("knee", &[("flex", 144.0), ("extend", 5.0)]),
    ("ankle", &[("dorsiflex", 15.0), ("plantarflex", 50.0)]),
    ("pelvis", &[("hinge", 120.0)]),
    (
        "spine",
        &[
            ("flex", 90.0),
            ("extend", 30.0),
            ("abduct", 35.0),
            ("adduct", 35.0),
            ("twist-left", 45.0),
            ("twist-right", 45.0),
        ],
    ),
    (
        "chest",
        &[
            ("flex", 30.0),
            ("extend", 20.0),
            ("twist-left", 35.0),
            ("twist-right", 35.0),
        ],
    ),
    (
        "neck",
        &[
            ("flex", 50.0),
            ("extend", 60.0),
            ("abduct", 45.0),
            ("adduct", 45.0),
            ("twist-left", 80.0),
            ("twist-right", 80.0),
        ],
    ),
    (
        "head",
        &[
            ("flex", 25.0),
            ("extend", 25.0),
            ("abduct", 20.0),
            ("adduct", 20.0),
            ("twist-left", 40.0),
            ("twist-right", 40.0),
        ],
    ),
    (
        "thumb",
        &[
            ("flex", 80.0),
            ("extend", 20.0),
            ("abduct", 50.0),
            ("adduct", 30.0),
        ],
    ),
    ("index", &[("flex", 100.0), ("extend", 20.0)]),
    ("middle", &[("flex", 100.0), ("extend", 20.0)]),
    ("ring", &[("flex", 100.0), ("extend", 20.0)]),
    ("pinky", &[("flex", 100.0), ("extend", 20.0)]),
];

const EXPRESSIONS: &[&str] = &[
    "neutral",
    "happy",
    "angry",
    "sad",
    "relaxed",
    "surprised",
    "aa",
    "ih",
    "ou",
    "ee",
    "oh",
    "blink",
    "blinkLeft",
    "blinkRight",
];
const MOODS: &[&str] = &[
    "neutral",
    "tired",
    "happy",
    "sad",
    "angry",
    "confident",
    "shy",
    "bored",
];
const TIMINGS: &[&str] = &["flow", "settle", "drive", "snap", "linear"];
const STARTS: &[&str] = &["standing", "seated"];
const LAYERS: &[&str] = &["full", "upper", "face"];

/// A gesture that passed validation: the normalized, clamped action (same
/// shape as `GestureAction` in gesture.ts) and the clamping warnings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidGesture {
    pub action: Value,
    pub warnings: Vec<String>,
}

fn is_entity(s: &str) -> bool {
    if s == "camera" || s == "none" {
        return true;
    }
    let Some((kind, id)) = s.split_once(':') else {
        return false;
    };
    matches!(kind, "user" | "npc" | "obj" | "seat" | "self")
        && (1..=64).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
}

fn expand(joint: &str) -> Vec<&'static str> {
    if let Some((_, bones)) = GROUPS.iter().find(|(g, _)| *g == joint) {
        return bones.to_vec();
    }
    BONES.iter().copied().filter(|b| *b == joint).collect()
}

fn rom(bone: &str, action: &str) -> Option<f64> {
    let ty = bone
        .strip_suffix("_left")
        .or_else(|| bone.strip_suffix("_right"))
        .unwrap_or(bone);
    ROM.iter()
        .find(|(t, _)| *t == ty)
        .and_then(|(_, list)| list.iter().find(|(a, _)| *a == action))
        .map(|(_, m)| *m)
}

struct Check {
    errors: Vec<String>,
    warnings: Vec<String>,
}

impl Check {
    fn err(&mut self, path: &str, msg: impl AsRef<str>) {
        self.errors.push(format!("{path}: {}", msg.as_ref()));
    }

    fn num(&mut self, v: Option<&Value>, path: &str, lo: f64, hi: f64, dflt: f64) -> f64 {
        match v {
            None => dflt,
            Some(Value::Number(n)) => {
                let x = n.as_f64().unwrap_or(dflt);
                if x < lo || x > hi {
                    self.warnings
                        .push(format!("{path}: {x} clamped to [{lo}, {hi}]"));
                    x.clamp(lo, hi)
                } else {
                    x
                }
            }
            Some(_) => {
                self.err(path, "must be a number");
                dflt
            }
        }
    }

    fn one_of(&mut self, v: Option<&Value>, path: &str, list: &[&str], dflt: &str) -> String {
        match v {
            None => dflt.to_string(),
            Some(Value::String(s)) if list.contains(&s.as_str()) => s.clone(),
            Some(other) => {
                self.err(
                    path,
                    format!("unknown value {other}; expected one of {}", list.join(", ")),
                );
                dflt.to_string()
            }
        }
    }

    fn keys(&mut self, o: &Map<String, Value>, path: &str, allowed: &[&str]) {
        for k in o.keys() {
            if !allowed.contains(&k.as_str()) {
                self.err(path, format!("unknown field \"{k}\""));
            }
        }
    }

    fn target(&mut self, v: Option<&Value>, path: &str) -> Option<String> {
        match v {
            None => None,
            Some(Value::String(s)) if is_entity(s) => Some(s.clone()),
            Some(_) => {
                self.err(path, "must be \"user:<id>\", \"npc:<id>\", \"obj:<id>\", \"seat:<id>\", \"self:<id>\", \"camera\" or \"none\"");
                None
            }
        }
    }

    fn joints(&mut self, raw: &Value, path: &str) -> Value {
        let mut out = Map::new();
        let Some(obj) = raw.as_object() else {
            self.err(path, "must be an object of joint → { action: degrees }");
            return Value::Object(out);
        };
        for (joint, spec) in obj {
            let p = format!("{path}.{joint}");
            let bones = expand(joint);
            if bones.is_empty() {
                self.err(&p, "unknown joint");
                continue;
            }
            if spec.as_str() == Some("neutral") {
                for b in bones {
                    out.insert(b.to_string(), json!("neutral"));
                }
                continue;
            }
            let Some(spec) = spec.as_object() else {
                self.err(&p, "must be { action: degrees } or \"neutral\"");
                continue;
            };
            for b in bones {
                let mut acts = match out.get(b) {
                    Some(Value::Object(m)) => m.clone(),
                    _ => Map::new(),
                };
                for (action, deg) in spec {
                    let ap = format!("{p}.{action}");
                    if !ACTIONS.contains(&action.as_str()) {
                        self.err(
                            &ap,
                            format!("unknown action; expected one of {}", ACTIONS.join(", ")),
                        );
                        continue;
                    }
                    let Some(max) = rom(b, action) else {
                        self.err(&ap, format!("\"{action}\" is not allowed on {b}"));
                        continue;
                    };
                    acts.insert(
                        action.clone(),
                        json!(self.num(Some(deg), &ap, 0.0, max, 0.0)),
                    );
                }
                out.insert(b.to_string(), Value::Object(acts));
            }
        }
        Value::Object(out)
    }

    fn spec(&mut self, raw: &Value) -> Option<Value> {
        let Some(g) = raw.as_object() else {
            self.err("gesture", "must be an object");
            return None;
        };
        match g.get("mode").and_then(Value::as_str) {
            Some("clip") => {
                self.keys(g, "gesture", &["mode", "clip", "params", "target"]);
                let clip = g.get("clip").and_then(Value::as_str).unwrap_or("");
                if !GESTURE_CLIPS.contains(&clip) {
                    self.err(
                        "gesture.clip",
                        format!(
                            "unknown clip \"{clip}\"; the catalog is: {}",
                            GESTURE_CLIPS.join(", ")
                        ),
                    );
                    return None;
                }
                let empty = Map::new();
                let params = match g.get("params") {
                    None => &empty,
                    Some(Value::Object(m)) => m,
                    Some(_) => {
                        self.err("gesture.params", "must be an object");
                        return None;
                    }
                };
                self.keys(
                    params,
                    "gesture.params",
                    &["side", "intensity", "speed", "loops"],
                );
                let side = self.one_of(
                    params.get("side"),
                    "gesture.params.side",
                    &["left", "right"],
                    "right",
                );
                let intensity = self.num(
                    params.get("intensity"),
                    "gesture.params.intensity",
                    0.0,
                    1.0,
                    0.8,
                );
                let speed = self.num(params.get("speed"), "gesture.params.speed", 0.25, 3.0, 1.0);
                let loops = self
                    .num(params.get("loops"), "gesture.params.loops", 1.0, 10.0, 2.0)
                    .round();
                let mut out = json!({
                    "mode": "clip",
                    "clip": clip,
                    "params": { "side": side, "intensity": intensity, "speed": speed, "loops": loops },
                });
                if let Some(t) = self.target(g.get("target"), "gesture.target") {
                    out["target"] = json!(t);
                }
                Some(out)
            }
            Some("compose") => {
                self.keys(
                    g,
                    "gesture",
                    &[
                        "mode", "name", "start", "phases", "joints", "dur_s", "timing", "hold",
                        "target",
                    ],
                );
                let phases_raw: Vec<Value> = if let Some(p) = g.get("phases") {
                    match p.as_array() {
                        Some(a) if !a.is_empty() => {
                            if a.len() > 16 {
                                self.err("gesture.phases", "at most 16 phases");
                            }
                            a.iter().take(16).cloned().collect()
                        }
                        _ => {
                            self.err("gesture.phases", "must be a non-empty array");
                            return None;
                        }
                    }
                } else if let Some(j) = g.get("joints") {
                    let mut ph = json!({ "joints": j });
                    if let Some(d) = g.get("dur_s") {
                        ph["dur_s"] = d.clone();
                    }
                    if let Some(t) = g.get("timing") {
                        ph["timing"] = t.clone();
                    }
                    vec![ph]
                } else {
                    self.err("gesture", "compose needs \"phases\" or \"joints\"");
                    return None;
                };
                let mut phases = Vec::new();
                for (i, ph) in phases_raw.iter().enumerate() {
                    let p = format!("gesture.phases[{i}]");
                    let Some(o) = ph.as_object() else {
                        self.err(&p, "must be an object");
                        continue;
                    };
                    self.keys(o, &p, &["dur_s", "timing", "joints", "face", "contacts"]);
                    if o.contains_key("contacts") {
                        self.warnings.push(format!(
                            "{p}.contacts: ignored (contacts are resolved by the world)"
                        ));
                    }
                    let dur = self.num(o.get("dur_s"), &format!("{p}.dur_s"), 0.05, 10.0, 0.6);
                    let timing =
                        self.one_of(o.get("timing"), &format!("{p}.timing"), TIMINGS, "flow");
                    let joints = match o.get("joints") {
                        Some(j) => self.joints(j, &format!("{p}.joints")),
                        None => json!({}),
                    };
                    let mut out = json!({ "dur_s": dur, "timing": timing, "joints": joints });
                    if let Some(face) = o.get("face") {
                        match face.as_object() {
                            Some(f) => {
                                let mut fm = Map::new();
                                for (k, v) in f {
                                    let e = self.one_of(
                                        Some(&json!(k)),
                                        &format!("{p}.face.{k}"),
                                        EXPRESSIONS,
                                        "neutral",
                                    );
                                    fm.insert(
                                        e,
                                        json!(self.num(
                                            Some(v),
                                            &format!("{p}.face.{k}"),
                                            0.0,
                                            1.0,
                                            0.0
                                        )),
                                    );
                                }
                                out["face"] = Value::Object(fm);
                            }
                            None => {
                                self.err(&format!("{p}.face"), "must be { expression: weight }")
                            }
                        }
                    }
                    phases.push(out);
                }
                let name: String = g
                    .get("name")
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| v.to_string())
                    })
                    .unwrap_or_else(|| "composed".into())
                    .chars()
                    .take(64)
                    .collect();
                let start = self.one_of(g.get("start"), "gesture.start", STARTS, "standing");
                let mut out = json!({
                    "mode": "compose",
                    "name": name,
                    "start": start,
                    "phases": phases,
                    "hold": g.get("hold") == Some(&Value::Bool(true)),
                });
                if let Some(t) = self.target(g.get("target"), "gesture.target") {
                    out["target"] = json!(t);
                }
                Some(out)
            }
            other => {
                self.err(
                    "gesture.mode",
                    format!("must be \"clip\" or \"compose\", got {other:?}"),
                );
                None
            }
        }
    }
}

/// Validates an avatar action from an LLM (the same rules as
/// `validateGesture` in gesture.ts). Accepts the full envelope
/// (`{v, actor, gesture, face, look_at, posture, blend}`) or a bare gesture
/// (`{mode, ...}`). Unknown words are errors; angles are clamped to the
/// joint's range and reported as warnings.
pub fn validate_gesture(input: &Value) -> Result<ValidGesture, Vec<String>> {
    let mut c = Check {
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    let env: Map<String, Value> = match input.as_object() {
        Some(o) if o.contains_key("mode") => {
            let mut m = Map::new();
            m.insert("gesture".into(), input.clone());
            m
        }
        Some(o) => o.clone(),
        None => return Err(vec!["the action must be a JSON object".into()]),
    };
    c.keys(
        &env,
        "action",
        &[
            "v", "actor", "gesture", "face", "look_at", "posture", "blend",
        ],
    );
    if let Some(v) = env.get("v") {
        if v != &json!(1) {
            c.err("v", "only version 1 is supported");
        }
    }
    let mut action = json!({ "v": 1, "blend": { "in_s": 0.25, "out_s": 0.4, "layer": "full" } });
    if let Some(a) = env.get("actor") {
        match a.as_str() {
            Some(s) if is_entity(s) => action["actor"] = json!(s),
            _ => c.err("actor", "must be an entity like \"npc:<id>\""),
        }
    }
    let mut any = false;
    if let Some(g) = env.get("gesture") {
        if let Some(spec) = c.spec(g) {
            action["gesture"] = spec;
            any = true;
        }
    }
    if let Some(f) = env.get("face") {
        match f.as_object() {
            Some(o) => {
                c.keys(o, "face", &["expression", "weight", "hold_s"]);
                let expression = c.one_of(
                    o.get("expression"),
                    "face.expression",
                    EXPRESSIONS,
                    "neutral",
                );
                let weight = c.num(o.get("weight"), "face.weight", 0.0, 1.0, 0.7);
                let hold = c.num(o.get("hold_s"), "face.hold_s", 0.0, 30.0, 2.0);
                action["face"] =
                    json!({ "expression": expression, "weight": weight, "hold_s": hold });
                any = true;
            }
            None => c.err("face", "must be an object"),
        }
    }
    if let Some(t) = c.target(env.get("look_at"), "look_at") {
        action["look_at"] = json!(t);
        any = true;
    }
    if let Some(p) = env.get("posture") {
        match p.as_object() {
            Some(o) => {
                c.keys(o, "posture", &["mood", "weight"]);
                let mood = c.one_of(o.get("mood"), "posture.mood", MOODS, "neutral");
                let weight = c.num(o.get("weight"), "posture.weight", 0.0, 1.0, 0.6);
                action["posture"] = json!({ "mood": mood, "weight": weight });
                any = true;
            }
            None => c.err("posture", "must be an object"),
        }
    }
    if let Some(b) = env.get("blend") {
        match b.as_object() {
            Some(o) => {
                c.keys(o, "blend", &["in_s", "out_s", "layer"]);
                let i = c.num(o.get("in_s"), "blend.in_s", 0.0, 3.0, 0.25);
                let out = c.num(o.get("out_s"), "blend.out_s", 0.0, 3.0, 0.4);
                let layer = c.one_of(o.get("layer"), "blend.layer", LAYERS, "full");
                action["blend"] = json!({ "in_s": i, "out_s": out, "layer": layer });
            }
            None => c.err("blend", "must be an object"),
        }
    }
    if !any && c.errors.is_empty() {
        c.err(
            "action",
            "needs at least one of gesture, face, look_at or posture",
        );
    }
    if c.errors.is_empty() {
        Ok(ValidGesture {
            action,
            warnings: c.warnings,
        })
    } else {
        Err(c.errors)
    }
}

/// `validate_gesture` over JSON text; a ```json fence around it is fine.
pub fn validate_gesture_str(text: &str) -> Result<ValidGesture, Vec<String>> {
    let t = text.trim();
    let body = if let Some(start) = t.find("```") {
        let rest = &t[start + 3..];
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        rest.split("```").next().unwrap_or(rest)
    } else {
        match (t.find('{'), t.rfind('}')) {
            (Some(a), Some(b)) if b > a => &t[a..=b],
            _ => t,
        }
    };
    let v: Value =
        serde_json::from_str(body.trim()).map_err(|e| vec![format!("not valid JSON: {e}")])?;
    validate_gesture(&v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_and_compose_validate_and_clamp() {
        let ok = validate_gesture_str(
            r#"```json
            {"v":1,"actor":"npc:padeiro-17","gesture":{"mode":"clip","clip":"wave","params":{"side":"left","intensity":1.4}},
             "posture":{"mood":"tired","weight":0.5}}
            ```"#,
        )
        .unwrap();
        assert_eq!(ok.action["gesture"]["params"]["intensity"], json!(1.0));
        assert_eq!(ok.warnings.len(), 1);

        let c = validate_gesture(
            &json!({"mode":"compose","joints":{"knees":{"flex":200},"neck":"neutral"}}),
        )
        .unwrap();
        assert_eq!(
            c.action["gesture"]["phases"][0]["joints"]["knee_left"]["flex"],
            json!(144.0)
        );
        assert_eq!(
            c.action["gesture"]["phases"][0]["joints"]["neck"],
            json!("neutral")
        );
    }

    #[test]
    fn unknown_words_are_errors() {
        let e = validate_gesture(
            &json!({"mode":"compose","joints":{"elbow_left":{"abduct":20},"tail":{"flex":1}}}),
        )
        .unwrap_err();
        assert_eq!(e.len(), 2);
        assert!(validate_gesture(&json!({"mode":"clip","clip":"moonwalk"})).is_err());
        assert!(validate_gesture(&json!({"look_at":"somewhere"})).is_err());
    }

    #[test]
    fn crop_and_decode_are_consistent() {
        let rgb = vec![255u8; 4 * 2 * 3];
        let t = crop_to_tensor(&rgb, 4, 2, Roi::full(4, 2), 4, false);
        assert_eq!(t.len(), 48);
        // Top row of the square crop is outside a 4×2 frame, so it is black.
        assert_eq!(t[0], 0.0);
        let lm = vec![128.0f32; 195];
        let world = vec![0.1f32; 117];
        let (body, roi) = decode_pose(&lm, &world, Roi::full(640, 480), 256.0, 640, 480);
        assert_eq!(body.world.len(), 99);
        assert_eq!(body.visibility.len(), 33);
        assert_eq!(roi, Roi::full(640, 480));
    }
}
