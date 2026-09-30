// Shared builders of the code-generated kit (merged before every style and
// layer script). Metres; x = east, y = up, z = south. The contract is the
// mesher's catalogue (omniget_citygen::mesher::keys::kit_catalog):
// - `dual` modules live in the 2×2×3 m dual cell centred on the origin; in
//   the canonical orientation the outside faces north (−z) and the building
//   sits on the south side:
//     parede = south half, convexo = south-east quadrant,
//     concavo = all but north-west, diagonal = north-west + south-east;
// - `edge` pieces (porta, bay, fences…) run along x on the facade line z = 0,
//   with the outside toward −z;
// - `tile` pieces are centred on a tile.
// Every builder takes the style map `s` returned by S() (style.rhai).
//
// Budget (VISUAL.md): storeys 8–80 triangles, roofs up to 56, trees around
// 550. Corners get one chamfer segment, clay tiles are the toon material's
// brush noise (no rib geometry), windows are flat overlays. `lod()` is 1 when
// the far variant (`<key>#lod1`) is built: mouldings, frames, shutters and
// small parts are left out.

// Plan outline (points [x, z]) of the building part of a dual cell.
// g grows the facades outward (negative = inward), ext pushes the cell-edge
// sides outward (for cutters), r is the radius of the rounded outer corner.
fn fp(kase, g, ext, r) {
    let e = 1.0 + ext;
    switch kase {
        "parede" => [[-e, -g], [e, -g], [e, e], [-e, e]],
        "concavo" => [[-e, -g], [-g, -g], [-g, -e], [e, -e], [e, e], [-e, e]],
        "cheio" => [[-e, -e], [e, -e], [e, e], [-e, e]],
        _ => {
            // Soft corners: one mid point on the arc for wide radii, a single
            // chamfer otherwise (and always far away).
            let rr = r + g;
            let pts = [[e, -g], [e, e], [-g, e], [-g, r]];
            if r >= 0.4 && lod() == 0 {
                pts.push([r - rr * 0.7071068, r - rr * 0.7071068]);
            }
            pts.push([r, -g]);
            pts
        },
    }
}

