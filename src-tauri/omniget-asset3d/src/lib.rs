//! Import normalizer of the 3D city (estudo 99, docs 04/05/06; sessão M6).
//!
//! Anything that enters the city (GLB, glTF, VRM, OBJ, a PNG/JPEG/WebP, the
//! output of a generator) goes through [`normalize`], nine steps:
//!
//! 1. signature and limits ([`sniff`]);
//! 2. conversion to the in-memory scene (`gltf` crate, `tobj`; FBX/USDZ
//!    refused with a message);
//! 3. cleaning: weld, dedup, prune, indices, normals;
//! 4. scale and pivot for the 2 m grid by class, feet on y = 0;
//! 5. triangle budget per class (meshoptimizer 1.3 simplify / remesh);
//! 6. textures to 256/512 px WebP, optional fit to the palette;
//! 7. collision and footprint in tiles;
//! 8. anchors (heuristic; [`anchors::anchor_prompt`] for a vision LLM);
//! 9. GLB with `EXT_meshopt_compression` and its SHA-256.
//!
//! The input is hostile: everything runs on a worker thread with a deadline,
//! panics are caught, sizes are checked before allocating, external URIs are
//! refused. The same crate runs in the app and in the server.

pub mod anchors;
pub mod budget;
pub mod clean;
pub mod collide;
pub mod fit;
pub mod image2d;
pub mod load_gltf;
pub mod load_obj;
pub mod meshopt;
pub mod model;
pub mod providers;
pub mod raster;
pub mod store;
pub mod texture;
pub mod types;
pub mod vox;
pub mod vrm;
pub mod write_glb;

pub use types::*;

use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unsupported file: {0}")]
    Unsupported(String),
    #[error("broken file: {0}")]
    Malformed(String),
    #[error("over the limit: {0}")]
    Limit(String),
    #[error("refused: {0}")]
    Refused(String),
    #[error("the import took longer than {0} s and was stopped")]
    Timeout(u64),
    #[error("the import crashed on this file: {0}")]
    Panic(String),
    #[error("licence: {0}")]
    License(String),
    #[error("{0}")]
    Provider(String),
    #[error("{0}")]
    Io(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Glb,
    Gltf,
    Obj,
    Png,
    Jpeg,
    Webp,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Glb => "glb",
            Format::Gltf => "gltf",
            Format::Obj => "obj",
            Format::Png => "png",
            Format::Jpeg => "jpeg",
            Format::Webp => "webp",
        }
    }

    pub fn is_image(self) -> bool {
        matches!(self, Format::Png | Format::Jpeg | Format::Webp)
    }
}

/// Passo 1: the real format from the bytes (the extension only breaks ties
/// for text formats and names the refusal).
pub fn sniff(bytes: &[u8], file_name: &str) -> Result<Format, Error> {
    let ext = file_name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let b = bytes;
    if b.starts_with(b"glTF") {
        return Ok(Format::Glb);
    }
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Ok(Format::Png);
    }
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok(Format::Jpeg);
    }
    if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return Ok(Format::Webp);
    }
    if b.starts_with(b"Kaydara FBX Binary") || b.starts_with(b"; FBX") || ext == "fbx" {
        return Err(Error::Unsupported(
            "FBX is not supported; export the model as GLB (Blender: File → Export → glTF 2.0)"
                .into(),
        ));
    }
    if ext == "usdz"
        || ext == "usd"
        || ext == "usda"
        || ext == "usdc"
        || b.starts_with(b"PXR-USDC")
        || b.starts_with(b"#usda")
    {
        return Err(Error::Unsupported(
            "USD/USDZ is not supported; export the model as GLB".into(),
        ));
    }
    if b.starts_with(b"PK\x03\x04") {
        return Err(Error::Unsupported(
            "archives are not supported; unzip and import the .glb".into(),
        ));
    }
    let head = &b[..b.len().min(64 * 1024)];
    let text = std::str::from_utf8(head)
        .map(|s| s.to_string())
        .unwrap_or_else(|e| String::from_utf8_lossy(&head[..e.valid_up_to()]).into_owned());
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with('{') {
        return Ok(Format::Gltf);
    }
    let looks_obj = text.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("v ") || l.starts_with("f ") || l.starts_with("mtllib ")
    });
    if looks_obj || ext == "obj" {
        return Ok(Format::Obj);
    }
    Err(Error::Unsupported(format!(
        "\"{}\" is not a GLB, glTF, VRM, OBJ, PNG, JPEG or WebP file",
        file_name.chars().take(80).collect::<String>()
    )))
}

