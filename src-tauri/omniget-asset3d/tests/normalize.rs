//! Hostile inputs are refused without panicking, 2D imports build, and the
//! normalized hash is deterministic.

use base64::Engine;
use omniget_asset3d::*;

fn gltf_json(v: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&v).unwrap()
}

fn triangle_buffer() -> String {
    let mut b = vec![];
    for f in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        b.extend_from_slice(&f.to_le_bytes());
    }
    format!(
        "data:application/octet-stream;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(b)
    )
}

fn tri_doc(count: u64, uri: &str) -> Vec<u8> {
    gltf_json(serde_json::json!({
        "asset": {"version": "2.0"},
        "buffers": [{"byteLength": 36, "uri": uri}],
        "bufferViews": [{"buffer": 0, "byteLength": 36}],
        "accessors": [{"bufferView": 0, "componentType": 5126, "count": count, "type": "VEC3",
                       "min": [0,0,0], "max": [1,1,0]}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
        "nodes": [{"mesh": 0}],
        "scenes": [{"nodes": [0]}]
    }))
}

fn run(bytes: Vec<u8>, name: &str, class: AssetClass) -> Result<NormalizedAsset, Error> {
    normalize(
        Input::bytes(bytes, name),
        NormalizeOptions::new(name, class),
    )
}

#[test]
fn good_triangle_imports_and_hash_is_stable() {
    let a = run(tri_doc(3, &triangle_buffer()), "t.gltf", AssetClass::Prop).unwrap();
    let b = run(tri_doc(3, &triangle_buffer()), "t.gltf", AssetClass::Prop).unwrap();
    assert_eq!(a.hash, b.hash);
    assert_eq!(a.report.output.triangles, 1);
    validate_glb(&a.glb, 1 << 20, 100).unwrap();
}

#[test]
fn lying_accessor_is_refused() {
    let e = run(
        tri_doc(1_000_000_000, &triangle_buffer()),
        "t.gltf",
        AssetClass::Prop,
    )
    .unwrap_err();
    assert!(matches!(e, Error::Malformed(_) | Error::Limit(_)), "{e}");
}

#[test]
fn external_uri_is_refused() {
    let e = run(tri_doc(3, "../../etc/passwd"), "t.gltf", AssetClass::Prop).unwrap_err();
    assert!(matches!(e, Error::Refused(_)), "{e}");
    let e = run(
        tri_doc(3, "https://example.com/x.bin"),
        "t.gltf",
        AssetClass::Prop,
    )
    .unwrap_err();
    assert!(matches!(e, Error::Refused(_)), "{e}");
}

#[test]
fn giant_data_uri_is_refused() {
    let uri = format!(
        "data:application/octet-stream;base64,{}",
        "A".repeat(40 << 20)
    );
    let e = run(tri_doc(3, &uri), "t.gltf", AssetClass::Prop).unwrap_err();
    assert!(matches!(e, Error::Limit(_)), "{e}");
}

#[test]
fn fbx_and_garbage_are_refused() {
    let e = run(
        b"Kaydara FBX Binary  \0junk".to_vec(),
        "a.fbx",
        AssetClass::Prop,
    )
    .unwrap_err();
    assert!(matches!(e, Error::Unsupported(_)));
    let e = run(
        b"glTF\x02\0\0\0\xff\xff\xff\xff".to_vec(),
        "a.glb",
        AssetClass::Prop,
    )
    .unwrap_err();
    assert!(matches!(e, Error::Malformed(_)), "{e}");
}

fn disc_png() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(128, 128, |x, y| {
        let (dx, dy) = (x as f32 - 64.0, y as f32 - 64.0);
        if dx * dx + dy * dy < 50.0 * 50.0 {
            image::Rgba([200, 60, 40, 255])
        } else {
            image::Rgba([0, 0, 0, 0])
        }
    });
    let mut out = std::io::Cursor::new(vec![]);
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[test]
fn image_papercraft_and_standee() {
    let mut o = NormalizeOptions::new("disc", AssetClass::Prop);
    let a = normalize(Input::bytes(disc_png(), "disc.png"), o.clone()).unwrap();
    // Front + back + sides of a simplified circle.
    assert!(
        a.report.output.triangles > 20 && a.report.output.triangles < 600,
        "{}",
        a.report.output.triangles
    );
    assert!(
        (a.footprint.size_mm[2] - 40).abs() <= 2,
        "depth {}",
        a.footprint.size_mm[2]
    );
    o.image_mode = ImageMode::Standee;
    let s = normalize(Input::bytes(disc_png(), "disc.png"), o.clone()).unwrap();
    assert_eq!(s.report.output.triangles, 2);
    o.image_mode = ImageMode::Relief;
    let depth = image2d::DepthMap {
        width: 2,
        height: 2,
        data: vec![0.0, 1.0, 1.0, 0.0],
    };
    let mut input = Input::bytes(disc_png(), "disc.png");
    input.depth = Some(depth);
    let r = normalize(input, o).unwrap();
    assert!(r.report.output.triangles <= 1000);
}

#[test]
fn bench_stool_gets_a_seat() {
    // 0.45 m high flat box: the heuristic marks a seat on top.
    let (w, h, d) = (0.4f32, 0.45f32, 0.4f32);
    let v = |x: f32, y: f32, z: f32| [x * w - w / 2.0, y * h, z * d - d / 2.0];
    let c = [
        v(0., 0., 0.),
        v(1., 0., 0.),
        v(1., 1., 0.),
        v(0., 1., 0.),
        v(0., 0., 1.),
        v(1., 0., 1.),
        v(1., 1., 1.),
        v(0., 1., 1.),
    ];
    let idx: [u32; 36] = [
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 3, 7, 6, 3, 6, 2, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5,
        0, 4, 7, 0, 7, 3,
    ];
    let mut b = vec![];
    for p in c {
        for f in p {
            b.extend_from_slice(&f.to_le_bytes());
        }
    }
    for i in idx {
        b.extend_from_slice(&(i as u16).to_le_bytes());
    }
    let uri = format!(
        "data:application/octet-stream;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&b)
    );
    let doc = gltf_json(serde_json::json!({
        "asset": {"version": "2.0"},
        "buffers": [{"byteLength": b.len(), "uri": uri}],
        "bufferViews": [{"buffer": 0, "byteLength": 96}, {"buffer": 0, "byteOffset": 96, "byteLength": 72}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 8, "type": "VEC3", "min": [-0.2,0,-0.2], "max": [0.2,0.45,0.2]},
            {"bufferView": 1, "componentType": 5123, "count": 36, "type": "SCALAR"}
        ],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
        "nodes": [{"mesh": 0}],
        "scenes": [{"nodes": [0]}]
    }));
    let a = run(doc, "stool.gltf", AssetClass::Prop).unwrap();
    assert!(
        a.anchors.iter().any(|x| x.action == "sit"),
        "{:?}",
        a.anchors
    );
}
