<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { t } from "$lib/i18n";

  type PlaylistEntry = {
    index: number;
    title: string;
    url: string;
  };

  type LinkKind = "channel" | "playlist" | "video" | "unknown";

  type ChannelTab = "all" | "videos" | "shorts" | "streams" | "playlists";

  let inputUrl = $state("");
  let currentUrl = $state("");
  let linkKind = $state<LinkKind>("unknown");
  let channelBaseUrl = $state("");

  const INITIAL_LIMIT = 200;

  let activeTab = $state<ChannelTab>("all");
  let entriesByTab = $state<Record<ChannelTab, PlaylistEntry[]>>({
    all: [],
    videos: [],
    shorts: [],
    streams: [],
    playlists: [],
  });
  let loadedTabs = $state<Set<ChannelTab>>(new Set());
  let fullyLoadedTabs = $state<Set<ChannelTab>>(new Set());
  let loadingTab = $state<ChannelTab | null>(null);
  let loadingFull = $state(false);
  let loadError = $state("");

  const urlCache = new Map<string, PlaylistEntry[]>();

  function extractVideoId(url: string): string | null {
    try {
      const u = new URL(url);
      const host = u.hostname.replace(/^www\./, "");
      if (host === "youtu.be") {
        const id = u.pathname.slice(1).split("/")[0];
        return id || null;
      }
      const v = u.searchParams.get("v");
      if (v) return v;
      const m = u.pathname.match(/^\/(?:shorts|live|embed)\/([^/?#]+)/);
      if (m) return m[1];
      return null;
    } catch {
      return null;
    }
  }

  function thumbnailFor(url: string): string | null {
    const id = extractVideoId(url);
    return id ? `https://i.ytimg.com/vi/${id}/mqdefault.jpg` : null;
  }

  let selectedByTab = $state<Record<ChannelTab, Set<string>>>({
    all: new Set(),
    videos: new Set(),
    shorts: new Set(),
    streams: new Set(),
    playlists: new Set(),
  });

  let batchActive = $state(false);
  let batchDone = $state(0);
  let batchTotal = $state(0);
  let batchCancelled = $state(false);

  let entries = $derived(entriesByTab[activeTab]);
  let selected = $derived(selectedByTab[activeTab]);
  let selectedCount = $derived(selected.size);
  let allSelected = $derived(entries.length > 0 && selectedCount === entries.length);

  const TABS: { key: ChannelTab; labelKey: string; segment: string }[] = [
    { key: "all", labelKey: "youtube.tab_all", segment: "" },
    { key: "videos", labelKey: "youtube.tab_videos", segment: "/videos" },
    { key: "shorts", labelKey: "youtube.tab_shorts", segment: "/shorts" },
    { key: "streams", labelKey: "youtube.tab_live", segment: "/streams" },
    { key: "playlists", labelKey: "youtube.tab_playlists", segment: "/playlists" },
  ];

  function detectLinkKind(url: string): { kind: LinkKind; base: string } {
    const trimmed = url.trim();
    if (!trimmed) return { kind: "unknown", base: "" };
    try {
      const parsed = new URL(trimmed);
      const host = parsed.hostname.replace(/^www\./, "");
      if (host !== "youtube.com" && host !== "m.youtube.com" && host !== "youtu.be") {
        return { kind: "unknown", base: "" };
      }
      const path = parsed.pathname;
      if (parsed.searchParams.get("list")) {
        return { kind: "playlist", base: trimmed };
      }
      if (host === "youtu.be" || path.startsWith("/watch") || path.startsWith("/shorts/") || path.startsWith("/live/")) {
        return { kind: "video", base: trimmed };
      }
      const channelMatch = path.match(/^\/(@[^/]+|channel\/[^/]+|c\/[^/]+|user\/[^/]+)(?:\/(videos|shorts|streams|live|playlists|community|featured)?)?/);
      if (channelMatch) {
        const base = `https://www.youtube.com/${channelMatch[1]}`;
        return { kind: "channel", base };
      }
      return { kind: "unknown", base: "" };
    } catch {
      return { kind: "unknown", base: "" };
    }
  }

  function resetState() {
    entriesByTab = { all: [], videos: [], shorts: [], streams: [], playlists: [] };
    loadedTabs = new Set();
    fullyLoadedTabs = new Set();
    selectedByTab = { all: new Set(), videos: new Set(), shorts: new Set(), streams: new Set(), playlists: new Set() };
    activeTab = "all";
    loadError = "";
  }

  async function submitUrl() {
    const trimmed = inputUrl.trim();
    if (!trimmed) return;
    const { kind, base } = detectLinkKind(trimmed);
    if (kind === "unknown") {
      showToast("error", $t("youtube.invalid_url"));
      return;
    }
    linkKind = kind;
    currentUrl = trimmed;
    channelBaseUrl = base;
    resetState();
    if (kind === "video") {
      const single: PlaylistEntry = { index: 1, title: trimmed, url: trimmed };
      entriesByTab = { ...entriesByTab, all: [single] };
      loadedTabs = new Set(["all"]);
      return;
    }
    await loadTab("all");
  }

  function urlForTab(tab: ChannelTab): string {
    if (linkKind !== "channel") return currentUrl;
    const seg = TABS.find((t) => t.key === tab)?.segment ?? "";
    return `${channelBaseUrl}${seg}`;
  }

  async function loadTab(tab: ChannelTab, fullFetch = false) {
    if (loadingTab !== null || loadingFull) return;
    if (loadedTabs.has(tab) && !fullFetch) return;
    if (fullyLoadedTabs.has(tab)) return;
    const url = urlForTab(tab);
    const cacheKey = fullFetch ? `${url}#full` : `${url}#${INITIAL_LIMIT}`;
    const cached = urlCache.get(cacheKey);
    if (cached) {
      entriesByTab = { ...entriesByTab, [tab]: cached };
      loadedTabs = new Set([...loadedTabs, tab]);
      if (fullFetch) fullyLoadedTabs = new Set([...fullyLoadedTabs, tab]);
      return;
    }
    if (fullFetch) loadingFull = true;
    else loadingTab = tab;
    loadError = "";
    try {
      const items = await invoke<PlaylistEntry[]>("playlist_entries", {
        url,
        limit: fullFetch ? null : INITIAL_LIMIT,
      });
      entriesByTab = { ...entriesByTab, [tab]: items };
      loadedTabs = new Set([...loadedTabs, tab]);
      urlCache.set(cacheKey, items);
      if (fullFetch) fullyLoadedTabs = new Set([...fullyLoadedTabs, tab]);
    } catch (e: any) {
      loadError = typeof e === "string" ? e : (e?.message ?? $t("youtube.load_error"));
    } finally {
      loadingTab = null;
      loadingFull = false;
    }
  }

  async function loadFullChannel() {
    await loadTab(activeTab, true);
  }

  async function switchTab(tab: ChannelTab) {
    activeTab = tab;
    if (!loadedTabs.has(tab)) await loadTab(tab);
  }

  function toggleSelection(url: string) {
    const next = new Set(selectedByTab[activeTab]);
    if (next.has(url)) next.delete(url);
    else next.add(url);
    selectedByTab = { ...selectedByTab, [activeTab]: next };
  }

  function selectAll() {
    const next = new Set(entries.map((e) => e.url));
    selectedByTab = { ...selectedByTab, [activeTab]: next };
  }

  function clearSelection() {
    selectedByTab = { ...selectedByTab, [activeTab]: new Set() };
  }

  function invertSelection() {
    const cur = selectedByTab[activeTab];
    const next = new Set<string>();
    for (const e of entries) if (!cur.has(e.url)) next.add(e.url);
    selectedByTab = { ...selectedByTab, [activeTab]: next };
  }

  async function resolveOutputDir(): Promise<string | null> {
    const appSettings = getSettings();
    if (appSettings?.download.always_ask_path) {
      return (await open({ directory: true, title: $t("youtube.choose_folder") })) as string | null;
    }
    const defaultDir = appSettings?.download.default_output_dir ?? null;
    if (defaultDir) return defaultDir;
    return (await open({ directory: true, title: $t("youtube.choose_folder") })) as string | null;
  }

  async function downloadUrls(urls: string[]) {
    if (batchActive || urls.length === 0) return;
    const outputDir = await resolveOutputDir();
    if (!outputDir) return;
    batchActive = true;
    batchCancelled = false;
    batchTotal = urls.length;
    batchDone = 0;
    let errors = 0;
    for (const url of urls) {
      if (batchCancelled) break;
      try {
        await invoke("download_from_url", { url, outputDir });
      } catch {
        errors++;
      }
      batchDone++;
    }
    batchActive = false;
    if (batchCancelled) {
      showToast("info", $t("youtube.batch_cancelled"));
    } else if (errors > 0) {
      showToast("error", $t("youtube.batch_partial", { done: batchDone - errors, total: urls.length }));
    } else {
      showToast("success", $t("youtube.batch_done", { count: urls.length }));
    }
  }

  async function downloadSelected() {
    const urls = Array.from(selected);
    if (urls.length === 0) {
      showToast("info", $t("youtube.no_selection"));
      return;
    }
    await downloadUrls(urls);
  }

  async function downloadAll() {
    const urls = entries.map((e) => e.url);
    await downloadUrls(urls);
  }

  function cancelBatch() {
    if (batchActive) batchCancelled = true;
  }

  function backToInput() {
    linkKind = "unknown";
    currentUrl = "";
    channelBaseUrl = "";
    resetState();
  }
</script>

<div class="page">
  <header class="header">
    <h1 class="title">{$t("youtube.title")}</h1>
    <p class="subtitle">{$t("youtube.subtitle")}</p>
  </header>

  {#if linkKind === "unknown"}
    <div class="url-input-section">
      <label class="input-label" for="yt-url-input">{$t("youtube.url_label")}</label>
      <div class="url-input-row">
        <input
          id="yt-url-input"
          type="url"
          class="input url-input"
          placeholder={$t("youtube.url_placeholder")}
          bind:value={inputUrl}
          onkeydown={(e) => { if (e.key === "Enter") submitUrl(); }}
        />
        <button class="button primary" onclick={submitUrl} disabled={!inputUrl.trim()}>
          {$t("youtube.load_btn")}
        </button>
      </div>
      <p class="hint">{$t("youtube.url_hint")}</p>
    </div>
  {:else}
    <div class="content">
      <div class="session-bar">
        <button class="button back-btn" onclick={backToInput}>
          {$t("youtube.back_to_input")}
        </button>
        <span class="url-preview" title={currentUrl}>{currentUrl}</span>
      </div>

      {#if linkKind === "channel"}
        <div class="tabs">
          {#each TABS as tab}
            <button
              class="button tab-btn"
              class:active={activeTab === tab.key}
              onclick={() => switchTab(tab.key)}
              disabled={loadingTab !== null || batchActive}
            >
              {$t(tab.labelKey)}
              {#if loadedTabs.has(tab.key) && entriesByTab[tab.key].length > 0}
                <span class="tab-count">{entriesByTab[tab.key].length}</span>
              {/if}
            </button>
          {/each}
        </div>
      {/if}

      {#if loadingTab === activeTab}
        <div class="spinner-section">
          <span class="spinner"></span>
          <span class="spinner-text">{$t("youtube.loading")}</span>
        </div>
      {:else if loadError}
        <div class="error-section">
          <p class="error-msg">{loadError}</p>
          <button class="button" onclick={() => loadTab(activeTab)}>{$t("common.retry")}</button>
        </div>
      {:else if entries.length === 0}
        <p class="empty-text">{$t("youtube.empty")}</p>
      {:else}
        <div class="list-header">
          <div class="header-left">
            <span class="subtext">
              {$t("youtube.count", { count: entries.length })}
              {#if !fullyLoadedTabs.has(activeTab) && entries.length >= INITIAL_LIMIT}
                &middot; <span class="partial-hint">{$t("youtube.partial_hint", { limit: INITIAL_LIMIT })}</span>
              {/if}
              {#if selectedCount > 0}
                &middot; <strong>{$t("youtube.selected_count", { count: selectedCount })}</strong>
              {/if}
            </span>
            <div class="select-actions">
              {#if allSelected}
                <button class="button link-btn" onclick={clearSelection} disabled={batchActive}>
                  {$t("youtube.clear_selection")}
                </button>
              {:else}
                <button class="button link-btn" onclick={selectAll} disabled={batchActive}>
                  {$t("youtube.select_all")}
                </button>
              {/if}
              <button class="button link-btn" onclick={invertSelection} disabled={batchActive || entries.length === 0}>
                {$t("youtube.invert")}
              </button>
            </div>
          </div>
          <div class="header-actions">
            {#if batchActive}
              <button class="button batch-cancel-btn" onclick={cancelBatch}>
                {$t("youtube.cancel_batch")}
              </button>
            {:else}
              {#if !fullyLoadedTabs.has(activeTab) && entries.length >= INITIAL_LIMIT}
                <button
                  class="button"
                  onclick={loadFullChannel}
                  disabled={loadingFull}
                >
                  {#if loadingFull}
                    <span class="spinner small"></span>
                  {/if}
                  {$t("youtube.load_full")}
                </button>
              {/if}
              <button
                class="button"
                onclick={downloadSelected}
                disabled={selectedCount === 0}
              >
                {$t("youtube.download_selected", { count: selectedCount })}
              </button>
              <button
                class="button primary"
                onclick={downloadAll}
                disabled={entries.length === 0}
              >
                {$t("youtube.download_all")}
              </button>
            {/if}
          </div>
        </div>

        {#if batchActive || batchTotal > 0}
          <div class="batch-progress-section">
            <div class="batch-progress-bar-outer">
              <div
                class="batch-progress-bar-inner"
                style:width="{batchTotal > 0 ? (batchDone / batchTotal) * 100 : 0}%"
              ></div>
            </div>
            <span class="subtext">
              {$t("youtube.batch_progress", { done: batchDone, total: batchTotal })}
            </span>
          </div>
        {/if}

        <div class="entry-list">
          {#each entries as entry (entry.url)}
            {@const isSelected = selected.has(entry.url)}
            {@const thumb = thumbnailFor(entry.url)}
            <label class="entry-row" class:selected={isSelected}>
              <input
                type="checkbox"
                class="entry-checkbox"
                checked={isSelected}
                onchange={() => toggleSelection(entry.url)}
                disabled={batchActive}
              />
              <div class="entry-thumb">
                {#if thumb}
                  <img
                    src={thumb}
                    alt=""
                    loading="lazy"
                    decoding="async"
                    referrerpolicy="no-referrer"
                    onerror={(e) => ((e.currentTarget as HTMLImageElement).style.display = "none")}
                  />
                {:else}
                  <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                    <rect x="2" y="5" width="20" height="14" rx="2" />
                    <path d="M10 9l5 3-5 3z" fill="currentColor" stroke="none" />
                  </svg>
                {/if}
              </div>
              <div class="entry-info">
                <span class="entry-title" title={entry.title}>{entry.title}</span>
                <span class="entry-url">{entry.url}</span>
              </div>
              <span class="entry-index">#{entry.index}</span>
            </label>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    padding: var(--padding);
    max-width: 1000px;
    margin: 0 auto;
    width: 100%;
  }

  .header {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .title {
    font-size: var(--text-lg);
    font-weight: 600;
    margin: 0;
  }

  .subtitle {
    color: var(--gray);
    font-size: var(--text-sm);
    margin: 0;
  }

  .url-input-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: var(--padding);
    background: var(--card);
    border-radius: var(--radius);
  }

  .input-label {
    font-size: var(--text-sm);
    color: var(--secondary);
    font-weight: 500;
  }

  .url-input-row {
    display: flex;
    gap: 8px;
  }

  .url-input {
    flex: 1;
  }

  .hint {
    color: var(--gray);
    font-size: 12px;
    margin: 0;
  }

  .content {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
  }

  .session-bar {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .back-btn {
    flex-shrink: 0;
  }

  .url-preview {
    color: var(--gray);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tabs {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }

  .tab-btn {
    padding: 6px 12px;
    font-size: var(--text-sm);
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .tab-btn.active {
    background: var(--button);
    color: var(--primary);
  }

  .tab-count {
    background: var(--button-elevated);
    padding: 1px 6px;
    border-radius: 100px;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }

  .spinner-section,
  .error-section {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: calc(var(--padding) * 2);
    color: var(--gray);
  }

  .empty-text {
    padding: calc(var(--padding) * 2);
    text-align: center;
    color: var(--gray);
  }

  .list-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 12px;
    flex-wrap: wrap;
  }

  .header-left {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .select-actions {
    display: flex;
    gap: 8px;
  }

  .link-btn {
    background: none;
    border: none;
    color: var(--blue);
    padding: 2px 4px;
    font-size: 12px;
    cursor: pointer;
  }

  .link-btn:hover:not(:disabled) {
    text-decoration: underline;
  }

  .link-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .header-actions {
    display: flex;
    gap: 8px;
  }

  .batch-cancel-btn {
    color: var(--red);
  }

  .batch-progress-section {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .batch-progress-bar-outer {
    width: 100%;
    height: 4px;
    background: var(--button-elevated);
    border-radius: 100px;
    overflow: hidden;
  }

  .batch-progress-bar-inner {
    height: 100%;
    background: var(--blue);
    border-radius: 100px;
    transition: width 200ms ease-out;
  }

  .subtext {
    color: var(--gray);
    font-size: var(--text-sm);
  }

  .entry-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .entry-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    background: var(--card);
    border-radius: var(--radius);
    cursor: pointer;
    transition: background 150ms;
  }

  .entry-row:hover {
    background: var(--button);
  }

  .entry-row.selected {
    background: color-mix(in oklab, var(--blue) 12%, var(--card));
    border: 1px solid color-mix(in oklab, var(--blue) 40%, transparent);
    padding: 9px 11px;
  }

  .entry-checkbox {
    flex-shrink: 0;
    cursor: pointer;
    width: 16px;
    height: 16px;
  }

  .entry-thumb {
    color: var(--gray);
    flex-shrink: 0;
    width: 96px;
    height: 54px;
    background: var(--button-elevated);
    border-radius: 6px;
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .entry-thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .entry-info {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .entry-title {
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .entry-url {
    color: var(--gray);
    font-size: 11px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .entry-index {
    color: var(--gray);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--button-elevated);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 800ms linear infinite;
  }

  .spinner.small {
    width: 12px;
    height: 12px;
    border-width: 1.5px;
  }

  .partial-hint {
    color: var(--gold, #f59e0b);
    font-size: 11.5px;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
