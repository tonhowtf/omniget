<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { CDRAGON, TAG_KEYS, type Champion, type RankedEntry } from "./shared";

  let {
    championById,
    active,
  }: {
    championById: Map<number, Champion>;
    active?: boolean;
  } = $props();

  let searchQuery = $state("");
  let searchResult = $state<any>(null);
  let searchLoading = $state(false);
  let searchError = $state("");
  let jungleReport = $state<any>(null);
  let duos = $state<any>(null);
  let duosLoading = $state(false);

  function rankLabel(entry: RankedEntry | undefined): string {
    if (!entry?.tier || entry.tier === "NONE" || entry.tier === "") return $t("league.unranked") as string;
    const tier = entry.tier.charAt(0) + entry.tier.slice(1).toLowerCase();
    return `${tier} ${entry.division ?? ""} · ${entry.leaguePoints ?? 0} LP`;
  }

  async function runSearch() {
    const raw = searchQuery.trim();
    if (!raw || searchLoading) return;
    const [namePart, tagPart = ""] = raw.split("#");
    searchLoading = true;
    searchError = "";
    jungleReport = null;
    try {
      searchResult = await invoke<any>("league_search_player", {
        gameName: namePart.trim(),
        tagLine: tagPart.trim(),
      });
      const puuid = searchResult?.summoner?.puuid;
      if (puuid) {
        invoke<any>("league_jungle_report", { puuid, sample: 8 })
          .then((r) => { jungleReport = r; })
          .catch(() => { jungleReport = null; });
      }
    } catch (e: any) {
      searchResult = null;
      searchError = typeof e === "string" ? e : (e?.message ?? String(e));
    } finally {
      searchLoading = false;
    }
  }

  let spectateLoading = $state(false);
  let spectateError = $state("");

  async function spectateSearched() {
    const s = searchResult?.summoner;
    if (!s?.puuid || spectateLoading) return;
    spectateLoading = true;
    spectateError = "";
    try {
      await invoke("league_spectate", {
        puuid: s.puuid,
        riotId: `${s.gameName}#${s.tagLine}`,
      });
    } catch (e: any) {
      const raw = typeof e === "string" ? e : (e?.message ?? String(e));
      spectateError = raw.includes(" 404") || raw.includes("not in")
        ? ($t("league.spectate_not_in_game") as string)
        : raw;
    } finally {
      spectateLoading = false;
    }
  }

  async function loadDuos() {
    if (duosLoading) return;
    duosLoading = true;
    try {
      duos = await invoke<any>("league_duos", { sample: 20 });
    } catch {
      duos = { duos: [] };
    } finally {
      duosLoading = false;
    }
  }
</script>

