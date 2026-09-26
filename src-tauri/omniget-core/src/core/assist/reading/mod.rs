//! Reading companion (spec 05): journeys, progress per edition, temporary
//! round context, access preferences, film identities, availability and
//! subtitle evidence, recommendation rounds and viewing events, with the
//! rules in [`rules`] enforced in the backend.
//!
//! Taste preferences are not kept here: they are generic memory and go to
//! `assist::memory` through the bot's memory tools. This module keeps only
//! what belongs to the journey.
//!
//! Reader integration: the Study plugin (0.3.6, proprietary, no sources on
//! this machine) answers `plugin_command("study", "study:read:library:list",
//! { filters: { search, pageSize } })` with `{ items: Book[], total }` where
//! `Book = { id, title, author, format, page_count, last_location,
//! reading_pct, last_opened_at, ... }` — checked against the plugin's
//! `read_books` table on 2026-09-24. The UI uses it, on the user's click, to
//! prefill a journey (external id `study:book:<id>`) and to record progress
//! as a percentage with origin `reader`. Manual entry works on its own.

pub mod rules;
pub mod skill;
pub mod store;

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::ctx::AssistCtx;
use super::db::{AssistDb, Migration};
use super::tools::{need_str, opt_str, spec, AssistToolset, ERR_ASSIST_TOOL};
use crate::core::llm::agent::{AgentDef, GrantMode, ToolSource};
use crate::core::llm::types::ToolSpec;

pub use rules::{assess, record_availability, record_round, record_subtitle, FinalStatus};
pub use store::*;

pub const MIGRATIONS: &[Migration] = store::MIGRATIONS;

/// Every tool this module offers; `bots` maps capabilities to these names.
pub const TOOL_NAMES: &[&str] = &[
    "reading_get_journey",
    "reading_start_journey",
    "reading_update_journey",
    "reading_record_progress",
    "reading_note_context",
    "reading_get_access_prefs",
    "reading_set_access_prefs",
    "reading_record_movie",
    "reading_record_availability",
    "reading_record_subtitle",
    "reading_assess_movie",
    "reading_record_round",
    "reading_record_viewing",
];

/// Spoiler terms of every unfinished journey `ctx` can see. Web tools use
/// them to refuse queries and to cut sentences from fetched pages.
pub fn spoiler_terms_for(db: &AssistDb, ctx: &AssistCtx) -> Vec<String> {
    if ctx.is_user_ui() {
        return Vec::new();
    }
    let bot = ctx.bot_id.as_deref();
    list_journeys(db, ctx, bot)
        .unwrap_or_default()
        .into_iter()
        .filter(|j| j.status != "finished")
        .flat_map(|j| j.spoiler_terms)
        .collect()
}

fn input_err(e: serde_json::Error) -> String {
    format!("{ERR_READING_INPUT}: {e}")
}

fn bot_of(ctx: &AssistCtx, input: &Value) -> Result<String, String> {
    if let Some(b) = &ctx.bot_id {
        return Ok(b.clone());
    }
    opt_str(input, "bot_id")
        .map(str::to_string)
        .ok_or_else(|| format!("{ERR_READING_INPUT}: `bot_id` is required"))
}

fn origin_of(ctx: &AssistCtx) -> &'static str {
    if ctx.is_user_ui() {
        "user"
    } else {
        "bot"
    }
}

fn movie_label(db: &AssistDb, id: &str) -> Value {
    match get_movie(db, id) {
        Ok(m) => serde_json::to_value(m).unwrap_or(Value::Null),
        Err(_) => json!({ "id": id }),
    }
}

