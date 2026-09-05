//! AI coach: the app's configured model reads the player's own data (match
//! detail, recent games, the live champ select) plus OP.GG's public MCP and
//! answers as a coach. Data is gathered here, in Rust, and handed to the model
//! in one compact prompt; there is no tool-calling loop, so cost and latency
//! stay predictable.

use super::{ensure_enabled, get_client, lcu_get_raw, LcuClient};
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// OP.GG MCP (https://mcp-api.op.gg/mcp, MIT). Plain JSON-RPC over HTTP with a
// session header; the tool result is JSON inside the first text content.
// ---------------------------------------------------------------------------

const OPGG_MCP: &str = "https://mcp-api.op.gg/mcp";
static OPGG_SESSION: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

static HTTP: Lazy<Option<reqwest::Client>> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(25))
        .build()
        .ok()
});

async fn opgg_init(http: &reqwest::Client) -> Result<String, String> {
    let response = http
        .post(OPGG_MCP)
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "omniget", "version": env!("CARGO_PKG_VERSION") }
            }
        }))
        .send()
        .await
        .map_err(|e| format!("op.gg mcp unreachable: {}", e))?;
    let session = response
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .ok_or_else(|| "op.gg mcp gave no session".to_string())?;
    Ok(session)
}

/// Pulls the tool result out of a JSON-RPC envelope.
pub fn opgg_result(envelope: &Value) -> Result<Value, String> {
    if let Some(err) = envelope.get("error") {
        let message = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        return Err(format!(
            "op.gg: {}",
            message.lines().next().unwrap_or(message)
        ));
    }
    let text = envelope
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(Value::as_array)
        .and_then(|c| c.first())
        .and_then(|c| c.get("text"))
        .and_then(Value::as_str)
        .ok_or_else(|| "op.gg: empty result".to_string())?;
    serde_json::from_str::<Value>(text).or_else(|_| Ok(json!(text)))
}

pub async fn opgg_call(tool: &str, args: Value) -> Result<Value, String> {
    let http = HTTP
        .clone()
        .ok_or_else(|| "http client init failed".to_string())?;
    for attempt in 0..2 {
        let session = {
            let cached = OPGG_SESSION.lock().await.clone();
            match cached {
                Some(s) => s,
                None => {
                    let s = opgg_init(&http).await?;
                    *OPGG_SESSION.lock().await = Some(s.clone());
                    s
                }
            }
        };
        let response = http
            .post(OPGG_MCP)
            .header("Accept", "application/json, text/event-stream")
            .header("Mcp-Session-Id", &session)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": { "name": tool, "arguments": args }
            }))
            .send()
            .await
            .map_err(|e| format!("op.gg mcp request failed: {}", e))?;
        let status = response.status();
        if (status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::BAD_REQUEST)
            && attempt == 0
        {
            // The session expired; start a fresh one and try once more.
            *OPGG_SESSION.lock().await = None;
            continue;
        }
        if !status.is_success() {
            return Err(format!("op.gg mcp {}", status.as_u16()));
        }
        let envelope = response
            .json::<Value>()
            .await
            .map_err(|e| format!("op.gg decode failed: {}", e))?;
        return opgg_result(&envelope);
    }
    Err("op.gg mcp session could not be established".to_string())
}

/// OP.GG wants champion names in UPPER_SNAKE_CASE ("KAI_SA" for Kai'Sa).
pub fn opgg_champion_name(name: &str) -> String {
    let mut out = String::new();
    let mut last_sep = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
            last_sep = false;
        } else if !last_sep {
            out.push('_');
            last_sep = true;
        }
    }
    out.trim_end_matches('_').to_string()
}

pub fn opgg_position(assigned: &str) -> &'static str {
    match assigned.to_ascii_uppercase().as_str() {
        "TOP" => "top",
        "JUNGLE" => "jungle",
        "MIDDLE" | "MID" => "mid",
        "BOTTOM" | "ADC" => "adc",
        "UTILITY" | "SUPPORT" => "support",
        _ => "all",
    }
}

