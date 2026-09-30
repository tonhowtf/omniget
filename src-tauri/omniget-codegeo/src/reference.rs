//! The kit documentation an LLM gets in its prompt, and ready examples.

use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct Example {
    pub id: &'static str,
    pub name: &'static str,
    pub class: &'static str,
    pub script: &'static str,
}

const REFERENCE: &str = r#"# OmniGet city geometry kit (Rhai)

Write a short Rhai script (JavaScript-like syntax: `let`, `for x in [..]`,
`if`, `fn`) that ends with the solid to build, or an array of solids (they are
united). The kit applies the city style for you: bevels on every edge, sizes
snapped to a 5 cm grid, colours only from palette slots. You never write
vertices or colours.

## Conventions
- Metres. x = east, y = up, z = south. The ground is y = 0.
- `at: [x, y, z]` is the CENTRE OF THE BASE of a part (y = its bottom).
- `rot` / `face`: degrees clockwise seen from above, 0 = north (−z), or
  "n" "e" "s" "w". `face` is the side a piece opens or looks to.
- Floats need a dot: `2.0`, not `2`. Options go in a map: `#{ key: value }`.
- Materials (`mat`) are palette slots: wall, roof, trim, wood, stone, glass,
  door, ground, grass, water, street, plaza, leaf, trunk, metal, cloth.

## Primitives (each returns a Solid)
- `box([w, h, d], #{ at, mat, bevel, edges, rot })` — edges: "all" (default),
  "x", "y", "z" or "none" pick which edges get the chamfer; bevel: 0 = sharp.
- `cyl(#{ r, h, r2, at, mat, seg, bevel })` — vertical cylinder; `r2` = top
  radius (cone frustum). seg 3–48 (default 12).
- `cone(#{ r, h, at, mat, seg })`
- `wedge([w, h, d], #{ at, mat, rot })` — ramp: full height at the north edge.
- `gable(#{ w, d, h, eave, thick, at, mat, wall, rot })` — two-pitch roof, ridge
  along x, `h` = ridge height above `at`, gable ends filled with `wall`.
- `hip(#{ w, d, h, eave, thick, at, mat, rot })` — four-pitch roof.
- `cone_roof(#{ r, h, eave, thick, at, mat, seg })` — conical roof.
- `arch(#{ w, h, d, at, mat, seg, rot })` — arch-shaped solid (for openings).
- `extrude([[a, b], ...], #{ h, axis, at, mat, rot })` — 3 to 64 points.
  axis "y" (default): points are [x, z] in plan, extruded up by h;
  axis "x": points are [z, y], extruded along x; axis "z": points [x, y].
- `blob(#{ r, at, mat, seg, squash, jitter })` — low-poly ball (foliage).

## Transforms and booleans
- `translate(s, [x, y, z])`, `turn(s, deg)`, `mirror(s, "x"|"y"|"z")` (copy only)
- `repeat(s, n, [dx, dy, dz])` — n copies (1–64), united.
- `union([a, b, ...])`, `subtract(a, [cuts...])` or `subtract(a, b)`,
  `intersect(a, b)`, `hull([a, b], #{ mat })`, `group([...])` (no boolean).
- Surfaces cut by a solid take that solid's material.
- Helpers: `bounds(s)` → #{min, max}, `tris(s)`, `snap(v)`, `seed(n)`,
  `rand(i)` → 0..1 (deterministic), `kind("bench")` declares what it is.
- Methods work too: `box([1.0, 1.0, 1.0]).translate([0.0, 2.0, 0.0])`.

## Semantic pieces (they create the anchors people use)
- `door(body, #{ at, face, w, h, arch, frame, step, mat })` — cuts the opening
  in `body`, adds leaf, frame and step, and an `enter` anchor outside.
  `at` = bottom centre of the doorway ON the facade plane.
- `window(body, #{ at, face, w, h, depth, sill, lintel, shutters, arch, mat })`
  — recess with glass, sill, optional lintel and shutters.
- `seat(#{ at, face, w, d, h, back, mat, capacity })` → `sit` anchor.
- `bed(#{ at, face, w, l, mat, cloth })` → `sleep` anchor.
- `counter(#{ at, face, w, d, h, mat, top })` → `work` anchor behind it.
- `bench_top(#{ at, face, w, d, h, mat })` → `work` anchor in front.
- `planter(#{ at, face, w, d, h, plants, mat })` → `plant` anchor in front.
- `anchor(action, #{ at, face, capacity })` — action: enter, sit, sleep, work,
  plant, water, harvest, talk, stand.

## Classes and what the validator checks
- `building` (a typology): needs a `door()` at ground level on an outer
  wall; footprint up to 16×16 tiles (a tile is 2 m); up to 45 m tall;
  up to 5000 triangles.
- `prop`: up to 5×5 tiles, 15 m, 1500 triangles. `prop:bench` needs a seat,
  `prop:bed` a bed, `prop:planter` a planter, `prop:kiosk` / `prop:counter`
  a counter, `prop:well` a `water` anchor. `kind("...")` does the same.
- `module`: a kit cell of 2×2×3 m centred on the origin, up to 400 triangles.
- Limits: 200 000 script steps, 400 primitives, 250 ms. Errors come back
  with line and column: fix that line and run again.

## Tips
- Build the walls as one box, then cut doors and windows into it.
- Keep a storey 3 m tall; windows sit 0.9 m above the floor.
- Put the building's door on the side that faces the street ("s" if unsure).

