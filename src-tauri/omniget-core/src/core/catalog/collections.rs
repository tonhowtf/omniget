//! Coleções e stacks locais em `<app_data>/agentkit/collections.db`, com
//! exportação/importação de arquivo `.omnistack` (JSON: ids + alvos + escopo +
//! hash de conteúdo; campo `signature` reservado para assinar depois).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::model::content_hash;
use super::{
    agentkit_dir, sha256_hex, write_atomic, CatalogError, Result, ERR_DB, ERR_HASH, ERR_INVALID,
    ERR_NOT_FOUND,
};

pub const STACK_FORMAT: &str = "omnistack";
pub const STACK_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CollectionItem {
    pub item_id: String,
    pub position: i64,
    /// Ids de ferramenta-alvo (`claude`, `codex`…); vazio = decidir na instalação.
    #[serde(default)]
    pub targets: Vec<String>,
    /// `user`, `project`, `local`… ; `None` = padrão da coleção.
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

fn db_err(e: impl std::fmt::Display) -> CatalogError {
    CatalogError::new(ERR_DB, e.to_string())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Loja de coleções (uma conexão protegida por mutex).
pub struct Store {
    conn: Mutex<Connection>,
}

const SCHEMA: &str = "
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS collections (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    position INTEGER NOT NULL DEFAULT 0,
    targets TEXT NOT NULL DEFAULT '[]',
    scope TEXT,
    created TEXT NOT NULL,
    updated TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS collection_items (
    collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    item_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    targets TEXT NOT NULL DEFAULT '[]',
    scope TEXT,
    added TEXT,
    PRIMARY KEY (collection_id, item_id)
);
CREATE INDEX IF NOT EXISTS collection_items_order ON collection_items(collection_id, position);
";

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(|e| super::io_err("create dir", e))?;
        }
        let conn = Connection::open(path).map_err(db_err)?;
        conn.execute_batch(SCHEMA).map_err(db_err)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(db_err)?;
        conn.execute_batch(SCHEMA).map_err(db_err)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn with<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut g = self.conn.lock().map_err(|_| db_err("lock poisoned"))?;
        f(&mut g)
    }

    fn items_of(conn: &Connection, id: &str) -> Result<Vec<CollectionItem>> {
        let mut st = conn
            .prepare(
                "SELECT item_id, position, targets, scope, added FROM collection_items
                 WHERE collection_id = ?1 ORDER BY position, item_id",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map([id], |r| {
                let targets: String = r.get(2)?;
                Ok(CollectionItem {
                    item_id: r.get(0)?,
                    position: r.get(1)?,
                    targets: serde_json::from_str(&targets).unwrap_or_default(),
                    scope: r.get(3)?,
                    added: r.get(4)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<std::result::Result<_, _>>().map_err(db_err)
    }

    fn one(conn: &Connection, id: &str) -> Result<Collection> {
        let row = conn
            .query_row(
                "SELECT id, name, description, position, targets, scope, created, updated
                 FROM collections WHERE id = ?1",
                [id],
                |r| {
                    let targets: String = r.get(4)?;
                    Ok(Collection {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        description: r.get(2)?,
                        position: r.get(3)?,
                        targets: serde_json::from_str(&targets).unwrap_or_default(),
                        scope: r.get(5)?,
                        created: r.get(6)?,
                        updated: r.get(7)?,
                        items: Vec::new(),
                    })
                },
            )
            .optional()
            .map_err(db_err)?
            .ok_or_else(|| CatalogError::new(ERR_NOT_FOUND, format!("collection `{id}`")))?;
        Ok(Collection {
            items: Self::items_of(conn, id)?,
            ..row
        })
    }

    pub fn list(&self) -> Result<Vec<Collection>> {
        self.with(|c| {
            let ids: Vec<String> = {
                let mut st = c
                    .prepare("SELECT id FROM collections ORDER BY position, created")
                    .map_err(db_err)?;
                let rows = st.query_map([], |r| r.get(0)).map_err(db_err)?;
                rows.collect::<std::result::Result<_, _>>()
                    .map_err(db_err)?
            };
            ids.iter().map(|id| Self::one(c, id)).collect()
        })
    }

    pub fn get(&self, id: &str) -> Result<Collection> {
        self.with(|c| Self::one(c, id))
    }

    pub fn create(
        &self,
        name: &str,
        description: Option<&str>,
        targets: &[String],
        scope: Option<&str>,
    ) -> Result<Collection> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CatalogError::new(ERR_INVALID, "collection name is empty"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let ts = now();
        self.with(|c| {
            let pos: i64 = c
                .query_row("SELECT COALESCE(MAX(position) + 1, 0) FROM collections", [], |r| r.get(0))
                .map_err(db_err)?;
            c.execute(
                "INSERT INTO collections (id, name, description, position, targets, scope, created, updated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![id, name, description, pos, serde_json::to_string(targets).unwrap_or("[]".into()), scope, ts],
            )
            .map_err(db_err)?;
            Self::one(c, &id)
        })
    }

    fn touch(c: &Connection, id: &str) -> Result<()> {
        let n = c
            .execute(
                "UPDATE collections SET updated = ?2 WHERE id = ?1",
                params![id, now()],
            )
            .map_err(db_err)?;
        if n == 0 {
            return Err(CatalogError::new(
                ERR_NOT_FOUND,
                format!("collection `{id}`"),
            ));
        }
        Ok(())
    }

    pub fn rename(&self, id: &str, name: &str, description: Option<&str>) -> Result<Collection> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CatalogError::new(ERR_INVALID, "collection name is empty"));
        }
        self.with(|c| {
            c.execute(
                "UPDATE collections SET name = ?2, description = COALESCE(?3, description) WHERE id = ?1",
                params![id, name, description],
            )
            .map_err(db_err)?;
            Self::touch(c, id)?;
            Self::one(c, id)
        })
    }

    pub fn set_defaults(
        &self,
        id: &str,
        targets: &[String],
        scope: Option<&str>,
    ) -> Result<Collection> {
        self.with(|c| {
            c.execute(
                "UPDATE collections SET targets = ?2, scope = ?3 WHERE id = ?1",
                params![
                    id,
                    serde_json::to_string(targets).unwrap_or("[]".into()),
                    scope
                ],
            )
            .map_err(db_err)?;
            Self::touch(c, id)?;
            Self::one(c, id)
        })
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        self.with(|c| {
            let n = c
                .execute("DELETE FROM collections WHERE id = ?1", [id])
                .map_err(db_err)?;
            if n == 0 {
                return Err(CatalogError::new(
                    ERR_NOT_FOUND,
                    format!("collection `{id}`"),
                ));
            }
            c.execute(
                "DELETE FROM collection_items WHERE collection_id = ?1",
                [id],
            )
            .map_err(db_err)?;
            Ok(())
        })
    }

    /// Adiciona (ou atualiza alvos/escopo de) um item no fim da coleção.
    pub fn add(
        &self,
        id: &str,
        item_id: &str,
        targets: &[String],
        scope: Option<&str>,
    ) -> Result<Collection> {
        if item_id.trim().is_empty() {
            return Err(CatalogError::new(ERR_INVALID, "item id is empty"));
        }
        self.with(|c| {
            Self::touch(c, id)?;
            let exists: bool = c
                .query_row(
                    "SELECT 1 FROM collection_items WHERE collection_id = ?1 AND item_id = ?2",
                    params![id, item_id],
                    |_| Ok(true),
                )
                .optional()
                .map_err(db_err)?
                .unwrap_or(false);
            let t = serde_json::to_string(targets).unwrap_or("[]".into());
            if exists {
                c.execute(
                    "UPDATE collection_items SET targets = ?3, scope = ?4 WHERE collection_id = ?1 AND item_id = ?2",
                    params![id, item_id, t, scope],
                )
                .map_err(db_err)?;
            } else {
                let pos: i64 = c
                    .query_row(
                        "SELECT COALESCE(MAX(position) + 1, 0) FROM collection_items WHERE collection_id = ?1",
                        [id],
                        |r| r.get(0),
                    )
                    .map_err(db_err)?;
                c.execute(
                    "INSERT INTO collection_items (collection_id, item_id, position, targets, scope, added)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![id, item_id, pos, t, scope, now()],
                )
                .map_err(db_err)?;
            }
            Self::one(c, id)
        })
    }

    fn renumber(c: &Connection, id: &str, order: &[String]) -> Result<()> {
        for (i, item) in order.iter().enumerate() {
            c.execute(
                "UPDATE collection_items SET position = ?3 WHERE collection_id = ?1 AND item_id = ?2",
                params![id, item, i as i64],
            )
            .map_err(db_err)?;
        }
        Ok(())
    }

    pub fn remove(&self, id: &str, item_id: &str) -> Result<Collection> {
        self.with(|c| {
            Self::touch(c, id)?;
            let n = c
                .execute(
                    "DELETE FROM collection_items WHERE collection_id = ?1 AND item_id = ?2",
                    params![id, item_id],
                )
                .map_err(db_err)?;
            if n == 0 {
                return Err(CatalogError::new(
                    ERR_NOT_FOUND,
                    format!("`{item_id}` not in collection"),
                ));
            }
            let order: Vec<String> = Self::items_of(c, id)?
                .into_iter()
                .map(|i| i.item_id)
                .collect();
            Self::renumber(c, id, &order)?;
            Self::one(c, id)
        })
    }

    /// Move um item para a posição `to` (0 = primeiro; além do fim = último).
    pub fn move_item(&self, id: &str, item_id: &str, to: usize) -> Result<Collection> {
        self.with(|c| {
            let tx = c.transaction().map_err(db_err)?;
            Self::touch(&tx, id)?;
            let mut order: Vec<String> = Self::items_of(&tx, id)?
                .into_iter()
                .map(|i| i.item_id)
                .collect();
            let from = order.iter().position(|x| x == item_id).ok_or_else(|| {
                CatalogError::new(ERR_NOT_FOUND, format!("`{item_id}` not in collection"))
            })?;
            let it = order.remove(from);
            let to = to.min(order.len());
            order.insert(to, it);
            Self::renumber(&tx, id, &order)?;
            tx.commit().map_err(db_err)?;
            Self::one(c, id)
        })
    }

    /// Reordena as próprias coleções.
    pub fn move_collection(&self, id: &str, to: usize) -> Result<Vec<Collection>> {
        self.with(|c| {
            let mut ids: Vec<String> = {
                let mut st = c
                    .prepare("SELECT id FROM collections ORDER BY position, created")
                    .map_err(db_err)?;
                let rows = st.query_map([], |r| r.get(0)).map_err(db_err)?;
                rows.collect::<std::result::Result<_, _>>()
                    .map_err(db_err)?
            };
            let from = ids
                .iter()
                .position(|x| x == id)
                .ok_or_else(|| CatalogError::new(ERR_NOT_FOUND, format!("collection `{id}`")))?;
            let x = ids.remove(from);
            ids.insert(to.min(ids.len()), x);
            for (i, cid) in ids.iter().enumerate() {
                c.execute(
                    "UPDATE collections SET position = ?2 WHERE id = ?1",
                    params![cid, i as i64],
                )
                .map_err(db_err)?;
            }
            Ok(())
        })?;
        self.list()
    }
}

