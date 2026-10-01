//! Statistical model for pre-game win probability and per-role performance.
//!
//! The estimator is deliberately conservative: matchmaking targets balanced
//! games, so honest outputs cluster near 50% and always carry an interval.

/// Prior strength for the Beta-Binomial shrinkage, expressed in games.
/// A player with `PRIOR_GAMES` games is pulled halfway to the 50% baseline.
pub const PRIOR_GAMES: f64 = 40.0;

/// Elo scale: a rating gap of `ELO_SCALE` means 10:1 odds in the logistic model.
pub const ELO_SCALE: f64 = 400.0;

/// Converts log-odds into Elo-equivalent rating points (400 / ln(10)).
pub const LOGIT_TO_ELO: f64 = 173.717_792_76;

/// Rating points per full tier (Iron -> Bronze -> ...).
pub const TIER_POINTS: f64 = 400.0;

/// Rating points per division inside a tier.
pub const DIVISION_POINTS: f64 = 100.0;

/// Rating floor: Iron IV. Community MMR estimates put Iron around 600 and the
/// median player (high Silver / low Gold) around 1400-1500.
pub const RATING_FLOOR: f64 = 600.0;

/// Default rating for players whose rank is unknown or hidden (ladder median).
pub const UNRANKED_RATING: f64 = 1500.0;

/// Rating uncertainty (in points) assumed for a player with no usable data.
pub const UNKNOWN_SIGMA: f64 = 260.0;

/// How much the win-rate signal is allowed to move a player's rating.
///
/// Kept below 1 on purpose: matchmaking drives win rate back to 50%, so it
/// measures displacement from one's own bracket, not absolute skill.
pub const FORM_WEIGHT: f64 = 0.5;

/// Rating uncertainty from the gap between hidden MMR and the visible rank,
/// about one division.
pub const RANK_SIGMA: f64 = 100.0;

/// z value for a 90% two-sided interval.
pub const Z_90: f64 = 1.644_853_63;

/// Beta-Binomial posterior mean with a prior centred on 50%.
///
/// `(w + k/2) / (n + k)` — the estimate of a 10-game sample is dominated by the
/// prior, while a 1000-game sample is essentially its own raw rate. This is what
/// makes 50% over 1000 games and 50% over 10 games behave differently: the point
/// estimate matches, but the confidence (see [`beta_posterior_sd`]) does not.
pub fn shrunk_winrate(wins: u32, losses: u32, prior_games: f64) -> f64 {
    let n = (wins + losses) as f64;
    let half = prior_games / 2.0;
    (wins as f64 + half) / (n + prior_games)
}

/// Standard deviation of the Beta posterior, i.e. how unsure we are about the
/// shrunk win rate. Shrinks as 1/sqrt(n), so large samples are trusted more.
pub fn beta_posterior_sd(wins: u32, losses: u32, prior_games: f64) -> f64 {
    let half = prior_games / 2.0;
    let a = wins as f64 + half;
    let b = losses as f64 + half;
    let total = a + b;
    ((a * b) / (total * total * (total + 1.0))).sqrt()
}

/// Wilson score interval for a binomial proportion.
///
/// Preferred over the normal approximation because it stays inside `[0, 1]` and
/// keeps sensible coverage for small `n` (where `p̂ ± z·sqrt(p̂q̂/n)` breaks down).
pub fn wilson_interval(wins: u32, losses: u32, z: f64) -> (f64, f64) {
    let n = (wins + losses) as f64;
    if n <= 0.0 {
        return (0.0, 1.0);
    }
    let p = wins as f64 / n;
    let z2 = z * z;
    let denom = 1.0 + z2 / n;
    let center = (p + z2 / (2.0 * n)) / denom;
    let margin = (z / denom) * ((p * (1.0 - p) / n) + z2 / (4.0 * n * n)).sqrt();
    ((center - margin).max(0.0), (center + margin).min(1.0))
}

