//! What an LLM needs to write a charter, a program and patch ops: the format
//! (estudo 09 §1 and §4) and the options that are valid right now. The solver
//! only offers valid options (09 §6), so free lots come from it, never from a
//! guess.

use omniget_citygen::program::CityProgram;
use omniget_citygen::{solver, Charter};
use serde_json::Value;

/// The program and patch format, for system prompts and tool descriptions.
pub const FORMAT: &str = r##"UNITS: 1 tile = 2 m, 1 floor = 3 m. A region is 64x64 tiles; x grows east (0..63), y grows south (0..63). Rects are [x, y, w, h] in tiles.

CHARTER (omni.city.charter/v1), the city's founding document, never geometry:
{"schema":"omni.city.charter/v1","city_id":"...","name":"...","seed":"...","story":"one paragraph","climate":"temperate|tropical|arid|cold|coastal","culture":"free text","style":"vila-colonial","palette":"<palette key>","palettes":[{"key":"cal-e-terracota","slots":{"wall":"#f2ead8","roof":"#b5553a","trim":"#ffffff","wood":"#7a5230","door":"#5b3a22","glass":"#9fc3d6","stone":"#a39e93","ground":"#c9b38a","grass":"#7fa35a","water":"#4f8fb8","street":"#8f8a80","plaza":"#d8c8a8","leaf":"#5f8f3f","trunk":"#6b4a2f","metal":"#555a60","cloth":"#c0392b"}}],
 "relief":{"sea_level_mm":0,"hill_mm":6000,"mountain_mm":0,"erosion":400,"rivers":300,"forest":400,"city_flatten_slope":100},
 "proximity":[],"districts":[{"id":"centro","kind":"civico","regions":[[0,0]],"style":null}],"publish_policy":"auto|review"}

PROGRAM (omni.city.program/v1), one per region, declarative intent:
{"schema":"omni.city.program/v1","city_id":"...","region":[0,0],"catalog":"kit-vila-v1",
 "zones":[{"id":"centro","kind":"civico|residencial|comercial|rural|parque","rect":[0,0,64,32],"style":"vila-colonial","floors":[1,3],"density":800,"setback":{"front":0,"side":0,"back":1},"palette":null}],
 "streets":[{"id":"av-central","class":"avenida|rua|beco|trilha","width":3,"path":[[0,30],[63,30]]}],
 "water":[{"id":"rio","rect":[0,56,64,4],"bridges":["rua-do-porto"]}],
 "landmarks":[{"id":"praca-matriz","kind":"praca|parque|mercado|porto|templo","rect":[24,18,14,11]}],
 "lots":{"auto":true,"min_front":4,"min_depth":5,"max_area":60,"min_buildable":5},
 "buildings":[{"id":"prefeitura","lot":"auto:near:praca-matriz","intent":{"use":"civico","floors":3,"shape":"U","roof":"quatro-aguas","tower":true,"palette":"cal-e-terracota","label":"Prefeitura"}}],
 "fill":[{"zone":"centro","use":"loja-sobrado","shape":["rect","L"],"roof":["duas-aguas"]}],
 "props":[{"kind":"arvore","preset":"oak-estilizada","scatter":{"zone":"beira-rio","on":"ground","spacing":5}}]}
Rules: ids are stable and semantic (cite "prefeitura", not "building 7"). "lot" is an exact lot id, "auto:near:<landmark>" or "auto:zone:<zone>". density is 0..1000. Streets must connect into one network and touch every landmark; a plaza must touch a street. Widths: avenida >= 3, rua >= 2, beco = 1 (<= 24 tiles long). A street crossing water becomes a bridge (width >= 2). Never write tiles, heights or coordinates of buildings: the solver places them.

PATCH OPS (edit an existing region, never resend the whole program):
{"op":"add_zone", ...zone} | {"op":"add_street", ...street} | {"op":"add_landmark", ...landmark} | {"op":"add_water", ...water} | {"op":"add_building","id":"padaria","lot":"auto:zone:centro","intent":{"use":"loja-sobrado","floors":2,"shape":"rect","roof":"duas-aguas","awning":true}} | {"op":"add_props", ...props} | {"op":"add_fill", ...fill} | {"op":"set_intent","id":"prefeitura","patch":{"tower":false}} | {"op":"move","id":"padaria","lot":"<lot id>"} | {"op":"move","id":"rua-nova","path":[[20,40],[44,40]]} | {"op":"remove","id":"beco-norte"}
A proposal is {"region":[rx,ry],"base_revision":"sha256:...","ops":[...]}; base_revision is the revision you read. New buildings start as construction sites and rise over time.

BUILDING CODE (checker): each violation is [code, id, measure] + message. Errors block publishing, warnings do not. R1 no overlap, R2 nothing on public ground, R3 buildable area >= 3x3, R4 setbacks, R5 door reaches a street within 12 tiles, R6 one connected street network, R7 stairs, R8 support, R9 floors within the zone, R10 bridges >= 2 wide, R11 no dead 1-tile gaps, R12 lot fronts a street >= 3 tiles, R13 street width per class, R14 slope, R15 landmarks reachable, R16 catalog keys exist, R17 owned lots only change with admin, R18 budget per chunk. P* codes are program or patch errors (unknown id, bad JSON, stale revision)."##;

/// Valid choices for a program of this charter now (agent B's solver:
/// zones with their free lots, landmarks, streets and every enum).
pub fn options(charter: &Charter, program: &CityProgram) -> Value {
    solver::options(charter, program)
}

/// The program of region [0,0] a city gets when it is founded without an LLM.
pub fn starter_program(charter: &Charter) -> CityProgram {
    solver::default_program(&charter.city_id, [0, 0])
}
