<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import type { HistoryEntry, Shelf as ShelfT, VideoItem } from "$lib/tube/types";
  import type { Route } from "$lib/tube/ui.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TIcon from "../../TIcon.svelte";
  import Sentinel from "../../cards/Sentinel.svelte";
  import SkeletonGrid from "../../cards/SkeletonGrid.svelte";
  import State from "../../cards/State.svelte";
  import UndoPill from "../../cards/UndoPill.svelte";
  import VideoRow from "../../cards/VideoRow.svelte";
  import { isVisible } from "../../cards/feedback.svelte";
  import { Pager } from "../../cards/pager.svelte";
  import { ensureSession, loggedIn } from "../../cards/session.svelte";
  import { dayBucket, errText } from "../../cards/util";

  let { route: _route }: { route: Extract<Route, { name: "history" }> } = $props();

  let local = $state<HistoryEntry[] | null>(null);
  let account = $state<boolean | null>(null);
  let localError = $state<string | null>(null);
  let q = $state("");
  let confirming = $state(false);

  const pager = new Pager("history", () => api.history());

  async function load() {
    await ensureSession();
    account = loggedIn();
    if (account) {
      await pager.load();
      return;
    }
    localError = null;
    try {
      local = await api.localHistory(1000);
    } catch (e) {
      localError = errText(e);
      local ??= [];
    }
  }

  $effect(() => {
    void load();
  });

  function toItem(h: HistoryEntry): VideoItem {
    return {
      kind: "video",
      id: h.video_id,
      title: h.title,
      channel: h.channel ?? null,
      thumbs: h.thumb ? [{ url: h.thumb, width: 320, height: 180 }] : [],
      duration_s: h.duration_s ?? null,
      badges: [],
      watched_pct: h.duration_s ? Math.min(100, Math.round((h.position_s / h.duration_s) * 100)) : null,
    };
  }

  function matches(title: string, channel: string | undefined | null): boolean {
    const needle = q.trim().toLowerCase();
    return !needle || `${title} ${channel ?? ""}`.toLowerCase().includes(needle);
  }

  type Group = { key: string; label: string; items: { item: VideoItem; entry?: HistoryEntry }[] };

  let groups = $derived.by((): Group[] => {
    if (account === null) return [];
    if (!account) {
      const out: Group[] = [];
      for (const h of local ?? []) {
        if (h.kind !== "video" || !matches(h.title, h.channel?.name)) continue;
        const k = dayBucket(h.watched_ms);
        const last = out[out.length - 1];
        const row = { item: toItem(h), entry: h };
        if (last && last.key === k) last.items.push(row);
        else out.push({ key: k, label: $t(`tube.subs.day_${k}`) as string, items: [row] });
      }
      return out;
    }
    return pager.shelves.map((s: ShelfT, i) => ({
      key: s.id + i,
      label: s.title ?? "",
      items: s.items
        .filter((it): it is VideoItem => it.kind === "video" && isVisible(it) && matches(it.title, it.channel?.name))
        .map((item) => ({ item })),
    })).filter((g) => g.items.length);
  });

  let loading = $derived(account === null || (account ? pager.loading && !pager.page : local === null));
  let error = $derived(account ? pager.error && !pager.page ? pager.error : null : localError && !local?.length ? localError : null);

  async function remove(h: HistoryEntry) {
    try {
      await api.removeLocalHistory(h.video_id);
      local = (local ?? []).filter((x) => x.video_id !== h.video_id);
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function clearAll() {
    try {
      await api.clearLocalHistory();
      local = [];
      confirming = false;
      showToast("success", $t("tube.history.cleared") as string);
    } catch (e) {
      showToast("error", errText(e));
    }
  }
</script>

<div class="hist" data-agent-readiness={loading ? "loading" : error ? "error" : "ready"} data-agent-surface="omnitube-history">
  <div class="layout">
    <section class="main">
      <h1>{$t("tube.history.title")}</h1>
      {#if loading}
        <SkeletonGrid layout="rows" count={6} />
      {:else if error}
        <State icon="history" {error} onretry={load} />
      {:else if !groups.length}
        <State
          icon="history"
          title={$t(q.trim() ? "tube.history.no_match" : "tube.history.empty_title") as string}
          text={q.trim() ? "" : ($t("tube.history.empty_text") as string)}
        />
      {:else}
        {#each groups as g (g.key)}
          <section class="day">
            {#if g.label}<h2>{g.label}</h2>{/if}
            <div class="rows">
              {#each g.items as r, i (r.item.id + ":" + i)}
                <VideoRow item={r.item} size="search" feedbackActions={false} onremove={r.entry ? () => remove(r.entry!) : undefined} />
              {/each}
            </div>
          </section>
        {/each}
        {#if account && pager.hasMore}
          <Sentinel onmore={() => pager.more()} loading={pager.loadingMore} error={pager.moreError} />
        {/if}
      {/if}
    </section>

    <aside class="side">
      <label class="find">
        <TIcon name="search" size={20} />
        <input bind:value={q} type="search" placeholder={$t("tube.history.search") as string} aria-label={$t("tube.history.search") as string} />
      </label>
      {#if !account}
        {#if confirming}
          <div class="confirm" role="alertdialog" aria-label={$t("tube.history.clear") as string}>
            <p>{$t("tube.history.clear_confirm")}</p>
            <div class="row">
              <button class="yt-btn yt-btn--ghost yt-btn--s" onclick={() => (confirming = false)}>{$t("tube.common.cancel")}</button>
              <button class="yt-btn yt-btn--s danger" onclick={clearAll}>{$t("tube.history.clear")}</button>
            </div>
          </div>
        {:else}
          <button class="yt-btn yt-btn--ghost side-btn" onclick={() => (confirming = true)} disabled={!local?.length}>
            <TIcon name="trash" size={20} />{$t("tube.history.clear")}
          </button>
        {/if}
        <p class="hint">{$t("tube.history.local_hint")}</p>
      {:else}
        <p class="hint">{$t("tube.history.account_hint")}</p>
      {/if}
    </aside>
  </div>
  <UndoPill />
</div>

<style>
  .hist {
    padding: 0 var(--yt-grid-gutter) 24px;
  }
  .layout {
    display: flex;
    gap: 48px;
    max-width: 1284px;
    margin: 0 auto;
  }
  .main {
    flex: 1;
    min-width: 0;
    max-width: 860px;
  }
  h1 {
    margin: 24px 0 16px;
    font: var(--yt-type-display);
  }
  .day + .day {
    margin-top: 32px;
  }
  h2 {
    margin: 0 0 16px;
    font: var(--yt-type-headline-s);
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .side {
    position: sticky;
    top: 24px;
    align-self: flex-start;
    display: flex;
    flex-direction: column;
    gap: 12px;
    width: 300px;
    padding-top: 88px;
  }
  .find {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 4px 6px;
    border-bottom: 1px solid var(--yt-divider);
    color: var(--yt-text-2);
  }
  .find:focus-within {
    border-bottom: 2px solid var(--yt-text);
    padding-bottom: 5px;
  }
  .find input {
    flex: 1;
    min-width: 0;
    height: 32px;
    border: 0;
    font: 400 16px/22px var(--yt-font);
    color: var(--yt-text);
    background: none;
    outline: none;
  }
  .side-btn {
    justify-content: flex-start;
    gap: 16px;
  }
  .hint {
    margin: 0;
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
  }
  .confirm {
    padding: 12px;
    border-radius: 12px;
    background: var(--yt-tonal);
  }
  .confirm p {
    margin: 0 0 8px;
  }
  .confirm .row {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .danger {
    color: #fff;
    background: var(--yt-live-red);
  }
  .danger:hover {
    background: var(--yt-live-red);
    filter: brightness(1.1);
  }
  @container omnitube (max-width: 1000px) {
    .layout {
      flex-direction: column-reverse;
      gap: 0;
    }
    .side {
      position: static;
      width: 100%;
      padding-top: 16px;
    }
  }
</style>
