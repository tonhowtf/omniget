<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { library, seriesCover } from "$lib/read/library-store.svelte";
  import { chapterLabel, go, openChapter, relTime, tr } from "$lib/read/ui-store.svelte";
  import type { UpdateItem } from "$lib/read/types";
  import Cover from "../Cover.svelte";
  import ReadIcon from "../ReadIcon.svelte";

  let items = $state<UpdateItem[]>([]);
  let loading = $state(true);
  let onlyUnread = $state(true);
  let lib = $derived(library());

  async function load() {
    items = await invoke<UpdateItem[]>("read_updates_feed", { limit: 400 }).catch(() => []);
    loading = false;
  }
  onMount(load);
  $effect(() => {
    void lib.series;
    void load();
  });

  let groups = $derived.by(() => {
    const m = new Map<string, UpdateItem[]>();
    for (const u of items) {
      if (onlyUnread && u.chapter.read) continue;
      const d = new Date(u.chapter.fetched_at).toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
      m.set(d, [...(m.get(d) ?? []), u]);
    }
    return [...m.entries()];
  });

  async function check() {
    try {
      const r = await invoke<{ new_chapters: number }>("read_updates_check", { includeCompleted: false });
      showToast("success", tr("read.updates.found", { count: r.new_chapters }));
      await load();
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function markAll() {
    const ids = items.filter((u) => !u.chapter.read).map((u) => u.chapter.id);
    if (!ids.length) return;
    await invoke("read_chapters_mark", { ids, read: true });
    await load();
  }
</script>

<div class="r-h">
  <h1>{$t("read.nav.updates")}</h1>
  <span class="grow"></span>
  <label class="r-chip"><input type="checkbox" bind:checked={onlyUnread} /> {$t("read.updates.only_unread")}</label>
  <button class="btn" onclick={markAll}><ReadIcon name="check" size={14} /> {$t("read.updates.mark_all")}</button>
  <button class="btn btn-primary" onclick={check} disabled={!!lib.updating}><ReadIcon name="refresh" size={14} spin={!!lib.updating} /> {$t("read.updates.check")}</button>
</div>

{#if loading}
  <div class="r-empty"><ReadIcon name="refresh" size={22} spin /></div>
{:else if !groups.length}
  <div class="r-empty">
    <ReadIcon name="bell" size={38} />
    <p>{$t("read.updates.empty")}</p>
  </div>
{:else}
  {#each groups as [day, list] (day)}
    <h2 class="r-sub">{day}</h2>
    <ul class="r-list">
      {#each list as u (u.chapter.id)}
        <li class="r-li up" class:read={u.chapter.read}>
          <button class="cv" onclick={() => go({ kind: "series", id: u.series_id })} aria-label={u.series_title}>
            <Cover src={seriesCover({ cover: u.cover, cover_url: u.cover_url, source: "", url: "" })} title={u.series_title} ratio={1.45} radius={4} />
          </button>
          <button class="tx" onclick={() => openChapter(u.chapter)}>
            <b>{u.series_title}</b>
            <span>{chapterLabel(u.chapter)}</span>
            <em class="r-muted">{u.chapter.scanlator}{u.chapter.scanlator ? " · " : ""}{relTime(u.chapter.published ?? u.chapter.fetched_at)}</em>
          </button>
          {#if u.chapter.book_id}<ReadIcon name="check" size={14} />{/if}
          <button
            class="r-icon-btn"
            onclick={async () => {
              await invoke("read_chapters_mark", { ids: [u.chapter.id], read: !u.chapter.read });
              await load();
            }}
            title={u.chapter.read ? ($t("read.series.mark_unread") as string) : ($t("read.series.mark_read") as string)}
            ><ReadIcon name={u.chapter.read ? "eye-off" : "eye"} size={15} /></button
          >
        </li>
      {/each}
    </ul>
  {/each}
{/if}

<style>
  label.r-chip input {
    margin: 0;
  }
  .up.read {
    opacity: 0.55;
  }
  .cv {
    all: unset;
    width: 40px;
    flex-shrink: 0;
    cursor: pointer;
  }
  .tx {
    all: unset;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    cursor: pointer;
  }
  .tx span {
    color: var(--text-dim);
  }
  .tx em {
    font-style: normal;
    font-size: var(--text-xs);
  }
</style>
