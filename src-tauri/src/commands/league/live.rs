use serde::Serialize;
use serde_json::Value;

/// Respawn windows are balance constants and move between patches, so every
/// number derived from them is presented as an estimate.
const BARON_RESPAWN: f64 = 360.0;
const DRAGON_RESPAWN: f64 = 300.0;
const INHIBITOR_RESPAWN: f64 = 300.0;

#[derive(Debug, Serialize, PartialEq)]
pub struct ObjectiveTimer {
    pub kind: &'static str,
    pub killed_at: f64,
    pub respawns_at: f64,
    pub remaining: f64,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct GameEvent {
    pub id: i64,
    pub name: String,
    pub at: f64,
    pub actor: Option<String>,
    pub target: Option<String>,
    pub detail: Option<String>,
}

fn event_time(event: &Value) -> Option<f64> {
    event.get("EventTime").and_then(Value::as_f64)
}

fn text(event: &Value, key: &str) -> Option<String> {
    event
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn last_kill_time(events: &[Value], name: &str) -> Option<f64> {
    events
        .iter()
        .filter(|e| event_name(e) == name)
        .filter_map(event_time)
        .fold(None, |acc: Option<f64>, t| {
            Some(acc.map_or(t, |a| a.max(t)))
        })
}

fn event_name(event: &Value) -> &str {
    event.get("EventName").and_then(Value::as_str).unwrap_or("")
}

/// Turns the kill events into "when does it come back" timers. Objectives whose
/// window already elapsed are dropped: an expired timer says nothing the map
/// doesn't already show.
pub fn objective_timers(events: &[Value], game_time: f64) -> Vec<ObjectiveTimer> {
    let mut timers = Vec::new();
    for (kind, event, window) in [
        ("baron", "BaronKill", BARON_RESPAWN),
        ("dragon", "DragonKill", DRAGON_RESPAWN),
        ("inhibitor", "InhibKilled", INHIBITOR_RESPAWN),
    ] {
        if let Some(killed_at) = last_kill_time(events, event) {
            let respawns_at = killed_at + window;
            let remaining = respawns_at - game_time;
            if remaining > 0.0 {
                timers.push(ObjectiveTimer {
                    kind,
                    killed_at,
                    respawns_at,
                    remaining,
                });
            }
        }
    }
    timers.sort_by(|a, b| a.remaining.total_cmp(&b.remaining));
    timers
}

/// Normalises the raw event list into a feed the UI can render without knowing
/// the shape of every event type.
pub fn game_events(events: &[Value], limit: usize) -> Vec<GameEvent> {
    let mut feed: Vec<GameEvent> = events
        .iter()
        .filter_map(|event| {
            let name = event_name(event);
            if name.is_empty() {
                return None;
            }
            let (actor, target, detail) = match name {
                "ChampionKill" => (
                    text(event, "KillerName"),
                    text(event, "VictimName"),
                    assist_count(event),
                ),
                "TurretKilled" => (text(event, "KillerName"), text(event, "TurretKilled"), None),
                "InhibKilled" | "InhibRespawned" | "InhibRespawningSoon" => {
                    (text(event, "KillerName"), text(event, "InhibKilled"), None)
                }
                "DragonKill" => (
                    text(event, "KillerName"),
                    None,
                    text(event, "DragonType").or_else(|| stolen(event)),
                ),
                "BaronKill" | "HeraldKill" => (text(event, "KillerName"), None, stolen(event)),
                "Multikill" => (
                    text(event, "KillerName"),
                    None,
                    event
                        .get("KillStreak")
                        .and_then(Value::as_i64)
                        .map(|n| n.to_string()),
                ),
                "Ace" => (text(event, "Acer"), None, text(event, "AcingTeam")),
                "FirstBrick" | "FirstBlood" => (
                    text(event, "KillerName").or_else(|| text(event, "Recipient")),
                    None,
                    None,
                ),
                _ => (text(event, "KillerName"), None, None),
            };
            Some(GameEvent {
                id: event.get("EventID").and_then(Value::as_i64).unwrap_or(-1),
                name: name.to_string(),
                at: event_time(event).unwrap_or(0.0),
                actor,
                target,
                detail,
            })
        })
        .collect();
    // Newest first, and the client is not guaranteed to keep them ordered.
    feed.sort_by(|a, b| b.at.total_cmp(&a.at).then(b.id.cmp(&a.id)));
    feed.truncate(limit);
    feed
}

fn stolen(event: &Value) -> Option<String> {
    match event.get("Stolen") {
        Some(Value::String(s)) if s.eq_ignore_ascii_case("true") => Some("stolen".to_string()),
        Some(Value::Bool(true)) => Some("stolen".to_string()),
        _ => None,
    }
}

fn assist_count(event: &Value) -> Option<String> {
    let count = event
        .get("Assisters")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);
    if count > 0 {
        Some(count.to_string())
    } else {
        None
    }
}
