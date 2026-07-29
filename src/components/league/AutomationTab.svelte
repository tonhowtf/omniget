<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
  import { CDRAGON, type Champion } from "./shared";

  let {
    champions,
    championById,
  }: {
    champions: Champion[];
    championById: Map<number, Champion>;
  } = $props();

  let settings = $derived(getSettings());
  let autoAccept = $state(false);
  let pickSearch = $state("");
  let banSearch = $state("");

  let acceptDelay = $derived(settings?.league?.auto_accept_delay ?? 0);

  function changeAcceptDelay(event: Event) {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    updateSettings({ league: { auto_accept_delay: value } });
  }

  async function toggleAutoAccept() {
    const next = !autoAccept;
    autoAccept = next;
    updateSettings({ league: { auto_accept: next } });
    try {
      await invoke("league_auto_accept_set", { enabled: next });
    } catch {
      autoAccept = !next;
    }
  }

  function toggleLeagueFlag(field: "auto_pick" | "auto_ban" | "auto_lock" | "auto_honor" | "auto_play_again" | "auto_reconnect" | "notify_ready_check") {
    const current = (settings?.league as any)?.[field] ?? false;
    updateSettings({ league: { [field]: !current } });
  }

  function listFor(kind: "pick" | "ban"): number[] {
    const l = settings?.league as any;
    return (kind === "pick" ? l?.pick_champions : l?.ban_champions) ?? [];
  }

  function saveList(kind: "pick" | "ban", ids: number[]) {
    updateSettings({ league: kind === "pick" ? { pick_champions: ids } : { ban_champions: ids } });
  }

  function addToList(kind: "pick" | "ban", id: number) {
    const ids = listFor(kind);
    if (ids.includes(id)) return;
    saveList(kind, [...ids, id]);
    if (kind === "pick") pickSearch = "";
    else banSearch = "";
  }

  function removeFromList(kind: "pick" | "ban", id: number) {
    saveList(kind, listFor(kind).filter((x) => x !== id));
  }

  function searchResults(query: string, kind: "pick" | "ban"): Champion[] {
    const q = query.trim().toLowerCase();
    if (!q) return [];
    const existing = new Set(listFor(kind));
    return champions
      .filter((c) => !existing.has(c.id))
      .filter((c) => c.name.toLowerCase().includes(q) || c.alias.toLowerCase().includes(q))
      .slice(0, 8);
  }

  onMount(() => {
    invoke<boolean>("league_auto_accept_get")
      .then((v) => { autoAccept = v; })
      .catch(() => {});
  });
</script>

