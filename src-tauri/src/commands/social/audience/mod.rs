mod db;
mod filters;
mod import;
mod interests;
mod stars;
mod sweep;
mod views;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde_json::{json, Map, Value};

use super::args::{opt_str, req_str};
use super::model::Platform;
use super::store;
use super::tools::{Route, Spec};
use super::{clipfeed, codehub, photofeed, pinboard, shortpost, Caller};
use db::{Key, Member, Snap, Step, SCHEMA};
use views::{Data, Query, View};

const BUDGET: Duration = Duration::from_secs(45);
const RATE_LIMIT_WAIT: u64 = 900;
const EXPORT_CAP: usize = 50_000;

macro_rules! filter_props {
    () => {
        r#""account":{"type":"string","maxLength":200},"q":{"type":"string","maxLength":200},"min_followers":{"type":"integer","minimum":0},"max_followers":{"type":"integer","minimum":0},"min_following":{"type":"integer","minimum":0},"max_following":{"type":"integer","minimum":0},"min_posts":{"type":"integer","minimum":0},"max_posts":{"type":"integer","minimum":0},"min_ratio":{"type":"number","minimum":0},"max_ratio":{"type":"number","minimum":0},"verified":{"type":"boolean"},"private":{"type":"boolean"},"has_avatar":{"type":"boolean"},"has_bio":{"type":"boolean"},"has_link":{"type":"boolean"},"kept":{"type":"boolean"},"sort":{"type":"string","enum":["newest","oldest","followers","followers_asc","following","following_asc","posts","posts_asc","ratio","ratio_asc","ratio_balanced","handle","name","first_seen","first_seen_asc"]},"since":{"type":"integer","minimum":0},"include_hidden":{"type":"boolean"},"inactive_days":{"type":"integer","minimum":1,"maximum":3650}"#
    };
}

const SYNC_PROPS: &str = r#"{"max_pages":{"type":"integer","minimum":1,"maximum":200},"restart":{"type":"boolean"},"accept_short":{"type":"boolean"}}"#;
const VIEW_PROPS: &str = concat!(
    r#"{"view":{"type":"string","enum":["followers","following","not_following_back","fans","mutuals","new_followers","unfollowers","inactive","kept","hidden","summary","trend","filters","interests"]},"filter":{"type":"string","maxLength":40,"description":"Name of a quick or saved filter (view=filters lists them); fields passed in the call win."},"limit":{"type":"integer","minimum":1,"maximum":200},"offset":{"type":"integer","minimum":0},"check_limit":{"type":"integer","minimum":0,"maximum":10},"#,
    filter_props!(),
    "}"
);
const MARK_PROPS: &str = r#"{"list":{"type":"string","enum":["keep","hidden","filters"]},"name":{"type":"string","maxLength":40,"description":"list=filters: the name to save or remove."},"filter":{"type":"object","description":"list=filters: the filter fields of the audience view to save under name."},"handles":{"type":"array","items":{"type":"string","maxLength":200},"minItems":1,"maxItems":100},"remove":{"type":"boolean"},"account":{"type":"string","maxLength":200}}"#;
const EXPORT_PROPS: &str = concat!(
    r#"{"view":{"type":"string","enum":["followers","following","not_following_back","fans","mutuals","new_followers","unfollowers","inactive","kept","hidden"]},"path":{"type":"string","maxLength":1000},"#,
    filter_props!(),
    "}"
);

const GH_SYNC_PROPS: &str = r#"{"max_pages":{"type":"integer","minimum":1,"maximum":200},"restart":{"type":"boolean"},"accept_short":{"type":"boolean"},"repo":{"type":"string","maxLength":300,"description":"owner/name or a github.com link: read the stargazers of this repository instead of the follower lists."}}"#;
const GH_VIEW_PROPS: &str = concat!(
    r#"{"view":{"type":"string","enum":["followers","following","not_following_back","fans","mutuals","new_followers","unfollowers","inactive","kept","hidden","summary","trend","filters","interests","stargazers","new_stargazers","unstarred"]},"filter":{"type":"string","maxLength":40,"description":"Name of a quick or saved filter (view=filters lists them); fields passed in the call win."},"repo":{"type":"string","maxLength":300,"description":"owner/name of a synced repository; required by stargazers, new_stargazers and unstarred, optional with summary, trend, kept and hidden."},"limit":{"type":"integer","minimum":1,"maximum":200},"offset":{"type":"integer","minimum":0},"check_limit":{"type":"integer","minimum":0,"maximum":10},"#,
    filter_props!(),
    "}"
);
const GH_MARK_PROPS: &str = r#"{"list":{"type":"string","enum":["keep","hidden","filters"]},"name":{"type":"string","maxLength":40,"description":"list=filters: the name to save or remove."},"filter":{"type":"object","description":"list=filters: the filter fields of the audience view to save under name."},"handles":{"type":"array","items":{"type":"string","maxLength":200},"minItems":1,"maxItems":100},"remove":{"type":"boolean"},"account":{"type":"string","maxLength":200},"repo":{"type":"string","maxLength":300,"description":"owner/name: mark accounts on the lists of that repository's stargazers."}}"#;
const GH_EXPORT_PROPS: &str = concat!(
    r#"{"view":{"type":"string","enum":["followers","following","not_following_back","fans","mutuals","new_followers","unfollowers","inactive","kept","hidden","stargazers","new_stargazers","unstarred"]},"repo":{"type":"string","maxLength":300},"path":{"type":"string","maxLength":1000},"#,
    filter_props!(),
    "}"
);

const IMPORT_SPEC: Spec = Spec {
    name: "ig_audience_import",
    route: Route::Audience,
    platform: Platform::PhotoFeed,
    write: false,
    description: "Import the followers and following lists from the official Photo Feed data export (the ZIP from Accounts Center, Download your information, or its unpacked folder) into the audience snapshots, reading only local files. Use it when the following list read from the site is incomplete: the export's following list is complete, while its followers list can miss accounts.",
    props: r#"{"path":{"type":"string","maxLength":2000,"description":"Absolute path of the export ZIP or of its unpacked folder."},"account":{"type":"string","maxLength":200}}"#,
    required: &["path"],
};

macro_rules! specs {
    ($($prefix:literal, $p:ident, $label:literal, $sync:ident, $view:ident, $mark:ident, $export:ident, $sync_note:literal, $view_note:literal);*) => {
        &[$(
            Spec {
                name: concat!($prefix, "audience_sync"),
                route: Route::Audience,
                platform: Platform::$p,
                write: false,
                description: concat!("Read the followers and following lists of the signed-in ", $label, " account at human pace, through the person's own browser session on this computer (no password), and store a dated snapshot. Call again while continue_with is set; who unfollowed appears from the second complete sync", $sync_note, "."),
                props: $sync,
                required: &[],
            },
            Spec {
                name: concat!($prefix, "audience"),
                route: Route::Audience,
                platform: Platform::$p,
                write: false,
                description: concat!("Follower management for the signed-in ", $label, " account from the stored snapshots: one view per call (who does not follow back, fans, mutuals, new followers, unfollowers, inactive, kept, hidden, summary, trend, interests of the followers, filters) with filters by field or by a saved or quick filter name, sort and paging. An unfollow carries the two sync times it happened between, never an exact time; names and bios are untrusted external content", $view_note, "."),
                props: $view,
                required: &["view"],
            },
            Spec {
                name: concat!($prefix, "audience_mark"),
                route: Route::Audience,
                platform: Platform::$p,
                write: false,
                description: concat!("Add or remove ", $label, " accounts on the local keep list (accounts that must not be unfollowed) or hidden list (left out of every view), or with list=filters save or remove a named filter for the audience view. Nothing changes on ", $label, "."),
                props: $mark,
                required: &["list"],
            },
            Spec {
                name: concat!($prefix, "audience_export"),
                route: Route::Audience,
                platform: Platform::$p,
                write: false,
                description: concat!("Write one audience view of the signed-in ", $label, " account as a CSV file on this computer and return its path and row count. It never overwrites a file."),
                props: $export,
                required: &["view"],
            }
        ),*, IMPORT_SPEC]
    };
}