/// Loja padrão em `<app_data>/agentkit/collections.db`.
pub fn store() -> Result<&'static Store> {
    static STORE: std::sync::OnceLock<Store> = std::sync::OnceLock::new();
    if let Some(s) = STORE.get() {
        return Ok(s);
    }
    let s = Store::open(&agentkit_dir()?.join("collections.db"))?;
    Ok(STORE.get_or_init(|| s))
}

// ---------------------------------------------------------------- .omnistack

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StackItem {
    pub id: String,
    /// Hash de conteúdo no índice na hora da exportação (`None` p/ plugin/registry).
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
    /// sha256 do JSON canônico de `{name, targets, scope, items}`.
    pub hash: String,
    /// Reservado (ed25519 depois).
    #[serde(default)]
    pub signature: Option<Value>,
}

/// JSON canônico (chaves ordenadas, sem espaço) do que o hash cobre.
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

/// JSON com chaves ordenadas em todo nível, sem espaço (independe de `preserve_order`).
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

/// Monta o `.omnistack` de uma coleção (hash de conteúdo tirado do catálogo atual).
pub fn build_stack(col: &Collection, lookup: impl Fn(&str) -> Option<String>) -> StackFile {
    let items: Vec<StackItem> = col
        .items
        .iter()
        .map(|i| StackItem {
            id: i.item_id.clone(),
            content_hash: lookup(&i.item_id),
            targets: i.targets.clone(),
            scope: i.scope.clone(),
        })
        .collect();
    let hash = stack_hash(&col.name, &col.targets, col.scope.as_deref(), &items);
    StackFile {
        format: STACK_FORMAT.into(),
        version: STACK_VERSION,
        name: col.name.clone(),
        description: col.description.clone(),
        targets: col.targets.clone(),
        scope: col.scope.clone(),
        created: now(),
        generator: Some(concat!("OmniGet ", env!("CARGO_PKG_VERSION")).into()),
        items,
        hash,
        signature: None,
    }
}

