use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{client, fail, uint};
use crate::commands::social::store;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS research_ad_baselines(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    page_id TEXT,
    company TEXT,
    country TEXT NOT NULL,
    status TEXT NOT NULL,
    complete INTEGER NOT NULL,
    created_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS research_ads(
    baseline_id INTEGER NOT NULL,
    archive_id TEXT NOT NULL,
    data TEXT NOT NULL,
    PRIMARY KEY(baseline_id, archive_id)
);
";

const PAGES: usize = 10;
const STATUSES: &[&str] = &["ACTIVE", "ALL", "INACTIVE"];

pub const CAVEATS: &[&str] = &[
    "The Meta Ad Library shows no spend, no conversions and no targeting for commercial ads; nothing here measures results.",
    "An ad that has been running for a long time is not proof that it performs.",
    "An ad that is no longer listed may have ended, been paused or been removed; the library does not say which.",
    "Only public creative text, title, CTA, format, landing URL, status and dates are compared, for the saved country and status filter.",
];

pub const GUIDE: &str = "Write a short weekly review from this evidence only: what is new, what stopped being listed and what changed (text, CTA, format, landing URL), each with its ad archive ID and public URL and how long it has run. Describe ads as active, new or repeated, never as winning or best performing, and never estimate spend, conversions, reach or targeting. End with a section \"What public ads cannot prove\" that restates the caveats, and if `complete` is false say the fetch was cut so the lists may be partial.";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Ad {
    pub archive_id: String,
    pub url: String,
    pub page_id: Option<String>,
    pub page_name: Option<String>,
    pub active: Option<bool>,
    pub started: Option<i64>,
    pub ended: Option<i64>,
    pub text: Option<String>,
    pub title: Option<String>,
    pub cta_text: Option<String>,
    pub cta_type: Option<String>,
    pub format: Option<String>,
    pub landing_url: Option<String>,
    pub platforms: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Meta {
    pub name: String,
    pub page_id: Option<String>,
    pub company: Option<String>,
    pub country: String,
    pub status: String,
    pub complete: bool,
    pub created_ms: i64,
    pub updated_ms: i64,
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn id_text(value: &Value) -> Option<String> {
    text(value).or_else(|| value.as_u64().map(|n| n.to_string()))
}

pub fn ad(item: &Value) -> Option<Ad> {
    let archive_id = id_text(&item["ad_archive_id"])?;
    let snapshot = &item["snapshot"];
    Some(Ad {
        url: text(&item["url"])
            .unwrap_or_else(|| format!("https://www.facebook.com/ads/library?id={archive_id}")),
        archive_id,
        page_id: id_text(&item["page_id"]),
        page_name: text(&item["page_name"]).or_else(|| text(&snapshot["page_name"])),
        active: item["is_active"].as_bool(),
        started: item["start_date"].as_i64().filter(|t| *t > 0),
        ended: item["end_date"].as_i64().filter(|t| *t > 0),
        text: text(&snapshot["body"]["text"]).or_else(|| text(&snapshot["body"])),
        title: text(&snapshot["title"]),
        cta_text: text(&snapshot["cta_text"]),
        cta_type: text(&snapshot["cta_type"]),
        format: text(&snapshot["display_format"]),
        landing_url: text(&snapshot["link_url"]),
        platforms: item["publisher_platform"]
            .as_array()
            .map(|list| list.iter().filter_map(text).collect())
            .unwrap_or_default(),
    })
}

pub fn ads(body: &Value) -> Result<Vec<Ad>, String> {
    match body.get("results") {
        Some(Value::Array(items)) => Ok(items.iter().filter_map(ad).collect()),
        Some(Value::Null) => Ok(Vec::new()),
        _ => Err(fail(
            "UPSTREAM_CHANGED",
            "the Ad Library answer has no results list",
            false,
            "call facebook_adlibrary_company_ads with research_call to see the new shape and report it",
            Value::Null,
        )),
    }
}

pub fn running_days(ad: &Ad, now_secs: i64) -> Option<i64> {
    let started = ad.started?;
    let until = match (ad.active, ad.ended) {
        (Some(false), Some(ended)) => ended,
        _ => now_secs,
    };
    Some(((until - started).max(0)) / 86_400)
}

pub fn view(ad: &Ad, now_secs: i64) -> Value {
    json!({
        "archive_id": ad.archive_id,
        "url": ad.url,
        "page_name": ad.page_name,
        "active": ad.active,
        "started": ad.started.and_then(super::tiktok::iso),
        "ended": ad.ended.and_then(super::tiktok::iso),
        "running_days": running_days(ad, now_secs),
        "format": ad.format,
        "cta_text": ad.cta_text,
        "cta_type": ad.cta_type,
        "title": ad.title,
        "text": ad.text,
        "landing_url": ad.landing_url,
        "platforms": ad.platforms,
    })
}

fn changes(before: &Ad, after: &Ad) -> Vec<Value> {
    let fields: [(&str, &Option<String>, &Option<String>); 6] = [
        ("text", &before.text, &after.text),
        ("title", &before.title, &after.title),
        ("cta_text", &before.cta_text, &after.cta_text),
        ("cta_type", &before.cta_type, &after.cta_type),
        ("format", &before.format, &after.format),
        ("landing_url", &before.landing_url, &after.landing_url),
    ];
    let mut out: Vec<Value> = fields
        .iter()
        .filter(|(_, a, b)| a != b)
        .map(|(field, a, b)| json!({"field": field, "before": a, "after": b}))
        .collect();
    if before.active != after.active {
        out.push(json!({"field": "active", "before": before.active, "after": after.active}));
    }
    out
}

fn find<'a>(list: &'a [Ad], id: &str) -> Option<&'a Ad> {
    list.iter().find(|a| a.archive_id == id)
}

pub fn diff(before: &[Ad], after: &[Ad], now_secs: i64, complete: bool, limit: usize) -> Value {
    let new_ads: Vec<&Ad> = after
        .iter()
        .filter(|a| find(before, &a.archive_id).is_none())
        .collect();
    let missing: Vec<&Ad> = before
        .iter()
        .filter(|a| find(after, &a.archive_id).is_none())
        .collect();
    let mut changed: Vec<Value> = Vec::new();
    let mut unchanged = 0usize;
    for now in after {
        let Some(then) = find(before, &now.archive_id) else {
            continue;
        };
        let found = changes(then, now);
        if found.is_empty() {
            unchanged += 1;
        } else {
            changed.push(json!({
                "archive_id": now.archive_id,
                "url": now.url,
                "running_days": running_days(now, now_secs),
                "changes": found,
            }));
        }
    }
    let shown = |list: &[&Ad]| -> Vec<Value> {
        list.iter().take(limit).map(|a| view(a, now_secs)).collect()
    };
    let (gone, not_seen) = if complete {
        (missing, Vec::new())
    } else {
        (Vec::new(), missing)
    };
    let totals = json!({
        "new_ads": new_ads.len(),
        "no_longer_listed": gone.len(),
        "not_seen_in_partial_fetch": not_seen.len(),
        "changed": changed.len(),
        "unchanged": unchanged,
    });
    let truncated = [new_ads.len(), gone.len(), not_seen.len(), changed.len()]
        .iter()
        .any(|n| *n > limit);
    changed.truncate(limit);
    json!({
        "new_ads": shown(&new_ads),
        "no_longer_listed": shown(&gone),
        "not_seen_in_partial_fetch": shown(&not_seen),
        "changed": changed,
        "totals": totals,
        "truncated": truncated,
    })
}

pub fn save_baseline(conn: &mut Connection, meta: &Meta, list: &[Ad]) -> rusqlite::Result<bool> {
    let tx = conn.transaction()?;
    let existing: Option<i64> = tx
        .query_row(
            "SELECT id FROM research_ad_baselines WHERE name = ?1 COLLATE NOCASE",
            params![meta.name],
            |row| row.get(0),
        )
        .optional()?;
    let id = match existing {
        Some(id) => {
            tx.execute(
                "UPDATE research_ad_baselines SET page_id = ?2, company = ?3, country = ?4, status = ?5, complete = ?6, updated_ms = ?7 WHERE id = ?1",
                params![id, meta.page_id, meta.company, meta.country, meta.status, meta.complete, meta.updated_ms],
            )?;
            tx.execute(
                "DELETE FROM research_ads WHERE baseline_id = ?1",
                params![id],
            )?;
            id
        }
        None => {
            tx.execute(
                "INSERT INTO research_ad_baselines(name, page_id, company, country, status, complete, created_ms, updated_ms) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![meta.name, meta.page_id, meta.company, meta.country, meta.status, meta.complete, meta.created_ms, meta.updated_ms],
            )?;
            tx.last_insert_rowid()
        }
    };
    for ad in list {
        let data = serde_json::to_string(ad).unwrap_or_default();
        tx.execute(
            "INSERT OR REPLACE INTO research_ads(baseline_id, archive_id, data) VALUES(?1, ?2, ?3)",
            params![id, ad.archive_id, data],
        )?;
    }
    tx.commit()?;
    Ok(existing.is_some())
}

pub fn load_baseline(conn: &Connection, name: &str) -> rusqlite::Result<Option<(Meta, Vec<Ad>)>> {
    let found = conn
        .query_row(
            "SELECT id, name, page_id, company, country, status, complete, created_ms, updated_ms FROM research_ad_baselines WHERE name = ?1 COLLATE NOCASE",
            params![name],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    Meta {
                        name: row.get(1)?,
                        page_id: row.get(2)?,
                        company: row.get(3)?,
                        country: row.get(4)?,
                        status: row.get(5)?,
                        complete: row.get(6)?,
                        created_ms: row.get(7)?,
                        updated_ms: row.get(8)?,
                    },
                ))
            },
        )
        .optional()?;
    let Some((id, meta)) = found else {
        return Ok(None);
    };
    let mut statement =
        conn.prepare("SELECT data FROM research_ads WHERE baseline_id = ?1 ORDER BY archive_id")?;
    let list = statement
        .query_map(params![id], |row| row.get::<_, String>(0))?
        .filter_map(Result::ok)
        .filter_map(|data| serde_json::from_str::<Ad>(&data).ok())
        .collect();
    Ok(Some((meta, list)))
}

