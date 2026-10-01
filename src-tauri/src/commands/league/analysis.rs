use super::stats;

pub const EARLY_MINUTES: usize = 14;

pub const KILL_WEIGHT: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Top,
    Mid,
    Bot,
}

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

pub fn is_invade(first_camp_blue_side: bool, player_on_blue_side: bool) -> bool {
    first_camp_blue_side != player_on_blue_side
}

pub fn looks_like_level3_gank(cs_at_frame3: u32, level: u32, had_champion_action: bool) -> bool {
    (12..20).contains(&cs_at_frame3) && level == 3 && had_champion_action
}

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

    pub fn score(&self) -> f64 {
        stats::shrunk_winrate(self.wins, self.games - self.wins, stats::PRIOR_GAMES)
    }
}

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
