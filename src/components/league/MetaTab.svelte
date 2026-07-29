<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";
  import { featureById, needsBadge } from "./registry";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
  import { CDRAGON, type Champion } from "./shared";

  let {
    champSelectChampionId,
    myAssignedPosition,
    championById,
    champions,
    region,
  }: {
    champSelectChampionId: number;
    myAssignedPosition: string;
    championById: Map<number, Champion>;
    champions: Champion[];
    region: string | null;
  } = $props();

  let settings = $derived(getSettings());

  function toggleAutoRunes() {
    const current = settings?.league?.auto_runes ?? false;
    updateSettings({ league: { auto_runes: !current } });
  }

  let runePages = $state<any[]>([]);
  let runeApplying = $state(false);
  let runeError = $state("");
  let appliedRuneIndex = $state<number | null>(null);
  let lastRuneChampion = 0;

  let perkMeta = $state<Record<number, string>>({});
  let spellMeta = $state<Record<number, string>>({});

  function perkName(id: number): string {
    return perkMeta[id] ?? String(id);
  }

  function spellName(id: number): string {
    return spellMeta[id] ?? String(id);
  }

  async function loadPerkMeta() {
    try {
      const perks = await invoke<any[]>("league_get", { path: "/lol-perks/v1/perks" });
      const map: Record<number, string> = {};
      for (const p of perks ?? []) map[p.id] = p.name;
      perkMeta = map;
    } catch {
      perkMeta = {};
    }
    try {
      const spells = await invoke<any[]>("league_get", {
        path: "/lol-game-data/assets/v1/summoner-spells.json",
      });
      const map: Record<number, string> = {};
      for (const s of spells ?? []) map[s.id] = s.name;
      spellMeta = map;
    } catch {
      spellMeta = {};
    }
  }

  async function loadRunes(championId: number) {
    if (championId <= 0) {
      runePages = [];
      return;
    }
    runeError = "";
    try {
      const result = await invoke<any>("league_rune_recommendations", {
        championId,
        position: myAssignedPosition || null,
      });
      runePages = result?.pages ?? [];
      appliedRuneIndex = null;
    } catch (e: any) {
      runePages = [];
      runeError = translateBackendError(typeof e === "string" ? e : (e?.message ?? String(e)), (k) => $t(k) as string);
    }
  }

  async function applyRunePage(index: number) {
    const page = runePages[index];
    if (!page || runeApplying) return;
    runeApplying = true;
    runeError = "";
    try {
      const champName = championById.get(champSelectChampionId)?.name ?? "";
      await invoke("league_apply_runes", {
        name: `${champName} ${page.keystoneName ?? ""}`.trim(),
        primaryStyleId: page.primaryStyleId,
        subStyleId: page.subStyleId,
        selectedPerkIds: page.selectedPerkIds,
        spell1: page.summonerSpellIds?.[0] ?? null,
        spell2: page.summonerSpellIds?.[1] ?? null,
      });
      appliedRuneIndex = index;
    } catch (e: any) {
      runeError = translateBackendError(typeof e === "string" ? e : (e?.message ?? String(e)), (k) => $t(k) as string);
    } finally {
      runeApplying = false;
    }
  }

  // Load recommendations when the locked champion changes; apply the first one
  // automatically only when the user asked for it. This component stays mounted
  // even while its panel is hidden, so auto-apply works from any tab.
  $effect(() => {
    const champ = champSelectChampionId;
    if (champ > 0 && champ !== lastRuneChampion) {
      lastRuneChampion = champ;
      loadRunes(champ).then(() => {
        if (settings?.league?.auto_runes && runePages.length > 0) {
          applyRunePage(0);
        }
      });
    } else if (champ === 0 && lastRuneChampion !== 0) {
      lastRuneChampion = 0;
      runePages = [];
    }
  });

  const buildFeature = featureById("build-reference");
  let metaInfo = $state<any>(null);
  let metaLoading = $state(false);
  let metaError = $state("");

  async function loadMeta(championId: number) {
    if (championId <= 0 || metaLoading) return;
    metaLoading = true;
    metaError = "";
    try {
      metaInfo = await invoke<any>("league_champion_meta", {
        championId,
        position: myAssignedPosition ? opggPosition(myAssignedPosition) : undefined,
      });
    } catch (e: any) {
      metaInfo = null;
      metaError = typeof e === "string" ? e : (e?.message ?? String(e));
    } finally {
      metaLoading = false;
    }
  }

  // op.gg names positions differently from the client.
  function opggPosition(position: string): string {
    const map: Record<string, string> = {
      TOP: "top",
      JUNGLE: "jungle",
      MIDDLE: "mid",
      BOTTOM: "adc",
      UTILITY: "support",
    };
    return map[position] ?? "mid";
  }

  function itemIcon(id: number): string {
    return `${CDRAGON}/../../game/assets/items/icons2d/${id}.png`;
  }

  let buildChampionId = $state<number>(0);
  let buildInfo = $state<any>(null);
  let buildLoading = $state(false);

  async function loadBuild(championId: number) {
    if (championId <= 0 || buildLoading) return;
    buildLoading = true;
    try {
      buildInfo = await invoke<any>("league_champion_build", { championId, sample: 20 });
    } catch {
      buildInfo = null;
    } finally {
      buildLoading = false;
    }
  }

  const TIER_POSITIONS = ["TOP", "JUNGLE", "MID", "ADC", "SUPPORT"] as const;
  let tierPosition = $state<string>("JUNGLE");
  let tiers = $state<any>(null);
  let tiersLoading = $state(false);
  let tiersError = $state("");

  // op.gg encodes tiers as 1 = best; the labels mirror what their site shows.
  function tierLabel(tier: number): string {
    if (tier <= 1) return "S+";
    if (tier === 2) return "S";
    if (tier === 3) return "A";
    if (tier === 4) return "B";
    if (tier === 5) return "C";
    return "D";
  }

  let tierRows = $derived.by(() => {
    const wanted = tierPosition;
    const rows: any[] = [];
    for (const champ of tiers?.champions ?? []) {
      const entry = (champ.positions ?? []).find((p: any) => (p.position ?? "").toUpperCase() === wanted);
      if (!entry || entry.tier === null || entry.tier === undefined) continue;
      rows.push({
        championId: champ.championId,
        tier: entry.tier,
        rank: entry.rank ?? 999,
        winRate: Math.round((entry.winRate ?? 0) * 1000) / 10,
        pickRate: Math.round((entry.pickRate ?? 0) * 1000) / 10,
        banRate: Math.round((entry.banRate ?? 0) * 1000) / 10,
      });
    }
    rows.sort((a, b) => a.tier - b.tier || a.rank - b.rank);
    return rows.slice(0, 40);
  });

  async function loadTiers() {
    if (tiersLoading) return;
    tiersLoading = true;
    tiersError = "";
    try {
      tiers = await invoke<any>("league_champion_tiers", {
        region: (region ?? "br").toLowerCase(),
      });
    } catch (e: any) {
      tiers = null;
      tiersError = typeof e === "string" ? e : (e?.message ?? String(e));
    } finally {
      tiersLoading = false;
    }
  }

  onMount(() => {
    loadPerkMeta();
  });