pub fn list_baselines(conn: &Connection) -> rusqlite::Result<Vec<Value>> {
    let mut statement = conn.prepare(
        "SELECT b.name, b.page_id, b.company, b.country, b.status, b.complete, b.created_ms, b.updated_ms, (SELECT COUNT(*) FROM research_ads a WHERE a.baseline_id = b.id) FROM research_ad_baselines b ORDER BY b.updated_ms DESC",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(json!({
                "name": row.get::<_, String>(0)?,
                "page_id": row.get::<_, Option<String>>(1)?,
                "company": row.get::<_, Option<String>>(2)?,
                "country": row.get::<_, String>(3)?,
                "status": row.get::<_, String>(4)?,
                "complete": row.get::<_, bool>(5)?,
                "created_ms": row.get::<_, i64>(6)?,
                "updated_ms": row.get::<_, i64>(7)?,
                "ads": row.get::<_, i64>(8)?,
            }))
        })?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

fn meta_view(meta: &Meta, count: usize) -> Value {
    json!({
        "name": meta.name,
        "page_id": meta.page_id,
        "company": meta.company,
        "country": meta.country,
        "status": meta.status,
        "complete": meta.complete,
        "created_ms": meta.created_ms,
        "updated_ms": meta.updated_ms,
        "ads": count,
    })
}

