// Stylised trees: solid low-poly crowns on a tapered trunk (Townscaper /
// ez-tree "copa sólida"), 4 to 9 m tall, trunk at the origin. CASE is the
// preset, VARIANT picks proportions; rand(i) is seeded by the module key, so
// every variant is stable.
//
// Trees are the most repeated piece of a region (forests scatter hundreds),
// so crowns are low-poly on purpose: 32-triangle blobs up close; far away
// (`lod() == 1`) one blob on a four-sided trunk. Parts are grouped, not
// unioned: overlapping blobs draw the same and the boolean would only add
// triangles along the seams.
let v = VARIANT.to_float();
let near = lod() == 0;

fn trunk(h, r, y, x) {
    let seg = if lod() == 0 { 6 } else { 4 };
    cyl(#{ r: r, r2: r * 0.7, h: h, at: [x, y, 0.0], mat: "trunk", seg: seg, bevel: 0.0 })
}

fn crown_ring(n, d, y, r, r_var, k, mat, seg) {
    let out = [];
    for i in 0..n {
        let a = (i.to_float() * 360.0 / n.to_float() + rand(k + i) * 40.0).to_radians();
        let rr = r + rand(k + 10 + i) * r_var;
        out.push(blob(#{ r: rr, at: [a.cos() * d, y + rand(k + 20 + i) * 0.5, a.sin() * d],
                         mat: mat, seg: seg, squash: 0.85, jitter: 0.1 }));
    }
    out
}

switch CASE {
    "oak-estilizada" => {
        let h = 2.3 + v * 0.2;
        let r = 1.6 + rand(1) * 0.3;
        let parts = [trunk(h + 0.8, 0.26, 0.0, 0.0),
                     blob(#{ r: r, at: [0.0, h + r * 0.55, 0.0], mat: "leaf", seg: 8, squash: 0.85, jitter: 0.12 })];
        if near {
            parts += crown_ring(3, r * 0.7, h + 0.2, 0.95, 0.3, 100, "leaf", 8);
        }
        group(parts)
    },
    "pinheiro-estilizado" => {
        let h = 1.0 + v * 0.15;
        let parts = [trunk(h + 0.8, 0.2, 0.0, 0.0)];
        if !near {
            parts.push(cone(#{ r: 1.7, h: 5.4, at: [0.0, h, 0.0], mat: "leaf", seg: 6 }));
            return group(parts);
        }
        for i in 0..4 {
            let f = i.to_float();
            let r = 1.7 - f * 0.35 + rand(40 + i) * 0.1;
            parts.push(cone(#{ r: r, h: 1.7 + rand(50 + i) * 0.2, at: [0.0, h + f * 1.25, 0.0], mat: "leaf", seg: 8 }));
        }
        group(parts)
    },
    "palmeira" | "coqueiro" => {
        let coco = CASE == "coqueiro";
        let n = if coco { 4 } else { 3 };
        let seg_h = if coco { 2.0 } else { 1.9 };
        seg_h += v * 0.05;
        if !near {
            n = 2;
            seg_h *= if coco { 2.0 } else { 1.5 };
        }
        let bend = if coco { 0.14 } else { 0.07 };
        let parts = [];
        let lean = 0.0;
        let y = 0.0;
        for i in 0..n {
            let r = 0.2 - i.to_float() * 0.015;
            parts.push(cyl(#{ r: r, r2: r * 0.9, h: seg_h, at: [lean, y, 0.0], mat: "trunk", seg: if near { 5 } else { 4 }, bevel: 0.0 }));
            y += seg_h * 0.97;
            lean += bend * (0.6 + rand(60 + i));
        }
        // Fronds: a drooping leaf profile, turned around the top.
        let leaf = if near {
            [[0.0, 0.0], [1.0, 0.3], [2.2, -0.4], [2.4, -0.9], [1.1, -0.1], [0.4, -0.2]]
        } else {
            [[0.0, 0.0], [1.2, 0.2], [2.4, -0.9], [0.6, -0.2]]
        };
        let fronds = if coco { 7 } else { 6 };
        if !near { fronds = 5; }
        for i in 0..fronds {
            let f = extrude(leaf, #{ h: 0.35, axis: "z", mat: "leaf" });
            parts.push(turn(f, i * (360 / fronds) + VARIANT * 11).translate([lean, y, 0.0]));
        }
        if coco && near {
            for i in 0..3 {
                let a = (i.to_float() * 120.0).to_radians();
                parts.push(blob(#{ r: 0.16, at: [lean + a.cos() * 0.2, y - 0.35, a.sin() * 0.2], mat: "wood", seg: 4 }));
            }
        }
        group(parts)
    },
    "ipe" => {
        let h = 2.6 + v * 0.2;
        let parts = [trunk(h + 0.6, 0.22, 0.0, 0.0)];
        parts.push(blob(#{ r: 1.5, at: [0.0, h + 1.3, 0.0], mat: "cloth", seg: 8, squash: 0.6, jitter: 0.12 }));
        if near {
            parts.push(trunk(1.4, 0.13, h - 0.3, 0.35));
            parts.push(trunk(1.4, 0.13, h - 0.3, -0.35));
            parts += crown_ring(3, 1.1, h + 0.9, 0.8, 0.25, 200, "cloth", 8);
        }
        group(parts)
    },
    "arbusto" => {
        let parts = [];
        let n = if near { 3 + VARIANT % 2 } else { 1 };
        for i in 0..n {
            let a = (i.to_float() * 137.5 + v * 31.0).to_radians();
            let d = if i == 0 { 0.0 } else { 0.45 };
            let r = 0.65 - i.to_float() * 0.06 + rand(70 + i) * 0.1;
            let rr = if near { r } else { r + 0.2 };
            parts.push(blob(#{ r: rr, at: [a.cos() * d, 0.0, a.sin() * d], mat: "leaf", seg: 8, squash: 0.8, jitter: 0.12 }));
        }
        group(parts)
    },
    "cacto" => {
        let h = 3.0 + v * 0.3;
        let parts = [cyl(#{ r: 0.28, r2: 0.22, h: h, mat: "leaf", seg: if near { 6 } else { 4 }, bevel: 0.0 })];
        let arms = if near { 2 + VARIANT % 2 } else { 0 };
        for i in 0..arms {
            let side = if i % 2 == 0 { 1.0 } else { -1.0 };
            let y0 = 0.9 + i.to_float() * 0.5 + rand(80 + i) * 0.3;
            let turn_deg = i * 70 + VARIANT * 23;
            let arm = group([
                box([0.55, 0.3, 0.3], #{ at: [side * 0.35, y0, 0.0], mat: "leaf", bevel: 0.0 }),
                cyl(#{ r: 0.17, r2: 0.13, h: 1.2 + rand(90 + i) * 0.5, at: [side * 0.6, y0, 0.0], mat: "leaf", seg: 5, bevel: 0.0 }),
            ]);
            parts.push(turn(arm, turn_deg));
        }
        group(parts)
    },
    _ => (),
}
