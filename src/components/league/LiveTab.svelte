<script lang="ts">
  import { t } from "$lib/i18n";
  import { CDRAGON, ROLES, assetUrl, formatGameTime, type GoalKey, type Role } from "./shared";
  import { availability, featureById, needsBadge, type Platform } from "./registry";

  let {
    liveMetrics,
    cooldowns,
    liveEvents,
    goalValue,
    platform,
    clientConnected,
  }: {
    liveMetrics: any;
    cooldowns: any;
    liveEvents: any;
    goalValue: (role: Role, key: GoalKey) => number;
    platform?: Platform;
    clientConnected?: boolean;
  } = $props();

  let context = $derived({
    platform: platform ?? "macos",
    clientConnected: clientConnected ?? true,
    inGame: Boolean(liveMetrics?.players?.length),
  });

  let objectivesFeature = featureById("objectives");
  let objectivesAvailability = $derived(
    objectivesFeature ? availability(objectivesFeature, context) : { available: true as const },
  );

  let liveFeature = featureById("live-metrics");
  let liveAvailability = $derived(
    liveFeature ? availability(liveFeature, context) : { available: true as const },
  );

  let selfRow = $derived(liveMetrics?.players?.find((r: any) => r.isSelf) ?? null);
  let myTeam = $derived(selfRow?.team ?? "ORDER");
  let enemyTeam = $derived(myTeam === "ORDER" ? "CHAOS" : "ORDER");
  let teamGoldLead = $derived(
    (liveMetrics?.teamGold?.[myTeam] ?? 0) - (liveMetrics?.teamGold?.[enemyTeam] ?? 0)
  );

  let liveGoals = $derived.by(() => {
    if (!selfRow) return [];
    const role: Role = (ROLES as readonly string[]).includes(selfRow.position)
      ? (selfRow.position as Role)
      : "MIDDLE";
    const minutes = Math.max((liveMetrics?.gameTime ?? 0) / 60, 0.1);
    const visionPerMin = (selfRow.visionScore ?? 0) / minutes;
    return [
      {
        key: "csPerMin",
        labelKey: "league.goal_cs",
        current: (selfRow.csPerMin ?? 0).toFixed(1),
        target: goalValue(role, "csPerMin").toFixed(1),
        ratio: (selfRow.csPerMin ?? 0) / goalValue(role, "csPerMin"),
      },
      {
        key: "goldPerMin",
        labelKey: "league.goal_gold",
        current: String(selfRow.goldPerMin ?? 0),
        target: String(goalValue(role, "goldPerMin")),
        ratio: (selfRow.goldPerMin ?? 0) / goalValue(role, "goldPerMin"),
      },
      {
        key: "kda",
        labelKey: "league.goal_kda",
        current: (selfRow.kda ?? 0).toFixed(2),
        target: goalValue(role, "kda").toFixed(1),
        ratio: (selfRow.kda ?? 0) / goalValue(role, "kda"),
      },
      {
        key: "visionPerMin",
        labelKey: "league.goal_vision",
        current: visionPerMin.toFixed(1),
        target: goalValue(role, "visionPerMin").toFixed(1),
        ratio: visionPerMin / goalValue(role, "visionPerMin"),
      },
    ];
  });

  let enemyCooldowns = $derived(
    (cooldowns?.players ?? []).filter((p: any) => p.team && p.team !== myTeam)
  );

  let spellTimers = $state<Record<string, number>>({});
  let timerNow = $state(Date.now());

  $effect(() => {
    if (Object.keys(spellTimers).length === 0) return;
    const tick = setInterval(() => {
      timerNow = Date.now();
      const expired = Object.entries(spellTimers).filter(([, end]) => end <= Date.now());
      if (expired.length > 0) {
        const next = { ...spellTimers };
        for (const [key] of expired) delete next[key];
        spellTimers = next;
      }
    }, 500);
    return () => clearInterval(tick);
  });

  function toggleSpellTimer(riotId: string, spell: any) {
    const key = `${riotId}:${spell.name}`;
    if (spellTimers[key]) {
      const next = { ...spellTimers };
      delete next[key];
      spellTimers = next;
    } else if (spell.cooldown > 0) {
      timerNow = Date.now();
      spellTimers = { ...spellTimers, [key]: Date.now() + spell.cooldown * 1000 };
    }
  }

  function spellRemaining(riotId: string, spell: any): number | null {
    const end = spellTimers[`${riotId}:${spell.name}`];
    if (!end) return null;
    return Math.max(0, Math.ceil((end - timerNow) / 1000));
  }

  // The objective payload only refreshes every few seconds, so the countdown is
  // continued locally instead of jumping in steps.
  let eventsFetchedAt = $state(Date.now());
  let objectiveNow = $state(Date.now());

  $effect(() => {
    void liveEvents;
    eventsFetchedAt = Date.now();
    objectiveNow = Date.now();
  });

  $effect(() => {
    if (!liveEvents?.objectives?.length) return;
    const tick = setInterval(() => {
      objectiveNow = Date.now();
    }, 1000);
    return () => clearInterval(tick);
  });

  let objectives = $derived.by(() => {
    const elapsed = (objectiveNow - eventsFetchedAt) / 1000;
    return (liveEvents?.objectives ?? [])
      .map((o: any) => ({ ...o, left: Math.max(0, Math.round(o.remaining - elapsed)) }))
      .filter((o: any) => o.left > 0);
  });

  const EVENT_LABELS: Record<string, string> = {
    ChampionKill: "league.event_kill",
    TurretKilled: "league.event_turret",
    InhibKilled: "league.event_inhib",
    DragonKill: "league.event_dragon",
    BaronKill: "league.event_baron",
    HeraldKill: "league.event_herald",
    Multikill: "league.event_multikill",
    Ace: "league.event_ace",
    FirstBrick: "league.event_first_turret",
    FirstBlood: "league.event_first_blood",
  };

  let feed = $derived(
    (liveEvents?.events ?? []).filter((e: any) => EVENT_LABELS[e.name])
  );
