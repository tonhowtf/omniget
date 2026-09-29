//! What the icon and the panel show, computed from the engine's snapshot.
//! Pure functions with tests; nothing here reads a file or the network.
//!
//! The forecast ("at this pace, limit at 16:40") lives in
//! `limits_strip::pace`, shared with the strip and its alerts.

use crate::limits_strip::engine::{Ring, Snapshot};
pub use crate::limits_strip::pace::runs_out_at;
use crate::limits_strip::LimitWindow;
use serde::Serialize;

use super::prefs::{Shows, TrayPrefs};

/// `ok` below the warning threshold, then `warn`, then `critical`.
pub fn level(used_pct: f64, warn: u8, critical: u8) -> &'static str {
    if used_pct >= critical as f64 {
        "critical"
    } else if used_pct >= warn as f64 {
        "warn"
    } else {
        "ok"
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Win {
    pub id: String,
    pub label: String,
    /// 0..1+
    pub used: f64,
    pub resets_at: Option<i64>,
    pub runs_out_at: Option<i64>,
    /// Share of the window's time gone (where "on pace" sits), when known.
    pub expected: Option<f64>,
    pub level: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Account {
    pub id: String,
    /// `claude` / `codex`: which CLI the ring belongs to.
    pub cli: String,
    pub name: String,
    pub email: Option<String>,
    pub plan: Option<String>,
    pub status: &'static str,
    pub message: Option<String>,
    pub session: Option<Win>,
    pub weekly: Option<Win>,
    pub others: Vec<Win>,
    pub read_at: Option<i64>,
    pub spend_today: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct View {
    pub accounts: Vec<Account>,
    /// What the icon shows right now: percent used and its level.
    pub headline: Option<(f64, &'static str)>,
    pub shows: &'static str,
    pub warn: u8,
    pub critical: u8,
}

fn is_session(w: &LimitWindow) -> bool {
    w.id == "session" || w.span_ms.map(|s| s <= 12 * 3_600_000).unwrap_or(false)
}

fn is_weekly(w: &LimitWindow) -> bool {
    w.group.is_none()
        && (w.id == "weekly_all"
            || w.id == "weekly"
            || (w
                .span_ms
                .map(|s| s >= 6 * 86_400_000 && s <= 8 * 86_400_000)
                .unwrap_or(false)
                && !w.id.starts_with("weekly_")))
}

fn win(w: &LimitWindow, prefs: &TrayPrefs, now: i64) -> Option<Win> {
    let used = w.used? as f64;
    Some(Win {
        id: w.id.clone(),
        label: w.label.clone(),
        used,
        resets_at: w.resets_at,
        runs_out_at: match (w.resets_at, w.span_ms) {
            (Some(r), Some(s)) => runs_out_at(used, r, s, now),
            _ => None,
        },
        expected: crate::limits_strip::pace::of(w, now).map(|p| p.expected),
        level: level(used * 100.0, prefs.warn, prefs.critical),
    })
}

pub fn account(ring: &Ring, prefs: &TrayPrefs, now: i64) -> Account {
    let windows = ring
        .reading
        .as_ref()
        .map(|r| r.windows.as_slice())
        .unwrap_or(&[]);
    let session_w = windows.iter().find(|w| w.group.is_none() && is_session(w));
    let weekly_w = windows.iter().find(|w| is_weekly(w));
    let others = windows
        .iter()
        .filter(|w| Some(*w) != session_w && Some(*w) != weekly_w)
        .filter_map(|w| win(w, prefs, now))
        .collect();
    let tag = ring.account.clone().unwrap_or_default();
    let cli = ring.id.split(':').next().unwrap_or(&ring.id).to_string();
    Account {
        id: ring.id.clone(),
        name: tag
            .label
            .clone()
            .or_else(|| tag.email.clone())
            .unwrap_or_else(|| ring.label.clone()),
        cli,
        email: tag
            .email
            .or_else(|| ring.reading.as_ref().and_then(|r| r.account.clone())),
        plan: ring.reading.as_ref().and_then(|r| r.plan.clone()),
        status: ring.status,
        message: ring.message.clone(),
        session: session_w.and_then(|w| win(w, prefs, now)),
        weekly: weekly_w.and_then(|w| win(w, prefs, now)),
        others,
        read_at: ring.read_at,
        spend_today: None,
    }
}

/// The number on the icon: the fullest matching window across the accounts.
pub fn headline(accounts: &[Account], prefs: &TrayPrefs) -> Option<(f64, &'static str)> {
    let pick = |a: &Account| -> Vec<f64> {
        match prefs.shows {
            Shows::Session => a.session.iter().map(|w| w.used).collect(),
            Shows::Weekly => a.weekly.iter().map(|w| w.used).collect(),
            Shows::Busiest => a
                .session
                .iter()
                .chain(a.weekly.iter())
                .chain(a.others.iter())
                .map(|w| w.used)
                .collect(),
        }
    };
    let top = accounts
        .iter()
        .flat_map(pick)
        .fold(None::<f64>, |m, u| Some(m.map_or(u, |m| m.max(u))))?;
    let pct = (top * 100.0).max(0.0);
    Some((pct, level(pct, prefs.warn, prefs.critical)))
}

pub fn build(snap: &Snapshot, prefs: &TrayPrefs, now: i64) -> View {
    let mut accounts: Vec<Account> = prefs
        .accounts
        .iter()
        .filter_map(|id| snap.rings.iter().find(|r| &r.id == id))
        .map(|r| account(r, prefs, now))
        .collect();
    // Rings the engine has not listed yet keep the user's order at the end.
    accounts.dedup_by(|a, b| a.id == b.id);
    View {
        headline: headline(&accounts, prefs),
        accounts,
        shows: prefs.shows.as_str(),
        warn: prefs.warn,
        critical: prefs.critical,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits_strip::activity::Activity;
    use crate::limits_strip::{AccountTag, Reading, DAY_MS, HOUR_MS};

    const NOW: i64 = 1_800_000_000_000;

    fn ring(id: &str, windows: Vec<LimitWindow>) -> Ring {
        Ring {
            id: id.into(),
            label: "Claude Code".into(),
            local: false,
            beta: false,
            account: Some(AccountTag {
                label: Some("work".into()),
                email: Some("me@example.com".into()),
            }),
            status: "ok",
            message: None,
            reading: Some(Reading {
                windows,
                ..Default::default()
            }),
            read_at: Some(NOW),
            next_at: None,
            activity: Activity::default(),
        }
    }

    #[test]
    fn levels_follow_the_thresholds() {
        assert_eq!(level(79.9, 80, 95), "ok");
        assert_eq!(level(80.0, 80, 95), "warn");
        assert_eq!(level(95.0, 80, 95), "critical");
    }

    #[test]
    fn session_and_weekly_are_told_apart() {
        let r = ring(
            "claude:work",
            vec![
                LimitWindow::percent("session", "Current session", 37.0)
                    .resets(Some(NOW + HOUR_MS))
                    .span(5 * HOUR_MS),
                LimitWindow::percent("weekly_all", "Weekly (all models)", 52.0)
                    .resets(Some(NOW + 2 * DAY_MS))
                    .span(7 * DAY_MS),
                LimitWindow::percent("weekly_opus", "Weekly (Opus)", 90.0)
                    .resets(Some(NOW + 2 * DAY_MS))
                    .span(7 * DAY_MS),
            ],
        );
        let prefs = TrayPrefs::default();
        let a = account(&r, &prefs, NOW);
        assert_eq!(a.session.as_ref().unwrap().id, "session");
        assert_eq!(a.weekly.as_ref().unwrap().id, "weekly_all");
        assert_eq!(a.others.len(), 1);
        assert_eq!(a.others[0].level, "warn");
        assert_eq!(a.name, "work");
        assert_eq!(a.cli, "claude");
    }

    #[test]
    fn the_headline_follows_what_the_icon_shows() {
        let snap = Snapshot {
            open: false,
            edge: Default::default(),
            look: Default::default(),
            rings: vec![
                ring(
                    "claude",
                    vec![
                        LimitWindow::percent("session", "S", 20.0).span(5 * HOUR_MS),
                        LimitWindow::percent("weekly_all", "W", 96.0).span(7 * DAY_MS),
                    ],
                ),
                ring(
                    "claude:work",
                    vec![LimitWindow::percent("session", "S", 81.0).span(5 * HOUR_MS)],
                ),
                ring("codex", vec![]),
            ],
        };
        let mut prefs = TrayPrefs {
            accounts: vec!["claude:work".into(), "claude".into()],
            ..Default::default()
        };
        let v = build(&snap, &prefs, NOW);
        assert_eq!(v.accounts.len(), 2, "codex is not listed");
        assert_eq!(v.accounts[0].id, "claude:work", "the user's order");
        let (pct, lvl) = v.headline.unwrap();
        assert!((pct - 81.0).abs() < 0.01 && lvl == "warn");
        prefs.shows = Shows::Weekly;
        assert_eq!(build(&snap, &prefs, NOW).headline.unwrap().1, "critical");
        prefs.accounts.clear();
        assert_eq!(build(&snap, &prefs, NOW).headline, None);
    }
}
