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

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn shrinkage_pulls_small_samples_toward_the_baseline() {
        // 60% over 10 games is mostly prior; 60% over 1000 games is trusted.
        let small = shrunk_winrate(6, 4, PRIOR_GAMES);
        let large = shrunk_winrate(600, 400, PRIOR_GAMES);
        assert!(small < large, "small sample must shrink harder");
        assert!(close(small, 0.52, 0.01), "got {}", small);
        assert!(close(large, 0.596, 0.005), "got {}", large);
    }

    #[test]
    fn equal_rates_differ_only_in_confidence() {
        // The user's question: 50% over 1000 vs 50% over 10.
        let few = shrunk_winrate(5, 5, PRIOR_GAMES);
        let many = shrunk_winrate(500, 500, PRIOR_GAMES);
        assert!(close(few, 0.5, 1e-9));
        assert!(close(many, 0.5, 1e-9));

        let sd_few = beta_posterior_sd(5, 5, PRIOR_GAMES);
        let sd_many = beta_posterior_sd(500, 500, PRIOR_GAMES);
        assert!(sd_few > sd_many * 4.0, "large sample must be far tighter");
    }

    #[test]
    fn wilson_interval_matches_known_values() {
        // Textbook check: 10 successes in 20 trials at 95% -> about (0.299, 0.701).
        let (lo, hi) = wilson_interval(10, 10, 1.959_964);
        assert!(close(lo, 0.299, 0.002), "lo={}", lo);
        assert!(close(hi, 0.701, 0.002), "hi={}", hi);

        // Zero successes must stay inside [0, 1] (where the normal approx fails).
        let (lo0, hi0) = wilson_interval(0, 10, 1.959_964);
        assert!(lo0 >= 0.0 && hi0 < 0.32, "({}, {})", lo0, hi0);
    }

    #[test]
    fn wilson_tightens_with_sample_size() {
        let (lo_small, hi_small) = wilson_interval(5, 5, Z_90);
        let (lo_big, hi_big) = wilson_interval(500, 500, Z_90);
        assert!((hi_small - lo_small) > (hi_big - lo_big) * 8.0);
    }

    #[test]
    fn rank_rating_is_monotonic() {
        let iron4 = rank_rating("IRON", "IV", 0).unwrap();
        assert!(close(iron4, RATING_FLOOR, 1e-9));
        // Silver IV should land near the ladder median used for unranked.
        let silver4 = rank_rating("SILVER", "IV", 0).unwrap();
        assert!(
            (silver4 - UNRANKED_RATING).abs() < 150.0,
            "silver4={}",
            silver4
        );
        let gold4 = rank_rating("GOLD", "IV", 0).unwrap();
        let gold1 = rank_rating("GOLD", "I", 75).unwrap();
        let emerald4 = rank_rating("EMERALD", "IV", 0).unwrap();
        let challenger = rank_rating("CHALLENGER", "I", 900).unwrap();
        assert!(iron4 < gold4 && gold4 < gold1 && gold1 < emerald4 && emerald4 < challenger);
        assert!(close(gold4 - iron4, 3.0 * TIER_POINTS, 1e-9));
        assert!(rank_rating("UNRANKED", "", 0).is_none());
    }

    #[test]
    fn elo_expectancy_is_symmetric_and_calibrated() {
        assert!(close(elo_expectancy(1500.0, 1500.0), 0.5, 1e-12));
        // A 400-point gap is 10:1 odds by construction.
        assert!(close(elo_expectancy(1900.0, 1500.0), 10.0 / 11.0, 1e-9));
        let p = elo_expectancy(1600.0, 1500.0);
        assert!(close(p + elo_expectancy(1500.0, 1600.0), 1.0, 1e-12));
    }

    #[test]
    fn log5_agrees_with_the_logit_path() {
        // Log5 of two rates equals the logistic on their rating deltas.
        let pa = 0.6;
        let pb = 0.45;
        let via_log5 = log5(pa, pb);
        let ra = winrate_rating_delta(pa);
        let rb = winrate_rating_delta(pb);
        let via_elo = elo_expectancy(ra, rb);
        assert!(
            close(via_log5, via_elo, 1e-9),
            "{} vs {}",
            via_log5,
            via_elo
        );
        assert!(close(log5(0.5, 0.5), 0.5, 1e-12));
    }

    #[test]
    fn winrate_delta_is_zero_at_the_baseline() {
        assert!(close(winrate_rating_delta(0.5), 0.0, 1e-12));
        assert!(winrate_rating_delta(0.6) > 0.0);
        assert!(winrate_rating_delta(0.4) < 0.0);
        assert!(close(
            winrate_rating_delta(0.6),
            -winrate_rating_delta(0.4),
            1e-9
        ));
    }

    #[test]
    fn sample_size_changes_strength_not_just_confidence() {
        let rank = rank_rating("GOLD", "II", 50);
        // Same 60% rate, very different sample sizes.
        let veteran = player_strength(rank, 600, 400, 12, 8);
        let rookie = player_strength(rank, 6, 4, 6, 4);
        assert!(
            veteran.rating > rookie.rating,
            "1000 games at 60% must outrank 10 games at 60% ({} vs {})",
            veteran.rating,
            rookie.rating
        );
        assert!(
            veteran.sigma < rookie.sigma,
            "the big sample must also be more certain"
        );
    }

    #[test]
    fn balanced_teams_land_near_even() {
        let rank = rank_rating("PLATINUM", "II", 40);
        let team: Vec<PlayerStrength> = (0..5)
            .map(|_| player_strength(rank, 100, 100, 10, 10))
            .collect();
        let p = team_win_probability(&team, &team);
        assert!(close(p.probability, 0.5, 1e-9));
        assert!(close(p.rating_gap, 0.0, 1e-9));
        assert_eq!(p.known_players, 10);
    }

    #[test]
    fn stronger_team_is_favoured_but_not_absurdly() {
        let high = rank_rating("DIAMOND", "IV", 0);
        let low = rank_rating("GOLD", "IV", 0);
        let allies: Vec<PlayerStrength> = (0..5)
            .map(|_| player_strength(high, 120, 80, 12, 8))
            .collect();
        let enemies: Vec<PlayerStrength> = (0..5)
            .map(|_| player_strength(low, 80, 120, 8, 12))
            .collect();
        let p = team_win_probability(&allies, &enemies);
        assert!(p.probability > 0.8, "got {}", p.probability);
        assert!(p.rating_gap > 0.0);
        assert!(p.low < p.probability && p.high > p.probability);
    }

    #[test]
    fn unknown_players_widen_the_interval() {
        let rank = rank_rating("SILVER", "I", 20);
        let known: Vec<PlayerStrength> = (0..5)
            .map(|_| player_strength(rank, 50, 50, 10, 10))
            .collect();
        let mut hidden = known.clone();
        hidden[0] = PlayerStrength::unknown();
        hidden[1] = PlayerStrength::unknown();

        let tight = team_win_probability(&known, &known);
        let loose = team_win_probability(&hidden, &known);
        assert!(
            (loose.high - loose.low) > (tight.high - tight.low),
            "hidden profiles must not fake precision"
        );
        assert_eq!(loose.known_players, 8);
        assert_eq!(loose.total_players, 10);
    }

    #[test]
    fn probability_stays_inside_the_interval() {
        let a = rank_rating("EMERALD", "III", 60);
        let b = rank_rating("PLATINUM", "I", 90);
        let allies: Vec<PlayerStrength> =
            (0..5).map(|_| player_strength(a, 33, 27, 11, 9)).collect();
        let enemies: Vec<PlayerStrength> =
            (0..5).map(|_| player_strength(b, 40, 40, 10, 10)).collect();
        let p = team_win_probability(&allies, &enemies);
        assert!(p.low <= p.probability && p.probability <= p.high);
        assert!(p.low >= 0.0 && p.high <= 1.0);
    }

    #[test]
    fn kda_and_participation_behave() {
        assert!(close(kda(6, 3, 6), 4.0, 1e-12));
        // Zero deaths reports the raw kill+assist count instead of dividing by 0.
        assert!(close(kda(5, 0, 5), 10.0, 1e-12));
        assert!(close(kill_participation(3, 5, 16), 0.5, 1e-12));
        assert!(close(kill_participation(3, 5, 0), 0.0, 1e-12));
        // Participation cannot exceed 100% even with odd inputs.
        assert!(close(kill_participation(10, 10, 5), 1.0, 1e-12));
    }

    #[test]
    fn per_minute_and_items_gold_are_exact() {
        // 128 CS in a 28:06 game (1686s) — matches the live-client sample.
        assert!(close(per_minute(128.0, 1686.0), 4.556, 0.001));
        assert!(close(per_minute(9002.0, 1686.0), 320.4, 0.1));
        assert!(close(per_minute(1.0, 0.0), 0.0, 1e-12));
        // Doran's Ring (400) + trinket (0) as reported by the live client.
        assert_eq!(items_gold(&[(400, 1), (0, 1)]), 400);
        assert_eq!(items_gold(&[(1300, 1), (900, 2)]), 3100);
    }

    #[test]
    fn contribution_ratio_measures_fair_share() {
        // 20% of the team total in a 5-man team is exactly a fair share.
        assert!(close(contribution_ratio(200.0, 1000.0, 5), 1.0, 1e-12));
        assert!(close(contribution_ratio(400.0, 1000.0, 5), 2.0, 1e-12));
        assert!(close(contribution_ratio(1.0, 0.0, 5), 0.0, 1e-12));
    }

    #[test]
    fn kill_damage_efficiency_flags_finishers_and_feeders() {
        // 40% of kills on 20% of the damage: a finisher.
        let ks = kill_damage_efficiency(4, 10, 200.0, 1000.0);
        assert!(close(ks, 2.0, 1e-9), "got {}", ks);
        // Half the damage but only a fifth of the kills: enabling others.
        let dealer = kill_damage_efficiency(2, 10, 500.0, 1000.0);
        assert!(close(dealer, 0.4, 1e-9), "got {}", dealer);
        // Missing data stays neutral instead of exploding.
        assert!(close(kill_damage_efficiency(0, 0, 0.0, 0.0), 1.0, 1e-12));
    }

    #[test]
    fn impact_score_separates_a_carry_from_a_passenger() {
        let base = ImpactInput {
            team_kills: 40,
            team_damage: 100_000.0,
            team_damage_taken: 100_000.0,
            team_gold: 60_000.0,
            team_vision: 100.0,
            duration_seconds: 1800.0,
            team_size: 5,
            ..Default::default()
        };
        let carry = ImpactInput {
            kills: 15,
            deaths: 3,
            assists: 10,
            damage_to_champions: 40_000.0,
            damage_taken: 20_000.0,
            gold: 18_000.0,
            cs: 250.0,
            vision_score: 30.0,
            ..base
        };
        let passenger = ImpactInput {
            kills: 1,
            deaths: 9,
            assists: 4,
            damage_to_champions: 8_000.0,
            damage_taken: 12_000.0,
            gold: 8_000.0,
            cs: 90.0,
            vision_score: 10.0,
            ..base
        };
        let cs_score = impact_score(&carry);
        let ps_score = impact_score(&passenger);
        assert!(cs_score > ps_score + 3.0, "{} vs {}", cs_score, ps_score);
        assert!((0.0..=10.0).contains(&cs_score));
        assert!((0.0..=10.0).contains(&ps_score));
        // An empty game must not panic or produce a negative score.
        assert!(close(impact_score(&ImpactInput::default()), 0.0, 1e-12));
    }

    #[test]
    fn premade_detection_merges_transitively() {
        // 0-1 and 1-2 both cleared the bar, so all three queued together.
        let pairs = [(0usize, 1usize, 6u32), (1, 2, 5), (3, 4, 1)];
        let groups = premade_groups(5, &pairs, 3);
        assert_eq!(groups, vec![vec![0, 1, 2]]);
    }

    #[test]
    fn premade_detection_respects_the_threshold() {
        // Two shared games is noise, not a duo.
        let pairs = [(0usize, 1usize, 2u32)];
        assert!(premade_groups(5, &pairs, 3).is_empty());
        let solo = premade_groups(5, &[], 3);
        assert!(solo.is_empty(), "no pairs means no premades");
        // Out-of-range indices are ignored rather than panicking.
        assert!(premade_groups(2, &[(0, 9, 10)], 3).is_empty());
    }

    #[test]
    fn premade_detection_finds_two_separate_duos() {
        let pairs = [(0usize, 1usize, 8u32), (2, 3, 9)];
        let groups = premade_groups(5, &pairs, 5);
        assert_eq!(groups, vec![vec![0, 1], vec![2, 3]]);
    }

    fn ids(values: &[&str]) -> Vec<Option<String>> {
        values
            .iter()
            .map(|v| {
                if v.is_empty() {
                    None
                } else {
                    Some(v.to_string())
                }
            })
            .collect()
    }

    #[test]
    fn party_ids_group_a_duo_and_a_trio_and_drop_solos() {
        // Solo queuers each carry their own party id.
        let groups = party_groups(&ids(&["p1", "p2", "p1", "p3", "p2", "p2"]));
        assert_eq!(groups, vec![vec![0, 2], vec![1, 4, 5]]);
    }

    #[test]
    fn missing_party_ids_are_never_grouped_together() {
        // Champ select carries no party id: absent must not read as "same party".
        assert!(party_groups(&ids(&["", "", "", "", ""])).is_empty());
        assert!(party_groups(&[]).is_empty());
        let groups = party_groups(&ids(&["", "p9", "", "p9"]));
        assert_eq!(groups, vec![vec![1, 3]]);
    }

    #[test]
    fn role_targets_reflect_role_reality() {
        let sup = default_targets("UTILITY");
        let adc = default_targets("BOTTOM");
        assert!(sup.cs_per_min < adc.cs_per_min);
        assert!(sup.vision_per_min > adc.vision_per_min);
        assert!(adc.damage_share > sup.damage_share);
        // Unknown roles fall back to a neutral profile rather than panicking.
        let unknown = default_targets("");
        assert!(unknown.cs_per_min > 0.0);
    }
}
