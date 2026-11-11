use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use serde_json::Value;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS audience_accounts (
  platform TEXT NOT NULL, account TEXT NOT NULL, user_id TEXT NOT NULL DEFAULT '',
  name TEXT NOT NULL DEFAULT '', followers INTEGER, following INTEGER, updated INTEGER NOT NULL,
  PRIMARY KEY (platform, account));
CREATE TABLE IF NOT EXISTS audience_snapshots (
  id INTEGER PRIMARY KEY AUTOINCREMENT, platform TEXT NOT NULL, account TEXT NOT NULL, kind TEXT NOT NULL,
  started INTEGER NOT NULL, finished INTEGER, state TEXT NOT NULL DEFAULT 'open',
  count INTEGER NOT NULL DEFAULT 0, expected INTEGER, cursor TEXT);
CREATE INDEX IF NOT EXISTS audience_snapshots_key ON audience_snapshots (platform, account, kind, id);
CREATE TABLE IF NOT EXISTS audience_members (
  snapshot_id INTEGER NOT NULL, user_id TEXT NOT NULL, position INTEGER NOT NULL,
  handle TEXT NOT NULL, name TEXT NOT NULL DEFAULT '', avatar TEXT,
  verified INTEGER NOT NULL DEFAULT 0, private INTEGER NOT NULL DEFAULT 0,
  followers INTEGER, following INTEGER, posts INTEGER, bio TEXT NOT NULL DEFAULT '', first_seen INTEGER,
  PRIMARY KEY (snapshot_id, user_id)) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS audience_marks (
  platform TEXT NOT NULL, account TEXT NOT NULL, mark TEXT NOT NULL, user_id TEXT NOT NULL,
  handle TEXT NOT NULL, created INTEGER NOT NULL,
  PRIMARY KEY (platform, account, mark, user_id)) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS audience_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT, platform TEXT NOT NULL, account TEXT NOT NULL, kind TEXT NOT NULL,
  event TEXT NOT NULL, older INTEGER NOT NULL, newer INTEGER NOT NULL, snapshot_id INTEGER NOT NULL,
  gone INTEGER, checked INTEGER,
  user_id TEXT NOT NULL, handle TEXT NOT NULL, name TEXT NOT NULL DEFAULT '', avatar TEXT,
  verified INTEGER NOT NULL DEFAULT 0, private INTEGER NOT NULL DEFAULT 0,
  followers INTEGER, following INTEGER, posts INTEGER, bio TEXT NOT NULL DEFAULT '',
  UNIQUE (snapshot_id, user_id));
CREATE INDEX IF NOT EXISTS audience_events_key ON audience_events (platform, account, kind, newer);
CREATE TABLE IF NOT EXISTS audience_activity (
  platform TEXT NOT NULL, user_id TEXT NOT NULL, last_post INTEGER, checked INTEGER NOT NULL,
  PRIMARY KEY (platform, user_id)) WITHOUT ROWID;
";

pub const RESUME_WINDOW: i64 = 24 * 3600;
pub const FRESH: i64 = 15 * 60;
pub const ACTIVITY_TTL: i64 = 7 * 86400;
pub const KINDS: [&str; 2] = ["followers", "following"];
const COLS: &str =
    "user_id, handle, name, avatar, verified, private, followers, following, posts, bio";

#[derive(Clone, Copy)]
pub struct Key<'a> {
    pub platform: &'a str,
    pub account: &'a str,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Member {
    pub id: String,
    pub handle: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    pub verified: bool,
    pub private: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub followers: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub following: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posts: Option<i64>,
    pub bio: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_seen: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub followed_at: Option<i64>,
}

impl Member {
    pub fn from_profile(v: &Value) -> Option<Self> {
        let text = |k: &str| v[k].as_str().unwrap_or_default().to_string();
        let id = text("id");
        let handle = text("handle");
        if id.is_empty() || handle.is_empty() {
            return None;
        }
        Some(Self {
            id,
            handle,
            name: text("name"),
            avatar: v["avatar"]
                .as_str()
                .filter(|x| !x.is_empty())
                .map(String::from),
            verified: v["verified"].as_bool().unwrap_or(false),
            private: v["private"].as_bool().unwrap_or(false),
            followers: v["followers"].as_i64(),
            following: v["following"].as_i64(),
            posts: v["posts"].as_i64(),
            bio: text("bio"),
            first_seen: None,
            followed_at: None,
        })
    }
}