pub type ResolverFn = dyn Fn(&str) -> Result<Vec<u8>, Error> + Send + Sync;

/// Everything [`normalize`] reads besides the options.
#[derive(Clone)]
pub struct Input {
    pub bytes: Vec<u8>,
    pub file_name: String,
    /// Reads sibling files of a `.gltf`/`.obj` (buffers, textures, .mtl).
    pub resolver: Option<Arc<ResolverFn>>,
    /// Depth map already computed for [`ImageMode::Relief`].
    pub depth: Option<image2d::DepthMap>,
    pub depth_estimator: Option<Arc<dyn image2d::DepthEstimator>>,
}

impl Input {
    pub fn bytes(bytes: Vec<u8>, file_name: &str) -> Self {
        Input {
            bytes,
            file_name: file_name.into(),
            resolver: None,
            depth: None,
            depth_estimator: None,
        }
    }
}

/// Resolver that reads plain file names from one folder (no sub-paths).
pub fn folder_resolver(dir: std::path::PathBuf, max_bytes: usize) -> Arc<ResolverFn> {
    Arc::new(move |name: &str| {
        let name = name.replace('\\', "/");
        let file = name.rsplit('/').next().unwrap_or("");
        if file.is_empty() || file.starts_with('.') || name.contains("..") {
            return Err(Error::Refused(format!(
                "\"{name}\" is not a file next to the model"
            )));
        }
        // Allow one level of sub-folder (textures/foo.png) but nothing above.
        let rel = std::path::Path::new(&name);
        if rel.is_absolute() || rel.components().count() > 2 {
            return Err(Error::Refused(format!(
                "\"{name}\" is not a file next to the model"
            )));
        }
        let path = dir.join(rel);
        let meta = std::fs::metadata(&path)
            .map_err(|_| Error::Malformed(format!("\"{name}\" is missing")))?;
        if meta.len() as usize > max_bytes {
            return Err(Error::Limit(format!(
                "\"{name}\" is larger than the import allows"
            )));
        }
        std::fs::read(&path).map_err(|e| Error::Io(e.to_string()))
    })
}

/// Runs `f` on a worker thread with a deadline; panics become errors. On
/// timeout the worker is abandoned (it holds no lock and writes nothing).
pub fn guarded<T: Send + 'static>(
    timeout_ms: u64,
    f: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("asset3d-normalize".into())
        .stack_size(64 << 20)
        .spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
            let r = match r {
                Ok(r) => r,
                Err(p) => Err(Error::Panic(
                    p.downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| p.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown".into()),
                )),
            };
            let _ = tx.send(r);
        })
        .map_err(|e| Error::Io(e.to_string()))?;
    match rx.recv_timeout(Duration::from_millis(timeout_ms.max(1))) {
        Ok(r) => r,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(Error::Timeout(timeout_ms / 1000)),
        Err(_) => Err(Error::Panic("worker vanished".into())),
    }
}

/// The nine steps, guarded. See the crate docs.
pub fn normalize(input: Input, opts: NormalizeOptions) -> Result<NormalizedAsset, Error> {
    let timeout = opts.limits.timeout_ms;
    guarded(timeout, move || normalize_unguarded(&input, &opts))
}

