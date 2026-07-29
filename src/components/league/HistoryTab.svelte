<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t, locale } from "$lib/i18n";
  import timeAgo from "$lib/time-ago";
  import { CDRAGON, type Champion } from "./shared";
  import { filterByQueue, queuesInGames, summarise } from "$lib/league-history";
  import { findTeam, teamBans, teamObjectives } from "$lib/league-match-detail";

  let {
    games,
    loading,
    onRefresh,
    championById,
  }: {
    games: any[];
    loading: boolean;
    onRefresh: () => void;
    championById: Map<number, Champion>;
  } = $props();

  const QUEUE_NAMES: Record<number, string> = {
    420: "Solo/Duo",
    440: "Flex",
    400: "Draft",
    430: "Blind",
    450: "ARAM",
    480: "Swiftplay",
    900: "URF",
    1700: "Arena",
  };

  function queueName(id: number, mode: string): string {
    return QUEUE_NAMES[id] ?? mode ?? "";
  }

  let queueFilter = $state<number | null>(null);
  let availableQueues = $derived(queuesInGames(games));
  let visibleGames = $derived(filterByQueue(games, queueFilter));
  let summary = $derived(summarise(visibleGames));

  $effect(() => {
    // A filter kept after a refresh that no longer has the queue would show an
    // empty list with no explanation.
    if (queueFilter !== null && !availableQueues.includes(queueFilter)) {
      queueFilter = null;
    }
  });

  function playerStats(game: any): { championId: number; kills: number; deaths: number; assists: number; win: boolean } {
    const p = game?.participants?.[0];
    return {
      championId: p?.championId ?? 0,
      kills: p?.stats?.kills ?? 0,
      deaths: p?.stats?.deaths ?? 0,
      assists: p?.stats?.assists ?? 0,
      win: p?.stats?.win ?? false,
    };
  }

  let expandedGame = $state<number | null>(null);
  let gameDetails = $state<Record<number, any>>({});
  let gameDetailLoading = $state<number | null>(null);

  async function toggleGameDetail(gameId: number) {
    if (expandedGame === gameId) {
      expandedGame = null;
      return;
    }
    expandedGame = gameId;
    if (gameDetails[gameId]) return;
    gameDetailLoading = gameId;
    try {
      const detail = await invoke<any>("league_get", {
        path: `/lol-match-history/v1/games/${gameId}`,
      });
      gameDetails = { ...gameDetails, [gameId]: detail };
    } catch {
      gameDetails = { ...gameDetails, [gameId]: null };
    } finally {
      gameDetailLoading = null;
    }
  }

  function scoreboardTeams(detail: any): { teamId: number; players: any[] }[] {
    const participants: any[] = detail?.participants ?? [];
    const identities: any[] = detail?.participantIdentities ?? [];
    const nameOf = (pid: number): string => {
      const player = identities.find((i) => i.participantId === pid)?.player;
      if (!player) return "";
      return player.gameName || player.summonerName || "";
    };
    const rows = participants.map((p) => ({
      participantId: p.participantId,
      teamId: p.teamId,
      championId: p.championId,
      name: nameOf(p.participantId),
      kills: p.stats?.kills ?? 0,
      deaths: p.stats?.deaths ?? 0,
      assists: p.stats?.assists ?? 0,
      cs: (p.stats?.totalMinionsKilled ?? 0) + (p.stats?.neutralMinionsKilled ?? 0),
      gold: p.stats?.goldEarned ?? 0,
      damage: p.stats?.totalDamageDealtToChampions ?? 0,
      win: p.stats?.win ?? false,
    }));
    const teamIds = [...new Set(rows.map((r) => r.teamId))];
    return teamIds.map((teamId) => ({ teamId, players: rows.filter((r) => r.teamId === teamId) }));
  }
</script>