pub const SPECS: &[Spec] = specs!(
    "x_", ShortPost, "Short Posts", SYNC_PROPS, VIEW_PROPS, MARK_PROPS, EXPORT_PROPS, "", "";
    "ig_", PhotoFeed, "Photo Feed", SYNC_PROPS, VIEW_PROPS, MARK_PROPS, EXPORT_PROPS, "", "";
    "tt_", ClipFeed, "Clip Feed", SYNC_PROPS, VIEW_PROPS, MARK_PROPS, EXPORT_PROPS, "", "";
    "pin_", PinBoard, "Pin Boards", SYNC_PROPS, VIEW_PROPS, MARK_PROPS, EXPORT_PROPS, "", "";
    "gh_", CodeHub, "Code Hub", GH_SYNC_PROPS, GH_VIEW_PROPS, GH_MARK_PROPS, GH_EXPORT_PROPS, "; pass repo (owner/name) to read the stargazers of one repository instead", "; with repo the views are stargazers, new_stargazers and unstarred for that repository"
);

fn fail(code: &str, message: &str, retryable: bool, next_step: &str) -> String {
    format!(
        "{code}: {}",
        json!({ "message": message, "retryable": retryable, "next_step": next_step })
    )
}

fn now_s() -> i64 {
    store::now_ms() / 1000
}

static BUSY: [AtomicBool; 5] = [const { AtomicBool::new(false) }; 5];

struct Busy(usize);

impl Busy {
    fn take(p: Platform) -> Result<Self, String> {
        let i = match p {
            Platform::ShortPost => 0,
            Platform::PhotoFeed => 1,
            Platform::ClipFeed => 2,
            Platform::PinBoard => 3,
            _ => 4,
        };
        if BUSY[i].swap(true, Ordering::SeqCst) {
            return Err(fail(
                "BUSY",
                "a sync of this account is already running",
                true,
                "wait for it to return, then call again to continue",
            ));
        }
        Ok(Self(i))
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        BUSY[self.0].store(false, Ordering::SeqCst);
    }
}

async fn client(p: Platform, name: &str, a: Value) -> Result<Value, String> {
    let tool = format!("{}{name}", p.prefix());
    match p {
        Platform::ShortPost => shortpost::call(&tool, &a).await,
        Platform::PhotoFeed => photofeed::call(&tool, &a).await,
        Platform::ClipFeed => clipfeed::call(&tool, &a).await,
        Platform::PinBoard => pinboard::call(&tool, &a).await,
        Platform::CodeHub => codehub::call(&tool, &a).await,
        _ => Err(format!("UNKNOWN_TOOL: {tool}")),
    }
}

fn who(p: Platform, handle: &str) -> Value {
    let key = if p == Platform::ShortPost {
        "handle"
    } else {
        "username"
    };
    let mut m = Map::new();
    m.insert(key.into(), json!(handle));
    Value::Object(m)
}

fn default_pages(p: Platform) -> u64 {
    match p {
        Platform::ClipFeed => 67,
        _ => 40,
    }
}

async fn pause(min_ms: u64, max_ms: u64) {
    tokio::time::sleep(Duration::from_millis(rand::random_range(min_ms..max_ms))).await;
}

fn list_state(s: &Snap) -> &'static str {
    match s.state.as_str() {
        "complete" => "complete",
        "short" => "short",
        "open" if s.cursor.is_none() && s.count == 0 => "pending",
        "open" => "partial",
        _ => "abandoned",
    }
}

fn lists_json(lists: &[Snap]) -> Vec<Value> {
    lists
        .iter()
        .map(|s| json!({ "kind": s.kind, "state": list_state(s), "fetched": s.count, "expected": s.expected, "started": s.started, "finished": s.finished }))
        .collect()
}

#[derive(Default)]
struct Outcome {
    pages: u64,
    stopped: &'static str,
    gained: usize,
    lost: usize,
    first: bool,
    finished: bool,
    retry_after: Option<u64>,
}

impl Outcome {
    fn add(&mut self, step: &Step) -> bool {
        match step {
            Step::Complete {
                gained,
                lost,
                first,
            } => {
                self.gained += gained;
                self.lost += lost;
                self.first |= first;
                self.finished = true;
                false
            }
            Step::Short => true,
            Step::More => false,
        }
    }
}

fn sync_result(p: Platform, account: &str, lists: &[Snap], o: &Outcome) -> Value {
    let tool = format!("{}audience_sync", p.prefix());
    let repo = account.contains('/');
    let wanted = if repo { 1 } else { db::KINDS.len() };
    let complete = lists.len() == wanted && lists.iter().all(|s| s.state == "complete");
    let open = lists.iter().any(|s| s.state == "open");
    let short = lists.iter().any(|s| s.state == "short");
    let mut continue_with = if open {
        json!({ "tool": tool, "args": {} })
    } else if short {
        json!({ "tool": tool, "args": { "accept_short": true } })
    } else {
        Value::Null
    };
    if repo && !continue_with.is_null() {
        continue_with["args"]["repo"] = json!(account);
    }
    let next_step = match o.stopped {
        "rate_limited" if p == Platform::PhotoFeed => "Photo Feed stopped answering list requests (a soft block). Wait at least an hour before calling continue_with and do not retry in a loop; what was read is saved. The official export can be imported meanwhile with ig_audience_import.".to_string(),
        "rate_limited" => "The network is limiting requests. Wait retry_after_seconds, then call continue_with; what was read is saved.".to_string(),
        "sweep_wait" => "Photo Feed leaves accounts out of the plain following list, so the missing ones are being searched one letter at a time at a slow pace. Wait retry_after_seconds, then call continue_with; or import the official export with ig_audience_import, or pass accept_short=true to use the list as it is.".to_string(),
        "short" => "The network returned clearly fewer accounts than the profile count, so that list was not used for any comparison. Sync again later, or call continue_with to accept the list as it is.".to_string(),
        "fresh" => "Both lists were read less than 15 minutes ago. Read the views, or pass restart=true to read again now.".to_string(),
        _ if complete && o.first => format!("History starts now. New followers and unfollowers appear after the next complete sync; the other views are ready in {}audience.", p.prefix()),
        _ if complete => format!("Read the views with {}audience.", p.prefix()),
        _ => "Call continue_with to keep reading from where it stopped.".to_string(),
    };
    let expected: Vec<i64> = lists.iter().filter_map(|s| s.expected).collect();
    json!({
        "account": account,
        "complete": complete,
        "stopped": o.stopped,
        "pages": o.pages,
        "fetched": lists.iter().map(|s| s.count).sum::<i64>(),
        "total_expected": (expected.len() == lists.len() && !lists.is_empty()).then(|| expected.iter().sum::<i64>()),
        "lists": lists_json(lists),
        "changes": o.finished.then(|| json!({ "gained": o.gained, "lost": o.lost })),
        "first_snapshot": o.first,
        "continue_with": continue_with,
        "retry_after_seconds": o.retry_after.or((o.stopped == "rate_limited").then_some(RATE_LIMIT_WAIT)),
        "next_step": next_step,
    })
}

fn latest_lists(conn: &Connection, key: Key) -> rusqlite::Result<Vec<Snap>> {
    let mut out = Vec::new();
    for kind in db::KINDS {
        out.extend(db::latest(conn, key, kind)?);
    }
    Ok(out)
}