fn member_at(r: &Row, at: usize) -> rusqlite::Result<Member> {
    Ok(Member {
        id: r.get(at)?,
        handle: r.get(at + 1)?,
        name: r.get(at + 2)?,
        avatar: r.get(at + 3)?,
        verified: r.get(at + 4)?,
        private: r.get(at + 5)?,
        followers: r.get(at + 6)?,
        following: r.get(at + 7)?,
        posts: r.get(at + 8)?,
        bio: r.get(at + 9)?,
        first_seen: None,
        followed_at: None,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snap {
    pub id: i64,
    pub kind: String,
    pub started: i64,
    pub finished: Option<i64>,
    pub state: String,
    pub count: i64,
    pub expected: Option<i64>,
    pub cursor: Option<String>,
}

const SNAP: &str = "id, kind, started, finished, state, count, expected, cursor";

fn snap(r: &Row) -> rusqlite::Result<Snap> {
    Ok(Snap {
        id: r.get(0)?,
        kind: r.get(1)?,
        started: r.get(2)?,
        finished: r.get(3)?,
        state: r.get(4)?,
        count: r.get(5)?,
        expected: r.get(6)?,
        cursor: r.get(7)?,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct EventRow {
    pub id: i64,
    pub event: String,
    pub older: i64,
    pub newer: i64,
    pub gone: Option<bool>,
    pub member: Member,
}

#[derive(Debug, PartialEq)]
pub enum Step {
    More,
    Complete {
        gained: usize,
        lost: usize,
        first: bool,
    },
    Short,
}

pub struct Account<'a> {
    pub user_id: &'a str,
    pub name: &'a str,
    pub followers: Option<i64>,
    pub following: Option<i64>,
}

pub fn upsert_account(conn: &Connection, key: Key, a: &Account, now: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO audience_accounts (platform, account, user_id, name, followers, following, updated)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (platform, account) DO UPDATE SET user_id = ?3, name = ?4, followers = ?5, following = ?6, updated = ?7",
        params![key.platform, key.account, a.user_id, a.name, a.followers, a.following, now],
    )?;
    Ok(())
}

pub fn latest_account(conn: &Connection, platform: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT account FROM audience_accounts WHERE platform = ?1 ORDER BY updated DESC LIMIT 1",
        [platform],
        |r| r.get(0),
    )
    .optional()
}

pub fn latest(conn: &Connection, key: Key, kind: &str) -> rusqlite::Result<Option<Snap>> {
    conn.query_row(
        &format!("SELECT {SNAP} FROM audience_snapshots WHERE platform = ?1 AND account = ?2 AND kind = ?3 AND state != 'export' ORDER BY id DESC LIMIT 1"),
        params![key.platform, key.account, kind],
        snap,
    )
    .optional()
}

pub fn latest_export(conn: &Connection, key: Key, kind: &str) -> rusqlite::Result<Option<Snap>> {
    conn.query_row(
        &format!("SELECT {SNAP} FROM audience_snapshots WHERE platform = ?1 AND account = ?2 AND kind = ?3 AND state = 'export' ORDER BY id DESC LIMIT 1"),
        params![key.platform, key.account, kind],
        snap,
    )
    .optional()
}

pub fn replace_export(
    conn: &mut Connection,
    key: Key,
    kind: &str,
    members: &[Member],
    now: i64,
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM audience_members WHERE snapshot_id IN
         (SELECT id FROM audience_snapshots WHERE platform = ?1 AND account = ?2 AND kind = ?3 AND state = 'export')",
        params![key.platform, key.account, kind],
    )?;
    tx.execute(
        "DELETE FROM audience_snapshots WHERE platform = ?1 AND account = ?2 AND kind = ?3 AND state = 'export'",
        params![key.platform, key.account, kind],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO audience_accounts (platform, account, updated) VALUES (?1, ?2, ?3)",
        params![key.platform, key.account, now],
    )?;
    tx.execute(
        "INSERT INTO audience_snapshots (platform, account, kind, started, finished, state) VALUES (?1, ?2, ?3, ?4, ?4, 'export')",
        params![key.platform, key.account, kind, now],
    )?;
    let snapshot = tx.last_insert_rowid();
    let mut added = 0i64;
    {
        let mut ins = tx.prepare(
            "INSERT OR IGNORE INTO audience_members (snapshot_id, position, user_id, handle, first_seen) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for m in members {
            added += ins.execute(params![snapshot, added, m.id, m.handle, m.followed_at])? as i64;
        }
    }
    tx.execute(
        "UPDATE audience_snapshots SET count = ?1 WHERE id = ?2",
        params![added, snapshot],
    )?;
    tx.commit()
}

pub fn site_ids(conn: &Connection, key: Key) -> rusqlite::Result<HashMap<String, String>> {
    conn.prepare(
        "SELECT lower(m.handle), m.user_id FROM audience_members m JOIN audience_snapshots s ON s.id = m.snapshot_id
         WHERE s.platform = ?1 AND s.account = ?2 AND s.state != 'export' ORDER BY s.id",
    )?
    .query_map(params![key.platform, key.account], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?
    .collect()
}

pub fn cursor(conn: &Connection, snapshot: i64) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT cursor FROM audience_snapshots WHERE id = ?1",
        [snapshot],
        |r| r.get(0),
    )
}

pub fn set_cursor(conn: &Connection, snapshot: i64, cursor: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE audience_snapshots SET cursor = ?1 WHERE id = ?2 AND state = 'open'",
        params![cursor, snapshot],
    )?;
    Ok(())
}

pub fn latest_complete(conn: &Connection, key: Key, kind: &str) -> rusqlite::Result<Option<Snap>> {
    conn.query_row(
        &format!("SELECT {SNAP} FROM audience_snapshots WHERE platform = ?1 AND account = ?2 AND kind = ?3 AND state = 'complete' ORDER BY id DESC LIMIT 1"),
        params![key.platform, key.account, kind],
        snap,
    )
    .optional()
}

