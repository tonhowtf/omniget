<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t, locale } from "$lib/i18n";
  import timeAgo from "$lib/time-ago";
  import { CDRAGON, assetUrl, type Champion } from "./shared";
  import { filterByQueue, queuesInGames, summarise } from "$lib/league-history";
  import { findTeam, teamBans, teamObjectives } from "$lib/league-match-detail";
  import Skeleton from "./Skeleton.svelte";

  let {
    games,
    loading,
    onRefresh,
    championById,
    active,
  }: {
    games: any[];
    loading: boolean;
    onRefresh: () => void;
    championById: Map<number, Champion>;
    active?: boolean;
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
      const detail = await invoke<any>("league_match_detail", { gameId });
      gameDetails = { ...gameDetails, [gameId]: detail };
      expandedDuration = detail?.gameDuration ?? 0;
      loadPerkIcons();
    } catch {
      gameDetails = { ...gameDetails, [gameId]: null };
    } finally {
      gameDetailLoading = null;
    }
  }

  let perkIcons = $state<Record<number, string>>({});
  let lookupPuuid = $state("");
  let lookupName = $state("");
  let lookupGames = $state<any[]>([]);
  let lookupLoading = $state(false);
  let lookupError = $state("");

  async function loadPerkIcons() {
    if (Object.keys(perkIcons).length > 0) return;
    try {
      const res = await invoke<any>("league_perks");
      const map: Record<number, string> = {};
      for (const perk of res?.perks ?? []) {
        if (perk?.id && perk?.iconPath) map[perk.id] = assetUrl(perk.iconPath);
      }
      perkIcons = map;
    } catch {
      perkIcons = {};
    }
  }

  async function openPlayer(player: any) {
    if (!player?.puuid) return;
    lookupPuuid = player.puuid;
    lookupName = player.tagLine ? `${player.gameName}#${player.tagLine}` : player.gameName;
    lookupGames = [];
    lookupError = "";
    lookupLoading = true;
    try {
      const res = await invoke<any>("league_player_history", {
        puuid: player.puuid,
        begIndex: 0,
        endIndex: 9,
      });
      lookupGames = res?.games?.games ?? [];
    } catch (e: any) {
      lookupError = typeof e === "string" ? e : (e?.message ?? String(e));
    } finally {
      lookupLoading = false;
    }
  }

  function closeLookup() {
    lookupPuuid = "";
    lookupGames = [];
    lookupError = "";
  }

  function statLine(p: any): { key: string; label: string; value: string }[] {
    const minutes = Math.max((expandedDuration || 1) / 60, 0.1);
    return [
      { key: "dmg", label: "league.stat_damage", value: `${(p.damageToChampions / 1000).toFixed(1)}k` },
      { key: "taken", label: "league.stat_taken", value: `${(p.damageTaken / 1000).toFixed(1)}k` },
      { key: "mitigated", label: "league.stat_mitigated", value: `${(p.damageMitigated / 1000).toFixed(1)}k` },
      { key: "heal", label: "league.stat_healing", value: `${(p.healing / 1000).toFixed(1)}k` },
      { key: "obj", label: "league.stat_objectives", value: `${(p.damageToObjectives / 1000).toFixed(1)}k` },
      { key: "gpm", label: "league.stat_gold_min", value: Math.round(p.gold / minutes).toString() },
      { key: "cspm", label: "league.stat_cs_min", value: (p.cs / minutes).toFixed(1) },
      { key: "vision", label: "league.stat_vision", value: String(p.visionScore) },
      { key: "wards", label: "league.stat_wards", value: `${p.wardsPlaced}/${p.wardsKilled}/${p.controlWards}` },
      { key: "cc", label: "league.stat_cc", value: `${p.ccTime}s` },
      { key: "spree", label: "league.stat_spree", value: String(p.largestSpree) },
    ];
  }

  let expandedDuration = $state(0);

  function scoreboardTeams(detail: any): { teamId: number; players: any[] }[] {
    const rows: any[] = detail?.participants ?? [];
    const teamIds = [...new Set(rows.map((r) => r.teamId))];
    return teamIds.map((teamId) => ({ teamId, players: rows.filter((r) => r.teamId === teamId) }));
  }

  function itemIcon(id: number): string {
    return `${CDRAGON}/../../game/assets/items/icons2d/${id}.png`;
  }

  function spellIcon(id: number): string {
    return `${CDRAGON}/summoner-spells/${id}.png`;
  }
</script>