async fn db<T: Send + 'static>(
    work: impl FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(move || store::with(SCHEMA, work))
        .await
        .map_err(|e| {
            fail(
                "STORE_FAILED",
                &e.to_string(),
                true,
                "try again",
                Value::Null,
            )
        })?
        .map_err(|e| fail("STORE_FAILED", &e, true, "try again", Value::Null))
}

async fn fetch(meta: &Meta, max_ads: usize) -> Result<(Vec<Ad>, bool), String> {
    let mut all: Vec<Ad> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..PAGES {
        let body = client::get(
            "facebook_adlibrary_company_ads",
            json!({
                "pageId": meta.page_id,
                "companyName": meta.company,
                "country": meta.country,
                "status": meta.status,
                "cursor": cursor,
            }),
        )
        .await?;
        let page = ads(&body)?;
        let empty = page.is_empty();
        for ad in page {
            if !all.iter().any(|a| a.archive_id == ad.archive_id) {
                all.push(ad);
            }
        }
        cursor = text(&body["cursor"]);
        if cursor.is_none() || empty {
            return Ok((all, true));
        }
        if all.len() >= max_ads {
            break;
        }
    }
    all.truncate(max_ads);
    Ok((all, false))
}

fn now_secs() -> i64 {
    store::now_ms() / 1000
}

