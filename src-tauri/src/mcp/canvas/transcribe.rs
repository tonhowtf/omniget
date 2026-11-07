use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Mutex, OnceLock};

use omniget_core::core::canvas::transcript::{self as tx, AudioFormat};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use super::{need, num, ready, st, ws_floor};

type Fut<'a> = Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 'a>>;

fn sub<'a>(app: &'a AppHandle, name: &'a str, a: Value) -> Fut<'a> {
    Box::pin(super::dispatch(app, name, a))
}

pub const EVENT: &str = "canvas://transcribe";

#[derive(Clone, Default)]
struct Job {
    stage: String,
    progress: f32,
    error: Option<String>,
    files: Value,
    nodes: Value,
    started_ms: i64,
    finished_ms: Option<i64>,
}

fn jobs() -> &'static Mutex<HashMap<String, Job>> {
    static J: OnceLock<Mutex<HashMap<String, Job>>> = OnceLock::new();
    J.get_or_init(Default::default)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn set(app: &AppHandle, id: &str, f: impl FnOnce(&mut Job)) {
    let snapshot = {
        let Ok(mut g) = jobs().lock() else { return };
        let j = g.entry(id.to_string()).or_default();
        f(j);
        job_json(id, j)
    };
    let _ = app.emit(EVENT, snapshot);
}

fn job_json(id: &str, j: &Job) -> Value {
    let context_ready = j.files["context"]
        .as_str()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|t| !t.contains("_The context pack"));
    json!({
        "contextReady": context_ready,
        "jobId": id,
        "stage": j.stage,
        "progress": (j.progress * 100.0).round() / 100.0,
        "error": j.error,
        "files": j.files,
        "nodes": j.nodes,
        "elapsedMs": j.finished_ms.unwrap_or_else(now_ms) - j.started_ms,
        "done": j.finished_ms.is_some(),
    })
}

