//! Edge geometry and snapping, in logical pixels and free of any window
//! handle, so the part that goes wrong on a second monitor with a negative
//! origin is the part the tests reach.
//!
//! The window is exactly as big as what it draws, because a transparent
//! window still swallows the clicks that land on it: collapsed it is the
//! strip, expanded it grows away from the edge to make room for the card.

use super::prefs::Edge;

/// One ring and the air around it.
pub const CELL: f64 = 36.0;
pub const PAD: f64 = 8.0;
/// The handle the strip is dragged by, at its start.
pub const GRIP: f64 = 14.0;
/// Thickness of the strip across its edge.
pub const THICK: f64 = 44.0;
pub const CARD_W: f64 = 300.0;
pub const CARD_H: f64 = 290.0;
/// Air between the strip and the edge of the work area.
pub const MARGIN: f64 = 6.0;
/// Room for the percentage under a ring (`StripPrefs::show_percent`).
pub const LABEL: f64 = 12.0;

/// A monitor work area: menu bar, dock and taskbar already subtracted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Length of the strip along its edge. An empty strip still shows one cell,
/// so there is always something to grab.
pub fn strip_len(rings: usize) -> f64 {
    2.0 * PAD + GRIP + rings.max(1) as f64 * CELL
}

pub fn window_size(edge: Edge, rings: usize, expanded: bool) -> (f64, f64) {
    window_size_for(edge, rings, expanded, 1.0, false)
}

/// [`window_size`] for a strip drawn at `scale` (the page zooms everything by
/// the same factor) and with a percentage under each ring. On a horizontal
/// edge the label makes the strip thicker; on a vertical one every cell
/// longer.
pub fn window_size_for(
    edge: Edge,
    rings: usize,
    expanded: bool,
    scale: f64,
    labels: bool,
) -> (f64, f64) {
    let label = if labels { LABEL } else { 0.0 };
    let (len, thick) = if edge.is_vertical() {
        (strip_len(rings) + rings.max(1) as f64 * label, THICK)
    } else {
        (strip_len(rings), THICK + label)
    };
    let s = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let (along, across) = match (expanded, edge.is_vertical()) {
        (false, _) => (len, thick),
        (true, false) => (len.max(CARD_W), thick + CARD_H),
        (true, true) => (len.max(CARD_H), thick + CARD_W),
    };
    let (along, across) = (along * s, across * s);
    if edge.is_vertical() {
        (across, along)
    } else {
        (along, across)
    }
}

fn clamp_start(center: f64, size: f64, lo: f64, span: f64) -> f64 {
    let min = lo + MARGIN.min(((span - size) / 2.0).max(0.0));
    let max = (lo + span - size - MARGIN).max(min);
    (center - size / 2.0).clamp(min, max)
}

/// Where a window of `size` goes: glued to `edge`, centred on `along` (0..1
/// of the edge), and pushed back inside when that would hang off the area.
pub fn window_rect(edge: Edge, along: f64, area: Area, size: (f64, f64)) -> Rect {
    let (w, h) = size;
    let along = along.clamp(0.0, 1.0);
    let (x, y) = match edge {
        Edge::Top | Edge::Bottom => {
            let x = clamp_start(area.x + along * area.w, w, area.x, area.w);
            let y = if edge == Edge::Top {
                area.y + MARGIN
            } else {
                area.y + (area.h - h - MARGIN).max(0.0)
            };
            (x, y)
        }
        Edge::Left | Edge::Right => {
            let y = clamp_start(area.y + along * area.h, h, area.y, area.h);
            let x = if edge == Edge::Left {
                area.x + MARGIN
            } else {
                area.x + (area.w - w - MARGIN).max(0.0)
            };
            (x, y)
        }
    };
    Rect { x, y, w, h }
}

/// How far, along the edge, the strip sits from the start of the expanded
/// window, so the rings do not move under the cursor when the card opens.
pub fn strip_offset(edge: Edge, collapsed: Rect, expanded: Rect) -> f64 {
    if edge.is_vertical() {
        (collapsed.y - expanded.y).max(0.0)
    } else {
        (collapsed.x - expanded.x).max(0.0)
    }
}

/// The edge nearest to where the strip was dropped, and the spot along it.
/// Ties go to the first of top, bottom, left, right.
pub fn snap(center_x: f64, center_y: f64, area: Area) -> (Edge, f64) {
    let candidates = [
        (Edge::Top, center_y - area.y),
        (Edge::Bottom, area.y + area.h - center_y),
        (Edge::Left, center_x - area.x),
        (Edge::Right, area.x + area.w - center_x),
    ];
    let mut best = candidates[0];
    for c in candidates {
        if c.1 < best.1 {
            best = c;
        }
    }
    let edge = best.0;
    let along = if edge.is_vertical() {
        (center_y - area.y) / area.h.max(1.0)
    } else {
        (center_x - area.x) / area.w.max(1.0)
    };
    (edge, along.clamp(0.0, 1.0))
}
