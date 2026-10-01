<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { fmtTime, qualityLabel, looksUpscaled, type SortKey } from "$lib/music/library";
  import { patchTracksLocal } from "$lib/music/library-store.svelte";
  import { player } from "$lib/music/player-store.svelte";
  import type { Track } from "$lib/music/types";
  import Cover from "./Cover.svelte";
  import MusicIcon from "./MusicIcon.svelte";

  type Column = "album" | "year" | "plays" | "bpm" | "key" | "quality" | "added" | "genre";

  let {
    tracks,
    onplay,
    onmenu,
    columns = ["album", "plays"] as Column[],
    numbered = "index",
    showCover = true,
    sort = null,
    desc = false,
    onsort,
    selected = $bindable(new Set<number>()),
    rowHeight = 46,
    emptyText = "",
    reorderable = false,
    onreorder,
  }: {
    tracks: Track[];
    onplay: (t: Track, list: Track[]) => void;
    onmenu?: (e: MouseEvent, ids: number[]) => void;
    columns?: Column[];
    numbered?: "index" | "track" | "none";
    showCover?: boolean;
    sort?: SortKey | null;
    desc?: boolean;
    onsort?: (k: SortKey) => void;
    selected?: Set<number>;
    rowHeight?: number;
    emptyText?: string;
    reorderable?: boolean;
    onreorder?: (from: number, to: number) => void;
  } = $props();

  let scroller: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let viewH = $state(600);
  let anchor = $state<number | null>(null);
  let dropAt = $state<number | null>(null);

  const OVERSCAN = 8;
  let start = $derived(Math.max(0, Math.floor(scrollTop / rowHeight) - OVERSCAN));
  let end = $derived(Math.min(tracks.length, Math.ceil((scrollTop + viewH) / rowHeight) + OVERSCAN));
  let visible = $derived(tracks.slice(start, end));
  let currentId = $derived(player.current?.id ?? null);
  let gridCols = $derived(
    `44px minmax(180px, 3fr) ${columns.map((c) => (c === "album" ? "minmax(120px, 2fr)" : "minmax(56px, 0.9fr)")).join(" ")} 56px 36px`,
  );

  $effect(() => {
    if (!scroller) return;
    const ro = new ResizeObserver((e) => (viewH = e[0].contentRect.height));
    ro.observe(scroller);
    return () => ro.disconnect();
  });

  function clickRow(e: MouseEvent, i: number, tr: Track) {
    const next = new Set(e.metaKey || e.ctrlKey || e.shiftKey ? selected : []);
    if (e.shiftKey && anchor != null) {
      const [a, b] = anchor < i ? [anchor, i] : [i, anchor];
      for (let k = a; k <= b; k++) next.add(tracks[k].id);
    } else if (e.metaKey || e.ctrlKey) {
      if (next.has(tr.id)) next.delete(tr.id);
      else next.add(tr.id);
      anchor = i;
    } else {
      next.add(tr.id);
      anchor = i;
    }
    selected = next;
  }

  function contextRow(e: MouseEvent, i: number, tr: Track) {
    e.preventDefault();
    if (!selected.has(tr.id)) {
      selected = new Set([tr.id]);
      anchor = i;
    }
    onmenu?.(e, [...selected]);
  }

  function keydown(e: KeyboardEvent) {
    if (!tracks.length) return;
    const idx = anchor ?? -1;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      const n = Math.max(0, Math.min(tracks.length - 1, idx + (e.key === "ArrowDown" ? 1 : -1)));
      const next = new Set(e.shiftKey ? selected : []);
      next.add(tracks[n].id);
      selected = next;
      anchor = n;
      ensureVisible(n);
      e.preventDefault();
    } else if (e.key === "Enter" && idx >= 0) {
      onplay(tracks[idx], tracks);
      e.preventDefault();
    } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "a") {
      selected = new Set(tracks.map((x) => x.id));
      e.preventDefault();
    } else if (e.key === "Escape") {
      selected = new Set();
    }
  }

  function ensureVisible(i: number) {
    if (!scroller) return;
    const top = i * rowHeight;
    if (top < scroller.scrollTop) scroller.scrollTop = top;
    else if (top + rowHeight > scroller.scrollTop + viewH) scroller.scrollTop = top + rowHeight - viewH;
  }

  export function scrollToTrack(id: number) {
    const i = tracks.findIndex((x) => x.id === id);
    if (i >= 0 && scroller) scroller.scrollTop = Math.max(0, i * rowHeight - viewH / 3);
  }

  function dragStart(e: DragEvent, tr: Track, i: number) {
    const ids = selected.has(tr.id) ? tracks.filter((x) => selected.has(x.id)).map((x) => x.id) : [tr.id];
    e.dataTransfer?.setData("application/x-omniget-tracks", JSON.stringify(ids));
    e.dataTransfer?.setData("text/plain", ids.map((id) => tracks.find((x) => x.id === id)?.title ?? "").join("\n"));
    if (reorderable) e.dataTransfer?.setData("application/x-omniget-row", String(i));
    if (e.dataTransfer) e.dataTransfer.effectAllowed = reorderable ? "copyMove" : "copy";
  }

  function dragOver(e: DragEvent, i: number) {
    if (!reorderable || !e.dataTransfer?.types.includes("application/x-omniget-row")) return;
    e.preventDefault();
    dropAt = i;
  }

  function drop(e: DragEvent, i: number) {
    if (!reorderable) return;
    const from = Number(e.dataTransfer?.getData("application/x-omniget-row"));
    dropAt = null;
    if (Number.isFinite(from) && from !== i) onreorder?.(from, i);
  }

  async function toggleLike(e: MouseEvent, tr: Track) {
    e.stopPropagation();
    const liked = !tr.liked;
    patchTracksLocal([tr.id], { liked });
    await invoke("music_set_liked", { ids: [tr.id], liked }).catch(() => patchTracksLocal([tr.id], { liked: !liked }));
  }

  const SORTABLE: Partial<Record<Column, SortKey>> = {
    album: "album",
    year: "year",
    plays: "plays",
    bpm: "bpm",
    key: "key",
    added: "added",
  };

  function head(k: SortKey | undefined) {
    if (k) onsort?.(k);
  }