/// Everything about one journey: for the tool (the bot) and the panel.
pub fn journey_detail(
    db: &AssistDb,
    ctx: &AssistCtx,
    journey_id: &str,
    now: i64,
) -> Result<Value, String> {
    let j = get_journey(db, ctx, journey_id)?;
    let prefs = get_prefs(db, ctx, &j.bot_id)?;
    let progress = progress_history(db, &j.id)?;
    let events = events_of(db, &j.id)?;
    let contexts = live_contexts(db, &j.id, now)?;
    let rounds = rounds_of(db, &j.id)?;

    let mut films = Vec::new();
    for latest in latest_by_movie(&events) {
        let mine: Vec<&ViewingEvent> = events
            .iter()
            .filter(|e| e.movie_id == latest.movie_id)
            .collect();
        films.push(json!({
            "movie": movie_label(db, &latest.movie_id),
            "state": latest.kind,
            "watched": mine.iter().any(|e| e.kind == "watched"),
            "events": mine,
        }));
    }
    let mut round_views = Vec::new();
    for (r, items) in &rounds {
        let mut iv = Vec::new();
        for it in items {
            let current = assess(db, &it.movie_id, &prefs, now)?;
            let availability = it
                .availability_id
                .as_deref()
                .and_then(|id| get_availability(db, id).ok());
            let subtitle = it
                .subtitle_id
                .as_deref()
                .and_then(|id| get_subtitle(db, id).ok());
            let why_evidence: Vec<Value> = it
                .evidence_event_ids
                .iter()
                .filter_map(|id| events.iter().find(|e| &e.id == id))
                .filter(|e| e.reaction.is_some() || !e.reasons.is_empty())
                .map(|e| {
                    json!({
                        "event_id": e.id,
                        "movie": movie_label(db, &e.movie_id),
                        "kind": e.kind,
                        "reaction": e.reaction,
                        "reasons": e.reasons,
                        "created_ms": e.created_ms,
                    })
                })
                .collect();
            let state = latest_by_movie(&events)
                .into_iter()
                .find(|e| e.movie_id == it.movie_id)
                .map(|e| e.kind.clone());
            iv.push(json!({
                "item": it,
                "movie": movie_label(db, &it.movie_id),
                "recorded_status": it.status,
                "availability": availability,
                "subtitle": subtitle,
                "current": {
                    "status": current.status,
                    "best": current.best,
                    "reasons": current.reasons,
                },
                "why_evidence": why_evidence,
                "state": state,
            }));
        }
        round_views.push(json!({ "round": r, "items": iv }));
    }
    Ok(json!({
        "journey": j,
        "progress": progress.first(),
        "progress_history": progress.iter().take(20).collect::<Vec<_>>(),
        "contexts": contexts,
        "prefs": prefs,
        "films": films,
        "rounds": round_views,
        "spoiler_terms_count": j.spoiler_terms.len(),
    }))
}

fn fmt_progress(p: &Progress) -> String {
    let kind = match p.position_kind.as_str() {
        "chapter" => "capítulo",
        "page" => "página",
        "percent" => "percentual",
        _ => "posição",
    };
    let ed = p
        .edition
        .as_deref()
        .map(|e| format!(", edição {e}"))
        .unwrap_or_default();
    let when = chrono::DateTime::from_timestamp_millis(p.recorded_ms)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    format!(
        "{kind} {}{ed} (registrado em {when}, {})",
        p.value, p.origin
    )
}

