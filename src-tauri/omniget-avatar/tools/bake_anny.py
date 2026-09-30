"""Bake the Anny body (Apache-2.0 code, CC0 MakeHuman data) into the compact
binaries the city runtime loads from `static/city3d/anny/`.

Offline only; the app never runs Python. Needs the Anny package (naver/anny)
with torch, and its cache (`ANNY_CACHE_DIR`).

    ANNY_CACHE_DIR=... python bake_anny.py <out_dir> [catalog.json]

Per LOD (1229 and 369 vertices, the `notoes_collapse*` topologies) it writes:
- the triangle list and the skin weights on the 52-bone VRM humanoid skeleton
  (Anny's 104 bones folded into their nearest mapped ancestor, top 4 weights);
- the body in VRM T-pose (Y up, facing +Z, left at +X), as a PCA basis over the
  vertices and the joint heads: x = mean + basis^T (M c), where c are the exact
  Anny phenotype blend coefficients (the runtime ports the ~40-line mixer) and
  M = basis . B_phenotype, so the phenotype -> PCA table is exact, not fitted;
- a vertex part id and a coordinate along the part, for procedural clothing;
- on the 1229 LOD only: the curated local changes of the catalog (face) and
  the 52 ARKit facial actions, as sparse float16 vertex deltas.

The T-pose is a fixed rotation of each arm subtree about the upper arm head,
so the whole pipeline stays linear in the blend coefficients.
"""
import hashlib
import json
import math
import os
import struct
import sys
import time

import numpy as np
import torch
import anny
from anny.models.model_data import PHENOTYPE_VARIATIONS

OUT = sys.argv[1] if len(sys.argv) > 1 else "."
CATALOG = sys.argv[2] if len(sys.argv) > 2 else os.path.join(
    os.path.dirname(__file__), "..", "catalog", "catalog_v1.json")
K = {"1229": 16, "369": 12}
TOPO = {"1229": "notoes_collapse10pc", "369": "notoes_collapse3pc"}
SAMPLES = 2400
SEED = 7

# VRM humanoid bone -> Anny bone (same map as the F0 prototype).
VRM = [("hips", "root"), ("spine", "spine05"), ("chest", "spine03"),
       ("upperChest", "spine01"), ("neck", "neck01"), ("head", "head"),
       ("leftEye", "eye.L"), ("rightEye", "eye.R")]
for s, S in (("L", "left"), ("R", "right")):
    VRM += [(S + "UpperLeg", "upperleg01." + s), (S + "LowerLeg", "lowerleg01." + s),
            (S + "Foot", "foot." + s), (S + "Shoulder", "clavicle." + s),
            (S + "UpperArm", "upperarm01." + s), (S + "LowerArm", "lowerarm01." + s),
            (S + "Hand", "wrist." + s)]
    for f, F in ((1, "Thumb"), (2, "Index"), (3, "Middle"), (4, "Ring"), (5, "Little")):
        segs = ["Metacarpal", "Proximal", "Distal"] if f == 1 else ["Proximal", "Intermediate", "Distal"]
        for k, seg in enumerate(segs):
            VRM.append((S + F + seg, f"finger{f}-{k + 1}.{s}"))
assert len(VRM) == 52

PART_NAMES = ["head", "neck", "torso", "hips", "upperArm", "lowerArm", "hand",
              "upperLeg", "lowerLeg", "foot", "eye"]


def part_of(vrm):
    if vrm in ("head",):
        return 0
    if vrm == "neck":
        return 1
    if vrm in ("spine", "chest", "upperChest") or vrm.endswith("Shoulder"):
        return 2
    if vrm == "hips":
        return 3
    if vrm.endswith("UpperArm"):
        return 4
    if vrm.endswith("LowerArm"):
        return 5
    if vrm.endswith("Hand") or any(f in vrm for f in ("Thumb", "Index", "Middle", "Ring", "Little")):
        return 6
    if vrm.endswith("UpperLeg"):
        return 7
    if vrm.endswith("LowerLeg"):
        return 8
    if vrm.endswith("Foot"):
        return 9
    if vrm.endswith("Eye"):
        return 10
    raise ValueError(vrm)


