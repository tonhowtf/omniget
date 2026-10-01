use super::model::{CanvasNode, FloorDoc, Point, Rect, GRID};
use super::ops::Op;
use serde::{Deserialize, Serialize};

pub const GAP: f64 = 40.0;

fn snap(v: f64) -> f64 {
    (v / GRID).round() * GRID
}

fn obstacles(doc: &FloorDoc, skip: &[&str]) -> Vec<Rect> {
    doc.nodes
        .iter()
        .filter(|n| n.is_on_canvas() && !skip.contains(&n.id.as_str()))
        .map(CanvasNode::rect)
        .collect()
}

fn free(r: &Rect, obstacles: &[Rect], gap: f64) -> bool {
    let probe = r.inflate(gap / 2.0);
    obstacles
        .iter()
        .all(|o| !probe.intersects(&o.inflate(gap / 2.0)))
}

pub fn nearest_free_spot(obstacles: &[Rect], w: f64, h: f64, near: Point, gap: f64) -> Point {
    let start = Point {
        x: snap(near.x),
        y: snap(near.y),
    };
    let at = |dx: i64, dy: i64| Rect {
        x: start.x + dx as f64 * GRID,
        y: start.y + dy as f64 * GRID,
        w,
        h,
    };
    if free(&at(0, 0), obstacles, gap) {
        return start;
    }
    for ring in 1..=1000i64 {
        let mut best: Option<(f64, Rect)> = None;
        for i in -ring..=ring {
            for (dx, dy) in [(i, -ring), (i, ring), (-ring, i), (ring, i)] {
                let r = at(dx, dy);
                if free(&r, obstacles, gap) {
                    let d = ((r.x - near.x).powi(2) + (r.y - near.y).powi(2)).sqrt();
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, r));
                    }
                }
            }
        }
        if let Some((_, r)) = best {
            return Point { x: r.x, y: r.y };
        }
    }
    start
}