pub fn complete_snapshots(conn: &Connection, key: Key) -> rusqlite::Result<Vec<Snap>> {
    conn.prepare(&format!("SELECT {SNAP} FROM audience_snapshots WHERE platform = ?1 AND account = ?2 AND state = 'complete' ORDER BY id"))?
        .query_map(params![key.platform, key.account], snap)?
        .collect()
}

fn open(conn: &Connection, key: Key) -> rusqlite::Result<Vec<Snap>> {
    conn.prepare(&format!("SELECT {SNAP} FROM audience_snapshots WHERE platform = ?1 AND account = ?2 AND state = 'open' ORDER BY id"))?
        .query_map(params![key.platform, key.account], snap)?
        .collect()
}

pub struct Plan {
    pub open: Vec<Snap>,
    pub accepted: Vec<Step>,
}

#[cfg(test)]
pub fn plan(
    conn: &mut Connection,
    key: Key,
    now: i64,
    restart: bool,
    accept_short: bool,
    expected: [Option<i64>; 2],
) -> rusqlite::Result<Plan> {
    plan_kinds(conn, key, &KINDS, now, restart, accept_short, &expected)
}

pub fn plan_kinds(
    conn: &mut Connection,
    key: Key,
    kinds: &[&str],
    now: i64,
    restart: bool,
    accept_short: bool,
    expected: &[Option<i64>],
) -> rusqlite::Result<Plan> {
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE audience_snapshots SET state = 'abandoned', finished = ?1
         WHERE platform = ?2 AND account = ?3 AND state = 'open' AND (?4 OR started < ?5)",
        params![now, key.platform, key.account, restart, now - RESUME_WINDOW],
    )?;
    tx.execute(
        "DELETE FROM audience_members WHERE snapshot_id IN
         (SELECT id FROM audience_snapshots WHERE platform = ?1 AND account = ?2 AND state = 'abandoned')",
        params![key.platform, key.account],
    )?;
    let mut accepted = Vec::new();
    if accept_short && !restart {
        for kind in kinds.iter().copied() {
            if let Some(s) = latest(&tx, key, kind)? {
                if s.state == "short" && s.finished.is_some_and(|f| f >= now - RESUME_WINDOW) {
                    accepted.push(finalize(&tx, key, s.id)?);
                }
            }
        }
    }
    if open(&tx, key)?.is_empty() {
        for kind in kinds.iter().copied() {
            let fresh = !restart
                && latest(&tx, key, kind)?.is_some_and(|s| {
                    s.state == "complete" && s.finished.is_some_and(|f| f >= now - FRESH)
                });
            if !fresh {
                tx.execute(
                    "INSERT INTO audience_snapshots (platform, account, kind, started) VALUES (?1, ?2, ?3, ?4)",
                    params![key.platform, key.account, kind, now],
                )?;
            }
        }
    }
    for (kind, n) in kinds.iter().zip(expected) {
        if n.is_some() {
            tx.execute(
                "UPDATE audience_snapshots SET expected = ?1 WHERE platform = ?2 AND account = ?3 AND kind = ?4 AND state = 'open'",
                params![n, key.platform, key.account, kind],
            )?;
        }
    }
    let open = open(&tx, key)?;
    tx.commit()?;
    Ok(Plan { open, accepted })
}

pub fn is_short(count: i64, expected: Option<i64>) -> bool {
    expected.is_some_and(|e| count + (e * 10 / 100).max(5) < e)
}

pub fn add_page(
    conn: &mut Connection,
    key: Key,
    snapshot: i64,
    members: &[Member],
    next: Option<&str>,
    now: i64,
    accept_short: bool,
) -> rusqlite::Result<Step> {
    store_page(
        conn,
        key,
        snapshot,
        members,
        now,
        accept_short,
        |_, added, _| next.filter(|_| added > 0).map(String::from),
    )
}