async fn opgg_champion_brief(champion: &str, position: &str) -> Option<Value> {
    opgg_call(
        "lol_get_champion_analysis",
        json!({
            "game_mode": "ranked",
            "champion": champion,
            "position": position,
            "lang": "en_US",
            "desired_output_fields": [
                "data.summary.average_stats.{win_rate,pick_rate,ban_rate,tier,kda}",
                "data.core_items.{ids_names[],win}",
                "data.boots.{ids_names[],win}",
                "data.starter_items.ids_names[]",
                "data.summoner_spells.ids_names[]",
                "data.skill_masteries.ids[]",
                "data.runes.{primary_page_name,primary_rune_names[],secondary_page_name,secondary_rune_names[],stat_mod_names[]}",
                "data.weak_counters[].{champion_name,my_win_rate,play}",
                "data.strong_counters[].{champion_name,my_win_rate,play}",
                "data.damage_type"
            ]
        }),
    )
    .await
    .ok()
}

async fn opgg_matchup(mine: &str, theirs: &str, position: &str) -> Option<Value> {
    opgg_call(
        "lol_get_lane_matchup_guide",
        json!({
            "position": position,
            "my_champion": mine,
            "opponent_champion": theirs,
            "lang": "en_US"
        }),
    )
    .await
    .ok()
    .map(|v| {
        // The guide is large; keep only what a coach would quote.
        let data = v.get("data").cloned().unwrap_or(v);
        json!({
            "summary": data.get("summary").and_then(|s| s.get("average_stats")).cloned(),
            "core_items": data.get("core_items").cloned(),
            "runes": data.get("runes").cloned(),
            "matchup_tips": data.get("matchup_tips").or_else(|| data.get("tips")).cloned(),
        })
    })
}

// ---------------------------------------------------------------------------
// Prompts
// ---------------------------------------------------------------------------

pub fn style_instruction(style: &str) -> &'static str {
    match style {
        "roast" => "Be brutally honest and funny, roast the mistakes without mercy, but every jab must carry a real, specific lesson.",
        "praise" => "Be warm and encouraging: lead with what went well, frame every problem as the next thing to improve.",
        _ => "Be objective and direct, like a professional analyst: facts, numbers, priorities.",
    }
}

fn system_prompt(style: &str, language: &str) -> String {
    format!(
        "You are a League of Legends coach reading a player's own match data.\n\
         {}\n\
         Answer in {}. Use short paragraphs and bullet points; no headings larger than a line.\n\
         Only use numbers that appear in the data; never invent stats, builds or patch facts.\n\
         When OP.GG data is included, treat it as the current meta reference and cite it as such.\n\
         Keep the whole answer under 350 words unless the user asks for depth.",
        style_instruction(style),
        language
    )
}

/// A participant trimmed to what a coach reads, from the match detail shape.
pub fn compact_participant(p: &Value, minutes: f64) -> Value {
    let f = |k: &str| p.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    let per_min = |v: f64| {
        if minutes > 0.0 {
            (v / minutes * 10.0).round() / 10.0
        } else {
            0.0
        }
    };
    json!({
        "player": p.get("gameName").and_then(Value::as_str).unwrap_or(""),
        "championId": p.get("championId"),
        "team": p.get("teamId"),
        "win": p.get("win"),
        "kda": format!("{}/{}/{}", f("kills"), f("deaths"), f("assists")),
        "level": p.get("level"),
        "csPerMin": per_min(f("cs")),
        "goldPerMin": per_min(f("gold")).round(),
        "damageToChampions": f("damageToChampions"),
        "damageTaken": f("damageTaken"),
        "visionScore": f("visionScore"),
        "wards": format!("{}/{}/{}", f("wardsPlaced"), f("wardsKilled"), f("controlWards")),
        "objectiveDamage": f("damageToObjectives"),
        "ccSeconds": f("ccTime"),
        "firstBlood": p.get("firstBlood"),
        "multikill": p.get("largestMultiKill"),
    })
}

fn champion_name(champions: &Value, id: i64) -> String {
    champions
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .find(|c| c.get("id").and_then(Value::as_i64) == Some(id))
                .and_then(|c| c.get("name").and_then(Value::as_str))
        })
        .unwrap_or("?")
        .to_string()
}

async fn champion_summary(client: &LcuClient) -> Value {
    lcu_get_raw(client, "/lol-game-data/assets/v1/champion-summary.json")
        .await
        .unwrap_or(Value::Null)
}

fn name_champions(mut value: Value, champions: &Value) -> Value {
    fn walk(v: &mut Value, champions: &Value) {
        match v {
            Value::Object(map) => {
                if let Some(id) = map.get("championId").and_then(Value::as_i64) {
                    map.insert("champion".to_string(), json!(champion_name(champions, id)));
                    map.remove("championId");
                }
                for (_, child) in map.iter_mut() {
                    walk(child, champions);
                }
            }
            Value::Array(arr) => {
                for child in arr.iter_mut() {
                    walk(child, champions);
                }
            }
            _ => {}
        }
    }
    walk(&mut value, champions);
    value
}

