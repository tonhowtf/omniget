use std::collections::BTreeSet;

use rusqlite::{params, Connection};
use serde_json::{json, Value};

use super::super::args::opt_str;
use super::super::model::Platform;
use super::super::{monitor, schedule, store};
use super::{fail, invalid};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS social_notify (
  platform TEXT NOT NULL, scope TEXT NOT NULL, target TEXT NOT NULL DEFAULT '', enabled INTEGER NOT NULL,
  PRIMARY KEY (platform, scope, target)) WITHOUT ROWID;
";

pub const MAX_PER_CHECK: usize = 5;
const SURGE: &str = "surge";
const UNFOLLOWERS: &str = "unfollowers";
const MONITOR: &str = "monitor";

#[derive(Debug, Default, PartialEq)]
pub struct Settings {
    pub surge: bool,
    pub unfollowers: bool,
    pub monitors: BTreeSet<i64>,
}

pub fn load(conn: &Connection, platform: &str) -> rusqlite::Result<Settings> {
    let mut st = conn
        .prepare("SELECT scope, target FROM social_notify WHERE platform = ?1 AND enabled = 1")?;
    let rows = st.query_map([platform], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut out = Settings::default();
    for row in rows {
        let (scope, target) = row?;
        match scope.as_str() {
            SURGE => out.surge = true,
            UNFOLLOWERS => out.unfollowers = true,
            _ => {
                if let Ok(id) = target.parse() {
                    out.monitors.insert(id);
                }
            }
        }
    }
    Ok(out)
}

pub fn set(
    conn: &Connection,
    platform: &str,
    scope: &str,
    target: &str,
    on: bool,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO social_notify (platform, scope, target, enabled) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (platform, scope, target) DO UPDATE SET enabled = ?4",
        params![platform, scope, target, on],
    )?;
    Ok(())
}

pub fn check_notices(label: &str, s: &Settings, monitors: &[Value]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for m in monitors {
        let name = m["name"].as_str().unwrap_or_default();
        let new = m["new"].as_u64().unwrap_or(0);
        let surge = &m["surge"];
        if s.surge && surge["surge"] == true {
            out.push((
                format!("Surge on {label}: {name}"),
                format!(
                    "{} posts today against an average of {} a day; {new} new since the last check.",
                    surge["today"], surge["trailing_average"]
                ),
            ));
        } else if new > 0 && m["id"].as_i64().is_some_and(|id| s.monitors.contains(&id)) {
            out.push((
                format!("{new} new on {label}: {name}"),
                "Open the Monitor tab to read them.".to_string(),
            ));
        }
    }
    if out.len() > MAX_PER_CHECK {
        let rest = out.len() - (MAX_PER_CHECK - 1);
        out.truncate(MAX_PER_CHECK - 1);
        out.push((
            format!("{rest} more monitors on {label} have news"),
            "Open the Monitor tab to read them.".to_string(),
        ));
    }
    out
}

pub fn sync_notice(label: &str, s: &Settings, sync: &Value) -> Option<(String, String)> {
    let lost = sync["changes"]["lost"].as_u64().unwrap_or(0);
    if !s.unfollowers || lost == 0 || !sync["repo"].is_null() {
        return None;
    }
    Some((
        format!(
            "{lost} new unfollower{} on {label}",
            if lost == 1 { "" } else { "s" }
        ),
        format!(
            "Since the previous sync of @{}: {} gained, {lost} lost. Open the Audience tab to see who.",
            sync["account"].as_str().unwrap_or_default(),
            sync["changes"]["gained"].as_u64().unwrap_or(0)
        ),
    ))
}

fn settings(p: Platform) -> Settings {
    store::with(SCHEMA, |c| load(c, p.id())).unwrap_or_default()
}

pub fn after_check(p: Platform, monitors: &[Value]) -> usize {
    let notices = check_notices(p.label(), &settings(p), monitors);
    let n = notices.len();
    for (title, body) in notices {
        schedule::notify(title, body);
    }
    n
}

