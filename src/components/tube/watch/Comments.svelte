<script lang="ts">
  // Comments (21st 4340 adapted): count + sort menu, then the list with
  // infinite scroll. Loads only when the section gets near the screen, so
  // opening a video doesn't spend a request on comments nobody reads.
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import type { Comment, CommentSort } from "$lib/tube/types";
  import TIcon from "../TIcon.svelte";
  import Popup from "../cards/Popup.svelte";
  import Sentinel from "../cards/Sentinel.svelte";
  import { errText } from "../cards/util";
  import CommentItem from "./CommentItem.svelte";

  let {
    token,
    countText = null,
    videoId,
    ownerAvatar = null,
    ownerName = "",
    onseek,
  }: {
    token: string | null | undefined;
    countText?: string | null;
    videoId: string;
    ownerAvatar?: string | null;
    ownerName?: string;
    onseek: (t: number) => void;
  } = $props();

  let items = $state<Comment[] | null>(null);
  let sorts = $state<CommentSort[]>([]);
  let count = $state<string | null>(null);
  let next = $state<string | null>(null);
  let error = $state<string | null>(null);
  let loadingMore = $state(false);
  let moreError = $state<string | null>(null);
  let sortOpen = $state(false);
  let sortBtn = $state<HTMLButtonElement>();
  let root = $state<HTMLElement>();
  let near = $state(false);
  let gen = 0;

  async function load(tk: string) {
    const g = ++gen;
    items = null;
    error = null;
    moreError = null;
    try {
      const p = await api.comments(tk);
      if (g !== gen) return;
      items = p.items;
      if (p.sorts.length) sorts = p.sorts;
      count = p.count_text ?? count;
      next = p.continuation ?? null;
    } catch (e) {
      if (g === gen) error = errText(e);
    }
  }

  $effect(() => {
    void token;
    items = null;
    sorts = [];
    count = countText;
    near = false;
  });

  $effect(() => {
    if (!root || near) return;
    const io = new IntersectionObserver((es) => {
      if (es.some((e) => e.isIntersecting)) near = true;
    }, { rootMargin: "600px 0px" });
    io.observe(root);
    return () => io.disconnect();
  });

  $effect(() => {
    if (near && token) void load(token);
  });

  async function more() {
    if (!next || loadingMore) return;
    loadingMore = true;
    moreError = null;
    const g = gen;
    try {
      const p = await api.comments(next);
      if (g !== gen) return;
      items = [...(items ?? []), ...p.items];
      next = p.continuation ?? null;
    } catch (e) {
      if (g === gen) moreError = errText(e);
    } finally {
      loadingMore = false;
    }
  }

  function pickSort(s: CommentSort) {
    sortOpen = false;
    sorts = sorts.map((x) => ({ ...x, selected: x.token === s.token }));
    void load(s.token);
  }
</script>

<section class="comments" bind:this={root} aria-label={$t("tube.comments.title") as string}>
  {#if !token}
    <p class="off">{$t("tube.comments.off")}</p>
  {:else}
    <header>
      <h2>{count ?? countText ?? $t("tube.comments.title")}</h2>
      {#if sorts.length > 1}
        <button bind:this={sortBtn} class="sort" aria-haspopup="menu" aria-expanded={sortOpen} onclick={() => (sortOpen = !sortOpen)}>
          <TIcon name="sort" size={22} />{$t("tube.comments.sort")}
        </button>
      {/if}
    </header>
    {#if sortOpen}
      <Popup anchor={sortBtn ?? null} align="start" onclose={() => (sortOpen = false)} label={$t("tube.comments.sort") as string}>
        {#each sorts as s (s.token)}
          <button class="yt-menu-item" class:sel={s.selected} role="menuitemradio" aria-checked={s.selected} onclick={() => pickSort(s)}>{s.label}</button>
        {/each}
      </Popup>
    {/if}

    {#if error}
      <p class="off">{error}</p>
      <button class="yt-btn yt-btn--s" onclick={() => token && load(token)}>{$t("tube.common.retry")}</button>
    {:else if items === null}
      <div class="list">
        {#each Array(4) as _, i (i)}
          <div class="sk">
            <div class="yt-skel av"></div>
            <div class="lines">
              <div class="yt-skel" style="height: 14px; width: 30%"></div>
              <div class="yt-skel" style="height: 14px; width: 80%"></div>
              <div class="yt-skel" style="height: 14px; width: 55%"></div>
            </div>
          </div>
        {/each}
      </div>
    {:else if !items.length}
      <p class="off">{$t("tube.comments.empty")}</p>
    {:else}
      <div class="list">
        {#each items as c (c.id)}
          <CommentItem {c} {videoId} {ownerAvatar} {ownerName} {onseek} />
        {/each}
      </div>
      {#if next}
        <Sentinel onmore={more} loading={loadingMore} error={moreError} />
      {/if}
    {/if}
  {/if}
</section>

<style>
  .comments {
    margin-top: 24px;
  }
  header {
    display: flex;
    align-items: center;
    gap: 32px;
    margin-bottom: 24px;
  }
  h2 {
    margin: 0;
    font: var(--yt-type-headline-s);
  }
  .sort {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 36px;
    padding: 0 8px;
    border: 0;
    border-radius: 18px;
    font: var(--yt-type-body-strong);
    color: var(--yt-text);
    background: none;
    cursor: pointer;
  }
  .sort:hover {
    background: var(--yt-hover);
  }
  .sort:focus-visible {
    outline: 2px solid var(--yt-link);
  }
  .sel {
    background: var(--yt-tonal);
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .sk {
    display: flex;
    gap: 16px;
  }
  .av {
    flex: none;
    width: 40px;
    height: 40px;
    border-radius: 50%;
  }
  .lines {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .off {
    color: var(--yt-text-2);
  }
</style>