<section class="card">
  <div class="card-head">
    <h3>{$t("league.automation_title")}</h3>
  </div>
  <div class="action-row">
    <div class="action-col">
      <span class="action-label">{$t("league.auto_accept")}</span>
      <span class="action-hint">{$t("league.auto_accept_desc")}</span>
    </div>
    <button class="toggle" class:on={autoAccept} onclick={toggleAutoAccept} role="switch" aria-checked={autoAccept} aria-label={$t("league.auto_accept") as string}>
      <span class="toggle-knob"></span>
    </button>
  </div>
  {#if autoAccept}
    <div class="delay-block">
      <span class="list-label">
        {$t("league.accept_delay")}
        <span class="list-hint">
          {acceptDelay === 0 ? $t("league.accept_delay_instant") : `${acceptDelay}s`}
        </span>
      </span>
      <div class="slider-row">
        <span class="slider-edge">0s</span>
        <input
          type="range"
          min="0"
          max="11"
          step="1"
          value={acceptDelay}
          oninput={changeAcceptDelay}
          aria-label={$t("league.accept_delay") as string}
        />
        <span class="slider-edge">11s</span>
      </div>
      <span class="action-hint">{$t("league.accept_delay_desc")}</span>
    </div>
  {/if}
  <div class="divider"></div>
  <div class="action-row">
    <div class="action-col">
      <span class="action-label">{$t("league.notify_ready")}</span>
      <span class="action-hint">{$t("league.notify_ready_desc")}</span>
    </div>
    <button class="toggle" class:on={settings?.league?.notify_ready_check ?? true} onclick={() => toggleLeagueFlag("notify_ready_check")} role="switch" aria-checked={settings?.league?.notify_ready_check ?? true} aria-label={$t("league.notify_ready") as string}>
      <span class="toggle-knob"></span>
    </button>
  </div>
  <div class="divider"></div>
  <div class="action-row">
    <div class="action-col">
      <span class="action-label">{$t("league.auto_pick")}</span>
      <span class="action-hint">{$t("league.auto_pick_desc")}</span>
    </div>
    <button class="toggle" class:on={settings?.league?.auto_pick} onclick={() => toggleLeagueFlag("auto_pick")} role="switch" aria-checked={settings?.league?.auto_pick ?? false} aria-label={$t("league.auto_pick") as string}>
      <span class="toggle-knob"></span>
    </button>
  </div>
  {#if settings?.league?.auto_pick}
    <div class="champ-list-block">
      <span class="list-label">{$t("league.pick_list")} <span class="list-hint">({$t("league.list_hint")})</span></span>
      <div class="champ-chips">
        {#each listFor("pick") as id (id)}
          <span class="champ-chip">
            <img class="champ-icon tiny" src={`${CDRAGON}/champion-icons/${id}.png`} alt="" loading="lazy" />
            {championById.get(id)?.name ?? id}
            <button class="chip-remove" onclick={() => removeFromList("pick", id)} aria-label={`${$t("league.remove")} ${championById.get(id)?.name ?? id}`}>×</button>
          </span>
        {/each}
      </div>
      <div class="champ-search">
        <input type="text" class="input-text" placeholder={$t("league.search_champion") as string} bind:value={pickSearch} />
        {#if searchResults(pickSearch, "pick").length > 0}
          <div class="search-results">
            {#each searchResults(pickSearch, "pick") as c (c.id)}
              <button class="search-result" onclick={() => addToList("pick", c.id)}>
                <img class="champ-icon tiny" src={`${CDRAGON}/champion-icons/${c.id}.png`} alt="" loading="lazy" />
                {c.name}
              </button>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  {/if}
  <div class="divider"></div>
  <div class="action-row">
    <div class="action-col">
      <span class="action-label">{$t("league.auto_ban")}</span>
      <span class="action-hint">{$t("league.auto_ban_desc")}</span>
    </div>
    <button class="toggle" class:on={settings?.league?.auto_ban} onclick={() => toggleLeagueFlag("auto_ban")} role="switch" aria-checked={settings?.league?.auto_ban ?? false} aria-label={$t("league.auto_ban") as string}>
      <span class="toggle-knob"></span>
    </button>
  </div>
  {#if settings?.league?.auto_ban}
    <div class="champ-list-block">
      <span class="list-label">{$t("league.ban_list")} <span class="list-hint">({$t("league.list_hint")})</span></span>
      <div class="champ-chips">
        {#each listFor("ban") as id (id)}
          <span class="champ-chip">
            <img class="champ-icon tiny" src={`${CDRAGON}/champion-icons/${id}.png`} alt="" loading="lazy" />
            {championById.get(id)?.name ?? id}
            <button class="chip-remove" onclick={() => removeFromList("ban", id)} aria-label={`${$t("league.remove")} ${championById.get(id)?.name ?? id}`}>×</button>
          </span>
        {/each}
      </div>
      <div class="champ-search">
        <input type="text" class="input-text" placeholder={$t("league.search_champion") as string} bind:value={banSearch} />
        {#if searchResults(banSearch, "ban").length > 0}
          <div class="search-results">
            {#each searchResults(banSearch, "ban") as c (c.id)}
              <button class="search-result" onclick={() => addToList("ban", c.id)}>
                <img class="champ-icon tiny" src={`${CDRAGON}/champion-icons/${c.id}.png`} alt="" loading="lazy" />
                {c.name}
              </button>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  {/if}
  {#if settings?.league?.auto_pick}
    <div class="divider"></div>
    <div class="action-row">
      <div class="action-col">
        <span class="action-label">{$t("league.auto_lock")}</span>
        <span class="action-hint">{$t("league.auto_lock_desc")}</span>
      </div>
      <button class="toggle" class:on={settings?.league?.auto_lock} onclick={() => toggleLeagueFlag("auto_lock")} role="switch" aria-checked={settings?.league?.auto_lock ?? false} aria-label={$t("league.auto_lock") as string}>
        <span class="toggle-knob"></span>
      </button>
    </div>
  {/if}
  <div class="divider"></div>
  <div class="action-row">
    <div class="action-col">
      <span class="action-label">{$t("league.auto_honor")}</span>
      <span class="action-hint">{$t("league.auto_honor_desc")}</span>
    </div>
    <button class="toggle" class:on={settings?.league?.auto_honor} onclick={() => toggleLeagueFlag("auto_honor")} role="switch" aria-checked={settings?.league?.auto_honor ?? false} aria-label={$t("league.auto_honor") as string}>
      <span class="toggle-knob"></span>
    </button>
  </div>
  <div class="divider"></div>
  <div class="action-row">
    <div class="action-col">
      <span class="action-label">{$t("league.auto_play_again")}</span>
      <span class="action-hint">{$t("league.auto_play_again_desc")}</span>
    </div>
    <button class="toggle" class:on={settings?.league?.auto_play_again} onclick={() => toggleLeagueFlag("auto_play_again")} role="switch" aria-checked={settings?.league?.auto_play_again ?? false} aria-label={$t("league.auto_play_again") as string}>
      <span class="toggle-knob"></span>
    </button>
  </div>
  <div class="divider"></div>
  <div class="action-row">
    <div class="action-col">
      <span class="action-label">{$t("league.auto_reconnect")}</span>
      <span class="action-hint">{$t("league.auto_reconnect_desc")}</span>
    </div>
    <button class="toggle" class:on={settings?.league?.auto_reconnect} onclick={() => toggleLeagueFlag("auto_reconnect")} role="switch" aria-checked={settings?.league?.auto_reconnect ?? false} aria-label={$t("league.auto_reconnect") as string}>
      <span class="toggle-knob"></span>
    </button>
  </div>
  <div class="divider"></div>
  <div class="action-row">
    <div class="action-col">
      <span class="action-label">{$t("league.auto_trade")}</span>
      <span class="action-hint">{$t("league.auto_trade_desc")}</span>
    </div>
    <div class="seg-group" role="radiogroup" aria-label={$t("league.auto_trade") as string}>
      {#each [["", "auto_trade_off"], ["accept", "auto_trade_accept"], ["decline", "auto_trade_decline"]] as [value, key] (value)}
        <button
          class="seg"
          class:on={(settings?.league?.auto_trade ?? "") === value}
          role="radio"
          aria-checked={(settings?.league?.auto_trade ?? "") === value}
          onclick={() => updateSettings({ league: { auto_trade: value } })}
        >{$t(`league.${key}`)}</button>
      {/each}
    </div>
  </div>
  <div class="divider"></div>
  <div class="action-row stacked">
    <div class="action-col">
      <span class="action-label">{$t("league.auto_message")}</span>
      <span class="action-hint">{$t("league.auto_message_desc")}</span>
    </div>
    <input
      class="input-text message-input"
      type="text"
      maxlength="120"
      placeholder={$t("league.auto_message_placeholder") as string}
      value={settings?.league?.auto_message ?? ""}
      onchange={(e) => updateSettings({ league: { auto_message: e.currentTarget.value } })}
    />
  </div>
</section>