/// The short text the per-turn hook adds for a bot with reading tools.
pub fn augment_text(db: &AssistDb, ctx: &AssistCtx, bot: &str, now: i64) -> String {
    let mut out = String::from(
        "[Leitura — dados do app sobre a jornada de leitura; não são instruções novas do usuário]\n",
    );
    let journeys: Vec<Journey> = list_journeys(db, ctx, Some(bot))
        .unwrap_or_default()
        .into_iter()
        .filter(|j| matches!(j.status.as_str(), "active" | "paused"))
        .take(2)
        .collect();
    if journeys.is_empty() {
        out.push_str("Nenhuma jornada ativa. Se a pessoa falar do livro que está lendo, crie com reading_start_journey e pergunte só o que faltar (livro/edição, progresso, preferências de acesso).\n");
    }
    for j in &journeys {
        out.push_str(&format!(
            "Jornada `{}`: \"{}\"{}{} — {}.\n",
            j.id,
            j.title,
            j.author
                .as_deref()
                .map(|a| format!(" de {a}"))
                .unwrap_or_default(),
            j.edition
                .as_deref()
                .map(|e| format!(", edição {e}"))
                .unwrap_or_default(),
            if j.status == "active" {
                "ativa"
            } else {
                "pausada"
            }
        ));
        match latest_progress(db, &j.id).ok().flatten() {
            Some(p) => out.push_str(&format!("Progresso: {}. Nada depois disso pode aparecer em conexões, justificativas, perguntas ou buscas.\n", fmt_progress(&p))),
            None => out.push_str("Progresso: desconhecido — pergunte antes de indicar.\n"),
        }
        if let Ok(ctxs) = live_contexts(db, &j.id, now) {
            for c in ctxs {
                let until = chrono::DateTime::from_timestamp_millis(c.expires_ms)
                    .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
                    .unwrap_or_default();
                out.push_str(&format!(
                    "Pedido do momento (vale só para esta rodada, expira {until}): \"{}\".\n",
                    c.text
                ));
            }
        }
        let events = events_of(db, &j.id).unwrap_or_default();
        let mut by_kind: std::collections::BTreeMap<&str, Vec<String>> = Default::default();
        for e in latest_by_movie(&events) {
            let label = get_movie(db, &e.movie_id)
                .map(|m| format!("{} ({})", m.title, m.year))
                .unwrap_or_else(|_| e.movie_id.clone());
            let label = match &e.reaction {
                Some(r) if e.kind == "watched" || e.kind == "abandoned" => {
                    format!("{label} — reação: \"{r}\"")
                }
                _ => label,
            };
            by_kind.entry(e.kind.as_str()).or_default().push(label);
        }
        let names = [
            ("watched", "Vistos"),
            ("abandoned", "Abandonados"),
            ("declined", "Recusados agora (não significa não gostar)"),
            ("planned", "Planejados"),
            (
                "recommended",
                "Indicados e ainda sem retorno (não repita sem dizer que é retomada)",
            ),
        ];
        for (k, name) in names {
            if let Some(v) = by_kind.get(k) {
                out.push_str(&format!("{name}: {}.\n", v.join("; ")));
            }
        }
        if !j.spoiler_terms.is_empty() {
            out.push_str(&format!(
                "A pessoa marcou {} termo(s) como spoiler; buscas e textos que os contenham são recusados pelo app.\n",
                j.spoiler_terms.len()
            ));
        }
    }
    if let Ok(p) = get_prefs(db, ctx, bot) {
        out.push_str(&format!(
            "Acesso: região {}; assinaturas: {}; tipos aceitos: {}; {}; evidência vale {} dia(s).\n",
            p.region,
            if p.subscriptions.is_empty() { "não informadas".into() } else { p.subscriptions.join(", ") },
            p.access_kinds.join(", "),
            if p.only_confirmed { "somente confirmados (prováveis ficam fora)" } else { "prováveis permitidos com o que falta explicado" },
            p.freshness_days
        ));
    }
    out.push_str(
        "Regras do app: rodada de 1 a 3 filmes (a primeira com entrada, deslocamento e surpresa); nada de três pesados sem pedido; \
filmes que dependem do desfecho só após o livro concluído; acesso e legenda PT verificados nesta rodada com web_fetch e registrados como evidência; \
não confirmado fica fora da seleção (vale procurar se aparecer); indicado não é visto. Registre a rodada com reading_record_round — se o app recusar, corrija e não apresente a rodada recusada.\n",
    );
    out
}

// ── Toolset ──────────────────────────────────────────────────────────────

pub struct ReadingToolset {
    /// `None` = the app's database.
    pub db: Option<Arc<AssistDb>>,
}

