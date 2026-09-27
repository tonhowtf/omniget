//! Bounded asynchronous native snapshot recording. No microphone or desktop capture.
use crate::mcp::{
    artifacts,
    policy::{self, Principal},
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use tauri::AppHandle;
struct Job {
    owner: String,
    signal: Arc<AtomicU8>,
    state: Value,
}
static JOBS: OnceLock<Mutex<HashMap<String, Job>>> = OnceLock::new();
fn jobs() -> &'static Mutex<HashMap<String, Job>> {
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn retryable_frame(error: &str) -> bool {
    ["STALE_TARGET", "FRONTEND_NOT_READY", "SESSION_BUSY"]
        .contains(&error.split(':').next().unwrap_or(error))
}
fn update(id: &str, state: Value) {
    if let Ok(mut jobs) = jobs().lock() {
        if let Some(j) = jobs.get_mut(id) {
            j.state = state;
        }
    }
}
pub async fn call(app: &AppHandle, p: &Principal, args: Value) -> Result<Value, String> {
    if !cfg!(target_os = "macos") {
        return Err("CAPABILITY_UNAVAILABLE".into());
    }
    let action = args["action"].as_str().ok_or("INVALID_ARGUMENT")?;
    if action != "start" {
        let id = args["jobId"].as_str().ok_or("INVALID_ARGUMENT")?;
        let mut jobs = jobs().lock().map_err(|_| "SESSION_BUSY")?;
        let job = jobs
            .get_mut(id)
            .filter(|j| j.owner == p.id)
            .ok_or("RECORDING_NOT_FOUND")?;
        match action {
            "stop" => {
                job.signal.store(1, Ordering::SeqCst);
            }
            "cancel" => {
                job.signal.store(2, Ordering::SeqCst);
            }
            "status" => {}
            _ => return Err("INVALID_ARGUMENT".into()),
        }
        return Ok(json!({"schemaVersion":1,"jobId":id,"state":job.state}));
    }
    let root = super::native_agent_media::directory(p)?;
    let ffmpeg = crate::core::dependencies::find_tool("ffmpeg")
        .await
        .ok_or("CAPABILITY_UNAVAILABLE: install FFmpeg")?;
    let window = args["windowId"].as_str().unwrap_or("main").to_string();
    let id = uuid::Uuid::new_v4().to_string();
    let signal = Arc::new(AtomicU8::new(0));
    {
        let mut jobs = jobs().lock().map_err(|_| "SESSION_BUSY")?;
        if jobs
            .values()
            .any(|j| j.state["status"] == "recording" || j.state["status"] == "finalizing")
        {
            return Err("SESSION_BUSY: recording active".into());
        }
        if jobs.len() >= 32 {
            return Err("SESSION_BUSY: recording job limit; restart app".into());
        }
        jobs.insert(
            id.clone(),
            Job {
                owner: p.id.clone(),
                signal: signal.clone(),
                state: json!({"status":"recording","targetFps":5,"frames":0}),
            },
        );
    }
    let app = app.clone();
    let p = p.clone();
    let job_id = id.clone();
    tauri::async_runtime::spawn(async move {
        let partial = root.join(format!("{job_id}-frames"));
        let output = root.join(format!("{job_id}.mp4"));
        let result:Result<Value,String>=async {
            std::fs::create_dir(&partial).map_err(|e|format!("RECORDING_FAILED: {e}"))?;
            let start=Instant::now();let mut timestamps=Vec::new();let mut bytes=0usize;
            let mut dimensions=None;let mut discarded=0u32;
            while start.elapsed()<Duration::from_secs(60) && signal.load(Ordering::SeqCst)==0 {
                policy::active(&p)?;
                let cycle=Instant::now();
                let (png,_) = match super::native_agent_media::pixels(&app,&window).await {
                    Ok(frame) => frame,
                    Err(error) if retryable_frame(&error) => {discarded+=1;tokio::time::sleep(Duration::from_millis(200)).await;continue;}
                    Err(error) => return Err(error),
                };
                let img=image::load_from_memory(&png).map_err(|_|"RECORDING_FAILED: frame")?;
                let dim=(img.width(),img.height());
                if dimensions.is_some_and(|d|d!=dim){return Err("RECORDING_FAILED: window resized".into());}dimensions=Some(dim);
                bytes+=png.len();if bytes>128*1024*1024 {return Err("RECORDING_FAILED: 128 MiB frame limit".into());}
                let n=timestamps.len();timestamps.push(start.elapsed().as_secs_f64());
                std::fs::write(partial.join(format!("{n:04}.png")),png).map_err(|e|format!("RECORDING_FAILED: {e}"))?;
                update(&job_id,json!({"status":"recording","frames":timestamps.len(),"durationSeconds":start.elapsed().as_secs_f64(),"targetFps":5,"discardedUnstableFrames":discarded,"dimensions":dimensions}));
                tokio::time::sleep(Duration::from_millis(200).saturating_sub(cycle.elapsed())).await;
            }
            policy::active(&p)?;
            if signal.load(Ordering::SeqCst)==2 {return Err("CANCELLED".into());}
            if timestamps.len()<2 {return Err("RECORDING_FAILED: fewer than two frames".into());}
            let duration=start.elapsed().as_secs_f64();
            let mut concat=String::from("ffconcat version 1.0\n");
            for (n,t) in timestamps.iter().enumerate(){concat.push_str(&format!("file '{n:04}.png'\nduration {:.6}\n",timestamps.get(n+1).copied().unwrap_or(duration)-t));}
            concat.push_str(&format!("file '{:04}.png'\n",timestamps.len()-1));
            std::fs::write(partial.join("frames.txt"),concat).map_err(|_|"RECORDING_FAILED: concat")?;
            update(&job_id,json!({"status":"finalizing","frames":timestamps.len()}));
            let mut command=tokio::process::Command::new(&ffmpeg);
            command.kill_on_drop(true).args(["-nostdin","-v","error","-f","concat","-safe","1","-i"]).arg(partial.join("frames.txt"))
                .args(["-an","-c:v","libx264","-pix_fmt","yuv420p","-vf","pad=ceil(iw/2)*2:ceil(ih/2)*2","-fps_mode","vfr","-movflags","+faststart"]).arg(&output)
                .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
            let status=tokio::time::timeout(Duration::from_secs(60),command.status()).await.map_err(|_|"TIMEOUT: video encoder")?.map_err(|_|"RECORDING_FAILED: encoder")?;
            if !status.success(){return Err("RECORDING_FAILED: encoder exit".into());}
            let mut decoder=tokio::process::Command::new(&ffmpeg);
            decoder.kill_on_drop(true).args(["-nostdin","-v","error","-xerror","-i"]).arg(&output)
                .args(["-map","0:v:0","-f","null","-"]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
            let decoded=tokio::time::timeout(Duration::from_secs(60),decoder.status()).await.map_err(|_|"TIMEOUT: video validation")?.map_err(|_|"RECORDING_FAILED: video validation")?;
            if !decoded.success(){return Err("RECORDING_FAILED: invalid final video".into());}
            policy::active(&p)?;
            if signal.load(Ordering::SeqCst)==2 {return Err("CANCELLED".into());}
            artifacts::grant_root(&p.id,&root)?;
            let artifact=artifacts::register(&p,&output)?;
            Ok(json!({"status":"complete","artifact":artifact,"durationSeconds":duration,"frames":timestamps.len(),"targetFps":5,"discardedUnstableFrames":discarded,"missedTargetSlots":((duration*5.).floor() as usize).saturating_sub(timestamps.len()),"observedAcquisitionFps":timestamps.len() as f64/duration,"timestamps":timestamps,"dimensions":dimensions,"audio":false,"cursor":false,"validation":"FFmpeg full video decode","encodedDimensions":dimensions.map(|(w,h)|(w.div_ceil(2)*2,h.div_ceil(2)*2)),"source":"native WKWebView snapshots; variable frame rate MP4"}))
        }.await;
        let _ = std::fs::remove_dir_all(partial);
        match result {
            Ok(state) => {
                let _ = std::fs::write(
                    root.join(format!("{job_id}.json")),
                    serde_json::to_vec_pretty(&state).unwrap_or_default(),
                );
                update(&job_id, state);
            }
            Err(error) => {
                let _ = std::fs::remove_file(output);
                update(
                    &job_id,
                    json!({"status":if error=="CANCELLED"{"cancelled"}else{"failed"},"error":error}),
                );
            }
        }
    });
    Ok(
        json!({"schemaVersion":1,"jobId":id,"state":{"status":"recording","targetFps":5,"limitSeconds":60}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_transient_frames_retry_and_security_failures_stop() {
        for error in [
            "STALE_TARGET: transition",
            "FRONTEND_NOT_READY: loading",
            "SESSION_BUSY: snapshot acquisition",
        ] {
            assert!(retryable_frame(error));
        }
        for error in [
            "PERMISSION_REQUIRED: sensitive surface",
            "PRINCIPAL_INACTIVE",
            "WINDOW_NOT_FOUND",
            "CAPTURE_FAILED",
            "TIMEOUT",
        ] {
            assert!(!retryable_frame(error));
        }
    }
}
