use super::super::proto::DanmakuElem;
use super::lanes::{ScrollLaneSet, StaticLaneSet};

#[derive(Debug, Clone)]
pub struct AssRenderOptions {
    pub resolution_x: u32,
    pub resolution_y: u32,
    pub font_name: String,
    pub font_size: u32,
    pub scroll_duration_secs: f64,
    pub static_duration_secs: f64,
    pub alpha: u8,
    pub bold: bool,
    pub outline: f32,
    pub shadow: f32,
    pub avg_char_width_px: f32,
}

impl Default for AssRenderOptions {
    fn default() -> Self {
        Self {
            resolution_x: 1920,
            resolution_y: 1080,
            font_name: "Microsoft YaHei".to_string(),
            font_size: 42,
            scroll_duration_secs: 12.0,
            static_duration_secs: 5.0,
            alpha: 0x40,
            bold: false,
            outline: 1.0,
            shadow: 0.0,
            avg_char_width_px: 25.0,
        }
    }
}

pub fn render_ass(elems: &[DanmakuElem], opts: &AssRenderOptions) -> String {
    let scroll_lane_count = ((opts.resolution_y as f32) / (opts.font_size as f32 * 1.2)) as usize;
    let static_lane_count = scroll_lane_count;
    let mut scroll = ScrollLaneSet::new(scroll_lane_count.max(8));
    let mut top = StaticLaneSet::new(static_lane_count.max(8));
    let mut bottom = StaticLaneSet::new(static_lane_count.max(8));

    let mut sorted: Vec<&DanmakuElem> = elems.iter().collect();
    sorted.sort_by_key(|e| e.progress_ms);

    let mut events: Vec<String> = Vec::new();
    let line_height = (opts.font_size as f32 * 1.2) as i32;

    for elem in sorted {
        if elem.content.is_empty() {
            continue;
        }
        let start_secs = (elem.progress_ms as f64) / 1000.0;
        let text_width_px = (elem.content.chars().count() as f32) * opts.avg_char_width_px;
        let mode = elem.mode;

        let line = match mode {
            1 | 2 | 3 => {
                let exit_secs = start_secs + opts.scroll_duration_secs;
                let lane_idx = scroll.allocate(start_secs, exit_secs);
                let y = line_height + (lane_idx as i32) * line_height;
                let x1 = opts.resolution_x as i32;
                let x2 = -(text_width_px as i32);
                build_scroll_event(elem, start_secs, exit_secs, x1, x2, y, opts)
            }
            4 => {
                let end_secs = start_secs + opts.static_duration_secs;
                let lane_idx = bottom.allocate(start_secs, end_secs);
                let y = (opts.resolution_y as i32) - line_height - (lane_idx as i32) * line_height;
                let x = (opts.resolution_x as i32) / 2;
                build_static_event(elem, start_secs, end_secs, x, y, opts)
            }
            5 => {
                let end_secs = start_secs + opts.static_duration_secs;
                let lane_idx = top.allocate(start_secs, end_secs);
                let y = line_height + (lane_idx as i32) * line_height;
                let x = (opts.resolution_x as i32) / 2;
                build_static_event(elem, start_secs, end_secs, x, y, opts)
            }
            _ => continue,
        };
        events.push(line);
    }

    let mut out = String::new();
    out.push_str("[Script Info]\n");
    out.push_str("ScriptType: v4.00+\n");
    out.push_str("Collisions: Normal\n");
    out.push_str(&format!("PlayResX: {}\n", opts.resolution_x));
    out.push_str(&format!("PlayResY: {}\n", opts.resolution_y));
    out.push_str("Timer: 100.0000\n");
    out.push_str("WrapStyle: 2\n\n");

    out.push_str("[V4+ Styles]\n");
    out.push_str("Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n");
    out.push_str(&format!(
        "Style: Danmaku,{font},{size},&H{alpha:02X}FFFFFF,&H{alpha:02X}FFFFFF,&H{alpha:02X}000000,&H{alpha:02X}000000,{bold},0,0,0,100,100,0,0,1,{outline:.1},{shadow:.1},7,0,0,0,1\n\n",
        font = opts.font_name,
        size = opts.font_size,
        alpha = opts.alpha,
        bold = if opts.bold { -1 } else { 0 },
        outline = opts.outline,
        shadow = opts.shadow,
    ));

    out.push_str("[Events]\n");
    out.push_str(
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    );
    for ev in events {
        out.push_str(&ev);
        out.push('\n');
    }
    out
}

