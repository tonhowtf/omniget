<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { patchTubePrefs, tubePrefs } from "$lib/tube/prefs.svelte";
  import type { Item, WatchInfo } from "$lib/tube/types";
  import ChipBar from "../cards/ChipBar.svelte";
  import PlaylistCard from "../cards/PlaylistCard.svelte";
  import Sentinel from "../cards/Sentinel.svelte";
  import VideoRow from "../cards/VideoRow.svelte";
  import { isVisible } from "../cards/feedback.svelte";
  import { errText } from "../cards/util";

  let { info }: { info: WatchInfo } = $props();

  type Filter = "all" | "channel" | "related";
  let filter = $state<Filter>("all");
  let extra = $state<Item[]>([]);
  let next = $state<string | null>(null);
  let loadingMore = $state(false);
  let moreError = $state<string | null>(null);

  $effect(() => {
    void info.id;
    extra = [];
    next = info.related_continuation ?? null;
    filter = "all";
  });

  async function more() {
    if (!next || loadingMore) return;
    const id = info.id;
    loadingMore = true;
    moreError = null;
    try {
      const p = await api.continuation("related", next);
      if (info.id !== id) return;
      extra = [...extra, ...p.shelves.flatMap((s) => s.items)];
      next = p.continuation ?? null;
    } catch (e) {
      moreError = errText(e);
    } finally {
      loadingMore = false;
    }
  }

  let chId = $derived(info.channel?.id ?? null);
  let all = $derived([...info.related, ...extra].filter(isVisible));
  let fromChannel = $derived(all.filter((i) => (i.kind === "video" || i.kind === "playlist") && i.channel?.id === chId));
  let shown = $derived(
    filter === "channel" ? fromChannel : filter === "related" ? all.filter((i) => !fromChannel.includes(i)) : all,
  );
  let chips = $derived([
    { key: "all", label: $t("tube.watch.upnext_all") as string, selected: filter === "all" },
    ...(chId && fromChannel.length
      ? [{ key: "channel", label: $t("tube.watch.upnext_channel", { name: info.channel?.name ?? "" }) as string, selected: filter === "channel" }]
      : []),
    { key: "related", label: $t("tube.watch.upnext_related") as string, selected: filter === "related" },
  ]);
  let autoplay = $derived(tubePrefs().autoplay_next);
</script>

<section class="upnext" aria-label={$t("tube.watch.up_next") as string}>
  <div class="bar">
    <div class="chips"><ChipBar {chips} onselect={(k) => (filter = k as Filter)} label={$t("tube.watch.up_next") as string} /></div>
    <label class="auto" title={$t("tube.watch.autoplay") as string}>
      <span>{$t("tube.watch.autoplay")}</span>
      <input type="checkbox" role="switch" checked={autoplay} onchange={(e) => patchTubePrefs({ autoplay_next: e.currentTarget.checked })} />
      <i aria-hidden="true"></i>
    </label>
  </div>
  <div class="list">
    {#each shown as it, i (("id" in it ? it.id : "") + ":" + i)}
      {#if it.kind === "video"}
        <VideoRow item={it} size="compact" />
      {:else if it.kind === "playlist"}
        <PlaylistCard item={it} layout="compact" />
      {/if}
    {/each}
  </div>
  {#if next && filter === "all"}
    <Sentinel onmore={more} loading={loadingMore} error={moreError} />
  {/if}
</section>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 4px;
  }
  .chips {
    flex: 1;
    min-width: 0;
  }
  .auto {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    flex: none;
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
    cursor: pointer;
  }
  .auto input {
    position: absolute;
    opacity: 0;
    width: 30px;
    height: 18px;
    right: 0;
    margin: 0;
    cursor: pointer;
  }
  .auto i {
    position: relative;
    width: 30px;
    height: 18px;
    border-radius: 9px;
    background: var(--yt-tonal-hover);
    transition: background var(--yt-dur-base);
  }
  .auto i::after {
    content: "";
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--yt-text);
    transition: transform var(--yt-dur-base) var(--yt-ease-standard);
  }
  .auto input:checked + i {
    background: var(--yt-accent);
  }
  .auto input:checked + i::after {
    transform: translateX(12px);
    background: #fff;
  }
  .auto input:focus-visible + i {
    outline: 2px solid var(--yt-link);
    outline-offset: 2px;
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
</style>