fn prompt_json(value: &Value, cap: usize) -> String {
    let mut text = serde_json::to_string(value).unwrap_or_default();
    if text.len() > cap {
        text.truncate(cap);
        text.push('…');
    }
    text
}

/// Post-game review of one match.
#[tauri::command]
pub async fn league_coach_review(
    game_id: i64,
    style: Option<String>,
    language: Option<String>,
) -> Result<String, String> {
    ensure_enabled()?;
    let client = get_client().await?;
    let detail = super::league_match_detail(game_id).await?;
    let me = lcu_get_raw(&client, "/lol-summoner/v1/current-summoner").await?;
    let my_puuid = me
        .get("puuid")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let duration = detail
        .get("gameDuration")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let minutes = duration / 60.0;
    let champions = champion_summary(&client).await;
    let participants: Vec<&Value> = detail
        .get("participants")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let mine = participants
        .iter()
        .find(|p| p.get("puuid").and_then(Value::as_str) == Some(my_puuid.as_str()))
        .map(|p| compact_participant(p, minutes))
        .unwrap_or(Value::Null);
    let others: Vec<Value> = participants
        .iter()
        .filter(|p| p.get("puuid").and_then(Value::as_str) != Some(my_puuid.as_str()))
        .map(|p| compact_participant(p, minutes))
        .collect();
    let context = name_champions(
        json!({
            "queueId": detail.get("queueId"),
            "gameMode": detail.get("gameMode"),
            "durationMinutes": (minutes * 10.0).round() / 10.0,
            "teams": detail.get("teams"),
            "me": mine,
            "others": others,
            "myRunes": detail.get("participants").and_then(Value::as_array).and_then(|a| {
                a.iter().find(|p| p.get("puuid").and_then(Value::as_str) == Some(my_puuid.as_str()))
                    .and_then(|p| p.get("runes").cloned())
            }),
        }),
        &champions,
    );
    let style = style.unwrap_or_else(|| super::league_settings().coach_style);
    let user = format!(
        "Review this game for the player marked as \"me\". Give: the result in one line, \
         the 3 biggest mistakes or missed opportunities with the numbers that show them, \
         what was done well, and one concrete goal for the next game.\n\nDATA:\n{}",
        prompt_json(&context, 14_000)
    );
    let system = system_prompt(&style, language.as_deref().unwrap_or("English"));
    omniget_core::core::ai::chat(&system, &user).await
}

