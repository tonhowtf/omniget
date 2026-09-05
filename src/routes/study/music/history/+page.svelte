<script lang="ts">
  import { onMount } from "svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    studyMusicHistoryList,
    studyMusicHistoryClear,
    studyMusicHistoryRemove,
    type MusicHistoryEntry,
    type MusicHistorySource,
  } from "$lib/study-bridge";
  import { musicPlayer } from "$lib/study-music/player-store.svelte";
  import EmptyPlaceholder from "$lib/study-music-components/EmptyPlaceholder.svelte";
  import { t } from "$lib/i18n";
  import { goto } from "$app/navigation";

  type LocalFilter = "all" | "local" | "spotify" | "soundcloud";
  const FILTERS: { key: LocalFilter; labelKey: string }[] = [
    { key: "all", labelKey: "study.music.history_filter_all" },
    { key: "local", labelKey: "study.music.history_filter_local" },
    { key: "spotify", labelKey: "study.music.history_filter_spotify" },
    { key: "soundcloud", labelKey: "study.music.history_filter_soundcloud" },
  ];

  let entries = $state<MusicHistoryEntry[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let activeFilter = $state<LocalFilter>("all");

  async function load() {
    loading = true;
    error = null;
    try {
      if (activeFilter === "all") {
        const res = await studyMusicHistoryList({ limit: 300 });
        entries = (res.entries ?? []).filter((e) => e.source !== "youtube");
      } else {
        const res = await studyMusicHistoryList({
          limit: 300,
          source: activeFilter as MusicHistorySource,
        });
        entries = res.entries ?? [];
      }
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  function pickFilter(next: LocalFilter) {
    if (next === activeFilter) return;
    activeFilter = next;
    void load();
  }

  function fmtAgo(tsMs: number): string {
    const seconds = Math.floor((Date.now() - tsMs) / 1000);
    if (seconds < 60) return $t("study.music.history_now") as string;
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes} min`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours} h`;
    const days = Math.floor(hours / 24);
    if (days < 30) return `${days} d`;
    const months = Math.floor(days / 30);
    if (months < 12) return `${months} m`;
    const years = Math.floor(months / 12);
    return `${years} a`;
  }

  function sourceLabel(source: MusicHistorySource): string {
    if (source === "spotify") return "Spotify";
    if (source === "soundcloud") return "SoundCloud";
    return $t("study.music.history_filter_local") as string;
  }

  async function play(entry: MusicHistoryEntry) {
    try {
      if (entry.track_id != null) {
        const track = musicPlayer.queue.find((t) => t.id === entry.track_id);
        if (track) {
          await musicPlayer.play(track);
          return;
        }
      }
      showToast("info", $t("study.music.history_unplayable") as string);
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    }
  }

  async function remove(id: number) {
    try {
      await studyMusicHistoryRemove({ id });
      entries = entries.filter((e) => e.id !== id);
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    }
  }

  async function clearAll() {
    if (!window.confirm($t("study.music.history_clear_confirm_local") as string)) return;
    try {
      if (activeFilter === "all") {
        const res = await studyMusicHistoryClear({});
        showToast("success", `${res.removed} ${$t("study.music.history_entries_removed") as string}`);
      } else {
        const res = await studyMusicHistoryClear({
          source: activeFilter as MusicHistorySource,
        });
        showToast("success", `${res.removed} ${$t("study.music.history_entries_removed") as string}`);
      }
      entries = [];
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    }
  }

  onMount(() => {
    void load();
  });
</script>

<section class="history-page">
  <header class="head">
    <h1>{$t("study.music.history_title")}</h1>
    {#if entries.length > 0}
      <button class="clear-btn" onclick={clearAll} type="button">{$t("study.music.history_clear")}</button>
    {/if}
  </header>

  <div class="filters" role="tablist" aria-label={$t("study.music.history_filter_aria") as string}>
    {#each FILTERS as f (f.key)}
      <button
        type="button"
        class="chip"
        class:active={activeFilter === f.key}
        role="tab"
        aria-selected={activeFilter === f.key}
        onclick={() => pickFilter(f.key)}
      >
        {$t(f.labelKey as never) as string}
      </button>
    {/each}
  </div>

  {#if loading}
    <div class="skel-list">
      {#each Array(6) as _, i (i)}
        <div class="skel-row"></div>
      {/each}
    </div>
  {:else if error}
    <div class="error">
      <p>{error}</p>
      <button onclick={() => { error = null; void load(); }} type="button">{$t("study.music.retry")}</button>
    </div>
  {:else if entries.length === 0}
    <EmptyPlaceholder
      title={$t("study.music.history_empty_title_local") as string}
      ctaLabel={$t("study.music.history_empty_cta_local") as string}
      onCta={() => goto("/study/music/search")}
    />
  {:else}
    <ul class="list">
      {#each entries as entry (entry.id)}
        <li class="row">
          <button class="thumb" onclick={() => play(entry)} type="button" aria-label={$t("study.music.play") as string}>
            {#if entry.cover_url}
              <img src={entry.cover_url} alt="" loading="lazy" />
            {/if}
            <span class="source-tag source-{entry.source}">{sourceLabel(entry.source)}</span>
          </button>
          <div class="info">
            <button class="title-btn" onclick={() => play(entry)} type="button">
              {entry.title}
            </button>
            <p class="meta">
              {#if entry.artist}
                <span>{entry.artist}</span>
                <span> · </span>
              {/if}
              <span>{fmtAgo(entry.played_at)}</span>
            </p>
          </div>
          <button class="remove" onclick={() => remove(entry.id)} aria-label={$t("study.music.history_remove_aria") as string} type="button">×</button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .history-page {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 16px 20px 32px;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .head h1 { margin: 0; font-size: 22px; font-weight: 700; color: var(--secondary); }
  .clear-btn {
    padding: 6px 14px;
    background: transparent;
    color: var(--tertiary);
    border: none;
    border-radius: 999px;
    font-size: 12px;
    cursor: pointer;
  }
  .clear-btn:hover { color: #d33; border-color: #d33; }
  .filters {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .chip {
    padding: 6px 14px;
    background: transparent;
    color: var(--tertiary);
    border: none;
    border-radius: 999px;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: color 120ms, border-color 120ms, background 120ms;
  }
  .chip:hover { color: var(--secondary); border-color: var(--accent); }
  .chip.active {
    color: var(--on-accent);
    background: var(--accent);
    border-color: var(--accent);
  }
  .list { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 8px; }
  .row {
    display: grid;
    grid-template-columns: 120px 1fr auto;
    align-items: center;
    gap: 14px;
    padding: 8px;
    border-radius: 10px;
  }
  .row:hover { background: color-mix(in oklab, var(--button) 30%, transparent); }
  .thumb {
    position: relative;
    width: 120px;
    aspect-ratio: 16 / 9;
    background: color-mix(in oklab, var(--button) 60%, transparent);
    border: 0;
    border-radius: 8px;
    overflow: hidden;
    cursor: pointer;
    padding: 0;
  }
  .thumb img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .source-tag {
    position: absolute;
    left: 6px;
    bottom: 6px;
    padding: 2px 6px;
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    border-radius: 4px;
    color: #fff;
    background: rgba(0, 0, 0, 0.65);
  }
  .source-spotify { background: rgba(30, 215, 96, 0.85); color: #000; }
  .source-soundcloud { background: rgba(255, 119, 0, 0.85); }
  .source-local { background: rgba(0, 0, 0, 0.7); }
  .info { min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .title-btn {
    padding: 0;
    background: transparent;
    border: 0;
    color: var(--secondary);
    font-size: 14px;
    font-weight: 600;
    text-align: left;
    cursor: pointer;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .title-btn:hover { color: var(--accent); }
  .meta { margin: 0; font-size: 12px; color: var(--tertiary); }
  .remove {
    width: 30px;
    height: 30px;
    padding: 0;
    background: transparent;
    border: 0;
    border-radius: 50%;
    color: var(--tertiary);
    font-size: 18px;
    cursor: pointer;
  }
  .remove:hover { color: #d33; background: color-mix(in oklab, var(--button) 40%, transparent); }
  .skel-list { display: flex; flex-direction: column; gap: 8px; }
  .skel-row {
    height: 80px;
    border-radius: 10px;
    background: linear-gradient(90deg, color-mix(in oklab, var(--button) 40%, transparent) 0%, color-mix(in oklab, var(--button) 60%, transparent) 50%, color-mix(in oklab, var(--button) 40%, transparent) 100%);
    background-size: 200% 100%;
    animation: shimmer 1.5s ease-in-out infinite;
  }
  @keyframes shimmer {
    0% { background-position: 200% 0; }
    100% { background-position: -200% 0; }
  }
  .error { padding: 24px; text-align: center; color: #d33; }
  .error button {
    margin-top: 8px;
    padding: 6px 14px;
    background: var(--accent);
    color: var(--on-accent);
    border: 0;
    border-radius: 999px;
    font-size: 12px;
    cursor: pointer;
  }
</style>
