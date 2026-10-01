<script lang="ts">
  import { t } from "$lib/i18n";
  import { fmtTime } from "$lib/video/format";
  import { openPlayer, player } from "$lib/video/player.svelte";
  import type { MediaItem } from "$lib/video/types";
  import VIcon from "./VIcon.svelte";

  let currentId = $derived(player.info?.source.kind === "local" ? player.info.source.id : null);
  let list: HTMLElement | undefined = $state();

  $effect(() => {
    void currentId;
    void player.course;
    void player.folder.length;
    queueMicrotask(() => list?.querySelector<HTMLElement>(".cur")?.scrollIntoView({ block: "center" }));
  });

  function stateOf(m: MediaItem): "done" | "current" | "pending" {
    if (m.id === currentId) return "current";
    return m.progress?.watched ? "done" : "pending";
  }

  function play(m: MediaItem) {
    if (m.id !== currentId) void openPlayer({ kind: "local", id: m.id }, { title: m.title });
  }
</script>

<div class="lessons" bind:this={list}>
  {#if player.course}
    {#each player.course.modules as mod (mod.index)}
      <details open={mod.lessons.some((l) => l.id === currentId) || player.course.modules.length < 4}>
        <summary>
          <VIcon name="chevron" size={13} class="chev" />
          <span class="mt">{mod.title}</span>
          <span class="mc">{mod.watched}/{mod.lessons.length}</span>
        </summary>
        <ol>
          {#each mod.lessons as l (l.id)}
            {@const st = stateOf(l)}
            <li>
              <button class="row {st}" class:cur={st === "current"} onclick={() => play(l)} aria-current={st === "current" ? "step" : undefined}>
                <span class="dot">
                  {#if st === "done"}<VIcon name="check" size={11} />{:else if st === "current"}<VIcon name={player.playing ? "wave" : "play"} size={11} />{/if}
                </span>
                <span class="num">{l.number}</span>
                <span class="tt">{l.title}</span>
                <span class="du">{l.duration_ms ? fmtTime(l.duration_ms) : ""}</span>
              </button>
            </li>
          {/each}
        </ol>
      </details>
    {/each}
  {:else if player.folder.length}
    <ol class="flat">
      {#each player.folder as m, i (m.id)}
        {@const st = stateOf(m)}
        <li>
          <button class="row {st}" class:cur={st === "current"} onclick={() => play(m)} aria-current={st === "current" ? "step" : undefined}>
            <span class="dot">
              {#if st === "done"}<VIcon name="check" size={11} />{:else if st === "current"}<VIcon name={player.playing ? "wave" : "play"} size={11} />{/if}
            </span>
            <span class="num">{i + 1}</span>
            <span class="tt">{m.title}</span>
            <span class="du">{m.duration_ms ? fmtTime(m.duration_ms) : ""}</span>
          </button>
        </li>
      {/each}
    </ol>
  {:else}
    <p class="empty">{$t("video.panel.no_queue")}</p>
    {#if player.info?.next || player.info?.prev}
      <div class="np">
        {#if player.info?.prev}<button class="btn btn-sm" onclick={() => player.info?.prev && openPlayer(player.info.prev)}><VIcon name="prev" size={12} /> {$t("video.player.prev")}</button>{/if}
        {#if player.info?.next}<button class="btn btn-sm" onclick={() => player.info?.next && openPlayer(player.info.next)}>{$t("video.player.next")} <VIcon name="next" size={12} /></button>{/if}
      </div>
    {/if}
  {/if}
</div>

<style>
  .lessons {
    height: 100%;
    overflow-y: auto;
    padding: 4px 8px 16px;
  }
  details {
    margin-bottom: 4px;
  }
  summary {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 6px;
    border-radius: var(--radius-sm);
    list-style: none;
    cursor: pointer;
    font-size: var(--text-sm);
    font-weight: 650;
  }
  summary::-webkit-details-marker {
    display: none;
  }
  summary:hover {
    background: var(--fill-1);
  }
  summary :global(.chev) {
    transition: transform var(--duration-fast);
    transform: rotate(-90deg);
    color: var(--text-muted);
  }
  details[open] > summary :global(.chev) {
    transform: none;
  }
  .mt {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mc {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  ol {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .row {
    all: unset;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 32px;
    padding: 4px 6px;
    border-radius: var(--radius-sm);
    font-size: var(--text-sm);
    color: var(--text-dim);
    cursor: pointer;
  }
  .row:hover {
    background: var(--fill-1);
    color: var(--text);
  }
  .row:focus-visible {
    outline: var(--focus-ring);
  }
  .row.done {
    color: var(--text-muted);
  }
  .row.current {
    background: var(--accent-soft);
    color: var(--text);
    font-weight: 600;
  }
  .dot {
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    flex-shrink: 0;
    border-radius: 5px;
    box-shadow: inset 0 0 0 1.5px var(--fill-3);
  }
  .done .dot {
    box-shadow: none;
    background: color-mix(in srgb, var(--success) 16%, transparent);
    color: var(--success);
  }
  .current .dot {
    box-shadow: none;
    background: var(--accent);
    color: var(--on-accent);
  }
  .num {
    min-width: 30px;
    font-variant-numeric: tabular-nums;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .tt {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .du {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .empty {
    padding: 16px 8px;
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .np {
    display: flex;
    gap: 8px;
    padding: 0 8px;
  }
</style>
