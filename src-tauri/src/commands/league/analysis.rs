//! Aggregations over match history: champion records, duo synergy and the
//! early-game map reading used for jungle pathing.

use super::stats;

/// Minutes of a game considered "early" for pathing purposes.
pub const EARLY_MINUTES: usize = 14;

/// A kill counts for more than a position sample when reading intent.
pub const KILL_WEIGHT: f64 = 5.0;

/// Lanes of the map, plus the ambiguous middle ground.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Top,
    Mid,
    Bot,
}

/// Classifies a map coordinate into a lane.
///
/// The top-left and bottom-right corners are unambiguous; everything else is
/// decided by distance from the mid diagonal (`y == x`).
pub fn classify_zone(x: f64, y: f64) -> Zone {
    if x < 5_000.0 && y > 9_000.0 {
        return Zone::Top;
    }
    if x > 9_000.0 && y < 5_000.0 {
        return Zone::Bot;
    }
    if (y - x).abs() <= 3_500.0 {
        return Zone::Mid;
    }
    if y > x {
        Zone::Top
    } else {
        Zone::Bot
    }
}

/// Share of early-game presence per lane, from position samples and kills.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ZoneWeights {
    pub top: f64,
    pub mid: f64,
    pub bot: f64,
}

impl ZoneWeights {
    pub fn add(&mut self, zone: Zone, weight: f64) {
        match zone {
            Zone::Top => self.top += weight,
            Zone::Mid => self.mid += weight,
            Zone::Bot => self.bot += weight,
        }
    }

    pub fn total(&self) -> f64 {
        self.top + self.mid + self.bot
    }

    /// Normalises to shares that sum to 1 (all zeros when there is no data).
    pub fn shares(&self) -> ZoneWeights {
        let t = self.total();
        if t <= 0.0 {
            return ZoneWeights::default();
        }
        ZoneWeights {
            top: self.top / t,
            mid: self.mid / t,
            bot: self.bot / t,
        }
    }

    /// Short verdict about where this jungler spends the early game.
    pub fn preference(&self) -> &'static str {
        let s = self.shares();
        if s.total() <= 0.0 {
            return "unknown";
        }
        let max = s.top.max(s.mid).max(s.bot);
        if s.mid + s.bot >= 0.70 && s.top <= 0.28 {
            return "mid_bot";
        }
        if s.top + s.mid >= 0.70 && s.bot <= 0.28 {
            return "top_mid";
        }
        let second = {
            let mut v = [s.top, s.mid, s.bot];
            v.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
            v[1]
        };
        if max >= 0.40 && max - second >= 0.08 {
            if (max - s.top).abs() < f64::EPSILON {
                "top"
            } else if (max - s.mid).abs() < f64::EPSILON {
                "mid"
            } else {
                "bot"
            }
        } else {
            "balanced"
        }
    }
}

/// The eight buff camps, as (x, y, blue_side).
pub const CAMPS: [(&str, f64, f64, bool); 8] = [
    ("blue_buff", 3_830.0, 7_880.0, true),
    ("wolves", 3_800.0, 6_440.0, true),
    ("red_buff", 7_760.0, 4_010.0, true),
    ("raptors", 6_970.0, 5_460.0, true),
    ("blue_buff", 10_990.0, 7_000.0, false),
    ("wolves", 11_020.0, 8_440.0, false),
    ("red_buff", 7_060.0, 10_870.0, false),
    ("raptors", 7_850.0, 9_420.0, false),
];

/// Nearest starting camp to a position, and whether it sits on the blue side.
pub fn nearest_camp(x: f64, y: f64) -> (&'static str, bool) {
    let mut best = (CAMPS[0].0, CAMPS[0].3);
    let mut best_d = f64::MAX;
    for (name, cx, cy, blue) in CAMPS.iter() {
        let d = (x - cx).powi(2) + (y - cy).powi(2);
        if d < best_d {
            best_d = d;
            best = (name, *blue);
        }
    }
    best
}

