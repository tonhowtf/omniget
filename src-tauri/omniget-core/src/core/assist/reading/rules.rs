//! The deterministic rules of spec 05, enforced in the backend whatever the
//! prompt says:
//!
//! - access status: **Confirmed** (availability in the region + PT subtitles
//!   on that option), **Probable** (availability confirmed, subtitles only
//!   inferred/unknown, with what is missing spelled out), **Not confirmed**
//!   (anything else). PT audio is not PT subtitles. An option whose platform
//!   says there are no PT subtitles is out, never "probable". Evidence older
//!   than the freshness window, or a page that dates itself older, is not
//!   current. When a platform page and an aggregator disagree, the platform
//!   wins. The newest fresh record of an option wins, so "left the
//!   subscription" replaces an older "available".
//! - evidence a bot records as `confirmed`/`absent` must be grounded on a
//!   `web_fetch` the backend really made (fetch id, same site, fresh,
//!   literal quote found in the page text, film year on the page). A
//!   confirmed availability whose quote only shows that the page exists
//!   ("Globo Filmes", the title and year) and no access signal (rent, buy,
//!   subscription, a price… see [`ACCESS_SIGNALS`]) counts as **Probable**,
//!   with that reason, never Confirmed;
//! - rounds: 1–3 films; the first round has three (one per role) unless a
//!   shortfall is explained; no three heavy films unless asked; no film the
//!   user watched/abandoned; no repeat unless marked as a resumption; nothing
//!   whose connection depends on the ending before the book is finished; no
//!   spoiler term in any justification; "not confirmed" never in the main
//!   selection (it goes to "worth looking for"), "probable" out too when the
//!   user asked for confirmed only; progress must be known.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::super::ctx::AssistCtx;
use super::super::db::AssistDb;
use super::super::web::{self, extract::fold};
use super::super::{new_id, now_ms};
use super::store::*;

pub const ERR_READING_EVIDENCE: &str = "ERR_READING_EVIDENCE";
pub const ERR_READING_ROUND: &str = "ERR_READING_ROUND";

pub const ROLES: &[&str] = &["entry", "shift", "surprise"];
pub const MOODS: &[&str] = &["leve", "divertido", "romantico", "comovente", "intenso"];
pub const PACES: &[&str] = &["easy", "attentive"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalStatus {
    Unconfirmed,
    Probable,
    Confirmed,
}

impl FinalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            FinalStatus::Confirmed => "confirmed",
            FinalStatus::Probable => "probable",
            FinalStatus::Unconfirmed => "unconfirmed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionAssessment {
    pub availability: Availability,
    pub subtitle: Option<Subtitle>,
    pub status: FinalStatus,
    /// What is missing for "confirmed", or why the option is out.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assessment {
    pub movie_id: String,
    pub status: FinalStatus,
    pub best: Option<OptionAssessment>,
    pub options: Vec<OptionAssessment>,
    /// Why options were set aside (region, stale, gone, incompatible...).
    pub reasons: Vec<String>,
    pub assessed_ms: i64,
}

fn day(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| ms.to_string())
}

fn is_pt(lang: &str) -> bool {
    let l = fold(lang);
    l == "pt" || l.starts_with("pt-") || l.starts_with("pt_") || l.starts_with("portugu")
}

/// Age that counts: the page's own date when it is older than our check.
fn effective_ms(checked: i64, page_date: Option<i64>) -> i64 {
    page_date.map(|d| d.min(checked)).unwrap_or(checked)
}

fn source_rank(kind: &str) -> u8 {
    match kind {
        "platform" => 3,
        "user" => 2,
        "aggregator" => 1,
        _ => 0,
    }
}

/// Picks the record that speaks for one option: among fresh rows, the most
/// authoritative source, newest first.
fn pick<'a, T>(rows: &[&'a T], rank: impl Fn(&T) -> u8, when: impl Fn(&T) -> i64) -> Option<&'a T> {
    rows.iter()
        .copied()
        .max_by(|a, b| rank(a).cmp(&rank(b)).then(when(a).cmp(&when(b))))
}

