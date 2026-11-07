use std::f64::consts::TAU;

use acadrust::entities::{
    EntityType, HorizontalAlignment, TextHorizontalAlignment, TextVerticalAlignment,
    VerticalAlignment,
};
use acadrust::types::{Color, Vector3};
use acadrust::CadDocument;

const MAX_DEPTH: usize = 32;
const MAX_ITEMS: usize = 2_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Start,
    Middle,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Baseline {
    Alphabetic,
    Middle,
    Hanging,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Line {
        points: Vec<(f64, f64)>,
        color: Rgb,
    },
    Text {
        x: f64,
        y: f64,
        height: f64,
        rotation: f64,
        text: String,
        color: Rgb,
        anchor: Anchor,
        baseline: Baseline,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

#[derive(Debug, Default)]
pub struct Drawing {
    pub items: Vec<Item>,
}

impl Drawing {
    pub fn bounds(&self) -> Option<Bounds> {
        let mut b: Option<Bounds> = None;
        let mut add = |x: f64, y: f64| {
            if !x.is_finite() || !y.is_finite() {
                return;
            }
            let e = b.get_or_insert(Bounds {
                min_x: x,
                min_y: y,
                max_x: x,
                max_y: y,
            });
            e.min_x = e.min_x.min(x);
            e.min_y = e.min_y.min(y);
            e.max_x = e.max_x.max(x);
            e.max_y = e.max_y.max(y);
        };
        for item in &self.items {
            match item {
                Item::Line { points, .. } => points.iter().for_each(|(x, y)| add(*x, *y)),
                Item::Text {
                    x,
                    y,
                    height,
                    text,
                    rotation,
                    ..
                } => {
                    let width = height * 0.6 * text.chars().count() as f64;
                    add(*x, *y);
                    add(x + width * rotation.cos(), y + width * rotation.sin());
                    add(x - height * rotation.sin(), y + height * rotation.cos());
                }
            }
        }
        b
    }
}

struct Inherited {
    layer: String,
    color: Rgb,
}

struct Collector<'a> {
    doc: &'a CadDocument,
    out: Drawing,
}

pub fn collect(doc: &CadDocument) -> Result<Drawing, String> {
    let mut c = Collector {
        doc,
        out: Drawing::default(),
    };
    for entity in doc.model_space_entities() {
        c.visit(entity, None, 0)?;
    }
    if c.out.items.is_empty() {
        return Err("This drawing has nothing to show in model space.".into());
    }
    Ok(c.out)
}

const MAX_LUMA: f64 = 0.55;

fn ink(rgb: (u8, u8, u8)) -> Rgb {
    if rgb.0 > 240 && rgb.1 > 240 && rgb.2 > 240 {
        return Rgb(0, 0, 0);
    }
    let luma =
        (0.2126 * f64::from(rgb.0) + 0.7152 * f64::from(rgb.1) + 0.0722 * f64::from(rgb.2)) / 255.0;
    if luma <= MAX_LUMA {
        return Rgb(rgb.0, rgb.1, rgb.2);
    }
    let k = MAX_LUMA / luma;
    let dim = |c: u8| (f64::from(c) * k).round() as u8;
    Rgb(dim(rgb.0), dim(rgb.1), dim(rgb.2))
}

fn clean_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' && chars.peek() == Some(&'%') {
            chars.next();
            match chars.next().map(|c| c.to_ascii_lowercase()) {
                Some('d') => out.push('°'),
                Some('c') => out.push('⌀'),
                Some('p') => out.push('±'),
                Some('%') => out.push('%'),
                Some('u' | 'o') | None => {}
                Some(other) => {
                    out.push_str("%%");
                    out.push(other);
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn arc_points(
    center: Vector3,
    radius: f64,
    start: f64,
    end: f64,
    normal: Vector3,
) -> Vec<(f64, f64)> {
    let (cx, start, end) = if normal.z < 0.0 {
        (
            -center.x,
            std::f64::consts::PI - end,
            std::f64::consts::PI - start,
        )
    } else {
        (center.x, start, end)
    };
    let mut span = (end - start).rem_euclid(TAU);
    if span < 1e-9 {
        span = TAU;
    }
    let steps = ((span / TAU) * 96.0).ceil().max(4.0) as usize;
    (0..=steps)
        .map(|i| {
            let a = start + span * i as f64 / steps as f64;
            (cx + radius * a.cos(), center.y + radius * a.sin())
        })
        .collect()
}

impl Collector<'_> {
    fn push(&mut self, item: Item) -> Result<(), String> {
        let item = match item {
            Item::Line { points, color } => {
                let points: Vec<(f64, f64)> = points
                    .into_iter()
                    .filter(|(x, y)| x.is_finite() && y.is_finite())
                    .collect();
                if points.len() < 2 {
                    return Ok(());
                }
                Item::Line { points, color }
            }
            Item::Text { x, y, rotation, .. }
                if !(x.is_finite() && y.is_finite() && rotation.is_finite()) =>
            {
                return Ok(())
            }
            text => text,
        };
        if self.out.items.len() >= MAX_ITEMS {
            return Err("This drawing is too complex to convert.".into());
        }
        self.out.items.push(item);
        Ok(())
    }

    fn style(&self, entity: &EntityType, inherited: Option<&Inherited>) -> Option<(String, Rgb)> {
        let common = entity.common();
        if common.invisible {
            return None;
        }
        let layer_name = match inherited {
            Some(parent) if common.layer == "0" => parent.layer.clone(),
            _ => common.layer.clone(),
        };
        let layer = self.doc.layers.get(&layer_name);
        if layer.is_some_and(|l| !l.is_visible()) {
            return None;
        }
        let layer_color = layer
            .and_then(|l| l.color.rgb())
            .map(ink)
            .unwrap_or(Rgb(0, 0, 0));
        let color = match common.color {
            Color::ByLayer => layer_color,
            Color::ByBlock => inherited.map_or(Rgb(0, 0, 0), |p| p.color),
            Color::None => return None,
            other => other.rgb().map(ink).unwrap_or(layer_color),
        };
        Some((layer_name, color))
    }

    fn visit(
        &mut self,
        entity: &EntityType,
        inherited: Option<&Inherited>,
        depth: usize,
    ) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Ok(());
        }
        let Some((layer, color)) = self.style(entity, inherited) else {
            return Ok(());
        };
        match entity {
            EntityType::Line(l) => self.push(Item::Line {
                points: vec![(l.start.x, l.start.y), (l.end.x, l.end.y)],
                color,
            }),
            EntityType::Arc(a) => self.push(Item::Line {
                points: arc_points(a.center, a.radius, a.start_angle, a.end_angle, a.normal),
                color,
            }),
            EntityType::Text(t) => {
                let anchor = match t.horizontal_alignment {
                    TextHorizontalAlignment::Center | TextHorizontalAlignment::Middle => {
                        Anchor::Middle
                    }
                    TextHorizontalAlignment::Right => Anchor::End,
                    _ => Anchor::Start,
                };
                let baseline = match (t.horizontal_alignment, t.vertical_alignment) {
                    (TextHorizontalAlignment::Middle, _) | (_, TextVerticalAlignment::Middle) => {
                        Baseline::Middle
                    }
                    (_, TextVerticalAlignment::Top) => Baseline::Hanging,
                    _ => Baseline::Alphabetic,
                };
                let at = match (anchor, baseline, t.alignment_point) {
                    (Anchor::Start, Baseline::Alphabetic, _) | (_, _, None) => t.insertion_point,
                    (_, _, Some(p)) => p,
                };
                self.text(&t.value, at, t.height, t.rotation, color, anchor, baseline)
            }
            EntityType::AttributeEntity(a) => {
                if a.flags.invisible {
                    return Ok(());
                }
                let anchor = match a.horizontal_alignment {
                    HorizontalAlignment::Center | HorizontalAlignment::Middle => Anchor::Middle,
                    HorizontalAlignment::Right => Anchor::End,
                    _ => Anchor::Start,
                };
                let baseline = match a.vertical_alignment {
                    VerticalAlignment::Middle => Baseline::Middle,
                    VerticalAlignment::Top => Baseline::Hanging,
                    _ => Baseline::Alphabetic,
                };
                let at = if anchor == Anchor::Start && baseline == Baseline::Alphabetic {
                    a.insertion_point
                } else {
                    a.alignment_point
                };
                self.text(&a.value, at, a.height, a.rotation, color, anchor, baseline)
            }
            EntityType::Insert(insert) => {
                let parent = Inherited { layer, color };
                for part in insert.explode_from_document(self.doc) {
                    self.visit(&part, Some(&parent), depth + 1)?;
                }
                for attribute in &insert.attributes {
                    self.visit(
                        &EntityType::AttributeEntity(attribute.clone()),
                        Some(&parent),
                        depth + 1,
                    )?;
                }
                Ok(())
            }
            EntityType::Point(_)
            | EntityType::Ray(_)
            | EntityType::XLine(_)
            | EntityType::Viewport(_)
            | EntityType::Block(_)
            | EntityType::BlockEnd(_)
            | EntityType::Seqend(_)
            | EntityType::AttributeDefinition(_) => Ok(()),
            other => {
                let mut parts = other.flatten();
                if parts.is_empty() {
                    parts = other.explode();
                }
                for part in &parts {
                    self.visit(part, inherited, depth + 1)?;
                }
                Ok(())
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        raw: &str,
        at: Vector3,
        height: f64,
        rotation: f64,
        color: Rgb,
        anchor: Anchor,
        baseline: Baseline,
    ) -> Result<(), String> {
        let text = clean_text(raw);
        if text.trim().is_empty() || !(height.is_finite() && height > 0.0) {
            return Ok(());
        }
        self.push(Item::Text {
            x: at.x,
            y: at.y,
            height,
            rotation,
            text,
            color,
            anchor,
            baseline,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arcs_turn_counterclockwise_and_mirror_with_negative_normals() {
        let up = Vector3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        };
        let pts = arc_points(
            Vector3 {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
            5.0,
            0.0,
            std::f64::consts::FRAC_PI_2,
            up,
        );
        let (first, last) = (pts[0], pts[pts.len() - 1]);
        assert!((first.0 - 15.0).abs() < 1e-9 && first.1.abs() < 1e-9);
        assert!((last.0 - 10.0).abs() < 1e-9 && (last.1 - 5.0).abs() < 1e-9);
        let down = Vector3 {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        };
        let mirrored = arc_points(
            Vector3 {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
            5.0,
            0.0,
            std::f64::consts::FRAC_PI_2,
            down,
        );
        assert!(mirrored.iter().all(|(x, _)| *x <= -5.0 + 1e-9));
        let full = arc_points(
            Vector3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            1.0,
            0.0,
            0.0,
            up,
        );
        assert!(full.len() > 90);
    }

    #[test]
    fn broken_coordinates_are_dropped() {
        let doc = CadDocument::new();
        let mut c = Collector {
            doc: &doc,
            out: Drawing::default(),
        };
        let black = Rgb(0, 0, 0);
        c.push(Item::Line {
            points: vec![(0.0, 0.0), (f64::NAN, 1.0), (2.0, 2.0)],
            color: black,
        })
        .unwrap();
        c.push(Item::Line {
            points: vec![(0.0, 0.0), (f64::INFINITY, 1.0)],
            color: black,
        })
        .unwrap();
        c.push(Item::Text {
            x: 0.0,
            y: 0.0,
            height: 1.0,
            rotation: f64::NAN,
            text: "x".into(),
            color: black,
            anchor: Anchor::Start,
            baseline: Baseline::Alphabetic,
        })
        .unwrap();
        assert_eq!(
            c.out.items,
            vec![Item::Line {
                points: vec![(0.0, 0.0), (2.0, 2.0)],
                color: black,
            }]
        );
    }

    #[test]
    fn special_codes_become_symbols() {
        assert_eq!(clean_text("45%%d %%c10 %%p0.5 100%%%"), "45° ⌀10 ±0.5 100%");
        assert_eq!(clean_text("%%uUnderlined%%u"), "Underlined");
    }

    #[test]
    fn colors_stay_readable_on_white_paper() {
        assert_eq!(ink((255, 255, 255)), Rgb(0, 0, 0));
        assert_eq!(ink((255, 0, 0)), Rgb(255, 0, 0));
        let Rgb(r, g, b) = ink((255, 255, 0));
        assert_eq!(r, g);
        assert!(r < 160 && b == 0, "{r} {g} {b}");
    }
}