impl ReadingToolset {
    fn db(&self) -> Result<Arc<AssistDb>, String> {
        match &self.db {
            Some(d) => Ok(d.clone()),
            None => super::db::global(),
        }
    }

    pub async fn run(&self, ctx: &AssistCtx, tool: &str, input: Value) -> Result<Value, String> {
        let db = self.db()?;
        let db = &*db;
        let now = rules::now();
        match tool {
            "reading_get_journey" => {
                if let Some(id) = opt_str(&input, "journey_id") {
                    return journey_detail(db, ctx, id, now);
                }
                let bot = bot_of(ctx, &input)?;
                let list = list_journeys(db, ctx, Some(&bot))?;
                let active = list.iter().find(|j| j.status == "active").or(list.first());
                Ok(json!({
                    "journeys": list.iter().map(|j| json!({"id": j.id, "title": j.title, "author": j.author, "status": j.status})).collect::<Vec<_>>(),
                    "detail": match active { Some(j) => journey_detail(db, ctx, &j.id, now)?, None => Value::Null },
                }))
            }
            "reading_start_journey" => {
                let bot = bot_of(ctx, &input)?;
                let nj: NewJourney = serde_json::from_value(input).map_err(input_err)?;
                let j = create_journey(db, ctx, &bot, nj)?;
                Ok(
                    json!({ "journey": j, "next": "Registre o progresso com reading_record_progress antes da primeira rodada." }),
                )
            }
            "reading_update_journey" => {
                let id = need_str(&input, "journey_id")?.to_string();
                let p: JourneyPatch = serde_json::from_value(input).map_err(input_err)?;
                Ok(json!({ "journey": update_journey(db, ctx, &id, p)? }))
            }
            "reading_record_progress" => {
                let id = need_str(&input, "journey_id")?;
                let kind = need_str(&input, "kind")?;
                let value = input
                    .get("value")
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .unwrap_or_default();
                let origin = if ctx.is_user_ui() {
                    opt_str(&input, "origin").unwrap_or("manual")
                } else {
                    "bot"
                };
                let out = record_progress(
                    db,
                    ctx,
                    id,
                    kind,
                    &value,
                    opt_str(&input, "edition").map(str::to_string),
                    origin,
                    opt_str(&input, "note").map(str::to_string),
                )?;
                Ok(json!({
                    "progress": out.progress,
                    "previous": out.compared_with.as_ref().map(|(p, _)| p),
                    "direction": out.compared_with.as_ref().map(|(_, o)| match o {
                        std::cmp::Ordering::Less => "back",
                        std::cmp::Ordering::Equal => "same",
                        std::cmp::Ordering::Greater => "forward",
                    }),
                    "not_comparable": out.not_comparable,
                }))
            }
            "reading_note_context" => {
                let id = need_str(&input, "journey_id")?;
                let text = need_str(&input, "text")?;
                let hours = input.get("hours").and_then(Value::as_i64);
                Ok(
                    json!({ "context": note_context(db, ctx, id, text, hours)?, "note": "Vale só para esta rodada; não é preferência permanente." }),
                )
            }
            "reading_get_access_prefs" => {
                let bot = bot_of(ctx, &input)?;
                Ok(json!({ "prefs": get_prefs(db, ctx, &bot)? }))
            }
            "reading_set_access_prefs" => {
                let bot = bot_of(ctx, &input)?;
                let p: PrefsPatch = serde_json::from_value(input).map_err(input_err)?;
                Ok(json!({ "prefs": set_prefs(db, ctx, &bot, p)? }))
            }
            "reading_record_movie" => {
                let m: NewMovie = serde_json::from_value(input).map_err(input_err)?;
                let (movie, created) = upsert_movie(db, m)?;
                Ok(json!({ "movie": movie, "created": created }))
            }
            "reading_record_availability" => {
                let a: rules::AvailabilityInput =
                    serde_json::from_value(input).map_err(input_err)?;
                let rec = record_availability(db, ctx, a, now)?;
                Ok(json!({ "availability": rec }))
            }
            "reading_record_subtitle" => {
                let s: rules::SubtitleInput = serde_json::from_value(input).map_err(input_err)?;
                let rec = record_subtitle(db, ctx, s, now)?;
                Ok(json!({ "subtitle": rec }))
            }
            "reading_assess_movie" => {
                let movie = need_str(&input, "movie_id")?;
                let bot = bot_of(ctx, &input)?;
                let mut prefs = get_prefs(db, ctx, &bot)?;
                if let Some(o) = input.get("only_confirmed").and_then(Value::as_bool) {
                    prefs.only_confirmed = o;
                }
                let a = assess(db, movie, &prefs, now)?;
                let eligible = match a.status {
                    FinalStatus::Confirmed => true,
                    FinalStatus::Probable => !prefs.only_confirmed,
                    FinalStatus::Unconfirmed => false,
                };
                Ok(json!({ "assessment": a, "eligible_for_main_selection": eligible }))
            }
            "reading_record_round" => {
                let r: rules::RoundInput = serde_json::from_value(input).map_err(input_err)?;
                let rec = record_round(db, ctx, r, now)?;
                Ok(
                    json!({ "recorded": rec, "note": "Indicado não é visto: marque visto/abandonado/recusado quando a pessoa contar." }),
                )
            }
            "reading_record_viewing" => {
                let id = need_str(&input, "journey_id")?;
                let movie = need_str(&input, "movie_id")?;
                let kind = need_str(&input, "kind")?;
                let reasons: Vec<String> = input
                    .get("reasons")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                let e = record_viewing(
                    db,
                    ctx,
                    id,
                    movie,
                    kind,
                    opt_str(&input, "reaction").map(str::to_string),
                    reasons,
                    origin_of(ctx),
                )?;
                Ok(json!({ "event": e }))
            }
            other => Err(format!("{ERR_ASSIST_TOOL}: unknown tool `{other}`")),
        }
    }
}

