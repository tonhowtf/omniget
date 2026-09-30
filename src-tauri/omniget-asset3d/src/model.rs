//! In-memory scene the normalizer works on. Loaders fill it, passes rewrite
//! it, the GLB writer serializes it. Indices of nodes, meshes, materials,
//! textures and images keep the source order so VRM extensions that point at
//! them stay valid.

use serde_json::Value;

pub type Mat4 = [f32; 16];

pub const IDENTITY: Mat4 = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

#[derive(Clone, Debug, Default)]
pub struct MorphTarget {
    pub pos: Vec<[f32; 3]>,
    pub nrm: Option<Vec<[f32; 3]>>,
}

#[derive(Clone, Debug, Default)]
pub struct Prim {
    pub pos: Vec<[f32; 3]>,
    pub nrm: Option<Vec<[f32; 3]>>,
    pub uv: Option<Vec<[f32; 2]>>,
    pub col: Option<Vec<[f32; 4]>>,
    pub joints: Option<Vec<[u16; 4]>>,
    pub weights: Option<Vec<[f32; 4]>>,
    pub targets: Vec<MorphTarget>,
    /// Triangle list.
    pub idx: Vec<u32>,
    pub material: Option<usize>,
}

impl Prim {
    pub fn tris(&self) -> usize {
        self.idx.len() / 3
    }

