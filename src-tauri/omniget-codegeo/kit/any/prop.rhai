// Street props (base up to about 1×1 m, centred on the origin) with their
// use anchors: bench and table (sit), fountain and trough (water), vase
// (plant), campfire (talk). Two variants each.
switch CASE {
    "banco" => {
        if VARIANT == 1 {
            seat(#{ w: 1.6, d: 0.5, face: "n", mat: "stone", back: false })
        } else {
            let s = seat(#{ w: 1.5, d: 0.45, face: "n", mat: "wood" });
            let arms = [];
            for x in [-0.78, 0.78] {
                arms.push(box([0.08, 0.3, 0.45], #{ at: [x, 0.45, 0.0], mat: "metal", bevel: 0.02 }));
            }
            union([s, arms])
        }
    },
    "poste" => {
        let parts = [cyl(#{ r: 0.12, h: 0.3, mat: "metal", seg: 8 }),
                     cyl(#{ r: 0.05, h: 3.0, at: [0.0, 0.3, 0.0], mat: "metal", seg: 6, bevel: 0.0 })];
        let lamp = |x| [box([0.26, 0.4, 0.26], #{ at: [x, 3.0, 0.0], mat: "glass", bevel: 0.03 }),
                        cone(#{ r: 0.24, h: 0.2, at: [x, 3.4, 0.0], mat: "metal", seg: 4 })];
        if VARIANT == 1 {
            parts.push(box([1.1, 0.06, 0.06], #{ at: [0.0, 3.1, 0.0], mat: "metal", bevel: 0.0 }));
            parts += lamp.call(-0.5);
            parts += lamp.call(0.5);
        } else {
            parts += lamp.call(0.0);
        }
        union(parts)
    },
    "lixeira" => {
        if VARIANT == 1 {
            union([box([0.5, 0.8, 0.5], #{ mat: "wood", edges: "y", bevel: 0.05 }),
                   box([0.56, 0.06, 0.56], #{ at: [0.0, 0.8, 0.0], mat: "metal", bevel: 0.02 })])
        } else {
            union([cyl(#{ r: 0.25, r2: 0.28, h: 0.8, mat: "metal", seg: 10 }),
                   cyl(#{ r: 0.04, h: 0.9, at: [0.0, 0.0, 0.3], mat: "metal", seg: 5, bevel: 0.0 })])
        }
    },
    "vaso" => {
        let pot = if VARIANT == 1 {
            box([0.7, 0.5, 0.7], #{ mat: "stone", bevel: 0.05 })
        } else {
            cyl(#{ r: 0.3, r2: 0.38, h: 0.55, mat: "stone", seg: 10 })
        };
        anchor("plant", #{ at: [0.0, 0.0, -0.7], face: "s" });
        union([pot, blob(#{ r: 0.45, at: [0.0, 0.6, 0.0], mat: "leaf", seg: 10, squash: 0.85, jitter: 0.1 })])
    },
    "chafariz" => {
        let r = if VARIANT == 1 { 0.9 } else { 1.1 };
        let basin = subtract(cyl(#{ r: r, h: 0.5, mat: "stone", seg: 12 }),
                             cyl(#{ r: r - 0.18, h: 0.6, at: [0.0, 0.15, 0.0], mat: "stone", seg: 12, bevel: 0.0 }));
        let water = cyl(#{ r: r - 0.17, h: 0.3, at: [0.0, 0.1, 0.0], mat: "water", seg: 12, bevel: 0.0 });
        let column = cyl(#{ r: 0.14, r2: 0.1, h: 1.1, at: [0.0, 0.3, 0.0], mat: "stone", seg: 8 });
        let bowl = cyl(#{ r: 0.4, r2: 0.2, h: 0.18, at: [0.0, 1.2, 0.0], mat: "stone", seg: 10 });
        let jet = blob(#{ r: 0.1, at: [0.0, 1.45, 0.0], mat: "water", seg: 6 });
        anchor("water", #{ at: [0.0, 0.0, -(r + 0.4)], face: "s", capacity: 4 });
        union([basin, water, column, bowl, jet])
    },
    "placa" => {
        let board = if VARIANT == 1 {
            box([0.9, 0.5, 0.06], #{ at: [0.0, 1.6, 0.0], mat: "wood", edges: "z", bevel: 0.03 })
        } else {
            extrude([[-0.45, 0.0], [0.35, 0.0], [0.5, 0.2], [0.35, 0.4], [-0.45, 0.4]],
                #{ h: 0.06, axis: "z", at: [0.0, 1.7, 0.0], mat: "wood" })
        };
        union([box([0.1, 2.2, 0.1], #{ mat: "wood", edges: "y", bevel: 0.02 }), board])
    },
    "barril" => {
        let b = cyl(#{ r: 0.35, r2: 0.35, h: 0.9, mat: "wood", seg: 10 });
        let hoops = [cyl(#{ r: 0.37, h: 0.05, at: [0.0, 0.18, 0.0], mat: "metal", seg: 10, bevel: 0.0 }),
                     cyl(#{ r: 0.37, h: 0.05, at: [0.0, 0.67, 0.0], mat: "metal", seg: 10, bevel: 0.0 })];
        if VARIANT == 1 {
            union([turn(b, 0).translate([-0.3, 0.0, 0.0]), cyl(#{ r: 0.3, h: 0.75, at: [0.4, 0.0, 0.2], mat: "wood", seg: 10 })] + hoops)
        } else {
            union([b] + hoops)
        }
    },
    "caixote" => {
        let c = box([0.7, 0.6, 0.7], #{ mat: "wood", bevel: 0.03 });
        let slats = [box([0.72, 0.08, 0.72], #{ at: [0.0, 0.26, 0.0], mat: "wood", bevel: 0.0 })];
        if VARIANT == 1 {
            slats.push(turn(box([0.5, 0.45, 0.5], #{ at: [0.0, 0.6, 0.0], mat: "wood", bevel: 0.03 }), 20));
        }
        union([c] + slats)
    },
    "mesa" => {
        let top = if VARIANT == 1 {
            cyl(#{ r: 0.5, h: 0.06, at: [0.0, 0.72, 0.0], mat: "wood", seg: 10 })
        } else {
            box([1.0, 0.06, 0.7], #{ at: [0.0, 0.72, 0.0], mat: "wood", bevel: 0.02 })
        };
        let leg = cyl(#{ r: 0.05, h: 0.72, mat: "metal", seg: 6, bevel: 0.0 });
        let s1 = seat(#{ at: [0.0, 0.0, -0.75], face: "s", w: 0.5, d: 0.4, back: false, capacity: 1 });
        let s2 = seat(#{ at: [0.0, 0.0, 0.75], face: "n", w: 0.5, d: 0.4, back: false, capacity: 1 });
        union([top, leg, s1, s2])
    },
    "carroca" => {
        let bed = box([1.6, 0.35, 0.9], #{ at: [0.0, 0.45, 0.0], mat: "wood", bevel: 0.03 });
        let wheels = [];
        for z in [-0.5, 0.5] {
            wheels.push(extrude([[0.35, 0.0], [0.25, 0.25], [0.0, 0.35], [-0.25, 0.25], [-0.35, 0.0], [-0.25, -0.25], [0.0, -0.35], [0.25, -0.25]],
                #{ h: 0.08, axis: "z", at: [0.2, 0.35, z], mat: "wood" }));
        }
        let shafts = [box([1.2, 0.06, 0.06], #{ at: [-1.3, 0.55, -0.35], mat: "wood", bevel: 0.0 }),
                      box([1.2, 0.06, 0.06], #{ at: [-1.3, 0.55, 0.35], mat: "wood", bevel: 0.0 })];
        let load = if VARIANT == 1 {
            [blob(#{ r: 0.35, at: [0.0, 0.8, 0.0], mat: "leaf", seg: 8, squash: 0.7 })]
        } else {
            [box([0.5, 0.4, 0.5], #{ at: [-0.3, 0.8, 0.0], mat: "wood", bevel: 0.03 }),
             cyl(#{ r: 0.2, h: 0.5, at: [0.35, 0.8, 0.1], mat: "wood", seg: 8 })]
        };
        union([bed] + wheels + shafts + load)
    },
    "bebedouro" => {
        let trough = subtract(box([1.4, 0.6, 0.6], #{ mat: "stone", bevel: 0.04 }),
                              box([1.2, 0.6, 0.4], #{ at: [0.0, 0.2, 0.0], mat: "stone", bevel: 0.0 }));
        let water = box([1.21, 0.3, 0.41], #{ at: [0.0, 0.2, 0.0], mat: "water", bevel: 0.0 });
        let parts = [trough, water];
        if VARIANT == 1 {
            parts.push(box([0.2, 1.2, 0.2], #{ at: [0.0, 0.0, 0.4], mat: "stone", bevel: 0.03 }));
            parts.push(cyl(#{ r: 0.03, h: 0.25, at: [0.0, 0.95, 0.2], mat: "metal", seg: 5, bevel: 0.0 }));
        }
        anchor("water", #{ at: [0.0, 0.0, -0.8], face: "s", capacity: 2 });
        union(parts)
    },
    "fogueira" => {
        let parts = [];
        for i in 0..6 {
            let a = (i.to_float() * 60.0).to_radians();
            parts.push(blob(#{ r: 0.14, at: [a.cos() * 0.45, 0.0, a.sin() * 0.45], mat: "stone", seg: 6, squash: 0.7 }));
        }
        for d in [0, 60, 120] {
            parts.push(turn(box([0.7, 0.1, 0.1], #{ at: [0.0, 0.05, 0.0], mat: "wood", bevel: 0.02 }), d));
        }
        parts.push(cone(#{ r: 0.22, h: 0.55, at: [0.0, 0.1, 0.0], mat: "cloth", seg: 5 }));
        if VARIANT == 1 {
            for x in [-1.1, 1.1] {
                parts.push(box([0.35, 0.4, 0.9], #{ at: [x, 0.0, 0.0], mat: "wood", bevel: 0.04 }));
            }
        }
        anchor("talk", #{ at: [0.0, 0.0, -1.0], face: "s", capacity: 6 });
        union(parts)
    },
    "estatua" => {
        let ped = union([box([0.8, 0.9, 0.8], #{ mat: "stone", bevel: 0.05 }),
                         box([0.95, 0.12, 0.95], #{ at: [0.0, 0.9, 0.0], mat: "stone", bevel: 0.03 })]);
        let fig = if VARIANT == 1 {
            union([cyl(#{ r: 0.22, r2: 0.12, h: 0.9, at: [0.0, 1.02, 0.0], mat: "metal", seg: 8 }),
                   blob(#{ r: 0.3, at: [0.0, 2.05, 0.0], mat: "metal", seg: 8 })])
        } else {
            union([cyl(#{ r: 0.18, r2: 0.22, h: 0.85, at: [0.0, 1.02, 0.0], mat: "metal", seg: 8 }),
                   blob(#{ r: 0.14, at: [0.0, 2.02, 0.0], mat: "metal", seg: 6 }),
                   box([0.08, 0.5, 0.08], #{ at: [0.2, 1.75, 0.0], mat: "metal", bevel: 0.02 })])
        };
        union([ped, fig])
    },
    _ => {
        // generico: a crate stack or a sack pile.
        if VARIANT == 1 {
            union([blob(#{ r: 0.3, at: [-0.2, 0.0, 0.0], mat: "cloth", seg: 8, squash: 0.7 }),
                   blob(#{ r: 0.28, at: [0.25, 0.0, 0.1], mat: "cloth", seg: 8, squash: 0.7 }),
                   blob(#{ r: 0.26, at: [0.0, 0.3, 0.05], mat: "cloth", seg: 8, squash: 0.7 })])
        } else {
            union([box([0.6, 0.5, 0.6], #{ mat: "wood", bevel: 0.03 }),
                   turn(box([0.45, 0.4, 0.45], #{ at: [0.0, 0.5, 0.0], mat: "wood", bevel: 0.03 }), 15)])
        }
    },
}