## Example: a small house (class building)
let w = 4.0; let d = 3.2; let h = 2.8;
let body = box([w, h, d], #{ mat: "wall" });
body = door(body, #{ at: [0.0, 0.0, d / 2.0], face: "s" });
for x in [-1.3, 1.3] {
    body = window(body, #{ at: [x, 0.9, d / 2.0], face: "s", shutters: true });
}
let roof = gable(#{ w: w, d: d, h: 1.4, eave: 0.4, at: [0.0, h, 0.0] });
[body, roof, box([w + 0.1, 0.4, d + 0.1], #{ mat: "stone" })]

## Example: a bench (class prop:bench)
let b = seat(#{ w: 1.6, face: "s" });
let arms = repeat(box([0.08, 0.25, 0.45], #{ at: [-0.8, 0.45, 0.0], mat: "metal" }), 2, [1.6, 0.0, 0.0]);
[b, arms]
"#;

/// Short documentation of the kit with functions and examples, for the LLM
/// prompt (G2) and the in-app help.
pub fn kit_reference() -> String {
    REFERENCE.to_string()
}

const FONTE: &str = r#"// Fonte: round basin with a two-tier spout.
kind("fountain");
let basin = subtract(cyl(#{ r: 1.5, h: 0.5, mat: "stone", seg: 16 }),
                     cyl(#{ r: 1.3, h: 0.6, at: [0.0, 0.15, 0.0], mat: "stone", seg: 16, bevel: 0.0 }));
let water = cyl(#{ r: 1.35, h: 0.3, at: [0.0, 0.1, 0.0], mat: "water", seg: 16, bevel: 0.0 });
let column = cyl(#{ r: 0.2, r2: 0.15, h: 1.2, at: [0.0, 0.3, 0.0], mat: "stone", seg: 8 });
let bowl = cyl(#{ r: 0.6, r2: 0.3, h: 0.2, at: [0.0, 1.3, 0.0], mat: "stone", seg: 12 });
let jet = blob(#{ r: 0.15, at: [0.0, 1.5, 0.0], mat: "water", seg: 6 });
anchor("water", #{ at: [0.0, 0.0, 1.9], face: "n", capacity: 4 });
[basin, water, column, bowl, jet]
"#;

const QUIOSQUE: &str = r#"// Quiosque: octagonal kiosk with a serving counter facing south.
kind("kiosk");
let base = cyl(#{ r: 1.6, h: 0.2, mat: "stone", seg: 8 });
let body = cyl(#{ r: 1.3, h: 2.4, at: [0.0, 0.2, 0.0], mat: "wood", seg: 8 });
body = window(body, #{ at: [0.0, 1.1, 1.25], face: "s", w: 1.4, h: 1.0, depth: 0.3, sill: false });
let shop = counter(#{ at: [0.0, 0.2, 1.55], face: "s", w: 1.4, d: 0.4 });
let roof = cone_roof(#{ r: 1.4, h: 1.0, eave: 0.4, at: [0.0, 2.6, 0.0], seg: 8 });
let finial = blob(#{ r: 0.15, at: [0.0, 3.6, 0.0], mat: "metal", seg: 6 });
[base, body, shop, roof, finial]
"#;

const SOBRADO: &str = r#"// Sobrado: two-storey town house, 6 × 5 m, arched door on the south facade.
let w = 6.0; let d = 5.0; let h = 6.0;
let body = box([w, h, d], #{ mat: "wall", bevel: 0.1 });
body = door(body, #{ at: [0.0, 0.0, d / 2.0], face: "s", arch: true });
for x in [-2.0, 2.0] {
    body = window(body, #{ at: [x, 0.9, d / 2.0], face: "s", shutters: true });
}
for x in [-2.0, 0.0, 2.0] {
    body = window(body, #{ at: [x, 3.8, d / 2.0], face: "s", shutters: true, lintel: true });
}
let plinth = box([w + 0.1, 0.45, d + 0.1], #{ mat: "stone" });
let cornice = box([w + 0.2, 0.2, d + 0.2], #{ at: [0.0, h - 0.3, 0.0], mat: "trim" });
let balcony = box([2.0, 0.12, 0.7], #{ at: [0.0, 3.6, d / 2.0 + 0.35], mat: "stone" });
let rail = box([2.0, 0.8, 0.05], #{ at: [0.0, 3.7, d / 2.0 + 0.65], mat: "metal", bevel: 0.0 });
let roof = hip(#{ w: w, d: d, h: 2.0, eave: 0.5, at: [0.0, h, 0.0] });
[body, plinth, cornice, balcony, rail, roof]
"#;

/// Ready scripts shown in the editor and usable as few-shot examples.
pub fn examples() -> Vec<Example> {
    vec![
        Example {
            id: "fonte",
            name: "Fonte",
            class: "prop:fountain",
            script: FONTE,
        },
        Example {
            id: "quiosque",
            name: "Quiosque",
            class: "prop:kiosk",
            script: QUIOSQUE,
        },
        Example {
            id: "sobrado",
            name: "Sobrado",
            class: "building:house",
            script: SOBRADO,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples_pass_the_validator() {
        for e in examples() {
            let out =
                crate::run_script(e.script, e.class).unwrap_or_else(|x| panic!("{}: {x:?}", e.id));
            assert!(out.triangles > 0, "{}", e.id);
        }
    }
}
