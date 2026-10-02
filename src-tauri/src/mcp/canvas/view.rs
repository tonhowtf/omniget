use std::sync::Mutex;

use omniget_core::core::canvas::model::Camera;
use omniget_core::core::canvas::ops::Op;
use omniget_core::core::canvas::store::CanvasStore;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::{
    apply, arr, b, blocking, def, flag, floor, locate_arg, n, node, num, out, ready, st, ty, who,
    ws, ws_floor, Def,
};

const NAMES: &[&str] = &[
    "canvas_camera_get",
    "canvas_camera_set",
    "canvas_node_focus",
    "canvas_view",
];

const DEFAULT_SCREEN: Screen = Screen {
    x: 0.0,
    y: 0.0,
    w: 1200.0,
    h: 760.0,
};

#[derive(Clone, Copy)]
struct Screen {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

static LAST_SCREEN: Mutex<Option<Screen>> = Mutex::new(None);

fn view_out() -> Value {
    out(
        json!({
            "workspaceId": ty("string"),
            "floorId": ty("string"),
            "camera": ty("object"),
            "visibleRect": ty("object"),
            "source": ty("string"),
            "window": {},
        }),
        &["camera", "workspaceId", "floorId", "source"],
    )
}

pub(super) fn defs() -> Vec<Def> {
    vec![
        def(
            "canvas_camera_get",
            "Camera of a floor. camera.x, camera.y = world point at the top-left of the VISIBLE canvas (side panel, toolbar, minimap and bottom bars excluded); visibleRect {x,y,w,h} = the world area the person actually sees. Always says which workspaceId/floorId it is about. source window = live camera of the window; source saved = the floor's saved camera, used when the window shows another workspace (or is not on the canvas), and then `window` says what the window shows.",
            false,
            json!({"workspaceId": ws(), "floorId": floor()}),
            &[],
            view_out(),
        ),
        def(
            "canvas_camera_set",
            "Move a floor's camera. x, y = world point at the top-left of the visible canvas (panels excluded); zoom 0.1 to 3 (alone it zooms around the visible centre); fit frames everything in the visible area. When workspaceId/floorId is not what the window shows, only that floor's saved camera changes (source saved) and the window is not touched. The answer's visibleRect says what the person sees.",
            true,
            json!({"x": n("World x of the visible top-left"), "y": n("World y of the visible top-left"), "zoom": n("Zoom"), "animate": b("Fly there"), "fit": b("Fit everything"), "workspaceId": ws(), "floorId": floor()}),
            &[],
            view_out(),
        ),
        def(
            "canvas_node_focus",
            "Fly to a node in the canvas window and give it the keyboard (switches workspace/floor when needed). The node is framed in the visible area (not under the side panel, toolbar or minimap); the answer has the camera and visibleRect it lands on.",
            true,
            json!({"nodeId": node()}),
            &["nodeId"],
            out(
                json!({"focus": {}, "workspaceId": ty("string"), "floorId": ty("string"), "camera": ty("object"), "visibleRect": ty("object")}),
                &[],
            ),
        ),
        def(
            "canvas_view",
            "What the person sees of the AI work in the canvas window. activity true opens the Activity panel (who did what, live), false closes it. follow = an origin (mcp:<client>, agent:<node>), an actor name, \"me\" (you), \"any\" (whoever writes) or null (stop): the camera follows that author's changes. presence false hides the AI cursors and change glows (true shows them; on by default). Without arguments it only reports the state.",
            true,
            json!({
                "activity": b("Open (true) or close (false) the Activity panel"),
                "follow": {"type": ["string", "null"], "description": "Origin, actor name, \"me\", \"any\" or null to stop following"},
                "presence": b("Show AI cursors and change glows")
            }),
            &[],
            out(
                json!({"workspaceId": ty("string"), "floorId": ty("string"), "activity": ty("boolean"), "presence": ty("boolean"), "follow": {"type": ["string", "null"]}, "actors": arr(ty("object"))}),
                &["activity", "follow"],
            ),
        ),
    ]
}

pub(super) fn owns(name: &str) -> bool {
    NAMES.contains(&name)
}

fn remember(v: &Value) {
    let r = &v["visibleScreen"];
    let (Some(x), Some(y), Some(w), Some(h)) = (
        r["x"].as_f64(),
        r["y"].as_f64(),
        r["w"].as_f64(),
        r["h"].as_f64(),
    ) else {
        return;
    };
    if w > 0.0 && h > 0.0 {
        *LAST_SCREEN.lock().unwrap_or_else(|e| e.into_inner()) = Some(Screen { x, y, w, h });
    }
}

fn screen() -> (Screen, bool) {
    match *LAST_SCREEN.lock().unwrap_or_else(|e| e.into_inner()) {
        Some(s) => (s, true),
        None => (DEFAULT_SCREEN, false),
    }
}

fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn saved_view(ws: &str, floor: &str, cam: Camera, window: Option<&Value>) -> Value {
    let (sc, known) = screen();
    let z = cam.zoom.max(0.01);
    let vx = cam.x + sc.x / z;
    let vy = cam.y + sc.y / z;
    let mut v = json!({
        "workspaceId": ws,
        "floorId": floor,
        "camera": { "x": r2(vx), "y": r2(vy), "zoom": (cam.zoom * 1000.0).round() / 1000.0 },
        "visibleRect": { "x": r2(vx), "y": r2(vy), "w": r2(sc.w / z), "h": r2(sc.h / z) },
        "source": "saved",
    });
    if !known {
        v["visibleRectEstimated"] = json!(true);
    }
    if let Some(w) = window {
        v["window"] = json!({ "workspaceId": w["workspaceId"], "floorId": w["floorId"] });
        v["note"] = json!(format!(
            "The window shows {}; this is the saved camera of {ws}, used when it is opened.",
            w["workspaceId"].as_str().unwrap_or("another workspace")
        ));
    }
    v
}

fn content_bounds(
    s: &CanvasStore,
    ws: &str,
    floor: &str,
) -> Result<Option<(f64, f64, f64, f64)>, String> {
    s.read(ws, floor, |d| {
        let mut it = d.nodes.iter().filter(|n| n.stack_id.is_none());
        let first = it.next()?;
        let (mut x0, mut y0, mut x1, mut y1) =
            (first.x, first.y, first.x + first.w, first.y + first.h);
        for n in it {
            x0 = x0.min(n.x);
            y0 = y0.min(n.y);
            x1 = x1.max(n.x + n.w);
            y1 = y1.max(n.y + n.h);
        }
        Some((x0, y0, x1 - x0, y1 - y0))
    })
}

fn target_args(s: &CanvasStore, a: &Value) -> Result<(Option<(String, String)>, Value), String> {
    if st(a, "workspaceId").is_none() && st(a, "floorId").is_none() {
        return Ok((None, json!({})));
    }
    let (ws, floor) = ws_floor(s, a)?;
    let mut args = json!({ "workspaceId": ws });
    if st(a, "floorId").is_some() {
        args["floorId"] = json!(floor);
    }
    Ok((Some((ws, floor)), args))
}

fn window_unreachable(e: &str) -> bool {
    e.starts_with("CANVAS_NOT_OPEN") || e.starts_with("UI_")
}

async fn camera_get(app: &AppHandle, s: &CanvasStore, a: &Value) -> Result<Value, String> {
    let (target, args) = target_args(s, a)?;
    let window = match super::super::canvas_ui::ui(app, "camera", args).await {
        Ok(mut v) if v["elsewhere"] != json!(true) => {
            remember(&v);
            v["source"] = json!("window");
            return Ok(v);
        }
        Ok(v) => Some(v),
        Err(e) if window_unreachable(&e) => None,
        Err(e) => return Err(e),
    };
    let (ws, floor) = match target {
        Some(t) => t,
        None => ws_floor(s, a)?,
    };
    let cam = s.read(&ws, &floor, |d| d.camera)?;
    Ok(saved_view(&ws, &floor, cam, window.as_ref()))
}

async fn camera_set(app: &AppHandle, s: &CanvasStore, a: &Value) -> Result<Value, String> {
    let (target, mut args) = target_args(s, a)?;
    let fit = flag(a, "fit") == Some(true);
    if let (Some(src), Some(dst)) = (a.as_object(), args.as_object_mut()) {
        for k in ["x", "y", "zoom", "animate"] {
            if let Some(v) = src.get(k) {
                dst.insert(k.into(), v.clone());
            }
        }
    }
    let op = if fit { "fit" } else { "camera" };
    let window = match super::super::canvas_ui::ui(app, op, args).await {
        Ok(mut v) if v["elsewhere"] != json!(true) => {
            remember(&v);
            v["source"] = json!("window");
            return Ok(v);
        }
        Ok(v) => Some(v),
        Err(e) if window_unreachable(&e) => None,
        Err(e) => return Err(e),
    };
    let (ws, floor) = match target {
        Some(t) => t,
        None => ws_floor(s, a)?,
    };
    let (sc, _) = screen();
    let mut cam = s.read(&ws, &floor, |d| d.camera)?;
    if fit {
        match content_bounds(s, &ws, &floor)? {
            Some((bx, by, bw, bh)) => {
                let m = 64f64.min(sc.w / 10.0).min(sc.h / 10.0);
                let z = ((sc.w - 2.0 * m) / bw.max(1.0))
                    .min((sc.h - 2.0 * m) / bh.max(1.0))
                    .clamp(0.1, 1.0);
                cam.zoom = z;
                cam.x = bx + bw / 2.0 - (sc.x + sc.w / 2.0) / z;
                cam.y = by + bh / 2.0 - (sc.y + sc.h / 2.0) / z;
            }
            None => {
                cam.zoom = 1.0;
                cam.x = -(sc.x + sc.w / 2.0);
                cam.y = -(sc.y + sc.h / 2.0);
            }
        }
    } else {
        let old = cam.zoom.max(0.01);
        let z = num(a, "zoom").map(|z| z.clamp(0.1, 3.0)).unwrap_or(old);
        let (x, y) = (num(a, "x"), num(a, "y"));
        if x.is_none() && y.is_none() {
            let cx = cam.x + (sc.x + sc.w / 2.0) / old;
            let cy = cam.y + (sc.y + sc.h / 2.0) / old;
            cam.x = cx - (sc.x + sc.w / 2.0) / z;
            cam.y = cy - (sc.y + sc.h / 2.0) / z;
        } else {
            let vx = x.unwrap_or(cam.x + sc.x / old);
            let vy = y.unwrap_or(cam.y + sc.y / old);
            cam.x = vx - sc.x / z;
            cam.y = vy - sc.y / z;
        }
        cam.zoom = z;
    }
    let s2 = ready(app);
    let (w2, f2) = (ws.clone(), floor.clone());
    blocking(move || apply(&s2, &w2, &f2, vec![Op::SetCamera { camera: cam }])).await?;
    Ok(saved_view(&ws, &floor, cam, window.as_ref()))
}

async fn node_focus(app: &AppHandle, s: &CanvasStore, a: &Value) -> Result<Value, String> {
    let (ws, floor, n) = locate_arg(s, a, "nodeId")?;
    let mut v = super::super::canvas_ui::ui(
        app,
        "focus",
        json!({ "nodeId": n.id, "workspaceId": ws, "floorId": floor }),
    )
    .await?;
    remember(&v);
    v["source"] = json!("window");
    Ok(v)
}

async fn view(app: &AppHandle, a: &Value) -> Result<Value, String> {
    let mut args = json!({});
    if let Some(v) = a.get("activity").and_then(Value::as_bool) {
        args["activity"] = json!(v);
    }
    if let Some(v) = a.get("presence").and_then(Value::as_bool) {
        args["presence"] = json!(v);
    }
    if let Some(f) = a.get("follow") {
        args["follow"] = match f.as_str().map(str::trim) {
            Some(m) if m.eq_ignore_ascii_case("me") || m.eq_ignore_ascii_case("self") => {
                json!(who::origin())
            }
            Some("") => Value::Null,
            _ => f.clone(),
        };
    }
    super::super::canvas_ui::ui(app, "view", args).await
}

pub(super) async fn call(app: &AppHandle, name: &str, a: &Value) -> Result<Value, String> {
    let s = ready(app);
    match name {
        "canvas_camera_get" => camera_get(app, &s, a).await,
        "canvas_camera_set" => camera_set(app, &s, a).await,
        "canvas_node_focus" => node_focus(app, &s, a).await,
        "canvas_view" => view(app, a).await,
        _ => Err(format!("UNKNOWN_TOOL: {name}")),
    }
}
