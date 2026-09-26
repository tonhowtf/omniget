<script lang="ts">
  import { onMount } from "svelte";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { t } from "$lib/i18n";
  import ConfirmDialog from "$lib/study-components/ConfirmDialog.svelte";

  let confirmOpen = $state(false);
  let confirmMessage = $state("");
  let confirmAction: (() => void) | null = null;

  function askConfirm(message: string, action: () => void) {
    confirmMessage = message;
    confirmAction = action;
    confirmOpen = true;
  }

  function runConfirm() {
    if (confirmAction) confirmAction();
    confirmAction = null;
  }

  type Series = {
    id: number;
    kind: string;
    source_type: string;
    title: string;
    author: string | null;
    cover_path: string | null;
    reading_direction: string;
    folder_path: string | null;
    added_at: number;
    last_opened_at: number | null;
    favorite: boolean;
    entry_count: number;
    read_count: number;
    progress_pct: number;
  };

  let series = $state<Series[]>([]);
  let loading = $state(true);
  let scanning = $state(false);
  let scanMsg = $state("");
  let search = $state("");
  let onlyFavorites = $state(false);

  const visible = $derived.by(() => {
    const term = search.trim().toLowerCase();
    return series.filter((s) => {
      if (onlyFavorites && !s.favorite) return false;
      if (term) {
        const hay = `${s.title} ${s.author ?? ""}`.toLowerCase();
        if (!hay.includes(term)) return false;
      }
      return true;
    });
  });

  async function loadSeries() {
    loading = true;
    try {
      const arr = await pluginInvoke<Series[]>("study", "study:read:series:list", {
        filters: {},
      });
      series = Array.isArray(arr) ? arr : [];
    } catch (e) {
      console.error(e);
    } finally {
      loading = false;
    }
  }

  async function rescan() {
    if (scanning) return;
    scanning = true;
    try {
      const res = await pluginInvoke<{
        series_found: number;
        added: number;
        updated: number;
        removed: number;
      }>("study", "study:read:series:scan");
      scanMsg = $t("study.read.series_scan_done", {
        found: res?.series_found ?? 0,
        added: res?.added ?? 0,
      });
      await loadSeries();
    } catch (e) {
      scanMsg = e instanceof Error ? e.message : String(e);
    } finally {
      scanning = false;
    }
  }

  let cleaning = $state(false);

  async function cleanOrphans() {
    if (cleaning) return;
    cleaning = true;
    try {
      const res = await pluginInvoke<{ removed: number }>(
        "study",
        "study:read:series:clear_orphans",
      );
      scanMsg = $t("study.read.manga_orphans_cleared", { count: res?.removed ?? 0 });
      await loadSeries();
    } catch (e) {
      scanMsg = e instanceof Error ? e.message : String(e);
    } finally {
      cleaning = false;
    }
  }

  async function clearRemote() {
    if (cleaning) return;
    askConfirm($t("study.read.manga_clear_remote_confirm"), () => doClearRemote());
  }
  async function doClearRemote() {
    cleaning = true;
    try {
      const res = await pluginInvoke<{ removed: number }>(
        "study",
        "study:read:series:clear_remote",
      );
      scanMsg = $t("study.read.manga_remote_cleared", { count: res?.removed ?? 0 });
      await loadSeries();
    } catch (e) {
      scanMsg = e instanceof Error ? e.message : String(e);
    } finally {
      cleaning = false;
    }
  }

  async function clearAll() {
    if (cleaning) return;
    askConfirm($t("study.read.manga_clear_all_confirm"), () => doClearAll());
  }
  async function doClearAll() {
    cleaning = true;
    try {
      const res = await pluginInvoke<{ removed: number }>(
        "study",
        "study:read:series:clear_all",
      );
      scanMsg = $t("study.read.manga_all_cleared", { count: res?.removed ?? 0 });
      await loadSeries();
    } catch (e) {
      scanMsg = e instanceof Error ? e.message : String(e);
    } finally {
      cleaning = false;
    }
  }

  async function toggleFav(s: Series, ev: MouseEvent) {
    ev.stopPropagation();
    ev.preventDefault();
    try {
      const res = await pluginInvoke<{ id: number; favorite: boolean }>(
        "study",
        "study:read:series:toggle_favorite",
        { seriesId: s.id },
      );
      series = series.map((x) => (x.id === res.id ? { ...x, favorite: res.favorite } : x));
    } catch (e) {
      console.error(e);
    }
  }

  function open(s: Series) {
    goto(`/study/read/manga/${s.id}`);
  }

  function kindLabel(kind: string): string {
    const key = `study.read.kind_${kind}`;
    const tl = $t(key);
    return tl === key ? kind : tl;
  }

  onMount(loadSeries);