# Anny: Z up, front -Y, left +X  ->  glTF/VRM: Y up, front +Z, left +X.
C = np.array([[1, 0, 0], [0, 0, 1], [0, -1, 0]], dtype=np.float64)


def phen_coefficients(ph6, anchors, mask):
    """Port of Anny._get_phenotype_blendshape_coefficients (default phenotypes,
    cupsize/firmness at 0.5, the three race weights at 1/3)."""
    vals = dict(zip(["gender", "age", "muscle", "weight", "height", "proportions"], ph6))
    vals.update(cupsize=0.5, firmness=0.5)
    phens = []
    for feat, keys in PHENOTYPE_VARIATIONS.items():
        if feat == "race":
            phens += [1 / 3] * 3
            continue
        a = anchors[feat]
        v = min(max(vals[feat], a[0]), a[-1])
        w = [0.0] * len(a)
        i = int(np.searchsorted(a, v, side="left"))
        i = min(max(i, 1), len(a) - 1)
        t = (v - a[i - 1]) / (a[i] - a[i - 1])
        w[i - 1] = 1 - t
        w[i] = t
        phens += w
    phens = np.array(phens)
    return np.prod(phens[None, :] * mask + (1 - mask), axis=1)


def bake_lod(tag, catalog, report):
    t0 = time.time()
    full = tag == "1229"
    m = anny.Anny(topology=TOPO[tag], local_changes="all" if full else "none",
                  facial_actions="all" if full else "none").to(dtype=torch.float64)
    labels = list(m.bone_labels)
    parents = [int(p) for p in m.bone_parents]
    T = m.template_vertices.double().numpy()            # V x 3
    B = m.blendshapes.double().numpy()                  # S x V x 3
    TH = m.template_bone_heads.double().numpy()         # 104 x 3
    BH = m.bone_heads_blendshapes.double().numpy()      # S x 104 x 3
    faces = m.faces.numpy().astype(np.int64)
    W9 = m.vertex_bone_weights.double().numpy()
    J9 = m.vertex_bone_indices.numpy().astype(np.int64)
    mask = m.stacked_phenotype_blend_shapes_mask.double().numpy()   # 624 x 26
    n_ph = mask.shape[0]
    V = T.shape[0]
    anchors = {k: m.anchors[k].double().numpy() for k in
               ["age", "gender", "muscle", "weight", "height", "proportions", "cupsize", "firmness"]}

    # Fold the 104 bones into the 52 VRM bones.
    vrm_names = [v for v, _ in VRM]
    anny_to_vrm = {a: i for i, (_, a) in enumerate(VRM)}
    fold = []
    for i, b in enumerate(labels):
        j = i
        while labels[j] not in anny_to_vrm:
            j = parents[j]
            assert j >= 0, b
        fold.append(anny_to_vrm[labels[j]])
    fold = np.array(fold)
    vrm_parent = []
    for v, a in VRM:
        j = parents[labels.index(a)]
        while j >= 0 and labels[j] not in anny_to_vrm:
            j = parents[j]
        vrm_parent.append(anny_to_vrm[labels[j]] if j >= 0 else -1)
    heads_idx = np.array([labels.index(a) for _, a in VRM])

    W = np.zeros((V, 52))
    for k in range(W9.shape[1]):
        np.add.at(W, (np.arange(V), fold[J9[:, k]]), W9[:, k])
    order = np.argsort(-W, axis=1)[:, :4]
    W4 = np.take_along_axis(W, order, 1)
    W4 /= W4.sum(1, keepdims=True)
    J4 = order

    # T-pose: one fixed rotation per arm, taking the mean upper arm direction to +-X.
    c_mid = phen_coefficients([0.5] * 6, anchors, mask)
    heads_mid = TH + np.tensordot(c_mid, BH[:n_ph], 1)
    sub = {}
    rot = {}
    for side, sx in (("L", 1.0), ("R", -1.0)):
        ua = labels.index("upperarm01." + side)
        la = labels.index("lowerarm01." + side)
        d = heads_mid[la] - heads_mid[ua]
        d /= np.linalg.norm(d)
        tgt = np.array([sx, 0.0, 0.0])
        ax = np.cross(d, tgt)
        s = np.linalg.norm(ax)
        ang = math.atan2(s, float(d @ tgt))
        ax /= s
        Kx = np.array([[0, -ax[2], ax[1]], [ax[2], 0, -ax[0]], [-ax[1], ax[0], 0]])
        R = np.eye(3) + math.sin(ang) * Kx + (1 - math.cos(ang)) * Kx @ Kx
        rot[side] = (R, ua, math.degrees(ang))
        members = set()
        for i in range(len(labels)):
            j = i
            while j >= 0:
                if j == ua:
                    members.add(i)
                    break
                j = parents[j]
        sub[side] = members
    arm_vrm = {}
    for side in ("L", "R"):
        for i in sub[side]:
            if labels[i] in anny_to_vrm:
                arm_vrm[anny_to_vrm[labels[i]]] = side
    # per-vertex blend of the arm rotations (linear in the rest data)
    wL = np.zeros(V)
    wR = np.zeros(V)
    for k in range(4):
        side = np.array([arm_vrm.get(int(j), "") for j in J4[:, k]])
        wL += np.where(side == "L", W4[:, k], 0)
        wR += np.where(side == "R", W4[:, k], 0)
    head_side = {}
    for i in range(len(labels)):
        for side in ("L", "R"):
            if i in sub[side]:
                head_side[i] = side

    def tpose(verts, heads, affine):
        """verts V x 3, heads 104 x 3 (Anny frame). affine=False for deltas."""
        out_v = verts.copy()
        out_h = heads.copy()
        for side, wv in (("L", wL), ("R", wR)):
            R, ua, _ = rot[side]
            p = heads[ua]
            rv = (verts - p) @ R.T + p
            out_v += wv[:, None] * (rv - verts)
            for i, s in head_side.items():
                if s == side:
                    out_h[i] = (heads[i] - p) @ R.T + p
        return out_v, out_h

    def to_vec(verts, heads):
        return np.concatenate([(verts @ C.T).reshape(-1), (heads[heads_idx] @ C.T).reshape(-1)])

    base_v, base_h = tpose(T, TH, True)
    base = to_vec(base_v, base_h)
    D = V * 3 + 52 * 3
    # phenotype blend shapes after T-pose (the op is linear; deltas carry their pivot delta)
    Bp = np.zeros((n_ph, D))
    for i in range(n_ph):
        v, h = tpose(T + B[i], TH + BH[i], True)
        Bp[i] = to_vec(v, h) - base

    # Check the ported mixer against Anny's own coefficients.
    rng = np.random.default_rng(SEED)
    P = rng.random((SAMPLES, 6))
    P[:64] = np.array(np.meshgrid(*[[0, 1]] * 6)).reshape(6, -1).T  # corners
    Cs = np.stack([phen_coefficients(p, anchors, mask) for p in P])
    with torch.no_grad():
        ref = m._get_phenotype_blendshape_coefficients(
            torch.tensor(P[:200]), torch.zeros(200, len(m.local_change_labels), dtype=torch.float64),
            torch.zeros(200, len(m.facial_action_labels), dtype=torch.float64)).numpy()[:, :n_ph]
    port_err = float(np.abs(ref - Cs[:200]).max())
    X = base[None, :] + Cs @ Bp
    mu = X.mean(0)
    U, Sv, Vt = np.linalg.svd(X - mu, full_matrices=False)
    k = K[tag]
    Vk = Vt[:k]
    Vk16 = Vk.astype(np.float16)
    Mz = Vk @ Bp.T                         # k x 624: exact phenotype -> PCA table
    z0 = Vk @ (base - mu)
    mean = mu + Vk.T @ z0                  # so that x = mean + Vk^T (Mz c)
    rec = mean[None, :] + (Cs @ Mz.T) @ Vk16.astype(np.float64)
    err = np.linalg.norm((rec - X).reshape(SAMPLES, -1, 3), axis=2)
    # And against Anny's own forward (rest vertices, before the T-pose) at a few points.
    with torch.no_grad():
        full_c = m._get_phenotype_blendshape_coefficients(
            torch.tensor(P[:8]), torch.zeros(8, len(m.local_change_labels), dtype=torch.float64),
            torch.zeros(8, len(m.facial_action_labels), dtype=torch.float64))
        ref_rest = m.get_rest_vertices(full_c).numpy()
    mine_rest = T[None] + np.einsum("bs,svc->bvc", Cs[:8], B[:n_ph])
    fwd_err = float(np.abs(ref_rest - mine_rest).max())

    # Parts and the coordinate along each part (for clothing masks), on the mid body.
    mid = (base + c_mid @ Bp)
    mv = mid[:V * 3].reshape(V, 3)
    mh = mid[V * 3:].reshape(52, 3)
    dom = J4[:, 0]
    part = np.array([part_of(vrm_names[j]) for j in dom])
    # eyes: vertices dominated by head that sit right on the eye bone
    for e in (6, 7):
        d = np.linalg.norm(mv - mh[e], axis=1)
        part[(d < 0.016) & ((part == 0) | (part == 10))] = 10
    along = np.zeros(V)
    child_of = {}
    for j, p in enumerate(vrm_parent):
        if p >= 0 and p not in child_of:
            child_of[p] = j
    prefer = {"leftUpperArm": "leftLowerArm", "rightUpperArm": "rightLowerArm",
              "leftLowerArm": "leftHand", "rightLowerArm": "rightHand",
              "leftUpperLeg": "leftLowerLeg", "rightUpperLeg": "rightLowerLeg",
              "leftLowerLeg": "leftFoot", "rightLowerLeg": "rightFoot",
              "hips": "spine", "spine": "chest", "chest": "upperChest",
              "upperChest": "neck", "neck": "head"}
    for j, name in enumerate(vrm_names):
        sel = dom == j
        if not sel.any():
            continue
        a = mh[j]
        if name in prefer:
            b = mh[vrm_names.index(prefer[name])]
        elif j in child_of:
            b = mh[child_of[j]]
        else:
            b = a + np.array([0, 0.1, 0])
        axis = b - a
        L2 = float(axis @ axis) or 1.0
        along[sel] = np.clip(((mv[sel] - a) @ axis) / L2, 0, 1)

    sections = {}
    blob = bytearray()

    def put(name, arr):
        arr = np.ascontiguousarray(arr)
        while len(blob) % 8:
            blob.append(0)
        sections[name] = {"offset": len(blob), "length": int(arr.nbytes),
                          "dtype": str(arr.dtype), "shape": list(arr.shape)}
        blob.extend(arr.tobytes())

    put("indices", faces.astype(np.uint16).reshape(-1))
    put("joints", J4.astype(np.uint8))
    wq = np.round(W4 * 255).astype(np.int32)
    wq[:, 0] += 255 - wq.sum(1)
    put("weights", wq.astype(np.uint8))
    put("part", part.astype(np.uint8))
    put("along", np.round(along * 255).astype(np.uint8))
    put("mean", mean.astype(np.float32))
    put("basis", Vk16)
    put("pheno_table", Mz.astype(np.float32))
    put("pheno_mask", mask.astype(np.uint8))

    extra = {}
    if full:
        lab = list(m.blendshape_labels)
        n_fa = len(m.facial_action_labels)

        def sparse(delta_v, thr):
            nz = np.where(np.abs(delta_v).max(1) > thr)[0]
            return nz.astype(np.uint16), delta_v[nz].astype(np.float16)

        def pack(name, entries):
            idx = []
            val = []
            table = []
            off = 0
            for e_idx, e_val in entries:
                table.append([off, len(e_idx)])
                idx.append(e_idx)
                val.append(e_val.reshape(-1))
                off += len(e_idx)
            put(name + "_table", np.array(table, dtype=np.uint32).reshape(-1, 2))
            put(name + "_index", np.concatenate(idx) if idx else np.zeros(0, np.uint16))
            put(name + "_delta", np.concatenate(val) if val else np.zeros(0, np.float16))

        # facial actions: blend shapes n_ph .. n_ph+52
        fa = []
        for i in range(n_fa):
            s = n_ph + i
            v, _ = tpose(T + B[s], TH + BH[s], True)
            fa.append(sparse(((v - base_v) @ C.T), 2e-5))
        pack("expr", fa)
        extra["expressions"] = list(m.facial_action_labels)
        # curated local changes: each catalog target sums its labels, both signs
        lc = list(m.local_change_labels)
        lo = []
        for tgt in catalog["face_targets"]:
            for sign in (0, 1):
                dv = np.zeros((V, 3))
                for l in tgt["labels"]:
                    s = n_ph + n_fa + 2 * lc.index(l) + sign
                    assert lab[s].startswith("local_change"), lab[s]
                    v, _ = tpose(T + B[s], TH + BH[s], True)
                    dv += (v - base_v) @ C.T
                lo.append(sparse(dv, 2e-5))
        pack("local", lo)
        extra["face_targets"] = [t["id"] for t in catalog["face_targets"]]
        extra["expr_nonzero"] = int(sum(len(e[0]) for e in fa))
        extra["local_nonzero"] = int(sum(len(e[0]) for e in lo))

    fname = f"anny_lod{tag}.bin"
    with open(os.path.join(OUT, fname), "wb") as f:
        f.write(blob)
    arm_deg = {s: round(rot[s][2], 2) for s in rot}
    report[tag] = {
        "k": k, "vertices": V, "triangles": int(faces.shape[0]),
        "pca_err_mean_mm": round(float(err.mean()) * 1000, 3),
        "pca_err_max_mm": round(float(err.max()) * 1000, 3),
        "mixer_port_max_abs_err": port_err, "linear_vs_anny_forward_max_m": fwd_err,
        "tpose_arm_rotation_deg": arm_deg, "seconds": round(time.time() - t0, 1),
    }
    return {
        "file": fname, "bytes": len(blob), "sha256": hashlib.sha256(blob).hexdigest(),
        "vertices": V, "triangles": int(faces.shape[0]), "k": k, "sections": sections,
        "pca": {"err_mean_mm": report[tag]["pca_err_mean_mm"], "err_max_mm": report[tag]["pca_err_max_mm"],
                "samples": SAMPLES},
        **extra,
    }, {"bones": [{"name": v, "parent": p} for (v, _), p in zip(VRM, vrm_parent)],
        "anchors": {k2: [float(x) for x in a] for k2, a in anchors.items()}}