    /// Reorders/compacts every vertex stream with a remap table
    /// (`u32::MAX` = dropped) into `unique` vertices.
    pub fn apply_remap(&mut self, remap: &[u32], unique: usize) {
        fn go<T: Copy + Default>(v: &[T], remap: &[u32], unique: usize) -> Vec<T> {
            let mut out = vec![T::default(); unique];
            for (i, &r) in remap.iter().enumerate() {
                if r != u32::MAX {
                    out[r as usize] = v[i];
                }
            }
            out
        }
        self.pos = go(&self.pos, remap, unique);
        if let Some(n) = &self.nrm {
            self.nrm = Some(go(n, remap, unique));
        }
        if let Some(n) = &self.uv {
            self.uv = Some(go(n, remap, unique));
        }
        if let Some(n) = &self.col {
            self.col = Some(go(n, remap, unique));
        }
        if let Some(n) = &self.joints {
            self.joints = Some(go(n, remap, unique));
        }
        if let Some(n) = &self.weights {
            self.weights = Some(go(n, remap, unique));
        }
        for t in &mut self.targets {
            t.pos = go(&t.pos, remap, unique);
            if let Some(n) = &t.nrm {
                t.nrm = Some(go(n, remap, unique));
            }
        }
        for i in &mut self.idx {
            *i = remap[*i as usize];
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub name: Option<String>,
    pub prims: Vec<Prim>,
    pub weights: Option<Vec<f32>>,
    /// `extras` of the source mesh (morph target names live here).
    pub extras: Option<Value>,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub name: Option<String>,
    pub t: [f32; 3],
    pub r: [f32; 4],
    pub s: [f32; 3],
    pub mesh: Option<usize>,
    pub skin: Option<usize>,
    pub children: Vec<usize>,
    pub weights: Option<Vec<f32>>,
    /// Kept verbatim (VRMC_node_constraint lives here).
    pub extensions: Option<Value>,
}

impl Default for Node {
    fn default() -> Self {
        Node {
            name: None,
            t: [0.0; 3],
            r: [0.0, 0.0, 0.0, 1.0],
            s: [1.0; 3],
            mesh: None,
            skin: None,
            children: vec![],
            weights: None,
            extensions: None,
        }
    }
}

impl Node {
    pub fn local(&self) -> Mat4 {
        from_trs(self.t, self.r, self.s)
    }
}

#[derive(Clone, Debug)]
pub struct Material {
    pub name: Option<String>,
    /// Source JSON (written back verbatim when the hierarchy is preserved).
    pub raw: Value,
    pub base_color: [f32; 4],
    pub base_tex: Option<usize>,
    pub alpha_mode: String,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
    pub emissive: [f32; 3],
    pub unlit: bool,
}

impl Default for Material {
    fn default() -> Self {
        Material {
            name: None,
            raw: Value::Object(Default::default()),
            base_color: [1.0; 4],
            base_tex: None,
            alpha_mode: "OPAQUE".into(),
            alpha_cutoff: 0.5,
            double_sided: false,
            emissive: [0.0; 3],
            unlit: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Texture {
    pub name: Option<String>,
    pub image: Option<usize>,
    pub sampler: Option<Value>,
}

#[derive(Clone, Debug)]
pub struct ImageData {
    pub name: Option<String>,
    pub rgba: image::RgbaImage,
    /// Encoded bytes to write (filled by the texture pass).
    pub encoded: Option<Vec<u8>>,
    pub mime: String,
}

#[derive(Clone, Debug, Default)]
pub struct Skin {
    pub name: Option<String>,
    pub joints: Vec<usize>,
    pub ibm: Option<Vec<Mat4>>,
    pub skeleton: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct AnimSampler {
    pub input: Vec<f32>,
    pub output: Vec<f32>,
    pub comps: usize,
    pub interpolation: String,
}

#[derive(Clone, Debug)]
pub struct AnimChannel {
    pub sampler: usize,
    pub node: usize,
    pub path: String,
}

#[derive(Clone, Debug, Default)]
pub struct Animation {
    pub name: Option<String>,
    pub channels: Vec<AnimChannel>,
    pub samplers: Vec<AnimSampler>,
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub images: Vec<ImageData>,
    pub skins: Vec<Skin>,
    pub animations: Vec<Animation>,
    /// Root `extensions` kept verbatim (VRM, VRMC_*).
    pub root_extensions: serde_json::Map<String, Value>,
}

impl Model {
    pub fn triangles(&self) -> usize {
        self.meshes
            .iter()
            .flat_map(|m| &m.prims)
            .map(Prim::tris)
            .sum()
    }

    pub fn vertices(&self) -> usize {
        self.meshes
            .iter()
            .flat_map(|m| &m.prims)
            .map(|p| p.pos.len())
            .sum()
    }

    pub fn is_vrm(&self) -> bool {
        self.root_extensions.contains_key("VRMC_vrm") || self.root_extensions.contains_key("VRM")
    }

    /// Global matrix of every node (nodes not reachable from a root get their
    /// local matrix). Cycles were rejected by the loader.
    pub fn world_matrices(&self) -> Vec<Mat4> {
        let mut out = vec![IDENTITY; self.nodes.len()];
        let mut done = vec![false; self.nodes.len()];
        let mut stack: Vec<(usize, Mat4)> = self.roots.iter().map(|&r| (r, IDENTITY)).collect();
        while let Some((n, parent)) = stack.pop() {
            if n >= self.nodes.len() || done[n] {
                continue;
            }
            done[n] = true;
            let m = mul(&parent, &self.nodes[n].local());
            out[n] = m;
            for &c in &self.nodes[n].children {
                stack.push((c, m));
            }
        }
        for (i, d) in done.iter().enumerate() {
            if !d {
                out[i] = self.nodes[i].local();
            }
        }
        out
    }

    /// Nodes reachable from the scene roots.
    pub fn reachable(&self) -> Vec<bool> {
        let mut seen = vec![false; self.nodes.len()];
        let mut stack = self.roots.clone();
        while let Some(n) = stack.pop() {
            if n >= self.nodes.len() || seen[n] {
                continue;
            }
            seen[n] = true;
            stack.extend(self.nodes[n].children.iter().copied());
        }
        seen
    }

    /// Positions of every drawn vertex in world space, in the rest pose of
    /// the file (skinned meshes are skinned with the node globals and IBMs).
    pub fn world_positions(&self) -> Vec<[f32; 3]> {
        let world = self.world_matrices();
        let reach = self.reachable();
        let mut out = vec![];
        for (ni, node) in self.nodes.iter().enumerate() {
            if !reach[ni] {
                continue;
            }
            let Some(mi) = node.mesh else { continue };
            let Some(mesh) = self.meshes.get(mi) else {
                continue;
            };
            let skin_mats = node
                .skin
                .and_then(|s| self.skins.get(s))
                .map(|s| skin_matrices(s, &world));
            for p in &mesh.prims {
                for (vi, v) in p.pos.iter().enumerate() {
                    let w = match (&skin_mats, &p.joints, &p.weights) {
                        (Some(sm), Some(j), Some(wt)) => skin_point(sm, j[vi], wt[vi], *v),
                        _ => transform_point(&world[ni], *v),
                    };
                    out.push(w);
                }
            }
        }
        out
    }
}

/// `joint_global * inverse_bind` per joint.
pub fn skin_matrices(skin: &Skin, world: &[Mat4]) -> Vec<Mat4> {
    skin.joints
        .iter()
        .enumerate()
        .map(|(k, &j)| {
            let g = world.get(j).copied().unwrap_or(IDENTITY);
            let ibm = skin
                .ibm
                .as_ref()
                .and_then(|v| v.get(k))
                .copied()
                .unwrap_or(IDENTITY);
            mul(&g, &ibm)
        })
        .collect()
}

pub fn skin_point(mats: &[Mat4], j: [u16; 4], w: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let mut acc = [0f32; 3];
    let mut total = 0.0;
    for k in 0..4 {
        if w[k] <= 0.0 {
            continue;
        }
        let Some(m) = mats.get(j[k] as usize) else {
            continue;
        };
        let p = transform_point(m, v);
        for a in 0..3 {
            acc[a] += p[a] * w[k];
        }
        total += w[k];
    }
    if total <= 1e-6 {
        return v;
    }
    [acc[0] / total, acc[1] / total, acc[2] / total]
}

pub fn skin_dir(mats: &[Mat4], j: [u16; 4], w: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let mut acc = [0f32; 3];
    for k in 0..4 {
        if w[k] <= 0.0 {
            continue;
        }
        let Some(m) = mats.get(j[k] as usize) else {
            continue;
        };
        let p = transform_normal(m, v);
        for a in 0..3 {
            acc[a] += p[a] * w[k];
        }
    }
    normalize3(acc)
}

// ── Small column-major matrix math ─────────────────────────────────────────

pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut o = [0f32; 16];
    for c in 0..4 {
        for r in 0..4 {
            let mut s = 0.0;
            for k in 0..4 {
                s += a[k * 4 + r] * b[c * 4 + k];
            }
            o[c * 4 + r] = s;
        }
    }
    o
}

pub fn from_trs(t: [f32; 3], q: [f32; 4], s: [f32; 3]) -> Mat4 {
    let [x, y, z, w] = q;
    let (x2, y2, z2) = (x + x, y + y, z + z);
    let (xx, xy, xz) = (x * x2, x * y2, x * z2);
    let (yy, yz, zz) = (y * y2, y * z2, z * z2);
    let (wx, wy, wz) = (w * x2, w * y2, w * z2);
    [
        (1.0 - (yy + zz)) * s[0],
        (xy + wz) * s[0],
        (xz - wy) * s[0],
        0.0,
        (xy - wz) * s[1],
        (1.0 - (xx + zz)) * s[1],
        (yz + wx) * s[1],
        0.0,
        (xz + wy) * s[2],
        (yz - wx) * s[2],
        (1.0 - (xx + yy)) * s[2],
        0.0,
        t[0],
        t[1],
        t[2],
        1.0,
    ]
}

pub fn transform_point(m: &Mat4, v: [f32; 3]) -> [f32; 3] {
    [
        m[0] * v[0] + m[4] * v[1] + m[8] * v[2] + m[12],
        m[1] * v[0] + m[5] * v[1] + m[9] * v[2] + m[13],
        m[2] * v[0] + m[6] * v[1] + m[10] * v[2] + m[14],
    ]
}

/// Normal through the inverse transpose of the upper 3×3.
pub fn transform_normal(m: &Mat4, n: [f32; 3]) -> [f32; 3] {
    let a = [m[0], m[1], m[2]];
    let b = [m[4], m[5], m[6]];
    let c = [m[8], m[9], m[10]];
    // Rows of the inverse-transpose are the cross products of the columns.
    let bc = cross(b, c);
    let ca = cross(c, a);
    let ab = cross(a, b);
    let r = [
        bc[0] * n[0] + ca[0] * n[1] + ab[0] * n[2],
        bc[1] * n[0] + ca[1] * n[1] + ab[1] * n[2],
        bc[2] * n[0] + ca[2] * n[1] + ab[2] * n[2],
    ];
    let det = dot(a, bc);
    let r = if det < 0.0 { [-r[0], -r[1], -r[2]] } else { r };
    normalize3(r)
}

pub fn determinant3(m: &Mat4) -> f32 {
    let a = [m[0], m[1], m[2]];
    let b = [m[4], m[5], m[6]];
    let c = [m[8], m[9], m[10]];
    dot(a, cross(b, c))
}

pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn len3(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

pub fn normalize3(a: [f32; 3]) -> [f32; 3] {
    let l = len3(a);
    if l > 1e-12 && l.is_finite() {
        [a[0] / l, a[1] / l, a[2] / l]
    } else {
        [0.0, 1.0, 0.0]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    pub fn empty() -> Self {
        Aabb {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        }
    }

    pub fn of(points: &[[f32; 3]]) -> Self {
        let mut b = Aabb::empty();
        for p in points {
            b.add(*p);
        }
        b
    }

    pub fn add(&mut self, p: [f32; 3]) {
        #[allow(clippy::needless_range_loop)]
        for a in 0..3 {
            if p[a].is_finite() {
                self.min[a] = self.min[a].min(p[a]);
                self.max[a] = self.max[a].max(p[a]);
            }
        }
    }

    pub fn is_valid(&self) -> bool {
        (0..3).all(|a| self.min[a] <= self.max[a])
    }

    pub fn size(&self) -> [f32; 3] {
        if !self.is_valid() {
            return [0.0; 3];
        }
        [
            self.max[0] - self.min[0],
            self.max[1] - self.min[1],
            self.max[2] - self.min[2],
        ]
    }
}
