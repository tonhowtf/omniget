<script lang="ts">
  /** Hashtag: total de posts, recentes ou populares, com download. */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText } from "$lib/tools/rt";
  import { cancelJob, igState, jobId, n, slugArg, type MediaItem, type TagInfo } from "$lib/tools/ig.svelte";
  import IgAccountRow from "./IgAccountRow.svelte";
  import IgMediaGrid from "./IgMediaGrid.svelte";

  let tag = $state("");
  let tab = $state("recent");
  let limit = $state(36);
  let busy = $state(false);
  let job = $state("");
  let info = $state<TagInfo | null>(null);
  let items = $state<MediaItem[]>([]);

  async function run() {
    if (!tag.trim() || busy) return;
    busy = true;
    info = null;
    items = [];
    job = jobId("tag");
    try {
      const r = await invoke<{ info: TagInfo; items: MediaItem[] }>("tool_ig_hashtag", { slug: slugArg(), tag: tag.trim(), tab, limit, job });
      info = r.info;
      items = r.items;
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  let stats = $derived.by(() => {
    if (!items.length) return null;
    const likes = items.reduce((a, i) => a + i.like_count, 0) / items.length;
    const comments = items.reduce((a, i) => a + i.comment_count, 0) / items.length;
    const related: Record<string, number> = {};
    for (const i of items) for (const h of i.hashtags) if (h !== info?.name) related[h] = (related[h] ?? 0) + 1;
    const top = Object.entries(related).sort((a, b) => b[1] - a[1]).slice(0, 20);
    return { likes, comments, top, videos: items.filter((i) => i.media_type === 2).length };
  });
</script>

<div class="tool">
  <IgAccountRow />
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="text" bind:value={tag} placeholder="#hashtag" onkeydown={(e) => e.key === "Enter" && run()} /></div>
        <div class="group-row-trailing btn-row">
          <select class="select" bind:value={tab}><option value="recent">{$t("tools.ig.hashtag.recent")}</option><option value="top">{$t("tools.ig.hashtag.top")}</option></select>
          <select class="select" bind:value={limit}><option value={18}>18</option><option value={36}>36</option><option value={72}>72</option><option value={150}>150</option></select>
          {#if busy}<button class="btn btn-secondary btn-sm" type="button" onclick={() => cancelJob(job)}>{$t("tools.ig.common.cancel")}</button>
          {:else}<button class="btn btn-primary" type="button" disabled={!tag.trim() || !igState.me} onclick={run}>{$t("tools.ig.hashtag.run")}</button>{/if}
        </div>
      </div>
      {#if info}
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">#{info.name} · {info.formatted_media_count || n(info.media_count)} {$t("tools.ig.common.posts")}</div>
          {#if stats}<div class="group-row-sub">{$t("tools.ig.hashtag.avg")}: ♥ {n(Math.round(stats.likes))} · 💬 {stats.comments.toFixed(1)} · {stats.videos} {$t("tools.ig.hashtag.videos")} / {items.length}</div>{/if}
        </div></div>
        {#if stats?.top.length}
          <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{$t("tools.ig.hashtag.related")}</div><div class="chips">{#each stats.top as [h, c] (h)}<button class="tag" type="button" onclick={() => { tag = h; run(); }}>#{h} · {c}</button>{/each}</div></div></div>
        {/if}
      {/if}
    </div>
  </section>
  <IgMediaGrid {items} jobPrefix="tag" />
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .chips { display: flex; flex-wrap: wrap; gap: var(--space-1); margin-top: var(--space-1); }
  button.tag { cursor: pointer; border: 0; }
</style>