/// One row per recent game, plus aggregates, so the model sees trends and
/// not just a wall of numbers.
pub fn trend_rows(games: &[Value], puuid: &str, champions: &Value) -> Value {
    let mut rows: Vec<Value> = Vec::new();
    let mut wins = 0usize;
    let mut by_champion: std::collections::HashMap<String, (usize, usize)> =
        std::collections::HashMap::new();
    let mut by_role: std::collections::HashMap<String, (usize, usize)> =
        std::collections::HashMap::new();
    let (mut k, mut d, mut a, mut cs, mut vision, mut minutes) =
        (0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
    for game in games {
        let participants = match game.get("participants").and_then(Value::as_array) {
            Some(p) => p,
            None => continue,
        };
        let me = if participants.len() == 1 {
            participants.first()
        } else {
            let identities = game.get("participantIdentities").and_then(Value::as_array);
            let pid = identities.and_then(|ids| {
                ids.iter()
                    .find(|i| {
                        i.get("player")
                            .and_then(|p| p.get("puuid"))
                            .and_then(Value::as_str)
                            == Some(puuid)
                    })
                    .and_then(|i| i.get("participantId").and_then(Value::as_i64))
            });
            pid.and_then(|pid| {
                participants
                    .iter()
                    .find(|p| p.get("participantId").and_then(Value::as_i64) == Some(pid))
            })
            .or_else(|| participants.first())
        };
        let me = match me {
            Some(m) => m,
            None => continue,
        };
        let stats = me.get("stats").cloned().unwrap_or(Value::Null);
        let f = |key: &str| stats.get(key).and_then(Value::as_f64).unwrap_or(0.0);
        let win = stats.get("win").and_then(Value::as_bool).unwrap_or(false);
        let duration = game
            .get("gameDuration")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let mins = duration / 60.0;
        let champ = champion_name(
            champions,
            me.get("championId").and_then(Value::as_i64).unwrap_or(0),
        );
        let role = me
            .get("timeline")
            .and_then(|t| t.get("teamPosition").or_else(|| t.get("lane")))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let game_cs = f("totalMinionsKilled") + f("neutralMinionsKilled");
        if win {
            wins += 1;
        }
        let entry = by_champion.entry(champ.clone()).or_insert((0, 0));
        entry.0 += 1;
        if win {
            entry.1 += 1;
        }
        if !role.is_empty() {
            let entry = by_role.entry(role.clone()).or_insert((0, 0));
            entry.0 += 1;
            if win {
                entry.1 += 1;
            }
        }
        k += f("kills");
        d += f("deaths");
        a += f("assists");
        cs += game_cs;
        vision += f("visionScore");
        minutes += mins;
        rows.push(json!({
            "champion": champ,
            "role": role,
            "queueId": game.get("queueId"),
            "win": win,
            "kda": format!("{}/{}/{}", f("kills"), f("deaths"), f("assists")),
            "csPerMin": if mins > 0.0 { (game_cs / mins * 10.0).round() / 10.0 } else { 0.0 },
            "goldPerMin": if mins > 0.0 { (f("goldEarned") / mins).round() } else { 0.0 },
            "visionPerMin": if mins > 0.0 { (f("visionScore") / mins * 100.0).round() / 100.0 } else { 0.0 },
            "damageToChampions": f("totalDamageDealtToChampions"),
            "minutes": (mins * 10.0).round() / 10.0,
        }));
    }
    let total = rows.len().max(1) as f64;
    let mut champion_table: Vec<Value> = by_champion
        .into_iter()
        .map(|(name, (g, w))| json!({ "champion": name, "games": g, "wins": w }))
        .collect();
    champion_table.sort_by(|x, y| y["games"].as_u64().cmp(&x["games"].as_u64()));
    let role_table: Vec<Value> = by_role
        .into_iter()
        .map(|(name, (g, w))| json!({ "role": name, "games": g, "wins": w }))
        .collect();
    json!({
        "games": rows.len(),
        "wins": wins,
        "winrate": ((wins as f64 / total) * 1000.0).round() / 10.0,
        "avgKda": format!("{:.1}/{:.1}/{:.1}", k / total, d / total, a / total),
        "kdaRatio": if d > 0.0 { ((k + a) / d * 100.0).round() / 100.0 } else { k + a },
        "csPerMin": if minutes > 0.0 { (cs / minutes * 10.0).round() / 10.0 } else { 0.0 },
        "visionPerMin": if minutes > 0.0 { (vision / minutes * 100.0).round() / 100.0 } else { 0.0 },
        "byChampion": champion_table.into_iter().take(8).collect::<Vec<_>>(),
        "byRole": role_table,
        "recentFirst": rows,
    })
}

/// Trends over the last N games of the current player.
#[tauri::command]
pub async fn league_coach_trends(
    count: Option<u32>,
    style: Option<String>,
    language: Option<String>,
) -> Result<String, String> {
    ensure_enabled()?;
    let client = get_client().await?;
    let count = count.unwrap_or(20).clamp(5, 40);
    let me = lcu_get_raw(&client, "/lol-summoner/v1/current-summoner").await?;
    let puuid = me
        .get("puuid")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let history = lcu_get_raw(
        &client,
        &format!(
            "/lol-match-history/v1/products/lol/current-summoner/matches?begIndex=0&endIndex={}",
            count.saturating_sub(1)
        ),
    )
    .await?;
    let games: Vec<Value> = history
        .get("games")
        .and_then(|g| g.get("games"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if games.is_empty() {
        return Err("no recent games".to_string());
    }
    let champions = champion_summary(&client).await;
    let context = trend_rows(&games, &puuid, &champions);
    let style = style.unwrap_or_else(|| super::league_settings().coach_style);
    let user = format!(
        "Here are my last {} games (most recent first) with aggregates. Find my strengths, \
         my 3 most costly habits, which champions and roles I should play more or less, \
         and a one-week improvement plan with measurable targets.\n\nDATA:\n{}",
        games.len(),
        prompt_json(&context, 14_000)
    );
    let system = system_prompt(&style, language.as_deref().unwrap_or("English"));
    omniget_core::core::ai::chat(&system, &user).await
}

/// What the client is doing right now, in words the model can use.
async fn live_context(client: &LcuClient, champions: &Value) -> Value {
    let phase = lcu_get_raw(client, "/lol-gameflow/v1/gameflow-phase")
        .await
        .ok()
        .and_then(|p| p.as_str().map(str::to_string))
        .unwrap_or_default();
    let mut context = json!({ "phase": phase });
    if phase == "ChampSelect" {
        if let Ok(session) = lcu_get_raw(client, "/lol-champ-select/v1/session").await {
            let cell = session
                .get("localPlayerCellId")
                .and_then(Value::as_i64)
                .unwrap_or(-1);
            let team = |key: &str| -> Vec<Value> {
                session
                    .get(key)
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .map(|m| {
                                let id = m.get("championId").and_then(Value::as_i64).unwrap_or(0);
                                json!({
                                    "champion": if id > 0 { champion_name(champions, id) } else { "?".to_string() },
                                    "position": m.get("assignedPosition").and_then(Value::as_str).unwrap_or(""),
                                    "me": m.get("cellId").and_then(Value::as_i64) == Some(cell),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let allies = team("myTeam");
            let enemies = team("theirTeam");
            let me = allies
                .iter()
                .find(|m| m.get("me") == Some(&json!(true)))
                .cloned();
            let my_champ = me
                .as_ref()
                .and_then(|m| m.get("champion").and_then(Value::as_str))
                .unwrap_or("?")
                .to_string();
            let my_pos = me
                .as_ref()
                .and_then(|m| m.get("position").and_then(Value::as_str))
                .unwrap_or("")
                .to_string();
            context["allies"] = json!(allies);
            context["enemies"] = json!(enemies);
            if my_champ != "?" {
                let position = opgg_position(&my_pos);
                let champ_key = opgg_champion_name(&my_champ);
                if let Some(brief) = opgg_champion_brief(&champ_key, position).await {
                    context["opgg"] = brief;
                }
                // The lane opponent is only known once both sides show a champion
                // in the same position; enemies do not expose positions, so the
                // first enemy champion is offered when the position is a lane.
                if position != "all" {
                    if let Some(enemy) = enemies
                        .iter()
                        .find_map(|e| e.get("champion").and_then(Value::as_str))
                        .filter(|c| *c != "?")
                    {
                        if let Some(guide) =
                            opgg_matchup(&champ_key, &opgg_champion_name(enemy), position).await
                        {
                            context["opggMatchupSample"] =
                                json!({ "opponent": enemy, "guide": guide });
                        }
                    }
                }
            }
        }
    } else if phase == "InProgress" {
        if let Ok(live) = super::league_live_game().await {
            context["live"] = json!({
                "gameTime": live.get("stats").and_then(|s| s.get("gameTime")),
                "activePlayer": live.get("activePlayer"),
                "players": live.get("players").and_then(Value::as_array).map(|arr| {
                    arr.iter().map(|p| json!({
                        "champion": p.get("championName"),
                        "team": p.get("team"),
                        "riotId": p.get("riotId").or_else(|| p.get("summonerName")),
                        "kda": p.get("scores"),
                        "level": p.get("level"),
                        "dead": p.get("isDead"),
                    })).collect::<Vec<_>>()
                }),
            });
        }
        if let Ok(metrics) = super::league_live_metrics().await {
            context["liveMetrics"] = metrics;
        }
    }
    context
}

/// Free question, answered against whatever the client is doing right now.
#[tauri::command]
pub async fn league_coach_ask(
    question: String,
    style: Option<String>,
    language: Option<String>,
) -> Result<String, String> {
    ensure_enabled()?;
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("empty question".to_string());
    }
    if question.chars().count() > 2_000 {
        return Err("question too long".to_string());
    }
    let client = get_client().await?;
    let champions = champion_summary(&client).await;
    let mut context = live_context(&client, &champions).await;
    // Outside a game the last few results give the model something to anchor on.
    if context
        .get("phase")
        .and_then(Value::as_str)
        .map(|p| p != "ChampSelect" && p != "InProgress")
        .unwrap_or(true)
    {
        if let Ok(me) = lcu_get_raw(&client, "/lol-summoner/v1/current-summoner").await {
            let puuid = me
                .get("puuid")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if let Ok(history) = lcu_get_raw(
                &client,
                "/lol-match-history/v1/products/lol/current-summoner/matches?begIndex=0&endIndex=7",
            )
            .await
            {
                let games: Vec<Value> = history
                    .get("games")
                    .and_then(|g| g.get("games"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                context["recent"] = trend_rows(&games, &puuid, &champions);
            }
        }
    }
    let style = style.unwrap_or_else(|| super::league_settings().coach_style);
    let user = format!(
        "QUESTION: {}\n\nCONTEXT (what the client is doing right now, plus OP.GG meta data when present):\n{}",
        question,
        prompt_json(&context, 16_000)
    );
    let system = system_prompt(&style, language.as_deref().unwrap_or("English"));
    omniget_core::core::ai::chat(&system, &user).await
}

/// Cheap check the tab uses to decide what to show.
#[tauri::command]
pub fn league_coach_ready() -> bool {
    omniget_core::core::ai::get().is_configured()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn champion_names_become_op_gg_keys() {
        assert_eq!(opgg_champion_name("Kai'Sa"), "KAI_SA");
        assert_eq!(opgg_champion_name("Miss Fortune"), "MISS_FORTUNE");
        assert_eq!(opgg_champion_name("Garen"), "GAREN");
        assert_eq!(opgg_champion_name("Nunu & Willump"), "NUNU_WILLUMP");
        assert_eq!(opgg_position("BOTTOM"), "adc");
        assert_eq!(opgg_position("utility"), "support");
        assert_eq!(opgg_position(""), "all");
    }

    #[test]
    fn the_tool_result_is_unwrapped_and_errors_surface() {
        let ok = json!({ "result": { "content": [{ "type": "text", "text": "{\"a\":1}" }] } });
        assert_eq!(opgg_result(&ok).unwrap(), json!({ "a": 1 }));
        let plain = json!({ "result": { "content": [{ "type": "text", "text": "hello" }] } });
        assert_eq!(opgg_result(&plain).unwrap(), json!("hello"));
        let err = json!({ "error": { "code": -1, "message": "HTTP 422:\nmore" } });
        assert_eq!(opgg_result(&err).unwrap_err(), "op.gg: HTTP 422:");
    }

    #[test]
    fn trend_rows_aggregate_wins_and_per_minute_rates() {
        let champions = json!([{ "id": 1, "name": "Annie" }, { "id": 2, "name": "Olaf" }]);
        let game = |champ: i64, win: bool, kills: f64| {
            json!({
                "gameDuration": 1800, "queueId": 420,
                "participantIdentities": [{ "participantId": 1, "player": { "puuid": "me" } }],
                "participants": [{ "participantId": 1, "championId": champ,
                    "timeline": { "teamPosition": "MIDDLE" },
                    "stats": { "win": win, "kills": kills, "deaths": 2, "assists": 4, "totalMinionsKilled": 210, "neutralMinionsKilled": 0, "goldEarned": 12000, "visionScore": 30 } }]
            })
        };
        let games = vec![game(1, true, 6.0), game(1, false, 2.0), game(2, true, 8.0)];
        let rows = trend_rows(&games, "me", &champions);
        assert_eq!(rows["games"], 3);
        assert_eq!(rows["wins"], 2);
        assert_eq!(rows["winrate"], 66.7);
        assert_eq!(rows["csPerMin"], 7.0);
        assert_eq!(rows["byChampion"][0]["champion"], "Annie");
        assert_eq!(rows["byChampion"][0]["games"], 2);
        assert_eq!(rows["byRole"][0]["role"], "MIDDLE");
        assert_eq!(rows["recentFirst"][0]["kda"], "6/2/4");
        assert_eq!(rows["recentFirst"][0]["goldPerMin"], 400.0);
    }

    #[test]
    fn a_participant_is_compacted_with_per_minute_rates() {
        let p = json!({ "gameName": "Me", "championId": 7, "teamId": 100, "win": true, "kills": 5, "deaths": 1, "assists": 9,
            "cs": 240, "gold": 15000, "damageToChampions": 20000, "visionScore": 40, "wardsPlaced": 10, "wardsKilled": 3, "controlWards": 2 });
        let c = compact_participant(&p, 30.0);
        assert_eq!(c["kda"], "5/1/9");
        assert_eq!(c["csPerMin"], 8.0);
        assert_eq!(c["goldPerMin"], 500.0);
        assert_eq!(c["wards"], "10/3/2");
        let named = name_champions(json!({ "me": c }), &json!([{ "id": 7, "name": "Leona" }]));
        assert_eq!(named["me"]["champion"], "Leona");
        assert!(named["me"].get("championId").is_none());
    }

    #[test]
    fn styles_change_the_instruction() {
        assert!(style_instruction("roast").contains("roast"));
        assert!(style_instruction("praise").contains("encouraging"));
        assert!(style_instruction("anything").contains("objective"));
    }
}
