// Ground-floor wall on a low wooden skirt; corner boards on convex corners.
let s = S();
let m = wall_module(s, CASE, VARIANT, true);
if lod() > 0 {
    m
} else {
    let parts = [m, band(CASE, s, 0.04, 0.35, 0.0, "wood")];
    if CASE == "convexo" && VARIANT != 1 { parts.push(corner_board(s)); }
    union(parts)
}
