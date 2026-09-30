//! The Tripo provider against a local fake of the OpenAPI v2 (shapes from
//! the official tripo-python-sdk): multipart upload -> `image_token`, task
//! with `file_token`, polling `queued` -> `running` -> `success`, download
//! of `output.model` without the key, then the normalizer.

use omniget_asset3d::providers::tripo::Tripo;
use omniget_asset3d::providers::{GenerateRequest, Generator};
use omniget_asset3d::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const KEY: &str = "tsk_fake_key";
const TOKEN: &str = "0d8a7f3e-2b61-4c1e-9a55-7f0b2d4e8c11";
const TASK: &str = "1ec04ced-4b87-44f6-a296-beee80777941";

#[derive(Default)]
struct Seen {
    uploads: usize,
    task_body: Option<serde_json::Value>,
    polls: usize,
    downloads: usize,
    download_had_auth: bool,
}

/// 2 × 3 × 2 m textured box, the way Tripo ships one: a single mesh with
/// POSITION/NORMAL/TEXCOORD_0, indices and an embedded PNG base colour.
fn tripo_like_glb() -> Vec<u8> {
    let (w, h, d) = (1.0f32, 1.5f32, 1.0f32);
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        (
            [0., 0., 1.],
            [[-w, 0., d], [w, 0., d], [w, 2. * h, d], [-w, 2. * h, d]],
        ),
        (
            [0., 0., -1.],
            [[w, 0., -d], [-w, 0., -d], [-w, 2. * h, -d], [w, 2. * h, -d]],
        ),
        (
            [1., 0., 0.],
            [[w, 0., d], [w, 0., -d], [w, 2. * h, -d], [w, 2. * h, d]],
        ),
        (
            [-1., 0., 0.],
            [[-w, 0., -d], [-w, 0., d], [-w, 2. * h, d], [-w, 2. * h, -d]],
        ),
        (
            [0., 1., 0.],
            [
                [-w, 2. * h, d],
                [w, 2. * h, d],
                [w, 2. * h, -d],
                [-w, 2. * h, -d],
            ],
        ),
        (
            [0., -1., 0.],
            [[-w, 0., -d], [w, 0., -d], [w, 0., d], [-w, 0., d]],
        ),
    ];
    let (mut pos, mut nrm, mut uv, mut idx) = (vec![], vec![], vec![], vec![]);
    for (n, quad) in faces {
        let base = (pos.len() / 12) as u16;
        for (k, p) in quad.iter().enumerate() {
            for f in p {
                pos.extend_from_slice(&f.to_le_bytes());
            }
            for f in n {
                nrm.extend_from_slice(&f.to_le_bytes());
            }
            let t = [[0f32, 1.], [1., 1.], [1., 0.], [0., 0.]][k];
            for f in t {
                uv.extend_from_slice(&f.to_le_bytes());
            }
        }
        for i in [0u16, 1, 2, 0, 2, 3] {
            idx.extend_from_slice(&(base + i).to_le_bytes());
        }
    }
    let img = image::RgbaImage::from_fn(64, 64, |x, y| {
        if (x / 8 + y / 8) % 2 == 0 {
            image::Rgba([196, 120, 80, 255])
        } else {
            image::Rgba([230, 220, 200, 255])
        }
    });
    let mut png = std::io::Cursor::new(vec![]);
    img.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let png = png.into_inner();

    let mut bin = vec![];
    let mut views = vec![];
    for chunk in [&pos, &nrm, &uv, &idx, &png] {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        views.push(
            serde_json::json!({"buffer": 0, "byteOffset": bin.len(), "byteLength": chunk.len()}),
        );
        bin.extend_from_slice(chunk);
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let json = serde_json::json!({
        "asset": {"version": "2.0", "generator": "tripo"},
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": views,
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 24, "type": "VEC3", "min": [-w, 0., -d], "max": [w, 2. * h, d]},
            {"bufferView": 1, "componentType": 5126, "count": 24, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5126, "count": 24, "type": "VEC2"},
            {"bufferView": 3, "componentType": 5123, "count": 36, "type": "SCALAR"}
        ],
        "images": [{"bufferView": 4, "mimeType": "image/png"}],
        "textures": [{"source": 0}],
        "materials": [{"name": "Material_0", "pbrMetallicRoughness": {"baseColorTexture": {"index": 0}, "metallicFactor": 0.0}}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}, "indices": 3, "material": 0}]}],
        "nodes": [{"mesh": 0, "name": "tripo_node"}],
        "scenes": [{"nodes": [0]}],
        "scene": 0
    });
    let mut json = serde_json::to_vec(&json).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = vec![];
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut r = BufReader::new(stream);
    let mut line = String::new();
    r.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut headers = vec![];
    loop {
        let mut h = String::new();
        r.read_line(&mut h).ok()?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let len = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0usize);
    let mut body = vec![0; len];
    r.read_exact(&mut body).ok()?;
    Some(Request {
        method,
        path,
        headers,
        body,
    })
}