</script>

<div class="tl" role="grid" aria-rowcount={tracks.length} tabindex="0" onkeydown={keydown} style:--grid={gridCols}>
  <div class="tl-head" role="row" class:has-cover={showCover}>
    <span class="c-num" role="columnheader">{numbered === "none" ? "" : "#"}</span>
    <button class="c-title sortable" class:on={sort === "title"} role="columnheader" onclick={() => head("title")}>
      {$t("music.col.title")}{#if sort === "title"}<MusicIcon name="chevron" size={12} class={desc ? "" : "flip"} />{/if}
    </button>
    {#each columns as c (c)}
      <button class="c-{c} sortable" class:on={SORTABLE[c] != null && sort === SORTABLE[c]} role="columnheader" onclick={() => head(SORTABLE[c])}>
        {$t(`music.col.${c}`)}
      </button>
    {/each}
    <button class="c-dur sortable" class:on={sort === "duration"} role="columnheader" onclick={() => head("duration")}><MusicIcon name="clock" size={13} /></button>
    <span class="c-like" role="columnheader"></span>
  </div>

  <div class="tl-body" bind:this={scroller} onscroll={() => (scrollTop = scroller?.scrollTop ?? 0)}>
    {#if !tracks.length}
      <p class="tl-empty">{emptyText || $t("music.empty_list")}</p>
    {:else}
      <div class="tl-spacer" style:height="{tracks.length * rowHeight}px">
        {#each visible as tr, j (tr.id)}
          {@const i = start + j}
          <div
            class="tl-row"
            class:sel={selected.has(tr.id)}
            class:cur={tr.id === currentId}
            class:drop={dropAt === i}
            class:has-cover={showCover}
            role="row"
            aria-selected={selected.has(tr.id)}
            tabindex="-1"
            style:top="{i * rowHeight}px"
            style:height="{rowHeight}px"
            draggable="true"
            ondragstart={(e) => dragStart(e, tr, i)}
            ondragover={(e) => dragOver(e, i)}
            ondragleave={() => (dropAt = null)}
            ondrop={(e) => drop(e, i)}
            onclick={(e) => clickRow(e, i, tr)}
            ondblclick={() => onplay(tr, tracks)}
            onkeydown={(e) => {
              if (e.key === "Enter") {
                e.stopPropagation();
                onplay(tr, tracks);
              }
            }}
            oncontextmenu={(e) => contextRow(e, i, tr)}
          >
            <span class="c-num">
              {#if tr.id === currentId}
                <span class="eqbars" class:paused={!player.playing}><i></i><i></i><i></i></span>
              {:else}
                <span class="num">{numbered === "track" ? tr.track_no ?? "" : numbered === "index" ? i + 1 : ""}</span>
                <button class="rowplay" aria-label={$t("music.play") as string} onclick={(e) => { e.stopPropagation(); onplay(tr, tracks); }}><MusicIcon name="play" size={14} /></button>
              {/if}
            </span>
            <span class="c-title">
              {#if showCover}<Cover hash={tr.cover} seed={tr.album || tr.title} size={32} radius={4} />{/if}
              <span class="tt">
                <span class="t1">{tr.title}</span>
                <span class="t2">{tr.artist || $t("music.unknown_artist")}</span>
              </span>
            </span>
            {#each columns as c (c)}
              <span class="c-{c}">
                {#if c === "album"}{tr.album}
                {:else if c === "year"}{tr.year ?? ""}
                {:else if c === "plays"}{tr.play_count || ""}
                {:else if c === "bpm"}{tr.bpm ? Math.round(tr.bpm) : ""}
                {:else if c === "key"}{#if tr.camelot}<span class="keychip">{tr.camelot}</span>{/if}
                {:else if c === "genre"}{tr.genre}
                {:else if c === "added"}{new Date(tr.added_at).toLocaleDateString()}
                {:else if c === "quality"}
                  <span class="q" class:warn={looksUpscaled(tr)} title={looksUpscaled(tr) ? ($t("music.upscaled_hint") as string) : ""}>{qualityLabel(tr)}</span>
                {/if}
              </span>
            {/each}
            <span class="c-dur">{fmtTime(tr.duration_ms)}</span>
            <span class="c-like">
              <button class="like" class:on={tr.liked} aria-label={$t(tr.liked ? "music.unlike" : "music.like") as string} onclick={(e) => toggleLike(e, tr)}>
                <MusicIcon name={tr.liked ? "heart-fill" : "heart"} size={15} />
              </button>
            </span>
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .tl {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
    outline: none;
  }
  .tl-head,
  .tl-row {
    display: grid;
    grid-template-columns: var(--grid);
    align-items: center;
    gap: var(--space-2);
    padding: 0 var(--space-3);
  }
  .tl-head {
    height: 30px;
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
    border-bottom: var(--hairline) solid var(--content-border);
  }
  .tl-head .sortable {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    cursor: pointer;
    font-size: var(--text-xs);
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    min-width: 0;
  }
  .tl-head .sortable.on,
  .tl-head .sortable:hover {
    color: var(--text);
  }
  .tl-head .sortable:focus-visible {
    outline: var(--focus-ring);
  }
  :global(.flip) {
    transform: rotate(180deg);
  }
  .tl-body {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .tl-spacer {
    position: relative;
  }
  .tl-row {
    position: absolute;
    left: 0;
    right: 0;
    border-radius: var(--radius-sm);
    font-size: var(--text-base);
    color: var(--text);
    cursor: default;
    user-select: none;
  }
  .tl-row:hover {
    background: var(--fill-1);
  }
  .tl-row.sel {
    background: var(--accent-soft);
  }
  .tl-row.drop {
    box-shadow: inset 0 2px 0 var(--accent);
  }
  .tl-row.cur .t1 {
    color: var(--accent-hi);
  }
  .c-num {
    position: relative;
    display: grid;
    place-items: center;
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .rowplay {
    all: unset;
    position: absolute;
    inset: 0;
    display: none;
    place-items: center;
    color: var(--text);
    cursor: pointer;
  }
  .tl-row:hover .rowplay {
    display: grid;
  }
  .tl-row:hover .num {
    visibility: hidden;
  }
  .c-title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    text-align: left;
  }
  .tt {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .t1,
  .t2 {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .t1 {
    font-weight: 550;
  }
  .t2 {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .tl-row > span:not(.c-num):not(.c-title):not(.c-like) {
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .c-dur {
    text-align: right;
    font-variant-numeric: tabular-nums;
    justify-self: end;
  }
  .like {
    all: unset;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    color: var(--text-muted);
    cursor: pointer;
    opacity: 0;
  }
  .tl-row:hover .like,
  .like.on,
  .like:focus-visible {
    opacity: 1;
  }
  .like.on {
    color: #ff4d6d;
  }
  .like:focus-visible {
    outline: var(--focus-ring);
  }
  .keychip {
    padding: 1px 6px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
  }
  .q.warn {
    color: var(--warning);
  }
  .tl-empty {
    margin: var(--space-7) auto;
    text-align: center;
    color: var(--text-dim);
  }
  .eqbars {
    display: inline-flex;
    align-items: flex-end;
    gap: 2px;
    height: 14px;
  }
  .eqbars i {
    width: 3px;
    background: var(--accent);
    border-radius: 1px;
    animation: eqb 0.9s ease-in-out infinite;
  }
  .eqbars i:nth-child(2) {
    animation-delay: -0.3s;
  }
  .eqbars i:nth-child(3) {
    animation-delay: -0.6s;
  }
  .eqbars.paused i {
    animation-play-state: paused;
    height: 4px;
  }
  @keyframes eqb {
    0%,
    100% {
      height: 4px;
    }
    50% {
      height: 14px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .eqbars i {
      animation: none;
      height: 8px;
    }
  }
</style>