<section class="history-section">
  <div class="history-head">
    <h3>{$t("league.history_title")}</h3>
    <button class="button" onclick={onRefresh} disabled={loading}>{$t("league.refresh")}</button>
  </div>
  {#if games.length === 0}
    <p class="empty-hint">{$t("league.history_empty")}</p>
  {:else}
    {#if availableQueues.length > 1}
      <div class="queue-filter" role="group" aria-label={$t("league.filter_queue") as string}>
        <button class="queue-chip" class:on={queueFilter === null} onclick={() => (queueFilter = null)} aria-pressed={queueFilter === null}>
          {$t("league.filter_all")}
        </button>
        {#each availableQueues as id (id)}
          <button class="queue-chip" class:on={queueFilter === id} onclick={() => (queueFilter = id)} aria-pressed={queueFilter === id}>
            {queueName(id, "")}
          </button>
        {/each}
      </div>
    {/if}
    <p class="history-summary">
      {summary.counted}
      {$t("league.summary_games")}
      {#if summary.winrate !== null}
        · <strong>{summary.wins}{$t("league.summary_win_short")} {summary.losses}{$t("league.summary_loss_short")}</strong> · {summary.winrate}%
      {/if}
      {#if summary.kda !== null}
        · KDA {summary.kda.toFixed(2)}
      {/if}
      {#if summary.remakes > 0}
        · <span class="dim">{summary.remakes} {$t("league.summary_remakes")}</span>
      {/if}
    </p>
    <div class="game-list">
      {#each visibleGames as game (game.gameId)}
        {@const p = playerStats(game)}
        <button
          class="game-row"
          class:expanded={expandedGame === game.gameId}
          onclick={() => toggleGameDetail(game.gameId)}
          aria-expanded={expandedGame === game.gameId}
        >
          <img class="champ-icon" src={`${CDRAGON}/champion-icons/${p.championId}.png`} alt="" loading="lazy" />
          <div class="game-info">
            <span class="game-result" class:win={p.win} class:loss={!p.win}>{p.win ? $t("league.victory") : $t("league.defeat")}</span>
            <span class="game-mode">{queueName(game.queueId, game.gameMode)}</span>
          </div>
          <span class="game-kda">{p.kills} / {p.deaths} / {p.assists}</span>
          <span class="game-time">{timeAgo(game.gameCreation, $locale)}</span>
          <span class="game-chevron" aria-hidden="true">{expandedGame === game.gameId ? "▾" : "▸"}</span>
        </button>
        {#if expandedGame === game.gameId}
          {#if gameDetailLoading === game.gameId}
            <p class="empty-hint">…</p>
          {:else if gameDetails[game.gameId]}
            <div class="scoreboard">
              {#each scoreboardTeams(gameDetails[game.gameId]) as team (team.teamId)}
                {@const objectives = teamObjectives(findTeam(gameDetails[game.gameId]?.teams, team.teamId))}
                {@const bans = teamBans(findTeam(gameDetails[game.gameId]?.teams, team.teamId))}
                <div class="scoreboard-team">
                  <span class="scoreboard-result" class:win={team.players[0]?.win} class:loss={!team.players[0]?.win}>
                    {team.players[0]?.win ? $t("league.victory") : $t("league.defeat")}
                  </span>
                  {#if objectives}
                    <div class="objective-line">
                      <span>{$t("league.obj_towers")} <strong>{objectives.towers}</strong></span>
                      <span>{$t("league.obj_inhibitors")} <strong>{objectives.inhibitors}</strong></span>
                      <span>{$t("league.objective_baron")} <strong>{objectives.barons}</strong></span>
                      <span>{$t("league.objective_dragon")} <strong>{objectives.dragons}</strong></span>
                      <span>{$t("league.obj_heralds")} <strong>{objectives.heralds}</strong></span>
                    </div>
                  {/if}
                  {#if bans.length > 0}
                    <div class="ban-line">
                      <span class="dim">{$t("league.obj_bans")}</span>
                      {#each bans as banId, i (`${banId}-${i}`)}
                        <img
                          class="champ-icon tiny"
                          src={`${CDRAGON}/champion-icons/${banId}.png`}
                          alt=""
                          title={championById.get(banId)?.name ?? ""}
                          loading="lazy"
                        />
                      {/each}
                    </div>
                  {/if}
                  {#each team.players as sp (sp.participantId)}
                    <div class="scoreboard-row">
                      <img class="champ-icon tiny" src={`${CDRAGON}/champion-icons/${sp.championId}.png`} alt="" loading="lazy" />
                      <span class="scoreboard-name">{sp.name || championById.get(sp.championId)?.name || "—"}</span>
                      <span class="scoreboard-kda">{sp.kills}/{sp.deaths}/{sp.assists}</span>
                      <span class="scoreboard-cs dim">{sp.cs} CS</span>
                      <span class="scoreboard-gold dim">{(sp.gold / 1000).toFixed(1)}k</span>
                      <span class="scoreboard-dmg dim">{(sp.damage / 1000).toFixed(1)}k {$t("league.match_damage")}</span>
                    </div>
                  {/each}
                </div>
              {/each}
            </div>
          {:else}
            <p class="empty-hint">{$t("league.match_detail_unavailable")}</p>
          {/if}
        {/if}
      {/each}
    </div>
  {/if}
</section>
