//! OBJ (+ MTL) reader on `tobj`. Materials and their diffuse maps come only
//! through the resolver (files next to the .obj); without one the model is
//! imported with plain grey materials.

use crate::load_gltf::{decode_image, NoBudget, Resolver};
use crate::model::*;
use crate::types::Limits;
use crate::Error;
use std::collections::HashMap;
use std::io::BufReader;

pub fn load(
    bytes: &[u8],
    limits: &Limits,
    resolver: Option<Resolver>,
) -> Result<(Model, Vec<String>), Error> {
    let mut warnings = vec![];
    let opts = tobj::LoadOptions {
        single_index: true,
        triangulate: true,
        ignore_points: true,
        ignore_lines: true,
    };
    let mtl_loader = |p: &std::path::Path| -> tobj::MTLLoadResult {
        let name = p.to_string_lossy().to_string();
        match resolver {
            Some(r) => match r(&name) {
                Ok(b) => tobj::load_mtl_buf(&mut BufReader::new(&b[..])),
                Err(_) => Err(tobj::LoadError::OpenFileFailed),
            },
            None => Err(tobj::LoadError::OpenFileFailed),
        }
    };
    let (models, mats) = tobj::load_obj_buf(&mut BufReader::new(bytes), &opts, mtl_loader)
        .map_err(|e| Error::Malformed(format!("OBJ: {e}")))?;
    let mats = match mats {
        Ok(m) => m,
        Err(_) => {
            warnings.push("the .mtl file was not found; colours are grey".into());
            vec![]
        }
    };

    let mut model = Model::default();
    let mut image_by_name: HashMap<String, usize> = HashMap::new();
    for m in &mats {
        let d = m.diffuse.unwrap_or([0.8, 0.8, 0.8]);
        let mut mat = Material {
            name: Some(m.name.clone()),
            base_color: [d[0], d[1], d[2], m.dissolve.unwrap_or(1.0).clamp(0.0, 1.0)],
            ..Default::default()
        };
        if mat.base_color[3] < 0.999 {
            mat.alpha_mode = "BLEND".into();
        }
        if let (Some(tex), Some(r)) = (&m.diffuse_texture, resolver) {
            let key = tex.trim().to_string();
            let img = match image_by_name.get(&key) {
                Some(&i) => Some(i),
                None => match r(&key)
                    .and_then(|b| decode_image(&b, limits, &mut NoBudget).map_err(Error::Malformed))
                {
                    Ok(rgba) => {
                        model.images.push(ImageData {
                            name: Some(key.clone()),
                            rgba,
                            encoded: None,
                            mime: "image/webp".into(),
                        });
                        let i = model.images.len() - 1;
                        image_by_name.insert(key.clone(), i);
                        Some(i)
                    }
                    Err(e) => {
                        warnings.push(format!("texture {key}: {e}"));
                        None
                    }
                },
            };
            if let Some(i) = img {
                model.textures.push(Texture {
                    name: None,
                    image: Some(i),
                    sampler: None,
                });
                mat.base_tex = Some(model.textures.len() - 1);
            }
        }
        model.materials.push(mat);
    }

    let mut total = 0usize;
    let mut mesh = Mesh {
        name: Some("obj".into()),
        ..Default::default()
    };
    for m in models {
        let t = m.mesh;
        let n = t.positions.len() / 3;
        if n == 0 {
            continue;
        }
        let pos: Vec<[f32; 3]> = t
            .positions
            .chunks_exact(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect();
        let nrm = (t.normals.len() == n * 3).then(|| {
            t.normals
                .chunks_exact(3)
                .map(|c| [c[0], c[1], c[2]])
                .collect()
        });
        // OBJ v points down the image; glTF v points up from the top.
        let uv = (t.texcoords.len() == n * 2).then(|| {
            t.texcoords
                .chunks_exact(2)
                .map(|c| [c[0], 1.0 - c[1]])
                .collect()
        });
        let col = (t.vertex_color.len() == n * 3).then(|| {
            t.vertex_color
                .chunks_exact(3)
                .map(|c| [c[0], c[1], c[2], 1.0])
                .collect()
        });
        if t.indices.iter().any(|&i| i as usize >= n) {
            return Err(Error::Malformed("OBJ index out of range".into()));
        }
        total += t.indices.len() / 3;
        if total > limits.max_triangles {
            return Err(Error::Limit(format!(
                "more than {} triangles",
                limits.max_triangles
            )));
        }
        mesh.prims.push(Prim {
            pos,
            nrm,
            uv,
            col,
            idx: t.indices,
            material: t.material_id.filter(|&i| i < model.materials.len()),
            ..Default::default()
        });
    }
    model.meshes.push(mesh);
    model.nodes.push(Node {
        name: Some("obj".into()),
        mesh: Some(0),
        ..Default::default()
    });
    model.roots = vec![0];
    Ok((model, warnings))
}