/// Maps a ranked tier/division/LP triple onto a continuous rating scale.
///
/// Divisions are numbered from IV (lowest) to I (highest); apex tiers have a
/// single division and let LP run past 100.
pub fn rank_rating(tier: &str, division: &str, league_points: u32) -> Option<f64> {
    let tier_index = match tier.to_ascii_uppercase().as_str() {
        "IRON" => 0.0,
        "BRONZE" => 1.0,
        "SILVER" => 2.0,
        "GOLD" => 3.0,
        "PLATINUM" => 4.0,
        "EMERALD" => 5.0,
        "DIAMOND" => 6.0,
        "MASTER" => 7.0,
        "GRANDMASTER" => 8.0,
        "CHALLENGER" => 9.0,
        _ => return None,
    };
    let division_index = match division.to_ascii_uppercase().as_str() {
        "IV" | "4" => 0.0,
        "III" | "3" => 1.0,
        "II" | "2" => 2.0,
        "I" | "1" | "NA" | "" => 3.0,
        _ => 0.0,
    };
    let apex = tier_index >= 7.0;
    let base = RATING_FLOOR + tier_index * TIER_POINTS;
    if apex {
        // Apex tiers have no divisions; LP keeps climbing, so compress it.
        Some(base + (league_points as f64).min(2000.0) * 0.4)
    } else {
        Some(base + division_index * DIVISION_POINTS + (league_points as f64).min(100.0))
    }
}

/// Rating adjustment derived from a win rate, in Elo-equivalent points.
///
/// A shrunk 50% rate contributes nothing; deviations are converted through the
/// logit so the scale matches the logistic win model.
pub fn winrate_rating_delta(shrunk: f64) -> f64 {
    let p = shrunk.clamp(0.001, 0.999);
    (p / (1.0 - p)).ln() * LOGIT_TO_ELO
}

/// Logistic expectancy: probability that a rating of `a` beats a rating of `b`.
pub fn elo_expectancy(a: f64, b: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf((b - a) / ELO_SCALE))
}

/// Log5 (Bradley-Terry) combination of two independent win rates.
///
/// `P = (pa − pa·pb) / (pa + pb − 2·pa·pb)` — used as a cross-check of the Elo
/// path; both reduce to the same answer for symmetric inputs.
pub fn log5(pa: f64, pb: f64) -> f64 {
    let num = pa - pa * pb;
    let den = pa + pb - 2.0 * pa * pb;
    if den.abs() < f64::EPSILON {
        0.5
    } else {
        num / den
    }
}

/// One player's contribution to the team strength estimate.
#[derive(Debug, Clone, Copy)]
pub struct PlayerStrength {
    /// Point estimate of the player's rating.
    pub rating: f64,
    /// Standard deviation of that estimate, in rating points.
    pub sigma: f64,
    /// False when the player had no usable rank or history.
    pub known: bool,
}

impl PlayerStrength {
    pub fn unknown() -> Self {
        Self {
            rating: UNRANKED_RATING,
            sigma: UNKNOWN_SIGMA,
            known: false,
        }
    }
}

/// Builds a player's strength from rank plus win-rate evidence.
///
/// Rank carries the skill signal (matchmaking pushes win rate back to ~50% at
/// equilibrium, so a win rate mostly measures whether someone is currently
/// climbing or sliding). Season and recent samples enter as shrunk deviations.
pub fn player_strength(
    rank: Option<f64>,
    season_wins: u32,
    season_losses: u32,
    recent_wins: u32,
    recent_losses: u32,
) -> PlayerStrength {
    let season_games = season_wins + season_losses;
    let recent_games = recent_wins + recent_losses;
    if rank.is_none() && season_games == 0 && recent_games == 0 {
        return PlayerStrength::unknown();
    }

    let base = rank.unwrap_or(UNRANKED_RATING);
    // Rank is the strong signal; without it we fall back to a wide prior.
    let base_sigma = if rank.is_some() {
        RANK_SIGMA
    } else {
        UNKNOWN_SIGMA
    };

    let season_p = shrunk_winrate(season_wins, season_losses, PRIOR_GAMES);
    let season_delta = winrate_rating_delta(season_p);
    let season_sd = beta_posterior_sd(season_wins, season_losses, PRIOR_GAMES);

    let recent_p = shrunk_winrate(recent_wins, recent_losses, PRIOR_GAMES);
    let recent_delta = winrate_rating_delta(recent_p);
    let recent_sd = beta_posterior_sd(recent_wins, recent_losses, PRIOR_GAMES);

    // Season sample is larger and steadier; recent games carry the form signal.
    let delta = FORM_WEIGHT * (0.65 * season_delta + 0.35 * recent_delta);

    // Delta method: d(rating)/d(p) = LOGIT_TO_ELO / (p(1-p)).
    let season_slope = LOGIT_TO_ELO / (season_p * (1.0 - season_p)).max(1e-6);
    let recent_slope = LOGIT_TO_ELO / (recent_p * (1.0 - recent_p)).max(1e-6);
    let delta_sigma = FORM_WEIGHT
        * ((0.65 * season_slope * season_sd).powi(2) + (0.35 * recent_slope * recent_sd).powi(2))
            .sqrt();

    PlayerStrength {
        rating: base + delta,
        sigma: (base_sigma.powi(2) + delta_sigma.powi(2)).sqrt(),
        known: true,
    }
}

