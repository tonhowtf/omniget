use rand::RngExt;
use serde_json::Value;
use std::collections::HashSet;

pub const MAX_BAN_DELAY: u8 = 25;

pub fn ban_delay_seconds(setting: u8) -> u8 {
    setting.min(MAX_BAN_DELAY)
}

pub fn delay_elapsed(elapsed_secs: f64, delay: u8) -> bool {
    elapsed_secs >= delay as f64
}

pub fn ally_pick_intents(session: &Value, local_cell: i64) -> HashSet<i64> {
    let mut intents = HashSet::new();
    if let Some(team) = session.get("myTeam").and_then(Value::as_array) {
        for member in team {
            let cell = member.get("cellId").and_then(Value::as_i64).unwrap_or(-1);
            if cell == local_cell {
                continue;
            }
            for key in ["championPickIntent", "championId"] {
                if let Some(id) = member.get(key).and_then(Value::as_i64) {
                    if id > 0 {
                        intents.insert(id);
                    }
                }
            }
        }
    }
    intents
}

pub fn choose_champion(
    list: &[i64],
    pool: &HashSet<i64>,
    taken: &HashSet<i64>,
    avoid: &HashSet<i64>,
) -> Option<i64> {
    list.iter()
        .find(|c| pool.contains(c) && !taken.contains(c) && !avoid.contains(c))
        .copied()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PickConfirm {
    Immediate,
    Declare,
    AtTimeout,
}

pub const TIMEOUT_LOCK_MARGIN_MS: i64 = 2_500;

pub fn pick_confirm(auto_lock: bool, lock_at_timeout: bool) -> PickConfirm {
    if lock_at_timeout {
        PickConfirm::AtTimeout
    } else if auto_lock {
        PickConfirm::Immediate
    } else {
        PickConfirm::Declare
    }
}

pub fn time_left_ms(session: &Value) -> Option<i64> {
    session
        .get("timer")
        .and_then(|t| t.get("adjustedTimeLeftInPhase"))
        .and_then(Value::as_i64)
}

pub fn should_lock_now(mode: PickConfirm, time_left: Option<i64>) -> bool {
    match mode {
        PickConfirm::Immediate => true,
        PickConfirm::Declare => false,
        PickConfirm::AtTimeout => time_left.is_none_or(|left| left <= TIMEOUT_LOCK_MARGIN_MS),
    }
}

pub fn choose_random_champion<R: rand::Rng>(
    pool: &HashSet<i64>,
    taken: &HashSet<i64>,
    avoid: &HashSet<i64>,
    rng: &mut R,
) -> Option<i64> {
    let mut candidates: Vec<i64> = pool
        .iter()
        .copied()
        .filter(|c| !taken.contains(c) && !avoid.contains(c))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_unstable();
    let index = rng.random_range(0..candidates.len());
    Some(candidates[index])
}

pub fn local_pick_action(session: &Value, cell: i64) -> Option<(i64, bool)> {
    let groups = session.get("actions")?.as_array()?;
    for group in groups {
        for action in group.as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            let actor = action
                .get("actorCellId")
                .and_then(Value::as_i64)
                .unwrap_or(-2);
            let kind = action.get("type").and_then(Value::as_str).unwrap_or("");
            if actor != cell || kind != "pick" {
                continue;
            }
            let id = action.get("id").and_then(Value::as_i64)?;
            let completed = action
                .get("completed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            return Some((id, completed));
        }
    }
    None
}

pub fn locked_champion(session: &Value) -> i64 {
    let cell = session
        .get("localPlayerCellId")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    if cell < 0 {
        return 0;
    }
    let locked = matches!(local_pick_action(session, cell), Some((_, true)));
    if !locked {
        return 0;
    }
    session
        .get("myTeam")
        .and_then(Value::as_array)
        .and_then(|team| {
            team.iter()
                .find(|m| m.get("cellId").and_then(Value::as_i64) == Some(cell))
        })
        .and_then(|me| me.get("championId").and_then(Value::as_i64))
        .filter(|id| *id > 0)
        .unwrap_or(0)
}