pub fn after_sync(p: Platform, sync: &Value) {
    if let Some((title, body)) = sync_notice(p.label(), &settings(p), sync) {
        schedule::notify(title, body);
    }
}

fn state(p: Platform, s: &Settings) -> Value {
    json!({
        "platform": p.id(),
        "surge": s.surge,
        "unfollowers": s.unfollowers,
        "monitors": s.monitors,
        "channels": { "app": true, "slack": false, "email": false },
        "when": "A notification is raised by this app only when social_monitor_check or an audience sync runs and finds it; nothing is sent while the app is closed. A failed or missed scheduled post always notifies.",
    })
}

fn flag(a: &Value, key: &str) -> Result<Option<bool>, String> {
    match &a[key] {
        Value::Null => Ok(None),
        Value::Bool(b) => Ok(Some(*b)),
        _ => Err(invalid(format!("{key} must be true or false"))),
    }
}

pub fn tool(p: Platform, a: &Value) -> Result<Value, String> {
    match opt_str(a, "op").as_deref() {
        Some("get") => {}
        Some("set") => {
            let surge = flag(a, "surge")?;
            let unfollowers = flag(a, "unfollowers")?;
            let new_hits = flag(a, "new_hits")?;
            let id = match &a["monitor"] {
                Value::Null => None,
                v => Some(
                    v.as_i64()
                        .filter(|n| *n > 0)
                        .ok_or_else(|| invalid("monitor must be the id of a monitor"))?,
                ),
            };
            if id.is_some() != new_hits.is_some() {
                return Err(invalid("monitor and new_hits go together"));
            }
            if surge.is_none() && unfollowers.is_none() && id.is_none() {
                return Err(invalid(
                    "op=set needs surge, unfollowers, or monitor with new_hits",
                ));
            }
            if let Some(id) = id {
                let found = store::with(monitor::db::SCHEMA, |c| monitor::db::get(c, p.id(), id))?;
                if found.is_none() {
                    return Err(fail(
                        "MONITOR_NOT_FOUND",
                        format!("no monitor {id} on {}", p.label()),
                        false,
                        "Call social_monitors to list the ids.",
                    ));
                }
            }
            store::with(SCHEMA, |c| {
                if let Some(on) = surge {
                    set(c, p.id(), SURGE, "", on)?;
                }
                if let Some(on) = unfollowers {
                    set(c, p.id(), UNFOLLOWERS, "", on)?;
                }
                if let (Some(id), Some(on)) = (id, new_hits) {
                    set(c, p.id(), MONITOR, &id.to_string(), on)?;
                }
                Ok(())
            })?;
        }
        _ => return Err(invalid("op must be get or set")),
    }
    Ok(state(p, &store::with(SCHEMA, |c| load(c, p.id()))?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_stored_per_network_and_per_monitor() {
        let c = store::memory(SCHEMA);
        assert_eq!(load(&c, "x").unwrap(), Settings::default());
        set(&c, "x", SURGE, "", true).unwrap();
        set(&c, "x", MONITOR, "7", true).unwrap();
        set(&c, "x", MONITOR, "9", true).unwrap();
        set(&c, "x", MONITOR, "9", false).unwrap();
        set(&c, "instagram", UNFOLLOWERS, "", true).unwrap();
        let x = load(&c, "x").unwrap();
        assert!(x.surge && !x.unfollowers);
        assert_eq!(x.monitors.into_iter().collect::<Vec<_>>(), vec![7]);
        let ig = load(&c, "instagram").unwrap();
        assert!(ig.unfollowers && !ig.surge && ig.monitors.is_empty());
        set(&c, "x", SURGE, "", false).unwrap();
        assert!(!load(&c, "x").unwrap().surge);
    }

    #[test]
    fn a_check_notifies_only_what_was_asked_for() {
        let monitors = vec![
            json!({ "id": 1, "name": "rust", "new": 4, "surge": { "surge": true, "today": 30, "trailing_average": 6.5 } }),
            json!({ "id": 2, "name": "svelte", "new": 3, "surge": { "surge": false } }),
            json!({ "id": 3, "name": "quiet", "new": 0, "surge": { "surge": false } }),
        ];
        assert!(check_notices("Short Posts", &Settings::default(), &monitors).is_empty());
        let surge_only = Settings {
            surge: true,
            ..Default::default()
        };
        let n = check_notices("Short Posts", &surge_only, &monitors);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].0, "Surge on Short Posts: rust");
        assert!(n[0].1.contains("30 posts today") && n[0].1.contains("6.5"));
        let marked = Settings {
            monitors: [1, 2, 3].into(),
            ..Default::default()
        };
        let n = check_notices("Short Posts", &marked, &monitors);
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].0, "4 new on Short Posts: rust");
        assert_eq!(n[1].0, "3 new on Short Posts: svelte");
        let both = Settings {
            surge: true,
            monitors: [1].into(),
            ..Default::default()
        };
        assert_eq!(check_notices("Short Posts", &both, &monitors).len(), 1);
    }

    #[test]
    fn many_notices_are_folded_into_one() {
        let monitors: Vec<Value> = (1..=8)
            .map(|i| json!({ "id": i, "name": format!("m{i}"), "new": 1, "surge": {} }))
            .collect();
        let all = Settings {
            monitors: (1..=8).collect(),
            ..Default::default()
        };
        let n = check_notices("Clip Feed", &all, &monitors);
        assert_eq!(n.len(), MAX_PER_CHECK);
        assert_eq!(n[4].0, "4 more monitors on Clip Feed have news");
    }

    #[test]
    fn a_sync_notifies_new_unfollowers_only_when_asked_and_only_for_follower_lists() {
        let on = Settings {
            unfollowers: true,
            ..Default::default()
        };
        let sync = json!({ "account": "ana", "changes": { "gained": 2, "lost": 1 } });
        assert_eq!(sync_notice("Photo Feed", &Settings::default(), &sync), None);
        let (title, body) = sync_notice("Photo Feed", &on, &sync).unwrap();
        assert_eq!(title, "1 new unfollower on Photo Feed");
        assert!(body.contains("@ana") && body.contains("2 gained, 1 lost"));
        let many = json!({ "account": "ana", "changes": { "gained": 0, "lost": 3 } });
        assert_eq!(
            sync_notice("Short Posts", &on, &many).unwrap().0,
            "3 new unfollowers on Short Posts"
        );
        for quiet in [
            json!({ "account": "ana", "changes": { "gained": 5, "lost": 0 } }),
            json!({ "account": "ana", "changes": null }),
            json!({ "account": "ana/tool", "repo": "ana/tool", "changes": { "gained": 0, "lost": 2 } }),
        ] {
            assert_eq!(sync_notice("Short Posts", &on, &quiet), None, "{quiet}");
        }
    }

    #[test]
    fn the_tool_validates_before_storing() {
        for bad in [
            json!({}),
            json!({ "op": "nope" }),
            json!({ "op": "set" }),
            json!({ "op": "set", "surge": "yes" }),
            json!({ "op": "set", "monitor": 3 }),
            json!({ "op": "set", "new_hits": true }),
            json!({ "op": "set", "monitor": 0, "new_hits": true }),
        ] {
            assert!(
                tool(Platform::ShortPost, &bad)
                    .unwrap_err()
                    .starts_with("INVALID_ARGUMENTS"),
                "{bad}"
            );
        }
        let s = state(
            Platform::ShortPost,
            &Settings {
                surge: true,
                monitors: [4].into(),
                ..Default::default()
            },
        );
        assert_eq!(s["surge"], true);
        assert_eq!(s["monitors"], json!([4]));
        assert_eq!(s["channels"]["slack"], false);
        assert_eq!(s["channels"]["email"], false);
    }
}