/// Result of the team-vs-team comparison.
#[derive(Debug, Clone, Copy)]
pub struct WinProbability {
    /// Point estimate for the ally team.
    pub probability: f64,
    /// Lower bound of the 90% interval.
    pub low: f64,
    /// High bound of the 90% interval.
    pub high: f64,
    /// Mean rating gap in Elo points (positive favours the allies).
    pub rating_gap: f64,
    /// How many of the ten players contributed real data.
    pub known_players: usize,
    pub total_players: usize,
}

/// Aggregates both teams and returns the ally win probability with an interval.
///
/// Teams are equal size, so the mean rating is the right aggregate under a
/// Bradley-Terry model with additive strengths. The interval comes from
/// propagating each player's variance through the mean and then through the
/// logistic — it is wide on purpose when players are unknown.
pub fn team_win_probability(
    allies: &[PlayerStrength],
    enemies: &[PlayerStrength],
) -> WinProbability {
    let total_players = allies.len() + enemies.len();
    let known_players =
        allies.iter().filter(|p| p.known).count() + enemies.iter().filter(|p| p.known).count();

    if allies.is_empty() || enemies.is_empty() {
        return WinProbability {
            probability: 0.5,
            low: 0.0,
            high: 1.0,
            rating_gap: 0.0,
            known_players,
            total_players,
        };
    }

    let mean = |team: &[PlayerStrength]| -> f64 {
        team.iter().map(|p| p.rating).sum::<f64>() / team.len() as f64
    };
    // Variance of a mean of independent estimates: sum(sigma^2) / n^2.
    let mean_var = |team: &[PlayerStrength]| -> f64 {
        team.iter().map(|p| p.sigma * p.sigma).sum::<f64>() / (team.len() as f64).powi(2)
    };

    let gap = mean(allies) - mean(enemies);
    let gap_sigma = (mean_var(allies) + mean_var(enemies)).sqrt();

    let probability = elo_expectancy(mean(allies), mean(enemies));
    let low = elo_expectancy(gap - Z_90 * gap_sigma, 0.0);
    let high = elo_expectancy(gap + Z_90 * gap_sigma, 0.0);

    WinProbability {
        probability,
        low: low.min(probability),
        high: high.max(probability),
        rating_gap: gap,
        known_players,
        total_players,
    }
}

/// Per-minute normalisation guarding against zero-length games.
pub fn per_minute(value: f64, seconds: f64) -> f64 {
    if seconds <= 0.0 {
        0.0
    } else {
        value / (seconds / 60.0)
    }
}

/// KDA ratio with the usual "perfect KDA" convention when deaths are zero.
pub fn kda(kills: u32, deaths: u32, assists: u32) -> f64 {
    let k = (kills + assists) as f64;
    if deaths == 0 {
        k
    } else {
        k / deaths as f64
    }
}

/// Kill participation: share of the team's kills the player was involved in.
pub fn kill_participation(kills: u32, assists: u32, team_kills: u32) -> f64 {
    if team_kills == 0 {
        0.0
    } else {
        ((kills + assists) as f64 / team_kills as f64).min(1.0)
    }
}

/// Gold value carried in a player's item slots (exact, from live item prices).
pub fn items_gold(prices: &[(u32, u32)]) -> u32 {
    prices.iter().map(|(price, count)| price * count).sum()
}

/// Default performance targets per role, used until the user overrides them.
#[derive(Debug, Clone, Copy)]
pub struct RoleTargets {
    pub cs_per_min: f64,
    pub gold_per_min: f64,
    pub kda: f64,
    pub vision_per_min: f64,
    pub damage_share: f64,
}