fn node_id(v: &Value) -> Option<String> {
    v.get("node")
        .and_then(|n| n.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn source_path(a: &Value) -> Result<PathBuf, String> {
    let p = PathBuf::from(need(a, "path")?);
    if !p.is_absolute() || !p.is_file() {
        return Err(format!("CANVAS_NOT_FOUND: {} is not a file", p.display()));
    }
    if !tx::is_media(&p) {
        return Err("INVALID_ARGUMENT: path is not a video or audio file".into());
    }
    Ok(p)
}

fn output_dir(s: &omniget_core::core::canvas::store::CanvasStore, ws: &str, src: &Path) -> PathBuf {
    let name = src
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let base = s
        .workspace(ws)
        .ok()
        .and_then(|w| w.working_directory)
        .filter(|d| !d.trim().is_empty())
        .map(|d| PathBuf::from(d).join("transcricoes"))
        .unwrap_or_else(|| {
            src.parent()
                .map(Path::to_path_buf)
                .unwrap_or_default()
                .join("transcricoes")
        });
    let mut dir = base.join(tx::slug(&name));
    let mut n = 2;
    while dir.join("transcricao.md").exists() {
        dir = base.join(format!("{}-{n}", tx::slug(&name)));
        n += 1;
    }
    dir
}

pub async fn call(app: &AppHandle, name: &str, a: &Value) -> Result<Value, String> {
    match name {
        "canvas_media_to_audio" => to_audio(app, a).await,
        "canvas_transcribe" => start(app, a).await,
        "canvas_transcribe_status" => {
            let id = need(a, "jobId")?;
            let g = jobs().lock().map_err(|_| "CANVAS_BUSY")?;
            g.get(&id)
                .map(|j| job_json(&id, j))
                .ok_or_else(|| format!("CANVAS_NOT_FOUND: transcription job {id}"))
        }
        _ => Err("TOOL_NOT_AUTHORIZED".into()),
    }
}

async fn to_audio(app: &AppHandle, a: &Value) -> Result<Value, String> {
    let s = ready(app);
    let src = source_path(a)?;
    let format = st(a, "format")
        .map(|f| AudioFormat::parse(&f).ok_or("INVALID_ARGUMENT: format is m4a, mp3 or wav"))
        .transpose()?
        .unwrap_or(AudioFormat::M4a);
    let dir = match st(a, "outDir") {
        Some(d) => PathBuf::from(d),
        None => src.parent().map(Path::to_path_buf).unwrap_or_default(),
    };
    let dst = tx::audio_target(&src, &dir, format);
    let started = now_ms();
    let out = tx::extract_audio(&src, &dst, format).await?;
    let bytes = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    let mut node = Value::Null;
    if a.get("place").and_then(Value::as_bool).unwrap_or(true) && a.get("workspaceId").is_some() {
        let (ws, _) = ws_floor(&s, a)?;
        let mut args = json!({ "workspaceId": ws, "kind": "file", "path": out.to_string_lossy() });
        if let (Some(x), Some(y)) = (num(a, "x"), num(a, "y")) {
            args["x"] = json!(x);
            args["y"] = json!(y);
        }
        node = sub(app, "canvas_node_spawn", args)
            .await?
            .get("node")
            .cloned()
            .unwrap_or(Value::Null);
    }
    Ok(
        json!({ "audio": out.to_string_lossy(), "bytes": bytes, "ms": now_ms() - started, "node": node }),
    )
}

async fn start(app: &AppHandle, a: &Value) -> Result<Value, String> {
    let s = ready(app);
    let (ws, _) = ws_floor(&s, a)?;
    let src = source_path(a)?;
    let engine = st(a, "engine").unwrap_or_else(|| "local".into());
    if !matches!(engine.as_str(), "local" | "provider") {
        return Err("INVALID_ARGUMENT: engine is local or provider".into());
    }
    let summarize = st(a, "summarize").unwrap_or_else(|| "agent".into());
    if !matches!(summarize.as_str(), "agent" | "none") {
        return Err("INVALID_ARGUMENT: summarize is agent or none".into());
    }
    let agent = st(a, "agent").unwrap_or_else(|| "claude".into());
    let lang = st(a, "lang");
    let model = st(a, "model");
    let dir = output_dir(&s, &ws, &src);
    let title = st(a, "title").unwrap_or_else(|| {
        src.file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Recording".into())
    });
    std::fs::create_dir_all(&dir).map_err(|e| format!("TRANSCRIPT_WRITE: {e}"))?;
    let meta = tx::Meta {
        title: &title,
        source: &src,
        duration_ms: None,
        engine: &engine,
    };
    let transcript_path = dir.join("transcricao.md");
    let context_path = dir.join("contexto.md");
    std::fs::write(
        &transcript_path,
        format!("# {title}\n\n_Transcribing..._\n"),
    )
    .map_err(|e| format!("TRANSCRIPT_WRITE: {e}"))?;
    std::fs::write(&context_path, tx::context_placeholder(&meta))
        .map_err(|e| format!("TRANSCRIPT_WRITE: {e}"))?;

    let (x0, y0) = match (num(a, "x"), num(a, "y")) {
        (Some(x), Some(y)) => (x, y),
        _ => {
            let floor = ws_floor(&s, a)?.1;
            let p = s.read(&ws, &floor, |d| {
                omniget_core::core::canvas::layout::free_spot_in(d, 2500.0, 1200.0, None, &[])
            })?;
            (p.x, p.y + 110.0)
        }
    };
    let id = format!("tx-{}", &super::new_id().replace('-', "")[..10]);
    set(app, &id, |j| {
        j.stage = "placing".into();
        j.started_ms = now_ms();
        j.files = json!({ "dir": dir.to_string_lossy(), "transcript": transcript_path.to_string_lossy(), "context": context_path.to_string_lossy() });
    });
    let wsv = json!(ws);
    sub(
        app,
        "canvas_node_spawn",
        json!({
            "workspaceId": wsv, "kind": "text", "text": format!("Transcript: {title}"),
            "fontSize": 36, "fontWeight": "bold", "x": x0, "y": y0 - 100.0,
            "data": { "transcription": { "jobId": id } }
        }),
    )
    .await?;
    let video = sub(app, "canvas_node_spawn", json!({
        "workspaceId": wsv, "kind": "file", "path": src.to_string_lossy(), "x": x0, "y": y0, "w": 420, "h": 120
    })).await?;
    let tnote = sub(app, "canvas_node_spawn", json!({
        "workspaceId": wsv, "kind": "note", "title": "Transcript", "path": transcript_path.to_string_lossy(),
        "x": x0 + 480.0, "y": y0, "w": 760, "h": 560
    })).await?;
    let cnote = sub(app, "canvas_node_spawn", json!({
        "workspaceId": wsv, "kind": "note", "title": "LLM context", "path": context_path.to_string_lossy(),
        "x": x0 + 1300.0, "y": y0, "w": 760, "h": 560
    })).await?;
    let nodes = json!({ "source": node_id(&video), "transcript": node_id(&tnote), "context": node_id(&cnote) });
    set(app, &id, |j| j.nodes = nodes.clone());

    let app2 = app.clone();
    let id2 = id.clone();
    let folder = dir.to_string_lossy().into_owned();
    tauri::async_runtime::spawn(async move {
        let result = run(
            &app2,
            &id2,
            &ws,
            &src,
            &dir,
            &title,
            &engine,
            lang,
            model,
            &summarize,
            &agent,
            (x0, y0),
        )
        .await;
        set(&app2, &id2, |j| {
            match result {
                Ok(()) => {
                    j.stage = "done".into();
                    j.progress = 1.0;
                }
                Err(e) => {
                    j.stage = "failed".into();
                    j.error = Some(e);
                }
            }
            j.finished_ms = Some(now_ms());
        });
    });
    Ok(json!({
        "jobId": id,
        "folder": folder,
        "nodes": nodes,
        "next": "canvas_transcribe_status {jobId} shows the stage; the transcript note fills in when it is done, then the summarizer agent writes contexto.md",
    }))
}

#[allow(clippy::too_many_arguments)]
async fn run(
    app: &AppHandle,
    id: &str,
    ws: &str,
    src: &Path,
    dir: &Path,
    title: &str,
    engine: &str,
    lang: Option<String>,
    model: Option<String>,
    summarize: &str,
    agent: &str,
    origin: (f64, f64),
) -> Result<(), String> {
    use omniget_core::core::asr;
    let duration = tx::duration_ms(src).await;
    let transcript = if engine == "provider" {
        set(app, id, |j| j.stage = "audio".into());
        let audio = tx::audio_target(src, dir, AudioFormat::Mp3);
        tx::extract_audio(src, &audio, AudioFormat::Mp3).await?;
        set(app, id, |j| j.stage = "transcribing".into());
        let text = omniget_core::core::ai::transcribe(&audio).await?;
        asr::Transcript {
            lang: lang.clone().unwrap_or_else(|| "?".into()),
            segments: vec![asr::Segment {
                start_ms: 0,
                end_ms: duration.unwrap_or(0) as i64,
                text: text.clone(),
            }],
            text,
            model: "provider".into(),
        }
    } else {
        if !asr::is_ready(model.as_deref()) {
            let st = asr::status();
            if !st.runtime_installed && !st.runtime_downloadable {
                return Err("ASR_UNAVAILABLE: local transcription needs the ONNX Runtime, which has no build for this system; use engine provider".into());
            }
            set(app, id, |j| j.stage = "model".into());
            asr::install(
                model.as_deref(),
                &omniget_core::core::tools::noop_progress(),
            )
            .await
            .map_err(|e| {
                format!("ASR_UNAVAILABLE: could not download the local Whisper model: {e}")
            })?;
        }
        set(app, id, |j| j.stage = "transcribing".into());
        let app2 = app.clone();
        let id2 = id.to_string();
        asr::transcribe_input(
            src.as_os_str(),
            duration,
            lang.as_deref(),
            model.as_deref(),
            move |p| {
                set(&app2, &id2, |j| j.progress = p);
            },
        )
        .await
        .map_err(|e| format!("TRANSCRIBE_FAILED: {e}"))?
    };
    set(app, id, |j| j.stage = "writing".into());
    let meta = tx::Meta {
        title,
        source: src,
        duration_ms: duration,
        engine,
    };
    let written = tx::write_all(dir, &transcript, &meta)?;
    set(app, id, |j| {
        j.files["parts"] = json!(written
            .parts
            .iter()
            .map(|p| p.to_string_lossy())
            .collect::<Vec<_>>());
        j.files["srt"] = json!(dir.join("transcricao.srt").to_string_lossy());
        j.files["words"] = json!(transcript.text.split_whitespace().count());
    });
    if summarize == "agent" {
        set(app, id, |j| j.stage = "summarizing".into());
        let prompt = tx::summarizer_prompt(dir, written.parts.len(), title);
        let cwd = ready(app)
            .workspace(ws)
            .ok()
            .and_then(|w| w.working_directory)
            .filter(|d| !d.trim().is_empty())
            .unwrap_or_else(|| dir.to_string_lossy().into_owned());
        let spawned = sub(app, "canvas_node_spawn", json!({
            "workspaceId": ws, "kind": "terminal", "agent": agent, "title": "Resumidor", "cwd": cwd,
            "prompt": prompt, "x": origin.0 + 480.0, "y": origin.1 + 620.0, "w": 1580, "h": 460
        }))
        .await?;
        if let (Some(agent_id), Some(ctx)) = (
            node_id(&spawned),
            jobs().lock().ok().and_then(|g| {
                g.get(id)
                    .and_then(|j| j.nodes["context"].as_str().map(str::to_string))
            }),
        ) {
            let _ = sub(
                app,
                "canvas_cord_create",
                json!({ "workspaceId": ws, "from": agent_id, "to": ctx, "kind": "note" }),
            )
            .await;
            set(app, id, |j| j.nodes["summarizer"] = json!(agent_id));
        }
    }
    Ok(())
}
