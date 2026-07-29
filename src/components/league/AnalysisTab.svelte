<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { CDRAGON, TAG_KEYS, type Champion, type RankedEntry, type ScoutPlayer } from "./shared";
  import { markedPlayers, winrateSquad } from "$lib/league-scouting";
  import { availability, featureById, type Platform } from "./registry";
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";

  let {
    analysis,
    analysisLoading,
    onRefreshAnalysis,
    phase,
    scoutPlayers,
    scoutReports,
    scoutLoading,
    onRefreshScouting,
    championById,
    notes,
    onSaveNote,
    timesSeenBefore,
    platform,
    clientConnected,
  }: {
    analysis: any;
    analysisLoading: boolean;
    onRefreshAnalysis: () => void;
    phase: string;
    scoutPlayers: ScoutPlayer[];
    scoutReports: Record<string, any>;
    scoutLoading: boolean;
    onRefreshScouting: () => void;
    championById: Map<number, Champion>;
    notes: Record<string, string>;
    onSaveNote: (puuid: string, text: string) => void;
    timesSeenBefore: (puuid: string) => number;
    platform?: Platform;
    clientConnected?: boolean;
  } = $props();

  let analysisFeature = featureById("analysis");
  let analysisAvailability = $derived.by(() => {
    if (!analysisFeature) return { available: true as const };
    return availability(analysisFeature, {
      platform: platform ?? "macos",
      clientConnected: clientConnected ?? true,
      inGame: phase === "ChampSelect" || phase === "InProgress",
    });
  });

  let marked = $derived(markedPlayers(scoutPlayers, notes));
  let squadSide = $derived(winrateSquad(scoutPlayers, analysis?.premades ?? [], scoutReports));

  let copied = $state("");

  async function copyRiotId(player: { gameName?: string; tagLine?: string; puuid?: string }) {
    const id = player.tagLine ? `${player.gameName}#${player.tagLine}` : (player.gameName ?? "");
    if (!id) return;
    try {
      await writeText(id);
      copied = player.puuid ?? id;
      setTimeout(() => { if (copied === (player.puuid ?? id)) copied = ""; }, 1500);
    } catch {
      copied = "";
    }
  }

  let openNotes = $state<Record<string, boolean>>({});
  let chatPreview = $state("");
  let chatSending = $state(false);
  let chatError = $state("");

  function rankLabel(entry: RankedEntry | undefined): string {
    if (!entry?.tier || entry.tier === "NONE" || entry.tier === "") return $t("league.unranked") as string;
    const tier = entry.tier.charAt(0) + entry.tier.slice(1).toLowerCase();
    return `${tier} ${entry.division ?? ""} · ${entry.leaguePoints ?? 0} LP`;
  }

  function scoutGroups(): { label: string; ally: boolean; list: ScoutPlayer[] }[] {
    return [
      { label: $t("league.scout_enemies") as string, ally: false, list: scoutPlayers.filter((p) => !p.isAlly) },
      { label: $t("league.scout_allies") as string, ally: true, list: scoutPlayers.filter((p) => p.isAlly) },
    ];
  }

  // Builds the one-line-per-player summary that gets posted to champ select.
  function buildChatSummary(): string {
    const parts: string[] = [];
    for (const p of scoutPlayers.filter((x) => !x.isAlly)) {
      const r = p.puuid ? scoutReports[p.puuid] : null;
      if (!r?.stats?.games) continue;
      const champ = championById.get(p.championId)?.name ?? "";
      parts.push(`${champ || p.gameName}: ${r.stats.winrate}%WR ${r.stats.kda}KDA (${r.stats.games})`);
    }
    return parts.join(" | ");
  }

  async function sendChatSummary() {
    const text = chatPreview.trim();
    if (!text || chatSending) return;
    chatSending = true;
    chatError = "";
    try {
      await invoke("league_send_chat", { message: text });
      chatPreview = "";
    } catch (e: any) {
      chatError = typeof e === "string" ? e : (e?.message ?? String(e));
    } finally {
      chatSending = false;
    }
  }
