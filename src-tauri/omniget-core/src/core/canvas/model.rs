//! Serde types of the canvas document, mirrored in `src/lib/canvas/model/types.ts`.
//!
//! Rules copied from Maestri's `CanvasSnapshot` (estudo 95, 01):
//! - ids are echoed literally, never reformatted;
//! - enums are open on read (an unknown string survives as `Unknown(s)` and the
//!   front draws a plain box) and strict on write (`ops` rejects them);
//! - optional fields are omitted, never `null`;
//! - `terminalId` (PTY session) is separate from the node id.
//!
//! Node payloads stay a JSON object in [`CanvasNode::data`] so keys written by a
//! newer build survive a round trip; the typed structs below validate writes
//! and give other modules typed access via [`CanvasNode::data_as`].

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub const FLOOR_FORMAT: &str = "omniget.canvas";
pub const FLOOR_VERSION: u32 = 1;
pub const WORKSPACE_FORMAT: &str = "omniget.canvas.workspace";
pub const WORKSPACE_VERSION: u32 = 1;
pub const GROUND_FLOOR_ID: &str = "ground";
pub const DEFAULT_WORKSPACE_ID: &str = "home";
pub const GRID: f64 = 20.0;
pub const ZOOM_MIN: f64 = 0.1;
pub const ZOOM_MAX: f64 = 3.0;

/// Open string enum: known variants serialize as their lowercase-ish tag,
/// anything else is kept verbatim in `Unknown`.
macro_rules! open_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident = $tag:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant,)+
            Unknown(String),
        }

        impl $name {
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $tag,)+
                    Self::Unknown(s) => s.as_str(),
                }
            }

            pub fn is_known(&self) -> bool {
                !matches!(self, Self::Unknown(_))
            }

            pub fn parse(s: &str) -> Self {
                match s {
                    $($tag => Self::$variant,)+
                    other => Self::Unknown(other.to_string()),
                }
            }

            pub const KNOWN: &'static [&'static str] = &[$($tag),+];
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                Ok(Self::parse(&s))
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

open_enum!(
    /// Node kind. `Unknown` also covers the literal "unknown".
    NodeKind {
        Terminal = "terminal",
        Agent = "agent",
        Note = "note",
        Text = "text",
        File = "file",
        FileTree = "filetree",
        Link = "link",
        Portal = "portal",
        Shape = "shape",
        Image = "image",
        Stack = "stack",
        Connector = "connector",
    }
);

open_enum!(AgentKind {
    Claude = "claude",
    Codex = "codex",
    Gemini = "gemini",
    OpenCode = "opencode",
    Ollama = "ollama",
    Shell = "shell",
    Native = "native",
});

open_enum!(ConnectionKind {
    Ask = "ask",
    Message = "message",
    Handoff = "handoff",
    Spawn = "spawn",
    Note = "note",
    Portal = "portal",
});

open_enum!(Condition {
    Always = "always",
    Success = "success",
    Failure = "failure",
});

open_enum!(PortalKind {
    Web = "web",
    Ios = "ios",
    Android = "android",
});

open_enum!(Edge {
    Top = "top",
    Bottom = "bottom",
    Left = "left",
    Right = "right",
});

open_enum!(GroupTarget {
    Nodes = "nodes",
    Drawings = "drawings",
});

open_enum!(RopeRouting {
    AvoidNodes = "avoidNodes",
    BehindNodes = "behindNodes",
    Circuit = "circuit",
});

fn is_false(b: &bool) -> bool {
    !*b
}

// ---------------------------------------------------------------- geometry

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn right(&self) -> f64 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
    pub fn inflate(&self, d: f64) -> Rect {
        Rect {
            x: self.x - d,
            y: self.y - d,
            w: self.w + 2.0 * d,
            h: self.h + 2.0 * d,
        }
    }
    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect {
            x,
            y,
            w: self.right().max(o.right()) - x,
            h: self.bottom().max(o.bottom()) - y,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

// ---------------------------------------------------------------- nodes

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasNode {
    pub id: String,
    pub kind: NodeKind,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub z: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
    /// Note living inside a fichário (`stack` node): hidden on the canvas,
    /// still addressable by id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_id: Option<String>,
    #[serde(default = "empty_object")]
    pub data: Value,
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

impl CanvasNode {
    pub fn rect(&self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
        }
    }

    /// Typed view of the payload. Unknown extra keys are ignored.
    pub fn data_as<T: DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_value(self.data.clone()).ok()
    }

    pub fn data_str(&self, key: &str) -> Option<&str> {
        self.data.get(key).and_then(|v| v.as_str())
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self.kind, NodeKind::Terminal | NodeKind::Agent)
    }

    /// Visible on the canvas (not tucked inside a stack).
    pub fn is_on_canvas(&self) -> bool {
        self.stack_id.is_none()
    }
}

