<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { CDRAGON, formatGameTime, type Champion, type RankedEntry, type LobbyQueue } from "./shared";
  import { CHAMPION_CLASSES, drawLane, secondaryLane, type Lane } from "$lib/league-raffle";
  import Skeleton from "./Skeleton.svelte";

  let {
    summoner,
    profileLoading = false,
    ranked,
    phase,
    champSelect,
    liveGame,
    lobby,
    queues,
    actionError,
    champions,
    championById,
    championByAlias,
    onAction,
    active,
  }: {
    summoner: any;
    profileLoading?: boolean;
    ranked: Record<string, RankedEntry>;
    phase: string;
    champSelect: any;
    liveGame: any;
    lobby: any;
    queues: LobbyQueue[];
    actionError: string;
    champions: Champion[];
    championById: Map<number, Champion>;
    championByAlias: Map<string, Champion>;
    onAction: (cmd: string, args?: Record<string, unknown>) => void;
    active?: boolean;
  } = $props();

  const PHASE_KEYS: Record<string, string> = {
    Lobby: "league.phase_lobby",
    Matchmaking: "league.phase_matchmaking",
    ReadyCheck: "league.phase_ready_check",
    ChampSelect: "league.phase_champ_select",
    GameStart: "league.phase_in_progress",
    InProgress: "league.phase_in_progress",
    WaitingForStats: "league.phase_end_of_game",
    PreEndOfGame: "league.phase_end_of_game",
    EndOfGame: "league.phase_end_of_game",
  };

  function phaseLabel(p: string): string {
    const key = PHASE_KEYS[p];
    return key ? ($t(key) as string) : p;
  }

  function rankLabel(entry: RankedEntry | undefined): string {
    if (!entry?.tier || entry.tier === "NONE" || entry.tier === "") return $t("league.unranked") as string;
    const tier = entry.tier.charAt(0) + entry.tier.slice(1).toLowerCase();
    return `${tier} ${entry.division ?? ""} · ${entry.leaguePoints ?? 0} LP`;
  }

  function myTeamPicks(session: any): { cellId: number; championId: number }[] {
    return (session?.myTeam ?? []).map((m: any) => ({ cellId: m.cellId, championId: m.championId }));
  }

  function liveChampionId(player: any): number | null {
    const raw: string = player?.rawChampionName ?? "";
    const alias = raw.split("_").pop() ?? "";
    return championByAlias.get(alias.toLowerCase())?.id ?? null;
  }

  function liveTeams(players: any[]): { order: any[]; chaos: any[] } {
    return {
      order: players.filter((p) => p.team === "ORDER"),
      chaos: players.filter((p) => p.team === "CHAOS"),
    };
  }

  const LOBBY_ROLES = ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY", "FILL"] as const;
  let firstRole = $state("FILL");
  let secondRole = $state("FILL");
  let roleError = $state("");

  async function saveRoles() {
    roleError = "";
    try {
      await invoke("league_set_positions", { first: firstRole, second: secondRole });
    } catch (e: any) {
      roleError = typeof e === "string" ? e : (e?.message ?? String(e));
    }
  }

  // Raffles: a lane, a champion, a skin. Small, honest randomness for the
  // player who wants the queue itself to decide.
  let drawnLane = $state<Lane | null>(null);
  let drawnSecond = $state<Lane | "FILL" | null>(null);
  let raffleError = $state("");
  let raffleClass = $state<string>("");
  let drawnChampion = $state<{ id: number; name: string; candidates: number } | null>(null);
  let drawingChampion = $state(false);

  function drawLaneNow() {
    drawnLane = drawLane(drawnLane);
    drawnSecond = secondaryLane(drawnLane);
  }

  async function applyDrawnLane() {
    if (!drawnLane) return;
    raffleError = "";
    try {
      await invoke("league_set_positions", { first: drawnLane, second: drawnSecond ?? "FILL" });
      firstRole = drawnLane;
      secondRole = drawnSecond ?? "FILL";
    } catch (e: any) {
      raffleError = typeof e === "string" ? e : (e?.message ?? String(e));
    }
  }

  async function drawChampion() {
    if (drawingChampion) return;
    drawingChampion = true;
    raffleError = "";
    try {
      const res = await invoke<any>("league_random_champion", { class: raffleClass || null });
      drawnChampion = { id: res.id, name: res.name, candidates: res.candidates };
    } catch (e: any) {
      raffleError = typeof e === "string" ? e : (e?.message ?? String(e));
    } finally {
      drawingChampion = false;
    }
  }

  async function declareDrawn(lock: boolean) {
    if (!drawnChampion) return;
    raffleError = "";
    try {
      await invoke("league_declare_champion", { championId: drawnChampion.id, lock });
    } catch (e: any) {
      raffleError = typeof e === "string" ? e : (e?.message ?? String(e));
    }
  }

  let skinRoll = $state<{ skin_name: string; chroma_name: string | null } | null>(null);
  let skinError = $state("");
  let skinBusy = $state(false);

  async function rollSkin() {
    if (skinBusy) return;
    skinBusy = true;
    skinError = "";
    try {
      skinRoll = await invoke<any>("league_roll_skin", {});
    } catch (e: any) {
      const raw = typeof e === "string" ? e : (e?.message ?? String(e));
      skinError = raw.includes("lock a champion") ? ($t("league.skin_lock_first") as string) : raw;
    } finally {
      skinBusy = false;
    }
  }

  async function rollWard() {
    if (skinBusy) return;
    skinBusy = true;
    skinError = "";
    try {
      await invoke("league_roll_ward");
      skinRoll = { skin_name: $t("league.skin_roll_ward") as string, chroma_name: null };
    } catch (e: any) {
      skinError = typeof e === "string" ? e : (e?.message ?? String(e));
    } finally {
      skinBusy = false;
    }
  }

  let myLockedChampion = $derived.by(() => {
    const cell = champSelect?.localPlayerCellId;
    const me = (champSelect?.myTeam ?? []).find((m: any) => m.cellId === cell);
    return me?.championId ?? 0;
  });

  let restartConfirming = $state(false);
  let restartLoading = $state(false);
  let restartError = $state("");

  async function restartClientUx() {
    if (restartLoading) return;
    restartLoading = true;
    restartError = "";
    try {
      await invoke("league_restart_ux");
    } catch (e: any) {
      restartError = typeof e === "string" ? e : (e?.message ?? String(e));
    } finally {
      restartLoading = false;
      restartConfirming = false;
    }
  }

  let dodgeConfirming = $state(false);
  let dodgeLoading = $state(false);
  let dodgeError = $state("");

  async function dodgeChampSelect() {
    if (dodgeLoading) return;
    dodgeLoading = true;
    dodgeError = "";
    try {
      await invoke("league_dodge");
    } catch (e: any) {
      dodgeError = typeof e === "string" ? e : (e?.message ?? String(e));
    } finally {
      dodgeLoading = false;
      dodgeConfirming = false;
    }
  }
