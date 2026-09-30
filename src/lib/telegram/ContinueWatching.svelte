<script lang="ts">
  // "Continue watching" row above the chat browser.
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import TgIcon from "./TgIcon.svelte";
  import { formatDuration, type PlaybackEntry } from "./api";
  import { chatRef, loadRecent, recentState, thumbnail } from "./session.svelte";

  let { onopen }: { onopen: (e: PlaybackEntry) => void } = $props();

  let rows = $derived(recentState.items.filter((r) => !r.finished && r.position_sec > 5).slice(0, 12));

  onMount(() => {
    if (!recentState.loaded) void loadRecent();
  });

  function art(node: HTMLElement, r: PlaybackEntry) {
    if (r.media_type === "audio" || r.media_type === "voice") return;
    void thumbnail(chatRef(r.chat_id, r.chat_type, r.chat_title), r.message_id).then((url) => {
      if (url) {
        node.style.backgroundImage = `url("${url}")`;
        node.classList.add("has-thumb");
      }
    });
  }
</script>

{#if rows.length}
  <section class="cw" aria-label={$t("telegram.player.continue")}>
    <h4>{$t("telegram.player.continue")}</h4>
    <div class="cw-row">
      {#each rows as r (`${r.chat_type}:${r.chat_id}:${r.message_id}`)}
        <button class="cw-card" onclick={() => onopen(r)} title={r.file_name ?? ""}>
          <span class="cw-thumb" use:art={r}>
            <span class="cw-play"><TgIcon name="play" size={14} /></span>
            <span class="cw-left">{formatDuration(Math.max(0, r.duration_sec - r.position_sec))}</span>
            <span class="cw-bar"><span style:width="{r.duration_sec ? (r.position_sec / r.duration_sec) * 100 : 0}%"></span></span>
          </span>
          <span class="cw-name">{r.file_name || `#${r.message_id}`}</span>
          <span class="cw-chat">{r.chat_title || chatRef(r.chat_id, r.chat_type).title}</span>
        </button>
      {/each}
    </div>
  </section>
{/if}

<style>
  .cw {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  h4 {
    margin: 0;
    font-size: var(--text-sm);
    font-weight: 650;
    color: var(--text);
  }

  .cw-row {
    display: flex;
    gap: var(--space-3);
    overflow-x: auto;
    padding-bottom: 4px;
    scroll-snap-type: x proximity;
  }

  .cw-card {
    flex: 0 0 184px;
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 0;
    border: none;
    background: none;
    font: inherit;
    text-align: left;
    color: var(--text);
    cursor: pointer;
    scroll-snap-align: start;
  }

  .cw-thumb {
    position: relative;
    aspect-ratio: 16 / 9;
    border-radius: var(--radius-md);
    overflow: hidden;
    background: linear-gradient(160deg, #2c313b, #15171c) center / cover no-repeat;
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    transition: transform var(--duration-fast) var(--ease-out);
  }

  .cw-card:hover .cw-thumb {
    transform: translateY(-1px);
  }

  .cw-card:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
    border-radius: var(--radius-md);
  }

  .cw-play {
    position: absolute;
    inset: 0;
    margin: auto;
    width: 32px;
    height: 32px;
    border-radius: var(--radius-full);
    display: grid;
    place-items: center;
    color: #fff;
    background: rgba(0, 0, 0, 0.45);
    backdrop-filter: blur(6px);
  }

  .cw-left {
    position: absolute;
    right: 6px;
    bottom: 8px;
    padding: 1px 5px;
    border-radius: 4px;
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: #fff;
    background: rgba(0, 0, 0, 0.6);
  }

  .cw-bar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 3px;
    background: rgba(255, 255, 255, 0.25);
  }

  .cw-bar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }

  .cw-name,
  .cw-chat {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .cw-name {
    font-size: var(--text-sm);
  }

  .cw-chat {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
</style>
