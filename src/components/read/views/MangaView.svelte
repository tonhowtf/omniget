<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { library, refreshSeries } from "$lib/read/library-store.svelte";
  import { confirm, followSeries, go, openMenu, tr, ui } from "$lib/read/ui-store.svelte";
  import type { Series } from "$lib/read/types";
  import SeriesCard from "../SeriesCard.svelte";
  import ReadIcon from "../ReadIcon.svelte";

  type Category = { id: number; name: string; position: number };
  let lib = $derived(library());
  let cats = $state<Category[]>([]);
  let cat = $state<number | "all">("all");
  let filter = $state<"all" | "unread" | "downloaded" | "completed">("all");
  let sort = $state<"title" | "unread" | "updated" | "read" | "added">("updated");
  let newCat = $state("");

  onMount(async () => {
    cats = await invoke<Category[]>("read_categories").catch(() => []);
    void refreshSeries();
  });

  let list = $derived.by(() => {
    let l = [...lib.series];
    if (cat !== "all") l = l.filter((s) => s.categories.includes(cat as number));
    if (filter === "unread") l = l.filter((s) => s.unread > 0);
    if (filter === "downloaded") l = l.filter((s) => s.downloaded > 0);
    if (filter === "completed") l = l.filter((s) => /complete|finished|ended/i.test(s.status));
    const q = ui.search.trim().toLowerCase();
    if (q) l = l.filter((s) => s.title.toLowerCase().includes(q) || s.authors.some((a) => a.toLowerCase().includes(q)));
    const by: Record<string, (a: Series, b: Series) => number> = {
      title: (a, b) => a.title.localeCompare(b.title),
      unread: (a, b) => b.unread - a.unread,
      updated: (a, b) => (b.new_at ?? 0) - (a.new_at ?? 0),
      read: (a, b) => (b.last_read_at ?? 0) - (a.last_read_at ?? 0),
      added: (a, b) => b.added_at - a.added_at,
    };
    return l.sort(by[sort]);
  });

  async function checkUpdates() {
    try {
      const r = await invoke<{ new_chapters: number; failed: number }>("read_updates_check", { includeCompleted: false });
      showToast("success", tr("read.updates.found", { count: r.new_chapters }));
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function addCat() {
    if (!newCat.trim()) return;
    await invoke("read_category_save", { id: null, name: newCat.trim() });
    newCat = "";
    cats = await invoke<Category[]>("read_categories");
  }

  function menuFor(s: Series, e: MouseEvent) {
    openMenu(e, [
      { label: tr("read.series.open"), icon: "note", action: () => go({ kind: "series", id: s.id }) },
      { label: tr("read.series.refresh"), icon: "refresh", action: () => invoke("read_series_refresh", { id: s.id }) },
      {
        label: tr("read.series.categories"),
        icon: "tag",
        sub: cats.map((c) => ({
          label: c.name,
          checked: s.categories.includes(c.id),
          action: () =>
            invoke("read_series_update", {
              id: s.id,
              patch: { categories: s.categories.includes(c.id) ? s.categories.filter((x) => x !== c.id) : [...s.categories, c.id] },
            }),
        })),
      },
      { divider: true },
      {
        label: tr("read.series.unfollow"),
        icon: "trash",
        danger: true,
        action: () => confirm(tr("read.series.unfollow"), tr("read.series.unfollow_body"), tr("read.series.unfollow"), () => followSeries(s, false)),
      },
    ]);
  }
</script>

<div class="r-h">
  <h1>{$t("read.nav.manga")}</h1>
  <span class="r-muted">{list.length}</span>
  <span class="grow"></span>
  <button class="btn" onclick={checkUpdates} disabled={!!lib.updating}>
    <ReadIcon name="refresh" size={14} spin={!!lib.updating} /> {$t("read.updates.check")}
  </button>
  <button class="btn btn-primary" onclick={() => go({ kind: "browse" })}><ReadIcon name="compass" size={14} /> {$t("read.nav.browse")}</button>
</div>

<div class="r-toolbar">
  <button class="r-chip" class:on={cat === "all"} onclick={() => (cat = "all")}>{$t("read.manga.all")}</button>
  {#each cats as c (c.id)}
    <button
      class="r-chip"
      class:on={cat === c.id}
      onclick={() => (cat = c.id)}
      oncontextmenu={(e) =>
        openMenu(e, [
          {
            label: tr("read.delete"),
            icon: "trash",
            danger: true,
            action: async () => {
              await invoke("read_category_delete", { id: c.id });
              cats = cats.filter((x) => x.id !== c.id);
            },
          },
        ])}>{c.name}</button
    >
  {/each}
  <form
    class="nc"
    onsubmit={(e) => {
      e.preventDefault();
      void addCat();
    }}
  >
    <input class="input" bind:value={newCat} placeholder={$t("read.manga.new_category") as string} />
  </form>
</div>
<div class="r-toolbar">
  {#each ["all", "unread", "downloaded", "completed"] as f (f)}
    <button class="r-chip" class:on={filter === f} onclick={() => (filter = f as typeof filter)}>{$t(`read.manga.filter_${f}`)}</button>
  {/each}
  <span style="flex:1"></span>
  <select class="input sel" bind:value={sort} aria-label={$t("read.library.sort") as string}>
    {#each ["updated", "unread", "read", "added", "title"] as s (s)}<option value={s}>{$t(`read.manga.sort_${s}`)}</option>{/each}
  </select>
</div>

{#if !list.length}
  <div class="r-empty">
    <ReadIcon name="manga" size={40} />
    <h2>{$t("read.manga.empty_title")}</h2>
    <p>{$t("read.manga.empty_body")}</p>
    <button class="btn btn-primary" onclick={() => go({ kind: "browse", source: "mangadex" })}>{$t("read.manga.browse_mangadex")}</button>
  </div>
{:else}
  <div class="r-grid">
    {#each list as s (s.id)}
      <SeriesCard series={s} onopen={() => go({ kind: "series", id: s.id })} onmenu={(e) => menuFor(s, e)} />
    {/each}
  </div>
{/if}

<style>
  .nc .input {
    height: 28px;
    width: 150px;
    font-size: var(--text-sm);
  }
  .sel {
    height: 28px;
    padding: 0 8px;
    font-size: var(--text-sm);
  }
</style>