fn s(desc: &str) -> Value {
    json!({ "type": "string", "description": desc })
}

#[async_trait]
impl AssistToolset for ReadingToolset {
    fn name(&self) -> &'static str {
        "reading"
    }

    fn specs(&self) -> Vec<ToolSpec> {
        let obj = |props: Value, req: &[&str]| json!({ "type": "object", "properties": props, "required": req });
        vec![
            spec("reading_get_journey", "Read the reading journey: book, edition, progress, today's context, films recommended/watched/declined with reactions, rounds with access status, and access preferences. Without journey_id returns this bot's journeys and the active one in detail.",
                obj(json!({ "journey_id": s("Optional journey id.") }), &[])),
            spec("reading_start_journey", "Start a reading journey for a book the user is reading.",
                obj(json!({
                    "title": s("Book title."), "author": s("Author."), "edition": s("Edition/publisher, so pages are not compared across editions."),
                    "language": s("Language the user reads it in."), "external_id": s("Reader id, e.g. study:book:12."),
                    "spoiler_terms": { "type": "array", "items": {"type": "string"}, "description": "Words that would reveal what comes later; the app refuses searches and texts with them." },
                    "notes": s("Short notes.")
                }), &["title"])),
            spec("reading_update_journey", "Change a journey: status (active, paused, finished, abandoned), edition, spoiler terms, notes.",
                obj(json!({
                    "journey_id": s("Journey id."), "title": s(""), "author": s(""), "edition": s(""), "language": s(""),
                    "status": { "type": "string", "enum": JOURNEY_STATUSES },
                    "spoiler_terms": { "type": "array", "items": {"type": "string"} }, "notes": s("")
                }), &["journey_id"])),
            spec("reading_record_progress", "Record where the user is in the book. Pages/locations of different editions are never compared.",
                obj(json!({
                    "journey_id": s("Journey id."),
                    "kind": { "type": "string", "enum": POSITION_KINDS },
                    "value": s("Chapter, page, percentage (0-100) or location."),
                    "edition": s("Edition, when different from the journey's."), "note": s("")
                }), &["journey_id", "kind", "value"])),
            spec("reading_note_context", "Record a request valid only for the next round (e.g. 'hoje quero algo leve'); it expires and is not a permanent taste.",
                obj(json!({ "journey_id": s(""), "text": s("What the user asked for now."), "hours": {"type": "integer", "minimum": 1, "maximum": 48} }), &["journey_id", "text"])),
            spec("reading_get_access_prefs", "Read the user's streaming preferences for this bot: region, subscriptions, accepted access kinds, confirmed-only, freshness window.",
                obj(json!({}), &[])),
            spec("reading_set_access_prefs", "Change the streaming preferences the user stated (subscriptions they have, accepted access kinds, confirmed-only).",
                obj(json!({
                    "region": s("Two-letter country, default BR."),
                    "subscriptions": { "type": "array", "items": {"type": "string"} },
                    "access_kinds": { "type": "array", "items": {"type": "string", "enum": ACCESS_KINDS} },
                    "only_confirmed": {"type": "boolean"},
                    "freshness_days": {"type": "integer", "minimum": 1, "maximum": 90}
                }), &[])),
            spec("reading_record_movie", "Record a film's identity (title, original title, year, director). A namesake from another year is another film. Returns movie_id.",
                obj(json!({
                    "title": s("Title (Brazilian title when there is one)."), "original_title": s(""), "year": {"type": "integer"},
                    "director": s(""), "external_ids": {"type": "object"}
                }), &["title", "year"])),
            spec("reading_record_availability", "Record where a film can be watched, per region/platform/access kind. `confirmed` and `absent` need the fetch_id of a web_fetch of that page and a literal quote from it; otherwise use `inferred` with a note of what is missing.",
                obj(json!({
                    "movie_id": s(""), "region": s("Two-letter country, BR for Brazil (explicit)."), "platform": s("Netflix, Prime Video..."),
                    "access_kind": {"type": "string", "enum": ACCESS_KINDS},
                    "url": s("The page."), "status": {"type": "string", "enum": EVIDENCE_STATUSES},
                    "source_kind": {"type": "string", "enum": ["platform", "aggregator", "search"]},
                    "quote": s("Literal words from the fetched page."), "fetch_id": s("From web_fetch."),
                    "page_date_ms": {"type": "integer", "description": "Date the page itself claims, when it shows one."},
                    "note": s("What is missing / caveats.")
                }), &["movie_id", "region", "platform", "access_kind", "status", "source_kind"])),
            spec("reading_record_subtitle", "Record Portuguese subtitles (or audio) for one availability record. Audio in Portuguese is kind=audio and never counts as subtitles. `confirmed`/`absent` need fetch_id + a quote that mentions subtitles; a login wall means login_required=true and at most `inferred`.",
                obj(json!({
                    "availability_id": s(""), "language": s("Default pt-BR."),
                    "kind": {"type": "string", "enum": ["subtitle", "audio"]},
                    "status": {"type": "string", "enum": EVIDENCE_STATUSES},
                    "source_kind": {"type": "string", "enum": ["platform", "aggregator", "search"]},
                    "url": s(""), "quote": s(""), "fetch_id": s(""), "login_required": {"type": "boolean"}, "note": s("")
                }), &["availability_id", "status", "source_kind"])),
            spec("reading_assess_movie", "Compute the access status of a film from the recorded evidence: confirmed, probable (with what is missing) or unconfirmed (with reasons). Old evidence does not count.",
                obj(json!({ "movie_id": s(""), "only_confirmed": {"type": "boolean"} }), &["movie_id"])),
            spec("reading_record_round", "Record a recommendation round. The app validates it (1-3 films; first round one entry/shift/surprise; not three heavy; no watched/repeated films unless resumes=true; nothing depending on the ending before the book is finished; no spoiler terms; access confirmed or probable) and refuses an invalid round listing what to fix.",
                obj(json!({
                    "journey_id": s(""), "reason": s("Why this round now."),
                    "requested_count": {"type": "integer", "minimum": 1, "maximum": 3},
                    "shortfall_reason": s("Why fewer films than asked."),
                    "allow_heavy": {"type": "boolean"}, "only_confirmed": {"type": "boolean"},
                    "skill_name": s("Skill applied, if any."),
                    "items": {"type": "array", "minItems": 1, "maxItems": 3, "items": {"type": "object", "properties": {
                        "movie_id": s(""), "role": {"type": "string", "enum": rules::ROLES},
                        "connection": s("Specific connection with the book, no spoilers past the progress."),
                        "why": s("One sentence: why you may like it."),
                        "moods": {"type": "array", "items": {"type": "string", "enum": rules::MOODS}},
                        "pace": {"type": "string", "enum": rules::PACES},
                        "heavy": {"type": "boolean"}, "depends_on_ending": {"type": "boolean"}, "resumes": {"type": "boolean"},
                        "questions": {"type": "array", "items": {"type": "string"}, "maxItems": 2},
                        "evidence_event_ids": {"type": "array", "items": {"type": "string"}, "description": "Viewing events whose reaction motivated this pick."}
                    }, "required": ["movie_id", "role", "connection", "why", "moods", "pace"]}},
                    "look_for": {"type": "array", "items": {"type": "object", "properties": {"movie_id": s(""), "note": s("")}, "required": ["movie_id"]}}
                }), &["journey_id", "items"])),
            spec("reading_record_viewing", "Record what the user did with a film: planned, watched, abandoned or declined (declining now is not disliking), with an optional short reaction.",
                obj(json!({
                    "journey_id": s(""), "movie_id": s(""),
                    "kind": {"type": "string", "enum": ["planned", "watched", "abandoned", "declined"]},
                    "reaction": s("The user's words, short."), "reasons": {"type": "array", "items": {"type": "string"}}
                }), &["journey_id", "movie_id", "kind"])),
        ]
    }

    async fn call(&self, ctx: &AssistCtx, tool: &str, input: Value) -> Result<Value, String> {
        self.run(ctx, tool, input).await
    }
}

