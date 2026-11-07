use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;

use super::draw::{Anchor, Baseline, Bounds, Drawing, Item, Rgb};

const SVG_LONG_SIDE: f64 = 2400.0;
const PDF_PAGE: (f64, f64) = (1190.55, 841.89);
const MARGIN: f64 = 0.03;
const CAP_HEIGHT: f64 = 0.72;

struct Frame {
    bounds: Bounds,
    scale: f64,
    left: f64,
    bottom: f64,
    width: f64,
    height: f64,
}

impl Frame {
    fn new(drawing: &Drawing, long_side: f64, page: Option<(f64, f64)>) -> Result<Self, String> {
        let bounds = drawing
            .bounds()
            .ok_or_else(|| "This drawing has nothing to show.".to_string())?;
        let span = (bounds.max_x - bounds.min_x)
            .max(bounds.max_y - bounds.min_y)
            .max(1e-9);
        let span_x = (bounds.max_x - bounds.min_x).max(span * 1e-3);
        let span_y = (bounds.max_y - bounds.min_y).max(span * 1e-3);
        let margin = long_side * MARGIN;
        let (width, height, scale) = match page {
            Some((long, short)) => {
                let (w, h) = if span_x >= span_y {
                    (long, short)
                } else {
                    (short, long)
                };
                (
                    w,
                    h,
                    ((w - 2.0 * margin) / span_x).min((h - 2.0 * margin) / span_y),
                )
            }
            None => {
                let scale = (long_side - 2.0 * margin) / span_x.max(span_y);
                (
                    (span_x * scale + 2.0 * margin).round(),
                    (span_y * scale + 2.0 * margin).round(),
                    scale,
                )
            }
        };
        let left = (width - span_x * scale) / 2.0;
        let bottom = (height - span_y * scale) / 2.0;
        Ok(Self {
            bounds,
            scale,
            left,
            bottom,
            width,
            height,
        })
    }

    fn x(&self, x: f64) -> f64 {
        self.left + (x - self.bounds.min_x) * self.scale
    }

    fn y_down(&self, y: f64) -> f64 {
        self.bottom + (self.bounds.max_y - y) * self.scale
    }

    fn y_up(&self, y: f64) -> f64 {
        self.bottom + (y - self.bounds.min_y) * self.scale
    }
}

fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