fn stats_of(model: &model::Model, bytes: usize) -> Stats {
    Stats {
        bytes,
        triangles: model.triangles(),
        vertices: model.vertices(),
        meshes: model.meshes.len(),
        materials: model.materials.len(),
        textures: model.images.len(),
        texture_px_max: model
            .images
            .iter()
            .map(|i| i.rgba.width().max(i.rgba.height()))
            .max()
            .unwrap_or(0),
    }
}

/// [`normalize`] on the calling thread (tests, benches, already-guarded
/// callers).
pub fn normalize_unguarded(
    input: &Input,
    opts: &NormalizeOptions,
) -> Result<NormalizedAsset, Error> {
    let t0 = Instant::now();
    let mut steps: Vec<StepTime> = vec![];
    let mut lap = {
        let mut last = Instant::now();
        move |name: &str, steps: &mut Vec<StepTime>| {
            let now = Instant::now();
            steps.push(StepTime {
                step: name.into(),
                ms: (now - last).as_secs_f64() * 1000.0,
            });
            last = now;
        }
    };
    let mut warnings: Vec<String> = vec![];
    let limits = &opts.limits;
    let mut class = opts.class;

    // 1. Signature and limits.
    if input.bytes.len() > limits.max_file_bytes {
        return Err(Error::Limit(format!(
            "the file has {} MB (max {} MB)",
            input.bytes.len() >> 20,
            limits.max_file_bytes >> 20
        )));
    }
    let format = sniff(&input.bytes, &input.file_name)?;
    let source_hash = hex::encode(Sha256::digest(&input.bytes));
    lap("sniff", &mut steps);

    // 2. Conversion.
    let resolver: Option<load_gltf::Resolver> = input
        .resolver
        .as_deref()
        .map(|r| r as &dyn Fn(&str) -> Result<Vec<u8>, Error>);
    let mut model = match format {
        Format::Glb | Format::Gltf => {
            let l = load_gltf::load(&input.bytes, limits, resolver)?;
            warnings.extend(l.warnings);
            l.model
        }
        Format::Obj => {
            let (m, w) = load_obj::load(&input.bytes, limits, resolver)?;
            warnings.extend(w);
            m
        }
        Format::Png | Format::Jpeg | Format::Webp => {
            let rgba = image2d::decode(&input.bytes, limits.max_texture_px)?;
            match opts.image_mode {
                ImageMode::Standee => image2d::standee(rgba, class),
                ImageMode::Papercraft => image2d::papercraft(rgba, class, 2.0)?,
                ImageMode::Relief => {
                    let target = class.triangle_budget().min(1000);
                    if let Some(d) = &input.depth {
                        image2d::relief_from_depth(rgba, d, class, target)?
                    } else if let Some(est) = &input.depth_estimator {
                        image2d::relief(rgba, est.as_ref(), class, target)?
                    } else {
                        return Err(Error::Provider(
                            "relief needs the depth model, which is not installed; use standee or papercraft".into(),
                        ));
                    }
                }
            }
        }
    };
    let input_stats = stats_of(&model, input.bytes.len());
    if input_stats.triangles == 0 {
        return Err(Error::Malformed("the file has no triangles".into()));
    }
    let vrm_info = vrm::info(&model.root_extensions);
    if vrm_info.is_some() && class != AssetClass::Character {
        warnings.push(format!(
            "VRM files are characters; imported as character instead of {}",
            class.as_str()
        ));
        class = AssetClass::Character;
    }
    if let Some(l) = &opts.license {
        l.validate(vrm_info.is_some()).map_err(Error::License)?;
    }
    lap("convert", &mut steps);

    // 3. Cleaning.
    let preserve = class == AssetClass::Character && (!model.skins.is_empty() || model.is_vrm());
    if !preserve {
        model = clean::flatten(&model);
    }
    for mesh in &mut model.meshes {
        for p in &mut mesh.prims {
            clean::weld(p);
            clean::ensure_normals(p);
        }
    }
    if !preserve {
        clean::dedup_and_prune(&mut model);
    } else {
        clean::resample_animations(&mut model);
    }
    lap("clean", &mut steps);

    // 4. Scale and pivot.
    let f = fit::fit(&mut model, class, !preserve, format == Format::Obj)?;
    if f.rotated_z_up {
        warnings.push("the model was Z-up and was stood upright".into());
    }
    lap("fit", &mut steps);

    // 5. Triangle budget.
    let b = budget::apply(&mut model, class, !preserve, &mut warnings);
    if b.remeshed > 0 {
        warnings.push(format!(
            "{} part(s) were remeshed; their colour is now per vertex",
            b.remeshed
        ));
    }
    lap("budget", &mut steps);

    // 6. Textures and style.
    let px = opts
        .texture_px
        .unwrap_or(class.texture_px())
        .clamp(64, 1024);
    if opts.fit_style {
        let palette = texture::Palette::new(opts.palette.as_ref());
        if preserve {
            texture::fit_style_textures(&mut model, &palette);
        } else {
            texture::fit_style_static(&mut model, &palette);
        }
    }
    texture::encode_images(&mut model, px, opts.webp_quality.clamp(0.0, 100.0));
    lap("texture", &mut steps);

    // 7. Collision and footprint.
    let tris = collide::world_triangles(&model);
    let bb = model::Aabb::of(
        &tris
            .iter()
            .flat_map(|t| t.iter().copied())
            .collect::<Vec<_>>(),
    );
    let footprint = collide::footprint(&tris, &bb);
    let collision = collide::collision(class, &tris, &bb);
    let front = collide::front(&tris);
    lap("collide", &mut steps);

    // 8. Anchors (heuristic; the vision pass is separate, see anchors.rs).
    let anchors = anchors::heuristic(class, &tris, &bb, &front);
    lap("anchors", &mut steps);

    // 9. Pack.
    let extras = serde_json::json!({
        "omniget": {
            "schema": NORMALIZER_VERSION,
            "name": opts.name,
            "class": class,
            "footprint": footprint,
            "collision": collision,
            "anchors": anchors,
            "front": front,
            "license": opts.license,
            "provenance": opts.provenance,
            "source_sha256": source_hash,
        }
    });
    let copyright = opts.license.as_ref().and_then(|l| l.author.clone());
    let glb = write_glb::write(
        &model,
        &write_glb::WriteOptions {
            quantize_positions: !preserve,
            compress: true,
            asset_extras: extras,
            copyright,
        },
    );
    let hash = hex::encode(Sha256::digest(&glb));
    let thumbnail_png = raster::thumbnail(&model, 256);
    lap("pack", &mut steps);

    let output = stats_of(&model, glb.len());
    Ok(NormalizedAsset {
        glb,
        hash,
        source_hash,
        name: opts.name.clone(),
        class,
        footprint,
        collision,
        anchors,
        front,
        vrm: vrm_info,
        thumbnail_png,
        report: Report {
            format: format.as_str().into(),
            input: input_stats,
            output,
            steps,
            total_ms: t0.elapsed().as_secs_f64() * 1000.0,
            scale: f.scale,
            warnings,
        },
    })
}