{#if active !== false}
  <section class="card">
    <div class="card-head">
      <h3>{$t("league.search_title")}</h3>
    </div>
    <form class="search-form" onsubmit={(e) => { e.preventDefault(); runSearch(); }}>
      <input
        class="input-text"
        placeholder={$t("league.search_placeholder") as string}
        bind:value={searchQuery}
        spellcheck="false"
      />
      <button class="button primary" type="submit" disabled={searchLoading}>
        {searchLoading ? $t("league.searching_player") : $t("league.search_button")}
      </button>
    </form>
    {#if searchError}
      <p class="action-error" role="alert">{searchError}</p>
    {:else}
      <p class="win-disclaimer">{$t("league.search_hint")}</p>
    {/if}
  </section>

  {#if searchResult}
    {@const s = searchResult.summoner}
    {@const r = searchResult.report}
    <section class="profile-card">
      <img class="profile-icon" src={`${CDRAGON}/profile-icons/${s.profileIconId}.jpg`} alt="" loading="lazy" />
      <div class="profile-info">
        <span class="profile-name">{s.gameName}<span class="tag">#{s.tagLine}</span></span>
        <span class="profile-level">{$t("league.level")} {s.summonerLevel ?? "—"}</span>
      </div>
      <div class="ranked-chips">
        <div class="ranked-chip">
          <span class="ranked-queue">{$t("league.ranked_solo")}</span>
          <span class="ranked-value">{rankLabel(r?.solo)}</span>
        </div>
        <div class="ranked-chip">
          <span class="ranked-queue">{$t("league.ranked_flex")}</span>
          <span class="ranked-value">{rankLabel(r?.flex)}</span>
        </div>
      </div>
      <button class="button" onclick={spectateSearched} disabled={spectateLoading}>
        {spectateLoading ? $t("league.spectate_loading") : $t("league.spectate")}
      </button>
    </section>
    {#if spectateError}
      <p class="action-error">{spectateError}</p>
    {/if}

    {#if r?.stats?.games > 0}
      <section class="card">
        <div class="card-head"><h3>{$t("league.search_recent")}</h3></div>
        <div class="stat-grid">
          <div class="stat-cell">
            <span class="stat-value" class:good={r.stats.winrate >= 55} class:bad={r.stats.winrate <= 45}>{r.stats.winrate}%</span>
            <span class="stat-label">{$t("league.stat_winrate")} ({r.stats.games})</span>
          </div>
          <div class="stat-cell">
            <span class="stat-value">{r.stats.kda}</span>
            <span class="stat-label">KDA</span>
          </div>
          <div class="stat-cell">
            <span class="stat-value">{r.stats.streak?.length ?? 0}</span>
            <span class="stat-label">{r.stats.streak?.win ? $t("league.tag_hot_streak") : $t("league.tag_cold_streak")}</span>
          </div>
          {#if r.impact !== null && r.impact !== undefined}
            <div class="stat-cell">
              <span class="stat-value">{r.impact}<span class="dim">/10</span></span>
              <span class="stat-label">{$t("league.impact_label")} ({r.impactGames})</span>
            </div>
          {/if}
          {#if searchResult.deep}
            <div class="stat-cell">
              <span class="stat-value">{searchResult.deep.soloKillsPerGame}</span>
              <span class="stat-label">{$t("league.solo_kills_label")} ({searchResult.deep.analysedGames})</span>
            </div>
            <div class="stat-cell">
              <span class="stat-value">{searchResult.deep.earlyDeathsPerGame}</span>
              <span class="stat-label">{$t("league.early_deaths_label")} ({searchResult.deep.analysedGames})</span>
            </div>
          {/if}
        </div>
        {#if r.stats.insights?.length}
          <div class="scout-champs">
            {#each r.stats.insights as tagId (tagId)}
              <span class="scout-tag">{$t(TAG_KEYS[tagId] ?? tagId)}</span>
            {/each}
          </div>
        {/if}
      </section>
    {/if}

    {#if searchResult.champions?.length}
      <section class="card">
        <div class="card-head">
          <h3>{$t("league.search_champions")}</h3>
          <span class="phase-tag">{$t("league.search_champions_hint")}</span>
        </div>
        <div class="champ-table">
          {#each searchResult.champions as ch (ch.championId)}
            <div class="champ-row">
              <img class="champ-icon small" src={`${CDRAGON}/champion-icons/${ch.championId}.png`} alt="" loading="lazy" />
              <span class="champ-row-name">{championById.get(ch.championId)?.name ?? ch.championId}</span>
              <span class="champ-row-games">{ch.games} {$t("league.games_short")}</span>
              <span class="champ-row-wr" class:good={ch.winrate >= 55} class:bad={ch.winrate <= 45}>{ch.winrate}%</span>
              <span class="champ-row-kda">{ch.kda} KDA</span>
              <span class="champ-row-cs dim">{ch.csPerMin}/m</span>
            </div>
          {/each}
        </div>
      </section>
    {/if}

    {#if r?.mastery?.length}
      <section class="card">
        <div class="card-head"><h3>{$t("league.search_mastery")}</h3></div>
        <div class="scout-champs">
          {#each r.mastery as m (m.championId)}
            <span class="champ-chip">
              <img class="champ-icon tiny" src={`${CDRAGON}/champion-icons/${m.championId}.png`} alt="" loading="lazy" />
              {championById.get(m.championId)?.name ?? m.championId}
              <span class="dim">M{m.championLevel} · {Math.round((m.championPoints ?? 0) / 1000)}k</span>
            </span>
          {/each}
        </div>
      </section>
    {/if}

    {#if jungleReport}
      <section class="card">
        <div class="card-head">
          <h3>{$t("league.jungle_title")}</h3>
          <span class="phase-tag">{jungleReport.analysedGames} {$t("league.games_short")}</span>
        </div>
        {#if jungleReport.analysedGames > 0}
          <div class="zone-bars">
            {#each [["top", jungleReport.zones.top], ["mid", jungleReport.zones.mid], ["bot", jungleReport.zones.bot]] as [zone, pct] (zone)}
              <div class="zone-row">
                <span class="zone-name">{$t(`league.zone_${zone}`)}</span>
                <div class="goal-bar"><div class="goal-fill" style={`width:${pct}%`}></div></div>
                <span class="goal-value">{pct}%</span>
              </div>
            {/each}
          </div>
          <p class="win-note">
            {$t(`league.pref_${jungleReport.preference}`)} · {$t("league.jungle_invade")} {jungleReport.invadeRate}% · {$t("league.jungle_gank3")} {jungleReport.level3GankRate}%
          </p>
        {:else}
          <p class="empty-hint">{$t("league.jungle_empty")}</p>
        {/if}
      </section>
    {/if}
  {/if}

  <section class="card">
    <div class="card-head">
      <h3>{$t("league.duos_title")}</h3>
      <button class="button" onclick={loadDuos} disabled={duosLoading}>{$t("league.refresh")}</button>
    </div>
    <p class="win-disclaimer">{$t("league.duos_desc")}</p>
    {#if duos?.duos?.length}
      <div class="champ-table">
        {#each duos.duos as d (d.puuid)}
          <div class="champ-row">
            <span class="champ-row-name">{d.gameName ?? "—"}{#if d.tagLine}<span class="dim">#{d.tagLine}</span>{/if}</span>
            <span class="champ-row-games">{d.games} {$t("league.games_short")}</span>
            <span class="champ-row-wr" class:good={d.winrate >= 55} class:bad={d.winrate <= 45}>{d.winrate}%</span>
            <span class="champ-row-kda dim">{$t("league.duos_score")} {d.score}%</span>
          </div>
        {/each}
      </div>
    {:else if duos}
      <p class="empty-hint">{$t("league.duos_empty")}</p>
    {/if}
  </section>
{/if}
