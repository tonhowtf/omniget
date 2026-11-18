use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{sha256_hex, CatalogError, Result, ERR_HASH, ERR_INVALID};

pub const STACK_FORMAT: &str = "omnistack";
pub const STACK_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CollectionItem {
    pub item_id: String,
    pub position: i64,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub added: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Collection {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub position: i64,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub scope: Option<String>,
    pub created: String,
    pub updated: String,
    pub items: Vec<CollectionItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StackItem {
    pub id: String,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StackFile {
    pub format: String,
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub scope: Option<String>,
    pub created: String,
    #[serde(default)]
    pub generator: Option<String>,
    pub items: Vec<StackItem>,
    pub hash: String,
    #[serde(default)]
    pub signature: Option<Value>,
}

pub fn stack_hash(
    name: &str,
    targets: &[String],
    scope: Option<&str>,
    items: &[StackItem],
) -> String {
    let v = serde_json::json!({
        "items": items.iter().map(|i| serde_json::json!({
            "content_hash": i.content_hash,
            "id": i.id,
            "scope": i.scope,
            "targets": i.targets,
        })).collect::<Vec<_>>(),
        "name": name,
        "scope": scope,
        "targets": targets,
    });
    sha256_hex(canonical_json(&v).as_bytes())
}

pub fn canonical_json(v: &Value) -> String {
    match v {
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", Value::String(k.clone()), canonical_json(&o[k])))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(canonical_json).collect::<Vec<_>>().join(",")
        ),
        other => other.to_string(),
    }
}

pub fn parse_stack(bytes: &[u8]) -> Result<StackFile> {
    let s: StackFile = serde_json::from_slice(bytes)
        .map_err(|e| CatalogError::new(ERR_INVALID, format!("omnistack: {e}")))?;
    if s.format != STACK_FORMAT || s.version == 0 || s.version > STACK_VERSION {
        return Err(CatalogError::new(
            ERR_INVALID,
            format!("omnistack: unsupported {} v{}", s.format, s.version),
        ));
    }
    let h = stack_hash(&s.name, &s.targets, s.scope.as_deref(), &s.items);
    if h != s.hash {
        return Err(CatalogError::new(
            ERR_HASH,
            format!("omnistack: expected {}, got {h}", s.hash),
        ));
    }
    Ok(s)
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub collection: Collection,
    pub missing: Vec<String>,
    pub changed: Vec<String>,
}
