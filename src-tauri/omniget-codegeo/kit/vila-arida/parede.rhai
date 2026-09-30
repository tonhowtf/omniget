// Upper-floor adobe wall; the straight wall gets vigas under the roof line.
let m = wall_module(S(), CASE, VARIANT, false);
if CASE == "parede" && VARIANT != 1 { m = vigas(m); }
m
