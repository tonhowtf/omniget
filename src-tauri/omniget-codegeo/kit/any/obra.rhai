// Construction: `andaime` is a tubular scaffold 2 m wide and 3 m tall along
// x, 0.6 m outside the wall plane (dual origin, same turn as the wall case);
// `fundacao` is a 2×2 m foundation slab with rebar, centred on a tile.
if CASE == "fundacao" {
    let parts = [box([2.0, 0.3, 2.0], #{ mat: "stone", edges: "y", bevel: 0.04 })];
    for p in [[-0.7, -0.7], [0.7, -0.7], [0.7, 0.7], [-0.7, 0.7], [0.0, 0.0]] {
        parts.push(box([0.04, 0.9, 0.04], #{ at: [p[0], 0.3, p[1]], mat: "metal", bevel: 0.0 }));
    }
    return union(parts);
}
let parts = [];
for x in [-0.95, 0.95] {
    for z in [-0.9, -0.3] {
        parts.push(box([0.07, 3.0, 0.07], #{ at: [x, 0.0, z], mat: "metal", bevel: 0.0 }));
    }
}
for y in [1.45, 2.95] {
    parts.push(box([2.0, 0.05, 0.6], #{ at: [0.0, y, -0.6], mat: "wood", bevel: 0.0 }));
    parts.push(box([2.0, 0.05, 0.05], #{ at: [0.0, y + 0.5, -0.92], mat: "metal", bevel: 0.0 }));
}
// Diagonal brace on the front, as a thin slanted bar.
parts.push(extrude([[-0.95, 0.1], [-0.85, 0.1], [0.95, 1.4], [0.85, 1.4]],
    #{ h: 0.04, axis: "z", at: [0.0, 0.0, -0.95], mat: "metal" }));
union(parts)
