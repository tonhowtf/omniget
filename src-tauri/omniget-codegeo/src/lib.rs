//! Geometry by code for the 3D city (estudo 99, arquivo 12; sessão M5).
//!
//! A person or an LLM writes a short Rhai script against a small kit of
//! styled primitives (`box`, `gable`, `door`, `seat`…). The script runs in a
//! sandbox with hard limits, the CSG goes through Manifold (pure Rust port),
//! and the result is a canonical GLB whose identity is the hash of the
//! recipe plus the engine version. The same machinery builds the modular kit
//! of each city style from the scripts in `kit/`.
//!
//! Entry points: [`run_script`], [`run_cached`], [`kit_reference`],
//! [`kit::build_kit`], [`typology`].

pub mod canon;
pub mod geom;
pub mod glb;
pub mod kit;
pub mod palette;
pub mod raster;
mod reference;
pub mod rng;
pub mod script;
pub mod sprites;
pub mod typology;
pub mod validate;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Instant;

pub use omniget_citygen::types::Anchor;
pub use reference::{examples, kit_reference, Example};
pub use validate::{Class, ClassKind};

/// Goes into every recipe hash: bump it whenever the kit's output changes.
pub const ENGINE_VERSION: &str = "codegeo-1/manifold-rust-0.13.1/rhai-1.26";

/// Anchor actions a script may create. `enter` is the door anchor.
pub const ACTIONS: &[&str] = &[
    "enter", "sit", "sleep", "work", "plant", "water", "harvest", "talk", "stand",
];

/// An anchor in the object's own metres, before snapping to the city grid.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AnchorPoint {
    pub action: String,
    pub pos: [f64; 3],
    /// Clockwise from north, degrees: the way the person faces while using it.
    pub yaw_deg: f64,
    pub capacity: u8,
}

/// Ground footprint in 2 m tiles; `origin_mm` is its north-west corner in the
/// object's frame.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Footprint {
    pub w: i32,
    pub d: i32,
    pub origin_mm: [i32; 2],
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ScriptError {
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub message: String,
    /// syntax | runtime | limit | unknown_function | result | geometry |
    /// validation | semantic | size | anchor | class | cache
    pub kind: String,
}

