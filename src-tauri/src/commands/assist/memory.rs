//! Commands for `assist::memory`: the "what it knows about you" panel of a
//! bot and the `/llm/memory` page. Every command acts as the person at the
//! screen (`AssistCtx::user()`), who owns every scope of their own.

use omniget_core::core::assist::ctx::{AssistCtx, Scope};
use omniget_core::core::assist::memory::{
    self, Category, Correction, MemoryRecord, NewMemory, ScopeSummary, Source,
};
use omniget_core::core::assist::{db, now_ms};
use serde::{Deserialize, Serialize};

fn parse_scope(key: &str) -> Result<Scope, String> {
    Scope::parse(key).ok_or_else(|| format!("{}: unknown scope", memory::ERR_MEMORY_INVALID))
}

fn parse_category(s: &str) -> Result<Category, String> {
    Category::parse(s).ok_or_else(|| format!("{}: unknown kind", memory::ERR_MEMORY_INVALID))
}

/// One record as the UI shows it: the record plus its readable source line
/// and its scope as a key.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryView {
    #[serde(flatten)]
    pub record: MemoryRecord,
    pub scope_key: String,
    pub source_line: String,
}

fn view(record: MemoryRecord) -> MemoryView {
    MemoryView {
        scope_key: record.scope.key(),
        source_line: memory::source_line(&record),
        record,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ScopeView {
    #[serde(flatten)]
    pub summary: ScopeSummary,
    pub scope_key: String,
}

/// Records of one scope (`"user"`, `"bot:<id>"`, `"room:<id>"`), or all.
#[tauri::command]
pub fn assist_memory_list(
    scope: Option<String>,
    include_inactive: Option<bool>,
) -> Result<Vec<MemoryView>, String> {
    let db = db::global()?;
    let scope = scope.as_deref().map(parse_scope).transpose()?;
    let rows = memory::list(
        &db,
        &AssistCtx::user(),
        scope.as_ref(),
        include_inactive.unwrap_or(false),
        now_ms(),
    )?;
    Ok(rows.into_iter().map(view).collect())
}

/// Scopes with records, with counts.
#[tauri::command]
pub fn assist_memory_scopes() -> Result<Vec<ScopeView>, String> {
    let db = db::global()?;
    Ok(memory::scopes(&db, &AssistCtx::user(), now_ms())?
        .into_iter()
        .map(|summary| ScopeView {
            scope_key: summary.scope.key(),
            summary,
        })
        .collect())
}

/// Every version of a record's chain, oldest first.
#[tauri::command]
pub fn assist_memory_history(id: String) -> Result<Vec<MemoryView>, String> {
    let db = db::global()?;
    Ok(memory::history(&db, &AssistCtx::user(), &id)?
        .into_iter()
        .map(view)
        .collect())
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateMemory {
    pub scope: String,
    pub kind: String,
    pub content: String,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub valid_until: Option<i64>,
}

/// The person writes a memory themselves (always a stated fact unless they
/// pick another kind). Brings back something forgotten only because they
/// typed it again.
#[tauri::command]
pub fn assist_memory_create(input: CreateMemory) -> Result<MemoryView, String> {
    let db = db::global()?;
    let now = now_ms();
    let mut new = NewMemory::new(
        parse_scope(&input.scope)?,
        parse_category(&input.kind)?,
        &input.content,
        Source::user(now),
    );
    new.subject = input.subject.filter(|s| !s.trim().is_empty());
    new.valid_until = input.valid_until;
    Ok(view(
        memory::remember(&db, &AssistCtx::user(), new, now)?.record,
    ))
}

#[derive(Debug, Clone, Deserialize)]
pub struct EditMemory {
    pub id: String,
    pub content: String,
    #[serde(default)]
    pub kind: Option<String>,
}

/// Edit = a new version stated by the person; the old one stays in history.
#[tauri::command]
pub fn assist_memory_correct(input: EditMemory) -> Result<MemoryView, String> {
    let db = db::global()?;
    let now = now_ms();
    let category = input.kind.as_deref().map(parse_category).transpose()?;
    // Learned procedure that leaned on the old text loses that support.
    let _ = omniget_core::core::assist::learning::on_memory_changed(
        &db,
        &input.id,
        "memory corrected by the user",
    );
    Ok(view(memory::correct(
        &db,
        &AssistCtx::user(),
        &input.id,
        Correction {
            content: input.content,
            category,
            ..Default::default()
        },
        Source::user(now),
        now,
    )?))
}

/// Confirms a guess: it becomes a stated preference.
#[tauri::command]
pub fn assist_memory_confirm(id: String) -> Result<MemoryView, String> {
    let db = db::global()?;
    Ok(view(memory::confirm(
        &db,
        &AssistCtx::user(),
        &id,
        now_ms(),
    )?))
}

/// Discards a guess (or withdraws any record) without erasing its history.
#[tauri::command]
pub fn assist_memory_retract(id: String) -> Result<MemoryView, String> {
    let db = db::global()?;
    Ok(view(memory::retract(
        &db,
        &AssistCtx::user(),
        &id,
        now_ms(),
    )?))
}

/// Forgets a record and every version of it. Returns how many rows went.
#[tauri::command]
pub fn assist_memory_forget(id: String) -> Result<usize, String> {
    let db = db::global()?;
    let _ = omniget_core::core::assist::learning::on_memory_changed(
        &db,
        &id,
        "memory forgotten by the user",
    );
    memory::forget(&db, &AssistCtx::user(), &id, now_ms())
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportOut {
    /// Where the file was written, when a path was given.
    pub path: Option<String>,
    /// The export itself, when no path was given.
    pub content: Option<String>,
}

/// `format`: `"json"` (importable) or `"markdown"` (readable).
#[tauri::command]
pub fn assist_memory_export(format: String, path: Option<String>) -> Result<ExportOut, String> {
    let db = db::global()?;
    let ctx = AssistCtx::user();
    let now = now_ms();
    let content = match format.as_str() {
        "json" => memory::export_json(&db, &ctx, now)?,
        "markdown" | "md" => memory::export_markdown(&db, &ctx, now)?,
        _ => return Err(format!("{}: unknown format", memory::ERR_MEMORY_INVALID)),
    };
    match path.filter(|p| !p.trim().is_empty()) {
        Some(p) => {
            std::fs::write(&p, content)
                .map_err(|e| format!("{}: writing the file: {e}", memory::ERR_MEMORY_IMPORT))?;
            Ok(ExportOut {
                path: Some(p),
                content: None,
            })
        }
        None => Ok(ExportOut {
            path: None,
            content: Some(content),
        }),
    }
}

/// Imports a JSON export from `path` or from `content`.
#[tauri::command]
pub fn assist_memory_import(
    path: Option<String>,
    content: Option<String>,
    allow_resurrect: Option<bool>,
) -> Result<memory::ImportReport, String> {
    let db = db::global()?;
    let text = match (path.filter(|p| !p.trim().is_empty()), content) {
        (Some(p), _) => std::fs::read_to_string(&p)
            .map_err(|e| format!("{}: reading the file: {e}", memory::ERR_MEMORY_IMPORT))?,
        (None, Some(c)) => c,
        (None, None) => {
            return Err(format!("{}: nothing to import", memory::ERR_MEMORY_IMPORT));
        }
    };
    memory::import_json(
        &db,
        &AssistCtx::user(),
        &text,
        allow_resurrect.unwrap_or(false),
        now_ms(),
    )
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupsOut {
    pub count: usize,
    pub newest_ms: Option<i64>,
}

/// The app's automatic database copies that may still hold forgotten text.
#[tauri::command]
pub fn assist_memory_backups() -> Result<BackupsOut, String> {
    let db = db::global()?;
    let files = memory::backup_files(&db);
    let newest_ms = files
        .iter()
        .filter_map(|p| p.file_name()?.to_str()?.rsplit("bak-").next()?.parse().ok())
        .max();
    Ok(BackupsOut {
        count: files.len(),
        newest_ms,
    })
}

/// Deletes those copies. Returns how many files were removed.
#[tauri::command]
pub fn assist_memory_delete_backups() -> Result<usize, String> {
    let db = db::global()?;
    memory::delete_backups(&db)
}

/// Rebuilds the search index from the records (respecting forgotten items).
#[tauri::command]
pub fn assist_memory_reindex() -> Result<usize, String> {
    let db = db::global()?;
    memory::reindex(&db)
}

// ── Hindsight-style layer: stats, entities, explained search, settings ──

#[tauri::command]
pub fn assist_memory_stats() -> Result<memory::Stats, String> {
    let db = db::global()?;
    memory::stats(&db, now_ms())
}

#[tauri::command]
pub fn assist_memory_entities(limit: Option<usize>) -> Result<Vec<memory::Entity>, String> {
    let db = db::global()?;
    memory::entities(&db, &AssistCtx::user(), limit.unwrap_or(60))
}

/// One hit of the explained search, shaped like the list rows.
#[derive(Debug, Clone, Serialize)]
pub struct HitView {
    #[serde(flatten)]
    pub view: MemoryView,
    pub score: f32,
    pub why: memory::Why,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchView {
    pub hits: Vec<HitView>,
    pub entities: Vec<String>,
    pub window: Option<(i64, i64)>,
    pub semantic: bool,
}

/// Hybrid recall over every scope, with why each memory came back.
#[tauri::command]
pub async fn assist_memory_search(
    query: String,
    limit: Option<usize>,
) -> Result<SearchView, String> {
    let db = db::global()?;
    let probe = tokio::task::spawn_blocking(move || {
        memory::search_explained(
            &db,
            &AssistCtx::user(),
            &query,
            limit.unwrap_or(20),
            now_ms(),
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(SearchView {
        hits: probe
            .hits
            .into_iter()
            .map(|h| HitView {
                view: view(h.record),
                score: h.score,
                why: h.why,
            })
            .collect(),
        entities: probe.entities,
        window: probe.window,
        semantic: probe.semantic,
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemorySettingsInput {
    pub auto_retain: Option<bool>,
    pub consolidate: Option<bool>,
}

#[tauri::command]
pub fn assist_memory_set_settings(input: MemorySettingsInput) -> Result<memory::Stats, String> {
    let db = db::global()?;
    if let Some(v) = input.auto_retain {
        memory::retain::set_setting(&db, "auto_retain", v)?;
    }
    if let Some(v) = input.consolidate {
        memory::retain::set_setting(&db, "consolidate", v)?;
    }
    memory::stats(&db, now_ms())
}

/// Downloads the local embedding model once, embeds every memory and
/// rebuilds the notes. Returns how many memories got a vector.
#[tauri::command]
pub async fn assist_memory_prepare_semantic() -> Result<usize, String> {
    let db = db::global()?;
    memory::prepare_semantic(db).await
}

/// Reads a text the person pastes and saves the facts in it (their own
/// memory, stated on the memory screen).
#[tauri::command]
pub async fn assist_memory_retain_text(
    text: String,
    context: Option<String>,
) -> Result<memory::RetainReport, String> {
    let db = db::global()?;
    let input = memory::RetainInput {
        person: text,
        other: String::new(),
        event_ms: now_ms(),
        source_kind: "user".into(),
        source_id: None,
        context,
    };
    let report = memory::retain::retain(
        db.clone(),
        AssistCtx::user(),
        input,
        memory::retain::default_llm(),
    )
    .await?;
    memory::upkeep(db);
    Ok(report)
}