/// `data.command` of a terminal node: `"claude --model x"` or
/// `["bash", "--rcfile", "my file.sh"]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CommandSpec {
    Line(String),
    Argv(Vec<String>),
}

/// Payload of `terminal` and `agent` nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalData {
    pub terminal_id: String,
    pub agent: AgentKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The program: one line (split on spaces) or argv (a preset with
    /// arguments that contain spaces).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<CommandSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_manager: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_activity: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefers_chat_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NoteData {
    /// Internal note: file name inside `<ws>/notes/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// External note: absolute path, the file stays where it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_external: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_custom_name: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_content_locked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextData {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_monospaced: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_weight: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileData {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_image: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_video: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImageData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub natural_w: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub natural_h: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkData {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTreeData {
    pub root_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_path: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub shows_hidden_files: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pinned_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expanded: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortalDevice {
    pub platform: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortalData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub portal_kind: PortalKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub chrome_hidden: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub muted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<PortalDevice>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeData {
    pub shape: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Fichário (Maestri `noteStack`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StackData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_custom_name: bool,
    #[serde(default)]
    pub tabs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front_note_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uniform_color: Option<String>,
}

/// Chip standing for a node on another floor or workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_terminal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_floor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_workspace_id: Option<String>,
}

/// Default frame size per kind (Maestri: note 260×200, text 240×60,
/// fileTree 320×400, terminal 600×420).
pub fn default_size(kind: &NodeKind) -> Size {
    let (w, h) = match kind {
        NodeKind::Terminal | NodeKind::Agent => (600.0, 420.0),
        NodeKind::Note => (260.0, 200.0),
        NodeKind::Text => (240.0, 60.0),
        NodeKind::FileTree => (320.0, 400.0),
        NodeKind::Link => (280.0, 72.0),
        NodeKind::Portal => (800.0, 600.0),
        NodeKind::File => (200.0, 160.0),
        NodeKind::Image => (320.0, 240.0),
        NodeKind::Shape => (160.0, 120.0),
        NodeKind::Stack => (260.0, 220.0),
        NodeKind::Connector => (160.0, 48.0),
        NodeKind::Unknown(_) => (200.0, 120.0),
    };
    Size { w, h }
}

/// Strict check of a node about to be written.
pub fn validate_node(node: &CanvasNode) -> Result<(), String> {
    if node.id.trim().is_empty() {
        return Err("node id is empty".into());
    }
    if !node.kind.is_known() {
        return Err(format!("unknown node kind '{}'", node.kind));
    }
    for (name, v) in [
        ("x", node.x),
        ("y", node.y),
        ("w", node.w),
        ("h", node.h),
        ("z", node.z),
    ] {
        if !v.is_finite() {
            return Err(format!("node {} has a non-finite {name}", node.id));
        }
    }
    if node.w <= 0.0 || node.h <= 0.0 {
        return Err(format!("node {} has an empty frame", node.id));
    }
    if !node.data.is_object() {
        return Err(format!("node {} data must be an object", node.id));
    }
    fn check<T: DeserializeOwned>(node: &CanvasNode) -> Result<T, String> {
        serde_json::from_value::<T>(node.data.clone())
            .map_err(|e| format!("node {} ({}) has invalid data: {e}", node.id, node.kind))
    }
    match &node.kind {
        NodeKind::Terminal | NodeKind::Agent => {
            let d: TerminalData = check(node)?;
            if !d.agent.is_known() {
                return Err(format!("unknown agent '{}'", d.agent));
            }
            if d.terminal_id.trim().is_empty() {
                return Err(format!("node {} has an empty terminalId", node.id));
            }
        }
        NodeKind::Note => {
            let d: NoteData = check(node)?;
            if let Some(f) = &d.file_name {
                if f.contains('/') || f.contains('\\') || f.starts_with('.') {
                    return Err(format!("note fileName '{f}' must be a plain file name"));
                }
            }
        }
        NodeKind::Text => {
            check::<TextData>(node)?;
        }
        NodeKind::File => {
            check::<FileData>(node)?;
        }
        NodeKind::Image => {
            check::<ImageData>(node)?;
        }
        NodeKind::Link => {
            check::<LinkData>(node)?;
        }
        NodeKind::FileTree => {
            check::<FileTreeData>(node)?;
        }
        NodeKind::Portal => {
            let d: PortalData = check(node)?;
            if !d.portal_kind.is_known() {
                return Err(format!("unknown portalKind '{}'", d.portal_kind));
            }
        }
        NodeKind::Shape => {
            check::<ShapeData>(node)?;
        }
        NodeKind::Stack => {
            check::<StackData>(node)?;
        }
        NodeKind::Connector => {
            check::<ConnectorData>(node)?;
        }
        NodeKind::Unknown(_) => unreachable!(),
    }
    Ok(())
}

// ---------------------------------------------------------------- cords

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: ConnectionKind,
    #[serde(default)]
    pub perms: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<Condition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Flat `[x0, y0, x1, y1, ...]` rope shape; may be stale, draw from the ends.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_note_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_note_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_active: Option<bool>,
}

impl Connection {
    pub fn touches(&self, node_id: &str) -> bool {
        self.from == node_id || self.to == node_id
    }
    pub fn other_end(&self, node_id: &str) -> Option<&str> {
        if self.from == node_id {
            Some(&self.to)
        } else if self.to == node_id {
            Some(&self.from)
        } else {
            None
        }
    }
}

pub fn validate_connection(c: &Connection) -> Result<(), String> {
    if c.id.trim().is_empty() {
        return Err("connection id is empty".into());
    }
    if !c.kind.is_known() {
        return Err(format!("unknown connection kind '{}'", c.kind));
    }
    if let Some(cond) = &c.condition {
        if !cond.is_known() {
            return Err(format!("unknown condition '{cond}'"));
        }
    }
    if c.from == c.to {
        return Err(format!("connection {} links a node to itself", c.id));
    }
    for p in &c.perms {
        if p.is_empty()
            || p.len() > 32
            || !p
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        {
            return Err(format!("invalid perm '{p}'"));
        }
    }
    if c.points.iter().any(|v| !v.is_finite()) {
        return Err(format!("connection {} has non-finite points", c.id));
    }
    Ok(())
}

// ---------------------------------------------------------------- drawings

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DrawingAnchor {
    pub drawing_id: String,
    pub edge: Edge,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Drawing {
    pub id: String,
    pub tool: String,
    /// Flat `[x, y, ...]`, local to `origin`, rounded to 0.1 on write.
    #[serde(default)]
    pub points: Vec<f64>,
    pub origin: Point,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_size: Option<Size>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Size>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    #[serde(default)]
    pub z: f64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_connection: Option<DrawingAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_connection: Option<DrawingAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_point_offset: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
}

impl Drawing {
    /// Axis-aligned bounds in canvas coordinates (ignores rotation).
    pub fn bounds(&self) -> Rect {
        let (sx, sy) = match (self.original_size, self.size) {
            (Some(o), Some(s)) if o.w > 0.0 && o.h > 0.0 => (s.w / o.w, s.h / o.h),
            _ => (1.0, 1.0),
        };
        let mut min = (f64::MAX, f64::MAX);
        let mut max = (f64::MIN, f64::MIN);
        for p in self.points.chunks_exact(2) {
            let (x, y) = (p[0] * sx, p[1] * sy);
            min = (min.0.min(x), min.1.min(y));
            max = (max.0.max(x), max.1.max(y));
        }
        if min.0 > max.0 {
            let s = self.size.unwrap_or_default();
            return Rect {
                x: self.origin.x,
                y: self.origin.y,
                w: s.w,
                h: s.h,
            };
        }
        Rect {
            x: self.origin.x + min.0,
            y: self.origin.y + min.1,
            w: max.0 - min.0,
            h: max.1 - min.1,
        }
    }
}

/// Drawing tools the canvas knows how to draw: the Maestri wire names plus
/// the front's short names (`rect`, `smart`) kept for older documents.
pub const DRAWING_TOOLS: &[&str] = &[
    "pen",
    "brush",
    "smartDraw",
    "smart",
    "highlighter",
    "arrow",
    "line",
    "rectangle",
    "rect",
    "ellipse",
    "oval",
    "triangle",
    "diamond",
    "hexagon",
    "parallelogram",
    "star",
    "cloud",
    "heart",
    "xBox",
    "checkBox",
    "blockArrowLeft",
    "blockArrowUp",
    "blockArrowDown",
    "blockArrowRight",
    "text",
];

pub fn is_drawing_tool(tool: &str) -> bool {
    DRAWING_TOOLS.contains(&tool)
}

/// Middle of one side of `r` (where an attached arrow end sits).
pub fn edge_point(r: Rect, edge: &Edge) -> Point {
    match edge {
        Edge::Top => Point {
            x: r.x + r.w / 2.0,
            y: r.y,
        },
        Edge::Bottom => Point {
            x: r.x + r.w / 2.0,
            y: r.y + r.h,
        },
        Edge::Left => Point {
            x: r.x,
            y: r.y + r.h / 2.0,
        },
        Edge::Right => Point {
            x: r.x + r.w,
            y: r.y + r.h / 2.0,
        },
        _ => Point {
            x: r.x + r.w / 2.0,
            y: r.y + r.h / 2.0,
        },
    }
}

pub fn round_points(points: &mut [f64]) {
    for v in points {
        *v = (*v * 10.0).round() / 10.0;
    }
}

pub fn validate_drawing(d: &Drawing) -> Result<(), String> {
    if d.id.trim().is_empty() {
        return Err("drawing id is empty".into());
    }
    if d.tool.trim().is_empty() {
        return Err(format!("drawing {} has no tool", d.id));
    }
    if !is_drawing_tool(&d.tool) {
        return Err(format!(
            "unknown drawing tool '{}' (known: {})",
            d.tool,
            DRAWING_TOOLS.join(", ")
        ));
    }
    if !d.points.len().is_multiple_of(2) {
        return Err(format!("drawing {} has an odd point array", d.id));
    }
    if d.points.iter().any(|v| !v.is_finite()) || !d.origin.x.is_finite() || !d.origin.y.is_finite()
    {
        return Err(format!("drawing {} has non-finite coordinates", d.id));
    }
    for a in [&d.source_connection, &d.target_connection]
        .into_iter()
        .flatten()
    {
        if !a.edge.is_known() {
            return Err(format!("unknown edge '{}'", a.edge));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- groups, ties

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default = "default_group_target")]
    pub target: GroupTarget,
}

fn default_group_target() -> GroupTarget {
    GroupTarget::Nodes
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CableTie {
    pub id: String,
    pub position: Point,
    #[serde(default)]
    pub member_connection_ids: Vec<String>,
}

// ---------------------------------------------------------------- floor file

/// `floors/<floorId>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FloorDoc {
    #[serde(default = "floor_format")]
    pub format: String,
    #[serde(default = "floor_version")]
    pub version: u32,
    #[serde(default)]
    pub epoch: String,
    #[serde(default)]
    pub rev: u64,
    #[serde(default)]
    pub camera: Camera,
    #[serde(default)]
    pub nodes: Vec<CanvasNode>,
    #[serde(default)]
    pub connections: Vec<Connection>,
    #[serde(default)]
    pub drawings: Vec<Drawing>,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub cable_ties: Vec<CableTie>,
}

fn floor_format() -> String {
    FLOOR_FORMAT.to_string()
}
fn floor_version() -> u32 {
    FLOOR_VERSION
}

impl Default for FloorDoc {
    fn default() -> Self {
        Self {
            format: floor_format(),
            version: FLOOR_VERSION,
            epoch: String::new(),
            rev: 0,
            camera: Camera::default(),
            nodes: Vec::new(),
            connections: Vec::new(),
            drawings: Vec::new(),
            groups: Vec::new(),
            cable_ties: Vec::new(),
        }
    }
}

impl FloorDoc {
    pub fn node(&self, id: &str) -> Option<&CanvasNode> {
        self.nodes.iter().find(|n| n.id == id)
    }
    pub fn node_mut(&mut self, id: &str) -> Option<&mut CanvasNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }
    pub fn connection(&self, id: &str) -> Option<&Connection> {
        self.connections.iter().find(|c| c.id == id)
    }
    pub fn drawing(&self, id: &str) -> Option<&Drawing> {
        self.drawings.iter().find(|d| d.id == id)
    }
    pub fn node_by_terminal(&self, terminal_id: &str) -> Option<&CanvasNode> {
        self.nodes
            .iter()
            .find(|n| n.is_terminal() && n.data_str("terminalId") == Some(terminal_id))
    }
    pub fn max_z(&self) -> f64 {
        self.nodes.iter().map(|n| n.z).fold(0.0, f64::max)
    }
}

/// What `canvas_load` returns.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub workspace_id: String,
    pub floor_id: String,
    pub epoch: String,
    pub rev: u64,
    pub seq: u64,
    pub camera: Camera,
    pub nodes: Vec<CanvasNode>,
    pub connections: Vec<Connection>,
    pub drawings: Vec<Drawing>,
    pub groups: Vec<Group>,
    pub cable_ties: Vec<CableTie>,
}