/// The access status of `movie_id` for these preferences at `now`.
pub fn assess(
    db: &AssistDb,
    movie_id: &str,
    prefs: &AccessPrefs,
    now: i64,
) -> Result<Assessment, String> {
    let avail = availability_of(db, movie_id)?;
    let subs = subtitles_of(db, movie_id)?;
    let window = prefs.window_ms();
    let fresh = |checked: i64, page: Option<i64>| now - effective_ms(checked, page) <= window;
    let mut reasons = Vec::new();
    let mut options = Vec::new();

    if avail.is_empty() {
        reasons.push("nenhuma evidência de onde assistir foi registrada".to_string());
    }
    // Group by (region, platform, access kind).
    type Group<'a> = ((String, String, String), Vec<&'a Availability>);
    let mut groups: Vec<Group> = Vec::new();
    for a in &avail {
        let k = (
            a.region.to_ascii_uppercase(),
            a.platform_key.clone(),
            a.access_kind.clone(),
        );
        match groups.iter_mut().find(|(g, _)| *g == k) {
            Some((_, v)) => v.push(a),
            None => groups.push((k, vec![a])),
        }
    }
    let region = prefs.region.to_ascii_uppercase();
    let subs_keys: Vec<String> = prefs
        .subscriptions
        .iter()
        .map(|s| platform_key(s))
        .collect();
    for ((reg, _pk, kind), rows) in &groups {
        let label = format!("{} ({})", rows[0].platform, kind);
        if *reg != region {
            reasons.push(format!(
                "{label}: evidência é do catálogo {reg}, não de {region}"
            ));
            continue;
        }
        let fresh_rows: Vec<&Availability> = rows
            .iter()
            .copied()
            .filter(|a| fresh(a.checked_ms, a.page_date_ms))
            .collect();
        let Some(chosen) = pick(
            &fresh_rows,
            |a| source_rank(&a.source_kind),
            |a| a.checked_ms,
        ) else {
            let newest = rows
                .iter()
                .map(|a| effective_ms(a.checked_ms, a.page_date_ms))
                .max()
                .unwrap_or(0);
            reasons.push(format!(
                "{label}: evidência antiga ({}), precisa ser verificada de novo antes de indicar",
                day(newest)
            ));
            continue;
        };
        let gap = access_gap(chosen);
        match chosen.status.as_str() {
            "confirmed" => {}
            "absent" => {
                reasons.push(format!(
                    "{label}: indisponível segundo {} em {}",
                    chosen.source_kind,
                    day(chosen.checked_ms)
                ));
                continue;
            }
            _ => {
                reasons.push(format!("{label}: disponibilidade sem evidência suficiente"));
                continue;
            }
        }
        if !prefs.access_kinds.iter().any(|k| k == kind) {
            reasons.push(format!("{label}: tipo de acesso fora das preferências"));
            continue;
        }
        if kind == "subscription"
            && !subs_keys.is_empty()
            && !subs_keys.contains(&chosen.platform_key)
        {
            reasons.push(format!("{label}: assinatura que o usuário não tem"));
            continue;
        }
        // Subtitles for this option: same platform (and this record or none).
        let cand: Vec<&Subtitle> = subs
            .iter()
            .filter(|s| s.platform_key == chosen.platform_key && is_pt(&s.language))
            .filter(|s| fresh(s.checked_ms, None))
            .collect();
        let sub_rows: Vec<&Subtitle> = cand
            .iter()
            .copied()
            .filter(|s| s.kind == "subtitle")
            .collect();
        let audio_pt = cand
            .iter()
            .any(|s| s.kind == "audio" && s.status == "confirmed");
        let sub = pick(&sub_rows, |s| source_rank(&s.source_kind), |s| s.checked_ms);
        let mut notes = Vec::new();
        let status = match sub.map(|s| s.status.as_str()) {
            Some("confirmed") => FinalStatus::Confirmed,
            Some("absent") => {
                reasons.push(format!(
                    "{label}: a plataforma informa que não há legenda em português ({})",
                    day(sub.map(|s| s.checked_ms).unwrap_or(0))
                ));
                continue;
            }
            other => {
                let mut what = match other {
                    Some("inferred") => {
                        "legenda em português apenas inferida, não vista na página".to_string()
                    }
                    _ => "legenda em português não verificada nesta opção".to_string(),
                };
                if sub.map(|s| s.login_required).unwrap_or(false) {
                    what.push_str(" (a página da plataforma pede login para mostrar os idiomas)");
                }
                notes.push(what);
                if audio_pt {
                    notes.push("há áudio em português, mas áudio não comprova legenda".into());
                }
                if let Some(n) = sub.and_then(|s| s.note.clone()) {
                    notes.push(n);
                }
                FinalStatus::Probable
            }
        };
        // The page exists, but the quote never showed a way to watch it.
        let status = match gap {
            Some(why) => {
                notes.insert(0, why);
                status.min(FinalStatus::Probable)
            }
            None => status,
        };
        options.push(OptionAssessment {
            availability: chosen.clone(),
            subtitle: sub.cloned(),
            status,
            notes,
        });
    }
    options.sort_by(|a, b| {
        b.status
            .cmp(&a.status)
            .then(b.availability.checked_ms.cmp(&a.availability.checked_ms))
    });
    let best = options.first().cloned();
    Ok(Assessment {
        movie_id: movie_id.into(),
        status: best
            .as_ref()
            .map(|b| b.status)
            .unwrap_or(FinalStatus::Unconfirmed),
        best,
        options,
        reasons,
        assessed_ms: now,
    })
}

