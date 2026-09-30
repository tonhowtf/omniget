//! Palette slots (the contract lives in `omniget_citygen::types`) and the
//! default colours each kit style previews with. Cities pick their own
//! palettes; these are only what the kit GLB carries as base colour.

use omniget_citygen::types::PALETTE_SLOTS;

pub fn slot_index(name: &str) -> Option<u8> {
    PALETTE_SLOTS
        .iter()
        .position(|s| *s == name)
        .map(|i| i as u8)
}

pub fn slot_name(i: u8) -> &'static str {
    PALETTE_SLOTS[i as usize]
}

/// `(slot, #rrggbb)` for a style, in [`PALETTE_SLOTS`] order.
pub fn default_palette(style: &str) -> Vec<(&'static str, &'static str)> {
    let base: [&str; 16] = match style {
        "vila-praiana" => [
            "#F3D9A4", "#5E7F86", "#F7F3EA", "#CBA568", "#B8AE9C", "#263C37", "#B9654C", "#D8C79F",
            "#7C9B71", "#679FA4", "#A89F8E", "#E6D8B8", "#6F9A62", "#6B4A34", "#263C37", "#D96C5A",
        ],
        "vila-arida" => [
            "#D9B98C", "#C9A77C", "#8A5A3B", "#8A5A3B", "#B09A7A", "#263C37", "#6B4A34", "#D4B98E",
            "#BFB27A", "#679FA4", "#B8A58A", "#E0CBA5", "#8FA36B", "#6B4A34", "#263C37", "#B9654C",
        ],
        _ => [
            "#F1E7D0", "#B9654C", "#436D59", "#8A5A3B", "#B8AE9C", "#263C37", "#436D59", "#C9B98F",
            "#7C9B71", "#679FA4", "#A89F8E", "#D8CBB0", "#7C9B71", "#6B4A34", "#263C37", "#B9654C",
        ],
    };
    PALETTE_SLOTS.iter().copied().zip(base).collect()
}

pub fn rgb(hex: &str) -> [u8; 3] {
    let h = hex.trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("80"), 16).unwrap_or(128);
    [p(0), p(2), p(4)]
}
