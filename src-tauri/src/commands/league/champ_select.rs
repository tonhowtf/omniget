use serde_json::Value;
use std::collections::HashSet;

/// Bans wait longer than picks by default in most rulesets, and a ban sent on the
/// first millisecond of the turn is the most visible tell of an automation.
pub const MAX_BAN_DELAY: u8 = 25;

pub fn ban_delay_seconds(setting: u8) -> u8 {
    setting.min(MAX_BAN_DELAY)
}

pub fn delay_elapsed(elapsed_secs: f64, delay: u8) -> bool {
    elapsed_secs >= delay as f64
}

/// Champions an ally is showing intent on. Banning one of these takes away a
/// teammate's pick, which is the single most disruptive thing an auto-ban can do.
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

/// First entry of the user's list that is legal to act on and not spoken for.
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

/// How the automation finishes a pick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PickConfirm {
    /// Locks the champion as soon as the turn opens.
    Immediate,
    /// Shows the intent to the team and leaves the lock to the user.
    Declare,
    /// Shows the intent immediately and locks only when the turn is about to
    /// expire, which leaves room for the team to ask for something else.
    AtTimeout,
}

/// Locking exactly at zero risks losing the pick to a slow round trip.
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

/// Milliseconds left in the current champion select phase, when the client
/// reports it.
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
        // Without a timer there is no safe moment to wait for, so the pick is
        // locked rather than risking losing it entirely.
        PickConfirm::AtTimeout => time_left.is_none_or(|left| left <= TIMEOUT_LOCK_MARGIN_MS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn set(ids: &[i64]) -> HashSet<i64> {
        ids.iter().copied().collect()
    }

    #[test]
    fn the_first_legal_and_unclaimed_entry_wins() {
        let list = [1i64, 2, 3, 4];
        let pool = set(&[2, 3, 4]);
        let taken = set(&[2]);
        let avoid = set(&[3]);
        assert_eq!(choose_champion(&list, &pool, &taken, &avoid), Some(4));
    }

    #[test]
    fn nothing_is_chosen_when_every_entry_is_spoken_for() {
        let list = [1i64, 2];
        assert_eq!(
            choose_champion(&list, &set(&[1, 2]), &set(&[1]), &set(&[2])),
            None
        );
        assert_eq!(choose_champion(&[], &set(&[1]), &set(&[]), &set(&[])), None);
        // Outside the legal pool, however free it looks.
        assert_eq!(
            choose_champion(&[9], &set(&[1, 2]), &set(&[]), &set(&[])),
            None
        );
    }

    #[test]
    fn ally_intents_exclude_the_local_player() {
        let session = json!({
            "myTeam": [
                { "cellId": 0, "championId": 0, "championPickIntent": 111 },
                { "cellId": 1, "championId": 0, "championPickIntent": 222 },
                { "cellId": 2, "championId": 333, "championPickIntent": 0 }
            ]
        });
        let intents = ally_pick_intents(&session, 0);
        assert_eq!(intents, set(&[222, 333]));
        assert!(!intents.contains(&111), "own intent must not block own ban");
    }

    #[test]
    fn a_session_without_a_team_yields_no_intents() {
        assert!(ally_pick_intents(&json!({}), 0).is_empty());
        assert!(ally_pick_intents(&json!({ "myTeam": [] }), 0).is_empty());
        assert!(ally_pick_intents(&json!({ "myTeam": [{}] }), 0).is_empty());
    }

    #[test]
    fn the_ban_delay_is_capped_and_measured_in_seconds() {
        assert_eq!(ban_delay_seconds(200), MAX_BAN_DELAY);
        assert_eq!(ban_delay_seconds(7), 7);
        assert!(delay_elapsed(7.0, 7));
        assert!(delay_elapsed(9.5, 7));
        assert!(!delay_elapsed(6.9, 7));
        // No delay configured means act on sight.
        assert!(delay_elapsed(0.0, 0));
    }

    #[test]
    fn the_three_confirm_modes_map_from_the_two_toggles() {
        assert_eq!(pick_confirm(false, false), PickConfirm::Declare);
        assert_eq!(pick_confirm(true, false), PickConfirm::Immediate);
        // Waiting for the timer wins over locking immediately.
        assert_eq!(pick_confirm(true, true), PickConfirm::AtTimeout);
        assert_eq!(pick_confirm(false, true), PickConfirm::AtTimeout);
    }

    #[test]
    fn immediate_locks_at_once_and_declare_never_locks() {
        assert!(should_lock_now(PickConfirm::Immediate, Some(30_000)));
        assert!(!should_lock_now(PickConfirm::Declare, Some(100)));
        assert!(!should_lock_now(PickConfirm::Declare, None));
    }

    #[test]
    fn waiting_for_the_timer_locks_only_near_the_end() {
        assert!(!should_lock_now(PickConfirm::AtTimeout, Some(20_000)));
        assert!(should_lock_now(
            PickConfirm::AtTimeout,
            Some(TIMEOUT_LOCK_MARGIN_MS)
        ));
        assert!(should_lock_now(PickConfirm::AtTimeout, Some(0)));
        // A missing timer must not mean "never lock", or the pick is lost.
        assert!(should_lock_now(PickConfirm::AtTimeout, None));
    }

    #[test]
    fn the_phase_timer_is_read_when_present() {
        let session = json!({ "timer": { "adjustedTimeLeftInPhase": 17_500 } });
        assert_eq!(time_left_ms(&session), Some(17_500));
        assert_eq!(time_left_ms(&json!({})), None);
        assert_eq!(time_left_ms(&json!({ "timer": {} })), None);
    }
}