pub fn free_spot_in(doc: &FloorDoc, w: f64, h: f64, near: Option<Point>, skip: &[&str]) -> Point {
    let near = near.unwrap_or_else(|| {
        let c = doc.camera;
        Point { x: c.x, y: c.y }
    });
    nearest_free_spot(&obstacles(doc, skip), w, h, near, GAP)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArrangeMode {
    Tidy,
    AlignLeft,
    AlignRight,
    AlignTop,
    AlignBottom,
    AlignCenterX,
    AlignCenterY,
    DistributeX,
    DistributeY,
}

pub type Moves = Vec<(String, Point)>;

pub fn arrange(doc: &FloorDoc, ids: &[String], mode: ArrangeMode) -> Moves {
    let nodes: Vec<&CanvasNode> = ids
        .iter()
        .filter_map(|id| doc.node(id))
        .filter(|n| !n.locked && n.is_on_canvas())
        .collect();
    if nodes.len() < 2 && mode != ArrangeMode::Tidy {
        return Vec::new();
    }
    if nodes.is_empty() {
        return Vec::new();
    }
    let bbox = nodes
        .iter()
        .map(|n| n.rect())
        .reduce(|a, b| a.union(&b))
        .unwrap_or_default();
    let mv = |n: &CanvasNode, x: f64, y: f64| (n.id.clone(), Point { x, y });
    match mode {
        ArrangeMode::AlignLeft => nodes.iter().map(|n| mv(n, bbox.x, n.y)).collect(),
        ArrangeMode::AlignRight => nodes
            .iter()
            .map(|n| mv(n, bbox.right() - n.w, n.y))
            .collect(),
        ArrangeMode::AlignTop => nodes.iter().map(|n| mv(n, n.x, bbox.y)).collect(),
        ArrangeMode::AlignBottom => nodes
            .iter()
            .map(|n| mv(n, n.x, bbox.bottom() - n.h))
            .collect(),
        ArrangeMode::AlignCenterX => {
            let cx = bbox.x + bbox.w / 2.0;
            nodes.iter().map(|n| mv(n, cx - n.w / 2.0, n.y)).collect()
        }
        ArrangeMode::AlignCenterY => {
            let cy = bbox.y + bbox.h / 2.0;
            nodes.iter().map(|n| mv(n, n.x, cy - n.h / 2.0)).collect()
        }
        ArrangeMode::DistributeX => {
            let mut v = nodes.clone();
            v.sort_by(|a, b| a.x.total_cmp(&b.x));
            let total: f64 = v.iter().map(|n| n.w).sum();
            let space = ((bbox.w - total) / (v.len() - 1) as f64).max(0.0);
            let mut x = bbox.x;
            v.iter()
                .map(|n| {
                    let m = mv(n, x, n.y);
                    x += n.w + space;
                    m
                })
                .collect()
        }
        ArrangeMode::DistributeY => {
            let mut v = nodes.clone();
            v.sort_by(|a, b| a.y.total_cmp(&b.y));
            let total: f64 = v.iter().map(|n| n.h).sum();
            let space = ((bbox.h - total) / (v.len() - 1) as f64).max(0.0);
            let mut y = bbox.y;
            v.iter()
                .map(|n| {
                    let m = mv(n, n.x, y);
                    y += n.h + space;
                    m
                })
                .collect()
        }
        ArrangeMode::Tidy => tidy(&nodes, bbox),
    }
}

fn tidy(nodes: &[&CanvasNode], bbox: Rect) -> Moves {
    let mut v: Vec<&CanvasNode> = nodes.to_vec();
    v.sort_by(|a, b| {
        let ra = (a.y / 200.0).floor();
        let rb = (b.y / 200.0).floor();
        ra.total_cmp(&rb).then(a.x.total_cmp(&b.x))
    });
    let per_row = (v.len() as f64).sqrt().ceil().max(1.0) as usize;
    let (x0, mut y) = (snap(bbox.x), snap(bbox.y));
    let mut out = Vec::new();
    for row in v.chunks(per_row) {
        let mut x = x0;
        let mut tallest: f64 = 0.0;
        for n in row {
            out.push((n.id.clone(), Point { x, y }));
            x = snap(x + n.w + GAP);
            tallest = tallest.max(n.h);
        }
        y = snap(y + tallest + GAP);
    }
    out
}

pub fn place_recruits(doc: &FloorDoc, anchor: &Rect, count: usize, size: (f64, f64)) -> Vec<Point> {
    if count == 0 {
        return Vec::new();
    }
    let (w, h) = size;
    let row_w = count as f64 * w + (count - 1) as f64 * GAP;
    let x0 = snap(anchor.x + anchor.w / 2.0 - row_w / 2.0);
    let obs = obstacles(doc, &[]);
    let mut y = snap(anchor.bottom() + GAP * 2.0);
    for _ in 0..200 {
        let spots: Vec<Point> = (0..count)
            .map(|i| Point {
                x: x0 + i as f64 * (w + GAP),
                y,
            })
            .collect();
        let ok = spots.iter().all(|p| {
            free(
                &Rect {
                    x: p.x,
                    y: p.y,
                    w,
                    h,
                },
                &obs,
                GAP,
            )
        });
        if ok {
            return spots;
        }
        y = snap(y + GRID * 4.0);
    }
    let mut obs = obs;
    (0..count)
        .map(|i| {
            let p = nearest_free_spot(
                &obs,
                w,
                h,
                Point {
                    x: x0 + i as f64 * (w + GAP),
                    y: snap(anchor.bottom() + GAP * 2.0),
                },
                GAP,
            );
            obs.push(Rect {
                x: p.x,
                y: p.y,
                w,
                h,
            });
            p
        })
        .collect()
}

pub fn arrange_ops(doc: &FloorDoc, moves: &Moves) -> Vec<Op> {
    moves
        .iter()
        .filter(|(id, p)| doc.node(id).is_some_and(|n| n.x != p.x || n.y != p.y))
        .map(|(id, p)| Op::update_node(id.clone(), serde_json::json!({ "x": p.x, "y": p.y })))
        .collect()
}
