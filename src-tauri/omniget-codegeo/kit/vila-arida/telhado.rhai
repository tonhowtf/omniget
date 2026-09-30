// Roof pieces: flat `plano-<case>` with a parapet, or a height-field piece
// of the pitched families (slope 1:2, conical 2:1).
let s = S();
if CASE.starts_with("plano-") {
    flat_piece(s, CASE.sub_string(6))
} else {
    roof_piece(s, CASE)
}