// ── Evidence ─────────────────────────────────────────────────────────────

/// Words that say a film can be watched on a page (rent, buy, subscription,
/// stream, available…), folded (lowercase, no accents), per language. A quote
/// needs one of them (or a price) to prove availability, not just that the
/// page exists.
pub const ACCESS_SIGNALS: &[(&str, &[&str])] = &[
    (
        "pt",
        &[
            "alugar",
            "alugue",
            "aluguel",
            "comprar",
            "compre",
            "compra",
            "assinatura",
            "assinaturas",
            "assinante",
            "assinantes",
            "assistir",
            "assista",
            "disponivel",
            "disponiveis",
            "incluido",
            "incluida",
            "incluso",
            "gratis",
            "gratuito",
            "streaming",
            "plano",
            "planos",
        ],
    ),
    (
        "en",
        &[
            "rent",
            "rental",
            "buy",
            "purchase",
            "stream",
            "streaming",
            "watch",
            "subscription",
            "subscribe",
            "subscribers",
            "available",
            "included",
            "free",
            "plan",
        ],
    ),
    (
        "es",
        &[
            "alquilar",
            "alquiler",
            "comprar",
            "suscripcion",
            "suscriptores",
            "disponible",
            "incluido",
            "gratis",
            "mirar",
        ],
    ),
    (
        "fr",
        &[
            "louer",
            "location",
            "acheter",
            "achat",
            "abonnement",
            "abonnes",
            "disponible",
            "regarder",
            "inclus",
            "gratuit",
        ],
    ),
    (
        "de",
        &[
            "leihen",
            "kaufen",
            "abo",
            "abonnement",
            "verfugbar",
            "ansehen",
            "streamen",
            "inklusive",
            "kostenlos",
        ],
    ),
    (
        "it",
        &[
            "noleggio",
            "noleggia",
            "noleggiare",
            "acquista",
            "acquistare",
            "acquisto",
            "abbonamento",
            "disponibile",
            "guarda",
            "incluso",
            "gratis",
        ],
    ),
];

/// Whether `quote` shows that the film can be watched there: an access word
/// of [`ACCESS_SIGNALS`] or a price (`R$ 14,90`, `$3.99`, `4,99 €`).
pub fn shows_access(quote: &str) -> bool {
    static PRICE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let price = PRICE.get_or_init(|| {
        regex::Regex::new(r"(?:[$€£]|R\$|US\$)\s?\d|\d\s?(?:€|£|reais|euros?|dollars?)")
            .expect("price pattern")
    });
    if price.is_match(quote) {
        return true;
    }
    let f = fold(quote);
    let words: Vec<&str> = f
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    ACCESS_SIGNALS
        .iter()
        .any(|(_, list)| list.iter().any(|s| words.contains(s)))
}

/// Why a confirmed availability only counts as probable, if it does: a bot's
/// quote that proves the page exists but not that the film can be watched.
fn access_gap(a: &Availability) -> Option<String> {
    if a.status != "confirmed" || a.source_kind == "user" || a.bot_id.is_none() {
        return None;
    }
    match a.quote.as_deref() {
        Some(q) if shows_access(q) => None,
        Some(q) => Some(format!(
            "o trecho \"{}\" mostra que a página existe, não que dá para {} lá (falta preço, aluguel, compra ou assinatura na página)",
            super::super::missions::clip(q, 80),
            access_verb(&a.access_kind)
        )),
        None => Some("a disponibilidade não tem trecho da página que mostre como assistir".into()),
    }
}

fn access_verb(kind: &str) -> &'static str {
    match kind {
        "rent" => "alugar",
        "buy" => "comprar",
        "subscription" => "assistir na assinatura",
        _ => "assistir",
    }
}

fn host_of(u: &str) -> Option<String> {
    url::Url::parse(u).ok().and_then(|u| {
        u.host_str()
            .map(|h| h.trim_start_matches("www.").to_ascii_lowercase())
    })
}

