use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::OnceLock;

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

use super::external::ToolStatus;
use super::formats::{self, Category};

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Native,
    Ffmpeg,
    Pandoc,
    ImageMagick,
    LibreOffice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: &'static str,
    pub to: &'static str,
    pub engine: Engine,
    pub cost: u32,
    pub lossy: bool,
}

impl Serialize for Edge {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("Edge", 4)?;
        st.serialize_field("from", self.from)?;
        st.serialize_field("to", self.to)?;
        st.serialize_field("engine", &self.engine)?;
        st.serialize_field("lossy", &self.lossy)?;
        st.end()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Route {
    pub steps: Vec<Edge>,
    pub total_cost: u32,
    pub lossy: bool,
}

impl Route {
    pub fn engines(&self) -> Vec<Engine> {
        let mut out: Vec<Engine> = Vec::new();
        for s in &self.steps {
            if !out.contains(&s.engine) {
                out.push(s.engine);
            }
        }
        out
    }
}

impl Serialize for Route {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("Route", 3)?;
        st.serialize_field("steps", &self.steps)?;
        st.serialize_field("lossy", &self.lossy)?;
        st.serialize_field("engines", &self.engines())?;
        st.end()
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct TargetOption {
    pub format: String,
    pub label: &'static str,
    pub category: Category,
    pub route: Route,
    pub missing_tool: Option<Engine>,
}

const DATA: &[&str] = &[
    "json", "jsonl", "yaml", "toml", "csv", "tsv", "xml", "plist",
];
const RASTER: &[&str] = &["png", "jpeg", "webp", "gif", "bmp", "ico", "tiff", "tga"];
const AUDIO: &[&str] = &["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus"];
const VIDEO: &[&str] = &["mp4", "mkv", "webm", "mov", "avi"];
const PANDOC_IN: &[&str] = &[
    "md", "txt", "html", "docx", "odt", "rtf", "epub", "tex", "rst", "org",
];
const PANDOC_OUT: &[&str] = &[
    "md", "txt", "html", "docx", "odt", "rtf", "epub", "tex", "rst", "org",
];
const OFFICE_IN: &[&str] = &["docx", "odt", "rtf", "html", "txt"];
const OFFICE_OUT: &[&str] = &["pdf", "docx", "odt", "rtf"];
const PHOTO_IN: &[&str] = &[
    #[cfg(feature = "heic")]
    "heic",
    "jxl",
    "psd",
    "raw",
];
const TRACE_IN: &[&str] = &["png", "jpeg", "webp"];

fn e(from: &'static str, to: &'static str, engine: Engine, cost: u32, lossy: bool) -> Edge {
    Edge {
        from,
        to,
        engine,
        cost,
        lossy,
    }
}

fn data_lossy(from: &str, to: &str) -> bool {
    let tabular = |x: &str| x == "csv" || x == "tsv";
    match to {
        "toml" | "xml" | "plist" => true,
        "csv" | "tsv" => !tabular(from),
        _ => from == "xml",
    }
}

fn build_edges() -> Vec<Edge> {
    let mut out = Vec::new();
    for &a in DATA {
        for &b in DATA {
            if a != b {
                out.push(e(a, b, Engine::Native, 1, data_lossy(a, b)));
            }
        }
    }
    for (a, b, lossy) in [
        ("md", "html", false),
        ("html", "md", true),
        ("html", "txt", true),
        ("md", "txt", true),
        ("txt", "md", false),
        ("txt", "html", false),
        ("md", "epub", false),
        ("html", "epub", false),
        ("txt", "epub", false),
        ("epub", "txt", true),
        ("epub", "md", true),
    ] {
        out.push(e(a, b, Engine::Native, 1, lossy));
    }
    for &a in PANDOC_IN {
        for &b in PANDOC_OUT {
            if a != b {
                out.push(e(a, b, Engine::Pandoc, 5, b == "txt" || a == "html"));
            }
        }
    }
    for &a in OFFICE_IN {
        for &b in OFFICE_OUT {
            if a != b {
                out.push(e(a, b, Engine::LibreOffice, 8, false));
            }
        }
    }
    for &a in RASTER {
        for &b in RASTER {
            if a != b {
                out.push(e(
                    a,
                    b,
                    Engine::Native,
                    2,
                    matches!(b, "jpeg" | "gif" | "ico"),
                ));
            }
        }
    }
    for &a in PHOTO_IN {
        for &b in RASTER {
            let lossy = matches!(b, "jpeg" | "gif" | "ico") || matches!(a, "psd" | "raw");
            out.push(e(a, b, Engine::Native, 2, lossy));
        }
    }
    out.push(e("psd", "layers", Engine::Native, 2, false));
    for &a in TRACE_IN {
        out.push(e(a, "svg", Engine::Native, 3, true));
    }
    out.push(e("svg", "png", Engine::Native, 3, false));
    out.push(e("dwg", "dxf", Engine::Native, 3, false));
    for a in ["dxf", "dwg"] {
        for b in ["svg", "pdf"] {
            out.push(e(a, b, Engine::Native, 3, true));
        }
    }
    for a in ["png", "jpeg", "webp"] {
        out.push(e(a, "avif", Engine::ImageMagick, 6, true));
    }
    for b in ["png", "jpeg"] {
        out.push(e("avif", b, Engine::ImageMagick, 6, b == "jpeg"));
    }
    for &a in AUDIO {
        for &b in AUDIO {
            if a != b {
                out.push(e(a, b, Engine::Ffmpeg, 4, !matches!(b, "wav" | "flac")));
            }
        }
    }
    for &a in VIDEO {
        for &b in VIDEO {
            if a != b {
                out.push(e(a, b, Engine::Ffmpeg, 5, b != "mkv"));
            }
        }
        for &b in AUDIO {
            out.push(e(a, b, Engine::Ffmpeg, 4, true));
        }
        out.push(e(a, "gif", Engine::Ffmpeg, 5, true));
    }
    out
}

pub fn edges() -> &'static [Edge] {
    static EDGES: OnceLock<Vec<Edge>> = OnceLock::new();
    EDGES.get_or_init(build_edges)
}

const MAX_HOPS: usize = 3;

fn ends_route(edge: &Edge) -> bool {
    formats::is_folder(edge.to)
        || formats::category_of(edge.from) == Some(Category::Video)
            && formats::category_of(edge.to) == Some(Category::Image)
}

fn weight(edge: &Edge) -> u32 {
    edge.cost * 10 + u32::from(edge.lossy)
}

fn search(from: &str, tools: &ToolStatus) -> HashMap<&'static str, Route> {
    let usable: Vec<&Edge> = edges().iter().filter(|e| tools.has(e.engine)).collect();
    let mut best: HashMap<(&'static str, usize), u32> = HashMap::new();
    let mut paths: HashMap<(&'static str, usize), Vec<Edge>> = HashMap::new();
    let mut heap: BinaryHeap<Reverse<(u32, usize, &'static str, bool)>> = BinaryHeap::new();
    let Some(start) = formats::by_id(from).map(|f| f.id) else {
        return HashMap::new();
    };
    best.insert((start, 0), 0);
    paths.insert((start, 0), Vec::new());
    heap.push(Reverse((0, 0, start, false)));
    let mut found: HashMap<&'static str, (u32, Vec<Edge>)> = HashMap::new();
    while let Some(Reverse((w, hops, node, terminal))) = heap.pop() {
        if best.get(&(node, hops)).is_some_and(|&b| w > b) {
            continue;
        }
        let path = paths.get(&(node, hops)).cloned().unwrap_or_default();
        if node != start {
            let better = found.get(node).is_none_or(|(fw, _)| w < *fw);
            if better {
                found.insert(node, (w, path.clone()));
            }
        }
        if terminal || hops >= MAX_HOPS {
            continue;
        }
        for edge in usable.iter().filter(|e| e.from == node) {
            if edge.to == start || path.iter().any(|p| p.from == edge.to) {
                continue;
            }
            let nw = w + weight(edge);
            let key = (edge.to, hops + 1);
            if best.get(&key).is_none_or(|&b| nw < b) {
                best.insert(key, nw);
                let mut np = path.clone();
                np.push(**edge);
                paths.insert(key, np);
                heap.push(Reverse((nw, hops + 1, edge.to, ends_route(edge))));
            }
        }
    }
    found
        .into_iter()
        .map(|(k, (_, steps))| {
            let total_cost = steps.iter().map(|s| s.cost).sum();
            let lossy = steps.iter().any(|s| s.lossy);
            (
                k,
                Route {
                    steps,
                    total_cost,
                    lossy,
                },
            )
        })
        .collect()
}

pub fn plan(from: &str, to: &str, tools: &ToolStatus) -> Option<Route> {
    search(from, tools).remove(to)
}

pub fn missing_tool(from: &str, to: &str, tools: &ToolStatus) -> Option<Engine> {
    plan(from, to, &ToolStatus::all())?
        .steps
        .iter()
        .map(|s| s.engine)
        .find(|e| !tools.has(*e))
}

pub fn targets(from: &str, tools: &ToolStatus) -> Vec<TargetOption> {
    let ready = search(from, tools);
    let all = search(from, &ToolStatus::all());
    let mut out: Vec<TargetOption> = Vec::new();
    for (id, route) in all {
        let Some(info) = formats::by_id(id) else {
            continue;
        };
        match ready.get(id) {
            Some(r) => out.push(TargetOption {
                format: id.to_string(),
                label: info.label,
                category: info.category,
                route: r.clone(),
                missing_tool: None,
            }),
            None => out.push(TargetOption {
                format: id.to_string(),
                label: info.label,
                category: info.category,
                route: Route::default(),
                missing_tool: route
                    .steps
                    .iter()
                    .map(|s| s.engine)
                    .find(|e| !tools.has(*e)),
            }),
        }
    }
    out.sort_by(|a, b| {
        let cost = |t: &TargetOption| {
            if t.missing_tool.is_some() {
                u32::MAX
            } else {
                t.route.total_cost
            }
        };
        a.category
            .cmp(&b.category)
            .then(cost(a).cmp(&cost(b)))
            .then(a.label.cmp(b.label))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_native_edge() {
        let r = plan("json", "yaml", &ToolStatus::default()).unwrap();
        assert_eq!(r.steps.len(), 1);
        assert_eq!(r.steps[0].engine, Engine::Native);
        assert!(!r.lossy);
    }

    #[test]
    fn two_hops_via_hub() {
        let tools = ToolStatus {
            libreoffice: true,
            ..Default::default()
        };
        let r = plan("md", "pdf", &tools).unwrap();
        assert_eq!(r.steps.len(), 2);
        assert_eq!(r.steps[0].to, "html");
        assert_eq!(r.engines(), vec![Engine::Native, Engine::LibreOffice]);
    }

    #[test]
    fn missing_tool_reported() {
        let none = ToolStatus::default();
        assert!(plan("docx", "pdf", &none).is_none());
        assert_eq!(
            missing_tool("docx", "pdf", &none),
            Some(Engine::LibreOffice)
        );
        let t = targets("mp4", &none);
        let mp3 = t.iter().find(|t| t.format == "mp3").unwrap();
        assert_eq!(mp3.missing_tool, Some(Engine::Ffmpeg));
        assert!(mp3.route.steps.is_empty());
    }

    #[test]
    fn prefers_native_and_stops_after_video_to_gif() {
        let tools = ToolStatus::all();
        let r = plan("md", "html", &tools).unwrap();
        assert_eq!(r.steps[0].engine, Engine::Native);
        assert!(plan("mp4", "png", &tools).is_none());
        assert!(plan("mp4", "gif", &tools).is_some());
        assert!(targets("png", &tools).iter().all(|t| t.format != "png"));
    }

    #[test]
    fn new_formats_use_the_built_in_engine() {
        let none = ToolStatus::default();
        for from in ["jxl", "psd", "raw"] {
            let r = plan(from, "jpeg", &none).unwrap();
            assert_eq!(r.engines(), vec![Engine::Native], "{from}");
        }
        #[cfg(feature = "heic")]
        assert_eq!(plan("heic", "png", &none).unwrap().steps.len(), 1);
        let svg = plan("svg", "png", &ToolStatus::all()).unwrap();
        assert_eq!(svg.engines(), vec![Engine::Native]);
        let trace = plan("jpeg", "svg", &none).unwrap();
        assert_eq!(trace.steps.len(), 1);
        assert!(trace.lossy);
        let layers = plan("psd", "layers", &none).unwrap();
        assert_eq!(layers.steps.len(), 1);
        assert!(targets("png", &none).iter().all(|t| t.format != "layers"));
        assert!(plan("png", "layers", &none).is_none());
    }

    #[test]
    fn drawings_reach_images_and_pdf() {
        let none = ToolStatus::default();
        let png = plan("dwg", "png", &none).unwrap();
        let hops: Vec<&str> = png.steps.iter().map(|s| s.to).collect();
        assert_eq!(hops, ["svg", "png"]);
        assert_eq!(plan("dxf", "pdf", &none).unwrap().steps.len(), 1);
        assert_eq!(plan("dwg", "dxf", &none).unwrap().steps.len(), 1);
        assert!(plan("dxf", "dwg", &none).is_none());
        let cats: Vec<Category> = targets("dwg", &none).iter().map(|t| t.category).collect();
        assert!(cats.contains(&Category::Cad) && cats.contains(&Category::Image));
    }

    #[test]
    fn targets_sorted_by_category() {
        let t = targets("md", &ToolStatus::all());
        let cats: Vec<Category> = t.iter().map(|t| t.category).collect();
        let mut sorted = cats.clone();
        sorted.sort();
        assert_eq!(cats, sorted);
    }

    #[test]
    fn route_serializes_contract_shape() {
        let r = plan("json", "yaml", &ToolStatus::default()).unwrap();
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["engines"][0], "native");
        assert_eq!(v["steps"][0]["from"], "json");
        assert!(v["steps"][0].get("cost").is_none());
    }
}