</script>

<section class="read-page">
  <header class="head">
    <div class="row-primary">
      <button type="button" class="back-btn" onclick={() => goto("/study/read")}>
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M15 18l-6-6 6-6"></path>
        </svg>
        <span>{$t("study.read.back_to_library")}</span>
      </button>
      <h1>{$t("study.read.manga_title")}</h1>
      <button type="button" class="ghost-btn" onclick={rescan} disabled={scanning}>
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M21 12a9 9 0 1 1-3-6.7"></path>
          <path d="M21 3v6h-6"></path>
        </svg>
        <span>{scanning ? $t("study.read.scanning") : $t("study.read.scan_now")}</span>
      </button>
      <a href="/study/read/discover" class="ghost-btn">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <circle cx="11" cy="11" r="7"></circle>
          <path d="M21 21l-4.35-4.35"></path>
        </svg>
        <span>{$t("study.read.discover_nav")}</span>
      </a>
      <button type="button" class="ghost-btn" onclick={cleanOrphans} disabled={cleaning || scanning} title={$t("study.read.manga_clean_orphans_hint")}>
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M3 6h18"></path>
          <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path>
          <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"></path>
        </svg>
        <span>{cleaning ? $t("study.common.loading") : $t("study.read.manga_clean_orphans_btn")}</span>
      </button>
      <button type="button" class="ghost-btn danger" onclick={clearRemote} disabled={cleaning || scanning}>
        <span>{$t("study.read.manga_clear_remote_btn")}</span>
      </button>
      <button type="button" class="ghost-btn danger" onclick={clearAll} disabled={cleaning || scanning}>
        <span>{$t("study.read.manga_clear_all_btn")}</span>
      </button>
    </div>
    <p class="subtitle">{$t("study.read.manga_subtitle")}</p>
    <div class="row-secondary">
      <input
        type="search"
        class="search"
        placeholder={$t("study.read.search_placeholder")}
        bind:value={search}
      />
      <label class="fav-filter">
        <input type="checkbox" bind:checked={onlyFavorites} />
        <span>{$t("study.read.filter_favorites")}</span>
      </label>
    </div>
    {#if scanMsg}
      <p class="muted small">{scanMsg}</p>
    {/if}
  </header>

  {#if loading}
    <p class="muted">{$t("study.common.loading")}</p>
  {:else if visible.length === 0}
    <div class="empty-hint">
      <h3>{$t("study.read.series_empty_title")}</h3>
      <p class="muted">{$t("study.read.series_empty_hint")}</p>
      <button type="button" class="cta" onclick={rescan} disabled={scanning}>
        {$t("study.read.scan_now")}
      </button>
    </div>
  {:else}
    <div class="grid">
      {#each visible as s (s.id)}
        <button type="button" class="card" onclick={() => open(s)}>
          <div
            class="thumb"
            style={s.cover_path
              ? `background-image: url('${convertFileSrc(s.cover_path)}');`
              : ""}
          >
            {#if !s.cover_path}
              <svg viewBox="0 0 64 64" width="44" height="56" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <rect x="8" y="6" width="48" height="52" rx="2"></rect>
                <path d="M8 16h48M8 48h48"></path>
              </svg>
            {/if}
            <span
              class="star"
              class:active={s.favorite}
              role="button"
              tabindex="0"
              onclick={(e) => toggleFav(s, e as MouseEvent)}
              onkeydown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  e.stopPropagation();
                  toggleFav(s, e as unknown as MouseEvent);
                }
              }}
              aria-label={s.favorite ? "unfavorite" : "favorite"}
            >
              {s.favorite ? "★" : "☆"}
            </span>
            <div class="progress" style:--pct="{Math.round(s.progress_pct * 100)}%"></div>
          </div>
          <div class="meta">
            <div class="title">{s.title}</div>
            {#if s.author}
              <div class="author">{s.author}</div>
            {/if}
            <div class="sub">
              <span>{kindLabel(s.kind)}</span>
              <span>·</span>
              <span>{$t("study.read.series_count_chapters", { current: s.read_count, total: s.entry_count })}</span>
            </div>
          </div>
        </button>
      {/each}
    </div>
  {/if}
</section>

<ConfirmDialog
  bind:open={confirmOpen}
  message={confirmMessage}
  variant="danger"
  onConfirm={runConfirm}
/>

<style>
  .read-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 2);
    width: 100%;
    max-width: 1200px;
    margin-inline: auto;
  }
  .head {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
  }
  .row-primary {
    display: flex;
    align-items: center;
    gap: 1rem;
    flex-wrap: wrap;
  }
  h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 500;
    letter-spacing: -0.5px;
    color: var(--secondary);
    flex: 1;
  }
  .back-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    background: transparent;
    color: var(--tertiary);
    border: none;
    border-radius: var(--border-radius);
    font-size: 12px;
    cursor: pointer;
  }
  .back-btn:hover {
    color: var(--secondary);
    background: var(--sidebar-highlight);
  }
  .ghost-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    background: transparent;
    color: var(--tertiary);
    border: none;
    border-radius: var(--border-radius);
    font-size: 12px;
    cursor: pointer;
    text-decoration: none;
    font-family: inherit;
  }
  .ghost-btn:hover:not(:disabled) {
    color: var(--secondary);
    background: var(--sidebar-highlight);
  }
  .ghost-btn.danger {
    color: var(--error);
    border-color: color-mix(in oklab, var(--error) 35%, var(--input-border));
  }
  .ghost-btn.danger:hover:not(:disabled) {
    background: color-mix(in oklab, var(--error) 12%, transparent);
  }
  .ghost-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .subtitle {
    margin: 0;
    font-size: 14px;
    color: var(--tertiary);
  }
  .row-secondary {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    flex-wrap: wrap;
  }
  .search {
    flex: 1;
    min-width: 240px;
    padding: 6px 12px;
    background: var(--input-bg);
    color: var(--secondary);
    border: none;
    border-radius: var(--border-radius);
    font-size: 13px;
    font-family: inherit;
  }
  .fav-filter {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--tertiary);
    cursor: pointer;
  }
  .muted {
    color: var(--tertiary);
  }
  .small {
    font-size: 11px;
  }
  .empty-hint {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 32px;
    text-align: center;
  }
  .empty-hint h3 {
    margin: 0;
    font-size: 16px;
    font-weight: 500;
    color: var(--secondary);
  }
  .cta {
    display: inline-block;
    padding: 10px 20px;
    background: var(--cta, var(--accent));
    color: var(--on-cta, white);
    border: 1px solid transparent;
    border-radius: var(--border-radius);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    font-family: inherit;
    margin-top: 8px;
  }
  .cta:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: calc(var(--padding) * 1.2);
  }
  .card {
    display: flex;
    flex-direction: column;
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    overflow: hidden;
    color: inherit;
    cursor: pointer;
    font-family: inherit;
    text-align: left;
    padding: 0;
    transition: border-color 150ms ease;
  }
  .card:hover {
    border-color: var(--accent);
  }
  .thumb {
    position: relative;
    aspect-ratio: 2 / 3;
    background: var(--input-bg);
    background-size: cover;
    background-position: center;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--tertiary);
  }
  .star {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 24px;
    height: 24px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: color-mix(in oklab, var(--primary) 75%, transparent);
    border: none;
    border-radius: 50%;
    font-size: 13px;
    cursor: pointer;
    color: var(--tertiary);
  }
  .star.active {
    color: var(--accent);
    border-color: var(--accent);
  }
  .progress {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    height: 3px;
    background: var(--input-border);
  }
  .progress::after {
    content: "";
    position: absolute;
    inset: 0;
    width: var(--pct);
    background: var(--accent);
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px 12px;
  }
  .title {
    font-size: 13px;
    font-weight: 500;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  .author {
    font-size: 11px;
    color: var(--tertiary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    display: flex;
    gap: 4px;
    font-size: 10px;
    color: var(--tertiary);
    text-transform: uppercase;
    letter-spacing: 0.4px;
    margin-top: 2px;
  }
</style>