fn escape_xml(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn to_svg(drawing: &Drawing) -> Result<String, String> {
    let f = Frame::new(drawing, SVG_LONG_SIDE, None)?;
    let mut paths: BTreeMap<Rgb, String> = BTreeMap::new();
    let mut texts = String::new();
    for item in &drawing.items {
        match item {
            Item::Line { points, color } => {
                let d = paths.entry(*color).or_default();
                for (i, (x, y)) in points.iter().enumerate() {
                    let _ = write!(
                        d,
                        "{}{:.2} {:.2}",
                        if i == 0 { "M" } else { "L" },
                        f.x(*x),
                        f.y_down(*y)
                    );
                }
            }
            Item::Text {
                x,
                y,
                height,
                rotation,
                text,
                color,
                anchor,
                baseline,
            } => {
                let (px, py) = (f.x(*x), f.y_down(*y));
                let size = height * f.scale / CAP_HEIGHT;
                let anchor = match anchor {
                    Anchor::Start => "start",
                    Anchor::Middle => "middle",
                    Anchor::End => "end",
                };
                let baseline = match baseline {
                    Baseline::Alphabetic => "alphabetic",
                    Baseline::Middle => "central",
                    Baseline::Hanging => "hanging",
                };
                let _ = writeln!(
                    texts,
                    r#"<text x="{px:.2}" y="{py:.2}" font-size="{size:.2}" fill="{}" text-anchor="{anchor}" dominant-baseline="{baseline}" transform="rotate({:.3} {px:.2} {py:.2})">{}</text>"#,
                    hex(*color),
                    -rotation.to_degrees(),
                    escape_xml(text)
                );
            }
        }
    }
    let mut svg = format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" version="1.1" width="{w}" height="{h}" viewBox="0 0 {w} {h}">
<rect width="100%" height="100%" fill="#ffffff"/>
<g fill="none" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round">
"##,
        w = f.width,
        h = f.height
    );
    for (color, d) in &paths {
        let _ = writeln!(svg, r#"<path stroke="{}" d="{d}"/>"#, hex(*color));
    }
    svg.push_str("</g>\n<g font-family=\"Arial, Helvetica, sans-serif\">\n");
    svg.push_str(&texts);
    svg.push_str("</g>\n</svg>\n");
    Ok(svg)
}

fn pdf_string(text: &str) -> Vec<u8> {
    let mut out = vec![b'('];
    for c in text.chars() {
        let mut buf = [0u8; 4];
        let (bytes, _, unmappable) = encoding_rs::WINDOWS_1252.encode(c.encode_utf8(&mut buf));
        let byte = if unmappable || bytes.len() != 1 {
            b'?'
        } else {
            bytes[0]
        };
        match byte {
            b'(' | b')' | b'\\' => out.extend([b'\\', byte]),
            0x20..=0x7e => out.push(byte),
            _ => out.extend(format!("\\{byte:03o}").bytes()),
        }
    }
    out.push(b')');
    out
}

pub fn to_pdf(drawing: &Drawing) -> Result<Vec<u8>, String> {
    let f = Frame::new(drawing, PDF_PAGE.0, Some(PDF_PAGE))?;
    let mut content: Vec<u8> = b"1 J 1 j 0.5 w\n".to_vec();
    let mut by_color: BTreeMap<Rgb, Vec<&[(f64, f64)]>> = BTreeMap::new();
    for item in &drawing.items {
        if let Item::Line { points, color } = item {
            by_color.entry(*color).or_default().push(points.as_slice());
        }
    }
    let unit = |c: u8| f64::from(c) / 255.0;
    for (Rgb(r, g, b), lines) in by_color {
        let _ = writeln!(content, "{:.3} {:.3} {:.3} RG", unit(r), unit(g), unit(b));
        for points in lines {
            for (i, (x, y)) in points.iter().enumerate() {
                let _ = writeln!(
                    content,
                    "{:.2} {:.2} {}",
                    f.x(*x),
                    f.y_up(*y),
                    if i == 0 { "m" } else { "l" }
                );
            }
            content.extend(b"S\n");
        }
    }
    for item in &drawing.items {
        let Item::Text {
            x,
            y,
            height,
            rotation,
            text,
            color,
            anchor,
            baseline,
        } = item
        else {
            continue;
        };
        let size = height * f.scale / CAP_HEIGHT;
        let width = 0.5 * size * text.chars().count() as f64;
        let along = match anchor {
            Anchor::Start => 0.0,
            Anchor::Middle => -width / 2.0,
            Anchor::End => -width,
        };
        let across = match baseline {
            Baseline::Alphabetic => 0.0,
            Baseline::Middle => -0.36 * size,
            Baseline::Hanging => -0.72 * size,
        };
        let (cos, sin) = (rotation.cos(), rotation.sin());
        let tx = f.x(*x) + along * cos - across * sin;
        let ty = f.y_up(*y) + along * sin + across * cos;
        let _ = write!(
            content,
            "BT /F1 {size:.2} Tf {:.3} {:.3} {:.3} rg {cos:.5} {sin:.5} {:.5} {cos:.5} {tx:.2} {ty:.2} Tm ",
            unit(color.0),
            unit(color.1),
            unit(color.2),
            -sin
        );
        content.extend(pdf_string(text));
        content.extend(b" Tj ET\n");
    }
    let mut zip = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    zip.write_all(&content).map_err(|e| e.to_string())?;
    let stream = zip.finish().map_err(|e| e.to_string())?;

    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    let mut object = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        let _ = writeln!(out, "{} 0 obj", offsets.len());
        out.extend(body);
        out.extend(b"\nendobj\n");
    };
    object(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    object(&mut out, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    object(
        &mut out,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
            f.width, f.height
        )
        .as_bytes(),
    );
    let mut body = format!(
        "<< /Length {} /Filter /FlateDecode >>\nstream\n",
        stream.len()
    )
    .into_bytes();
    body.extend(&stream);
    body.extend(b"\nendstream");
    object(&mut out, &body);
    object(
        &mut out,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    );
    let xref = out.len();
    let _ = write!(out, "xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1);
    for offset in &offsets {
        let _ = writeln!(out, "{offset:010} 00000 n ");
    }
    let _ = write!(
        out,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        offsets.len() + 1
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    fn sample() -> Drawing {
        Drawing {
            items: vec![
                Item::Line {
                    points: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 50.0)],
                    color: Rgb(255, 0, 0),
                },
                Item::Text {
                    x: 10.0,
                    y: 10.0,
                    height: 5.0,
                    rotation: 0.0,
                    text: "Sala (2) & 45°".into(),
                    color: Rgb(0, 0, 0),
                    anchor: Anchor::Start,
                    baseline: Baseline::Alphabetic,
                },
            ],
        }
    }

    #[test]
    fn svg_keeps_aspect_and_flips_y() {
        let svg = to_svg(&sample()).unwrap();
        assert!(svg.contains(r#"width="2400" height="1272""#), "{svg}");
        assert!(svg.contains("Sala (2) &amp; 45°"));
        assert!(
            svg.contains(r##"stroke="#ff0000" d="M72.00 1200.00L2328.00 1200.00L2328.00 72.00""##),
            "{svg}"
        );
    }

    #[test]
    fn pdf_has_a_valid_xref_and_drawing_commands() {
        let pdf = to_pdf(&sample()).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"));
        let find = |needle: &[u8]| {
            pdf.windows(needle.len())
                .rposition(|w| w == needle)
                .unwrap()
        };
        let tail = String::from_utf8(pdf[find(b"startxref\n")..].to_vec()).unwrap();
        let start: usize = tail.lines().nth(1).unwrap().parse().unwrap();
        let xref = String::from_utf8(pdf[start..].to_vec()).unwrap();
        assert!(xref.starts_with("xref\n0 6\n"));
        for (n, line) in xref.lines().skip(3).take(5).enumerate() {
            let offset: usize = line[..10].parse().unwrap();
            assert!(
                pdf[offset..].starts_with(format!("{} 0 obj", n + 1).as_bytes()),
                "object {}",
                n + 1
            );
        }
        let begin = pdf.windows(7).position(|w| w == b"stream\n").unwrap() + 7;
        let end = pdf.windows(10).position(|w| w == b"\nendstream").unwrap();
        let mut content = String::new();
        flate2::read::ZlibDecoder::new(&pdf[begin..end])
            .read_to_string(&mut content)
            .unwrap();
        assert!(content.contains("1.000 0.000 0.000 RG"));
        assert!(content.contains(r"(Sala \(2\) & 45\260) Tj"), "{content}");
    }

    #[test]
    fn control_characters_never_break_the_svg() {
        let mut drawing = sample();
        if let Item::Text { text, .. } = &mut drawing.items[1] {
            *text = "Room\u{1}\u{8} 1".into();
        }
        let svg = to_svg(&drawing).unwrap();
        assert!(svg.contains(">Room 1</text>"));
        crate::core::convert::svg::rasterize(svg.as_bytes()).unwrap();
    }

    #[test]
    fn empty_drawings_are_refused() {
        assert!(to_svg(&Drawing::default()).is_err());
    }
}