</script>

{#if liveMetrics?.players?.length}
  <section class="card">
    <div class="card-head">
      <h3>{$t("league.gold_title")}</h3>
      <span class="phase-tag">{formatGameTime(liveMetrics.gameTime ?? 0)}</span>
    </div>
    <p class="win-disclaimer">{$t("league.col_gold_hint")}</p>
    <div class="gold-summary">
      <span class="gold-team">{$t("league.your_team")}: <strong>{liveMetrics.teamGold?.[myTeam] ?? 0}</strong></span>
      <span class="gold-diff" class:good={teamGoldLead > 0} class:bad={teamGoldLead < 0}>
        {teamGoldLead > 0 ? "+" : ""}{teamGoldLead}
      </span>
      <span class="gold-team">{$t("league.enemy_team")}: <strong>{liveMetrics.teamGold?.[enemyTeam] ?? 0}</strong></span>
    </div>
    <div class="metric-table" role="table">
      <div class="metric-head" role="row">
        <span role="columnheader">{$t("league.col_player")}</span>
        <span role="columnheader">KDA</span>
        <span role="columnheader">CS</span>
        <span role="columnheader" title={$t("league.col_gold_hint") as string}>{$t("league.col_gold")}</span>
        <span role="columnheader">{$t("league.col_diff")}</span>
      </div>
      {#each liveMetrics.players as row (row.riotId)}
        <div class="metric-row" role="row" class:self={row.isSelf}>
          <span class="metric-name" role="cell">
            <span class="pos-chip">{(row.position ?? "?").slice(0, 3)}</span>
            {row.championName}
          </span>
          <span role="cell">{row.kills}/{row.deaths}/{row.assists}</span>
          <span role="cell">{row.cs} <span class="dim">({row.csPerMin}/m)</span></span>
          <span role="cell">{row.itemGold}</span>
          <span role="cell" class="diff" class:good={(row.goldDiff ?? 0) > 0} class:bad={(row.goldDiff ?? 0) < 0}>
            {#if row.goldDiff !== undefined && row.goldDiff !== null}
              {row.goldDiff > 0 ? "+" : ""}{row.goldDiff}g
              <span class="dim">{(row.csDiff ?? 0) > 0 ? "+" : ""}{Math.round(row.csDiff ?? 0)}cs</span>
            {:else}—{/if}
          </span>
        </div>
      {/each}
    </div>
  </section>

  {#if enemyCooldowns.length > 0}
    <section class="card">
      <div class="card-head">
        <h3>{$t("league.cd_title")}</h3>
        <span class="phase-tag">{$t("league.cd_estimated")}</span>
      </div>
      <p class="win-disclaimer">{$t("league.cd_desc")}</p>
      <div class="cd-list">
        {#each enemyCooldowns as p (p.riotId)}
          <div class="cd-row">
            <div class="cd-champ">
              {#if p.championId > 0}
                <img class="champ-icon small" src={`${CDRAGON}/champion-icons/${p.championId}.png`} alt="" loading="lazy" />
              {/if}
              <div class="cd-id">
                <span class="cd-name">{p.championName}</span>
                <span class="dim">{$t("league.cd_haste")} {p.abilityHaste} · lv{p.level}</span>
              </div>
            </div>
            <div class="cd-abilities">
              {#each p.abilities as ab (ab.key)}
                <span class="cd-ability" title={`${ab.name ?? ""} (${ab.key})`}>
                  {#if ab.iconPath}
                    <img class="ability-icon" src={assetUrl(ab.iconPath)} alt="" loading="lazy" onerror={(e) => { (e.currentTarget as HTMLImageElement).style.visibility = "hidden"; }} />
                  {/if}
                  <span class="cd-key">{ab.key}</span>
                  <span class="cd-value">{ab.cooldown > 0 ? `${ab.cooldown}s` : "—"}</span>
                </span>
              {/each}
              {#each p.spellTimers ?? [] as sp (sp.name)}
                {@const remaining = spellRemaining(p.riotId, sp)}
                <button
                  class="cd-ability spell-timer"
                  class:running={remaining !== null}
                  onclick={() => toggleSpellTimer(p.riotId, sp)}
                  title={`${sp.name} — ${$t("league.spell_timer_hint")}`}
                  aria-pressed={remaining !== null}
                >
                  {#if sp.iconPath}
                    <img class="ability-icon" src={assetUrl(sp.iconPath)} alt="" loading="lazy" onerror={(e) => { (e.currentTarget as HTMLImageElement).style.visibility = "hidden"; }} />
                  {/if}
                  <span class="cd-value">{remaining !== null ? `${remaining}s` : sp.cooldown > 0 ? `${sp.cooldown}s` : "—"}</span>
                </button>
              {/each}
            </div>
          </div>
        {/each}
      </div>
    </section>
  {/if}

  {#if objectives.length > 0 || feed.length > 0}
    <section class="card">
      <div class="card-head">
        <h3>
          {$t("league.objectives_title")}
          {#if objectivesFeature && needsBadge(objectivesFeature)}
            <span class="feature-badge">{$t(`league.badge_${objectivesFeature.state}`)}</span>
          {/if}
        </h3>
        <h3>{$t("league.objectives_title")}</h3>
      </div>
      {#if objectives.length > 0}
        <div class="objective-row">
          {#each objectives as objective (objective.kind)}
            <span class="objective-chip">
              <strong>{$t(`league.objective_${objective.kind}`)}</strong>
              {formatGameTime(objective.left)}
            </span>
          {/each}
        </div>
        <p class="win-disclaimer">{$t("league.objectives_estimate")}</p>
      {/if}
      {#if feed.length > 0}
        <ul class="event-feed">
          {#each feed as event (`${event.id}:${event.at}`)}
            <li class="event-item">
              <span class="event-time">{formatGameTime(event.at)}</span>
              <span class="event-text">
                {$t(EVENT_LABELS[event.name])}
                {#if event.actor}<strong>{event.actor}</strong>{/if}
                {#if event.target}<span class="dim">→ {event.target}</span>{/if}
                {#if event.detail}<span class="dim">({event.detail})</span>{/if}
              </span>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  {/if}

  {#if selfRow}
    <section class="card">
      <div class="card-head">
        <h3>{$t("league.goals_live")}</h3>
        <span class="phase-tag">{selfRow.position ?? "?"}</span>
      </div>
      <div class="goal-list">
        {#each liveGoals as goal (goal.key)}
          <div class="goal-row">
            <span class="goal-name">{$t(goal.labelKey)}</span>
            <div class="goal-bar"><div class="goal-fill" class:met={goal.ratio >= 1} style={`width:${Math.min(goal.ratio * 100, 100)}%`}></div></div>
            <span class="goal-value" class:met={goal.ratio >= 1}>{goal.current} <span class="dim">/ {goal.target}</span></span>
          </div>
        {/each}
      </div>
    </section>
  {/if}
{:else}
  <div class="guard-card">
    <p>{liveAvailability.available ? $t("league.gold_unavailable") : $t(liveAvailability.reasonKey)}</p>
  </div>
{/if}
