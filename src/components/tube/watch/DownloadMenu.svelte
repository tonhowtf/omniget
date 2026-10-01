<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { fmtBytes } from "$lib/tube/format";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import type { StreamSet } from "$lib/tube/types";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TIcon from "../TIcon.svelte";
  import Popup from "../cards/Popup.svelte";
  import { errText, streamsFor } from "../cards/util";
  import { audioOptions, videoLadder } from "./quality";

  let {
    videoId,
    title,
    anchor = null,
    onclose,
  }: { videoId: string; title: string; anchor?: HTMLElement | null; onclose: () => void } = $props();

  type SbChoice = "off" | "mark" | "remove";

  let set = $state<StreamSet | null>(null);
  let error = $state<string | null>(null);
  let sb = $state<SbChoice>(tubePrefs().sb_download);
  let busy = $state<string | null>(null);
  let done = $state<Record<string, boolean>>({});

  let ladder = $derived(set ? videoLadder(set, tubePrefs().prefer_codec) : []);
  let audios = $derived(set ? audioOptions(set).slice(0, 4) : []);

  async function load() {
    error = null;
    try {
      set = await streamsFor(videoId);
    } catch (e) {
      error = errText(e);
    }
  }
  $effect(() => {
    void videoId;
    void load();
  });

  async function start(key: string, videoItag: number | null, audioItag: number | null, audioOnly: boolean) {
    if (busy) return;
    busy = key;
    try {
      await api.download({
        video_id: videoId,
        title,
        video_itag: videoItag,
        audio_itag: audioItag,
        audio_only: audioOnly,
        sponsorblock: sb,
        music: false,
      });
      done[key] = true;
      showToast("success", $t("tube.download.queued", { name: title }) as string);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
    }
  }

  const SB: SbChoice[] = ["off", "mark", "remove"];
</script>

<Popup {anchor} {onclose} class="yt-menu dl" label={$t("tube.download.title") as string} width={320}>
  <header>
    <TIcon name="download" size={20} />
    <span>{$t("tube.download.title")}</span>
  </header>

  <button class="yt-menu-item row best" role="menuitem" disabled={!!busy} onclick={() => start("best", null, null, false)}>
    <span class="lab">{$t("tube.download.best")}</span>
    <span class="meta">{$t("tube.download.best_hint")}</span>
    {@render state("best")}
  </button>

  {#if error}
    <p class="note err">{error}</p>
    <button class="yt-btn yt-btn--s retry" onclick={load}>{$t("tube.common.retry")}</button>
  {:else if !set}
    {#each Array(4) as _, i (i)}
      <div class="skrow"><span class="yt-skel" style:width="{70 - i * 8}%"></span></div>
    {/each}
  {:else}
    <div class="yt-menu-sep"></div>
    <div class="group">{$t("tube.download.video")}</div>
    {#each ladder as q (q.key)}
      <button class="yt-menu-item row" role="menuitem" disabled={!!busy} onclick={() => start(q.key, q.itag, q.audioItag, false)}>
        <span class="lab">
          {q.label}
          {#if q.hdr}<span class="hdr">HDR</span>{/if}
          {#if q.badge}<sup class="badge">{q.badge}</sup>{/if}
        </span>
        <span class="meta">{q.codec}{q.bytes ? ` · ${fmtBytes(q.bytes)}` : ""}</span>
        {@render state(q.key)}
      </button>
    {/each}
    <div class="yt-menu-sep"></div>
    <div class="group">{$t("tube.download.audio_only")}</div>
    {#each audios as a (a.key)}
      <button class="yt-menu-item row" role="menuitem" disabled={!!busy} onclick={() => start(`a-${a.key}`, null, a.itag, true)}>
        <span class="lab">
          {a.codec === "mp4a" ? "M4A" : a.codec.toUpperCase()} · {a.kbps} kbps
        </span>
        <span class="meta">
          {#if a.name}{a.name}{#if a.original} · {$t("tube.download.original")}{/if}{/if}
          {a.bytes ? ` ${fmtBytes(a.bytes)}` : ""}
        </span>
        {@render state(`a-${a.key}`)}
      </button>
    {/each}
  {/if}

  <div class="yt-menu-sep"></div>
  <div class="group">
    <img src="/icons/tube/sponsorblock-32.webp" width="16" height="16" alt="" />
    {$t("tube.download.sponsorblock")}
  </div>
  <div class="seg" role="radiogroup" aria-label={$t("tube.download.sponsorblock") as string}>
    {#each SB as s (s)}
      <button role="menuitemradio" aria-checked={sb === s} class:on={sb === s} onclick={() => (sb = s)}>
        {$t(`tube.download.sb_${s}`)}
      </button>
    {/each}
  </div>
</Popup>

{#snippet state(key: string)}
  <span class="st" aria-hidden="true">
    {#if busy === key}
      <span class="spin"><TIcon name="loader" size={18} /></span>
    {:else if done[key]}
      <TIcon name="check" size={18} />
    {:else}
      <TIcon name="download" size={18} />
    {/if}
  </span>
  {#if busy === key}<i class="bar"></i>{/if}
{/snippet}

<style>
  header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 16px 10px;
    font: var(--yt-type-card-title);
  }
  .group {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 16px 4px;
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .row {
    position: relative;
    height: 40px;
    gap: 8px;
  }
  .row:disabled {
    cursor: progress;
  }
  .lab {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-weight: 500;
    min-width: 72px;
  }
  .best .lab {
    min-width: 0;
  }
  .meta {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--yt-text-2);
    font: var(--yt-type-caption);
  }
  .badge {
    padding: 0 3px;
    border-radius: 2px;
    font: 700 9px/12px var(--yt-font);
    color: #fff;
    background: var(--yt-live-red);
  }
  .hdr {
    font: 700 10px/12px var(--yt-font);
    color: var(--yt-text-2);
  }
  .st {
    display: inline-flex;
    color: var(--yt-text-2);
  }
  .row:hover .st {
    color: var(--yt-text);
  }
  .bar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 2px;
    background: var(--yt-accent);
    transform-origin: left;
    animation: grow 1.4s var(--yt-ease-standard) infinite;
  }
  .spin {
    display: inline-flex;
    animation: spin 1s linear infinite;
  }
  .skrow {
    height: 40px;
    display: flex;
    align-items: center;
    padding: 0 16px;
  }
  .skrow .yt-skel {
    height: 14px;
  }
  .note {
    margin: 8px 16px;
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
  }
  .note.err {
    color: var(--yt-error);
  }
  .retry {
    margin: 0 16px 8px;
  }
  .seg {
    display: flex;
    gap: 4px;
    margin: 4px 12px 8px;
    padding: 3px;
    border-radius: 999px;
    background: var(--yt-tonal);
  }
  .seg button {
    flex: 1;
    height: 30px;
    border: 0;
    border-radius: 999px;
    font: var(--yt-type-body-strong);
    font-size: 12px;
    color: var(--yt-text);
    background: transparent;
    cursor: pointer;
    transition: background var(--yt-dur-base) var(--yt-ease-standard);
  }
  .seg button.on {
    color: var(--yt-on-inverted);
    background: var(--yt-bg-inverted);
  }
  .seg button:focus-visible {
    outline: 2px solid var(--yt-link);
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @keyframes grow {
    from {
      transform: scaleX(0);
    }
    to {
      transform: scaleX(1);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spin,
    .bar {
      animation: none;
    }
  }
</style>