fn limit_of(a: &Value) -> usize {
    a["limit"].as_u64().unwrap_or(30).clamp(1, 200) as usize
}

fn unknown_baseline(name: &str) -> String {
    fail(
        "UNKNOWN_BASELINE",
        &format!("no baseline is named \"{name}\""),
        false,
        "call research_meta_ads with op=list to see the saved names, or op=baseline to save one",
        Value::Null,
    )
}

fn with_notes(mut out: Value) -> Value {
    out["caveats"] = json!(CAVEATS);
    out["guide"] = json!(GUIDE);
    out["untrusted"] = json!(true);
    out
}

pub async fn call(a: &Value) -> Result<Value, String> {
    let before = client::credits_spent();
    match a["op"].as_str().unwrap_or_default() {
        "find" => {
            let query = text(&a["query"]).ok_or_else(|| {
                fail(
                    "INVALID_PARAMS",
                    "find needs query",
                    false,
                    "pass the company or brand name as query",
                    Value::Null,
                )
            })?;
            let body = client::get(
                "facebook_adlibrary_search_companies",
                json!({ "query": query }),
            )
            .await?;
            let pages: Vec<Value> = body["searchResults"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .map(|p| {
                            json!({
                                "page_id": id_text(&p["page_id"]),
                                "name": p["name"],
                                "category": p["category"],
                                "likes": uint(&p["likes"]),
                                "verification": p["verification"],
                                "ig_username": p["ig_username"],
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mut out = json!({
                "pages": pages,
                "total": pages.len(),
                "returned": pages.len(),
                "truncated": false,
                "credits_used": client::credits_spent() - before,
                "untrusted": true,
            });
            if pages.is_empty() {
                out["reason"] = json!("no_page_matched");
                out["next_step"] =
                    json!("try the brand's exact page name, or pass company to op=baseline");
            }
            Ok(out)
        }
        "baseline" => {
            let page_id = text(&a["page_id"]);
            let company = text(&a["company"]);
            if page_id.is_none() && company.is_none() {
                return Err(fail(
                    "INVALID_PARAMS",
                    "baseline needs page_id or company",
                    false,
                    "call op=find with the brand name to get its page_id, then op=baseline",
                    Value::Null,
                ));
            }
            let status = text(&a["status"])
                .unwrap_or_else(|| "ACTIVE".into())
                .to_uppercase();
            if !STATUSES.contains(&status.as_str()) {
                return Err(fail(
                    "INVALID_PARAMS",
                    "status must be ACTIVE, ALL or INACTIVE",
                    false,
                    "use ACTIVE to monitor what is running now",
                    json!({ "allowed": STATUSES }),
                ));
            }
            let now = store::now_ms();
            let mut meta = Meta {
                name: text(&a["name"])
                    .or_else(|| company.clone())
                    .or_else(|| page_id.clone())
                    .unwrap_or_default(),
                page_id,
                company,
                country: text(&a["country"])
                    .unwrap_or_else(|| "ALL".into())
                    .to_uppercase(),
                status,
                complete: false,
                created_ms: now,
                updated_ms: now,
            };
            let max_ads = a["max_ads"].as_u64().unwrap_or(100).clamp(1, 300) as usize;
            let (list, complete) = fetch(&meta, max_ads).await?;
            meta.complete = complete;
            let (saved_meta, saved_list) = (meta.clone(), list.clone());
            let replaced = db(move |conn| save_baseline(conn, &saved_meta, &saved_list)).await?;
            let limit = limit_of(a);
            let secs = now_secs();
            let mut out = with_notes(json!({
                "baseline": meta_view(&meta, list.len()),
                "replaced": replaced,
                "ads": list.iter().take(limit).map(|ad| view(ad, secs)).collect::<Vec<_>>(),
                "total": list.len(),
                "returned": list.len().min(limit),
                "truncated": list.len() > limit,
                "complete": complete,
                "credits_used": client::credits_spent() - before,
            }));
            if list.is_empty() {
                out["reason"] = json!("no_ads_listed");
                out["next_step"] =
                    json!("check the page_id with op=find, or try status=ALL or another country");
            }
            Ok(out)
        }
        "compare" => {
            let name = text(&a["name"]).ok_or_else(|| {
                fail(
                    "INVALID_PARAMS",
                    "compare needs name",
                    false,
                    "call op=list to see the saved baselines",
                    Value::Null,
                )
            })?;
            let wanted = name.clone();
            let (mut meta, saved) = db(move |conn| load_baseline(conn, &wanted))
                .await?
                .ok_or_else(|| unknown_baseline(&name))?;
            let max_ads = a["max_ads"]
                .as_u64()
                .unwrap_or(saved.len().max(100) as u64)
                .clamp(1, 300) as usize;
            let (list, complete) = fetch(&meta, max_ads).await?;
            let mut out = diff(&saved, &list, now_secs(), complete, limit_of(a));
            out["baseline"] = meta_view(&meta, saved.len());
            out["fetched"] = json!(list.len());
            out["complete"] = json!(complete);
            let update = a["update"].as_bool() == Some(true);
            if update {
                meta.complete = complete;
                meta.updated_ms = store::now_ms();
                db(move |conn| save_baseline(conn, &meta, &list)).await?;
            }
            out["baseline_updated"] = json!(update);
            out["credits_used"] = json!(client::credits_spent() - before);
            Ok(with_notes(out))
        }
        "list" => {
            let rows = db(|conn| list_baselines(conn)).await?;
            let mut out = json!({
                "baselines": rows,
                "total": rows.len(),
                "returned": rows.len(),
                "truncated": false,
            });
            if rows.is_empty() {
                out["reason"] = json!("no_baseline_saved");
                out["next_step"] =
                    json!("call op=baseline with page_id or company to save the first one");
            }
            Ok(out)
        }
        other => Err(fail(
            "INVALID_PARAMS",
            &format!("unknown op \"{other}\" for research_meta_ads"),
            false,
            "use op=find, baseline, compare or list",
            Value::Null,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(id: &str, body: &str, cta: &str) -> Value {
        json!({
            "ad_archive_id": id,
            "end_date": 1_750_230_000,
            "is_active": true,
            "page_id": "15087023444",
            "page_name": "Nike",
            "snapshot": {
                "body": {"text": body},
                "cta_text": cta,
                "cta_type": "LEARN_MORE",
                "display_format": "VIDEO",
                "link_url": "https://fb.com/canvas_doc/569658969305127",
                "title": "Look de invierno activado",
            },
            "spend": null,
            "start_date": 1_750_143_600,
            "publisher_platform": ["FACEBOOK", "INSTAGRAM"],
            "url": format!("https://www.facebook.com/ads/library?id={id}"),
        })
    }

    fn parsed(id: &str, body: &str, cta: &str) -> Ad {
        ad(&raw(id, body, cta)).unwrap()
    }

    fn meta(name: &str) -> Meta {
        Meta {
            name: name.into(),
            page_id: Some("15087023444".into()),
            company: None,
            country: "ALL".into(),
            status: "ACTIVE".into(),
            complete: true,
            created_ms: 1,
            updated_ms: 1,
        }
    }

    #[test]
    fn an_ad_keeps_its_archive_id_public_url_and_public_creative_fields() {
        let found = parsed(
            "1212560900568826",
            "Que el frío no te detenga.",
            "Learn more",
        );
        assert_eq!(found.archive_id, "1212560900568826");
        assert_eq!(
            found.url,
            "https://www.facebook.com/ads/library?id=1212560900568826"
        );
        assert_eq!(found.text.as_deref(), Some("Que el frío no te detenga."));
        assert_eq!(found.cta_text.as_deref(), Some("Learn more"));
        assert_eq!(found.format.as_deref(), Some("VIDEO"));
        assert_eq!(
            found.landing_url.as_deref(),
            Some("https://fb.com/canvas_doc/569658969305127")
        );
        assert_eq!(found.platforms, vec!["FACEBOOK", "INSTAGRAM"]);
        assert_eq!(found.active, Some(true));
        let bare = ad(&json!({"ad_archive_id": 77, "snapshot": {"body": "plain"}})).unwrap();
        assert_eq!(bare.url, "https://www.facebook.com/ads/library?id=77");
        assert_eq!(bare.text.as_deref(), Some("plain"));
        assert_eq!(ad(&json!({"snapshot": {}})), None);
    }

    #[test]
    fn the_results_list_is_required_and_an_unknown_shape_is_reported() {
        let body = json!({"results": [raw("1", "a", "b"), {"x": 1}], "cursor": "next"});
        assert_eq!(ads(&body).unwrap().len(), 1);
        assert!(ads(&json!({"results": null})).unwrap().is_empty());
        assert!(ads(&json!({"ads": []}))
            .unwrap_err()
            .starts_with("UPSTREAM_CHANGED"));
    }

    #[test]
    fn running_time_counts_to_now_while_active_and_to_the_end_date_when_stopped() {
        let mut found = parsed("1", "a", "b");
        assert_eq!(
            running_days(&found, 1_750_143_600 + 10 * 86_400 + 5),
            Some(10)
        );
        found.active = Some(false);
        assert_eq!(running_days(&found, 1_750_143_600 + 10 * 86_400), Some(1));
        found.started = None;
        assert_eq!(running_days(&found, 1), None);
    }

    #[test]
    fn the_diff_finds_new_gone_and_changed_ads_with_ids_and_urls() {
        let before = vec![
            parsed("1", "old text", "Learn more"),
            parsed("2", "same", "Shop now"),
            parsed("3", "gone", "Shop now"),
        ];
        let mut edited = parsed("1", "new text", "Sign up");
        edited.active = Some(false);
        let after = vec![
            edited,
            parsed("2", "same", "Shop now"),
            parsed("4", "fresh", "Shop now"),
        ];
        let out = diff(&before, &after, 1_750_143_600 + 86_400 * 3, true, 30);
        assert_eq!(
            out["totals"],
            json!({"new_ads": 1, "no_longer_listed": 1, "not_seen_in_partial_fetch": 0, "changed": 1, "unchanged": 1})
        );
        assert_eq!(out["new_ads"][0]["archive_id"], "4");
        assert_eq!(
            out["new_ads"][0]["url"],
            "https://www.facebook.com/ads/library?id=4"
        );
        assert_eq!(out["new_ads"][0]["running_days"], 3);
        assert_eq!(out["no_longer_listed"][0]["archive_id"], "3");
        let changed = &out["changed"][0];
        assert_eq!(changed["archive_id"], "1");
        assert_eq!(changed["url"], "https://www.facebook.com/ads/library?id=1");
        assert_eq!(
            changed["changes"],
            json!([
                {"field": "text", "before": "old text", "after": "new text"},
                {"field": "cta_text", "before": "Learn more", "after": "Sign up"},
                {"field": "active", "before": true, "after": false},
            ])
        );
        assert_eq!(out["truncated"], false);
    }

    #[test]
    fn a_partial_fetch_never_declares_an_ad_gone_and_long_lists_are_capped() {
        let before = vec![parsed("1", "a", "b"), parsed("2", "a", "b")];
        let after = vec![parsed("1", "a", "b")];
        let out = diff(&before, &after, 0, false, 30);
        assert_eq!(out["totals"]["no_longer_listed"], 0);
        assert_eq!(out["totals"]["not_seen_in_partial_fetch"], 1);
        assert_eq!(out["not_seen_in_partial_fetch"][0]["archive_id"], "2");
        let many: Vec<Ad> = (0..5).map(|n| parsed(&n.to_string(), "a", "b")).collect();
        let capped = diff(&[], &many, 0, true, 2);
        assert_eq!(capped["new_ads"].as_array().unwrap().len(), 2);
        assert_eq!(capped["totals"]["new_ads"], 5);
        assert_eq!(capped["truncated"], true);
    }

    #[test]
    fn a_baseline_is_stored_listed_loaded_and_replaced_by_name() {
        let mut conn = store::memory(SCHEMA);
        let first = vec![parsed("1", "a", "b"), parsed("2", "c", "d")];
        assert!(!save_baseline(&mut conn, &meta("Nike"), &first).unwrap());
        let (loaded, list) = load_baseline(&conn, "nike").unwrap().unwrap();
        assert_eq!(loaded, meta("Nike"));
        assert_eq!(list, first);
        let mut later = meta("Nike");
        later.updated_ms = 9;
        later.created_ms = 9;
        later.complete = false;
        assert!(save_baseline(&mut conn, &later, &first[..1]).unwrap());
        let (loaded, list) = load_baseline(&conn, "Nike").unwrap().unwrap();
        assert_eq!(
            (loaded.created_ms, loaded.updated_ms, loaded.complete),
            (1, 9, false)
        );
        assert_eq!(list.len(), 1);
        save_baseline(&mut conn, &meta("Adidas"), &[]).unwrap();
        let listed = list_baselines(&conn).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0]["name"], "Nike");
        assert_eq!(listed[0]["ads"], 1);
        assert_eq!(listed[1]["ads"], 0);
        assert_eq!(load_baseline(&conn, "Puma").unwrap(), None);
    }

    #[test]
    fn every_result_carries_the_fixed_caveats() {
        let out = with_notes(json!({}));
        let caveats = out["caveats"].to_string();
        for word in ["spend", "conversions", "targeting", "not proof"] {
            assert!(caveats.contains(word), "{word}");
        }
        assert!(out["guide"].as_str().unwrap().contains("archive ID"));
        assert_eq!(out["untrusted"], true);
    }

    #[tokio::test]
    async fn unknown_ops_and_missing_arguments_are_refused_before_any_request() {
        assert!(call(&json!({"op": "delete"}))
            .await
            .unwrap_err()
            .starts_with("INVALID_PARAMS"));
        assert!(call(&json!({"op": "baseline"}))
            .await
            .unwrap_err()
            .starts_with("INVALID_PARAMS"));
        assert!(
            call(&json!({"op": "baseline", "company": "x", "status": "paused"}))
                .await
                .unwrap_err()
                .starts_with("INVALID_PARAMS")
        );
        assert!(call(&json!({"op": "find"}))
            .await
            .unwrap_err()
            .starts_with("INVALID_PARAMS"));
        assert!(call(&json!({"op": "compare"}))
            .await
            .unwrap_err()
            .starts_with("INVALID_PARAMS"));
    }
}