pub fn toolset() -> Arc<dyn AssistToolset> {
    Arc::new(ReadingToolset { db: None })
}

// ── Per-turn hook ────────────────────────────────────────────────────────

pub struct ReadingAugment {
    pub db: Option<Arc<AssistDb>>,
}

/// True when the agent may call at least one reading tool.
pub fn has_reading_grant(agent: &AgentDef) -> bool {
    agent.tools.iter().any(|g| {
        g.mode != GrantMode::Deny
            && matches!(&g.source, ToolSource::Internal { name } if TOOL_NAMES.contains(&name.as_str()))
    })
}

impl crate::core::llm::coordinator::TurnAugment for ReadingAugment {
    fn augment(
        &self,
        agent: &mut AgentDef,
        conversation_id: &str,
        _user_input: &str,
    ) -> Option<String> {
        if !has_reading_grant(agent) {
            return None;
        }
        let db = match &self.db {
            Some(d) => d.clone(),
            None => super::db::global().ok()?,
        };
        let ctx = super::ctx::resolve(&agent.id, Some(conversation_id));
        Some(augment_text(&db, &ctx, &agent.id, rules::now()))
    }
}

/// Per-turn hook (see `llm::coordinator::TurnAugment`).
pub fn augment() -> Arc<dyn crate::core::llm::coordinator::TurnAugment> {
    Arc::new(ReadingAugment { db: None })
}

#[cfg(test)]
mod tests;