</script>

{#if active !== false}
  {#if !summoner && profileLoading}
    <section class="profile-card" aria-busy="true">
      <Skeleton w="56px" h="56px" round="50%" />
      <div class="profile-info">
        <Skeleton w="150px" h="16px" />
        <Skeleton w="76px" h="12px" />
      </div>
      <div class="ranked-chips">
        <Skeleton w="118px" h="42px" round="8px" />
        <Skeleton w="118px" h="42px" round="8px" />
      </div>
    </section>
  {:else if summoner}
    <section class="profile-card">
      <img
        class="profile-icon"
        src={`${CDRAGON}/profile-icons/${summoner.profileIconId}.jpg`}
        alt=""
        loading="lazy"
      />
      <div class="profile-info">
        <span class="profile-name">
          {summoner.gameName ?? summoner.displayName}{#if summoner.tagLine}<span class="tag">#{summoner.tagLine}</span>{/if}
        </span>
        <span class="profile-level">{$t("league.level")} {summoner.summonerLevel}</span>
      </div>
      <div class="ranked-chips">
        <div class="ranked-chip">
          <span class="ranked-queue">{$t("league.ranked_solo")}</span>
          <span class="ranked-value">{rankLabel(ranked?.RANKED_SOLO_5x5)}</span>
        </div>
        <div class="ranked-chip">
          <span class="ranked-queue">{$t("league.ranked_flex")}</span>
          <span class="ranked-value">{rankLabel(ranked?.RANKED_FLEX_SR)}</span>
        </div>
      </div>
    </section>
  {/if}
  {#if actionError}
    <div class="action-error" role="alert">{actionError}</div>
  {/if}
  {#if restartError}
    <div class="action-error" role="alert">{restartError}</div>
  {/if}
  <div class="repair-row">
    {#if restartConfirming}
      <span class="repair-note">{$t("league.restart_ux_warning")}</span>
      <button class="button" onclick={() => (restartConfirming = false)}>{$t("league.dodge_cancel")}</button>
      <button class="button" onclick={restartClientUx} disabled={restartLoading}>{$t("league.restart_ux_confirm")}</button>
    {:else}
      <button class="button subtle" onclick={() => (restartConfirming = true)}>{$t("league.restart_ux")}</button>
    {/if}
  </div>
  {#if phase === "ChampSelect" && champSelect}
    <section class="card">
      <div class="card-head">
        <h3>{$t("league.champ_select_title")}</h3>
        <span class="phase-tag">{phaseLabel(phase)}</span>
      </div>
      <div class="team-picks">
        {#each myTeamPicks(champSelect) as pick (pick.cellId)}
          {#if pick.championId > 0}
            <img class="champ-icon" src={`${CDRAGON}/champion-icons/${pick.championId}.png`} alt={championById.get(pick.championId)?.name ?? ""} title={championById.get(pick.championId)?.name ?? ""} loading="lazy" />
          {:else}
            <div class="champ-icon champ-empty" aria-hidden="true"></div>
          {/if}
        {/each}
      </div>
      {#if champSelect.benchEnabled}
        <div class="bench-row">
          <span class="bench-label">{$t("league.bench_title")}</span>
          <div class="bench-champs">
            {#each champSelect.benchChampions ?? [] as bc (bc.championId)}
              <button
                class="bench-swap"
                onclick={() => onAction("league_bench_swap", { championId: bc.championId })}
                title={championById.get(bc.championId)?.name ?? ""}
                aria-label={`${$t("league.swap")} ${championById.get(bc.championId)?.name ?? bc.championId}`}
              >
                <img class="champ-icon" src={`${CDRAGON}/champion-icons/${bc.championId}.png`} alt="" loading="lazy" />
              </button>
            {/each}
          </div>
          <div class="reroll-actions">
            <button class="button" onclick={() => onAction("league_reroll")}>{$t("league.reroll")}</button>
            <button class="button" onclick={() => onAction("league_reroll_keeping_champion")} title={$t("league.reroll_keep_hint") as string}>
              {$t("league.reroll_keep")}
            </button>
          </div>
        </div>
      {/if}
      <div class="skin-row">
        <span class="bench-label">{$t("league.skin_title")}</span>
        <button class="button" onclick={rollSkin} disabled={skinBusy || myLockedChampion <= 0} title={myLockedChampion <= 0 ? ($t("league.skin_lock_first") as string) : ""}>{$t("league.skin_roll")}</button>
        <button class="button subtle" onclick={rollWard} disabled={skinBusy}>{$t("league.skin_roll_ward")}</button>
        {#if skinRoll}
          <span class="dim">{$t("league.skin_rolled")}: {skinRoll.skin_name}{skinRoll.chroma_name ? ` · ${skinRoll.chroma_name}` : ""}</span>
        {/if}
      </div>
      {#if skinError}
        <p class="action-error" role="alert">{skinError}</p>
      {/if}
      {#if myLockedChampion <= 0}
        <div class="skin-row">
          <span class="bench-label">{$t("league.raffle_champion")}</span>
          <select class="select-role" bind:value={raffleClass} aria-label={$t("league.raffle_champion") as string}>
            <option value="">{$t("league.raffle_class_any")}</option>
            {#each CHAMPION_CLASSES as cls (cls)}
              <option value={cls}>{$t(`league.raffle_class_${cls}`)}</option>
            {/each}
          </select>
          <button class="button" onclick={drawChampion} disabled={drawingChampion}>{drawnChampion ? $t("league.raffle_again") : $t("league.raffle_champion")}</button>
          {#if drawnChampion}
            <img class="champ-icon small" src={`${CDRAGON}/champion-icons/${drawnChampion.id}.png`} alt="" loading="lazy" />
            <strong>{drawnChampion.name}</strong>
            <button class="button subtle" onclick={() => declareDrawn(false)}>{$t("league.raffle_declare")}</button>
            <button class="button primary" onclick={() => declareDrawn(true)}>{$t("league.raffle_lock")}</button>
          {/if}
        </div>
        {#if raffleError}
          <p class="action-error" role="alert">{raffleError}</p>
        {/if}
      {/if}
      {#if dodgeError}
        <p class="action-error" role="alert">{dodgeError}</p>
      {/if}
      <div class="dodge-row">
        {#if dodgeConfirming}
          <span class="dodge-warning">{$t("league.dodge_warning")}</span>
          <button class="button" onclick={() => (dodgeConfirming = false)}>{$t("league.dodge_cancel")}</button>
          <button class="button danger" onclick={dodgeChampSelect} disabled={dodgeLoading}>{$t("league.dodge_confirm")}</button>
        {:else}
          <button class="button subtle-danger" onclick={() => (dodgeConfirming = true)}>{$t("league.dodge")}</button>
        {/if}
      </div>
    </section>
  {:else if phase === "InProgress" && liveGame?.stats}
    <section class="card">
      <div class="card-head">
        <h3>{$t("league.live_title")}</h3>
        <span class="phase-tag">{formatGameTime(liveGame.stats.gameTime ?? 0)}</span>
      </div>
      {#if Array.isArray(liveGame.players)}
        {@const teams = liveTeams(liveGame.players)}
        <div class="live-teams">
          {#each [teams.order, teams.chaos] as team, ti (ti)}
            <div class="live-team">
              {#each team as p (p.riotId ?? p.summonerName ?? p.championName)}
                {@const cid = liveChampionId(p)}
                <div class="live-row" class:me={(p.riotId ?? p.summonerName) === liveGame.activePlayer}>
                  {#if cid}
                    <img class="champ-icon small" src={`${CDRAGON}/champion-icons/${cid}.png`} alt="" loading="lazy" />
                  {:else}
                    <div class="champ-icon small champ-empty" aria-hidden="true"></div>
                  {/if}
                  <span class="live-name">{p.championName}</span>
                  <span class="live-kda">{p.scores?.kills ?? 0}/{p.scores?.deaths ?? 0}/{p.scores?.assists ?? 0}</span>
                  {#if p.isDead && p.respawnTimer > 0}
                    <span class="live-respawn">{$t("league.respawn_in")} {Math.ceil(p.respawnTimer)}s</span>
                  {/if}
                </div>
              {/each}
            </div>
          {/each}
        </div>
      {/if}
    </section>
  {:else}
    <section class="card">
      <div class="card-head">
        <h3>{$t("league.lobby_title")}</h3>
        {#if phase && phase !== "None"}
          <span class="phase-tag">{phaseLabel(phase)}</span>
        {/if}
      </div>
      {#if phase === "ReadyCheck"}
        <div class="lobby-actions">
          <button class="button primary" onclick={() => onAction("league_accept_ready_check")}>{$t("league.accept_now")}</button>
        </div>
      {:else if phase === "Matchmaking"}
        <div class="lobby-actions">
          <span class="searching-hint">{$t("league.searching")}</span>
          <button class="button" onclick={() => onAction("league_stop_matchmaking")}>{$t("league.stop_queue")}</button>
        </div>
      {:else if phase === "Lobby" && lobby}
        <div class="lobby-actions">
          <button class="button primary" onclick={() => onAction("league_start_matchmaking")}>{$t("league.start_queue")}</button>
          <button class="button" onclick={() => onAction("league_leave_lobby")}>{$t("league.leave_lobby")}</button>
        </div>
        {#if roleError}
          <p class="action-error" role="alert">{roleError}</p>
        {/if}
        <div class="profile-tool-row">
          <span class="list-label">{$t("league.role_preference")}</span>
          <select class="select-role" bind:value={firstRole} aria-label={$t("league.role_first") as string}>
            {#each LOBBY_ROLES as role (role)}
              <option value={role}>{$t(`league.role_${role.toLowerCase()}`)}</option>
            {/each}
          </select>
          <select class="select-role" bind:value={secondRole} aria-label={$t("league.role_second") as string}>
            {#each LOBBY_ROLES as role (role)}
              <option value={role}>{$t(`league.role_${role.toLowerCase()}`)}</option>
            {/each}
          </select>
          <button class="button" onclick={saveRoles}>{$t("league.apply")}</button>
        </div>
        <div class="profile-tool-row">
          <span class="list-label">{$t("league.raffle_title")}</span>
          <button class="button" onclick={drawLaneNow}>{$t("league.raffle_lane")}</button>
          {#if drawnLane}
            <span class="pos-chip">{$t(`league.role_${drawnLane.toLowerCase()}`)}</span>
            <span class="dim">+ {drawnSecond === "FILL" ? $t("league.role_fill") : $t(`league.role_${(drawnSecond ?? "fill").toLowerCase()}`)}</span>
            <button class="button subtle" onclick={applyDrawnLane}>{$t("league.raffle_lane_apply")}</button>
          {/if}
        </div>
        <div class="profile-tool-row">
          <select class="select-role" bind:value={raffleClass} aria-label={$t("league.raffle_champion") as string}>
            <option value="">{$t("league.raffle_class_any")}</option>
            {#each CHAMPION_CLASSES as cls (cls)}
              <option value={cls}>{$t(`league.raffle_class_${cls}`)}</option>
            {/each}
          </select>
          <button class="button" onclick={drawChampion} disabled={drawingChampion}>{drawnChampion ? $t("league.raffle_again") : $t("league.raffle_champion")}</button>
          {#if drawnChampion}
            <img class="champ-icon small" src={`${CDRAGON}/champion-icons/${drawnChampion.id}.png`} alt="" loading="lazy" />
            <strong>{drawnChampion.name}</strong>
            <span class="dim">({drawnChampion.candidates} {$t("league.raffle_pool")})</span>
          {/if}
        </div>
        {#if raffleError}
          <p class="action-error" role="alert">{raffleError}</p>
        {/if}
      {:else if phase === "EndOfGame" || phase === "PreEndOfGame" || phase === "WaitingForStats"}
        <div class="lobby-actions">
          <button class="button primary" onclick={() => onAction("league_play_again")}>{$t("league.play_again")}</button>
        </div>
      {:else if queues.length > 0}
        <div class="queue-grid">
          {#each queues as q (q.id)}
            <button class="button" onclick={() => onAction("league_create_lobby", { queueId: q.id })}>{q.shortName || q.name}</button>
          {/each}
        </div>
      {:else}
        <p class="empty-hint">{$t("league.lobby_hint")}</p>
      {/if}
    </section>
  {/if}
{/if}

<style>
  .skin-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
</style>
