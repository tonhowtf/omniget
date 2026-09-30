<script lang="ts">
  // Explore: three big entry buttons (New releases, Charts, Moods & genres),
  // then the FEmusic_explore shelves.
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { go, type Route } from "$lib/tube/ui.svelte";
  import type { Page } from "$lib/tube/types";
  import MusicShelf from "../../music/MusicShelf.svelte";
  import TIcon from "../../TIcon.svelte";
  import MusicPage from "./MusicPage.svelte";
  import MusicSkeleton from "./MusicSkeleton.svelte";
  import MusicState from "./MusicState.svelte";

  let { route }: { route: Extract<Route, { name: "m_explore" }> } = $props();

  let page = $state<Page | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  const ENTRIES = [
    { id: "FEmusic_new_releases", key: "tube.music.new_releases", icon: "sparkles", tint: "#ff7a45" },
    { id: "FEmusic_charts", key: "tube.music.charts", icon: "list", tint: "#3ea6ff" },
    { id: "FEmusic_moods_and_genres", key: "tube.music.moods_genres", icon: "music-note", tint: "#b16cff" },
  ];

  async function load() {
    loading = true;
    error = null;
    try {
      page = await api.musicExplore();
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => void load());
</script>

<MusicPage readiness={loading ? "loading" : error ? "error" : "ready"} label={$t("tube.music.explore") as string}>
  <nav class="ex-entries" aria-label={$t("tube.music.explore") as string}>
    {#each ENTRIES as e (e.id)}
      <button class="ex-entry" style:--ex-tint={e.tint} onclick={() => go({ name: "m_browse", id: e.id, title: $t(e.key) as string })}>
        <span class="ex-icon"><TIcon name={e.icon} size={24} /></span>
        <span class="ex-label">{$t(e.key)}</span>
      </button>
    {/each}
  </nav>

  {#if loading}
    <MusicSkeleton variant="shelves" />
  {:else if error}
    <MusicState icon="explore" error title={$t("tube.music.error_title") as string} text={error} onretry={load} />
  {:else if page}
    {#each page.shelves as shelf, i (shelf.id + i)}
      <MusicShelf {shelf} />
    {/each}
  {/if}
</MusicPage>

<style>
  .ex-entries {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 16px;
    margin-bottom: 48px;
  }
  .ex-entry {
    display: flex; align-items: center; gap: 16px;
    height: 72px; padding: 0 24px;
    border: 0; border-radius: 8px;
    font: 700 18px/24px var(--yt-font-display); text-align: left;
    color: var(--ytm-text, var(--yt-text));
    background: linear-gradient(120deg, color-mix(in srgb, var(--ex-tint) 22%, var(--ytm-bg-a, var(--yt-bg-raised))), var(--ytm-bg-a, var(--yt-bg-raised)) 70%);
    cursor: pointer;
    transition: background var(--yt-dur-base) var(--yt-ease-standard), transform var(--yt-dur-fast);
  }
  .ex-entry:hover { background: linear-gradient(120deg, color-mix(in srgb, var(--ex-tint) 34%, var(--ytm-bg-a, var(--yt-bg-raised))), var(--ytm-chip-hover, var(--yt-tonal-hover)) 80%); }
  .ex-entry:active { transform: scale(.98); }
  .ex-entry:focus-visible { outline: 2px solid var(--yt-link); outline-offset: 2px; }
  .ex-icon {
    display: grid; place-items: center; width: 40px; height: 40px; border-radius: 50%; flex: none;
    color: var(--ex-tint); background: color-mix(in srgb, var(--ex-tint) 18%, transparent);
  }
  .ex-label { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  @container omnitube (max-width: 760px) {
    .ex-entries { grid-template-columns: 1fr; }
    .ex-entry { height: 56px; }
  }
</style>