// The outline extruded from y to y + h.
fn prism(kase, g, ext, r, h, y, mat) {
    if kase == "diagonal" {
        let a = prism("convexo", g, ext, r, h, y, mat);
        return union([a, turn(a, 180)]);
    }
    extrude(fp(kase, g, ext, r), #{ h: h, axis: "y", at: [0.0, y, 0.0], mat: mat })
}

// A band that wraps the facades only (plinth, cornice, corner boards).
fn band(kase, s, g, h, y, mat) {
    if kase == "cheio" {
        return ();
    }
    prism(kase, g, 0.0, s.corner, h, y, mat)
}

// One storey of wall mass with the style's bands; `ground` adds the plinth.
fn storey(s, kase, ground) {
    if kase == "cheio" {
        return [box([2.0, 3.0, 2.0], #{ mat: "wall", bevel: 0.0 })];
    }
    let parts = [prism(kase, 0.0, 0.0, s.corner, 3.0, 0.0, "wall")];
    if lod() > 0 {
        return parts;
    }
    if ground && s.plinth {
        parts.push(band(kase, s, 0.05, 0.4, 0.0, "stone"));
    }
    if !ground && s.cornice {
        parts.push(band(kase, s, 0.08, 0.16, 2.74, "trim"));
    }
    parts
}

// Vertical board wrapped around the rounded corner of `convexo`.
fn corner_board(s) {
    let b = prism("convexo", 0.04, 0.0, s.corner, 3.0, 0.0, s.corner_mat);
    intersect(b, box([1.0, 3.0, 1.0], #{ mat: s.corner_mat, bevel: 0.0 }))
}

fn drainpipe(x, z) {
    cyl(#{ r: 0.05, h: 3.0, at: [x, 0.0, z], mat: "metal", seg: 4, bevel: 0.0 })
}

// Plain wall storey (openings are `porta` / `bay` overlays). Variants:
// 0 plain; 1 corner board / pilaster / drainpipe; 2 string course band.
fn wall_module(s, kase, v, ground) {
    // Two corners in one cell: corner boards replace the bands there.
    let parts = if v == 1 && kase == "diagonal" {
        [prism(kase, 0.0, 0.0, s.corner, 3.0, 0.0, "wall")]
    } else {
        storey(s, kase, ground)
    };
    if lod() > 0 {
        return union(parts);
    }
    if v == 1 {
        switch kase {
            "convexo" => parts.push(corner_board(s)),
            "diagonal" => {
                let c = corner_board(s);
                parts.push(c);
                parts.push(turn(c, 180));
            },
            "concavo" => parts.push(drainpipe(-0.12, -0.12)),
            _ => parts.push(box([0.3, 3.0, 0.08], #{ at: [0.0, 0.0, -0.03], mat: s.corner_mat, bevel: 0.0 })),
        }
    } else if v == 2 && kase != "cheio" && kase != "diagonal" {
        parts.push(band(kase, s, 0.04, 0.1, if ground { 2.9 } else { 0.0 }, "trim"));
    }
    union(parts)
}

// ── roofs: height fields y(x, z) in the canonical frame ──

// Solid under the plane y = k·z over x ∈ [xa, xb], z ∈ [za, zb], down to b.
fn ramp(k, za, zb, xa, xb, b) {
    extrude([[za, b], [zb, b], [zb, k * zb], [za, k * za]],
        #{ h: xb - xa, axis: "x", at: [(xa + xb) / 2.0, 0.0, 0.0], mat: "roof" })
}

// Slab of vertical thickness t under the plane y = k·z (for rake overhangs).
fn sheet(k, t, za, zb, xa, xb) {
    extrude([[za, k * za - t], [zb, k * zb - t], [zb, k * zb], [za, k * za]],
        #{ h: xb - xa, axis: "x", at: [(xa + xb) / 2.0, 0.0, 0.0], mat: "roof" })
}

// Fascia boards under the eave edge, per case.
fn fascia(kase, e, y) {
    let t = 0.08;
    let hx = |x0, x1, z| box([x1 - x0, 0.1, t], #{ at: [(x0 + x1) / 2.0, y, z + t / 2.0], mat: "trim", bevel: 0.0 });
    let hz = |z0, z1, x| box([t, 0.1, z1 - z0], #{ at: [x + t / 2.0, y, (z0 + z1) / 2.0], mat: "trim", bevel: 0.0 });
    switch kase {
        "parede" => [hx.call(-1.0, 1.0, -e)],
        "convexo" => [hx.call(-e, 1.0, -e), hz.call(-e + t, 1.0, -e)],
        "concavo" => [hx.call(-1.0, -e + t, -e), hz.call(-1.0, -e, -e)],
        _ => [],
    }
}

fn fascia_x(x0, x1, z, y) {
    [box([x1 - x0, 0.1, 0.08], #{ at: [(x0 + x1) / 2.0, y, z + 0.04], mat: "trim", bevel: 0.0 })]
}

// Gable wall under a roof line, in the facade plane (z 0..0.2), down to −4.5.
fn gable_fill(top) {
    let pts = [[-1.0, -4.5], [1.0, -4.5]];
    for p in top {
        pts.push(p);
    }
    extrude(pts, #{ h: 0.2, axis: "z", at: [0.0, 0.0, 0.1], mat: "wall" })
}

// South-east quadrant, slope y = k·z, eave to the north, gable facing west.
fn canto_beiral_n(s, k, t, e) {
    let roof = sheet(k, t, -e, 1.0, -0.2, 1.0);
    let fill = extrude([[0.0, -4.5], [1.0, -4.5], [1.0, k], [0.0, 0.0]],
        #{ h: 0.2, axis: "x", at: [0.1, 0.0, 0.0], mat: "wall" });
    if lod() > 0 { return group([roof, fill]); }
    group([roof, fill] + fascia_x(-0.2, 1.0, -e, -e * k - t - 0.05))
}

// Three quadrants (north-west empty), slope y = k·z, eave on the facade
// line z = 0 for x < 0, gable facing west on x = 0 for z < 0.
fn concavo_beiral_n(s, k, t, e) {
    let plan = extrude([[-1.0, -e], [-0.2, -e], [-0.2, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]],
        #{ h: 8.0, axis: "y", at: [0.0, -4.0, 0.0], mat: "roof" });
    let roof = intersect(sheet(k, t, -1.0, 1.0, -1.0, 1.0), plan);
    let fill = extrude([[-1.0, -4.5], [0.0, -4.5], [0.0, 0.0], [-1.0, -k]],
        #{ h: 0.2, axis: "x", at: [0.1, 0.0, 0.0], mat: "wall" });
    if lod() > 0 { return group([roof, fill]); }
    group([roof, fill] + fascia_x(-1.0, -0.2, -e, -e * k - t - 0.05))
}

// A roof piece of the catalogue (`duas-aguas-*`, `quatro-aguas-*`,
// `conico-*`). Slope 1:2 (0.5 m per metre); conical caps 2:1.
fn roof_piece(s, name) {
    let conico = name.starts_with("conico");
    let k = if conico { 2.0 } else { 0.5 };
    let e = if conico { 0.15 } else { s.eave };
    let t = s.thick;
    let lowb = -e * k - t;
    let deep = -k - 0.3;
    let full = ramp(k, -1.0, 1.0, -1.0, 1.0, deep);
    // Fascia boards and ridge caps only up close.
    let near = lod() == 0;
    let fas = |kase| if near { fascia(kase, e, lowb - 0.05) } else { [] };
    switch name {
        "duas-aguas-beiral" | "quatro-aguas-beiral" | "conico-beiral" => {
            union([ramp(k, -e, 1.0, -1.0, 1.0, lowb)] + fas.call("parede"))
        },
        "duas-aguas-rampa" | "quatro-aguas-rampa" => full,
        "duas-aguas-cumeeira" | "quatro-aguas-cumeeira" => {
            let roof = intersect(full, turn(full, 180));
            if !near { return roof; }
            let ridge = extrude([[-0.12, -0.08], [0.12, -0.08], [0.12, 0.0], [0.0, 0.08], [-0.12, 0.0]],
                #{ h: 2.0, axis: "x", mat: s.ridge_mat });
            union([roof, ridge])
        },
        "quatro-aguas-canto" | "conico-canto" => {
            let r = ramp(k, -e, 1.0, -1.0, 1.0, lowb);
            union([intersect(r, turn(r, 270))] + fas.call("convexo"))
        },
        "quatro-aguas-concavo" => {
            let r = ramp(k, -e, 1.0, -1.0, 1.0, lowb);
            union([r, turn(r, 270)] + fas.call("concavo"))
        },
        "quatro-aguas-diagonal" | "duas-aguas-diagonal" => {
            let r = ramp(k, -e, 1.0, -1.0, 1.0, lowb);
            let c = intersect(r, turn(r, 270));
            union([c, turn(c, 180)])
        },
        "quatro-aguas-espigao" => intersect(full, turn(full, 270)),
        "quatro-aguas-topo" => intersect(turn(full, 270), intersect(full, turn(full, 180))),
        "quatro-aguas-pico" | "conico-pico" => {
            let p = intersect(intersect(full, turn(full, 90)), intersect(turn(full, 180), turn(full, 270)));
            if conico && near {
                group([p, cyl(#{ r: 0.04, h: 0.8, mat: "metal", seg: 4, bevel: 0.0 }),
                       blob(#{ r: 0.12, at: [0.0, 0.7, 0.0], mat: "metal", seg: 4 })])
            } else {
                p
            }
        },
        "duas-aguas-oitao-leste" => union([turn(sheet(k, t, -1.0, 1.0, -1.0, 0.2), 270),
                                          gable_fill([[1.0, 0.5], [-1.0, -0.5]])]),
        "duas-aguas-oitao-oeste" => union([turn(sheet(k, t, -1.0, 1.0, -0.2, 1.0), 90),
                                          gable_fill([[1.0, -0.5], [-1.0, 0.5]])]),
        "duas-aguas-oitao-topo" => {
            // Both slopes as one extruded ∧ profile.
            let prof = [[-1.0, -k - t], [0.0, -t], [1.0, -k - t], [1.0, -k], [0.0, 0.0], [-1.0, -k]];
            let a = turn(extrude(prof, #{ h: 1.2, axis: "x", at: [-0.4, 0.0, 0.0], mat: "roof" }), 270);
            let parts = [a, gable_fill([[1.0, -0.5], [0.0, 0.0], [-1.0, -0.5]])];
            if near {
                parts.push(box([0.2, 0.1, 1.2], #{ at: [0.0, -0.04, 0.4], mat: s.ridge_mat, bevel: 0.0 }));
            }
            group(parts)
        },
        "duas-aguas-canto-beiral-n" => canto_beiral_n(s, k, t, e),
        "duas-aguas-canto-beiral-o" => mirror(canto_beiral_n(s, k, t, e), "xz"),
        "duas-aguas-concavo-beiral-n" => concavo_beiral_n(s, k, t, e),
        "duas-aguas-concavo-beiral-o" => mirror(concavo_beiral_n(s, k, t, e), "xz"),
        _ => (),
    }
}

// Flat roof with a parapet (plano-<case>).
fn flat_piece(s, kase) {
    let h = s.parapet;
    if kase == "cheio" {
        return box([2.0, 0.08, 2.0], #{ mat: "roof", bevel: 0.0 });
    }
    let rim = subtract(prism(kase, 0.0, 0.0, s.corner, h, 0.0, "wall"),
                       prism(kase, -0.2, 0.2, s.corner, h + 0.2, 0.08, "roof"));
    let parts = [rim];
    if s.coping && lod() == 0 && kase != "diagonal" {
        parts.push(subtract(band(kase, s, 0.04, 0.06, h, "trim"), prism(kase, -0.16, 0.2, s.corner, 0.3, h - 0.05, "trim")));
    }
    union(parts)
}

// ── overlays on a tile edge (facade plane z = 0, outside toward −z) ──

fn frame(w, h, y, depth, mat) {
    subtract(box([w + 0.2, h + 0.2, depth], #{ at: [0.0, y - 0.1, -depth / 2.0], mat: mat, edges: "z", bevel: 0.02 }),
             box([w, h, depth + 0.2], #{ at: [0.0, y, 0.0], mat: mat, bevel: 0.0 }))
}

// Doors: simples 1.0 × 2.2, dupla 1.6 × 2.8 with a pediment, loja 1.6 × 2.4
// with a glass transom and an awning. Variant 1 is arched (when the style
// has arched doors) or glazed. Adds the `enter` anchor outside. Far away a
// door is its leaf, its frame plate and the step.
fn door_piece(s, kind, v) {
    let w = if kind == "simples" { 1.0 } else { 1.6 };
    let h = switch kind { "simples" => 2.2, "dupla" => 2.8, _ => 2.4 };
    let arched = v == 1 && s.arch_door && kind != "loja";
    let near = lod() == 0;
    let parts = [];
    if arched {
        // Frame plate on the wall, leaf proud of it.
        parts.push(arch(#{ w: w + 0.24, h: h + 0.12, d: 0.04, at: [0.0, 0.0, -0.02], mat: s.frame_mat, seg: 6 }));
        parts.push(arch(#{ w: w - 0.05, h: h, d: 0.04, at: [0.0, 0.0, -0.06], mat: "door", seg: 6 }));
    } else {
        let leaf_h = if kind == "loja" { h - 0.5 } else { h };
        parts.push(box([w + 0.2, h + 0.1, 0.04], #{ at: [0.0, 0.0, -0.02], mat: s.frame_mat, bevel: 0.0 }));
        if kind == "dupla" && near {
            for x in [-w / 4.0, w / 4.0] {
                parts.push(box([w / 2.0 - 0.06, leaf_h - 0.02, 0.04], #{ at: [x, 0.0, -0.06], mat: "door", bevel: 0.0 }));
            }
        } else {
            parts.push(box([w - 0.06, leaf_h - 0.02, 0.04], #{ at: [0.0, 0.0, -0.06], mat: "door", bevel: 0.0 }));
        }
        if (kind == "loja" || v == 1) && near {
            let gh = if kind == "loja" { 0.45 } else { 0.4 };
            parts.push(box([w - 0.1, gh, 0.03], #{ at: [0.0, h - gh - 0.02, -0.055], mat: "glass", bevel: 0.0 }));
        }
    }
    if kind == "dupla" && near {
        parts.push(extrude([[-(w / 2.0 + 0.3), 0.0], [w / 2.0 + 0.3, 0.0], [0.0, 0.45]],
            #{ h: 0.16, axis: "z", at: [0.0, h + 0.1, -0.08], mat: s.frame_mat }));
    }
    if kind == "loja" {
        parts.push(wedge([w + 0.4, 0.35, 0.6], #{ at: [0.0, h + 0.15, -0.3], mat: "cloth", rot: 180 }));
    }
    parts.push(box([w + 0.4, 0.15, 0.35], #{ at: [0.0, 0.0, -0.175], mat: "stone", bevel: 0.0 }));
    anchor("enter", #{ at: [0.0, 0.0, -0.9], face: 180 });
    union(parts)
}

// Bays are flat overlays on the facade: a frame plate on the wall and the
// glass (or leaf) proud of it, no cut into the wall. janela 0.9 × 1.2 (sill
// 0.9), vitrine 1.6 × 2.2 (from 0.3), sacada (door-window 1.0 × 2.2 and a
// balcony to z = −0.8). Far away a window is only its glass.
fn bay_piece(s, kind, v) {
    let near = lod() == 0;
    let parts = [];
    switch kind {
        "janela" => {
            let arched = v == 1 && s.arch_win;
            if !near {
                return if arched {
                    arch(#{ w: 1.0, h: 1.35, d: 0.04, at: [0.0, 0.85, -0.02], mat: "glass", seg: 4 })
                } else {
                    box([1.0, 1.3, 0.04], #{ at: [0.0, 0.85, -0.02], mat: "glass", bevel: 0.0 })
                };
            }
            if arched {
                parts.push(arch(#{ w: 1.1, h: 1.4, d: 0.03, at: [0.0, 0.85, -0.015], mat: s.frame_mat, seg: 6 }));
                parts.push(arch(#{ w: 0.9, h: 1.3, d: 0.02, at: [0.0, 0.9, -0.04], mat: "glass", seg: 6 }));
            } else {
                parts.push(box([1.1, 1.4, 0.03], #{ at: [0.0, 0.8, -0.015], mat: s.frame_mat, bevel: 0.0 }));
                parts.push(box([0.9, 1.2, 0.02], #{ at: [0.0, 0.9, -0.04], mat: "glass", bevel: 0.0 }));
            }
            parts.push(box([1.15, 0.08, 0.18], #{ at: [0.0, 0.8, -0.09], mat: s.frame_mat, bevel: 0.0 }));
            if s.shutters && !arched {
                for x in [-0.8, 0.8] {
                    parts.push(box([0.45, 1.2, 0.04], #{ at: [x, 0.9, -0.02], mat: s.shutter_mat, bevel: 0.0 }));
                }
            }
        },
        "vitrine" => {
            if near {
                parts.push(box([1.8, 2.4, 0.03], #{ at: [0.0, 0.2, -0.015], mat: s.frame_mat, bevel: 0.0 }));
                parts.push(box([1.6, 2.2, 0.02], #{ at: [0.0, 0.3, -0.04], mat: "glass", bevel: 0.0 }));
                parts.push(box([0.05, 2.2, 0.03], #{ at: [0.0, 0.3, -0.065], mat: s.frame_mat, bevel: 0.0 }));
            } else {
                parts.push(box([1.7, 2.3, 0.04], #{ at: [0.0, 0.25, -0.02], mat: "glass", bevel: 0.0 }));
            }
            if v == 1 {
                parts.push(box([1.9, 0.4, 0.08], #{ at: [0.0, 2.65, -0.04], mat: "wood", bevel: 0.0 }));
            } else {
                parts.push(wedge([1.9, 0.4, 0.8], #{ at: [0.0, 2.6, -0.4], mat: "cloth", rot: 180 }));
                if near {
                    parts.push(box([1.9, 0.15, 0.04], #{ at: [0.0, 2.45, -0.8], mat: "cloth", bevel: 0.0 }));
                }
            }
        },
        _ => {
            parts.push(box([1.2, 2.3, 0.03], #{ at: [0.0, 0.0, -0.015], mat: s.frame_mat, bevel: 0.0 }));
            parts.push(box([1.0, 2.2, 0.03], #{ at: [0.0, 0.0, -0.045], mat: "door", bevel: 0.0 }));
            parts.push(box([1.6, 0.12, 0.8], #{ at: [0.0, -0.02, -0.4], mat: s.balcony_mat, bevel: 0.0 }));
            let rail = if v == 1 { "wood" } else { s.rail_mat };
            if !near {
                // A solid balustrade reads the same from afar.
                parts.push(box([1.6, 0.9, 0.05], #{ at: [0.0, 0.1, -0.775], mat: rail, bevel: 0.0 }));
                return union(parts);
            }
            parts.push(box([0.8, 1.1, 0.02], #{ at: [0.0, 1.0, -0.07], mat: "glass", bevel: 0.0 }));
            parts.push(box([1.6, 0.06, 0.06], #{ at: [0.0, 1.0, -0.77], mat: rail, bevel: 0.0 }));
            for x in [-0.75, 0.0, 0.75] {
                parts.push(box([0.05, 0.9, 0.05], #{ at: [x, 0.1, -0.77], mat: rail, bevel: 0.0 }));
            }
            for x in [-0.77, 0.77] {
                parts.push(box([0.05, 0.06, 0.8], #{ at: [x, 1.0, -0.4], mat: rail, bevel: 0.0 }));
            }
        },
    }
    union(parts)
}

// Seam between two buildings: a pilaster through the origin with a drainpipe.
fn joint_piece(s) {
    let p = box([0.3, 3.0, 0.3], #{ mat: s.corner_mat, bevel: 0.0 });
    if lod() > 0 { return p; }
    union([p, drainpipe(0.2, -0.2)])
}

// Tower storey: stone quoins and a narrow slit in the middle of the face.
// Variant 1 adds a top band and a small round-headed opening instead.
fn tower_module(s, kase, v) {
    let parts = [prism(kase, 0.0, 0.0, s.corner, 3.0, 0.0, "wall")];
    if lod() > 0 {
        return parts[0];
    }
    let has_face = kase == "parede" || kase == "convexo";
    let at = if kase == "convexo" { 0.7 } else { 0.0 };
    if has_face {
        // Openings are dark glass overlays on the face, not cuts.
        if v == 1 {
            parts.push(arch(#{ w: 0.5, h: 0.6, d: 0.04, at: [at, 1.4, -0.02], mat: "glass", seg: 4 }));
            parts.push(band(kase, s, 0.06, 0.18, 2.8, "trim"));
        } else {
            parts.push(box([0.15, 1.0, 0.04], #{ at: [at, 1.0, -0.02], mat: "glass", bevel: 0.0 }));
        }
    }
    // Stone quoins on the outer corner (one course less on the double
    // corner of `diagonal`). Grouped: they sit on the wall, no boolean.
    if kase == "convexo" || kase == "diagonal" {
        let q = [];
        let rows = if kase == "diagonal" { [0.0] } else { [0.0, 1.5] };
        for y in rows {
            q.push(box([0.5, 0.45, 0.1], #{ at: [0.45, y, -0.04], mat: "stone", bevel: 0.0 }));
            q.push(box([0.1, 0.45, 0.5], #{ at: [-0.04, y + 0.75, 0.45], mat: "stone", bevel: 0.0 }));
        }
        parts += q;
        if kase == "diagonal" {
            parts.push(turn(group(q), 180));
        }
    }
    group(parts)
}

// ── ornaments ──

fn post(x, z, h, mat) {
    box([0.1, h, 0.1], #{ at: [x, 0.0, z], mat: mat, bevel: 0.0 })
}

// Street and roof ornaments. Far away (`lod() == 1`) they keep their
// silhouette and drop small parts; the renderer also hides them past the
// tier's detail distance.
fn enfeite_piece(s, name, v) {
    let near = lod() == 0;
    let kerb = box([2.0, 0.15, 0.2], #{ at: [0.0, 0.0, 0.1], mat: "stone", bevel: 0.0 });
    switch name {
        "meio-fio" => kerb,
        "meio-fio-rampa" => union([
            box([0.4, 0.15, 0.2], #{ at: [-0.8, 0.0, 0.1], mat: "stone", bevel: 0.0 }),
            box([0.4, 0.15, 0.2], #{ at: [0.8, 0.0, 0.1], mat: "stone", bevel: 0.0 }),
            box([1.2, 0.03, 0.2], #{ at: [0.0, 0.0, 0.1], mat: "stone", bevel: 0.0 }),
        ]),
        "meio-fio-bueiro" => {
            if !near { return kerb; }
            union([kerb,
                box([0.8, 0.02, 0.35], #{ at: [0.0, 0.0, -0.175], mat: "metal", bevel: 0.0 }),
                box([0.7, 0.06, 0.03], #{ at: [0.0, 0.03, 0.0], mat: "metal", bevel: 0.0 }),
            ])
        },
        "meio-fio-poste" => {
            let pole = cyl(#{ r: 0.05, h: 3.5, at: [0.0, 0.0, 0.3], mat: "metal", seg: 4, bevel: 0.0 });
            let lamp = box([0.24, 0.3, 0.24], #{ at: [0.0, 3.1, -0.35], mat: "glass", bevel: 0.0 });
            let arm = box([0.06, 0.06, 0.7], #{ at: [0.0, 3.45, -0.05], mat: "metal", bevel: 0.0 });
            if !near { return union([kerb, pole, arm, lamp]); }
            union([kerb, pole, arm, lamp,
                cyl(#{ r: 0.1, h: 0.25, at: [0.0, 0.0, 0.3], mat: "metal", seg: 6, bevel: 0.0 }),
                cone(#{ r: 0.22, h: 0.15, at: [0.0, 3.4, -0.35], mat: "metal", seg: 4 }),
            ])
        },
        "calcada" => {
            let base = box([2.0, 0.05, 2.0], #{ mat: "plaza", bevel: 0.0 });
            if !near {
                base
            } else if v == 1 {
                // Wave pattern (pedra portuguesa) as three zig-zag strips.
                let waves = [];
                for z in [-0.6, 0.0, 0.6] {
                    waves.push(extrude([[-1.0, z], [0.0, z - 0.15], [1.0, z],
                                        [1.0, z + 0.1], [0.0, z - 0.05], [-1.0, z + 0.1]],
                        #{ h: 0.01, axis: "y", at: [0.0, 0.05, 0.0], mat: "street" }));
                }
                union([base] + waves)
            } else if v == 2 {
                union([base, box([1.0, 0.01, 1.0], #{ at: [-0.5, 0.05, -0.5], mat: "stone", bevel: 0.0 }),
                             box([1.0, 0.01, 1.0], #{ at: [0.5, 0.05, 0.5], mat: "stone", bevel: 0.0 })])
            } else {
                base
            }
        },
        "cerca" => {
            let m = s.fence_mat;
            if !near {
                return box([2.0, 0.75, 0.04], #{ at: [0.0, 0.1, 0.05], mat: m, bevel: 0.0 });
            }
            let parts = [post(-0.95, 0.0, 0.9, m), post(0.0, 0.0, 0.9, m), post(0.95, 0.0, 0.9, m)];
            parts.push(box([2.0, 0.07, 0.04], #{ at: [0.0, 0.3, 0.05], mat: m, bevel: 0.0 }));
            parts.push(box([2.0, 0.07, 0.04], #{ at: [0.0, 0.7, 0.05], mat: m, bevel: 0.0 }));
            union(parts)
        },
        "muro" => {
            let w = box([2.0, 1.1, 0.3], #{ at: [0.0, 0.0, 0.15], mat: "wall", bevel: 0.0 });
            if !near { return w; }
            union([w, box([2.0, 0.1, 0.38], #{ at: [0.0, 1.1, 0.15], mat: "trim", bevel: 0.0 })])
        },
        "sebe" => box([2.0, 1.0, 0.5], #{ at: [0.0, 0.0, 0.25], mat: "leaf", bevel: if near { 0.12 } else { 0.0 } }),
        "portao" => {
            let parts = [];
            for x in [-0.85, 0.85] {
                parts.push(box([0.3, 1.5, 0.3], #{ at: [x, 0.0, 0.15], mat: "wall", bevel: 0.0 }));
                if near {
                    parts.push(box([0.36, 0.1, 0.36], #{ at: [x, 1.5, 0.15], mat: "trim", bevel: 0.0 }));
                }
            }
            for x in [-0.36, 0.36] {
                parts.push(box([0.66, 1.1, 0.05], #{ at: [x, 0.1, 0.15], mat: s.fence_mat, bevel: 0.0 }));
            }
            union(parts)
        },
        "canteiro" => {
            // Stone kerb around a soil bed with two low bushes and a flower.
            let ring = subtract(box([1.4, 0.3, 1.4], #{ mat: "stone", bevel: 0.0 }),
                                box([1.0, 0.4, 1.0], #{ at: [0.0, 0.2, 0.0], mat: "ground", bevel: 0.0 }));
            if !near {
                return union([ring, blob(#{ r: 0.45, at: [0.0, 0.2, 0.0], mat: "leaf", seg: 4, squash: 0.6 })]);
            }
            let parts = [ring];
            for p in [[-0.2, -0.15], [0.22, 0.18]] {
                parts.push(blob(#{ r: 0.3, at: [p[0], 0.2, p[1]], mat: "leaf", seg: 8, squash: 0.75 }));
            }
            parts.push(blob(#{ r: 0.12, at: [0.2, 0.45, -0.2], mat: "cloth", seg: 4 }));
            union(parts)
        },
        "vaso" => {
            let seg = if near { 6 } else { 4 };
            union([cyl(#{ r: 0.25, r2: 0.3, h: 0.5, mat: "stone", seg: seg, bevel: 0.0 }),
                   blob(#{ r: 0.35, at: [0.0, 0.5, 0.0], mat: "leaf", seg: if near { 8 } else { 4 }, squash: 0.9, jitter: 0.1 })])
        },
        "chamine" => {
            let c = box([0.5, 1.8, 0.5], #{ at: [0.0, -0.6, 0.0], mat: s.chimney_mat, bevel: 0.0 });
            if !near { return c; }
            union([c, box([0.62, 0.1, 0.62], #{ at: [0.0, 1.2, 0.0], mat: "trim", bevel: 0.0 })])
        },
        "telha-capa" => {
            let parts = [];
            if !near {
                return extrude([[-0.13, -0.05], [0.13, -0.05], [0.0, 0.1]],
                    #{ h: 2.0, axis: "x", mat: "roof" });
            }
            for i in 0..5 {
                parts.push(extrude([[-0.13, -0.05], [0.13, -0.05], [0.1, 0.06], [0.0, 0.1], [-0.1, 0.06]],
                    #{ h: 0.42, axis: "x", at: [-0.8 + i.to_float() * 0.4, 0.0, 0.0], mat: "roof" }));
            }
            union(parts)
        },
        "cata-vento" => union([
            cyl(#{ r: 0.03, h: 1.2, mat: "metal", seg: 5, bevel: 0.0 }),
            extrude([[-0.35, 0.0], [0.25, 0.0], [0.25, -0.05], [0.4, 0.025], [0.25, 0.1], [0.25, 0.05], [-0.35, 0.05]],
                #{ h: 0.02, axis: "z", at: [0.0, 0.95, 0.0], mat: "metal" }),
            extrude([[-0.15, 0.0], [0.1, 0.0], [0.15, 0.15], [0.05, 0.12], [0.08, 0.28], [-0.02, 0.2], [-0.12, 0.25], [-0.1, 0.1]],
                #{ h: 0.03, axis: "z", at: [0.0, 1.2, 0.0], mat: "metal" }),
        ]),
        "ponte" => union([box([2.0, 0.3, 2.0], #{ at: [0.0, -0.3, 0.0], mat: "stone", bevel: 0.0 }),
                          box([0.6, 2.7, 0.6], #{ at: [0.0, -3.0, 0.0], mat: "stone", bevel: 0.0 })]),
        "ponte-guarda" => {
            let parts = [post(-0.95, 0.1, 1.0, "stone"), post(0.95, 0.1, 1.0, "stone")];
            parts.push(box([2.0, 0.1, 0.15], #{ at: [0.0, 0.95, 0.1], mat: s.fence_mat, bevel: 0.0 }));
            if !near { return union(parts); }
            for x in [-0.5, 0.0, 0.5] {
                parts.push(box([0.06, 0.95, 0.06], #{ at: [x, 0.0, 0.1], mat: "metal", bevel: 0.0 }));
            }
            union(parts)
        },
        _ => (),
    }
}