fn respond(stream: &mut TcpStream, status: u16, ctype: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn json_reply(stream: &mut TcpStream, status: u16, v: serde_json::Value) {
    respond(
        stream,
        status,
        "application/json",
        &serde_json::to_vec(&v).unwrap(),
    );
}

/// Serves the fake API on a random port; returns its origin.
fn fake_tripo(seen: Arc<Mutex<Seen>>, glb: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let model_url = format!("{origin}/cdn/tripo-data/{TASK}/model.glb?Policy=x&Signature=y");
    let served = origin.clone();
    std::thread::spawn(move || {
        let origin = served;
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let Some(req) = read_request(&mut s) else {
                continue;
            };
            let auth = req
                .headers
                .iter()
                .find(|(k, _)| k == "authorization")
                .map(|(_, v)| v.clone());
            let mut seen = seen.lock().unwrap();
            if req.path.starts_with("/cdn/") {
                seen.downloads += 1;
                seen.download_had_auth = auth.is_some();
                respond(&mut s, 200, "model/gltf-binary", &glb);
                continue;
            }
            if auth.as_deref() != Some(&format!("Bearer {KEY}")) {
                json_reply(
                    &mut s,
                    401,
                    serde_json::json!({"code": 1002, "message": "Authentication failed",
                                       "suggestion": "Check your API key"}),
                );
                continue;
            }
            match (req.method.as_str(), req.path.as_str()) {
                ("GET", "/v2/openapi/user/balance") => json_reply(
                    &mut s,
                    200,
                    serde_json::json!({"code": 0, "data": {"balance": 2480.0, "frozen": 30.0}}),
                ),
                ("POST", "/v2/openapi/upload") => {
                    let ctype = req
                        .headers
                        .iter()
                        .find(|(k, _)| k == "content-type")
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default();
                    let body = String::from_utf8_lossy(&req.body);
                    let ok = ctype.starts_with("multipart/form-data")
                        && body.contains("name=\"file\"")
                        && body.contains("filename=\"image.png\"")
                        && req.body.windows(4).any(|w| w == [0x89, b'P', b'N', b'G']);
                    seen.uploads += 1;
                    if ok {
                        json_reply(
                            &mut s,
                            200,
                            serde_json::json!({"code": 0, "data": {"image_token": TOKEN}}),
                        );
                    } else {
                        json_reply(
                            &mut s,
                            400,
                            serde_json::json!({"code": 2002, "message": "bad upload"}),
                        );
                    }
                }
                ("POST", "/v2/openapi/task") => {
                    seen.task_body = serde_json::from_slice(&req.body).ok();
                    json_reply(
                        &mut s,
                        200,
                        serde_json::json!({"code": 0, "data": {"task_id": TASK}}),
                    );
                }
                ("GET", p) if p == format!("/v2/openapi/task/{TASK}") => {
                    seen.polls += 1;
                    let (status, progress, output) = match seen.polls {
                        1 => ("queued", 0, serde_json::json!({})),
                        2 => ("running", 55, serde_json::json!({})),
                        _ => (
                            "success",
                            100,
                            serde_json::json!({
                                "model": model_url,
                                "rendered_image": format!("{origin}/cdn/tripo-data/{TASK}/preview.webp")
                            }),
                        ),
                    };
                    json_reply(
                        &mut s,
                        200,
                        serde_json::json!({"code": 0, "data": {
                            "task_id": TASK, "type": "image_to_model", "status": status,
                            "input": {"face_limit": 10000}, "output": output, "progress": progress,
                            "create_time": 1759160000, "running_left_time": 20, "queuing_num": 0
                        }}),
                    );
                }
                _ => json_reply(
                    &mut s,
                    404,
                    serde_json::json!({"code": 404, "message": "not found"}),
                ),
            }
        }
    });
    origin
}