async fn sync(p: Platform, a: &Value) -> Result<Value, String> {
    let repo = match opt_str(a, "repo") {
        Some(raw) => stars::resolve(p, "summary", Some(&raw))?.1,
        None => None,
    };
    let _busy = Busy::take(p)?;
    let me = match &repo {
        Some(r) => {
            let v = client(p, "repo", json!({ "repo": r })).await?;
            json!({ "handle": r, "followers": v["repo"]["stats"]["likes"] })
        }
        None => client(p, "me", json!({})).await?,
    };
    let account = me["handle"].as_str().unwrap_or_default().to_lowercase();
    if account.is_empty() {
        return Err(format!(
            "UPSTREAM_CHANGED: {} did not return the handle of the signed-in account",
            p.label()
        ));
    }
    let key = Key {
        platform: p.id(),
        account: &account,
    };
    let restart = a["restart"].as_bool().unwrap_or(false);
    let accept_short = a["accept_short"].as_bool().unwrap_or(false);
    let max_pages = a["max_pages"]
        .as_u64()
        .unwrap_or_else(|| default_pages(p))
        .clamp(1, 200);
    let expected = [me["followers"].as_i64(), me["following"].as_i64()];
    let kinds = &db::KINDS[..if repo.is_some() { 1 } else { db::KINDS.len() }];
    let plan = store::with(SCHEMA, |c| {
        if repo.is_none() {
            db::upsert_account(
                c,
                key,
                &db::Account {
                    user_id: me["id"].as_str().unwrap_or_default(),
                    name: me["name"].as_str().unwrap_or_default(),
                    followers: expected[0],
                    following: expected[1],
                },
                now_s(),
            )?;
        }
        db::plan_kinds(c, key, kinds, now_s(), restart, accept_short, &expected)
    })?;
    let mut o = Outcome {
        stopped: if plan.open.is_empty() && plan.accepted.is_empty() {
            "fresh"
        } else {
            "done"
        },
        ..Default::default()
    };
    for step in &plan.accepted {
        o.add(step);
    }
    let started = Instant::now();
    let mut short = false;
    'lists: for snap in &plan.open {
        let sweepable = p == Platform::PhotoFeed && repo.is_none() && snap.kind == "following";
        let mut cursor = snap.cursor.clone();
        let mut resumed = cursor.is_some();
        loop {
            let sweeping = cursor.as_deref().and_then(sweep::State::parse);
            if sweeping.is_some() && accept_short {
                let step = store::with(SCHEMA, |c| {
                    db::store_page(c, key, snap.id, &[], now_s(), true, |_, _, _| None)
                })?;
                o.add(&step);
                break;
            }
            if let Some(wait) = sweeping.as_ref().and_then(|s| s.wait(now_s())) {
                o.stopped = "sweep_wait";
                o.retry_after = Some(wait);
                break 'lists;
            }
            if o.pages >= max_pages {
                o.stopped = "page_cap";
                break 'lists;
            }
            if started.elapsed() > BUDGET {
                o.stopped = "time";
                break 'lists;
            }
            if o.pages > 0 {
                match sweeping {
                    Some(_) => pause(6000, 12000).await,
                    None => pause(1200, 3500).await,
                }
            }
            let (list, args) = match &sweeping {
                Some(s) => ("following", s.args()),
                None => stars::list_call(repo.as_deref(), &snap.kind, cursor.as_deref()),
            };
            let page = match client(p, list, args).await {
                Ok(v) => v,
                Err(e) if e.starts_with("RATE_LIMITED") => {
                    o.stopped = "rate_limited";
                    if p == Platform::PhotoFeed {
                        o.retry_after = Some(sweep::BLOCK_WAIT as u64);
                    }
                    if let Some(s) = &sweeping {
                        let held = s.blocked(now_s()).encode();
                        store::with(SCHEMA, |c| db::set_cursor(c, snap.id, &held))?;
                    }
                    break 'lists;
                }
                Err(e) if resumed && sweeping.is_none() => {
                    return Err(fail(
                        "RESUME_FAILED",
                        &format!(
                            "the saved position of the {} list was refused: {e}",
                            snap.kind
                        ),
                        true,
                        "call again with restart=true to read the lists from the start",
                    ))
                }
                Err(e) => return Err(e),
            };
            resumed = false;
            o.pages += 1;
            let members: Vec<Member> = page["items"]
                .as_array()
                .map(|l| l.iter().filter_map(Member::from_profile).collect())
                .unwrap_or_default();
            let next = page["next_cursor"].as_str().map(String::from);
            let located = interests::places(&page["items"]);
            if !located.is_empty() {
                let _ = store::with(interests::SCHEMA, |c| {
                    interests::save_places(c, p.id(), &located, now_s())
                });
            }
            let (step, saved) = store::with(SCHEMA, |c| {
                let now = now_s();
                let step = match &sweeping {
                    Some(s) => {
                        let after = s.after(next.clone(), now).map(|s| s.encode());
                        db::store_page(
                            c,
                            key,
                            snap.id,
                            &members,
                            now,
                            accept_short,
                            |total, _, expected| after.filter(|_| !sweep::reached(total, expected)),
                        )?
                    }
                    None if sweepable && !accept_short => db::store_page(
                        c,
                        key,
                        snap.id,
                        &members,
                        now,
                        accept_short,
                        |total, added, expected| {
                            next.clone()
                                .filter(|_| added > 0)
                                .or_else(|| sweep::begin(total, expected))
                        },
                    )?,
                    None => db::add_page(
                        c,
                        key,
                        snap.id,
                        &members,
                        next.as_deref(),
                        now,
                        accept_short,
                    )?,
                };
                Ok((step, db::cursor(c, snap.id)?))
            })?;
            short |= o.add(&step);
            if step != Step::More {
                break;
            }
            cursor = saved;
        }
    }
    if short && o.stopped == "done" {
        o.stopped = "short";
    }
    let lists = store::with(SCHEMA, |c| latest_lists(c, key))?;
    let mut out = sync_result(p, &account, &lists, &o);
    match &repo {
        Some(r) => stars::relabel_sync(&mut out, r),
        None => super::insights::alerts::after_sync(p, &out),
    }
    Ok(out)
}

fn is_export(s: &Snap) -> bool {
    s.state == "export"
}

fn whole(s: &Snap) -> bool {
    s.expected.is_none_or(|e| s.count >= e)
}

fn pick_followers(site: Option<Snap>, export: Option<Snap>) -> Option<Snap> {
    site.or(export)
}

fn pick_following(site: Option<Snap>, export: Option<Snap>) -> Option<Snap> {
    match (site, export) {
        (Some(s), Some(e)) if whole(&s) && s.finished >= e.finished => Some(s),
        (site, export) => export.or(site),
    }
}

fn side(s: &Option<Snap>) -> Value {
    let Some(s) = s else { return Value::Null };
    let export = is_export(s);
    json!({
        "fetched": s.count,
        "expected": if export && s.kind == "following" { Some(s.count) } else { s.expected },
        "source": if export { "export" } else { "site" },
        "at": s.finished,
    })
}

fn completeness(prefix: &str, used: &[Option<Snap>; 2]) -> (Value, Option<String>) {
    let short = |s: &Option<Snap>| {
        s.as_ref()
            .filter(|s| !is_export(s))
            .and_then(|s| Some((s.count, s.expected.filter(|e| *e > s.count)?)))
    };
    let mut parts = Vec::new();
    if let Some((got, want)) = short(&used[1]) {
        parts.push(format!("The following list is incomplete: {got} of {want} accounts were read ({} missing), so following, not_following_back, mutuals and fans are not exact.", want - got));
    }
    if let Some((got, want)) = short(&used[0]) {
        parts.push(format!("The followers list is incomplete: {got} of {want} accounts were read ({} missing), so an account can be listed as not following back by mistake.", want - got));
    }
    if !parts.is_empty() {
        parts.push(if prefix == "ig_" {
            "Sync again to complete the lists, or import the official export with ig_audience_import.".to_string()
        } else {
            format!("Sync again with {prefix}audience_sync to read the lists again.")
        });
    }
    if used[0].as_ref().is_some_and(is_export) {
        parts.push(format!("Followers come from the official export, which can leave followers out; confirm the accounts listed as not following back with {prefix}audience_sync."));
    }
    (
        json!({ "followers": side(&used[0]), "following": side(&used[1]) }),
        (!parts.is_empty()).then(|| parts.join(" ")),
    )
}

fn load(conn: &Connection, key: Key, now: i64) -> rusqlite::Result<(Data, [Option<Snap>; 2])> {
    let fo = pick_followers(
        db::latest_complete(conn, key, "followers")?,
        db::latest_export(conn, key, "followers")?,
    );
    let fg = pick_following(
        db::latest_complete(conn, key, "following")?,
        db::latest_export(conn, key, "following")?,
    );
    let ids = if fo.iter().chain(&fg).any(is_export) {
        db::site_ids(conn, key)?
    } else {
        Default::default()
    };
    let list = |s: &Option<Snap>| -> rusqlite::Result<Option<Vec<Member>>> {
        let Some(s) = s else { return Ok(None) };
        if !is_export(s) {
            return db::members(conn, s.id).map(Some);
        }
        let mut members = db::export_members(conn, s.id)?;
        for m in &mut members {
            if let Some(id) = ids.get(&m.handle.to_lowercase()) {
                m.id = id.clone();
            }
        }
        Ok(Some(members))
    };
    let data = Data {
        platform: key.platform.to_string(),
        followers: list(&fo)?,
        following: list(&fg)?,
        keep: db::marks(conn, key, "keep")?,
        hidden: db::marks(conn, key, "hidden")?,
        events: db::events(conn, key, "followers")?,
        activity: db::activity(conn, key.platform, now)?,
        now,
    };
    Ok((data, [fo, fg]))
}

