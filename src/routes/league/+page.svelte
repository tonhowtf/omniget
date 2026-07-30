<script lang="ts" module>
  // Survives remounts so the same lobby is not counted as a new encounter
  // every time the page is opened.
  const recordedEncounters = new Set<string>();
</script>

<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import OverviewTab from "$components/league/OverviewTab.svelte";
  import AnalysisTab from "$components/league/AnalysisTab.svelte";
  import MetaTab from "$components/league/MetaTab.svelte";
  import SearchTab from "$components/league/SearchTab.svelte";
  import LiveTab from "$components/league/LiveTab.svelte";
  import GoalsTab from "$components/league/GoalsTab.svelte";
  import AutomationTab from "$components/league/AutomationTab.svelte";
  import HistoryTab from "$components/league/HistoryTab.svelte";
  import type { Champion, GoalKey, LobbyQueue, RankedEntry, Role, ScoutPlayer } from "$components/league/shared";
  import type { Platform } from "$components/league/registry";

  type LeagueStatus = { connected: boolean; port: number | null; region: string | null };

  let settings = $derived(getSettings());
  let enabled = $derived(settings?.league?.enabled ?? true);

  let status = $state<LeagueStatus>({ connected: false, port: null, region: null });
  let summoner = $state<any>(null);
  let ranked = $state<Record<string, RankedEntry>>({});
  let phase = $state<string>("");
  let games = $state<any[]>([]);
  let loadingHistory = $state(false);
  let liveTimer: ReturnType<typeof setInterval> | null = null;
  let unlisteners: UnlistenFn[] = [];

  let champions = $state<Champion[]>([]);
  let championById = $derived(new Map(champions.map((c) => [c.id, c])));
  let championByAlias = $derived(new Map(champions.map((c) => [c.alias.toLowerCase(), c])));

  let queues = $state<LobbyQueue[]>([]);
  let lobby = $state<any>(null);
  let champSelect = $state<any>(null);
  let liveGame = $state<any>(null);
  let actionError = $state("");

  let scoutPlayers = $state<ScoutPlayer[]>([]);
  let scoutReports = $state<Record<string, any>>({});
  let scoutLoading = $state(false);
  let notes = $state<Record<string, string>>({});

  const NOTES_KEY = "league-player-notes";
  const ENCOUNTERS_KEY = "league-encounters";

  type Encounter = { count: number; lastSeen: number; name?: string };
  let encounters = $state<Record<string, Encounter>>({});

  function loadNotes() {
    try {
      notes = JSON.parse(localStorage.getItem(NOTES_KEY) ?? "{}");
    } catch {
      notes = {};
    }
  }

  function saveNote(puuid: string, text: string) {
    notes = { ...notes, [puuid]: text };
    const clean = Object.fromEntries(Object.entries(notes).filter(([, v]) => (v as string).trim() !== ""));
    localStorage.setItem(NOTES_KEY, JSON.stringify(clean));
  }

  function loadEncounters() {
    try {
      encounters = JSON.parse(localStorage.getItem(ENCOUNTERS_KEY) ?? "{}");
    } catch {
      encounters = {};
    }
  }

  function recordEncounters(players: ScoutPlayer[]) {
    const myPuuid = summoner?.puuid ?? "";
    let changed = false;
    const next = { ...encounters };
    for (const p of players) {
      if (!p.puuid || p.puuid === myPuuid || recordedEncounters.has(p.puuid)) continue;
      recordedEncounters.add(p.puuid);
      const e = next[p.puuid] ?? { count: 0, lastSeen: 0 };
      next[p.puuid] = {
        count: e.count + 1,
        lastSeen: Date.now(),
        name: p.gameName ? `${p.gameName}#${p.tagLine}` : e.name,
      };
      changed = true;
    }
    if (changed) {
      encounters = next;
      localStorage.setItem(ENCOUNTERS_KEY, JSON.stringify(next));
    }
  }

  function timesSeenBefore(puuid: string): number {
    const e = encounters[puuid];
    if (!e) return 0;
    return recordedEncounters.has(puuid) ? e.count - 1 : e.count;
  }

  async function loadScouting() {
    if (scoutLoading) return;
    scoutLoading = true;
    try {
      const data = await invoke<any>("league_game_players");
      scoutPlayers = (data?.players ?? []).filter((p: any) => p);
      recordEncounters(scoutPlayers);
      for (const p of scoutPlayers) {
        if (!p.puuid || scoutReports[p.puuid]) continue;
        invoke<any>("league_player_report", { puuid: p.puuid, withImpact: false })
          .then((r) => {
            scoutReports = { ...scoutReports, [p.puuid]: r };
          })
          .catch(() => {});
      }
    } catch {
      scoutPlayers = [];
    } finally {
      scoutLoading = false;
    }
  }

  const TAB_IDS = ["overview", "analysis", "meta", "search", "live", "goals", "automation", "history"] as const;
  type Tab = (typeof TAB_IDS)[number];
  let tab = $state<Tab>("overview");

  let analysis = $state<any>(null);
  let analysisLoading = $state(false);
  let liveMetrics = $state<any>(null);
  let cooldowns = $state<any>(null);
  let liveEvents = $state<any>(null);

  // The registry needs to know where it runs to explain platform limits.
  let platform = $derived<Platform>(
    navigator.userAgent.includes("Windows")
      ? "windows"
      : navigator.userAgent.includes("Mac")
        ? "macos"
        : "linux",
  );

  const GOALS_KEY = "league-role-goals";

  // Baselines mirror the Rust defaults; a support is not judged on CS.
  const DEFAULT_GOALS: Record<Role, Record<GoalKey, number>> = {
    TOP: { csPerMin: 7.0, goldPerMin: 380, kda: 2.5, visionPerMin: 0.6 },
    JUNGLE: { csPerMin: 5.5, goldPerMin: 360, kda: 3.0, visionPerMin: 1.0 },
    MIDDLE: { csPerMin: 7.5, goldPerMin: 400, kda: 3.0, visionPerMin: 0.7 },
    BOTTOM: { csPerMin: 8.0, goldPerMin: 420, kda: 3.0, visionPerMin: 0.6 },
    UTILITY: { csPerMin: 1.5, goldPerMin: 260, kda: 3.0, visionPerMin: 2.0 },
  };

  let goals = $state<Record<string, Record<string, number>>>({});

  function loadGoals() {
    try {
      goals = JSON.parse(localStorage.getItem(GOALS_KEY) ?? "{}");
    } catch {
      goals = {};
    }
  }

  function goalValue(role: Role, key: GoalKey): number {
    return goals[role]?.[key] ?? DEFAULT_GOALS[role][key];
  }

  function setGoal(role: Role, key: GoalKey, value: number) {
    if (!Number.isFinite(value) || value < 0) return;
    goals = { ...goals, [role]: { ...(goals[role] ?? {}), [key]: value } };
    localStorage.setItem(GOALS_KEY, JSON.stringify(goals));
  }

  function resetGoals(role: Role) {
    const next = { ...goals };
    delete next[role];
    goals = next;
    localStorage.setItem(GOALS_KEY, JSON.stringify(goals));
  }

  async function loadAnalysis() {
    if (analysisLoading) return;
    analysisLoading = true;
    try {
      analysis = await invoke<any>("league_match_analysis");
    } catch {
      analysis = null;
    } finally {
      analysisLoading = false;
    }
  }

  // Without these guards a slow client lets every 4s tick queue another round of
  // requests; the replies pile up on the UI thread and freeze the whole window.
  let liveMetricsInFlight = false;
  let liveEventsInFlight = false;
  let cooldownsInFlight = false;
  let refreshInFlight = false;

  async function loadLiveMetrics() {
    if (liveMetricsInFlight) return;
    liveMetricsInFlight = true;
    try {
      liveMetrics = await invoke<any>("league_live_metrics");
    } catch {
      liveMetrics = null;
    } finally {
      liveMetricsInFlight = false;
    }
  }

  async function loadLiveEvents() {
    if (liveEventsInFlight) return;
    liveEventsInFlight = true;
    try {
      liveEvents = await invoke<any>("league_live_events");
    } catch {
      liveEvents = null;
    } finally {
      liveEventsInFlight = false;
    }
  }

  async function loadCooldowns() {
    if (cooldownsInFlight) return;
    cooldownsInFlight = true;
    try {
      cooldowns = await invoke<any>("league_ability_cooldowns");
    } catch {
      cooldowns = null;
    } finally {
      cooldownsInFlight = false;
    }
  }

  // Champion the local player has locked in (or hovered) during champ select.
  let champSelectChampionId = $derived.by(() => {
    const cell = champSelect?.localPlayerCellId;
    if (cell === undefined || cell === null) return 0;
    const me = (champSelect?.myTeam ?? []).find((m: any) => m.cellId === cell);
    return me?.championId ?? 0;
  });

  let myAssignedPosition = $derived.by(() => {
    const cell = champSelect?.localPlayerCellId;
    const me = (champSelect?.myTeam ?? []).find((m: any) => m.cellId === cell);
    return (me?.assignedPosition ?? "").toUpperCase();
  });

  async function refreshStatus() {
    try {
      status = await invoke<LeagueStatus>("league_status");
    } catch {
      status = { connected: false, port: null, region: null };
    }
    if (!status.connected) {
      summoner = null;
      phase = "";
      lobby = null;
      champSelect = null;
      liveGame = null;
      return;
    }
    try {
      phase = await invoke<string>("league_gameflow");
    } catch {
      phase = "";
    }
    if (!summoner) {
      await loadProfile();
    }
    if (champions.length === 0) {
      loadChampions();
    }
    if (queues.length === 0) {
      loadQueues();
    }
    refreshPhaseData();
  }

  async function refreshPhaseData() {
    // A tick that arrives while the previous one is still running is dropped
    // rather than queued: the client is the slow part, and stacking rounds of
    // requests is what makes the window stop responding.
    if (refreshInFlight) return;
    refreshInFlight = true;
    try {
      await refreshPhaseDataInner();
    } finally {
      refreshInFlight = false;
    }
  }

  async function refreshPhaseDataInner() {
    if (phase === "ChampSelect") {
      try {
        champSelect = await invoke<any>("league_champ_select_session");
      } catch {
        champSelect = null;
      }
    } else {
      champSelect = null;
    }
    if (phase === "InProgress") {
      try {
        liveGame = await invoke<any>("league_live_game");
      } catch {
        liveGame = null;
      }
    } else {
      liveGame = null;
    }
    if (phase === "ChampSelect" || phase === "InProgress") {
      if (scoutPlayers.length === 0) loadScouting();
      if (!analysis && !analysisLoading) loadAnalysis();
    } else if (scoutPlayers.length > 0) {
      scoutPlayers = [];
      analysis = null;
    }
    if (phase === "InProgress") {
      loadLiveMetrics();
      loadCooldowns();
      loadLiveEvents();
    } else if (liveMetrics) {
      liveMetrics = null;
      cooldowns = null;
      liveEvents = null;
    }
    if (phase === "Lobby" || phase === "Matchmaking") {
      try {
        lobby = await invoke<any>("league_get", { path: "/lol-lobby/v2/lobby" });
      } catch {
        lobby = null;
      }
    } else {
      lobby = null;
    }
  }

  async function loadProfile() {
    try {
      summoner = await invoke<any>("league_summoner");
    } catch {
      summoner = null;
      return;
    }
    try {
      const stats = await invoke<any>("league_ranked");
      ranked = stats?.queueMap ?? {};
    } catch {
      ranked = {};
    }
    loadHistory();
  }

  async function loadChampions() {
    try {
      const data = await invoke<any[]>("league_get", { path: "/lol-game-data/assets/v1/champion-summary.json" });
      champions = (data ?? [])
        .filter((c) => c.id > 0)
        .map((c) => ({ id: c.id, name: c.name, alias: c.alias }))
        .sort((a, b) => a.name.localeCompare(b.name));
    } catch {
      champions = [];
    }
  }

  async function loadQueues() {
    try {
      queues = await invoke<LobbyQueue[]>("league_lobby_queues");
    } catch {
      queues = [];
    }
  }

  async function loadHistory() {
    if (loadingHistory) return;
    loadingHistory = true;
    try {
      const data = await invoke<any>("league_match_history", { begIndex: 0, endIndex: 12 });
      games = data?.games?.games ?? [];
    } catch {
      games = [];
    } finally {
      loadingHistory = false;
    }
  }

  async function action(cmd: string, args?: Record<string, unknown>) {
    actionError = "";
    try {
      await invoke(cmd, args ?? {});
      refreshStatus();
    } catch (e: any) {
      actionError = typeof e === "string" ? e : e.message ?? String(e);
    }
  }

  // The backend keeps a websocket to the client and pushes changes; the only
  // thing still polled is the in-game data server, which has no push channel.
  $effect(() => {
    if (phase === "InProgress") {
      liveTimer = setInterval(refreshPhaseData, 4000);
      return () => {
        if (liveTimer) clearInterval(liveTimer);
        liveTimer = null;
      };
    }
  });

  onMount(() => {
    if (!enabled) return;
    loadNotes();
    loadGoals();
    loadEncounters();
    refreshStatus();
    listen<any>("league-connected", (e) => {
      status = {
        connected: e.payload?.connected ?? false,
        port: e.payload?.port ?? null,
        region: e.payload?.region ?? null,
      };
      if (status.connected) {
        refreshStatus();
      } else {
        summoner = null;
        phase = "";
        lobby = null;
        champSelect = null;
        liveGame = null;
      }
    }).then((u) => unlisteners.push(u));
    listen<string>("league-phase", (e) => {
      const next = e.payload ?? "";
      // The client repeats the current phase on reconnects and on its own
      // heartbeat; refreshing again for the same phase re-fetches everything for
      // nothing.
      if (next === phase) return;
      phase = next;
      refreshPhaseData();
    }).then((u) => unlisteners.push(u));
    listen<any>("league-champ-select", (e) => {
      if (phase === "ChampSelect") champSelect = e.payload;
    }).then((u) => unlisteners.push(u));
    listen<any>("league-lobby", (e) => {
      if (phase === "Lobby" || phase === "Matchmaking") lobby = e.payload;
    }).then((u) => unlisteners.push(u));
  });

  onDestroy(() => {
    if (liveTimer) clearInterval(liveTimer);
    for (const u of unlisteners) u();
    unlisteners = [];
  });