</script>

{#if analysis}
  <section class="card">
    <div class="card-head">
      <h3>{$t("league.win_title")}</h3>
      <button class="button" onclick={onRefreshAnalysis} disabled={analysisLoading}>{$t("league.refresh")}</button>
    </div>
    <div class="winbar-wrap">
      <div class="winbar" role="img" aria-label={`${$t("league.win_allies")} ${analysis.winProbability}%`}>
        <div class="winbar-fill" style={`width:${analysis.winProbability}%`}></div>
        <div class="winbar-range" style={`left:${analysis.winLow}%;width:${Math.max(analysis.winHigh - analysis.winLow, 0)}%`}></div>
      </div>
      <div class="winbar-legend">
        <span class="win-value">{analysis.winProbability}%</span>
        <span class="win-range">{$t("league.win_interval")} {analysis.winLow}%–{analysis.winHigh}%</span>
      </div>
    </div>
    <p class="win-note">
      {$t("league.win_gap")} {analysis.ratingGap > 0 ? "+" : ""}{analysis.ratingGap} · {analysis.knownPlayers}/{analysis.totalPlayers} {$t("league.win_known")}
    </p>
    <p class="win-disclaimer">{$t("league.win_disclaimer")}</p>
    {#if chatError}
      <p class="action-error" role="alert">{chatError}</p>
    {/if}
    <div class="chat-send">
      <input
        class="input-text"
        placeholder={$t("league.chat_placeholder") as string}
        bind:value={chatPreview}
      />
      <button class="button" onclick={() => (chatPreview = buildChatSummary())}>{$t("league.chat_build")}</button>
      <button class="button primary" onclick={sendChatSummary} disabled={chatSending || !chatPreview.trim()}>{$t("league.chat_send")}</button>
    </div>
    {#if analysis.premades?.length}
      <div class="premade-row">
        <span class="bench-label">{$t("league.premades")}</span>
        {#each analysis.premades as group (group.label)}
          <span class="scout-tag">{group.label}: {group.puuids.length} {$t("league.players")}</span>
        {/each}
        <span class="premade-source">
          {analysis.premadeSource === "party"
            ? $t("league.premade_source_party")
            : $t("league.premade_source_history")}
        </span>
      </div>
    {/if}
  </section>
{:else}
  <div class="guard-card">
    <p>{analysisAvailability.available ? $t("league.win_unavailable") : $t(analysisAvailability.reasonKey)}</p>
    <button class="button" onclick={onRefreshAnalysis} disabled={analysisLoading}>{$t("league.refresh")}</button>
  </div>
{/if}

{#if (phase === "ChampSelect" || phase === "InProgress") && scoutPlayers.length > 0}
  <section class="card">
    <div class="card-head">
      <h3>{$t("league.scout_title")}</h3>
      <button class="button" onclick={onRefreshScouting} disabled={scoutLoading}>{$t("league.refresh")}</button>
    </div>
    <div class="scout-teams">
      {#if marked.ally.length > 0 || marked.enemy.length > 0 || squadSide}
        <div class="scout-notices">
          {#if marked.ally.length > 0}
            <p class="scout-notice">
              {$t("league.marked_in_allies")} <strong>{marked.ally.join(", ")}</strong>
            </p>
          {/if}
          {#if marked.enemy.length > 0}
            <p class="scout-notice">
              {$t("league.marked_in_enemies")} <strong>{marked.enemy.join(", ")}</strong>
            </p>
          {/if}
          {#if squadSide}
            <p class="scout-notice">
              {squadSide === "enemy" ? $t("league.squad_enemy") : $t("league.squad_ally")}
            </p>
          {/if}
        </div>
      {/if}
      {#each scoutGroups() as group (group.label)}
        <div class="scout-team">
          <h4 class="scout-team-title" class:enemy={!group.ally}>{group.label}</h4>
          {#if group.list.length === 0}
            <p class="empty-hint">{$t("league.scout_enemies_hidden")}</p>
          {:else}
            {#each group.list as p (p.puuid || String(p.cellId))}
              {@const r = p.puuid ? scoutReports[p.puuid] : null}
              <div class="scout-row">
                <div class="scout-main">
                  {#if p.championId > 0}
                    <img class="champ-icon small" src={`${CDRAGON}/champion-icons/${p.championId}.png`} alt="" title={championById.get(p.championId)?.name ?? ""} loading="lazy" />
                  {:else}
                    <div class="champ-icon small champ-empty" aria-hidden="true"></div>
                  {/if}
                  <div class="scout-id">
                    <span class="scout-name">
                      {p.gameName || "—"}{#if p.tagLine}<span class="tag">#{p.tagLine}</span>{/if}
                      {#if p.puuid && timesSeenBefore(p.puuid) > 0}
                        <span class="seen-badge" title={$t("league.seen_before_hint") as string}>{$t("league.seen_before")} ×{timesSeenBefore(p.puuid)}</span>
                      {/if}
                    </span>
                    <span class="scout-rank">{r ? rankLabel(r.solo) : "…"}</span>
                  </div>
                  {#if r?.stats?.games > 0}
                    <div class="scout-stats">
                      <span class="scout-wr" class:good={r.stats.winrate >= 55} class:bad={r.stats.winrate <= 45}>{r.stats.winrate}% WR</span>
                      <span class="scout-kda">
                        {r.stats.kda} KDA{#if r.impact !== null && r.impact !== undefined} · <span class="impact" title={$t("league.impact_hint") as string}>{r.impact}</span>{/if}
                      </span>
                      {#if r.stats.score !== null && r.stats.score !== undefined}
                        <span class="scout-score" title={`${$t("league.score_hint")} (${r.stats.scoredGames})`}>
                          {$t("league.score_label")} {r.stats.score.toFixed(1)}
                        </span>
                      {/if}
                    </div>
                  {:else if r?.privateProfile}
                    <span class="scout-private">{$t("league.scout_private")}</span>
                  {:else if r?.historyUnavailable}
                    <span class="scout-private">{$t("league.scout_no_history")}</span>
                  {:else if r}
                    <span class="scout-private">…</span>
                  {/if}
                  {#if p.gameName}
                    <button class="note-toggle" onclick={() => copyRiotId(p)} aria-label={$t("league.copy_riot_id") as string} title={$t("league.copy_riot_id") as string}>
                      {copied === (p.puuid ?? p.gameName) ? "✓" : "⧉"}
                    </button>
                  {/if}
                  {#if p.puuid}
                    <button class="note-toggle" class:has-note={(notes[p.puuid] ?? "").trim() !== ""} onclick={() => { openNotes = { ...openNotes, [p.puuid]: !openNotes[p.puuid] }; }} aria-label={$t("league.scout_note_placeholder") as string} aria-expanded={openNotes[p.puuid] ?? false}>✎</button>
                  {/if}
                </div>
                {#if r?.stats?.topChampions?.length}
                  <div class="scout-champs">
                    {#each r.stats.topChampions as tc (tc.championId)}
                      <span class="scout-champ">
                        <img class="champ-icon tiny" src={`${CDRAGON}/champion-icons/${tc.championId}.png`} alt="" title={championById.get(tc.championId)?.name ?? ""} loading="lazy" />
                        <span class="scout-champ-record">{tc.wins}/{tc.games}</span>
                      </span>
                    {/each}
                    {#if r?.stats?.insights?.length}
                      {#each r.stats.insights as tag (tag)}
                        <span class="scout-tag">{$t(TAG_KEYS[tag] ?? tag)}</span>
                      {/each}
                    {/if}
                  </div>
                {/if}
                {#if p.puuid && (openNotes[p.puuid] || (notes[p.puuid] ?? "").trim() !== "")}
                  <input class="input-text note-input" placeholder={$t("league.scout_note_placeholder") as string} value={notes[p.puuid] ?? ""} onchange={(e) => onSaveNote(p.puuid, e.currentTarget.value)} />
                {/if}
              </div>
            {/each}
          {/if}
        </div>
      {/each}
    </div>
  </section>
{/if}