// ---------------------------------------------------------------- workspace

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FloorMeta {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_git_isolated: bool,
    /// Working tree of an isolated floor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub created_at: String,
}

/// `workspace.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    #[serde(default = "workspace_format", skip_serializing_if = "String::is_empty")]
    pub format: String,
    #[serde(default = "workspace_version")]
    pub version: u32,
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<Environment>,
    #[serde(default = "ground_id")]
    pub active_floor_id: String,
    #[serde(default)]
    pub floors: Vec<FloorMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rope_routing: Option<RopeRouting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_style: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i64>,
    /// Editor the folder opens in ("vscode", "cursor", "zed", "idea"...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ide: Option<String>,
    /// Fixed jump number (1–9) for ⌃N and the ⌘⌘-number jump; the list
    /// position is used when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump_number: Option<u8>,
    /// Keep CLAUDE.md, AGENTS.md and GEMINI.md of the working folder equal.
    #[serde(default, skip_serializing_if = "is_false")]
    pub sync_agent_files: bool,
    /// Lifted and docked panels of the window (Maestri lift/dock), kept per
    /// workspace so they survive switching and relaunch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panels: Option<WorkspacePanels>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

/// A node shown outside the canvas at real size: lifted into the centre or
/// docked as a full-height column on one edge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePanels {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifted: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub docked: Vec<DockedPanel>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockedPanel {
    pub node_id: String,
    /// `left` or `right`.
    pub edge: String,
    pub width: f64,
}