/// Reads a normalized GLB back into the in-memory scene (for export, views
/// and the server's "validate only" mode).
pub fn load_glb(glb: &[u8], limits: &Limits) -> Result<model::Model, Error> {
    Ok(load_gltf::load(glb, limits, None)?.model)
}

/// Extensions a published asset may use.
pub const ALLOWED_EXTENSIONS: &[&str] = &[
    "EXT_meshopt_compression",
    "KHR_meshopt_compression",
    "KHR_mesh_quantization",
    "EXT_texture_webp",
    "KHR_materials_unlit",
    "KHR_texture_transform",
    "VRM",
    "VRMC_vrm",
    "VRMC_springBone",
    "VRMC_node_constraint",
    "VRMC_materials_mtoon",
];

/// Server-side check of an asset someone uploads (the client is never
/// trusted): size, triangles, extensions. Returns the stats.
pub fn validate_glb(glb: &[u8], max_bytes: usize, max_triangles: usize) -> Result<Stats, Error> {
    if glb.len() > max_bytes {
        return Err(Error::Limit(format!(
            "asset has {} bytes (max {max_bytes})",
            glb.len()
        )));
    }
    if !glb.starts_with(b"glTF") {
        return Err(Error::Unsupported("published assets must be GLB".into()));
    }
    let bytes = glb.to_vec();
    let limits = Limits {
        max_triangles,
        timeout_ms: 20_000,
        ..Default::default()
    };
    guarded(limits.timeout_ms, move || {
        let loaded = load_gltf::load(&bytes, &limits, None)?;
        for e in &loaded.extensions_used {
            if !ALLOWED_EXTENSIONS.contains(&e.as_str()) {
                return Err(Error::Refused(format!(
                    "extension {e} is not allowed in published assets"
                )));
            }
        }
        Ok(stats_of(&loaded.model, bytes.len()))
    })
}