/// Lê e valida um `.omnistack` (formato, versão, hash).
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

fn catalog_hash(id: &str) -> Option<String> {
    super::index::item(id).ok().and_then(|i| content_hash(&i))
}

/// Exporta uma coleção para `path` (acrescenta `.omnistack` se faltar).
pub fn export(id: &str, path: &Path) -> Result<PathBuf> {
    let col = store()?.get(id)?;
    let stack = build_stack(&col, catalog_hash);
    let mut p = path.to_path_buf();
    if p.extension().and_then(|e| e.to_str()) != Some("omnistack") {
        p.set_extension("omnistack");
    }
    let bytes = serde_json::to_vec_pretty(&stack)
        .map_err(|e| CatalogError::new(ERR_INVALID, e.to_string()))?;
    write_atomic(&p, &bytes)?;
    Ok(p)
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub collection: Collection,
    /// Itens que não estão no catálogo atual.
    pub missing: Vec<String>,
    /// Itens cujo conteúdo mudou desde a exportação.
    pub changed: Vec<String>,
}

/// Importa um `.omnistack` como coleção nova.
pub fn import(path: &Path) -> Result<ImportReport> {
    let bytes = std::fs::read(path).map_err(|e| super::io_err("read omnistack", e))?;
    let stack = parse_stack(&bytes)?;
    let st = store()?;
    let col = st.create(
        &stack.name,
        stack.description.as_deref(),
        &stack.targets,
        stack.scope.as_deref(),
    )?;
    let mut missing = Vec::new();
    let mut changed = Vec::new();
    for i in &stack.items {
        st.add(&col.id, &i.id, &i.targets, i.scope.as_deref())?;
        match super::index::item(&i.id) {
            Ok(item) => {
                if let (Some(want), Some(have)) = (&i.content_hash, content_hash(&item)) {
                    if *want != have {
                        changed.push(i.id.clone());
                    }
                }
            }
            Err(_) => missing.push(i.id.clone()),
        }
    }
    Ok(ImportReport {
        collection: st.get(&col.id)?,
        missing,
        changed,
    })
}