/// Baselines are per-role because a support's CS target is not a mid's.
pub fn default_targets(position: &str) -> RoleTargets {
    match position.to_ascii_uppercase().as_str() {
        "TOP" => RoleTargets {
            cs_per_min: 7.0,
            gold_per_min: 380.0,
            kda: 2.5,
            vision_per_min: 0.6,
            damage_share: 0.22,
        },
        "JUNGLE" => RoleTargets {
            cs_per_min: 5.5,
            gold_per_min: 360.0,
            kda: 3.0,
            vision_per_min: 1.0,
            damage_share: 0.18,
        },
        "MIDDLE" | "MID" => RoleTargets {
            cs_per_min: 7.5,
            gold_per_min: 400.0,
            kda: 3.0,
            vision_per_min: 0.7,
            damage_share: 0.27,
        },
        "BOTTOM" | "ADC" => RoleTargets {
            cs_per_min: 8.0,
            gold_per_min: 420.0,
            kda: 3.0,
            vision_per_min: 0.6,
            damage_share: 0.28,
        },
        "UTILITY" | "SUPPORT" => RoleTargets {
            cs_per_min: 1.5,
            gold_per_min: 260.0,
            kda: 3.0,
            vision_per_min: 2.0,
            damage_share: 0.10,
        },
        _ => RoleTargets {
            cs_per_min: 6.0,
            gold_per_min: 360.0,
            kda: 2.5,
            vision_per_min: 0.8,
            damage_share: 0.20,
        },
    }
}

/// Share of a team total, expressed against an even split.
///
/// `1.0` means the player carried exactly their fair share (1/team_size);
/// `2.0` means double it. Returns 0 when the team total is unusable.
pub fn contribution_ratio(value: f64, team_total: f64, team_size: usize) -> f64 {
    if team_total <= 0.0 || team_size == 0 {
        return 0.0;
    }
    (value / team_total) * team_size as f64
}

/// Kill-damage efficiency: kill share divided by damage share.
///
/// Above 1 the player converts less damage into more kills (finishing or
/// stealing); below 1 they deal damage others convert. Neutral when unknown.
pub fn kill_damage_efficiency(kills: u32, team_kills: u32, damage: f64, team_damage: f64) -> f64 {
    if team_kills == 0 || team_damage <= 0.0 || damage <= 0.0 {
        return 1.0;
    }
    let kill_share = kills as f64 / team_kills as f64;
    let damage_share = damage / team_damage;
    if damage_share <= 0.0 {
        1.0
    } else {
        kill_share / damage_share
    }
}

/// Linear ramp used by the impact components: `min` scores 0, `max` scores full.
fn ramp(value: f64, min: f64, max: f64, weight: f64) -> f64 {
    if max <= min {
        return 0.0;
    }
    ((value - min) / (max - min)).clamp(0.0, 1.0) * weight
}

/// Inputs for the composite impact score, all taken from one finished game.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImpactInput {
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub team_kills: u32,
    pub damage_to_champions: f64,
    pub team_damage: f64,
    pub damage_taken: f64,
    pub team_damage_taken: f64,
    pub gold: f64,
    pub team_gold: f64,
    pub cs: f64,
    pub vision_score: f64,
    pub team_vision: f64,
    pub duration_seconds: f64,
    pub team_size: usize,
}

/// Composite 0-10 impact score for a single game.
///
/// Every component is a bounded ramp, so no single stat can dominate and the
/// result stays comparable across roles and game lengths.
pub fn impact_score(input: &ImpactInput) -> f64 {
    let team_size = if input.team_size == 0 {
        5
    } else {
        input.team_size
    };

    // KDA on a square-root curve: the gap from 2 to 5 matters more than 8 to 11.
    let kda_value = kda(input.kills, input.deaths, input.assists);
    let kda_component = ramp((kda_value - 2.0).max(0.0).sqrt(), 0.0, 2.2, 1.5);

    let kp = kill_participation(input.kills, input.assists, input.team_kills);
    let kp_component = ramp(kp, 0.3, 1.0, 1.5);

    let damage_component = ramp(
        contribution_ratio(input.damage_to_champions, input.team_damage, team_size),
        1.0,
        2.0,
        2.0,
    );
    let tank_component = ramp(
        contribution_ratio(input.damage_taken, input.team_damage_taken, team_size),
        1.0,
        2.0,
        1.0,
    );
    let gold_component = ramp(
        contribution_ratio(input.gold, input.team_gold, team_size),
        1.0,
        1.5,
        1.5,
    );
    let vision_component = ramp(
        contribution_ratio(input.vision_score, input.team_vision, team_size),
        1.0,
        2.0,
        1.5,
    );
    let cs_component = ramp(per_minute(input.cs, input.duration_seconds), 4.0, 9.0, 1.0);

    (kda_component
        + kp_component
        + damage_component
        + tank_component
        + gold_component
        + vision_component
        + cs_component)
        .clamp(0.0, 10.0)
}