/// True when the first camp visited belongs to the opposing side of the map.
pub fn is_invade(first_camp_blue_side: bool, player_on_blue_side: bool) -> bool {
    first_camp_blue_side != player_on_blue_side
}

/// A gank is likely when the clear was cut short: enough camps for level 3 but
/// well below a full clear, with champion damage already dealt.
pub fn looks_like_level3_gank(cs_at_frame3: u32, level: u32, had_champion_action: bool) -> bool {
    (12..20).contains(&cs_at_frame3) && level == 3 && had_champion_action
}

/// One champion's record inside a player's history.
#[derive(Debug, Clone, Default)]
pub struct ChampionRecord {
    pub champion_id: i64,
    pub games: u32,
    pub wins: u32,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub cs: f64,
    pub seconds: f64,
}

impl ChampionRecord {
    pub fn winrate(&self) -> f64 {
        if self.games == 0 {
            0.0
        } else {
            self.wins as f64 / self.games as f64
        }
    }

    pub fn kda(&self) -> f64 {
        stats::kda(self.kills, self.deaths, self.assists)
    }

    pub fn cs_per_min(&self) -> f64 {
        stats::per_minute(self.cs, self.seconds)
    }
}

/// Sorts champion records the way a profile page does: most played first, then
/// win rate. Records under `min_games` are dropped (op.gg shows 2+ games).
pub fn rank_champion_records(
    mut records: Vec<ChampionRecord>,
    min_games: u32,
) -> Vec<ChampionRecord> {
    records.retain(|r| r.games >= min_games);
    records.sort_by(|a, b| {
        b.games.cmp(&a.games).then(
            b.winrate()
                .partial_cmp(&a.winrate())
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    records
}

/// A teammate the player keeps queueing with.
#[derive(Debug, Clone)]
pub struct DuoRecord {
    pub puuid: String,
    pub games: u32,
    pub wins: u32,
}

impl DuoRecord {
    pub fn winrate(&self) -> f64 {
        if self.games == 0 {
            0.0
        } else {
            self.wins as f64 / self.games as f64
        }
    }

    /// Shrunk win rate, so a 2-game 100% duo cannot outrank a 40-game 60% one.
    pub fn score(&self) -> f64 {
        stats::shrunk_winrate(self.wins, self.games - self.wins, stats::PRIOR_GAMES)
    }
}

/// Orders duos by shrunk win rate, keeping only those with enough games.
pub fn rank_duos(mut duos: Vec<DuoRecord>, min_games: u32) -> Vec<DuoRecord> {
    duos.retain(|d| d.games >= min_games);
    duos.sort_by(|a, b| {
        b.score()
            .partial_cmp(&a.score())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.games.cmp(&a.games))
    });
    duos
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_match_the_map_corners() {
        // Top lane sits in the upper-left, bot lane in the lower-right.
        assert_eq!(classify_zone(1_500.0, 12_000.0), Zone::Top);
        assert_eq!(classify_zone(12_500.0, 2_000.0), Zone::Bot);
        // Mid follows the diagonal.
        assert_eq!(classify_zone(7_500.0, 7_500.0), Zone::Mid);
        assert_eq!(classify_zone(6_000.0, 8_000.0), Zone::Mid);
        // Off-diagonal but not in a corner: decided by which side of mid.
        assert_eq!(classify_zone(2_000.0, 7_000.0), Zone::Top);
        assert_eq!(classify_zone(9_000.0, 3_000.0), Zone::Bot);
    }

    #[test]
    fn zone_weights_normalise_and_read_preference() {
        let mut w = ZoneWeights::default();
        for _ in 0..8 {
            w.add(Zone::Bot, 1.0);
        }
        for _ in 0..6 {
            w.add(Zone::Mid, 1.0);
        }
        w.add(Zone::Top, 1.0);
        let s = w.shares();
        assert!((s.top + s.mid + s.bot - 1.0).abs() < 1e-9);
        assert_eq!(w.preference(), "mid_bot");

        // No samples must not divide by zero.
        assert_eq!(ZoneWeights::default().preference(), "unknown");
        assert_eq!(ZoneWeights::default().shares().total(), 0.0);
    }

    #[test]
    fn balanced_pathing_is_reported_as_balanced() {
        let mut w = ZoneWeights::default();
        w.add(Zone::Top, 10.0);
        w.add(Zone::Mid, 10.0);
        w.add(Zone::Bot, 10.0);
        assert_eq!(w.preference(), "balanced");
    }

    #[test]
    fn kills_weigh_more_than_standing_around() {
        let mut w = ZoneWeights::default();
        w.add(Zone::Top, 1.0);
        w.add(Zone::Bot, KILL_WEIGHT);
        let s = w.shares();
        assert!(s.bot > s.top * 4.0);
    }

    #[test]
    fn first_camp_detects_side_and_invade() {
        // Standing on the blue-side blue buff at minute one.
        let (camp, blue) = nearest_camp(3_900.0, 7_900.0);
        assert_eq!(camp, "blue_buff");
        assert!(blue);
        // A blue-side jungler starting there is not invading.
        assert!(!is_invade(blue, true));
        // The same player showing up in the red-side jungle is.
        let (_, enemy_side) = nearest_camp(11_000.0, 7_100.0);
        assert!(!enemy_side);
        assert!(is_invade(enemy_side, true));
    }

    #[test]
    fn level3_gank_heuristic_needs_all_three_signals() {
        // Clear cut short at 15 CS, level 3, already fought: a gank.
        assert!(looks_like_level3_gank(15, 3, true));
        // A full clear reaches ~20 CS and means no detour.
        assert!(!looks_like_level3_gank(24, 3, true));
        // Damage matters: farming quietly is not a gank.
        assert!(!looks_like_level3_gank(15, 3, false));
        // Wrong level is not a level-3 gank.
        assert!(!looks_like_level3_gank(15, 4, true));
    }

    #[test]
    fn champion_records_rank_like_a_profile_page() {
        let make = |id, games, wins| ChampionRecord {
            champion_id: id,
            games,
            wins,
            seconds: 1_800.0 * games as f64,
            cs: 200.0 * games as f64,
            ..Default::default()
        };
        let ranked = rank_champion_records(vec![make(1, 3, 1), make(2, 9, 5), make(3, 1, 1)], 2);
        // The single-game champion is dropped, the most played leads.
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].champion_id, 2);
        assert_eq!(ranked[1].champion_id, 1);
        assert!((ranked[0].winrate() - 5.0 / 9.0).abs() < 1e-9);
        assert!((ranked[0].cs_per_min() - 200.0 / 30.0).abs() < 1e-9);
    }

    #[test]
    fn duos_are_ranked_by_shrunk_winrate() {
        let lucky = DuoRecord {
            puuid: "lucky".into(),
            games: 2,
            wins: 2,
        };
        let proven = DuoRecord {
            puuid: "proven".into(),
            games: 40,
            wins: 24,
        };
        let ranked = rank_duos(vec![lucky, proven], 2);
        assert_eq!(
            ranked[0].puuid, "proven",
            "a 40-game 60% duo must beat a 2-game 100% duo"
        );
        assert!((ranked[0].winrate() - 0.6).abs() < 1e-9);
    }

    #[test]
    fn duos_below_the_minimum_are_dropped() {
        let one_off = DuoRecord {
            puuid: "stranger".into(),
            games: 1,
            wins: 1,
        };
        assert!(rank_duos(vec![one_off], 2).is_empty());
        // A perfect record still scores below 100% after shrinkage.
        let d = DuoRecord {
            puuid: "x".into(),
            games: 10,
            wins: 10,
        };
        assert!(d.score() < 0.7 && d.score() > 0.5, "score={}", d.score());
    }
}