{#if active !== false}
  <section class="history-section">
    <div class="history-head">
      <h3>{$t("league.history_title")}</h3>
      <button class="button" onclick={onRefresh} disabled={loading}>{$t("league.refresh")}</button>
    </div>
    {#if loading && games.length === 0}
      <div class="game-list" aria-busy="true">
        {#each Array(4) as _, i (i)}
          <div class="game-row-skeleton">
            <Skeleton w="34px" h="34px" round="6px" />
            <div class="game-skeleton-info">
              <Skeleton w="88px" h="13px" />
              <Skeleton w="64px" h="11px" />
            </div>
            <Skeleton w="72px" h="13px" />
            <Skeleton w="56px" h="11px" />
          </div>
        {/each}
      </div>
    {:else if games.length === 0}
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
              <div class="scoreboard" aria-busy="true">
                {#each Array(2) as _, ti (ti)}
                  <div class="scoreboard-team">
                    <Skeleton w="72px" h="12px" />
                    {#each Array(5) as _, si (si)}
                      <div class="scoreboard-row">
                        <Skeleton w="22px" h="22px" round="5px" />
                        <Skeleton w="120px" h="12px" />
                        <Skeleton w="52px" h="12px" />
                      </div>
                    {/each}
                  </div>
                {/each}
              </div>
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
                      <div class="scoreboard-row full">
                        <div class="sb-identity">
                          <img class="champ-icon tiny" src={`${CDRAGON}/champion-icons/${sp.championId}.png`} alt="" title={championById.get(sp.championId)?.name ?? ""} loading="lazy" />
                          <div class="sb-spells">
                            {#each sp.spells ?? [] as spell (spell)}
                              <img class="sb-spell" src={spellIcon(spell)} alt="" loading="lazy" onerror={(e) => { (e.currentTarget as HTMLImageElement).style.visibility = "hidden"; }} />
                            {/each}
                          </div>
                          {#if sp.runes}
                            <div class="sb-runes">
                              {#if perkIcons[sp.runes.perks?.[0]]}
                                <img class="sb-rune keystone" src={perkIcons[sp.runes.perks[0]]} alt="" loading="lazy" />
                              {/if}
                              {#if sp.runes.subStyle}
                                <img class="sb-rune" src={`${CDRAGON}/perk-images/styles/${sp.runes.subStyle}.png`} alt="" loading="lazy" />
                              {/if}
                            </div>
                          {/if}
                          {#if sp.puuid}
                            <button class="sb-name link" onclick={() => openPlayer(sp)} title={$t("league.open_player") as string}>
                              {sp.gameName || championById.get(sp.championId)?.name || "—"}
                            </button>
                          {:else}
                            <span class="sb-name">{sp.gameName || championById.get(sp.championId)?.name || "—"}</span>
                          {/if}
                          <span class="scoreboard-kda">{sp.kills}/{sp.deaths}/{sp.assists}</span>
                          <span class="dim">{$t("league.stat_level")} {sp.level}</span>
                          <span class="scoreboard-cs dim">{sp.cs} CS</span>
                          <span class="scoreboard-gold dim">{(sp.gold / 1000).toFixed(1)}k</span>
                        </div>
                        <div class="sb-items">
                          {#each sp.items ?? [] as item, idx (`${idx}-${item}`)}
                            {#if item > 0}
                              <img class="item-icon tiny" src={itemIcon(item)} alt="" loading="lazy" onerror={(e) => { (e.currentTarget as HTMLImageElement).style.visibility = "hidden"; }} />
                            {:else}
                              <span class="item-empty" aria-hidden="true"></span>
                            {/if}
                          {/each}
                        </div>
                        <div class="sb-stats">
                          {#each statLine(sp) as stat (stat.key)}
                            <span class="sb-stat"><span class="dim">{$t(stat.label)}</span> {stat.value}</span>
                          {/each}
                          {#if sp.pentaKills > 0}<span class="sb-flag">{$t("league.stat_penta")}</span>{/if}
                          {#if sp.quadraKills > 0}<span class="sb-flag">{$t("league.stat_quadra")}</span>{/if}
                          {#if sp.tripleKills > 0}<span class="sb-flag">{$t("league.stat_triple")}</span>{/if}
                          {#if sp.firstBlood}<span class="sb-flag">{$t("league.stat_first_blood")}</span>{/if}
                          {#if sp.firstTower}<span class="sb-flag">{$t("league.stat_first_tower")}</span>{/if}
                        </div>
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
    {#if lookupPuuid}
      <div class="lookup-drawer">
        <div class="card-head">
          <h4 class="section-title">{lookupName}</h4>
          <button class="button" onclick={closeLookup}>{$t("league.close")}</button>
        </div>
        {#if lookupLoading}
          <p class="empty-hint">…</p>
        {:else if lookupError}
          <p class="action-error" role="alert">{lookupError}</p>
        {:else if lookupGames.length === 0}
          <p class="empty-hint">{$t("league.history_empty")}</p>
        {:else}
          <div class="game-list">
            {#each lookupGames as g (g.gameId)}
              {@const lp = playerStats(g)}
              <div class="game-row static">
                <img class="champ-icon" src={`${CDRAGON}/champion-icons/${lp.championId}.png`} alt="" loading="lazy" />
                <div class="game-info">
                  <span class="game-result" class:win={lp.win} class:loss={!lp.win}>{lp.win ? $t("league.victory") : $t("league.defeat")}</span>
                  <span class="game-mode">{queueName(g.queueId, g.gameMode)}</span>
                </div>
                <span class="game-kda">{lp.kills} / {lp.deaths} / {lp.assists}</span>
                <span class="game-time">{timeAgo(g.gameCreation, $locale)}</span>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </section>
{/if}
