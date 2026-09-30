<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { addLibraryFolder, library } from "$lib/read/library-store.svelte";
  import { go, openBook, openChapter, relTime, fmtDuration, chapterLabel } from "$lib/read/ui-store.svelte";
  import type { Stats, UpdateItem } from "$lib/read/types";
  import BookCard from "../BookCard.svelte";
  import SeriesCard from "../SeriesCard.svelte";
  import ReadIcon from "../ReadIcon.svelte";

  let lib = $derived(library());
  let stats = $state<Stats | null>(null);
  let updates = $state<UpdateItem[]>([]);

  onMount(async () => {
    [stats, updates] = await Promise.all([
      invoke<Stats>("read_stats").catch(() => null),
      invoke<UpdateItem[]>("read_updates_feed", { limit: 12 }).catch(() => []),
    ]);
  });

  let reading = $derived(
    lib.books
      .filter((b) => b.status === "reading" && !b.chapter_id)
      .sort((a, b) => (b.last_read_at ?? 0) - (a.last_read_at ?? 0))
      .slice(0, 12),
  );
  let recent = $derived([...lib.books].filter((b) => !b.chapter_id).sort((a, b) => b.added_at - a.added_at).slice(0, 12));
  let unreadSeries = $derived(
    lib.series
      .filter((s) => s.unread > 0)
      .sort((a, b) => (b.new_at ?? b.last_read_at ?? 0) - (a.new_at ?? a.last_read_at ?? 0))
      .slice(0, 12),
  );
  let want = $derived(lib.books.filter((b) => b.status === "want").slice(0, 12));
  let greeting = $derived.by(() => {
    const h = new Date().getHours();
    return h < 5 ? "night" : h < 12 ? "morning" : h < 18 ? "afternoon" : "evening";
  });
  let goalPct = $derived(stats?.goal?.books ? Math.min(100, Math.round((stats.finished_this_year / stats.goal.books) * 100)) : 0);

  async function addFolder() {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir !== "string") return;
    try {
      await addLibraryFolder(dir);
      showToast("success", $t("read.settings.folder_added") as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  let empty = $derived(lib.books.length === 0 && lib.series.length === 0);
</script>

{#if empty}
  <div class="r-empty">
    <ReadIcon name="book-open" size={42} />
    <h2>{$t("read.empty.title")}</h2>
    <p>{$t("read.empty.body")}</p>
    <div class="r-line" style="justify-content:center">
      <button class="btn btn-primary" onclick={addFolder}><ReadIcon name="folder" size={15} /> {$t("read.empty.add_folder")}</button>
      <button class="btn" onclick={() => go({ kind: "browse" })}><ReadIcon name="compass" size={15} /> {$t("read.empty.browse")}</button>
      <button class="btn" onclick={() => go({ kind: "settings", tab: "import" })}><ReadIcon name="upload" size={15} /> {$t("read.empty.import")}</button>
    </div>
    <p class="r-muted">{$t("read.drop_hint")}</p>
  </div>
{:else}
  <div class="r-h"><h1>{$t(`read.home.greeting_${greeting}`)}</h1></div>

  {#if stats}
    <div class="tiles">
      <button class="tile" onclick={() => go({ kind: "stats" })}>
        <ReadIcon name="clock" size={16} />
        <b>{fmtDuration(stats.today_ms)}</b><span>{$t("read.stats.today")}</span>
      </button>
      <button class="tile" onclick={() => go({ kind: "stats" })}>
        <ReadIcon name="flame" size={16} />
        <b>{stats.streak_days}</b><span>{$t("read.stats.streak")}</span>
      </button>
      <button class="tile" onclick={() => go({ kind: "stats" })}>
        <ReadIcon name="target" size={16} />
        {#if stats.goal?.books}
          <b>{stats.finished_this_year}/{stats.goal.books}</b><span>{$t("read.stats.goal_short")}</span>
          <div class="r-bar"><i style:width="{goalPct}%"></i></div>
        {:else}
          <b>{stats.finished_this_year}</b><span>{$t("read.stats.finished_year")}</span>
        {/if}
      </button>
      <button class="tile" onclick={() => go({ kind: "library" })}>
        <ReadIcon name="library" size={16} />
        <b>{lib.books.length}</b><span>{$t("read.stats.books")}</span>
      </button>
    </div>
  {/if}

  {#if reading.length}
    <h2 class="r-sub">{$t("read.home.continue")}</h2>
    <div class="r-row">
      {#each reading as b (b.id)}
        <BookCard book={b} onopen={() => go({ kind: "book", id: b.id })} onread={() => openBook(b)} />
      {/each}
    </div>
  {/if}

  {#if unreadSeries.length}
    <h2 class="r-sub">{$t("read.home.new_chapters")} <button class="r-chip" onclick={() => go({ kind: "updates" })}>{$t("read.see_all")}</button></h2>
    <div class="r-row">
      {#each unreadSeries as s (s.id)}
        <SeriesCard series={s} onopen={() => go({ kind: "series", id: s.id })} />
      {/each}
    </div>
  {/if}

  {#if updates.length}
    <h2 class="r-sub">{$t("read.home.latest_updates")}</h2>
    <ul class="r-list">
      {#each updates.slice(0, 6) as u (u.chapter.id)}
        <li class="r-li">
          <button class="upd" onclick={() => openChapter(u.chapter)}>
            <b>{u.series_title}</b>
            <span>{chapterLabel(u.chapter)}</span>
            <em class="r-muted">{relTime(u.chapter.published ?? u.chapter.fetched_at)}</em>
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  {#if want.length}
    <h2 class="r-sub">{$t("read.home.want")}</h2>
    <div class="r-row">
      {#each want as b (b.id)}
        <BookCard book={b} onopen={() => go({ kind: "book", id: b.id })} onread={() => openBook(b)} />
      {/each}
    </div>
  {/if}

  {#if recent.length}
    <h2 class="r-sub">{$t("read.home.recent")} <button class="r-chip" onclick={() => go({ kind: "library" })}>{$t("read.see_all")}</button></h2>
    <div class="r-row">
      {#each recent as b (b.id)}
        <BookCard book={b} onopen={() => go({ kind: "book", id: b.id })} onread={() => openBook(b)} />
      {/each}
    </div>
  {/if}
{/if}

<style>
  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
    gap: var(--space-3);
  }
  .tile {
    all: unset;
    box-sizing: border-box;
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    gap: 2px 10px;
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    cursor: pointer;
    color: var(--text-dim);
  }
  .tile:hover {
    background: var(--fill-1);
  }
  .tile:focus-visible {
    outline: var(--focus-ring);
  }
  .tile b {
    font-size: var(--text-xl);
    color: var(--text);
  }
  .tile span {
    grid-column: 1 / -1;
    font-size: var(--text-xs);
  }
  .tile .r-bar {
    grid-column: 1 / -1;
  }
  .upd {
    all: unset;
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    width: 100%;
    cursor: pointer;
  }
  .upd span {
    flex: 1;
    color: var(--text-dim);
  }
</style>