def main():
    os.makedirs(OUT, exist_ok=True)
    catalog_bytes = open(CATALOG, "rb").read()
    catalog = json.loads(catalog_bytes)
    report = {}
    lods = {}
    shared = None
    for tag in ("1229", "369"):
        lods[tag], shared = bake_lod(tag, catalog, report)
    manifest = {
        "schema": "omniget.anny.bake/1",
        "generator": "omniget-avatar/tools/bake_anny.py",
        "anny_version": getattr(anny, "__version__", "0.6"),
        "units": "meters, Y up, facing +Z, left +X, VRM T-pose; floor is applied at runtime",
        "catalog_sha256": hashlib.sha256(catalog_bytes).hexdigest(),
        "bones": shared["bones"],
        "parts": PART_NAMES,
        "phenotype": {
            "labels": ["gender", "age", "muscle", "weight", "height", "proportions"],
            "variations": [[f, k] for f, ks in PHENOTYPE_VARIATIONS.items() for k in ks],
            "anchors": shared["anchors"],
            "fixed": {"cupsize": 0.5, "firmness": 0.5, "race": 1 / 3},
        },
        "layout": "x = mean + basis^T (pheno_table . c); c_i = prod_j (mask_ij ? phens_j : 1); "
                  "x = [V*3 vertex coords, 52*3 joint heads]",
        "lods": lods,
    }
    with open(os.path.join(OUT, "manifest.json"), "w") as f:
        json.dump(manifest, f, separators=(",", ":"))
    print(json.dumps(report, indent=1))


if __name__ == "__main__":
    main()
