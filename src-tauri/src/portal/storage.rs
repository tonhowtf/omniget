//! Per-portal storage scopes (cookies, localStorage, caches).
//!
//! - `isolated` (default): one store per portal node. A logged-in portal is
//!   only reachable by the agents corded to it (study 94).
//! - `workspace`: shared by the portals of one canvas workspace.
//! - `global`: shared by every portal of every workspace.
//!
//! macOS 14+: `WKWebsiteDataStore(forIdentifier:)` with a UUID derived from
//! the scope key. Older macOS has no named stores: `isolated` falls back to a
//! non-persistent store, the others to the default store, which is also the
//! app's own (different origin, so pages cannot read the app's data).
//! Windows/Linux: a data directory per scope under
//! `<app_data>/portals/`.

use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    #[default]
    Isolated,
    Workspace,
    Global,
}

impl Scope {
    pub fn parse(s: Option<&str>) -> Scope {
        match s.map(str::trim) {
            Some("workspace") => Scope::Workspace,
            Some("global") => Scope::Global,
            _ => Scope::Isolated,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Isolated => "isolated",
            Scope::Workspace => "workspace",
            Scope::Global => "global",
        }
    }
}

/// Stable key of the store a portal uses.
pub fn scope_key(scope: Scope, node_id: &str, workspace_id: &str) -> String {
    match scope {
        Scope::Isolated => format!("portal:node:{node_id}"),
        Scope::Workspace => format!("portal:ws:{workspace_id}"),
        Scope::Global => "portal:global".to_string(),
    }
}

/// The portal whose store `node_id` uses: portals joined by portal↔portal
/// cords share one store (Maestri: connect two portals and they share the
/// session). The root is the member no cord points to (the one the chain
/// started from, where the user logged in); ties go to the smallest id.
/// Returns `(root id, root scope)`; a portal with no portal cords is its own
/// root.
pub fn session_root(
    doc: &omniget_core::core::canvas::model::FloorDoc,
    node_id: &str,
) -> (String, Scope) {
    use omniget_core::core::canvas::model::NodeKind;
    use std::collections::{BTreeSet, VecDeque};
    let is_portal = |id: &str| doc.node(id).is_some_and(|n| n.kind == NodeKind::Portal);
    let scope_of = |id: &str| Scope::parse(doc.node(id).and_then(|n| n.data_str("storageScope")));
    let links: Vec<(&str, &str)> = doc
        .connections
        .iter()
        .filter(|c| c.from != c.to && is_portal(&c.from) && is_portal(&c.to))
        .map(|c| (c.from.as_str(), c.to.as_str()))
        .collect();
    let mut group: BTreeSet<String> = BTreeSet::new();
    let mut queue = VecDeque::from([node_id.to_string()]);
    while let Some(id) = queue.pop_front() {
        if !group.insert(id.clone()) {
            continue;
        }
        for (a, b) in &links {
            if *a == id {
                queue.push_back(b.to_string());
            } else if *b == id {
                queue.push_back(a.to_string());
            }
        }
    }
    let root = group
        .iter()
        .find(|id| !links.iter().any(|(_, b)| b == id))
        .or_else(|| group.iter().next())
        .cloned()
        .unwrap_or_else(|| node_id.to_string());
    let scope = scope_of(&root);
    (root, scope)
}

/// Store key `node_id` really uses (its chain's root), read from the canvas.
pub fn effective_key(scope: Scope, node_id: &str, workspace_id: &str) -> String {
    let store = crate::canvas::doc::host().store();
    let root = store
        .locate_node(node_id)
        .and_then(|(ws, floor)| store.read(&ws, &floor, |d| session_root(d, node_id)).ok());
    match root {
        Some((root, root_scope)) if root != node_id => scope_key(root_scope, &root, workspace_id),
        _ => scope_key(scope, node_id, workspace_id),
    }
}

/// 16 bytes shaped as a UUID v4 (WebKit takes an `NSUUID`).
pub fn store_id(key: &str) -> [u8; 16] {
    let digest = Sha256::digest(key.as_bytes());
    let mut id = [0u8; 16];
    id.copy_from_slice(&digest[..16]);
    id[6] = (id[6] & 0x0f) | 0x40;
    id[8] = (id[8] & 0x3f) | 0x80;
    id
}

/// `<app_data>/portals/<hex>`, created on demand.
pub fn data_dir(key: &str) -> Option<PathBuf> {
    let hex: String = store_id(key).iter().map(|b| format!("{b:02x}")).collect();
    let dir = omniget_core::core::paths::app_data_dir()?
        .join("portals")
        .join(hex);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// macOS 14 or newer (named website data stores).
pub fn named_stores_available() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| {
        if !cfg!(target_os = "macos") {
            return false;
        }
        std::process::Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|v| {
                v.trim()
                    .split('.')
                    .next()
                    .and_then(|m| m.parse::<u32>().ok())
            })
            .is_some_and(|major| major >= 14)
    })
}

/// Applies a store key (see [`effective_key`]) to a window builder.
pub fn apply<'a, R, M>(
    builder: tauri::WebviewWindowBuilder<'a, R, M>,
    scope: Scope,
    key: &str,
) -> tauri::WebviewWindowBuilder<'a, R, M>
where
    R: tauri::Runtime,
    M: tauri::Manager<R>,
{
    let key = key.to_string();
    if cfg!(target_os = "macos") {
        if named_stores_available() {
            return builder.data_store_identifier(store_id(&key));
        }
        return if scope == Scope::Isolated {
            builder.incognito(true)
        } else {
            builder
        };
    }
    match data_dir(&key) {
        Some(dir) => builder.data_directory(dir),
        None => builder.incognito(scope == Scope::Isolated),
    }
}
