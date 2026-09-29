<script lang="ts">
  // Everything played, newest first, grouped by day. Searchable; clearing asks
  // inside the page. "Play again" starts from the beginning instead of resuming.
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import * as api from "$lib/video/api";
  import { openPlayer } from "$lib/video/player.svelte";
  import type { HistoryEntry } from "$lib/video/types";
  import Empty from "./parts/Empty.svelte";
  import Head from "./parts/Head.svelte";
  import Skel from "./parts/Skel.svelte";
  import VIcon from "./parts/VIcon.svelte";
  import { errText, fmtClock, fmtDay, fmtHour, lsGet, lsSet, ready } from "./parts/util";

  let list = $state<HistoryEntry[] | null>(null);
  let error = $state<string | null>(null);
  let q = $state("");
  let fromStart = $state(lsGet<boolean>("omniget.video.history.from_start", false));
  let confirming = $state(false);

  async function load() {
    try {
      await ready();
      list = await api.history(1000);
      error = null;
    } catch (e) {
      error = errText(e);
      list ??= [];
    }
  }

  onMount(() => void load());
  $effect(() => lsSet("omniget.video.history.from_start", fromStart));

  let groups = $derived.by(() => {
    const needle = q.trim().toLowerCase();
    const out: { day: string; items: HistoryEntry[] }[] = [];
    for (const h of list ?? []) {
      if (needle && !`${h.title} ${h.subtitle}`.toLowerCase().includes(needle)) continue;
      const day = fmtDay(h.at);
      const last = out[out.length - 1];
      if (last && last.day === day) last.items.push(h);
      else out.push({ day, items: [h] });
    }
    return out;
  });

  function dayLabel(d: string): string {
    if (d === "today") return $t("video.history.today") as string;
    if (d === "yesterday") return $t("video.history.yesterday") as string;
    return d;
  }

  function sourceIcon(h: HistoryEntry): string {
    switch (h.source.kind) {
      case "iptv":
        return "tv";
      case "jellyfin":
        return "server";
      case "url":
        return "link";
      case "addon":
        return "grid";
      default:
        return "film";
    }
  }

  function playAgain(h: HistoryEntry) {
    void openPlayer(h.source, fromStart ? { title: h.title, startMs: 0 } : { title: h.title });
  }

  async function clearAll() {
    try {
      await api.historyClear();
      list = [];
      confirming = false;
      showToast("success", $t("video.history.cleared") as string);
    } catch (e) {
      showToast("error", errText(e));
    }
  }
</script>

<div class="page">
  <Head title={$t("video.history.title") as string}>
    {#snippet actions()}
      {#if list?.length}
        <label class="fs">
          <button class="toggle toggle-sm" role="switch" aria-checked={fromStart} aria-label={$t("video.history.from_start") as string} onclick={() => (fromStart = !fromStart)}><span class="toggle-knob"></span></button>
          <span>{$t("video.history.from_start")}</span>
        </label>
        <button class="btn btn-sm" onclick={() => (confirming = true)}><VIcon name="trash" size={13} /> {$t("video.history.clear")}</button>
      {/if}
    {/snippet}
  </Head>

  {#if confirming}
    <div class="notice notice-warning">
      <VIcon name="alert" size={16} />
      <div class="notice-body">
        <span class="notice-title">{$t("video.history.clear_confirm_title")}</span>
        <span class="notice-text">{$t("video.history.clear_confirm_body")}</span>
        <div class="cf">
          <button class="btn btn-sm btn-destructive" onclick={() => void clearAll()}>{$t("video.history.clear")}</button>
          <button class="btn btn-sm" onclick={() => (confirming = false)}>{$t("video.common.cancel")}</button>
        </div>
      </div>
    </div>
  {/if}

  {#if list?.length}
    <input class="input input-search search" type="search" placeholder={$t("video.history.search") as string} bind:value={q} />
  {/if}

  {#if error}
    <div class="notice notice-danger"><VIcon name="alert" size={16} /><div class="notice-body">{error}</div></div>
  {/if}

  {#if !list}
    <Skel kind="list" count={10} />
  {:else if !list.length}
    <Empty icon="history" title={$t("video.history.empty_title") as string} body={$t("video.history.empty_body") as string} />
  {:else if !groups.length}
    <Empty compact icon="history" title={$t("video.common.no_match") as string} />
  {:else}
    {#each groups as g (g.day)}
      <section>
        <h2>{dayLabel(g.day)}</h2>
        <ul role="list">
          {#each g.items as h, i (`${h.key}-${h.at}-${i}`)}
            {@const pct = h.duration_ms ? Math.min(1, h.position_ms / h.duration_ms) : 0}
            <li>
              <button class="row" onclick={() => playAgain(h)}>
                <span class="ic"><VIcon name={sourceIcon(h)} size={16} /></span>
                <span class="tx">
                  <b>{h.title}</b>
                  <small>{[h.subtitle, fmtHour(h.at)].filter(Boolean).join(" · ")}</small>
                </span>
                {#if h.duration_ms}
                  <span class="pos">
                    <span class="progress"><span class="progress-fill" style:width="{pct * 100}%"></span></span>
                    <small>{fmtClock(h.position_ms)} / {fmtClock(h.duration_ms)}</small>
                  </span>
                {/if}
                <span class="go" aria-hidden="true"><VIcon name={fromStart ? "replay" : "play"} size={13} /></span>
              </button>
            </li>
          {/each}
        </ul>
      </section>
    {/each}
  {/if}
</div>

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding-bottom: var(--space-7);
  }
  .fs {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .cf {
    display: flex;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
  .search {
    max-width: 360px;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  h2 {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
    font-weight: 650;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: var(--track-caps);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .row {
    all: unset;
    box-sizing: border-box;
    width: 100%;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 8px 10px;
    border-radius: var(--radius-md);
    cursor: pointer;
  }
  .row:hover {
    background: var(--fill-1);
  }
  .row:focus-visible {
    outline: var(--focus-ring);
  }
  .ic {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    color: var(--text-muted);
  }
  .tx {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .tx b {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tx small {
    color: var(--text-dim);
    font-size: var(--text-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pos {
    width: 140px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    align-items: flex-end;
  }
  .pos small {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .go {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--on-accent);
    opacity: 0;
    transition: opacity var(--duration-fast);
  }
  .row:hover .go,
  .row:focus-visible .go {
    opacity: 1;
  }
</style>