fn empty_reason(
    view: View,
    synced: &[Option<Snap>; 2],
    history: usize,
    prefix: &str,
) -> (&'static str, String) {
    let (fo, fg) = (synced[0].is_some(), synced[1].is_some());
    let ready = match view {
        View::Followers | View::NewFollowers | View::Unfollowers => fo,
        View::Following | View::Inactive => fg,
        View::NotFollowingBack | View::Fans | View::Mutuals => fo && fg,
        _ => true,
    };
    if !ready {
        return (
            "not_synced",
            format!(
                "Call {prefix}audience_sync until complete is true, then read this view again."
            ),
        );
    }
    if matches!(view, View::NewFollowers | View::Unfollowers) && history < 2 {
        return (
            "history_too_short",
            format!("History started at the first sync. Run {prefix}audience_sync again later: changes between two complete syncs appear here."),
        );
    }
    (
        "no_matches",
        "No account matches this view and these filters.".to_string(),
    )
}

fn read(conn: &Connection, key: Key, view: View, q: &Query, now: i64) -> rusqlite::Result<Value> {
    let (d, synced) = load(conn, key, now)?;
    let prefix = Platform::parse(key.platform)
        .map(Platform::prefix)
        .unwrap_or_default();
    let all = db::complete_snapshots(conn, key)?;
    let history: Vec<&Snap> = all.iter().filter(|s| s.kind == "followers").collect();
    let started = history.first().and_then(|s| s.finished);
    let synced_at = json!({
        "followers": synced[0].as_ref().and_then(|s| s.finished),
        "following": synced[1].as_ref().and_then(|s| s.finished),
    });
    let mut out = match view {
        View::Summary => json!({
            "view": "summary",
            "counts": views::counts(&d, q),
            "inactive_days": q.inactive_days,
            "history": { "snapshots": history.len(), "started": started, "too_short": history.len() < 2 },
            "lists": lists_json(&latest_lists(conn, key)?),
        }),
        View::Trend => {
            let days = views::trend(&all, &d.events, q.since);
            json!({
                "view": "trend",
                "total": days.len(),
                "days": days,
                "retention": views::retention(d.followers.as_deref().unwrap_or_default(), started, now),
                "too_short": history.len() < 2,
            })
        }
        _ => {
            let mut v = views::paged(view, views::rows(&d, view, q), q);
            if v["total"] == 0 {
                let (reason, next) = empty_reason(view, &synced, history.len(), prefix);
                v["empty_reason"] = json!(reason);
                v["next_step"] = json!(next);
            }
            if view == View::Inactive {
                v["unknown"] = json!(views::inactive_unknown(&d, q).len());
                v["inactive_days"] = json!(q.inactive_days);
            }
            v
        }
    };
    out["account"] = json!(key.account);
    out["synced_at"] = synced_at;
    if !key.account.contains('/') && synced.iter().any(Option::is_some) {
        let (lists, warning) = completeness(prefix, &synced);
        out["completeness"] = lists;
        if let Some(w) = warning {
            out["warning"] = json!(w);
        }
    }
    Ok(out)
}

fn import_into(
    conn: &mut Connection,
    key: Key,
    export: &import::Export,
    now: i64,
) -> rusqlite::Result<Value> {
    for kind in [import::Kind::Followers, import::Kind::Following] {
        if let Some(list) = export.list(kind) {
            let members: Vec<Member> = list.iter().map(import::member).collect();
            db::replace_export(conn, key, kind.id(), &members, now)?;
        }
    }
    let (d, used) = load(conn, key, now)?;
    let (lists, warning) = completeness("ig_", &used);
    let both = used.iter().all(Option::is_some);
    let count = |list: Option<&[import::Entry]>| {
        list.map(|l| json!({ "accounts": l.len(), "with_date": l.iter().filter(|e| e.at.is_some()).count() }))
    };
    let next_step = if used[0].as_ref().is_some_and(is_export) {
        "Run ig_audience_sync to read the followers from the site (the export can leave followers out), then read the views with ig_audience."
    } else if used[1].as_ref().is_some_and(|s| !is_export(s)) {
        "The following list read from the site is complete and newer than this export, so it stays in use. Read the views with ig_audience."
    } else if !both {
        "Run ig_audience_sync to read the list that is still missing, then read the views with ig_audience."
    } else {
        "Read the views with ig_audience: followers come from the site and following from the export."
    };
    Ok(json!({
        "account": key.account,
        "files": import::files_json(export),
        "imported": {
            "followers": count(export.list(import::Kind::Followers)),
            "following": count(export.list(import::Kind::Following)),
        },
        "sources": { "followers": lists["followers"]["source"], "following": lists["following"]["source"] },
        "completeness": lists,
        "warning": warning,
        "not_following_back": both.then(|| views::rows(&d, View::NotFollowingBack, &Query::default()).len()),
        "next_step": next_step,
    }))
}

async fn import(p: Platform, a: &Value) -> Result<Value, String> {
    let path = PathBuf::from(req_str(a, "path")?);
    if !path.is_absolute() {
        return Err("INVALID_ARGUMENTS: path must be an absolute path".into());
    }
    let account = match opt_str(a, "account") {
        Some(h) => Some(h.trim_start_matches('@').to_lowercase()),
        None => match import::account_from_name(&path) {
            Some(h) => Some(h),
            None => store::with(SCHEMA, |c| db::latest_account(c, p.id()))?,
        },
    }
    .ok_or_else(|| {
        fail(
            "ACCOUNT_UNKNOWN",
            "the export does not say which account it belongs to and no account was synced yet",
            false,
            "pass account with the Photo Feed handle the export belongs to",
        )
    })?;
    let export = tokio::task::spawn_blocking(move || import::read(&path))
        .await
        .map_err(|e| format!("EXPORT_INVALID: {e}"))??;
    let key = Key {
        platform: p.id(),
        account: &account,
    };
    store::with(SCHEMA, |c| import_into(c, key, &export, now_s()))
}

fn account_of(p: Platform, a: &Value) -> Result<Option<String>, String> {
    if let Some(raw) = opt_str(a, "repo") {
        return stars::resolve(p, "summary", Some(&raw)).map(|r| r.1);
    }
    match opt_str(a, "account") {
        Some(h) => Ok(Some(h.trim_start_matches('@').to_lowercase())),
        None => store::with(SCHEMA, |c| db::latest_account(c, p.id())),
    }
}

fn newest_post(page: &Value) -> Option<i64> {
    page["items"]
        .as_array()?
        .iter()
        .filter_map(|i| i["created_at"].as_i64())
        .max()
}