</script>

<div class="league-page">
  {#if !enabled}
    <div class="guard-card">
      <h2>{$t("league.disabled_title")}</h2>
      <p>{$t("league.disabled_body")}</p>
      <button class="button primary" onclick={() => goto("/settings")}>{$t("league.open_settings")}</button>
    </div>
  {:else}
    <header class="league-header">
      <h2>{$t("league.nav")}</h2>
      <div class="status-chip" class:connected={status.connected}>
        <span class="dot"></span>
        {status.connected ? $t("league.connected") : $t("league.disconnected_title")}
        {#if status.connected && status.region}
          <span class="region">{status.region}</span>
        {/if}
      </div>
    </header>

    {#if !status.connected}
      <div class="guard-card">
        <p>{$t("league.disconnected_body")}</p>
      </div>
    {:else}
      <nav class="league-tabs" aria-label={$t("league.nav") as string}>
        {#each TAB_IDS as id (id)}
          <button class="league-tab" class:on={tab === id} onclick={() => (tab = id)} aria-current={tab === id}>
            {$t(`league.tab_${id}`)}
          </button>
        {/each}
      </nav>

      <!-- Panels stay mounted while hidden so tab state (search results, spell
           timers, expanded games) survives switching, and the meta tab's
           auto-rune effect keeps working from any tab. -->
      <div class="tab-panel" class:active={tab === "overview"}>
        <OverviewTab {summoner} {ranked} {phase} {champSelect} {liveGame} {lobby} {queues} {actionError} {champions} {championById} {championByAlias} onAction={action} active={tab === "overview"} />
      </div>
      <div class="tab-panel" class:active={tab === "analysis"}>
        <AnalysisTab {analysis} {analysisLoading} onRefreshAnalysis={loadAnalysis} {phase} {scoutPlayers} {scoutReports} {scoutLoading} onRefreshScouting={loadScouting} {championById} {notes} onSaveNote={saveNote} {timesSeenBefore} {platform} clientConnected={status.connected} active={tab === "analysis"} />
      </div>
      <div class="tab-panel" class:active={tab === "meta"}>
        <MetaTab {champSelectChampionId} {myAssignedPosition} {championById} {champions} region={status.region} active={tab === "meta"} />
      </div>
      <div class="tab-panel" class:active={tab === "search"}>
        <SearchTab {championById} active={tab === "search"} />
      </div>
      <div class="tab-panel" class:active={tab === "live"}>
        <LiveTab {liveMetrics} {cooldowns} {liveEvents} {goalValue} {platform} clientConnected={status.connected} active={tab === "live"} />
      </div>
      <div class="tab-panel" class:active={tab === "goals"}>
        <GoalsTab {goalValue} {setGoal} {resetGoals} active={tab === "goals"} />
      </div>
      <div class="tab-panel" class:active={tab === "automation"}>
        <AutomationTab {champions} {championById} active={tab === "automation"} />
      </div>
      <div class="tab-panel" class:active={tab === "history"}>
        <HistoryTab {games} loading={loadingHistory} onRefresh={loadHistory} {championById} active={tab === "history"} />
      </div>
    {/if}
  {/if}
</div>

<style>
  .league-page {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    padding: var(--padding);
    max-width: 720px;
    margin: 0 auto;
    width: 100%;
    }

  .league-page :global(.tab-panel) {
    display: none;
    }

  .league-page :global(.tab-panel.active) {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    }

  .league-page :global(.guard-card) {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 10px;
    padding: calc(var(--padding) * 2);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--border-radius);
    }

  .league-page :global(.guard-card h2) {
    margin: 0;
    font-size: 18px;
    }

  .league-page :global(.guard-card p) {
    margin: 0;
    color: var(--gray);
    font-size: 13.5px;
    }

  .league-page :global(.league-header) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    }

  .league-page :global(.league-header h2) {
    margin: 0;
    font-size: 20px;
    }

  .league-page :global(.status-chip) {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 5px 12px;
    font-size: 12.5px;
    color: var(--gray);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 999px;
    }

  .league-page :global(.status-chip .dot) {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--gray);
    }

  .league-page :global(.status-chip.connected) {
    color: var(--text);
    }

  .league-page :global(.status-chip.connected .dot) {
    background: var(--success);
    }

  .league-page :global(.status-chip .region) {
    color: var(--gray);
    text-transform: uppercase;
    }

  .league-page :global(.profile-card) {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: var(--padding);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--border-radius);
    }

  .league-page :global(.profile-icon) {
    width: 56px;
    height: 56px;
    border-radius: 50%;
    border: 2px solid var(--border);
    object-fit: cover;
    }

  .league-page :global(.profile-info) {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    }

  .league-page :global(.profile-name) {
    font-size: 16px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    }

  .league-page :global(.profile-name .tag) {
    color: var(--gray);
    font-weight: 400;
    }

  .league-page :global(.profile-level) {
    font-size: 12.5px;
    color: var(--gray);
    }

  .league-page :global(.ranked-chips) {
    display: flex;
    gap: 8px;
    margin-left: auto;
    flex-wrap: wrap;
    }

  .league-page :global(.ranked-chip) {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 7px 12px;
    background: var(--button);
    border: 1px solid var(--input-border);
    border-radius: calc(var(--border-radius) - 2px);
    }

  .league-page :global(.ranked-queue) {
    font-size: 11px;
    color: var(--gray);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    }

  .league-page :global(.ranked-value) {
    font-size: 13px;
    }

  .league-page :global(.card) {
    display: flex;
    flex-direction: column;
    padding: var(--padding);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--border-radius);
    }

  .league-page :global(.card-head) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 10px;
    }

  .league-page :global(.card-head h3) {
    margin: 0;
    font-size: 15px;
    }

  .league-page :global(.phase-tag) {
    font-size: 12px;
    color: var(--gray);
    padding: 3px 10px;
    background: var(--button);
    border: 1px solid var(--input-border);
    border-radius: 999px;
    }

  .league-page :global(.action-error) {
    font-size: 12.5px;
    color: var(--danger);
    padding: 8px 12px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: calc(var(--border-radius) - 2px);
    }

  .league-page :global(.lobby-actions) {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    }

  .league-page :global(.searching-hint) {
    font-size: 13px;
    color: var(--gray);
    }

  .league-page :global(.queue-grid) {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    }

  .league-page :global(.empty-hint) {
    color: var(--gray);
    font-size: 13px;
    margin: 0;
    }

  .league-page :global(.team-picks) {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    }

  .league-page :global(.bench-row) {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 12px;
    flex-wrap: wrap;
    }

  .league-page :global(.bench-label) {
    font-size: 12.5px;
    color: var(--gray);
    }

  .league-page :global(.bench-champs) {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    }

  .league-page :global(.bench-swap) {
    padding: 0;
    background: none;
    border: 2px solid transparent;
    border-radius: 8px;
    cursor: pointer;
    line-height: 0;
    }

  .league-page :global(.bench-swap:hover),
  .league-page :global(.bench-swap:focus-visible) {
    border-color: var(--accent);
    outline: none;
    }

  .league-page :global(.live-teams) {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
    }

  @media (max-width: 560px) {
    .league-page :global(.live-teams) {
      grid-template-columns: 1fr;
        }
    }

  .league-page :global(.live-team) {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    }

  .league-page :global(.live-row) {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border-radius: calc(var(--border-radius) - 4px);
    min-width: 0;
    }

  .league-page :global(.live-row.me) {
    background: var(--accent-soft, var(--button));
    }

  .league-page :global(.live-name) {
    font-size: 12.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    }

  .league-page :global(.live-kda) {
    margin-left: auto;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: var(--gray);
    }

  .league-page :global(.live-respawn) {
    font-size: 11.5px;
    color: var(--danger);
    }

  .league-page :global(.action-row) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    }

  .league-page :global(.action-col) {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    }

  .league-page :global(.action-label) {
    font-size: 14px;
    }

  .league-page :global(.action-hint) {
    font-size: 12.5px;
    color: var(--gray);
    }

  .league-page :global(.divider) {
    height: 1px;
    background: var(--border);
    margin: 10px 0;
    }

  .league-page :global(.champ-list-block) {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 10px;
    }

  .league-page :global(.list-label) {
    font-size: 12.5px;
    color: var(--gray);
    }

  .league-page :global(.scout-notices) {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-bottom: 10px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--border-radius);
    background: var(--surface-hover);
    }

  .league-page :global(.scout-notice) {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-secondary);
    }

  .league-page :global(.reroll-actions) {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    }

  .league-page :global(.queue-filter) {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-bottom: 8px;
    }

  .league-page :global(.queue-chip) {
    padding: 3px 10px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-secondary);
    font-size: 12px;
    cursor: pointer;
    }

  .league-page :global(.queue-chip:hover) {
    background: var(--surface-hover);
    }

  .league-page :global(.queue-chip.on) {
    border-color: var(--accent);
    color: var(--text);
    }

  .league-page :global(.history-summary) {
    font-size: 12.5px;
    color: var(--text-secondary);
    margin: 0 0 10px;
    }

  .league-page :global(.objective-row) {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    }

  .league-page :global(.objective-chip) {
    display: inline-flex;
    align-items: baseline;
    gap: 6px;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--border-radius);
    font-variant-numeric: tabular-nums;
    font-size: 12.5px;
    }

  .league-page :global(.event-feed) {
    list-style: none;
    margin: 12px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    }

  .league-page :global(.event-item) {
    display: flex;
    gap: 8px;
    font-size: 12.5px;
    }

  .league-page :global(.event-time) {
    color: var(--gray);
    font-variant-numeric: tabular-nums;
    min-width: 42px;
    }

  .league-page :global(.event-text) {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
    }

  .league-page :global(.delay-block) {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 10px;
    }

  .league-page :global(.slider-row) {
    display: flex;
    align-items: center;
    gap: 10px;
    }

  .league-page :global(.slider-row input[type="range"]) {
    flex: 1;
    accent-color: var(--accent);
    }

  .league-page :global(.slider-edge) {
    font-size: 11.5px;
    color: var(--gray);
    min-width: 26px;
    }

  .league-page :global(.list-hint) {
    font-size: 11.5px;
    }

  .league-page :global(.champ-chips) {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    }

  .league-page :global(.champ-chip) {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 6px 3px 3px;
    font-size: 12.5px;
    background: var(--button);
    border: 1px solid var(--input-border);
    border-radius: 999px;
    }

  .league-page :global(.chip-remove) {
    background: none;
    border: none;
    color: var(--gray);
    font-size: 14px;
    cursor: pointer;
    padding: 0 3px;
    line-height: 1;
    }

  .league-page :global(.chip-remove:hover),
  .league-page :global(.chip-remove:focus-visible) {
    color: var(--danger);
    outline: none;
    }

  .league-page :global(.champ-search) {
    position: relative;
    max-width: 260px;
    }

  .league-page :global(.input-text) {
    width: 100%;
    padding: 7px 10px;
    font-size: 13px;
    background: var(--button);
    border: 1px solid var(--input-border);
    border-radius: calc(var(--border-radius) - 2px);
    color: var(--text);
    }

  .league-page :global(.input-text:focus-visible) {
    border-color: var(--accent);
    outline: none;
    }

  .league-page :global(.search-results) {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    right: 0;
    z-index: 10;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: calc(var(--border-radius) - 2px);
    overflow: hidden;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.25);
    }

  .league-page :global(.search-result) {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    font-size: 13px;
    background: none;
    border: none;
    color: var(--text);
    cursor: pointer;
    text-align: left;
    }

  .league-page :global(.search-result:hover),
  .league-page :global(.search-result:focus-visible) {
    background: var(--button);
    outline: none;
    }

  .league-page :global(.league-tabs) {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
    padding-bottom: 2px;
    border-bottom: 1px solid var(--border);
    }

  .league-page :global(.league-tab) {
    padding: 6px 12px;
    font-size: 12.5px;
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--gray);
    cursor: pointer;
    }

  .league-page :global(.league-tab:hover) {
    color: var(--text);
    }

  .league-page :global(.league-tab.on) {
    color: var(--text);
    border-bottom-color: var(--accent);
    }

  .league-page :global(.league-tab:focus-visible) {
    outline: 1px solid var(--accent);
    outline-offset: -1px;
    }

  .league-page :global(.winbar-wrap) {
    display: flex;
    flex-direction: column;
    gap: 6px;
    }

  .league-page :global(.winbar) {
    position: relative;
    height: 12px;
    border-radius: 999px;
    background: var(--button);
    border: 1px solid var(--input-border);
    overflow: hidden;
    }

  .league-page :global(.winbar-fill) {
    height: 100%;
    background: var(--accent);
    }

  .league-page :global(.winbar-range) {
    position: absolute;
    top: 0;
    height: 100%;
    background: var(--accent);
    opacity: 0.25;
    }

  .league-page :global(.winbar-legend) {
    display: flex;
    align-items: baseline;
    gap: 10px;
    }

  .league-page :global(.win-value) {
    font-size: 22px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    }

  .league-page :global(.win-range),
  .league-page :global(.win-note) {
    font-size: 12px;
    color: var(--gray);
    }

  .league-page :global(.win-note) {
    margin: 8px 0 0;
    }

  .league-page :global(.win-disclaimer) {
    margin: 4px 0 0;
    font-size: 11.5px;
    color: var(--gray);
    line-height: 1.45;
    }

  .league-page :global(.premade-row) {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    margin-top: 10px;
    }

  .league-page :global(.premade-source) {
    font-size: 0.75rem;
    color: var(--text-muted);
    }

  .league-page :global(.gold-summary) {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 14px;
    margin-bottom: 10px;
    font-size: 13px;
    flex-wrap: wrap;
    }

  .league-page :global(.gold-team) {
    color: var(--gray);
    }

  .league-page :global(.gold-diff) {
    font-size: 17px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    }

  .league-page :global(.gold-diff.good),
  .league-page :global(.diff.good) {
    color: var(--success);
    }

  .league-page :global(.gold-diff.bad),
  .league-page :global(.diff.bad) {
    color: var(--danger);
    }

  .league-page :global(.metric-table) {
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow-x: auto;
    }

  .league-page :global(.metric-head),
  .league-page :global(.metric-row) {
    display: grid;
    grid-template-columns: minmax(120px, 1.6fr) 68px 92px 64px minmax(110px, 1fr);
    gap: 8px;
    align-items: center;
    padding: 5px 7px;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    min-width: 460px;
    }

  .league-page :global(.metric-head) {
    color: var(--gray);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    }

  .league-page :global(.metric-row) {
    border-radius: calc(var(--border-radius) - 4px);
    background: var(--button);
    }

  .league-page :global(.metric-row.self) {
    background: var(--accent-soft, var(--surface));
    }

  .league-page :global(.metric-name) {
    display: flex;
    align-items: center;
    gap: 6px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    }

  .league-page :global(.pos-chip) {
    font-size: 9.5px;
    padding: 1px 5px;
    border-radius: 4px;
    background: var(--surface);
    color: var(--gray);
    letter-spacing: 0.03em;
    }

  .league-page :global(.dim) {
    color: var(--gray);
    }

  .league-page :global(.goal-list) {
    display: flex;
    flex-direction: column;
    gap: 8px;
    }

  .league-page :global(.goal-row) {
    display: grid;
    grid-template-columns: minmax(80px, 1fr) minmax(90px, 2fr) minmax(90px, 1fr);
    gap: 10px;
    align-items: center;
    font-size: 12.5px;
    }

  .league-page :global(.goal-name) {
    color: var(--gray);
    }

  .league-page :global(.goal-bar) {
    height: 8px;
    border-radius: 999px;
    background: var(--button);
    border: 1px solid var(--input-border);
    overflow: hidden;
    }

  .league-page :global(.goal-fill) {
    height: 100%;
    background: var(--gray);
    }

  .league-page :global(.goal-fill.met) {
    background: var(--success);
    }

  .league-page :global(.goal-value) {
    text-align: right;
    font-variant-numeric: tabular-nums;
    }

  .league-page :global(.goal-value.met) {
    color: var(--success);
    }

  .league-page :global(.goal-config) {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    gap: 10px;
    margin: 10px 0;
    }

  .league-page :global(.goal-field) {
    display: flex;
    flex-direction: column;
    gap: 4px;
    }

  .league-page :global(.goal-field-label) {
    font-size: 12px;
    color: var(--gray);
    }

  .league-page :global(.select-role) {
    padding: 5px 10px;
    font-size: 12.5px;
    background: var(--button);
    border: 1px solid var(--input-border);
    border-radius: calc(var(--border-radius) - 2px);
    color: var(--text);
    }

  .league-page :global(.search-form) {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    }

  .league-page :global(.search-form .input-text) {
    flex: 1;
    min-width: 180px;
    }

  .league-page :global(.stat-grid) {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(110px, 1fr));
    gap: 10px;
    }

  .league-page :global(.stat-cell) {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px 12px;
    background: var(--button);
    border: 1px solid var(--input-border);
    border-radius: calc(var(--border-radius) - 2px);
    }

  .league-page :global(.stat-value) {
    font-size: 19px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    }

  .league-page :global(.stat-value.good) {
    color: var(--success);
    }

  .league-page :global(.stat-value.bad) {
    color: var(--danger);
    }

  .league-page :global(.stat-label) {
    font-size: 11.5px;
    color: var(--gray);
    }

  .league-page :global(.champ-table) {
    display: flex;
    flex-direction: column;
    gap: 3px;
    }

  .league-page :global(.champ-row) {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 8px;
    background: var(--button);
    border-radius: calc(var(--border-radius) - 4px);
    font-size: 12.5px;
    font-variant-numeric: tabular-nums;
    }

  .league-page :global(.champ-row-name) {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    }

  .league-page :global(.champ-row-games),
  .league-page :global(.champ-row-kda),
  .league-page :global(.champ-row-cs) {
    color: var(--gray);
    flex-shrink: 0;
    }

  .league-page :global(.champ-row-wr) {
    flex-shrink: 0;
    font-weight: 600;
    }

  .league-page :global(.champ-row-wr.good) {
    color: var(--success);
    }

  .league-page :global(.champ-row-wr.bad) {
    color: var(--danger);
    }

  .league-page :global(.zone-bars) {
    display: flex;
    flex-direction: column;
    gap: 7px;
    }

  .league-page :global(.zone-row) {
    display: grid;
    grid-template-columns: 60px 1fr 52px;
    gap: 10px;
    align-items: center;
    font-size: 12.5px;
    }

  .league-page :global(.zone-name) {
    color: var(--gray);
    }

  .league-page :global(.chat-send) {
    display: flex;
    gap: 8px;
    margin-top: 12px;
    flex-wrap: wrap;
    }

  .league-page :global(.chat-send .input-text) {
    flex: 1;
    min-width: 180px;
    }

  .league-page :global(.impact) {
    color: var(--accent);
    font-weight: 600;
    }

  .league-page :global(.rune-list) {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 10px;
    }

  .league-page :global(.rune-card) {
    display: flex;
    flex-direction: column;
    gap: 7px;
    padding: 9px 11px;
    background: var(--button);
    border: 1px solid var(--input-border);
    border-radius: calc(var(--border-radius) - 2px);
    }

  .league-page :global(.rune-card.applied) {
    border-color: var(--accent);
    }

  .league-page :global(.rune-head) {
    display: flex;
    align-items: center;
    gap: 8px;
    }

  .league-page :global(.rune-keystone) {
    font-size: 13px;
    font-weight: 600;
    }

  .league-page :global(.rune-perks) {
    display: flex;
    gap: 5px;
    flex-wrap: wrap;
    }

  .league-page :global(.perk-icon) {
    width: 26px;
    height: 26px;
    border-radius: 50%;
    background: var(--surface);
    }

  .league-page :global(.rune-foot) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    font-size: 12px;
    flex-wrap: wrap;
    }

  .league-page :global(.tier-controls) {
    display: flex;
    gap: 6px;
    align-items: center;
    }

  .league-page :global(.tier-badge) {
    flex-shrink: 0;
    min-width: 30px;
    text-align: center;
    padding: 2px 6px;
    border-radius: 4px;
    font-size: 11px;
    font-weight: 700;
    background: var(--surface);
    border: 1px solid var(--border);
    }

  .league-page :global(.tier-badge.tier-1) {
    color: var(--on-accent);
    background: var(--accent);
    border-color: transparent;
    }

  .league-page :global(.tier-badge.tier-2) {
    color: var(--success);
    border-color: var(--success);
    }

  .league-page :global(.tier-badge.tier-3) {
    color: var(--text);
    }

  .league-page :global(.build-picker) {
    margin: 8px 0;
    }

  .league-page :global(.build-items) {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    margin: 8px 0;
    }

  .league-page :global(.build-item) {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    font-size: 10.5px;
    }

  .league-page :global(.item-icon) {
    width: 34px;
    height: 34px;
    border-radius: 6px;
    background: var(--button);
    }

  .league-page :global(.cd-list) {
    display: flex;
    flex-direction: column;
    gap: 5px;
    }

  .league-page :global(.cd-row) {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 8px;
    background: var(--button);
    border-radius: calc(var(--border-radius) - 4px);
    flex-wrap: wrap;
    }

  .league-page :global(.cd-champ) {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 130px;
    }

  .league-page :global(.cd-id) {
    display: flex;
    flex-direction: column;
    gap: 1px;
    font-size: 11.5px;
    }

  .league-page :global(.cd-name) {
    font-size: 12.5px;
    font-weight: 600;
    }

  .league-page :global(.cd-abilities) {
    display: flex;
    gap: 8px;
    margin-left: auto;
    flex-wrap: wrap;
    }

  .league-page :global(.cd-ability) {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1px;
    min-width: 38px;
    }

  .league-page :global(.ability-icon) {
    width: 26px;
    height: 26px;
    border-radius: 5px;
    background: var(--surface);
    }

  .league-page :global(.cd-key) {
    font-size: 9.5px;
    color: var(--gray);
    letter-spacing: 0.05em;
    }

  .league-page :global(.cd-value) {
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
    }

  .league-page :global(.scout-teams) {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 14px;
    }

  @media (max-width: 620px) {
    .league-page :global(.scout-teams) {
      grid-template-columns: 1fr;
        }
    }

  .league-page :global(.scout-team) {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    }

  .league-page :global(.scout-team-title) {
    margin: 0 0 2px;
    font-size: 11.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--gray);
    }

  .league-page :global(.scout-team-title.enemy) {
    color: var(--danger);
    }

  .league-page :global(.scout-row) {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 7px 9px;
    background: var(--button);
    border: 1px solid var(--input-border);
    border-radius: calc(var(--border-radius) - 3px);
    }

  .league-page :global(.scout-main) {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    }

  .league-page :global(.scout-id) {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    flex: 1;
    }

  .league-page :global(.scout-name) {
    font-size: 12.5px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    }

  .league-page :global(.scout-name .tag) {
    color: var(--gray);
    font-weight: 400;
    }

  .league-page :global(.scout-rank) {
    font-size: 11px;
    color: var(--gray);
    }

  .league-page :global(.scout-stats) {
    display: flex;
    flex-direction: column;
    gap: 1px;
    align-items: flex-end;
    flex-shrink: 0;
    }

  .league-page :global(.scout-wr) {
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    }

  .league-page :global(.scout-wr.good) {
    color: var(--success);
    }

  .league-page :global(.scout-wr.bad) {
    color: var(--danger);
    }

  .league-page :global(.scout-kda) {
    font-size: 11px;
    color: var(--gray);
    font-variant-numeric: tabular-nums;
    }

  .league-page :global(.scout-private) {
    font-size: 11.5px;
    color: var(--gray);
    flex-shrink: 0;
    }

  .league-page :global(.note-toggle) {
    background: none;
    border: none;
    color: var(--gray);
    font-size: 13px;
    cursor: pointer;
    padding: 2px 4px;
    border-radius: 4px;
    flex-shrink: 0;
    }

  .league-page :global(.note-toggle.has-note),
  .league-page :global(.note-toggle:hover),
  .league-page :global(.note-toggle:focus-visible) {
    color: var(--accent);
    outline: none;
    }

  .league-page :global(.scout-champs) {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    }

  .league-page :global(.scout-champ) {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    }

  .league-page :global(.scout-champ-record) {
    font-size: 10.5px;
    color: var(--gray);
    font-variant-numeric: tabular-nums;
    }

  .league-page :global(.scout-tag) {
    font-size: 10.5px;
    padding: 2px 7px;
    border-radius: 999px;
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text);
    }

  .league-page :global(.note-input) {
    font-size: 12px;
    padding: 5px 8px;
    }

  .league-page :global(.history-section) {
    display: flex;
    flex-direction: column;
    gap: 8px;
    }

  .league-page :global(.history-head) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    }

  .league-page :global(.history-head h3) {
    margin: 0;
    font-size: 15px;
    }

  .league-page :global(.game-list) {
    display: flex;
    flex-direction: column;
    gap: 6px;
    }

  .league-page :global(.game-row) {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 9px 12px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: calc(var(--border-radius) - 2px);
    font: inherit;
    color: inherit;
    text-align: left;
    width: 100%;
    cursor: pointer;
    }

  .league-page :global(.game-row:hover),
  .league-page :global(.game-row.expanded) {
    border-color: var(--accent);
    }

  .league-page :global(.champ-icon) {
    width: 34px;
    height: 34px;
    border-radius: 6px;
    object-fit: cover;
    background: var(--button);
    }

  .league-page :global(.champ-icon.small) {
    width: 24px;
    height: 24px;
    border-radius: 5px;
    }

  .league-page :global(.champ-icon.tiny) {
    width: 20px;
    height: 20px;
    border-radius: 4px;
    }

  .league-page :global(.champ-empty) {
    border: 1px dashed var(--input-border);
    }

  .league-page :global(.game-info) {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    }

  .league-page :global(.game-result) {
    font-size: 13px;
    font-weight: 600;
    }

  .league-page :global(.game-result.win) {
    color: var(--success);
    }

  .league-page :global(.game-result.loss) {
    color: var(--danger);
    }

  .league-page :global(.game-mode) {
    font-size: 11.5px;
    color: var(--gray);
    }

  .league-page :global(.game-kda) {
    margin-left: auto;
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    }

  .league-page :global(.game-time) {
    font-size: 11.5px;
    color: var(--gray);
    min-width: 70px;
    text-align: right;
    }

  .league-page :global(.button) {
    padding: 6px 14px;
    font-size: 13px;
    background: var(--button);
    border: 1px solid var(--input-border);
    border-radius: calc(var(--border-radius) - 2px);
    color: var(--text);
    cursor: pointer;
    }

  .league-page :global(.button:hover) {
    background: var(--button-elevated, var(--button));
    }

  .league-page :global(.button:focus-visible) {
    border-color: var(--accent);
    outline: none;
    }

  .league-page :global(.button.primary) {
    background: var(--accent);
    color: var(--on-accent);
    border-color: transparent;
    }

  .league-page :global(.button:disabled) {
    opacity: 0.6;
    cursor: default;
    }

  .league-page :global(.button.danger) {
    background: var(--danger);
    color: var(--on-status, var(--on-accent));
    border-color: transparent;
    }

  .league-page :global(.button.subtle) {
    color: var(--text-secondary);
    background: transparent;
    }

  .league-page :global(.button.subtle:hover) {
    color: var(--text);
    background: var(--surface-hover);
    }

  .league-page :global(.scoreboard-row.full) {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 6px 0;
    border-bottom: 1px solid var(--border);
    }

  .league-page :global(.sb-identity) {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    }

  .league-page :global(.sb-spells),
  .league-page :global(.sb-runes) {
    display: flex;
    gap: 2px;
    }

  .league-page :global(.sb-spell),
  .league-page :global(.sb-rune) {
    width: 14px;
    height: 14px;
    border-radius: 3px;
    }

  .league-page :global(.sb-rune.keystone) {
    width: 16px;
    height: 16px;
    }

  .league-page :global(.sb-name) {
    font-size: 12.5px;
    min-width: 110px;
    }

  .league-page :global(.sb-name.link) {
    background: none;
    border: none;
    padding: 0;
    color: var(--text);
    cursor: pointer;
    text-align: left;
    }

  .league-page :global(.sb-name.link:hover) {
    color: var(--accent);
    text-decoration: underline;
    }

  .league-page :global(.sb-items) {
    display: flex;
    gap: 2px;
    }

  .league-page :global(.item-icon.tiny),
  .league-page :global(.item-empty) {
    width: 16px;
    height: 16px;
    border-radius: 3px;
    }

  .league-page :global(.item-empty) {
    border: 1px solid var(--border);
    display: inline-block;
    }

  .league-page :global(.sb-stats) {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    }

  .league-page :global(.sb-flag) {
    font-size: 10.5px;
    color: var(--accent);
    }

  .league-page :global(.lookup-drawer) {
    margin-top: 12px;
    padding-top: 10px;
    border-top: 1px solid var(--border);
    }

  .league-page :global(.game-row.static) {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: default;
    }

  .league-page :global(.feature-badge) {
    font-size: 10px;
    font-weight: 400;
    color: var(--text-muted);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 1px 6px;
    margin-left: 6px;
    vertical-align: middle;
    }

  .league-page :global(.profile-tools) {
    margin-bottom: 10px;
    }

  .league-page :global(.profile-tools summary) {
    cursor: pointer;
    font-size: 12.5px;
    color: var(--text-secondary);
    padding: 4px 0;
    }

  .league-page :global(.profile-tool-row) {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    margin: 8px 0;
    }

  .league-page :global(.tiny-input) {
    max-width: 96px;
    }

  .league-page :global(.skin-options) {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    }

  .league-page :global(.profile-saved) {
    font-size: 12px;
    color: var(--success);
    margin: 4px 0;
    }

  .league-page :global(.skill-order) {
    display: flex;
    gap: 3px;
    flex-wrap: wrap;
    margin: 6px 0 10px;
    }

  .league-page :global(.skill-step) {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border: 1px solid var(--border);
    border-radius: 4px;
    font-size: 10.5px;
    color: var(--text-secondary);
    }

  .league-page :global(.skill-step.ult) {
    border-color: var(--accent);
    color: var(--text);
    }

  .league-page :global(.build-phase) {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-bottom: 8px;
    }

  .league-page :global(.scout-score) {
    font-size: 11.5px;
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
    }

  .league-page :global(.objective-line) {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
    font-size: 11.5px;
    color: var(--text-secondary);
    margin: 4px 0;
    }

  .league-page :global(.ban-line) {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: wrap;
    font-size: 11.5px;
    margin-bottom: 6px;
    }

  .league-page :global(.repair-row) {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    flex-wrap: wrap;
    margin-bottom: 10px;
    }

  .league-page :global(.repair-note) {
    font-size: 12px;
    color: var(--text-secondary);
    }

  .league-page :global(.button.subtle-danger) {
    color: var(--danger);
    border-color: color-mix(in oklab, var(--danger) 35%, transparent);
    background: transparent;
    }

  .league-page :global(.button.subtle-danger:hover) {
    background: color-mix(in oklab, var(--danger) 10%, transparent);
    }

  .league-page :global(.dodge-row) {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 10px;
    }

  .league-page :global(.dodge-warning) {
    font-size: 12.5px;
    color: var(--text-secondary, var(--text));
    margin-right: auto;
    }

  .league-page :global(.seg-group) {
    display: flex;
    border: 1px solid var(--input-border);
    border-radius: calc(var(--border-radius) - 2px);
    overflow: hidden;
    }

  .league-page :global(.seg) {
    padding: 5px 12px;
    font-size: 12.5px;
    background: transparent;
    border: none;
    color: var(--text-secondary, var(--text));
    cursor: pointer;
    }

  .league-page :global(.seg + .seg) {
    border-left: 1px solid var(--input-border);
    }

  .league-page :global(.seg.on) {
    background: var(--accent);
    color: var(--on-accent);
    }

  .league-page :global(.action-row.stacked) {
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
    }

  .league-page :global(.message-input) {
    width: 100%;
    }

  .league-page :global(.seen-badge) {
    font-size: 11px;
    padding: 1px 6px;
    border-radius: 999px;
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    color: var(--accent);
    margin-left: 6px;
    white-space: nowrap;
    }

  .league-page :global(.spell-timer) {
    cursor: pointer;
    border: 1px solid var(--input-border);
    background: var(--surface);
    font: inherit;
    color: inherit;
    }

  .league-page :global(.spell-timer.running) {
    border-color: var(--accent);
    background: color-mix(in oklab, var(--accent) 12%, transparent);
    }

  .league-page :global(.spell-timer.running .cd-value) {
    color: var(--accent);
    font-weight: 600;
    }

  .league-page :global(.game-chevron) {
    color: var(--text-secondary, var(--text));
    font-size: 12px;
    }

  .league-page :global(.scoreboard) {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
    padding: 10px 12px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: calc(var(--border-radius) - 2px);
    }

  .league-page :global(.scoreboard-team) {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    }

  .league-page :global(.scoreboard-result) {
    font-size: 12px;
    font-weight: 600;
    }

  .league-page :global(.scoreboard-result.win) {
    color: var(--success, var(--accent));
    }

  .league-page :global(.scoreboard-result.loss) {
    color: var(--danger);
    }

  .league-page :global(.scoreboard-row) {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    min-width: 0;
    }

  .league-page :global(.scoreboard-name) {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    }

  .league-page :global(.scoreboard-kda) {
    white-space: nowrap;
    }

  .league-page :global(.scoreboard-cs),
  .league-page :global(.scoreboard-gold),
  .league-page :global(.scoreboard-dmg) {
    white-space: nowrap;
    }

  @media (max-width: 560px) {
    .league-page :global(.scoreboard) {
      grid-template-columns: 1fr;
        }

    .league-page :global(.scoreboard-cs),
    .league-page :global(.scoreboard-dmg) {
      display: none;
        }
    }

  .league-page :global(.toggle) {
    position: relative;
    width: 40px;
    height: 22px;
    border-radius: 999px;
    background: var(--button);
    border: 1px solid var(--input-border);
    cursor: pointer;
    flex-shrink: 0;
    }

  .league-page :global(.toggle:focus-visible) {
    border-color: var(--accent);
    outline: none;
    }

  .league-page :global(.toggle .toggle-knob) {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--gray);
    transition: transform 0.15s ease, background 0.15s ease;
    }

  .league-page :global(.toggle.on .toggle-knob) {
    transform: translateX(18px);
    background: var(--accent);
    }

  @media (prefers-reduced-motion: reduce) {
    .league-page :global(.toggle .toggle-knob) {
      transition: none;
        }
    }
</style>