/// The request G2 sends to a vision LLM for an asset (step 8).
pub fn vision_request(glb: &[u8], class: AssetClass) -> Result<anchors::AnchorPrompt, Error> {
    let model = load_glb(glb, &Limits::default())?;
    let tris = collide::world_triangles(&model);
    let bb = model::Aabb::of(
        &tris
            .iter()
            .flat_map(|t| t.iter().copied())
            .collect::<Vec<_>>(),
    );
    let front = collide::front(&tris);
    let heur = anchors::heuristic(class, &tris, &bb, &front);
    let s = bb.size();
    let size_mm = [
        (s[0] * 1000.0) as i32,
        (s[1] * 1000.0) as i32,
        (s[2] * 1000.0) as i32,
    ];
    Ok(anchors::anchor_prompt(
        raster::four_views(&model, 512),
        class,
        size_mm,
        &heur,
    ))
}

/// Validates a vision LLM reply against the asset's geometry.
pub fn vision_apply(glb: &[u8], reply: &str) -> Result<anchors::VisionAnnotation, Error> {
    let model = load_glb(glb, &Limits::default())?;
    let tris = collide::world_triangles(&model);
    let bb = model::Aabb::of(
        &tris
            .iter()
            .flat_map(|t| t.iter().copied())
            .collect::<Vec<_>>(),
    );
    anchors::parse_anchor_reply(reply, &tris, &bb).map_err(Error::Provider)
}

/// Plain, indexed, untextured GLB of an asset, for skin-tokens.cpp.
pub fn plain_glb_for_rigging(glb: &[u8]) -> Result<Vec<u8>, Error> {
    let model = load_glb(glb, &Limits::default())?;
    let mut flat = clean::flatten(&model);
    for p in flat.meshes.iter_mut().flat_map(|m| m.prims.iter_mut()) {
        p.uv = None;
        p.col = None;
        p.material = None;
        clean::weld(p);
    }
    flat.materials.clear();
    flat.textures.clear();
    flat.images.clear();
    clean::merge_same_material(&mut flat);
    Ok(write_glb::write(
        &flat,
        &write_glb::WriteOptions {
            quantize_positions: false,
            compress: false,
            asset_extras: serde_json::Value::Null,
            copyright: None,
        },
    ))
}

/// `.vox` of a normalized asset.
pub fn export_vox(glb: &[u8], voxel_mm: u32) -> Result<vox::VoxResult, Error> {
    let bytes = glb.to_vec();
    guarded(60_000, move || {
        let model = load_glb(&bytes, &Limits::default())?;
        vox::export(&model, voxel_mm).map_err(Error::Malformed)
    })
}