fn build_scroll_event(
    elem: &DanmakuElem,
    start_secs: f64,
    end_secs: f64,
    x1: i32,
    x2: i32,
    y: i32,
    opts: &AssRenderOptions,
) -> String {
    let start = ass_time(start_secs);
    let end = ass_time(end_secs);
    let color_tag = ass_color_tag(elem.color);
    let size_tag = ass_size_tag(elem.fontsize, opts.font_size);
    let text = escape_ass(&elem.content);
    format!(
        "Dialogue: 0,{},{},Danmaku,,0,0,0,,{{\\move({},{},{},{}){}{}}}{}",
        start, end, x1, y, x2, y, size_tag, color_tag, text
    )
}

fn build_static_event(
    elem: &DanmakuElem,
    start_secs: f64,
    end_secs: f64,
    x: i32,
    y: i32,
    opts: &AssRenderOptions,
) -> String {
    let start = ass_time(start_secs);
    let end = ass_time(end_secs);
    let color_tag = ass_color_tag(elem.color);
    let size_tag = ass_size_tag(elem.fontsize, opts.font_size);
    let text = escape_ass(&elem.content);
    format!(
        "Dialogue: 0,{},{},Danmaku,,0,0,0,,{{\\an8\\pos({},{}){}{}}}{}",
        start, end, x, y, size_tag, color_tag, text
    )
}

fn ass_time(seconds: f64) -> String {
    let total_cs = (seconds * 100.0).round() as i64;
    let hours = total_cs / 360_000;
    let rem = total_cs % 360_000;
    let minutes = rem / 6000;
    let rem = rem % 6000;
    let secs = rem / 100;
    let cs = rem % 100;
    format!("{:01}:{:02}:{:02}.{:02}", hours, minutes, secs, cs)
}

fn ass_color_tag(rgb: u32) -> String {
    if rgb == 0xFFFFFF || rgb == 0 {
        return String::new();
    }
    let r = (rgb >> 16) & 0xFF;
    let g = (rgb >> 8) & 0xFF;
    let b = rgb & 0xFF;
    format!("\\c&H{:02X}{:02X}{:02X}&", b, g, r)
}

fn ass_size_tag(actual: i32, default_size: u32) -> String {
    if actual <= 0 {
        return String::new();
    }
    if actual == default_size as i32 {
        return String::new();
    }
    format!("\\fs{}", actual)
}

fn escape_ass(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\N"),
            '\r' => continue,
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elem(progress_ms: i32, mode: i32, content: &str) -> DanmakuElem {
        DanmakuElem {
            progress_ms,
            mode,
            fontsize: 25,
            color: 0xFFFFFF,
            content: content.to_string(),
            ..DanmakuElem::default()
        }
    }

    #[test]
    fn renders_header_and_events() {
        let elems = vec![
            elem(0, 1, "scroll line"),
            elem(1000, 5, "top line"),
            elem(2000, 4, "bottom line"),
        ];
        let ass = render_ass(&elems, &AssRenderOptions::default());
        assert!(ass.contains("[Script Info]"));
        assert!(ass.contains("[V4+ Styles]"));
        assert!(ass.contains("[Events]"));
        assert!(ass.contains("scroll line"));
        assert!(ass.contains("top line"));
        assert!(ass.contains("bottom line"));
        assert!(ass.contains("\\move("));
        assert!(ass.contains("\\pos("));
    }

    #[test]
    fn escapes_braces_in_content() {
        let elems = vec![elem(0, 1, "hello {world}")];
        let ass = render_ass(&elems, &AssRenderOptions::default());
        assert!(ass.contains("\\{world\\}"));
    }

    #[test]
    fn color_tag_omitted_for_white() {
        let elems = vec![elem(0, 1, "white text")];
        let ass = render_ass(&elems, &AssRenderOptions::default());
        for line in ass.lines() {
            if line.starts_with("Dialogue:") {
                assert!(!line.contains("\\c&H"));
            }
        }
    }

    #[test]
    fn time_format() {
        assert_eq!(ass_time(0.0), "0:00:00.00");
        assert_eq!(ass_time(65.5), "0:01:05.50");
        assert_eq!(ass_time(3661.25), "1:01:01.25");
    }
}