async fn last_post(p: Platform, handle: &str) -> Result<Option<i64>, String> {
    match client(p, "user_posts", who(p, handle)).await {
        Ok(v) => Ok(newest_post(&v)),
        Err(e) if e.starts_with("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

async fn exists(p: Platform, handle: &str) -> Result<bool, String> {
    let tool = match p {
        Platform::PinBoard | Platform::CodeHub => "user",
        _ => "profile",
    };
    match client(p, tool, who(p, handle)).await {
        Ok(_) => Ok(true),
        Err(e) if e.starts_with("NOT_FOUND") => Ok(false),
        Err(e) => Err(e),
    }
}

fn unchecked(d: &Data, q: &Query, cap: usize) -> Vec<(i64, String)> {
    views::rows(d, View::Unfollowers, q)
        .into_iter()
        .skip(q.offset)
        .take(q.limit)
        .filter(|r| r.checked == Some(false) && r.current != Some(true))
        .filter_map(|r| Some((r.event_id?, r.member.handle)))
        .take(cap)
        .collect()
}

async fn view(p: Platform, a: &Value) -> Result<Value, String> {
    let name = req_str(a, "view")?;
    if name == "filters" {
        return filters::list(p);
    }
    let (merged, applied) = filters::apply(p, a)?;
    let a = &merged;
    let (base, repo) = stars::resolve(p, &name, opt_str(a, "repo").as_deref())?;
    let view =
        View::parse(&base).ok_or_else(|| format!("INVALID_ARGUMENTS: unknown view {name}"))?;
    let q = Query::parse(a)?;
    let account = account_of(p, a)?.unwrap_or_default();
    let key = Key {
        platform: p.id(),
        account: &account,
    };
    if view == View::Interests {
        return interests_view(p, key, q.limit);
    }
    let cap = a["check_limit"].as_u64().unwrap_or(5).min(10) as usize;
    let mut checked = 0usize;
    let mut stopped = None;
    if cap > 0 && view == View::Inactive {
        let todo: Vec<Member> = store::with(SCHEMA, |c| {
            let (d, _) = load(c, key, now_s())?;
            Ok(views::inactive_unknown(&d, &q)
                .into_iter()
                .take(cap)
                .cloned()
                .collect())
        })?;
        for m in &todo {
            if checked > 0 {
                pause(500, 1400).await;
            }
            match last_post(p, &m.handle).await {
                Ok(t) => {
                    store::with(SCHEMA, |c| db::set_activity(c, p.id(), &m.id, t, now_s()))?;
                    checked += 1;
                }
                Err(e) => {
                    stopped = Some(e);
                    break;
                }
            }
        }
    }
    if cap > 0 && view == View::Unfollowers {
        let todo = store::with(SCHEMA, |c| {
            let (d, _) = load(c, key, now_s())?;
            Ok(unchecked(&d, &q, cap))
        })?;
        for (event, handle) in &todo {
            if checked > 0 {
                pause(500, 1400).await;
            }
            match exists(p, handle).await {
                Ok(there) => {
                    store::with(SCHEMA, |c| db::set_gone(c, *event, !there, now_s()))?;
                    checked += 1;
                }
                Err(e) => {
                    stopped = Some(e);
                    break;
                }
            }
        }
    }
    let mut out = store::with(SCHEMA, |c| read(c, key, view, &q, now_s()))?;
    if matches!(view, View::Inactive | View::Unfollowers) {
        out["checked_now"] = json!(checked);
        if let Some(e) = stopped {
            out["check_stopped"] = json!(e);
        }
    }
    if let Some(r) = &repo {
        stars::relabel(&mut out, &name, r);
    }
    if !applied.is_null() {
        out["filter"] = applied;
    }
    Ok(out)
}

fn interests_view(p: Platform, key: Key, limit: usize) -> Result<Value, String> {
    let (followers, synced) = store::with(SCHEMA, |c| {
        let snap = db::latest_complete(c, key, "followers")?;
        let list = snap.as_ref().map(|s| db::members(c, s.id)).transpose()?;
        Ok((list, snap.and_then(|s| s.finished)))
    })?;
    let places = store::with(interests::SCHEMA, |c| interests::stored_places(c, p.id()))?;
    let mut out = interests::interests(followers.as_deref().unwrap_or_default(), &places, limit);
    out["account"] = json!(key.account);
    out["synced_at"] = json!({ "followers": synced });
    if followers.is_none() {
        out["empty_reason"] = json!("not_synced");
        out["next_step"] = json!(format!(
            "Call {}audience_sync until complete is true, then read this view again.",
            p.prefix()
        ));
    }
    Ok(out)
}

fn mark(p: Platform, a: &Value) -> Result<Value, String> {
    if a["list"].as_str() == Some("filters") {
        return filters::mark(p, a);
    }
    let list = match req_str(a, "list")?.as_str() {
        "keep" => "keep",
        "hidden" => "hidden",
        other => {
            return Err(format!(
                "INVALID_ARGUMENTS: list must be keep or hidden, not {other}"
            ))
        }
    };
    let handles: Vec<String> = a["handles"]
        .as_array()
        .map(|l| {
            l.iter()
                .filter_map(|h| h.as_str())
                .map(|h| h.trim().to_string())
                .filter(|h| !h.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if handles.is_empty() || handles.len() > 100 {
        return Err("INVALID_ARGUMENTS: handles must have 1 to 100 handles".into());
    }
    let remove = a["remove"].as_bool().unwrap_or(false);
    let account = account_of(p, a)?.ok_or_else(|| {
        fail(
            "NOT_SYNCED",
            "no audience snapshot exists for this network yet",
            false,
            &format!("call {}audience_sync first", p.prefix()),
        )
    })?;
    let key = Key {
        platform: p.id(),
        account: &account,
    };
    let (changed, missing, total) = store::with(SCHEMA, |c| {
        let (n, missing) = db::set_marks(c, key, list, &handles, remove, now_s())?;
        Ok((n, missing, db::marks(c, key, list)?.len()))
    })?;
    let mut out = json!({ "list": list, "account": account, "changed": changed, "removed": remove, "not_found": missing, "total": total });
    if !missing.is_empty() {
        out["next_step"] = json!(if remove {
            "The handles in not_found were not on this list."
        } else {
            "The handles in not_found are in no stored list. Check the spelling, or sync first: only accounts seen in a snapshot can be marked."
        });
    }
    Ok(out)
}

fn export_path(dir: &Path, platform: &str, account: &str, view: &str, stamp: &str) -> PathBuf {
    let safe: String = account
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let base = format!("{platform}-{safe}-{view}-{stamp}");
    let mut path = dir.join(format!("{base}.csv"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{base}-{n}.csv"));
        n += 1;
    }
    path
}

fn write_new(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            fail(
                "WRITE_FAILED",
                &format!("could not create the folder: {e}"),
                false,
                "choose another path",
            )
        })?;
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                fail(
                    "FILE_EXISTS",
                    "a file already exists at that path and exports never overwrite",
                    false,
                    "choose another path or leave path out",
                )
            } else {
                fail(
                    "WRITE_FAILED",
                    &format!("could not create the file: {e}"),
                    false,
                    "choose another path",
                )
            }
        })?;
    f.write_all(text.as_bytes()).map_err(|e| {
        fail(
            "WRITE_FAILED",
            &format!("could not write the file: {e}"),
            true,
            "try again",
        )
    })
}

async fn export(p: Platform, a: &Value) -> Result<Value, String> {
    let name = req_str(a, "view")?;
    let (base, repo) = stars::resolve(p, &name, opt_str(a, "repo").as_deref())?;
    let view = View::parse(&base)
        .filter(|v| v.is_list())
        .ok_or_else(|| format!("INVALID_ARGUMENTS: view {name} cannot be exported"))?;
    let q = Query::parse(a)?;
    let account = account_of(p, a)?.unwrap_or_default();
    let key = Key {
        platform: p.id(),
        account: &account,
    };
    let rows = store::with(SCHEMA, |c| {
        let (d, _) = load(c, key, now_s())?;
        Ok(views::rows(&d, view, &q))
    })?;
    if rows.is_empty() {
        return Err(fail(
            "NOTHING_TO_EXPORT",
            "this view has no rows for this account and these filters",
            false,
            &format!(
                "read the view with {}audience to see why it is empty",
                p.prefix()
            ),
        ));
    }
    let truncated = rows.len() > EXPORT_CAP;
    let rows = &rows[..rows.len().min(EXPORT_CAP)];
    let path = match opt_str(a, "path") {
        Some(raw) => {
            let path = PathBuf::from(raw);
            let csv = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("csv"));
            if !path.is_absolute() || !csv {
                return Err(
                    "INVALID_ARGUMENTS: path must be an absolute path ending in .csv".into(),
                );
            }
            path
        }
        None => {
            let dir = store::dir()
                .ok_or("STORE_UNAVAILABLE: the app data folder could not be found")?
                .join("exports");
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
            export_path(&dir, p.id(), &account, &name, &stamp)
        }
    };
    let text = views::csv(rows);
    let target = path.clone();
    tokio::task::spawn_blocking(move || write_new(&target, &text))
        .await
        .map_err(|e| format!("WRITE_FAILED: {e}"))??;
    Ok(
        json!({ "path": path, "rows": rows.len(), "view": name, "account": account, "repo": repo, "truncated": truncated }),
    )
}

pub async fn call(_caller: &Caller, tool: &str, a: &Value) -> Result<Value, String> {
    let spec = SPECS
        .iter()
        .find(|s| s.name == tool)
        .ok_or_else(|| format!("UNKNOWN_TOOL: {tool}"))?;
    let p = spec.platform;
    match tool.strip_prefix(p.prefix()).unwrap_or_default() {
        "audience_sync" => sync(p, a).await,
        "audience" => view(p, a).await,
        "audience_mark" => mark(p, a),
        "audience_export" => export(p, a).await,
        "audience_import" => import(p, a).await,
        _ => Err(format!("UNKNOWN_TOOL: {tool}")),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tools;
    use super::*;

    const K: Key = Key {
        platform: "instagram",
        account: "me",
    };

    fn people(ids: &[&str]) -> Vec<Member> {
        ids.iter()
            .map(|i| Member {
                id: (*i).into(),
                handle: format!("user{i}"),
                ..Default::default()
            })
            .collect()
    }

    fn sync_once(conn: &mut Connection, now: i64, followers: &[&str], following: &[&str]) {
        let plan = db::plan(conn, K, now, true, false, [None, None]).unwrap();
        for s in &plan.open {
            let ids = if s.kind == "followers" {
                followers
            } else {
                following
            };
            db::add_page(conn, K, s.id, &people(ids), None, now + 1, false).unwrap();
        }
    }

    #[test]
    fn specs_are_valid_and_cover_every_network() {
        let mut seen = std::collections::HashSet::new();
        assert_eq!(SPECS.len(), 21);
        for s in SPECS {
            assert!(seen.insert(s.name), "duplicate {}", s.name);
            assert!(s.name.starts_with(s.platform.prefix()), "{}", s.name);
            assert!(!s.write);
            assert_eq!(s.route, Route::Audience);
            let schema = tools::input_schema(s);
            for r in s.required {
                assert!(
                    schema["properties"][*r].is_object(),
                    "{} requires missing {r}",
                    s.name
                );
            }
            assert!(schema["properties"]
                .as_object()
                .is_some_and(|o| !o.is_empty()));
            assert!(s.description.matches(". ").count() <= 1, "{}", s.name);
        }
        let views_in_schema = tools::input_schema(&SPECS[1])["properties"]["view"]["enum"]
            .as_array()
            .unwrap()
            .len();
        assert_eq!(views_in_schema, views::VIEWS.len());
        let sorts = tools::input_schema(&SPECS[1])["properties"]["sort"]["enum"]
            .as_array()
            .unwrap()
            .len();
        assert_eq!(sorts, views::SORTS.len());
    }

    #[test]
    fn pinboard_and_codehub_have_the_tools_and_forums_has_none() {
        for name in [
            "audience_sync",
            "audience",
            "audience_mark",
            "audience_export",
        ] {
            assert!(tools::find(&format!("pin_{name}")).is_some());
            assert!(tools::find(&format!("gh_{name}")).is_some());
            assert!(tools::find(&format!("rd_{name}")).is_none());
        }
        let enum_of = |tool: &str| -> Vec<String> {
            tools::input_schema(tools::find(tool).unwrap())["properties"]["view"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        };
        for (tool, base) in [
            ("gh_audience", "x_audience"),
            ("gh_audience_export", "x_audience_export"),
        ] {
            let gh = enum_of(tool);
            for v in enum_of(base) {
                assert!(gh.contains(&v), "{tool} misses view {v}");
            }
            for (star, _) in stars::VIEWS {
                assert!(gh.contains(&star.to_string()), "{tool} misses {star}");
                assert!(!enum_of(base).contains(&star.to_string()));
            }
        }
        for tool in [
            "gh_audience_sync",
            "gh_audience",
            "gh_audience_mark",
            "gh_audience_export",
        ] {
            let schema = tools::input_schema(tools::find(tool).unwrap());
            assert!(schema["properties"]["repo"].is_object(), "{tool}");
        }
        assert!(
            tools::input_schema(tools::find("pin_audience").unwrap())["properties"]["repo"]
                .is_null()
        );
    }

    #[test]
    fn a_repository_sync_continues_with_its_repo_and_reads_as_stargazers() {
        let list = |state: &str| Snap {
            id: 1,
            kind: "followers".into(),
            started: 10,
            finished: (state == "complete").then_some(20),
            state: state.into(),
            count: 3,
            expected: Some(3),
            cursor: (state == "open").then(|| "2".to_string()),
        };
        let o = Outcome {
            stopped: "time",
            ..Default::default()
        };
        let mut out = sync_result(Platform::CodeHub, "ana/tool", &[list("open")], &o);
        assert_eq!(out["complete"], false);
        assert_eq!(
            out["continue_with"],
            json!({ "tool": "gh_audience_sync", "args": { "repo": "ana/tool" } })
        );
        stars::relabel_sync(&mut out, "ana/tool");
        assert_eq!(out["repo"], "ana/tool");
        assert_eq!(out["lists"][0]["kind"], "stargazers");
        let o = Outcome {
            stopped: "done",
            finished: true,
            first: true,
            ..Default::default()
        };
        let mut done = sync_result(Platform::CodeHub, "ana/tool", &[list("complete")], &o);
        assert_eq!(done["complete"], true);
        assert!(done["continue_with"].is_null());
        stars::relabel_sync(&mut done, "ana/tool");
        assert!(done["next_step"].as_str().unwrap().contains("unstarred"));
    }

    #[test]
    fn stargazer_views_come_from_the_same_snapshots_and_diff() {
        let mut c = store::memory(SCHEMA);
        let repo = Key {
            platform: "github",
            account: "ana/tool",
        };
        for (now, ids) in [(1000, vec!["1", "2", "3"]), (90_000, vec!["2", "3", "4"])] {
            let plan =
                db::plan_kinds(&mut c, repo, &db::KINDS[..1], now, true, false, &[None]).unwrap();
            db::add_page(
                &mut c,
                repo,
                plan.open[0].id,
                &people(&ids),
                None,
                now + 1,
                false,
            )
            .unwrap();
        }
        let read_view = |name: &str| {
            let (base, r) = stars::resolve(Platform::CodeHub, name, Some("Ana/Tool")).unwrap();
            let mut out = read(
                &c,
                repo,
                View::parse(&base).unwrap(),
                &Query::default(),
                100_000,
            )
            .unwrap();
            stars::relabel(&mut out, name, r.as_deref().unwrap());
            out
        };
        let handles = |v: &Value| -> Vec<String> {
            v["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["handle"].as_str().unwrap().to_string())
                .collect()
        };
        let all = read_view("stargazers");
        assert_eq!(all["total"], 3);
        assert_eq!(all["repo"], "ana/tool");
        assert_eq!(all["items"][0]["url"], "https://github.com/user2");
        assert_eq!(handles(&read_view("new_stargazers")), vec!["user4"]);
        let lost = read_view("unstarred");
        assert_eq!(handles(&lost), vec!["user1"]);
        assert_eq!(lost["items"][0]["status"], "unstarred");
        assert_eq!(lost["items"][0]["between"], json!([1000, 90_001]));
        let summary = read_view("summary");
        assert_eq!(summary["counts"]["stargazers"], 3);
        assert_eq!(summary["counts"]["unstarred"], 1);
        assert_eq!(summary["history"]["snapshots"], 2);
    }

    #[test]
    fn first_run_explains_itself_in_every_view() {
        let c = store::memory(SCHEMA);
        let q = Query::default();
        let s = read(&c, K, View::Summary, &q, 100).unwrap();
        assert_eq!(s["history"]["too_short"], true);
        assert_eq!(s["counts"]["followers"], 0);
        assert_eq!(s["synced_at"]["followers"], Value::Null);
        assert_eq!(s["lists"], json!([]));
        let v = read(&c, K, View::NotFollowingBack, &q, 100).unwrap();
        assert_eq!(v["total"], 0);
        assert_eq!(v["empty_reason"], "not_synced");
        assert!(v["next_step"]
            .as_str()
            .unwrap()
            .contains("ig_audience_sync"));
        let t = read(&c, K, View::Trend, &q, 100).unwrap();
        assert_eq!(t["too_short"], true);
        assert_eq!(t["days"], json!([]));
    }

    #[test]
    fn the_tool_reads_views_end_to_end_from_two_syncs() {
        let mut c = store::memory(SCHEMA);
        sync_once(&mut c, 1_000, &["1", "2", "3"], &["2", "3", "4"]);
        let q = Query::default();
        let first = read(&c, K, View::Unfollowers, &q, 2_000).unwrap();
        assert_eq!(first["empty_reason"], "history_too_short");
        let nfb = read(&c, K, View::NotFollowingBack, &q, 2_000).unwrap();
        assert_eq!(nfb["items"][0]["handle"], "user4");
        assert_eq!(nfb["items"][0]["url"], "https://www.instagram.com/user4/");
        assert_eq!(nfb["synced_at"]["following"], 1_001);

        sync_once(&mut c, 90_000, &["2", "3", "5"], &["2", "3", "4"]);
        db::set_marks(&mut c, K, "keep", &["user4".into()], false, 90_010).unwrap();
        let lost = read(&c, K, View::Unfollowers, &q, 90_100).unwrap();
        assert_eq!(lost["total"], 1);
        assert_eq!(lost["items"][0]["handle"], "user1");
        assert_eq!(lost["items"][0]["between"], json!([1_000, 90_001]));
        assert_eq!(lost["items"][0]["status"], "unfollowed");
        assert_eq!(lost["items"][0]["checked"], false);
        let gained = read(&c, K, View::NewFollowers, &q, 90_100).unwrap();
        assert_eq!(gained["items"][0]["handle"], "user5");
        let s = read(&c, K, View::Summary, &q, 90_100).unwrap();
        assert_eq!(s["history"]["snapshots"], 2);
        assert_eq!(s["history"]["too_short"], false);
        assert_eq!(s["counts"]["fans"], 1);
        assert_eq!(s["counts"]["mutuals"], 2);
        assert_eq!(s["counts"]["not_following_back"], 1);
        assert_eq!(s["counts"]["kept"], 1);
        assert_eq!(s["lists"][0]["state"], "complete");
        let nfb = read(&c, K, View::NotFollowingBack, &q, 90_100).unwrap();
        assert_eq!(nfb["items"][0]["kept"], true);
        let t = read(&c, K, View::Trend, &q, 90_100).unwrap();
        assert_eq!(t["days"][1]["gained"], 1);
        assert_eq!(t["days"][1]["lost"], 1);
        assert_eq!(t["days"][1]["followers"], 3);
        assert_eq!(t["retention"]["since_before_history"], 2);
        assert_eq!(t["retention"]["buckets"][0]["count"], 1);

        let d = load(&c, K, 90_100).unwrap().0;
        let todo = unchecked(&d, &q, 5);
        assert_eq!(todo.len(), 1);
        db::set_gone(&c, todo[0].0, true, 90_200).unwrap();
        let lost = read(&c, K, View::Unfollowers, &q, 90_300).unwrap();
        assert_eq!(lost["items"][0]["status"], "gone");
        assert!(unchecked(&load(&c, K, 90_300).unwrap().0, &q, 5).is_empty());
    }

    #[test]
    fn sync_result_says_how_to_continue() {
        let snap = |kind: &str, state: &str, count: i64, cursor: Option<&str>| Snap {
            id: 1,
            kind: kind.into(),
            started: 0,
            finished: None,
            state: state.into(),
            count,
            expected: Some(100),
            cursor: cursor.map(String::from),
        };
        let o = |stopped: &'static str| Outcome {
            stopped,
            pages: 2,
            ..Default::default()
        };
        let partial = [
            snap("followers", "open", 60, Some("c")),
            snap("following", "open", 0, None),
        ];
        let v = sync_result(Platform::ShortPost, "me", &partial, &o("page_cap"));
        assert_eq!(v["complete"], false);
        assert_eq!(v["fetched"], 60);
        assert_eq!(v["total_expected"], 200);
        assert_eq!(v["continue_with"]["tool"], "x_audience_sync");
        assert_eq!(v["lists"][0]["state"], "partial");
        assert_eq!(v["lists"][1]["state"], "pending");
        assert_eq!(v["changes"], Value::Null);
        let v = sync_result(Platform::ShortPost, "me", &partial, &o("rate_limited"));
        assert_eq!(v["retry_after_seconds"], 900);
        let done = [
            snap("followers", "complete", 100, None),
            snap("following", "complete", 100, None),
        ];
        let v = sync_result(
            Platform::ShortPost,
            "me",
            &done,
            &Outcome {
                stopped: "done",
                first: true,
                finished: true,
                ..Default::default()
            },
        );
        assert_eq!(v["complete"], true);
        assert_eq!(v["continue_with"], Value::Null);
        assert_eq!(v["changes"]["lost"], 0);
        assert!(v["next_step"]
            .as_str()
            .unwrap()
            .starts_with("History starts now"));
        let short = [
            snap("followers", "short", 10, None),
            snap("following", "complete", 100, None),
        ];
        let v = sync_result(Platform::ClipFeed, "me", &short, &o("short"));
        assert_eq!(v["complete"], false);
        assert_eq!(v["continue_with"]["args"]["accept_short"], true);
        assert_eq!(v["continue_with"]["tool"], "tt_audience_sync");
    }

    #[test]
    fn a_sweep_pause_and_an_photofeed_block_say_how_long_to_wait() {
        let open = [Snap {
            id: 2,
            kind: "following".into(),
            started: 0,
            finished: None,
            state: "open".into(),
            count: 198,
            expected: Some(222),
            cursor: Some("sweep:1:4:900:".into()),
        }];
        let o = Outcome {
            stopped: "sweep_wait",
            retry_after: Some(300),
            ..Default::default()
        };
        let v = sync_result(Platform::PhotoFeed, "me", &open, &o);
        assert_eq!(v["complete"], false);
        assert_eq!(v["retry_after_seconds"], 300);
        assert_eq!(v["continue_with"]["tool"], "ig_audience_sync");
        assert_eq!(v["lists"][0]["state"], "partial");
        assert!(v["next_step"]
            .as_str()
            .unwrap()
            .contains("ig_audience_import"));
        let o = Outcome {
            stopped: "rate_limited",
            retry_after: Some(3600),
            ..Default::default()
        };
        let v = sync_result(Platform::PhotoFeed, "me", &open, &o);
        assert_eq!(v["retry_after_seconds"], 3600);
        assert!(v["next_step"]
            .as_str()
            .unwrap()
            .contains("at least an hour"));
    }

    fn site_lists(c: &mut Connection, followers: &[&str], following: &[&str], expected: [i64; 2]) {
        let plan = db::plan(c, K, 1_000, true, false, expected.map(Some)).unwrap();
        for s in &plan.open {
            let ids = if s.kind == "followers" {
                followers
            } else {
                following
            };
            db::add_page(c, K, s.id, &people(ids), None, 1_001, true).unwrap();
        }
    }

    fn export_of(followers: Option<&[&str]>, following: Option<&[&str]>) -> import::Export {
        let list = |l: &[&str]| -> Vec<import::Entry> {
            l.iter()
                .enumerate()
                .map(|(i, h)| import::Entry {
                    handle: (*h).into(),
                    at: Some(500 + i as i64),
                })
                .collect()
        };
        import::Export {
            files: Vec::new(),
            followers: followers.map(list),
            following: following.map(list),
        }
    }

    #[test]
    fn a_view_says_when_a_list_is_incomplete_and_by_how_much() {
        let mut c = store::memory(SCHEMA);
        let followers: Vec<String> = (1..=30).map(|i| i.to_string()).collect();
        let followers: Vec<&str> = followers.iter().map(String::as_str).collect();
        site_lists(&mut c, &followers, &followers[..27], [30, 30]);
        let q = Query::default();
        for view in [
            View::NotFollowingBack,
            View::Mutuals,
            View::Fans,
            View::Following,
            View::Summary,
        ] {
            let v = read(&c, K, view, &q, 2_000).unwrap();
            assert_eq!(
                v["completeness"],
                json!({
                    "followers": { "fetched": 30, "expected": 30, "source": "site", "at": 1_001 },
                    "following": { "fetched": 27, "expected": 30, "source": "site", "at": 1_001 },
                }),
                "{view:?}"
            );
            let w = v["warning"].as_str().unwrap();
            assert!(w.contains("27 of 30") && w.contains("3 missing"), "{w}");
            assert!(w.contains("ig_audience_import"));
            assert!(!w.contains("followers list is incomplete"));
        }
        let mut whole = store::memory(SCHEMA);
        site_lists(&mut whole, &["1", "2"], &["2"], [2, 1]);
        let v = read(&whole, K, View::NotFollowingBack, &q, 2_000).unwrap();
        assert_eq!(v["completeness"]["following"]["fetched"], 1);
        assert_eq!(v["warning"], Value::Null);
    }

    #[test]
    fn the_export_gives_following_and_the_site_gives_followers() {
        let mut c = store::memory(SCHEMA);
        site_lists(&mut c, &["1", "2", "3"], &["1", "2"], [3, 5]);
        db::set_marks(&mut c, K, "keep", &["user2".into()], false, 1_500).unwrap();
        let export = export_of(
            Some(&["user1"]),
            Some(&["user1", "user2", "ghost", "user9"]),
        );
        let out = import_into(&mut c, K, &export, 2_000).unwrap();
        assert_eq!(
            out["imported"]["following"],
            json!({ "accounts": 4, "with_date": 4 })
        );
        assert_eq!(out["imported"]["followers"]["accounts"], 1);
        assert_eq!(
            out["sources"],
            json!({ "followers": "site", "following": "export" })
        );
        assert_eq!(out["not_following_back"], 2);
        assert_eq!(out["warning"], Value::Null);
        assert_eq!(
            out["completeness"]["following"],
            json!({ "fetched": 4, "expected": 4, "source": "export", "at": 2_000 })
        );
        let q = Query::default();
        let nfb = read(&c, K, View::NotFollowingBack, &q, 2_100).unwrap();
        let handles: Vec<&str> = nfb["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["handle"].as_str().unwrap())
            .collect();
        assert_eq!(handles, vec!["ghost", "user9"]);
        assert_eq!(nfb["items"][0]["followed_at"], 502);
        assert_eq!(nfb["items"][0]["follows_me"], false);
        let following = read(&c, K, View::Following, &q, 2_100).unwrap();
        assert_eq!(following["total"], 4);
        assert_eq!(following["items"][1]["id"], "2");
        assert_eq!(following["items"][1]["kept"], true);
        assert_eq!(following["items"][1]["follows_me"], true);
        assert_eq!(read(&c, K, View::Mutuals, &q, 2_100).unwrap()["total"], 2);
        assert_eq!(read(&c, K, View::Fans, &q, 2_100).unwrap()["total"], 1);
        assert_eq!(read(&c, K, View::Followers, &q, 2_100).unwrap()["total"], 3);
        let s = read(&c, K, View::Summary, &q, 2_100).unwrap();
        assert_eq!(s["history"]["snapshots"], 1);
        assert_eq!(s["counts"]["not_following_back"], 2);
    }

    #[test]
    fn an_export_alone_warns_that_followers_may_be_missing() {
        let mut c = store::memory(SCHEMA);
        let export = export_of(Some(&["ana"]), Some(&["ana", "bia"]));
        let out = import_into(&mut c, K, &export, 2_000).unwrap();
        assert_eq!(
            out["sources"],
            json!({ "followers": "export", "following": "export" })
        );
        assert_eq!(out["not_following_back"], 1);
        assert!(out["warning"]
            .as_str()
            .unwrap()
            .contains("can leave followers out"));
        assert!(out["next_step"]
            .as_str()
            .unwrap()
            .starts_with("Run ig_audience_sync"));
        assert_eq!(out["completeness"]["followers"]["expected"], Value::Null);
        let nfb = read(&c, K, View::NotFollowingBack, &Query::default(), 2_100).unwrap();
        assert_eq!(nfb["items"][0]["handle"], "bia");
        assert!(nfb["warning"]
            .as_str()
            .unwrap()
            .contains("ig_audience_sync"));
        let only = import_into(
            &mut store::memory(SCHEMA),
            K,
            &export_of(None, Some(&["ana"])),
            9,
        )
        .unwrap();
        assert_eq!(only["not_following_back"], Value::Null);
        assert_eq!(only["sources"]["followers"], Value::Null);
    }

    #[test]
    #[ignore = "real data: OMNIGET_IG_EXPORT=<zip or folder> OMNIGET_IG_DB=<a copy of social.db>"]
    fn a_real_export_over_a_copy_of_the_real_database() {
        let export = std::env::var("OMNIGET_IG_EXPORT").expect("OMNIGET_IG_EXPORT");
        let db_path = std::env::var("OMNIGET_IG_DB").expect("OMNIGET_IG_DB");
        let export = import::read(Path::new(&export)).unwrap();
        let mut c = Connection::open(db_path).unwrap();
        c.execute_batch(SCHEMA).unwrap();
        let account = db::latest_account(&c, "instagram").unwrap().unwrap();
        let key = Key {
            platform: "instagram",
            account: &account,
        };
        let before = read(&c, key, View::Summary, &Query::default(), now_s()).unwrap();
        let out = import_into(&mut c, key, &export, now_s()).unwrap();
        let after = read(&c, key, View::Summary, &Query::default(), now_s()).unwrap();
        println!(
            "before: counts {} completeness {} warning {}",
            before["counts"], before["completeness"], before["warning"]
        );
        println!(
            "import: imported {} sources {} not_following_back {} warning {}",
            out["imported"], out["sources"], out["not_following_back"], out["warning"]
        );
        println!(
            "after: counts {} completeness {}",
            after["counts"], after["completeness"]
        );
    }

    #[test]
    fn a_complete_and_newer_site_following_list_wins_over_the_export() {
        let snap = |state: &str, count: i64, expected: Option<i64>, finished: i64| Snap {
            id: 1,
            kind: "following".into(),
            started: 0,
            finished: Some(finished),
            state: state.into(),
            count,
            expected,
            cursor: None,
        };
        let export = || Some(snap("export", 225, None, 100));
        let pick = |site: Option<Snap>| pick_following(site, export()).map(|s| s.state);
        assert_eq!(pick(None).as_deref(), Some("export"));
        assert_eq!(
            pick(Some(snap("complete", 198, Some(222), 200))).as_deref(),
            Some("export")
        );
        assert_eq!(
            pick(Some(snap("complete", 222, Some(222), 50))).as_deref(),
            Some("export")
        );
        assert_eq!(
            pick(Some(snap("complete", 222, Some(222), 200))).as_deref(),
            Some("complete")
        );
        assert_eq!(
            pick_following(Some(snap("complete", 5, Some(9), 1)), None)
                .unwrap()
                .count,
            5
        );
        let site = snap("complete", 201, Some(222), 1);
        assert_eq!(
            pick_followers(Some(site), export()).unwrap().state,
            "complete"
        );
        assert_eq!(pick_followers(None, export()).unwrap().state, "export");
    }

    #[test]
    fn last_post_is_the_newest_item_even_after_a_pinned_one() {
        let page = json!({ "items": [{ "created_at": 50 }, { "created_at": 900 }, { "id": "x" }, { "created_at": 700 }] });
        assert_eq!(newest_post(&page), Some(900));
        assert_eq!(newest_post(&json!({ "items": [] })), None);
        assert_eq!(newest_post(&json!({})), None);
    }

    #[test]
    fn exports_never_overwrite() {
        let dir =
            std::env::temp_dir().join(format!("omniget-audience-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = export_path(&dir, "x", "a/b", "fans", "20261111-100000");
        assert_eq!(first.file_name().unwrap(), "x-a_b-fans-20261111-100000.csv");
        write_new(&first, "a").unwrap();
        let second = export_path(&dir, "x", "a/b", "fans", "20261111-100000");
        assert_eq!(
            second.file_name().unwrap(),
            "x-a_b-fans-20261111-100000-2.csv"
        );
        let err = write_new(&first, "b").unwrap_err();
        assert!(err.starts_with("FILE_EXISTS: {"));
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "a");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn errors_carry_a_next_step() {
        let e = fail("BUSY", "m", true, "wait");
        let (code, rest) = e.split_once(": ").unwrap();
        let v: Value = serde_json::from_str(rest).unwrap();
        assert_eq!(code, "BUSY");
        assert_eq!(v["retryable"], true);
        assert_eq!(v["next_step"], "wait");
    }
}