</script>

<section class="card">
  <div class="card-head">
    <h3>{$t("league.runes_title")}</h3>
    {#if champSelectChampionId > 0}
      <span class="phase-tag">{championById.get(champSelectChampionId)?.name ?? champSelectChampionId}</span>
    {/if}
  </div>
  <p class="win-disclaimer">{$t("league.runes_desc")}</p>
  <div class="action-row">
    <div class="action-col">
      <span class="action-label">{$t("league.runes_auto")}</span>
      <span class="action-hint">{$t("league.runes_auto_desc")}</span>
    </div>
    <button
      class="toggle"
      class:on={settings?.league?.auto_runes}
      onclick={toggleAutoRunes}
      role="switch"
      aria-checked={settings?.league?.auto_runes ?? false}
      aria-label={$t("league.runes_auto") as string}
    >
      <span class="toggle-knob"></span>
    </button>
  </div>
  {#if runeError}
    <p class="action-error" role="alert">{runeError}</p>
  {/if}
  {#if runePages.length > 0}
    <div class="rune-list">
      {#each runePages as page, i (page.recommendationId ?? i)}
        <div class="rune-card" class:applied={appliedRuneIndex === i}>
          <div class="rune-head">
            <span class="rune-keystone">{page.keystoneName ?? page.keystoneId}</span>
            {#if page.isDefault}
              <span class="scout-tag">{$t("league.runes_default")}</span>
            {/if}
          </div>
          <div class="rune-perks">
            {#each page.selectedPerkIds as perkId (perkId)}
              <img
                class="perk-icon"
                src={`${CDRAGON}/perk-images/styles/${perkId}.png`}
                alt=""
                title={perkName(perkId)}
                loading="lazy"
                onerror={(e) => { (e.currentTarget as HTMLImageElement).style.visibility = "hidden"; }}
              />
            {/each}
          </div>
          <div class="rune-foot">
            <span class="dim">{$t("league.runes_spells")}: {(page.summonerSpellIds ?? []).map(spellName).join(" + ")}</span>
            <button class="button" onclick={() => applyRunePage(i)} disabled={runeApplying}>
              {$t("league.runes_apply")}
            </button>
          </div>
        </div>
      {/each}
    </div>
  {:else}
    <p class="empty-hint">{$t("league.runes_empty")}</p>
  {/if}
</section>

<section class="card">
  <div class="card-head">
    <h3>{$t("league.build_title")}</h3>
    <button
      class="button"
      onclick={() => loadBuild(champSelectChampionId || (buildChampionId ?? 0))}
      disabled={buildLoading || (!champSelectChampionId && !buildChampionId)}
    >{$t("league.refresh")}</button>
  </div>
  <p class="win-disclaimer">{$t("league.build_desc")}</p>
  <div class="build-picker">
    <select class="select-role" bind:value={buildChampionId} aria-label={$t("league.build_champion") as string}>
      <option value={0}>{$t("league.build_champion")}</option>
      {#each champions as ch (ch.id)}
        <option value={ch.id}>{ch.name}</option>
      {/each}
    </select>
  </div>
  {#if buildInfo && buildInfo.gamesSeen > 0}
    <p class="win-note">
      {buildInfo.gamesSeen} {$t("league.build_samples")} · {buildInfo.winrate}% {$t("league.stat_winrate")}
    </p>
    <div class="build-items">
      {#each buildInfo.items as it (it.itemId)}
        <span class="build-item">
          <img class="item-icon" src={`${CDRAGON}/../../game/assets/items/icons2d/${it.itemId}.png`} alt="" loading="lazy" onerror={(e) => { (e.currentTarget as HTMLImageElement).style.visibility = "hidden"; }} />
          <span class="dim">{it.pickRate}%</span>
        </span>
      {/each}
    </div>
    {#if buildInfo.spells?.length}
      <p class="win-note">{$t("league.runes_spells")}: {buildInfo.spells.map((s: any) => s.spellIds.map(spellName).join(" + ")).join(" / ")}</p>
    {/if}
  {:else if buildInfo}
    <p class="empty-hint">{$t("league.build_empty")}</p>
  {/if}
  <div class="divider"></div>
  <div class="card-head">
    <h4 class="section-title">
      {$t("league.meta_reference")}
      {#if buildFeature && needsBadge(buildFeature)}
        <span class="feature-badge">{$t(`league.badge_${buildFeature.state}`)}</span>
      {/if}
    </h4>
    <h4 class="section-title">{$t("league.meta_reference")}</h4>
    <button
      class="button"
      onclick={() => loadMeta(champSelectChampionId || (buildChampionId ?? 0))}
      disabled={metaLoading || (!champSelectChampionId && !buildChampionId)}
    >{$t("league.refresh")}</button>
  </div>
  {#if metaError}
    <p class="action-error" role="alert">{metaError}</p>
  {:else if metaLoading}
    <p class="empty-hint">…</p>
  {:else if metaInfo}
    {#if metaInfo.skillPriority?.length}
      <p class="win-note">
        {$t("league.skill_priority")}: <strong>{metaInfo.skillPriority.join(" › ")}</strong>
      </p>
      <div class="skill-order">
        {#each metaInfo.skillOrder as skill, i (`${i}-${skill}`)}
          <span class="skill-step" class:ult={skill === "R"}>{skill}</span>
        {/each}
      </div>
    {/if}
    {#each [["starterItems", "items_starter"], ["coreItems", "items_core"], ["boots", "items_boots"], ["lastItems", "items_last"]] as [key, label] (key)}
      {#if metaInfo[key]?.ids?.length}
        <div class="build-phase">
          <span class="list-label">{$t(`league.${label}`)}</span>
          <div class="build-items">
            {#each metaInfo[key].ids as id (id)}
              <span class="build-item">
                <img class="item-icon" src={itemIcon(id)} alt="" loading="lazy" onerror={(e) => { (e.currentTarget as HTMLImageElement).style.visibility = "hidden"; }} />
              </span>
            {/each}
            {#if metaInfo[key].winrate !== null && metaInfo[key].winrate !== undefined}
              <span class="dim">{metaInfo[key].winrate}%</span>
            {/if}
          </div>
        </div>
      {/if}
    {/each}
    {#if metaInfo.counters?.length}
      <div class="build-phase">
        <span class="list-label">{$t("league.counters")}</span>
        <div class="build-items">
          {#each metaInfo.counters.slice(0, 6) as c (c.championId)}
            <span class="build-item">
              <img class="champ-icon tiny" src={`${CDRAGON}/champion-icons/${c.championId}.png`} alt="" title={championById.get(c.championId)?.name ?? ""} loading="lazy" />
              <span class="dim">{c.winrate}%</span>
            </span>
          {/each}
        </div>
      </div>
    {/if}
    <p class="win-disclaimer">{$t("league.meta_source")}</p>
  {/if}
</section>

<section class="card">
  <div class="card-head">
    <h3>{$t("league.tiers_title")}</h3>
    <div class="tier-controls">
      <select class="select-role" bind:value={tierPosition} aria-label={$t("league.tiers_position") as string}>
        {#each TIER_POSITIONS as pos (pos)}
          <option value={pos}>{$t(`league.role_${pos.toLowerCase()}`)}</option>
        {/each}
      </select>
      <button class="button" onclick={loadTiers} disabled={tiersLoading}>{$t("league.refresh")}</button>
    </div>
  </div>
  <p class="win-disclaimer">{$t("league.tiers_desc")}</p>
  {#if tiersError}
    <p class="action-error" role="alert">{tiersError}</p>
  {:else if tierRows.length > 0}
    <div class="champ-table">
      {#each tierRows as row (row.championId)}
        <div class="champ-row">
          <span class={`tier-badge tier-${row.tier}`}>{tierLabel(row.tier)}</span>
          <img class="champ-icon tiny" src={`${CDRAGON}/champion-icons/${row.championId}.png`} alt="" loading="lazy" />
          <span class="champ-row-name">{championById.get(row.championId)?.name ?? row.championId}</span>
          <span class="champ-row-wr" class:good={row.winRate >= 52} class:bad={row.winRate <= 48}>{row.winRate}%</span>
          <span class="champ-row-games dim">{$t("league.tiers_pick")} {row.pickRate}%</span>
          <span class="champ-row-kda dim">{$t("league.tiers_ban")} {row.banRate}%</span>
        </div>
      {/each}
    </div>
  {:else}
    <p class="empty-hint">{tiersLoading ? $t("league.searching_player") : $t("league.tiers_empty")}</p>
  {/if}
</section>