/// Queues whose result carries signal for a player score. Customs, bots and
/// rotating modes are left out.
pub const SCORED_QUEUES: [i64; 5] = [400, 420, 430, 440, 450];

/// Below this many usable games the average describes luck, not the player, so no
/// score is reported at all — an honest blank beats a confident wrong number.
pub const SCORE_MIN_GAMES: usize = 5;

/// Games shorter than this were remakes.
pub const SCORE_MIN_DURATION: i64 = 180;

pub struct ScoredGame {
    pub queue_id: i64,
    pub duration: i64,
    pub kills: i64,
    pub deaths: i64,
    pub assists: i64,
    pub win: bool,
    pub support: bool,
}

pub fn score_counts(game: &ScoredGame) -> bool {
    SCORED_QUEUES.contains(&game.queue_id) && game.duration >= SCORE_MIN_DURATION
}

/// One game's contribution: a KDA-like ratio (assists worth a little more for
/// supports, whose impact rarely shows up as kills), minus a penalty for games
/// that ran away, plus the result.
pub fn game_score(game: &ScoredGame) -> f64 {
    let assist_weight = if game.support { 1.2 } else { 1.0 };
    let ratio =
        (game.kills as f64 + game.assists as f64 * assist_weight) / (game.deaths.max(1) as f64);
    let penalty = if game.deaths > 20 && game.kills <= 10 {
        1.0
    } else if game.deaths > 15 && game.kills <= 10 {
        0.5
    } else {
        0.0
    };
    ratio - penalty + if game.win { 1.0 } else { -1.0 }
}

/// Average score over the usable games, or `None` when the sample is too small.
pub fn player_score(games: &[ScoredGame]) -> (Option<f64>, usize) {
    let usable: Vec<&ScoredGame> = games.iter().filter(|g| score_counts(g)).collect();
    if usable.len() < SCORE_MIN_GAMES {
        return (None, usable.len());
    }
    let total: f64 = usable.iter().map(|g| game_score(g)).sum();
    (Some(total / usable.len() as f64), usable.len())
}

/// Groups players that share a party id, which the gameflow session reports
/// directly. Solo players carry a unique id, so single-player groups are
/// dropped just like in the history-based detection.
pub fn party_groups(party_ids: &[Option<String>]) -> Vec<Vec<usize>> {
    let mut buckets: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
    for (index, id) in party_ids.iter().enumerate() {
        if let Some(id) = id.as_deref().filter(|s| !s.is_empty()) {
            buckets.entry(id).or_default().push(index);
        }
    }
    let mut groups: Vec<Vec<usize>> = buckets.into_values().filter(|g| g.len() > 1).collect();
    for group in groups.iter_mut() {
        group.sort_unstable();
    }
    groups.sort_by(|a, b| a[0].cmp(&b[0]));
    groups
}

/// Disjoint-set forest used to merge overlapping premade pairs.
struct DisjointSet {
    parent: Vec<usize>,
}

impl DisjointSet {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            let root = self.find(self.parent[x]);
            self.parent[x] = root;
        }
        self.parent[x]
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[rb] = ra;
        }
    }
}

/// Groups players that keep showing up in each other's recent games.
///
/// `pairs` holds `(i, j, shared_games)`. Pairs at or above `threshold` are
/// merged transitively, so a duo that both queue with a third player surfaces as
/// one trio. Groups of a single player are dropped.
pub fn premade_groups(
    player_count: usize,
    pairs: &[(usize, usize, u32)],
    threshold: u32,
) -> Vec<Vec<usize>> {
    if player_count == 0 {
        return Vec::new();
    }
    let mut sets = DisjointSet::new(player_count);
    for (a, b, shared) in pairs {
        if *shared >= threshold && *a < player_count && *b < player_count {
            sets.union(*a, *b);
        }
    }
    let mut buckets: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for i in 0..player_count {
        let root = sets.find(i);
        buckets.entry(root).or_default().push(i);
    }
    let mut groups: Vec<Vec<usize>> = buckets
        .into_values()
        .filter(|g| g.len() > 1)
        .map(|mut g| {
            g.sort_unstable();
            g
        })
        .collect();
    groups.sort_by(|a, b| a[0].cmp(&b[0]));
    groups
}