/// Checks that a bot's claim rests on a page this app really fetched.
/// Returns the URL to store (the evidence URL, or the page's final URL).
fn ground(
    db: &AssistDb,
    fetch_id: Option<&str>,
    url: Option<&str>,
    quote: Option<&str>,
    now: i64,
    window: i64,
    year: Option<i64>,
) -> Result<String, String> {
    let fid = fetch_id.ok_or_else(|| {
        format!("{ERR_READING_EVIDENCE}: `confirmed`/`absent` needs the fetch_id of a web_fetch of the page (search snippets and memory do not count). Otherwise record it as `inferred` and say what is missing.")
    })?;
    let rec = web::fetch_record(db, fid).ok_or_else(|| {
        format!(
            "{ERR_READING_EVIDENCE}: fetch `{fid}` does not exist or failed; fetch the page again"
        )
    })?;
    let status = rec.status.unwrap_or(0);
    if !(200..300).contains(&status) {
        return Err(format!(
            "{ERR_READING_EVIDENCE}: that page answered HTTP {status} (login or block), so it cannot confirm anything. \
Record `inferred` with login_required=true and say what is missing."
        ));
    }
    if now - rec.fetched_ms > window {
        return Err(format!(
            "{ERR_READING_EVIDENCE}: that fetch is from {}, older than the freshness window; fetch the page again",
            day(rec.fetched_ms)
        ));
    }
    let stored_url = url
        .map(str::to_string)
        .unwrap_or_else(|| rec.final_url.clone().unwrap_or(rec.url.clone()));
    let h = host_of(&stored_url);
    let ok_host =
        h.is_some() && (h == host_of(&rec.url) || h == rec.final_url.as_deref().and_then(host_of));
    if !ok_host {
        return Err(format!(
            "{ERR_READING_EVIDENCE}: the evidence URL is not the page that was fetched ({})",
            rec.final_url.unwrap_or(rec.url)
        ));
    }
    let q = quote.map(fold).unwrap_or_default();
    if q.chars().count() < 6 {
        return Err(format!("{ERR_READING_EVIDENCE}: give a short literal quote from the page (at least a few words)"));
    }
    let text = fold(&rec.text);
    if !text.contains(&q) {
        return Err(format!(
            "{ERR_READING_EVIDENCE}: the quote is not in the fetched page; copy the words exactly as the page shows them"
        ));
    }
    if let Some(y) = year {
        let t = format!(
            "{} {}",
            rec.title.as_deref().map(fold).unwrap_or_default(),
            text
        );
        if !t.contains(&y.to_string()) {
            return Err(format!(
                "{ERR_READING_EVIDENCE}: the page never shows the year {y}; it may be a namesake. Confirm it is the same film (title, year, director) before confirming"
            ));
        }
    }
    Ok(stored_url)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AvailabilityInput {
    pub movie_id: String,
    pub region: String,
    pub platform: String,
    pub access_kind: String,
    #[serde(default)]
    pub url: Option<String>,
    pub status: String,
    pub source_kind: String,
    #[serde(default)]
    pub quote: Option<String>,
    #[serde(default)]
    pub fetch_id: Option<String>,
    /// Date the page itself claims ("catálogo atualizado em ..."), ms.
    #[serde(default)]
    pub page_date_ms: Option<i64>,
    #[serde(default)]
    pub note: Option<String>,
}

fn check_enum(v: &str, allowed: &[&str], what: &str) -> Result<(), String> {
    if allowed.contains(&v) {
        Ok(())
    } else {
        Err(format!(
            "{ERR_READING_EVIDENCE}: {what} must be one of {allowed:?}"
        ))
    }
}

pub fn record_availability(
    db: &AssistDb,
    ctx: &AssistCtx,
    input: AvailabilityInput,
    now: i64,
) -> Result<Availability, String> {
    let movie = get_movie(db, &input.movie_id)?;
    let region = input.region.trim().to_ascii_uppercase();
    if region.len() != 2 || !region.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(format!("{ERR_READING_EVIDENCE}: region must be explicit, a two-letter country code (BR for Brazil)"));
    }
    check_enum(&input.access_kind, ACCESS_KINDS, "access_kind")?;
    check_enum(&input.status, EVIDENCE_STATUSES, "status")?;
    check_enum(&input.source_kind, SOURCE_KINDS, "source_kind")?;
    if input.platform.trim().is_empty() {
        return Err(format!("{ERR_READING_EVIDENCE}: platform is required"));
    }
    let bot = !ctx.is_user_ui();
    if bot && input.source_kind == "user" {
        return Err(format!(
            "{ERR_READING_EVIDENCE}: `user` evidence is recorded by the user in the app"
        ));
    }
    if bot
        && input.source_kind == "search"
        && matches!(input.status.as_str(), "confirmed" | "absent")
    {
        return Err(format!(
            "{ERR_READING_EVIDENCE}: a search snippet cannot confirm; open the page with web_fetch"
        ));
    }
    if bot
        && input.status == "inferred"
        && input
            .note
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
    {
        return Err(format!(
            "{ERR_READING_EVIDENCE}: say in `note` what is missing to confirm it"
        ));
    }
    let prefs = match &ctx.bot_id {
        Some(b) => get_prefs(db, ctx, b)?,
        None => AccessPrefs::defaults(""),
    };
    let url = if bot && matches!(input.status.as_str(), "confirmed" | "absent") {
        let year = (input.status == "confirmed").then_some(movie.year);
        Some(ground(
            db,
            input.fetch_id.as_deref(),
            input.url.as_deref(),
            input.quote.as_deref(),
            now,
            prefs.window_ms(),
            year,
        )?)
    } else {
        input.url.clone()
    };
    if let Some(u) = &url {
        web::check_url(u).map_err(|e| format!("{ERR_READING_EVIDENCE}: {e}"))?;
    }
    // A bot's confirmed quote with no access signal is kept, but says it
    // only counts as probable (the model reads this in the tool result).
    let mut note = input
        .note
        .map(|q| q.trim().to_string())
        .filter(|q| !q.is_empty());
    if bot && input.status == "confirmed" && !input.quote.as_deref().is_some_and(shows_access) {
        let why = "conta como provável: o trecho não mostra preço, aluguel, compra ou assinatura; cite um trecho que mostre como assistir para confirmar";
        note = Some(match note {
            Some(n) => format!("{n} · {why}"),
            None => why.to_string(),
        });
    }
    let a = Availability {
        id: new_id(),
        movie_id: movie.id,
        region,
        platform: input.platform.trim().into(),
        platform_key: platform_key(&input.platform),
        access_kind: input.access_kind,
        url,
        status: input.status,
        source_kind: input.source_kind,
        quote: input
            .quote
            .map(|q| q.trim().to_string())
            .filter(|q| !q.is_empty()),
        fetch_id: input.fetch_id,
        page_date_ms: input.page_date_ms,
        note,
        bot_id: ctx.bot_id.clone(),
        checked_ms: now,
    };
    insert_availability(db, &a)?;
    Ok(a)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubtitleInput {
    pub availability_id: String,
    #[serde(default = "pt_default")]
    pub language: String,
    /// `subtitle` or `audio` (audio never counts as subtitles).
    #[serde(default = "subtitle_default")]
    pub kind: String,
    pub status: String,
    pub source_kind: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub quote: Option<String>,
    #[serde(default)]
    pub fetch_id: Option<String>,
    #[serde(default)]
    pub login_required: bool,
    #[serde(default)]
    pub note: Option<String>,
}

fn pt_default() -> String {
    "pt-BR".into()
}
fn subtitle_default() -> String {
    "subtitle".into()
}

pub fn record_subtitle(
    db: &AssistDb,
    ctx: &AssistCtx,
    input: SubtitleInput,
    now: i64,
) -> Result<Subtitle, String> {
    let avail = get_availability(db, &input.availability_id)?;
    check_enum(&input.kind, &["subtitle", "audio"], "kind")?;
    check_enum(&input.status, EVIDENCE_STATUSES, "status")?;
    check_enum(&input.source_kind, SOURCE_KINDS, "source_kind")?;
    let bot = !ctx.is_user_ui();
    if bot && input.source_kind == "user" {
        return Err(format!(
            "{ERR_READING_EVIDENCE}: `user` evidence is recorded by the user in the app"
        ));
    }
    if bot
        && input.source_kind == "search"
        && matches!(input.status.as_str(), "confirmed" | "absent")
    {
        return Err(format!("{ERR_READING_EVIDENCE}: a search snippet cannot confirm subtitles; open the platform's title page"));
    }
    if input.login_required && input.status == "confirmed" {
        return Err(format!("{ERR_READING_EVIDENCE}: a page behind a login did not show the subtitles; record `inferred` or `unknown`"));
    }
    if bot
        && matches!(input.status.as_str(), "inferred" | "unknown")
        && input
            .note
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
        && !input.login_required
    {
        return Err(format!(
            "{ERR_READING_EVIDENCE}: say in `note` what could not be checked"
        ));
    }
    if bot && input.kind == "subtitle" && matches!(input.status.as_str(), "confirmed" | "absent") {
        let q = input.quote.as_deref().map(fold).unwrap_or_default();
        if !(q.contains("legend") || q.contains("subtit") || q.contains("closed caption")) {
            return Err(format!(
                "{ERR_READING_EVIDENCE}: the quote must be about subtitles (legendas). Audio in Portuguese is not subtitles: record it with kind=`audio`"
            ));
        }
    }
    let prefs = match &ctx.bot_id {
        Some(b) => get_prefs(db, ctx, b)?,
        None => AccessPrefs::defaults(""),
    };
    let url = if bot && matches!(input.status.as_str(), "confirmed" | "absent") {
        Some(ground(
            db,
            input.fetch_id.as_deref(),
            input.url.as_deref(),
            input.quote.as_deref(),
            now,
            prefs.window_ms(),
            None,
        )?)
    } else {
        input.url.clone()
    };
    if let Some(u) = &url {
        web::check_url(u).map_err(|e| format!("{ERR_READING_EVIDENCE}: {e}"))?;
    }
    let s = Subtitle {
        id: new_id(),
        movie_id: avail.movie_id,
        availability_id: Some(avail.id),
        platform_key: avail.platform_key,
        language: input.language.trim().to_string(),
        kind: input.kind,
        status: input.status,
        url,
        source_kind: input.source_kind,
        quote: input
            .quote
            .map(|q| q.trim().to_string())
            .filter(|q| !q.is_empty()),
        fetch_id: input.fetch_id,
        login_required: input.login_required,
        note: input
            .note
            .map(|q| q.trim().to_string())
            .filter(|q| !q.is_empty()),
        bot_id: ctx.bot_id.clone(),
        checked_ms: now,
    };
    insert_subtitle(db, &s)?;
    Ok(s)
}

// ── Rounds ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ItemInput {
    pub movie_id: String,
    pub role: String,
    pub connection: String,
    pub why: String,
    #[serde(default)]
    pub moods: Vec<String>,
    pub pace: String,
    #[serde(default)]
    pub heavy: bool,
    #[serde(default)]
    pub depends_on_ending: bool,
    /// True when this film was recommended before and comes back on purpose.
    #[serde(default)]
    pub resumes: bool,
    #[serde(default)]
    pub questions: Vec<String>,
    /// Viewing events (with a reaction) this choice leans on; shown in
    /// "Por que esta indicação?".
    #[serde(default)]
    pub evidence_event_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LookForInput {
    pub movie_id: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoundInput {
    pub journey_id: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub requested_count: Option<i64>,
    #[serde(default)]
    pub shortfall_reason: Option<String>,
    /// The user explicitly asked for heavy films this time.
    #[serde(default)]
    pub allow_heavy: bool,
    /// The user asked for confirmed only in this round.
    #[serde(default)]
    pub only_confirmed: Option<bool>,
    #[serde(default)]
    pub skill_name: Option<String>,
    pub items: Vec<ItemInput>,
    #[serde(default)]
    pub look_for: Vec<LookForInput>,
}

pub fn normalise_mood(m: &str) -> Option<&'static str> {
    Some(match fold(m).as_str() {
        "leve" | "light" => "leve",
        "divertido" | "engracado" | "funny" | "fun" => "divertido",
        "romantico" | "romantic" => "romantico",
        "comovente" | "moving" | "touching" => "comovente",
        "intenso" | "intense" => "intenso",
        _ => return None,
    })
}

fn contains_term(text: &str, terms: &[String]) -> Option<String> {
    let f = fold(text);
    terms
        .iter()
        .find(|t| {
            let ft = fold(t);
            ft.chars().count() >= 3 && f.contains(&ft)
        })
        .cloned()
}

/// Hash of the installed skill's `SKILL.md`, computed here, never taken from
/// the model.
pub fn skill_hash(name: &str) -> Option<String> {
    let root = crate::core::skills::install::skills_dir().ok()?;
    let dir = crate::core::skills::install::skill_path(&root, name).ok()?;
    let bytes = std::fs::read(dir.join(crate::core::skills::manifest::SKILL_FILE)).ok()?;
    Some(crate::core::skills::catalog::sha256_hex(&bytes))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedRound {
    pub round: Round,
    pub items: Vec<RoundItem>,
    pub assessments: Vec<Assessment>,
    pub look_for: Value,
}

/// Validates a round against every rule and records it (with one
/// `recommended` event per film) or returns every problem at once.
pub fn record_round(
    db: &AssistDb,
    ctx: &AssistCtx,
    input: RoundInput,
    now: i64,
) -> Result<RecordedRound, String> {
    let j = get_journey(db, ctx, &input.journey_id)?;
    if !can_change(ctx, &j) {
        return Err(format!(
            "{ERR_READING_SCOPE}: this conversation cannot change that journey"
        ));
    }
    let prefs = get_prefs(db, ctx, &j.bot_id)?;
    let only_confirmed = input.only_confirmed.unwrap_or(prefs.only_confirmed);
    let finished = j.status == "finished";
    let progress = latest_progress(db, &j.id)?;
    let previous = rounds_of(db, &j.id)?;
    let events = events_of(db, &j.id)?;
    let mut problems: Vec<String> = Vec::new();

    if progress.is_none() && !finished {
        problems.push("o progresso da leitura não está registrado: pergunte e registre com reading_record_progress antes de indicar".into());
    }
    let n = input.items.len();
    if n == 0 || n > 3 {
        problems.push(format!("uma rodada tem de 1 a 3 filmes (veio {n})"));
    }
    if let Some(req) = input.requested_count {
        if !(1..=3).contains(&req) {
            problems.push("requested_count vai de 1 a 3".into());
        } else if n as i64 > req {
            problems.push(format!("o usuário pediu {req}, a rodada trouxe {n}"));
        }
    }
    let first = previous.is_empty();
    let short = input
        .shortfall_reason
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty();
    let expected = if first {
        input.requested_count.unwrap_or(3)
    } else {
        input.requested_count.unwrap_or(n as i64)
    };
    if (n as i64) < expected && short {
        problems.push(format!(
            "vieram {n} de {expected} filmes: explique em shortfall_reason por que não havia mais opções verificáveis (nunca preencha vaga com certeza inventada)"
        ));
    }
    if first && n == 3 {
        let mut roles: Vec<&str> = input.items.iter().map(|i| i.role.as_str()).collect();
        roles.sort_unstable();
        if roles != ["entry", "shift", "surprise"] {
            problems.push("a primeira rodada com três filmes tem um de cada papel: entrada, deslocamento e surpresa".into());
        }
    }
    if n == 3 && input.items.iter().all(|i| i.heavy) && !input.allow_heavy {
        problems
            .push("três filmes pesados na mesma rodada só se o usuário pedir (allow_heavy)".into());
    }
    let mut seen = std::collections::HashSet::new();
    let text_fields = |i: &ItemInput| -> Vec<String> {
        let mut v = vec![i.connection.clone(), i.why.clone()];
        v.extend(i.questions.iter().cloned());
        v
    };
    if let Some(t) = input
        .reason
        .as_deref()
        .and_then(|r| contains_term(r, &j.spoiler_terms))
    {
        let _ = t;
        problems
            .push("o motivo da rodada contém um termo que o usuário marcou como spoiler".into());
    }
    let mut assessments = Vec::new();
    let mut resumed: Vec<Option<String>> = Vec::new();
    for (idx, it) in input.items.iter().enumerate() {
        let label = match get_movie(db, &it.movie_id) {
            Ok(m) => format!("{} ({})", m.title, m.year),
            Err(e) => {
                problems.push(format!("filme {}: {e}", idx + 1));
                assessments.push(None);
                resumed.push(None);
                continue;
            }
        };
        if !seen.insert(it.movie_id.clone()) {
            problems.push(format!("{label}: repetido na mesma rodada"));
        }
        if !ROLES.contains(&it.role.as_str()) {
            problems.push(format!("{label}: papel deve ser entry, shift ou surprise"));
        }
        if it.connection.trim().is_empty() {
            problems.push(format!("{label}: falta a conexão específica com o livro"));
        }
        if it.why.trim().is_empty() {
            problems.push(format!(
                "{label}: falta a frase \"por que você pode gostar\""
            ));
        }
        let moods: Vec<Option<&str>> = it.moods.iter().map(|m| normalise_mood(m)).collect();
        if moods.is_empty() || moods.iter().any(Option::is_none) {
            problems.push(format!("{label}: clima deve ser um ou mais de {MOODS:?}"));
        }
        if !PACES.contains(&it.pace.as_str()) {
            problems.push(format!("{label}: ritmo deve ser easy ou attentive"));
        }
        let light = !moods.is_empty()
            && moods
                .iter()
                .all(|m| matches!(m, Some("leve") | Some("divertido")));
        let max_q = if light { 1 } else { 2 };
        if it.questions.len() > max_q {
            problems.push(format!(
                "{label}: no máximo {max_q} pergunta(s) opcional(is)"
            ));
        }
        if it.depends_on_ending && !finished {
            problems.push(format!(
                "{label}: a conexão depende do desfecho; fica para depois que o livro for concluído"
            ));
        }
        if text_fields(it)
            .iter()
            .any(|t| contains_term(t, &j.spoiler_terms).is_some())
        {
            problems.push(format!("{label}: a justificativa usa um termo que o usuário marcou como spoiler; reescreva sem ele"));
        }
        // History of this film in the journey.
        let mine: Vec<&ViewingEvent> = events
            .iter()
            .filter(|e| e.movie_id == it.movie_id)
            .collect();
        if mine.iter().any(|e| e.kind == "watched") {
            problems.push(format!("{label}: o usuário já viu este filme"));
        } else if mine.iter().any(|e| e.kind == "abandoned") {
            problems.push(format!("{label}: o usuário abandonou este filme"));
        }
        let prior_item = mine
            .iter()
            .find(|e| e.kind == "recommended")
            .and_then(|e| e.round_item_id.clone());
        if mine.iter().any(|e| e.kind == "recommended") && !it.resumes {
            problems.push(format!(
                "{label}: já foi indicado antes; só volte com resumes=true e diga ao usuário que é uma retomada"
            ));
        }
        resumed.push(if it.resumes { prior_item } else { None });
        for eid in &it.evidence_event_ids {
            match events.iter().find(|e| &e.id == eid) {
                Some(e) if e.reaction.is_some() || !e.reasons.is_empty() => {}
                _ => problems.push(format!(
                    "{label}: a evidência `{eid}` não é uma reação registrada nesta jornada"
                )),
            }
        }
        let a = assess(db, &it.movie_id, &prefs, now)?;
        match a.status {
            FinalStatus::Unconfirmed => problems.push(format!(
                "{label}: acesso não confirmado ({}) — não entra na seleção principal; troque por outro filme e, se quiser, cite-o em look_for (\"vale procurar se aparecer\")",
                if a.reasons.is_empty() { "sem evidência".to_string() } else { a.reasons.join("; ") }
            )),
            FinalStatus::Probable if only_confirmed => problems.push(format!(
                "{label}: só provável (legenda não confirmada) e o usuário pediu somente confirmados"
            )),
            _ => {}
        }
        assessments.push(Some(a));
    }
    let mut look_for = Vec::new();
    for lf in &input.look_for {
        match get_movie(db, &lf.movie_id) {
            Ok(m) => {
                if input.items.iter().any(|i| i.movie_id == m.id) {
                    problems.push(format!(
                        "{}: está na seleção e em look_for ao mesmo tempo",
                        m.title
                    ));
                }
                if lf
                    .note
                    .as_deref()
                    .map(|t| contains_term(t, &j.spoiler_terms).is_some())
                    .unwrap_or(false)
                {
                    problems.push(format!(
                        "{}: a nota usa um termo marcado como spoiler",
                        m.title
                    ));
                }
                let a = assess(db, &m.id, &prefs, now)?;
                look_for.push(json!({
                    "movie_id": m.id, "title": m.title, "year": m.year,
                    "note": lf.note, "status": a.status, "reasons": a.reasons,
                }));
            }
            Err(e) => problems.push(e),
        }
    }
    if !problems.is_empty() {
        return Err(format!(
            "{ERR_READING_ROUND}: rodada recusada, nada foi gravado. Corrija:\n- {}",
            problems.join("\n- ")
        ));
    }

    let context = live_contexts(db, &j.id, now)?;
    let skill_name = input
        .skill_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let round = Round {
        id: new_id(),
        journey_id: j.id.clone(),
        progress_id: progress.as_ref().map(|p| p.id.clone()),
        reason: input
            .reason
            .clone()
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty()),
        context_text: (!context.is_empty()).then(|| {
            context
                .iter()
                .map(|c| c.text.clone())
                .collect::<Vec<_>>()
                .join(" · ")
        }),
        requested_count: input.requested_count,
        shortfall_reason: input
            .shortfall_reason
            .clone()
            .filter(|s| !s.trim().is_empty()),
        only_confirmed,
        skill_hash: skill_name.as_deref().and_then(skill_hash),
        skill_name,
        look_for: Value::Array(look_for.clone()),
        bot_id: ctx.bot_id.clone(),
        created_ms: now,
    };
    let mut items = Vec::new();
    let mut final_assessments = Vec::new();
    for (idx, (it, a)) in input.items.iter().zip(assessments).enumerate() {
        let a = a.expect("validated above");
        let best = a.best.clone().expect("validated: not unconfirmed");
        let mut moods: Vec<String> = Vec::new();
        for m in it.moods.iter().filter_map(|m| normalise_mood(m)) {
            if !moods.iter().any(|x| x == m) {
                moods.push(m.to_string());
            }
        }
        items.push(RoundItem {
            id: new_id(),
            round_id: round.id.clone(),
            movie_id: it.movie_id.clone(),
            position: idx as i64,
            role: it.role.clone(),
            connection: it.connection.trim().into(),
            why: it.why.trim().into(),
            moods,
            pace: it.pace.clone(),
            heavy: it.heavy,
            depends_on_ending: it.depends_on_ending,
            resumes_item_id: resumed[idx].clone(),
            questions: it
                .questions
                .iter()
                .map(|q| q.trim().to_string())
                .filter(|q| !q.is_empty())
                .collect(),
            evidence_event_ids: it.evidence_event_ids.clone(),
            status: best.status.as_str().into(),
            missing: best.notes.clone(),
            availability_id: Some(best.availability.id.clone()),
            subtitle_id: best.subtitle.as_ref().map(|s| s.id.clone()),
        });
        final_assessments.push(a);
    }
    db.tx(|tx| {
        insert_round(tx, &round).map_err(|e| e.to_string())?;
        for i in &items {
            insert_item(tx, i).map_err(|e| e.to_string())?;
            insert_event(
                tx,
                &ViewingEvent {
                    id: new_id(),
                    journey_id: j.id.clone(),
                    movie_id: i.movie_id.clone(),
                    round_item_id: Some(i.id.clone()),
                    kind: "recommended".into(),
                    reaction: None,
                    reasons: Vec::new(),
                    origin: "app".into(),
                    created_ms: now,
                },
            )
            .map_err(|e| e.to_string())?;
        }
        tx.execute(
            "UPDATE reading_journeys SET updated_ms = ?2 WHERE id = ?1",
            params![j.id, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    Ok(RecordedRound {
        round,
        items,
        assessments: final_assessments,
        look_for: Value::Array(look_for),
    })
}

/// Current wall clock, for callers that are not tests.
pub fn now() -> i64 {
    now_ms()
}
