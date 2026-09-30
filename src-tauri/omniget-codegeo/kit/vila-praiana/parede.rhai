// Upper-floor wall; corner boards on every convex corner (clapboard look).
let s = S();
let m = wall_module(s, CASE, VARIANT, false);
if CASE == "convexo" && VARIANT != 1 && lod() == 0 { m = union([m, corner_board(s)]); }
m