fn workspace_format() -> String {
    WORKSPACE_FORMAT.to_string()
}
fn workspace_version() -> u32 {
    WORKSPACE_VERSION
}
fn ground_id() -> String {
    GROUND_FLOOR_ID.to_string()
}

impl Workspace {
    pub fn new(id: String, name: String) -> Self {
        let now = now_rfc3339();
        Self {
            format: workspace_format(),
            version: WORKSPACE_VERSION,
            id,
            name,
            icon: None,
            color: None,
            working_directory: None,
            environment: None,
            active_floor_id: ground_id(),
            floors: vec![FloorMeta {
                id: ground_id(),
                name: "Ground".into(),
                color: None,
                branch: None,
                is_git_isolated: false,
                path: None,
                created_at: now.clone(),
            }],
            rope_routing: None,
            selection_style: None,
            is_pinned: false,
            group_name: None,
            folder_name: None,
            sort_index: None,
            ide: None,
            jump_number: None,
            sync_agent_files: false,
            panels: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    pub fn has_floor(&self, floor_id: &str) -> bool {
        self.floors.iter().any(|f| f.id == floor_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSummary {
    #[serde(flatten)]
    pub workspace: Workspace,
    pub node_count: usize,
    pub terminal_count: usize,
    /// Terminal nodes of every floor (sidebar hold-click list).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub terminals: Vec<WorkspaceTerminal>,
    /// Some terminal process is alive (filled by the app's runtime probe);
    /// false means hibernated (Maestri Unload).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loaded: Option<bool>,
    /// Changes when a `file:` icon changes (mtime + size), like the Wire's
    /// `iconRevision`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_revision: Option<String>,
    /// CLAUDE.md / AGENTS.md / GEMINI.md state when `syncAgentFiles` is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_files: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTerminal {
    pub node_id: String,
    pub floor_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default)]
    pub running: bool,
}

/// App hook that completes summaries with runtime facts the core cannot see
/// (live terminal processes, icon files, agent instruction files).
pub type SummaryEnricher = fn(&mut WorkspaceSummary);
static SUMMARY_ENRICHER: std::sync::OnceLock<SummaryEnricher> = std::sync::OnceLock::new();

pub fn set_summary_enricher(f: SummaryEnricher) {
    let _ = SUMMARY_ENRICHER.set(f);
}

pub fn enrich_summary(s: &mut WorkspaceSummary) {
    if let Some(f) = SUMMARY_ENRICHER.get() {
        f(s);
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInput {
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub environment: Option<Environment>,
    /// Caller-chosen id (echoed literally); generated when absent.
    #[serde(default)]
    pub id: Option<String>,
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Ids become file and folder names: keep them to a safe charset.
pub fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}