fn client(origin: &str, key: &str) -> Tripo {
    let mut t = Tripo::new(key);
    t.base = format!("{origin}/v2/openapi");
    t.poll_every = Duration::from_millis(20);
    t.timeout = Duration::from_secs(20);
    t
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

#[test]
fn tripo_image_to_model_end_to_end_through_the_normalizer() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let origin = fake_tripo(seen.clone(), tripo_like_glb());
    let tripo = client(&origin, KEY);

    let quote = block_on(tripo.quote()).unwrap();
    assert_eq!(quote.balance, Some(2480.0));

    let mut png = std::io::Cursor::new(vec![]);
    image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let got = block_on(tripo.generate(GenerateRequest {
        prompt: None,
        image: Some(png.into_inner()),
        face_limit: Some(10_000),
    }))
    .unwrap();
    assert_eq!(got.task_id, TASK);
    assert_eq!(got.provider, "tripo");

    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.uploads, 1);
        assert_eq!(seen.polls, 3, "queued, running, success");
        assert_eq!(seen.downloads, 1);
        assert!(!seen.download_had_auth, "the key must not go to the CDN");
        let body = seen.task_body.as_ref().unwrap();
        assert_eq!(body["type"], "image_to_model");
        assert_eq!(body["file"]["type"], "png");
        assert_eq!(body["file"]["file_token"], TOKEN);
        assert_eq!(body["face_limit"], 10_000);
        assert_eq!(body["texture"], true);
    }

    let mut opts = NormalizeOptions::new("tripo-house", AssetClass::Building);
    opts.provenance = Some(format!("tripo task {}", got.task_id));
    opts.license = Some(License {
        kind: LicenseKind::Own,
        author: None,
        url: None,
    });
    let a = normalize(Input::bytes(got.glb, "tripo.glb"), opts).unwrap();
    assert_eq!(a.class, AssetClass::Building);
    assert_eq!(a.report.output.triangles, 12);
    assert!(a.footprint.w_tiles >= 1 && a.footprint.d_tiles >= 1);
    assert!(a.footprint.size_mm[1] > 0);
    validate_glb(&a.glb, 1 << 20, 100).unwrap();
}

#[test]
fn tripo_text_task_and_bad_key() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let origin = fake_tripo(seen.clone(), tripo_like_glb());

    let e = block_on(client(&origin, "wrong").generate(GenerateRequest {
        prompt: Some("a small bakery".into()),
        image: None,
        face_limit: None,
    }))
    .unwrap_err();
    assert!(e.contains("Authentication failed"), "{e}");

    let got = block_on(client(&origin, KEY).generate(GenerateRequest {
        prompt: Some("a small bakery".into()),
        image: None,
        face_limit: None,
    }))
    .unwrap();
    assert!(got.glb.starts_with(b"glTF"));
    let seen = seen.lock().unwrap();
    let body = seen.task_body.as_ref().unwrap();
    assert_eq!(body["type"], "text_to_model");
    assert_eq!(body["prompt"], "a small bakery");
    assert_eq!(seen.uploads, 0);
}