impl ScriptError {
    pub fn plain(kind: &str, message: String) -> Self {
        ScriptError {
            line: None,
            column: None,
            message,
            kind: kind.into(),
        }
    }
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.line, self.column) {
            (Some(l), Some(c)) => write!(f, "line {l}, column {c}: {}", self.message),
            (Some(l), None) => write!(f, "line {l}: {}", self.message),
            _ => f.write_str(&self.message),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Output {
    /// sha256 of the recipe (class + script) plus [`ENGINE_VERSION`].
    pub hash: String,
    /// What a `BuildingSpec.typology` stores: `codegeo:<hash>`.
    pub typology: String,
    /// sha256 of the canonical mesh (for cross-platform checks).
    pub mesh_hash: String,
    pub class: String,
    #[serde(skip)]
    pub glb: Vec<u8>,
    pub triangles: usize,
    pub anchors: Vec<Anchor>,
    pub anchor_points: Vec<AnchorPoint>,
    pub footprint: Footprint,
    /// `[min, max]` in millimetres.
    pub bounds_mm: [[i32; 3]; 2],
    pub slots: Vec<String>,
    pub warnings: Vec<String>,
    pub ms: f64,
}

pub fn recipe_hash(script: &str, class: &str) -> String {
    let mut h = Sha256::new();
    h.update(ENGINE_VERSION.as_bytes());
    h.update(b"\n");
    h.update(class.trim().to_ascii_lowercase().as_bytes());
    h.update(b"\n");
    h.update(script.as_bytes());
    canon::hex(&h.finalize())
}

/// Compiles, runs, validates and packs a script. `class` is `prop`,
/// `building` or `module`, optionally with a kind (`prop:bench`,
/// `building:house`, `module:telhado`).
pub fn run_script(script: &str, class: &str) -> Result<Output, Vec<ScriptError>> {
    run_with_mesh(script, class).map(|(o, _)| o)
}

/// [`run_script`] that also hands back the canonical mesh (previews).
pub fn run_with_mesh(
    script: &str,
    class: &str,
) -> Result<(Output, canon::CanonMesh), Vec<ScriptError>> {
    let t0 = Instant::now();
    if script.len() > 32 * 1024 {
        return Err(vec![ScriptError::plain(
            "limit",
            "the script is over 32 KB".into(),
        )]);
    }
    let cls = Class::parse(class).map_err(|e| vec![e])?;
    let ast = script::compile(script).map_err(|e| vec![e])?;
    let raw = script::run(
        &ast,
        script::RunSpec {
            limits: script::Limits::user(cls.max_tris()),
            lib: None,
            vars: vec![],
            seed: 0,
            lod: 0,
            allow_empty: false,
            grid: true,
        },
    )
    .map_err(|e| vec![e])?;
    let solid = raw.solid.expect("non-empty");
    let mesh = canon::CanonMesh::from_manifold(&solid, &raw.ctx.slot_of)
        .map_err(|m| vec![ScriptError::plain("geometry", m)])?;
    let fp = validate::footprint(&mesh);
    let verdict = validate::check(&cls, raw.ctx.kind.as_deref(), &mesh, &raw.ctx.anchors, &fp);
    if !verdict.errors.is_empty() {
        return Err(verdict.errors);
    }
    let hash = recipe_hash(script, class);
    let glb = glb::write(
        std::slice::from_ref(&mesh),
        &[glb::GlbNode {
            name: format!("codegeo:{hash}"),
            mesh: if mesh.is_empty() { None } else { Some(0) },
            anchors: raw.ctx.anchors.clone(),
            extras: Some(serde_json::json!({ "class": class, "engine": ENGINE_VERSION })),
        }],
        &palette::default_palette("vila-colonial"),
        &format!("omniget-codegeo {ENGINE_VERSION}"),
    );
    let (lo, hi) = mesh.bounds();
    let mm = |v: [i32; 3]| [v[0] / 10, v[1] / 10, v[2] / 10];
    let out = Output {
        typology: format!("codegeo:{hash}"),
        hash,
        mesh_hash: mesh.hash(),
        class: class.trim().to_ascii_lowercase(),
        glb,
        triangles: mesh.len(),
        anchors: raw
            .ctx
            .anchors
            .iter()
            .map(|a| validate::grid_anchor(a, &fp))
            .collect(),
        anchor_points: raw.ctx.anchors,
        footprint: fp,
        bounds_mm: [mm(lo), mm(hi)],
        slots: mesh
            .slots()
            .into_iter()
            .map(|s| palette::slot_name(s).to_string())
            .collect(),
        warnings: verdict.warnings,
        ms: t0.elapsed().as_secs_f64() * 1e3,
    };
    Ok((out, mesh))
}

/// [`run_script`] with a cache in `dir` (`<hash>.glb` + `<hash>.json`).
/// Errors are not cached: a failing script is cheap to re-run.
pub fn run_cached(dir: &Path, script: &str, class: &str) -> Result<Output, Vec<ScriptError>> {
    let hash = recipe_hash(script, class);
    let (glb_path, meta_path) = (
        dir.join(format!("{hash}.glb")),
        dir.join(format!("{hash}.json")),
    );
    if let (Ok(glb), Ok(meta)) = (std::fs::read(&glb_path), std::fs::read(&meta_path)) {
        if let Ok(mut out) = serde_json::from_slice::<Output>(&meta) {
            out.glb = glb;
            return Ok(out);
        }
    }
    let out = run_script(script, class)?;
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{hash}.glb.tmp"));
        std::fs::write(&tmp, &out.glb)?;
        std::fs::rename(&tmp, &glb_path)?;
        std::fs::write(&meta_path, serde_json::to_vec(&out).unwrap_or_default())
    };
    // A cache write failure is not the script's fault.
    let _ = write();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUSE: &str = r#"
let w = 4.0; let d = 3.2; let h = 2.6;
let body = box([w, h, d], #{ mat: "wall" });
body = door(body, #{ at: [0.0, 0.0, d / 2.0], face: "s" });
body = window(body, #{ at: [-1.2, 1.0, d / 2.0], face: "s" });
[body, gable(#{ w: w, d: d, h: 1.4, at: [0.0, h, 0.0] })]
"#;

    #[test]
    fn same_script_same_bytes() {
        let a = run_script(HOUSE, "building:house").unwrap();
        let b = run_script(HOUSE, "building:house").unwrap();
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.mesh_hash, b.mesh_hash);
        assert_eq!(a.glb, b.glb);
        assert!(a.anchors.iter().any(|x| x.action == "enter"));
        assert_eq!(&a.glb[0..4], b"glTF");
    }

    #[test]
    fn house_without_door_is_refused() {
        let e = run_script("box([4.0, 3.0, 4.0])", "building").unwrap_err();
        assert!(e.iter().any(|x| x.kind == "semantic"), "{e:?}");
    }

    #[test]
    fn bench_needs_a_seat() {
        let e = run_script("box([1.2, 0.45, 0.5], #{ mat: \"wood\" })", "prop:bench").unwrap_err();
        assert!(e.iter().any(|x| x.kind == "semantic"));
        let ok = run_script("seat(#{ w: 1.2 })", "prop:bench").unwrap();
        assert_eq!(ok.anchors[0].action, "sit");
    }

    #[test]
    fn hostile_scripts_are_stopped() {
        let e = run_script("let a = []; loop { a.push(box([1.0,1.0,1.0])); }", "prop").unwrap_err();
        assert_eq!(e[0].kind, "limit", "{e:?}");
        assert!(e[0].line.is_some());
        let e = run_script("let x = 0; loop { x += 1; }", "prop").unwrap_err();
        assert_eq!(e[0].kind, "limit");
        let e = run_script("box([1.0, 1.0, 1.0], #{ mat: \"neon\" })", "prop").unwrap_err();
        assert!(e[0].message.contains("palette"), "{e:?}");
        assert_eq!(e[0].line, Some(1));
        let e = run_script("let x = ;", "prop").unwrap_err();
        assert_eq!(e[0].kind, "syntax");
    }
}
