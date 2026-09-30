// Vila árida: thick adobe with soft round corners, flat roofs behind a
// parapet, wooden beam ends (vigas), deep small openings, arched doors.
fn S() {
    #{
        corner: 0.5, plinth: false, cornice: false, corner_mat: "wall",
        eave: 0.2, thick: 0.1, ribs: false, ridge_mat: "trim", parapet: 0.45, coping: false,
        arch_door: true, arch_win: true, frame_mat: "wood", shutters: false, shutter_mat: "wood",
        balcony_mat: "wood", rail_mat: "wood", chimney_mat: "wall", fence_mat: "wood",
    }
}

// Beam ends sticking out under the roof line.
fn vigas(m) {
    if lod() > 0 {
        return m;
    }
    let parts = [m];
    for x in [-0.6, 0.0, 0.6] {
        parts.push(box([0.12, 0.12, 0.4], #{ at: [x, 2.7, -0.15], mat: "wood", bevel: 0.0 }));
    }
    union(parts)
}