pub fn store_page(
    conn: &mut Connection,
    key: Key,
    snapshot: i64,
    members: &[Member],
    now: i64,
    accept_short: bool,
    next: impl FnOnce(i64, i64, Option<i64>) -> Option<String>,
) -> rusqlite::Result<Step> {
    let tx = conn.transaction()?;
    let (count, expected): (i64, Option<i64>) = tx.query_row(
        "SELECT count, expected FROM audience_snapshots WHERE id = ?1 AND state = 'open'",
        [snapshot],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let mut added = 0i64;
    {
        let mut ins = tx.prepare(&format!(
            "INSERT OR IGNORE INTO audience_members (snapshot_id, position, {COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
        ))?;
        for m in members {
            added += ins.execute(params![
                snapshot,
                count + added,
                m.id,
                m.handle,
                m.name,
                m.avatar,
                m.verified,
                m.private,
                m.followers,
                m.following,
                m.posts,
                m.bio
            ])? as i64;
        }
    }
    let total = count + added;
    let next = next(total, added, expected);
    let ended = next.is_none();
    tx.execute(
        "UPDATE audience_snapshots SET count = ?1, cursor = ?2 WHERE id = ?3",
        params![total, next, snapshot],
    )?;
    let step = if !ended {
        Step::More
    } else {
        tx.execute(
            "UPDATE audience_snapshots SET finished = ?1, state = 'short' WHERE id = ?2",
            params![now, snapshot],
        )?;
        if !accept_short && is_short(total, expected) {
            Step::Short
        } else {
            finalize(&tx, key, snapshot)?
        }
    };
    tx.commit()?;
    Ok(step)
}

fn finalize(conn: &Connection, key: Key, snapshot: i64) -> rusqlite::Result<Step> {
    let (kind, finished): (String, i64) = conn.query_row(
        "SELECT kind, finished FROM audience_snapshots WHERE id = ?1",
        [snapshot],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let prev: Option<(i64, i64)> = conn
        .query_row(
            "SELECT id, started FROM audience_snapshots
             WHERE platform = ?1 AND account = ?2 AND kind = ?3 AND state = 'complete' AND id < ?4
             ORDER BY id DESC LIMIT 1",
            params![key.platform, key.account, kind, snapshot],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    conn.execute(
        "UPDATE audience_snapshots SET state = 'complete' WHERE id = ?1",
        [snapshot],
    )?;
    let Some((prev, older)) = prev else {
        conn.execute(
            "UPDATE audience_members SET first_seen = ?1 WHERE snapshot_id = ?2",
            params![finished, snapshot],
        )?;
        return Ok(Step::Complete {
            gained: 0,
            lost: 0,
            first: true,
        });
    };
    conn.execute(
        "UPDATE audience_members SET first_seen = COALESCE(
           (SELECT p.first_seen FROM audience_members p WHERE p.snapshot_id = ?1 AND p.user_id = audience_members.user_id), ?2)
         WHERE snapshot_id = ?3",
        params![prev, finished, snapshot],
    )?;
    let diff = |event: &str, present: i64, absent: i64| {
        conn.execute(
            &format!(
                "INSERT OR IGNORE INTO audience_events (platform, account, kind, event, older, newer, snapshot_id, {COLS})
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, {COLS} FROM audience_members m
                 WHERE m.snapshot_id = ?8 AND NOT EXISTS
                   (SELECT 1 FROM audience_members o WHERE o.snapshot_id = ?9 AND o.user_id = m.user_id)
                 ORDER BY m.position"
            ),
            params![key.platform, key.account, kind, event, older, finished, snapshot, present, absent],
        )
    };
    let gained = diff("gained", snapshot, prev)?;
    let lost = diff("lost", prev, snapshot)?;
    conn.execute(
        "DELETE FROM audience_members WHERE snapshot_id IN
         (SELECT id FROM audience_snapshots WHERE platform = ?1 AND account = ?2 AND kind = ?3 AND id < ?4 AND id != ?5 AND state != 'export')",
        params![key.platform, key.account, kind, snapshot, prev],
    )?;
    Ok(Step::Complete {
        gained,
        lost,
        first: false,
    })
}

pub fn members(conn: &Connection, snapshot: i64) -> rusqlite::Result<Vec<Member>> {
    conn.prepare(&format!(
        "SELECT {COLS}, first_seen FROM audience_members WHERE snapshot_id = ?1 ORDER BY position"
    ))?
    .query_map([snapshot], |r| {
        let mut m = member_at(r, 0)?;
        m.first_seen = r.get(10)?;
        Ok(m)
    })?
    .collect()
}

pub fn export_members(conn: &Connection, snapshot: i64) -> rusqlite::Result<Vec<Member>> {
    let mut list = members(conn, snapshot)?;
    for m in &mut list {
        m.followed_at = m.first_seen.take();
    }
    Ok(list)
}

pub fn events(conn: &Connection, key: Key, kind: &str) -> rusqlite::Result<Vec<EventRow>> {
    conn.prepare(&format!(
        "SELECT id, event, older, newer, gone, {COLS} FROM audience_events
         WHERE platform = ?1 AND account = ?2 AND kind = ?3 ORDER BY newer DESC, id"
    ))?
    .query_map(params![key.platform, key.account, kind], |r| {
        Ok(EventRow {
            id: r.get(0)?,
            event: r.get(1)?,
            older: r.get(2)?,
            newer: r.get(3)?,
            gone: r.get(4)?,
            member: member_at(r, 5)?,
        })
    })?
    .collect()
}

pub fn set_gone(conn: &Connection, event: i64, gone: bool, now: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE audience_events SET gone = ?1, checked = ?2 WHERE id = ?3",
        params![gone, now, event],
    )?;
    Ok(())
}

pub fn marks(conn: &Connection, key: Key, mark: &str) -> rusqlite::Result<HashMap<String, String>> {
    conn.prepare(
        "SELECT user_id, handle FROM audience_marks WHERE platform = ?1 AND account = ?2 AND mark = ?3",
    )?
    .query_map(params![key.platform, key.account, mark], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?
    .collect()
}

fn known(conn: &Connection, key: Key, handle: &str) -> rusqlite::Result<Option<(String, String)>> {
    let found = conn
        .query_row(
            "SELECT m.user_id, m.handle FROM audience_members m JOIN audience_snapshots s ON s.id = m.snapshot_id
             WHERE s.platform = ?1 AND s.account = ?2 AND lower(m.handle) = ?3 ORDER BY s.state = 'export', s.id DESC LIMIT 1",
            params![key.platform, key.account, handle],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if found.is_some() {
        return Ok(found);
    }
    conn.query_row(
        "SELECT user_id, handle FROM audience_events
         WHERE platform = ?1 AND account = ?2 AND lower(handle) = ?3 ORDER BY id DESC LIMIT 1",
        params![key.platform, key.account, handle],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

pub fn set_marks(
    conn: &mut Connection,
    key: Key,
    mark: &str,
    handles: &[String],
    remove: bool,
    now: i64,
) -> rusqlite::Result<(usize, Vec<String>)> {
    let tx = conn.transaction()?;
    let mut changed = 0;
    let mut missing = Vec::new();
    for raw in handles {
        let handle = raw.trim().trim_start_matches('@').to_lowercase();
        if remove {
            let n = tx.execute(
                "DELETE FROM audience_marks WHERE platform = ?1 AND account = ?2 AND mark = ?3 AND lower(handle) = ?4",
                params![key.platform, key.account, mark, handle],
            )?;
            if n == 0 {
                missing.push(raw.clone());
            }
            changed += n;
            continue;
        }
        match known(&tx, key, &handle)? {
            Some((id, shown)) => {
                changed += tx.execute(
                    "INSERT INTO audience_marks (platform, account, mark, user_id, handle, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT (platform, account, mark, user_id) DO UPDATE SET handle = ?5",
                    params![key.platform, key.account, mark, id, shown, now],
                )?;
            }
            None => missing.push(raw.clone()),
        }
    }
    tx.commit()?;
    Ok((changed, missing))
}

pub fn activity(
    conn: &Connection,
    platform: &str,
    now: i64,
) -> rusqlite::Result<HashMap<String, Option<i64>>> {
    conn.prepare(
        "SELECT user_id, last_post FROM audience_activity WHERE platform = ?1 AND checked >= ?2",
    )?
    .query_map(params![platform, now - ACTIVITY_TTL], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?
    .collect()
}

pub fn set_activity(
    conn: &Connection,
    platform: &str,
    user: &str,
    last_post: Option<i64>,
    now: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO audience_activity (platform, user_id, last_post, checked) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (platform, user_id) DO UPDATE SET last_post = ?3, checked = ?4",
        params![platform, user, last_post, now],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::super::store;
    use super::*;

    const K: Key = Key {
        platform: "instagram",
        account: "me",
    };

    fn m(id: &str) -> Member {
        Member {
            id: id.into(),
            handle: format!("user{id}"),
            ..Default::default()
        }
    }

    fn ms(ids: &[&str]) -> Vec<Member> {
        ids.iter().map(|i| m(i)).collect()
    }

    fn snapshot(conn: &mut Connection, kind: &str, ids: &[&str], at: i64) -> Step {
        conn.execute(
            "INSERT INTO audience_snapshots (platform, account, kind, started) VALUES ('instagram', 'me', ?1, ?2)",
            params![kind, at],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        add_page(conn, K, id, &ms(ids), None, at + 10, false).unwrap()
    }

    fn handles(conn: &Connection, event: &str) -> Vec<String> {
        events(conn, K, "followers")
            .unwrap()
            .into_iter()
            .filter(|e| e.event == event)
            .map(|e| e.member.id)
            .collect()
    }

    #[test]
    fn first_snapshot_is_a_baseline_without_events() {
        let mut c = store::memory(SCHEMA);
        let step = snapshot(&mut c, "followers", &["1", "2"], 100);
        assert_eq!(
            step,
            Step::Complete {
                gained: 0,
                lost: 0,
                first: true
            }
        );
        assert!(events(&c, K, "followers").unwrap().is_empty());
        let s = latest_complete(&c, K, "followers").unwrap().unwrap();
        assert_eq!(s.count, 2);
        assert_eq!(members(&c, s.id).unwrap()[0].first_seen, Some(110));
    }

    #[test]
    fn consecutive_complete_snapshots_give_gained_and_lost_with_the_window() {
        let mut c = store::memory(SCHEMA);
        snapshot(&mut c, "followers", &["1", "2", "3"], 100);
        let step = snapshot(&mut c, "followers", &["2", "3", "4"], 200);
        assert_eq!(
            step,
            Step::Complete {
                gained: 1,
                lost: 1,
                first: false
            }
        );
        let ev = events(&c, K, "followers").unwrap();
        assert_eq!(ev.len(), 2);
        assert!(ev.iter().all(|e| e.older == 100 && e.newer == 210));
        assert_eq!(handles(&c, "gained"), vec!["4"]);
        assert_eq!(handles(&c, "lost"), vec!["1"]);
        let s = latest_complete(&c, K, "followers").unwrap().unwrap();
        let seen: Vec<_> = members(&c, s.id)
            .unwrap()
            .into_iter()
            .map(|m| (m.id, m.first_seen))
            .collect();
        assert_eq!(
            seen,
            vec![
                ("2".into(), Some(110)),
                ("3".into(), Some(110)),
                ("4".into(), Some(210))
            ]
        );
    }

    #[test]
    fn kinds_do_not_mix() {
        let mut c = store::memory(SCHEMA);
        snapshot(&mut c, "followers", &["1"], 100);
        snapshot(&mut c, "following", &["9"], 150);
        snapshot(&mut c, "followers", &["1"], 200);
        assert!(events(&c, K, "followers").unwrap().is_empty());
        assert!(events(&c, K, "following").unwrap().is_empty());
    }

    #[test]
    fn an_unfinished_list_is_never_diffed() {
        let mut c = store::memory(SCHEMA);
        snapshot(&mut c, "followers", &["1", "2", "3"], 100);
        let p = plan(&mut c, K, 2000, false, false, [Some(3), None]).unwrap();
        let open = p.open.iter().find(|s| s.kind == "followers").unwrap().id;
        assert_eq!(
            add_page(&mut c, K, open, &ms(&["1"]), Some("c1"), 2005, false).unwrap(),
            Step::More
        );
        assert!(events(&c, K, "followers").unwrap().is_empty());
        let later = 2000 + RESUME_WINDOW + 1;
        let p = plan(&mut c, K, later, false, false, [Some(3), None]).unwrap();
        assert!(p.open.iter().all(|s| s.id != open));
        assert!(events(&c, K, "followers").unwrap().is_empty());
        let again = p.open.iter().find(|s| s.kind == "followers").unwrap().id;
        add_page(&mut c, K, again, &ms(&["1", "2"]), None, later + 5, false).unwrap();
        assert_eq!(handles(&c, "lost"), vec!["3"]);
        assert_eq!(events(&c, K, "followers").unwrap()[0].older, 100);
    }

    #[test]
    fn a_list_much_shorter_than_the_profile_count_is_not_trusted() {
        assert!(!is_short(95, Some(100)));
        assert!(is_short(80, Some(100)));
        assert!(!is_short(3, Some(8)));
        assert!(is_short(2, Some(8)));
        assert!(!is_short(0, None));
        let mut c = store::memory(SCHEMA);
        snapshot(&mut c, "followers", &["1", "2", "3"], 100);
        let p = plan(&mut c, K, 5000, false, false, [Some(40), None]).unwrap();
        let id = p.open.iter().find(|s| s.kind == "followers").unwrap().id;
        assert_eq!(
            add_page(&mut c, K, id, &ms(&["1"]), None, 5010, false).unwrap(),
            Step::Short
        );
        assert!(events(&c, K, "followers").unwrap().is_empty());
        assert_eq!(latest(&c, K, "followers").unwrap().unwrap().state, "short");
        assert_eq!(
            latest_complete(&c, K, "followers").unwrap().unwrap().count,
            3
        );
        let p = plan(&mut c, K, 5020, false, true, [Some(40), None]).unwrap();
        assert_eq!(
            p.accepted,
            vec![Step::Complete {
                gained: 0,
                lost: 2,
                first: false
            }]
        );
        assert_eq!(
            latest_complete(&c, K, "followers").unwrap().unwrap().count,
            1
        );
    }

    #[test]
    fn the_tolerance_for_a_short_list_is_ten_percent() {
        assert!(is_short(198, Some(222)));
        assert!(is_short(198, Some(221)));
        assert!(!is_short(214, Some(222)));
        assert!(!is_short(200, Some(222)));
        assert!(is_short(199, Some(222)));
    }

    #[test]
    fn a_page_can_hand_the_list_over_to_another_cursor_instead_of_ending() {
        let mut c = store::memory(SCHEMA);
        let p = plan(&mut c, K, 1000, false, false, [None, Some(4)]).unwrap();
        let id = p.open.iter().find(|s| s.kind == "following").unwrap().id;
        let step = store_page(
            &mut c,
            K,
            id,
            &ms(&["1", "2"]),
            1001,
            false,
            |total, added, expected| {
                assert_eq!((total, added, expected), (2, 2, Some(4)));
                Some("sweep:0".into())
            },
        )
        .unwrap();
        assert_eq!(step, Step::More);
        assert_eq!(cursor(&c, id).unwrap().as_deref(), Some("sweep:0"));
        let step = store_page(
            &mut c,
            K,
            id,
            &ms(&["2"]),
            1002,
            false,
            |total, added, _| {
                assert_eq!((total, added), (2, 0));
                Some("sweep:1".into())
            },
        )
        .unwrap();
        assert_eq!(step, Step::More);
        set_cursor(&c, id, "sweep:9").unwrap();
        assert_eq!(cursor(&c, id).unwrap().as_deref(), Some("sweep:9"));
        let step =
            store_page(&mut c, K, id, &ms(&["3", "4"]), 1003, false, |_, _, _| None).unwrap();
        assert!(matches!(step, Step::Complete { first: true, .. }));
        assert_eq!(
            latest_complete(&c, K, "following").unwrap().unwrap().count,
            4
        );
    }

    #[test]
    fn an_export_list_is_stored_apart_and_never_diffed_or_pruned() {
        let mut c = store::memory(SCHEMA);
        snapshot(&mut c, "following", &["1", "2"], 100);
        let ex = |h: &str, at: Option<i64>| Member {
            id: format!("@{h}"),
            handle: h.into(),
            followed_at: at,
            ..Default::default()
        };
        replace_export(
            &mut c,
            K,
            "following",
            &[ex("ana", Some(50)), ex("bia", None), ex("ana", Some(9))],
            150,
        )
        .unwrap();
        let e = latest_export(&c, K, "following").unwrap().unwrap();
        assert_eq!(
            (e.state.as_str(), e.count, e.finished),
            ("export", 2, Some(150))
        );
        let list = export_members(&c, e.id).unwrap();
        assert_eq!(list[0].followed_at, Some(50));
        assert_eq!(list[0].first_seen, None);
        assert_eq!(list[1].handle, "bia");
        assert_eq!(
            latest(&c, K, "following").unwrap().unwrap().state,
            "complete"
        );
        assert_eq!(
            latest_account(&c, "instagram").unwrap().as_deref(),
            Some("me")
        );
        snapshot(&mut c, "following", &["1"], 200);
        snapshot(&mut c, "following", &["1", "3"], 300);
        assert!(events(&c, K, "following")
            .unwrap()
            .iter()
            .all(|e| !e.member.id.starts_with('@')));
        assert_eq!(export_members(&c, e.id).unwrap().len(), 2);
        replace_export(&mut c, K, "following", &[ex("cris", None)], 400).unwrap();
        let again = latest_export(&c, K, "following").unwrap().unwrap();
        assert_eq!(again.count, 1);
        assert!(export_members(&c, e.id).unwrap().is_empty());
        assert_eq!(
            site_ids(&c, K).unwrap().get("user1").map(String::as_str),
            Some("1")
        );
        assert!(!site_ids(&c, K).unwrap().contains_key("cris"));
        let (n, _) = set_marks(&mut c, K, "keep", &["cris".into()], false, 5).unwrap();
        assert_eq!(n, 1);
        assert!(marks(&c, K, "keep").unwrap().contains_key("@cris"));
    }

    #[test]
    fn a_refollow_is_gained_again_and_restarts_first_seen() {
        let mut c = store::memory(SCHEMA);
        snapshot(&mut c, "followers", &["1", "2"], 100);
        snapshot(&mut c, "followers", &["2"], 200);
        snapshot(&mut c, "followers", &["2", "1"], 300);
        assert_eq!(handles(&c, "lost"), vec!["1"]);
        assert_eq!(handles(&c, "gained"), vec!["1"]);
        let s = latest_complete(&c, K, "followers").unwrap().unwrap();
        let one = members(&c, s.id)
            .unwrap()
            .into_iter()
            .find(|m| m.id == "1")
            .unwrap();
        assert_eq!(one.first_seen, Some(310));
    }

    #[test]
    fn old_member_rows_are_pruned_but_counts_stay() {
        let mut c = store::memory(SCHEMA);
        snapshot(&mut c, "followers", &["1", "2"], 100);
        snapshot(&mut c, "followers", &["1", "2", "3"], 200);
        snapshot(&mut c, "followers", &["1"], 300);
        let snaps = complete_snapshots(&c, K).unwrap();
        assert_eq!(
            snaps.iter().map(|s| s.count).collect::<Vec<_>>(),
            vec![2, 3, 1]
        );
        assert!(members(&c, snaps[0].id).unwrap().is_empty());
        assert_eq!(members(&c, snaps[1].id).unwrap().len(), 3);
    }

    #[test]
    fn resume_continues_the_open_snapshot_from_its_cursor() {
        let mut c = store::memory(SCHEMA);
        let p = plan(&mut c, K, 1000, false, false, [Some(4), Some(1)]).unwrap();
        assert_eq!(
            p.open.iter().map(|s| s.kind.as_str()).collect::<Vec<_>>(),
            vec!["followers", "following"]
        );
        let (fo, fg) = (p.open[0].id, p.open[1].id);
        assert_eq!(p.open[0].expected, Some(4));
        assert_eq!(
            add_page(&mut c, K, fo, &ms(&["1", "2"]), Some("c1"), 1001, false).unwrap(),
            Step::More
        );
        let p = plan(&mut c, K, 1100, false, false, [Some(4), Some(1)]).unwrap();
        assert_eq!(p.open[0].id, fo);
        assert_eq!(p.open[0].cursor.as_deref(), Some("c1"));
        assert_eq!(p.open[0].count, 2);
        assert!(matches!(
            add_page(&mut c, K, fo, &ms(&["2", "3", "4"]), None, 1101, false).unwrap(),
            Step::Complete { first: true, .. }
        ));
        assert_eq!(
            latest_complete(&c, K, "followers").unwrap().unwrap().count,
            4
        );
        let p = plan(&mut c, K, 1200, false, false, [Some(4), Some(1)]).unwrap();
        assert_eq!(p.open.iter().map(|s| s.id).collect::<Vec<_>>(), vec![fg]);
        add_page(&mut c, K, fg, &ms(&["9"]), None, 1201, false).unwrap();
        let p = plan(&mut c, K, 1300, false, false, [Some(4), Some(1)]).unwrap();
        assert!(p.open.is_empty());
        let p = plan(&mut c, K, 1300, true, false, [Some(4), Some(1)]).unwrap();
        assert_eq!(p.open.len(), 2);
        let p = plan(&mut c, K, 1300 + FRESH + 300, false, false, [None, None]).unwrap();
        assert_eq!(p.open.len(), 2);
        assert!(p.open.iter().all(|s| s.started == 1300));
    }

    #[test]
    fn a_repository_keeps_one_list_of_stargazers_with_its_own_diff() {
        let mut c = store::memory(SCHEMA);
        let repo = Key {
            platform: "github",
            account: "ana/tool",
        };
        let p = plan_kinds(&mut c, repo, &KINDS[..1], 1000, false, false, &[Some(3)]).unwrap();
        assert_eq!(p.open.len(), 1);
        assert_eq!(p.open[0].kind, "followers");
        assert_eq!(p.open[0].expected, Some(3));
        assert!(matches!(
            add_page(
                &mut c,
                repo,
                p.open[0].id,
                &ms(&["1", "2", "3"]),
                None,
                1001,
                false
            )
            .unwrap(),
            Step::Complete { first: true, .. }
        ));
        let p = plan_kinds(&mut c, repo, &KINDS[..1], 9000, false, false, &[Some(3)]).unwrap();
        assert_eq!(
            add_page(
                &mut c,
                repo,
                p.open[0].id,
                &ms(&["2", "3", "4"]),
                None,
                9001,
                false
            )
            .unwrap(),
            Step::Complete {
                gained: 1,
                lost: 1,
                first: false
            }
        );
        let ev = events(&c, repo, "followers").unwrap();
        let of = |kind: &str| -> Vec<&str> {
            ev.iter()
                .filter(|e| e.event == kind)
                .map(|e| e.member.id.as_str())
                .collect()
        };
        assert_eq!(of("gained"), vec!["4"]);
        assert_eq!(of("lost"), vec!["1"]);
        assert_eq!(ev[0].older, 1000);
        assert_eq!(ev[0].newer, 9001);
        assert!(events(&c, K, "followers").unwrap().is_empty());
        assert!(latest(&c, repo, "following").unwrap().is_none());
    }

    #[test]
    fn a_page_that_repeats_only_known_accounts_ends_the_list() {
        let mut c = store::memory(SCHEMA);
        let p = plan(&mut c, K, 1000, false, false, [None, None]).unwrap();
        let id = p.open[0].id;
        add_page(&mut c, K, id, &ms(&["1", "2"]), Some("a"), 1001, false).unwrap();
        assert!(matches!(
            add_page(&mut c, K, id, &ms(&["1", "2"]), Some("b"), 1002, false).unwrap(),
            Step::Complete { .. }
        ));
    }

    #[test]
    fn marks_need_a_known_account_and_can_be_removed() {
        let mut c = store::memory(SCHEMA);
        snapshot(&mut c, "followers", &["1", "2"], 100);
        let (n, missing) = set_marks(
            &mut c,
            K,
            "keep",
            &["@User1".into(), "ghost".into()],
            false,
            5,
        )
        .unwrap();
        assert_eq!(n, 1);
        assert_eq!(missing, vec!["ghost".to_string()]);
        assert_eq!(
            marks(&c, K, "keep").unwrap().get("1").map(String::as_str),
            Some("user1")
        );
        assert!(marks(&c, K, "hidden").unwrap().is_empty());
        let (n, _) = set_marks(&mut c, K, "keep", &["user1".into()], true, 6).unwrap();
        assert_eq!(n, 1);
        assert!(marks(&c, K, "keep").unwrap().is_empty());
    }

    #[test]
    fn activity_expires() {
        let c = store::memory(SCHEMA);
        set_activity(&c, "x", "1", Some(50), 1000).unwrap();
        set_activity(&c, "x", "2", None, 1000).unwrap();
        let a = activity(&c, "x", 1001).unwrap();
        assert_eq!(a.get("1"), Some(&Some(50)));
        assert_eq!(a.get("2"), Some(&None));
        assert!(activity(&c, "x", 1001 + ACTIVITY_TTL).unwrap().is_empty());
    }

    #[test]
    fn reads_a_member_from_a_profile() {
        let v = serde_json::json!({ "id": "7", "handle": "ana", "name": "Ana", "avatar": "a.jpg", "verified": true, "private": false, "followers": 10, "bio": "oi" });
        let m = Member::from_profile(&v).unwrap();
        assert_eq!(
            (m.id.as_str(), m.followers, m.following),
            ("7", Some(10), None)
        );
        assert!(Member::from_profile(&serde_json::json!({ "handle": "x" })).is_none());
    }
}
