<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { CDRAGON, assetUrl, type Champion } from "./shared";

  let {
    summoner,
    champions,
    active,
  }: {
    summoner: any;
    champions: Champion[];
    active?: boolean;
  } = $props();

  type ProfileState = {
    availability: string | null;
    statusMessage: string | null;
    chatIcon: number | null;
    rank: { queue: string | null; tier: string | null; division: string | null };
    challengeCrystal: { level: string | null; points: number | null };
    tokens: number[];
    title: string;
    bannerAccent: string;
    backgroundSkinId: number | null;
    wallet: { rp: number | null; blueEssence: number | null };
    friendsCount: number;
  };

  const QUEUES = ["RANKED_SOLO_5x5", "RANKED_FLEX_SR", "RANKED_TFT"] as const;
  const TIERS = ["UNRANKED", "IRON", "BRONZE", "SILVER", "GOLD", "PLATINUM", "EMERALD", "DIAMOND", "MASTER", "GRANDMASTER", "CHALLENGER"] as const;
  const DIVISIONS = ["I", "II", "III", "IV"] as const;
  const AVAILABILITIES = ["chat", "away", "dnd", "offline", "mobile"] as const;
  const STATUS_MAX = 255;

  let profile = $state<ProfileState | null>(null);
  let loading = $state(false);
  let loaded = false;
  let error = $state("");
  let saved = $state("");

  function errText(e: unknown): string {
    return typeof e === "string" ? e : ((e as any)?.message ?? String(e));
  }

  async function run(action: () => Promise<unknown>, label: string) {
    error = "";
    saved = "";
    try {
      await action();
      saved = label;
      setTimeout(() => { if (saved === label) saved = ""; }, 2500);
      await load(true);
    } catch (e) {
      error = errText(e);
    }
  }

  async function load(silent = false) {
    if (!silent) loading = true;
    try {
      profile = await invoke<ProfileState>("league_profile_state");
      statusMessage = profile.statusMessage ?? "";
      rankQueue = profile.rank.queue ?? "RANKED_SOLO_5x5";
      rankTier = profile.rank.tier && profile.rank.tier !== "" ? profile.rank.tier : "UNRANKED";
      rankDivision = DIVISIONS.includes(profile.rank.division as any) ? (profile.rank.division as string) : "I";
      crystalLevel = profile.challengeCrystal.level && profile.challengeCrystal.level !== "" ? profile.challengeCrystal.level : "UNRANKED";
      crystalPoints = profile.challengeCrystal.points ?? 0;
      slots = [profile.tokens[0] ?? 0, profile.tokens[1] ?? 0, profile.tokens[2] ?? 0];
      titleId = profile.title ?? "";
      bannerId = profile.bannerAccent ?? "1";
      loaded = true;
    } catch (e) {
      if (!silent) error = errText(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (active !== false && summoner && !loaded) load();
  });

  // Presence
  let statusMessage = $state("");

  // Rank shown in chat
  let rankQueue = $state<string>("RANKED_SOLO_5x5");
  let rankTier = $state<string>("UNRANKED");
  let rankDivision = $state<string>("I");
  let apex = $derived(["MASTER", "GRANDMASTER", "CHALLENGER", "UNRANKED"].includes(rankTier));

  // Challenge crystal
  let crystalLevel = $state<string>("UNRANKED");
  let crystalPoints = $state<number>(0);

  // Tokens, title, banner
  type Token = { id: number; name: string; level: string; iconPath: string; description: string };
  let tokens = $state<Token[]>([]);
  let titles = $state<{ id: string; name: string }[]>([]);
  let banners = $state<{ id: string; name: string }[]>([]);
  let challengesLoaded = $state(false);
  let slots = $state<number[]>([0, 0, 0]);
  let titleId = $state("");
  let bannerId = $state("1");
  let tokenSearch = $state("");
  let tokensByName = $derived(
    tokens.filter((tk) => !tokenSearch.trim() || tk.name.toLowerCase().includes(tokenSearch.trim().toLowerCase())),
  );
  let tokenById = $derived(new Map(tokens.map((tk) => [tk.id, tk])));

  async function loadChallenges() {
    if (challengesLoaded) return;
    challengesLoaded = true;
    try {
      const res = await invoke<any>("league_challenges");
      tokens = res?.tokens ?? [];
      titles = res?.titles ?? [];
    } catch (e) {
      error = errText(e);
    }
    try {
      const res = await fetch(`${CDRAGON}/regalia.json`);
      const data = (await res.json()) as any[];
      const list = data
        .filter((r) => r?.regaliaType === "kBanner" && r?.isSelectable && r?.id !== undefined)
        .map((r) => ({ id: String(r.id), name: r.localizedName || `#${r.id}` }));
      if (!list.some((b) => b.id === "1")) list.unshift({ id: "1", name: $t("league.banner_none") as string });
      banners = list.sort((a, b) => (a.id === "1" ? -1 : b.id === "1" ? 1 : a.name.localeCompare(b.name)));
    } catch {
      banners = [{ id: "1", name: $t("league.banner_none") as string }];
    }
  }

  function setSlot(index: number, id: number) {
    const next = [...slots];
    next[index] = id;
    slots = next;
  }

  function applyPrefs() {
    run(
      () =>
        invoke("league_set_challenge_prefs", {
          challengeIds: slots.filter((id) => id > 0),
          title: titleId,
          bannerAccent: bannerId,
        }),
      $t("league.tokens_saved") as string,
    );
  }

  // Icons
  type Icon = { id: number; title: string };
  let icons = $state<Icon[]>([]);
  let iconsLoaded = false;
  let iconSearch = $state("");
  let iconResults = $derived.by(() => {
    const q = iconSearch.trim().toLowerCase();
    if (!q) return icons.slice(0, 24);
    const asNumber = Number(q);
    return icons
      .filter((i) => i.title.toLowerCase().includes(q) || (Number.isFinite(asNumber) && i.id === asNumber))
      .slice(0, 40);
  });

  async function loadIcons() {
    if (iconsLoaded) return;
    iconsLoaded = true;
    try {
      const res = await fetch(`${CDRAGON}/summoner-icons.json`);
      const data = (await res.json()) as any[];
      icons = data
        .filter((i) => typeof i?.id === "number")
        .map((i) => ({ id: i.id, title: i.title || i.descriptions?.[0]?.description || `#${i.id}` }))
        .sort((a, b) => b.id - a.id);
    } catch {
      icons = [];
    }
  }

  // Background
  let bgChampion = $state<number>(0);
  let ownedSkins = $state<{ id: number; name: string }[]>([]);

  async function loadOwnedSkins(championId: number) {
    ownedSkins = [];
    if (championId <= 0) return;
    try {
      const res = await invoke<any>("league_owned_skins", { championId });
      ownedSkins = res?.skins ?? [];
    } catch {
      ownedSkins = [];
    }
  }

  // Friends
  type Friend = { id: string; gameName: string; gameTag: string; name: string; availability: string; icon: number; note: string; groupName: string };
  let friends = $state<Friend[]>([]);
  let friendsLoading = $state(false);
  let friendSearch = $state("");
  let selected = $state<Set<string>>(new Set());
  let confirmingRemove = $state(false);
  let removing = $state(false);
  let visibleFriends = $derived.by(() => {
    const q = friendSearch.trim().toLowerCase();
    return friends.filter((f) => !q || `${f.gameName}#${f.gameTag}`.toLowerCase().includes(q) || f.name.toLowerCase().includes(q));
  });

  async function loadFriends() {
    friendsLoading = true;
    error = "";
    try {
      const res = await invoke<any>("league_friends");
      friends = (res?.friends ?? []).sort((a: Friend, b: Friend) => a.gameName.localeCompare(b.gameName));
      selected = new Set();
    } catch (e) {
      error = errText(e);
    } finally {
      friendsLoading = false;
    }
  }

  function toggleFriend(id: string) {
    const next = new Set(selected);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    selected = next;
  }

  function selectVisible() {
    selected = new Set(visibleFriends.map((f) => f.id));
  }

  async function removeSelected() {
    if (removing || selected.size === 0) return;
    removing = true;
    error = "";
    try {
      const res = await invoke<any>("league_remove_friends", { ids: [...selected] });
      saved = `${$t("league.friends_removed")}: ${res?.removed ?? 0}`;
      confirmingRemove = false;
      await loadFriends();
      await load(true);
    } catch (e) {
      error = errText(e);
    } finally {
      removing = false;
    }
  }
</script>

{#if active !== false}
  <section class="card">
    <div class="card-head">
      <h3>{$t("league.profile_title")}</h3>
      <span class="feature-badge">{$t("league.badge_beta")}</span>
    </div>
    <p class="win-disclaimer">{$t("league.profile_desc")}</p>
    {#if error}
      <p class="action-error" role="alert">{error}</p>
    {/if}
    {#if saved}
      <p class="profile-saved">{saved}</p>
    {/if}
    {#if loading && !profile}
      <p class="dim">…</p>
    {/if}
    {#if profile}
      <div class="wallet-row">
        <span class="queue-chip">{$t("league.wallet_rp")} <strong>{profile.wallet.rp ?? "—"}</strong></span>
        <span class="queue-chip">{$t("league.wallet_be")} <strong>{profile.wallet.blueEssence ?? "—"}</strong></span>
        <span class="queue-chip">{$t("league.friends_count")} <strong>{profile.friendsCount}</strong></span>
      </div>
    {/if}
  </section>

  <!-- Presence -->
  <section class="card">
    <div class="card-head"><h3>{$t("league.profile_section_presence")}</h3></div>
    <div class="profile-tool-row">
      <span class="list-label">{$t("league.profile_status")}</span>
      <div class="seg-group" role="radiogroup" aria-label={$t("league.profile_status") as string}>
        {#each AVAILABILITIES as value (value)}
          <button
            class="seg"
            class:on={profile?.availability === value}
            role="radio"
            aria-checked={profile?.availability === value}
            onclick={() => run(() => invoke("league_set_status", { availability: value }), $t("league.profile_status_saved") as string)}
          >{$t(`league.status_${value}`)}</button>
        {/each}
      </div>
    </div>
    <div class="profile-tool-row stacked">
      <span class="list-label">{$t("league.profile_message")} <span class="list-hint">{statusMessage.length}/{STATUS_MAX}</span></span>
      <textarea class="input-text status-area" maxlength={STATUS_MAX} rows="3" bind:value={statusMessage} placeholder={$t("league.profile_message") as string}></textarea>
      <span class="action-hint">{$t("league.profile_message_hint")}</span>
      <div class="inline-actions">
        <button class="button" onclick={() => run(() => invoke("league_set_status", { message: statusMessage }), $t("league.profile_message_saved") as string)}>{$t("league.apply")}</button>
        <button class="button subtle" onclick={() => { statusMessage = ""; run(() => invoke("league_set_status", { message: "" }), $t("league.profile_message_saved") as string); }}>{$t("league.tokens_clear")}</button>
      </div>
    </div>
  </section>

  <!-- Icons -->
  <section class="card">
    <div class="card-head"><h3>{$t("league.profile_section_icon")}</h3></div>
    <span class="action-hint">{$t("league.profile_icon_hint")}</span>
    <div class="profile-tool-row">
      <input
        class="input-text"
        placeholder={$t("league.profile_icon_search") as string}
        bind:value={iconSearch}
        onfocus={loadIcons}
        oninput={loadIcons}
      />
    </div>
    {#if icons.length > 0}
      <div class="icon-grid">
        {#each iconResults as icon (icon.id)}
          <div class="icon-cell" class:current={summoner?.profileIconId === icon.id}>
            <img src={`${CDRAGON}/profile-icons/${icon.id}.jpg`} alt="" loading="lazy" />
            <span class="icon-title" title={icon.title}>{icon.title}</span>
            <div class="icon-actions">
              <button class="button subtle tiny" onclick={() => run(() => invoke("league_set_icon", { iconId: icon.id }), $t("league.profile_icon_saved") as string)}>{$t("league.profile_icon")}</button>
              <button class="button subtle tiny" onclick={() => run(() => invoke("league_set_chat_icon", { iconId: icon.id }), $t("league.profile_icon_chat_saved") as string)}>{$t("league.profile_icon_chat")}</button>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </section>

  <!-- Rank in chat -->
  <section class="card">
    <div class="card-head"><h3>{$t("league.profile_section_rank")}</h3></div>
    <span class="action-hint">{$t("league.rank_hint")}</span>
    <div class="profile-tool-row">
      <select class="select-role" bind:value={rankQueue} aria-label={$t("league.rank_queue") as string}>
        {#each QUEUES as q (q)}
          <option value={q}>{q === "RANKED_SOLO_5x5" ? $t("league.ranked_solo") : q === "RANKED_FLEX_SR" ? $t("league.ranked_flex") : "TFT"}</option>
        {/each}
      </select>
      <select class="select-role" bind:value={rankTier} aria-label={$t("league.rank_tier") as string}>
        {#each TIERS as tier (tier)}
          <option value={tier}>{tier === "UNRANKED" ? $t("league.unranked") : tier.charAt(0) + tier.slice(1).toLowerCase()}</option>
        {/each}
      </select>
      <select class="select-role" bind:value={rankDivision} disabled={apex} aria-label={$t("league.rank_division") as string}>
        {#each DIVISIONS as d (d)}
          <option value={d}>{d}</option>
        {/each}
      </select>
      <button class="button" onclick={() => run(() => invoke("league_set_chat_rank", { queue: rankQueue, tier: rankTier, division: apex ? "NA" : rankDivision }), $t("league.rank_saved") as string)}>{$t("league.apply")}</button>
      <button class="button subtle" onclick={() => run(() => invoke("league_reset_chat_rank"), $t("league.rank_saved") as string)}>{$t("league.rank_reset")}</button>
    </div>
    <div class="profile-tool-row">
      <span class="list-label">{$t("league.crystal_title")}</span>
      <select class="select-role" bind:value={crystalLevel} aria-label={$t("league.crystal_level") as string}>
        {#each TIERS as tier (tier)}
          <option value={tier}>{tier === "UNRANKED" ? $t("league.unranked") : tier.charAt(0) + tier.slice(1).toLowerCase()}</option>
        {/each}
      </select>
      <input class="input-text tiny-input" type="number" min="0" max="1000000" bind:value={crystalPoints} aria-label={$t("league.crystal_points") as string} />
      <button class="button" onclick={() => run(() => invoke("league_set_challenge_crystal", { level: crystalLevel, points: Number(crystalPoints) || 0 }), $t("league.crystal_saved") as string)}>{$t("league.apply")}</button>
    </div>
  </section>

  <!-- Tokens, title, banner -->
  <section class="card">
    <div class="card-head">
      <h3>{$t("league.tokens_title")}</h3>
      {#if !challengesLoaded || tokens.length === 0}
        <button class="button subtle" onclick={loadChallenges}>{$t("league.friends_load")}</button>
      {/if}
    </div>
    <span class="action-hint">{$t("league.tokens_desc")}</span>
    {#if tokens.length > 0 || titles.length > 0}
      <div class="token-slots">
        {#each [0, 1, 2] as index (index)}
          {@const current = tokenById.get(slots[index])}
          <div class="token-slot">
            {#if current?.iconPath}
              <img class="token-icon" src={assetUrl(current.iconPath)} alt="" loading="lazy" />
            {:else}
              <div class="token-icon empty" aria-hidden="true"></div>
            {/if}
            <span class="dim">{$t("league.tokens_slot")} {index + 1}</span>
            <select class="select-role" value={slots[index]} onchange={(e) => setSlot(index, Number(e.currentTarget.value))} aria-label={`${$t("league.tokens_slot")} ${index + 1}`}>
              <option value={0}>{$t("league.tokens_none")}</option>
              {#each tokensByName as tk (tk.id)}
                <option value={tk.id}>{tk.name} · {tk.level.charAt(0) + tk.level.slice(1).toLowerCase()}</option>
              {/each}
            </select>
          </div>
        {/each}
      </div>
      <div class="profile-tool-row">
        <input class="input-text" placeholder={$t("league.tokens_search") as string} bind:value={tokenSearch} />
      </div>
      <div class="profile-tool-row">
        <span class="list-label">{$t("league.title_label")}</span>
        <select class="select-role" bind:value={titleId} aria-label={$t("league.title_label") as string}>
          <option value="">{$t("league.title_none")}</option>
          {#each titles as title (title.id)}
            <option value={title.id}>{title.name}</option>
          {/each}
        </select>
      </div>
      <div class="profile-tool-row">
        <span class="list-label">{$t("league.banner_label")}</span>
        <select class="select-role" bind:value={bannerId} aria-label={$t("league.banner_label") as string}>
          {#each banners as banner (banner.id)}
            <option value={banner.id}>{banner.name}</option>
          {/each}
        </select>
      </div>
      <div class="inline-actions">
        <button class="button" onclick={applyPrefs}>{$t("league.apply")}</button>
        <button class="button subtle" onclick={() => { slots = [0, 0, 0]; applyPrefs(); }}>{$t("league.tokens_clear")}</button>
      </div>
    {:else if challengesLoaded}
      <p class="empty-hint">{$t("league.tokens_empty")}</p>
    {/if}
  </section>

  <!-- Background -->
  <section class="card">
    <div class="card-head"><h3>{$t("league.profile_background")}</h3></div>
    <div class="profile-tool-row">
      <select class="select-role" bind:value={bgChampion} onchange={() => loadOwnedSkins(bgChampion)} aria-label={$t("league.profile_background") as string}>
        <option value={0}>{$t("league.build_champion")}</option>
        {#each champions as ch (ch.id)}
          <option value={ch.id}>{ch.name}</option>
        {/each}
      </select>
      {#if ownedSkins.length > 0}
        <div class="skin-options">
          {#each ownedSkins as skin (skin.id)}
            <button class="button subtle" class:on={profile?.backgroundSkinId === skin.id} onclick={() => run(() => invoke("league_set_profile_background", { skinId: skin.id }), $t("league.profile_background_saved") as string)}>{skin.name}</button>
          {/each}
        </div>
      {:else if bgChampion > 0}
        <span class="dim">{$t("league.profile_no_skins")}</span>
      {/if}
    </div>
  </section>

  <!-- Friends -->
  <section class="card">
    <div class="card-head">
      <h3>{$t("league.friends_title")}</h3>
      <button class="button subtle" onclick={loadFriends} disabled={friendsLoading}>{$t("league.friends_load")}</button>
    </div>
    <span class="action-hint">{$t("league.friends_desc")}</span>
    {#if friends.length > 0}
      <div class="profile-tool-row">
        <input class="input-text" placeholder={$t("league.search_placeholder") as string} bind:value={friendSearch} />
        <button class="button subtle" onclick={selectVisible}>{$t("league.friends_select_all")}</button>
        <button class="button subtle" onclick={() => (selected = new Set())}>{$t("league.friends_clear_selection")}</button>
      </div>
      <div class="friend-list">
        {#each visibleFriends as friend (friend.id)}
          <label class="friend-row" class:picked={selected.has(friend.id)}>
            <input type="checkbox" checked={selected.has(friend.id)} onchange={() => toggleFriend(friend.id)} />
            <img class="friend-icon" src={`${CDRAGON}/profile-icons/${friend.icon}.jpg`} alt="" loading="lazy" />
            <span class="friend-name">{friend.gameName}<span class="tag">#{friend.gameTag}</span></span>
            <span class="friend-meta dim">{friend.availability}{friend.groupName ? ` · ${friend.groupName}` : ""}</span>
          </label>
        {/each}
      </div>
      <div class="inline-actions">
        {#if confirmingRemove}
          <span class="dodge-warning">{$t("league.friends_remove_confirm")}</span>
          <button class="button" onclick={() => (confirmingRemove = false)}>{$t("league.dodge_cancel")}</button>
          <button class="button danger" onclick={removeSelected} disabled={removing}>{$t("league.friends_remove_go")} ({selected.size})</button>
        {:else}
          <button class="button subtle-danger" disabled={selected.size === 0} onclick={() => (confirmingRemove = true)}>{$t("league.friends_remove")} ({selected.size})</button>
        {/if}
      </div>
    {:else if friendsLoading}
      <p class="dim">…</p>
    {/if}
  </section>
{/if}

<style>
  .wallet-row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .status-area {
    width: 100%;
    resize: vertical;
    font: inherit;
  }
  .profile-tool-row.stacked {
    flex-direction: column;
    align-items: stretch;
  }
  .inline-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }
  .icon-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(112px, 1fr));
    gap: 8px;
  }
  .icon-cell {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 6px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--surface) 70%, transparent);
  }
  .icon-cell.current {
    outline: 2px solid currentColor;
  }
  .icon-cell img {
    width: 56px;
    height: 56px;
    border-radius: 50%;
  }
  .icon-title {
    font-size: 11px;
    text-align: center;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .icon-actions {
    display: flex;
    gap: 4px;
  }
  .token-slots {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
  }
  .token-slot {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
  }
  .token-slot :global(.select-role) {
    width: 100%;
    max-width: 100%;
  }
  .token-icon {
    width: 48px;
    height: 48px;
  }
  .token-icon.empty {
    border-radius: 50%;
    border: 1px dashed currentColor;
    opacity: 0.4;
  }
  .friend-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 320px;
    overflow: auto;
  }
  .friend-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 6px;
    border-radius: 8px;
    cursor: pointer;
  }
  .friend-row.picked {
    background: color-mix(in srgb, var(--surface) 60%, transparent);
  }
  .friend-icon {
    width: 22px;
    height: 22px;
    border-radius: 50%;
  }
  .friend-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .friend-meta {
    font-size: 11px;
  }
  @media (max-width: 520px) {
    .token-slots {
      grid-template-columns: 1fr;
    }
  }
</style>
