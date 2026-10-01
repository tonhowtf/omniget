<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TgIcon from "./TgIcon.svelte";
  import WatchRuleEditor from "./WatchRuleEditor.svelte";
  import ScanList from "./ScanList.svelte";
  import { tg2, tg3, addedCount, formatBytes, type DedupeStats, type WatchRule } from "./api";
  import { failText, loadRealtime, loadWatches, realtimeState, watchState } from "./session.svelte";

  let editing = $state<WatchRule | null>(null);
  let creating = $state(false);
  let running = $state<Record<string, boolean>>({});
  let removing = $state<WatchRule | null>(null);
  let interval = $state<number | null>(null);
  let stats = $state<DedupeStats | null>(null);
  let adding = $state<Record<string, boolean>>({});
  const INTERVALS = [1, 5, 15, 30, 60];

  let realtime = $derived(realtimeState.state);

  onMount(() => {
    void loadWatches();
    tg2
      .watchSettings()
      .then((r) => (interval = r?.interval_min ?? null))
      .catch(() => {});
    void loadRealtime();
    void loadStats();
  });

  async function loadStats() {
    try {
      stats = await tg3.dedupeStats();
    } catch {
      stats = null;
    }
  }

  $effect(() => {
    if (Object.keys(watchState.lastRun).length) void loadStats();
  });

  async function setEvery(min: number) {
    const prev = interval;
    interval = min;
    try {
      const r = await tg2.watchSettings(min);
      interval = r?.interval_min ?? min;
    } catch (e) {
      interval = prev;
      showToast("error", failText(e));
    }
  }

  async function setRealtime(on: boolean) {
    const prev = realtimeState.state;
    if (prev) realtimeState.state = { ...prev, enabled: on };
    try {
      realtimeState.state = await tg3.realtimeSet(on);
    } catch (e) {
      realtimeState.state = prev;
      showToast("error", failText(e));
    }
  }

  async function setDedupe(on: boolean) {
    const prev = stats;
    if (prev) stats = { ...prev, enabled: on };
    try {
      stats = await tg3.dedupeSet(on);
    } catch (e) {
      stats = prev;
      showToast("error", failText(e));
    }
  }

  function triggerText(trigger: string | null | undefined): string {
    return trigger && ["realtime", "interval", "manual", "backfill"].includes(trigger) ? ($t(`telegram.watch.trigger.${trigger}`) as string) : "";
  }

  async function addFolderToWatch(r: WatchRule) {
    if (!r.outputDir) return;
    adding[r.id] = true;
    try {
      const n = addedCount(await tg3.addToWatch({ folder: r.outputDir }));
      showToast("success", (n == null ? $t("telegram.watch_lib.added_folder") : $t("telegram.watch_lib.added", { count: n })) as string);
    } catch (e) {
      showToast("error", failText(e));
    } finally {
      adding[r.id] = false;
    }
  }

  async function toggle(r: WatchRule) {
    const next = !r.enabled;
    r.enabled = next;
    try {
      await tg2.watchUpdate(r.id, { enabled: next });
    } catch (e) {
      r.enabled = !next;
      showToast("error", failText(e));
    }
  }

  async function run(r?: WatchRule) {
    const key = r?.id ?? "*";
    running[key] = true;
    try {
      await tg2.watchRun(r?.id);
      showToast("info", $t("telegram.watch.running") as string);
    } catch (e) {
      showToast("error", failText(e));
    } finally {
      running[key] = false;
      void loadWatches();
    }
  }

  async function remove(r: WatchRule) {
    removing = null;
    try {
      await tg2.watchRemove(r.id);
      watchState.rules = watchState.rules.filter((x) => x.id !== r.id);
    } catch (e) {
      showToast("error", failText(e));
    }
  }

  function ago(unix: number | null | undefined): string {
    if (!unix) return $t("telegram.watch.never") as string;
    const s = Math.max(0, Math.floor(Date.now() / 1000 - unix));
    if (s < 60) return $t("telegram.watch.just_now") as string;
    if (s < 3600) return $t("telegram.watch.minutes_ago", { count: Math.floor(s / 60) }) as string;
    if (s < 86400) return $t("telegram.watch.hours_ago", { count: Math.floor(s / 3600) }) as string;
    return new Date(unix * 1000).toLocaleDateString();
  }

  function filtersText(r: WatchRule): string {
    const parts: string[] = [];
    if (r.minSize) parts.push(`≥ ${formatBytes(r.minSize)}`);
    if (r.maxSize) parts.push(`≤ ${formatBytes(r.maxSize)}`);
    if (r.keyword) parts.push(`“${r.keyword}”`);
    if (r.extInclude?.length) parts.push(r.extInclude.map((x) => `.${x}`).join(" "));
    if (r.extExclude?.length) parts.push(`− ${r.extExclude.map((x) => `.${x}`).join(" ")}`);
    return parts.join(" · ");
  }
</script>

<section class="watch">
  <header class="w-head">
    <div>
      <h3>{$t("telegram.watch.title")}</h3>
      <p>{$t("telegram.watch.hint")}</p>
    </div>
    <div class="btn-row">
      {#if watchState.rules.length}
        <button class="btn btn-secondary btn-sm" onclick={() => run()} disabled={running["*"]}>
          {#if running["*"]}<span class="spinner"></span>{:else}<TgIcon name="refresh" />{/if}
          {$t("telegram.watch.run_all")}
        </button>
      {/if}
      <button class="btn btn-primary btn-sm" onclick={() => (creating = true)}><TgIcon name="plus" /> {$t("telegram.watch.new")}</button>
    </div>
  </header>

  <ScanList />

  {#if watchState.error}
    <div class="notice notice-warning">
      <TgIcon name="warn" size={16} />
      <div class="notice-body"><div class="notice-text">{watchState.error}</div></div>
      <button class="btn btn-ghost btn-sm" onclick={() => void loadWatches()}>{$t("telegram.common.retry")}</button>
    </div>
  {:else if !watchState.loaded}
    <p class="dim"><span class="spinner"></span> {$t("telegram.common.loading")}</p>
  {:else if !watchState.rules.length}
    <div class="empty-state">
      <div class="empty-icon"><TgIcon name="bolt" size={28} /></div>
      <div class="empty-state-title">{$t("telegram.watch.empty_title")}</div>
      <div class="empty-state-hint">{$t("telegram.watch.empty_hint")}</div>
      <button class="btn btn-primary" onclick={() => (creating = true)}><TgIcon name="plus" /> {$t("telegram.watch.new")}</button>
    </div>
  {:else}
    <ul class="rules">
      {#each watchState.rules as r (r.id)}
        {@const last = watchState.lastRun[r.id]}
        <li class="rule" class:off={!r.enabled}>
          <button class="toggle" role="switch" aria-checked={r.enabled} aria-label={$t("telegram.watch.enabled")} onclick={() => toggle(r)}><span class="toggle-knob"></span></button>
          <div class="r-body">
            <div class="r-title">{r.chatTitle}</div>
            <div class="r-tags">
              {#each r.mediaTypes.length ? r.mediaTypes : ["all"] as k (k)}
                <span class="tag">{k === "all" ? $t("telegram.browser.filter.all") : $t(`telegram.filters.type.${k}`)}</span>
              {/each}
              {#if filtersText(r)}<span class="r-filters">{filtersText(r)}</span>{/if}
            </div>
            <div class="r-meta">
              <span>{$t("telegram.watch.saved_count", { count: r.savedCount })}</span>
              {#if r.duplicateCount}<span>·</span><span>{$t("telegram.dedupe.rule_count", { count: r.duplicateCount })}</span>{/if}
              {#if r.dedupe === false}<span>·</span><span>{$t("telegram.dedupe.rule_off")}</span>{/if}
              {#if r.addToWatch}<span>·</span><span>{$t("telegram.watch_lib.rule_on")}</span>{/if}
              <span>·</span>
              <span>{$t("telegram.watch.last_run", { when: ago(r.lastRunAt) })}</span>
              {#if last}
                <span>·</span>
                <span>{$t("telegram.watch.last_result", { found: last.found, queued: last.queued })}{last.duplicates ? ` · ${$t("telegram.dedupe.run_count", { count: last.duplicates })}` : ""}</span>
                {#if triggerText(last.trigger)}<span class="tag">{triggerText(last.trigger)}</span>{/if}
              {:else if r.lastResult}
                <span>·</span><span>{r.lastResult}</span>
              {/if}
            </div>
            {#if r.lastError || last?.error}<div class="field-error">{last?.error ?? r.lastError}</div>{/if}
            {#if r.outputDir}
              <button class="r-dir" onclick={() => void invoke("reveal_file", { path: r.outputDir }).catch(() => {})} title={r.outputDir}>
                <TgIcon name="folder" size={12} /> <span>{r.outputDir}</span>
              </button>
            {/if}
          </div>
          <div class="r-actions">
            <button class="btn btn-secondary btn-sm" onclick={() => run(r)} disabled={running[r.id]}>
              {#if running[r.id]}<span class="spinner"></span>{:else}<TgIcon name="play" />{/if}
              {$t("telegram.watch.run_now")}
            </button>
            {#if r.outputDir}
              <button class="btn btn-ghost btn-sm" onclick={() => addFolderToWatch(r)} disabled={adding[r.id]} title={$t("telegram.watch_lib.add_folder")} aria-label={$t("telegram.watch_lib.add_folder")}>
                {#if adding[r.id]}<span class="spinner"></span>{:else}<TgIcon name="tv" />{/if}
              </button>
            {/if}
            <button class="btn btn-ghost btn-sm" onclick={() => (editing = r)} title={$t("telegram.watch.edit")} aria-label={$t("telegram.watch.edit")}><TgIcon name="edit" /></button>
            {#if removing?.id === r.id}
              <button class="btn btn-destructive btn-sm" onclick={() => remove(r)}>{$t("telegram.watch.confirm_remove")}</button>
            {:else}
              <button class="btn btn-ghost btn-sm" onclick={() => (removing = r)} title={$t("telegram.watch.remove")} aria-label={$t("telegram.watch.remove")}><TgIcon name="trash" /></button>
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  {/if}
    <div class="group">
      {#if realtime}
        <div class="group-row">
          <div class="group-row-content">
            <span class="group-row-title">
              {$t("telegram.watch.realtime")}
              {#if realtime.enabled}
                <span class="live" class:on={realtime.connected}>{realtime.connected ? $t("telegram.watch.realtime_live") : $t("telegram.watch.realtime_waiting")}</span>
              {/if}
            </span>
            <span class="group-row-sub">
              {#if realtime.enabled && realtime.last_event_at}
                {$t("telegram.watch.realtime_last", { when: ago(realtime.last_event_at) })}
              {:else}
                {realtime.enabled ? $t("telegram.watch.realtime_on_hint") : $t("telegram.watch.realtime_off_hint")}
              {/if}
            </span>
          </div>
          <button class="toggle" role="switch" aria-checked={realtime.enabled} aria-label={$t("telegram.watch.realtime")} onclick={() => realtime && setRealtime(!realtime.enabled)}><span class="toggle-knob"></span></button>
        </div>
      {/if}
      {#if interval !== null}
        <div class="group-row">
          <div class="group-row-content">
            <span class="group-row-title">{$t("telegram.watch.every")}</span>
            <span class="group-row-sub">{realtime?.enabled ? $t("telegram.watch.every_backup_hint") : $t("telegram.watch.background_hint")}</span>
          </div>
          <select class="input every" value={interval} aria-label={$t("telegram.watch.every")} onchange={(e) => setEvery(Number((e.currentTarget as HTMLSelectElement).value))}>
            {#each INTERVALS.includes(interval) ? INTERVALS : [...INTERVALS, interval].sort((a, b) => a - b) as m (m)}
              <option value={m}>{$t("telegram.watch.minutes", { count: m })}</option>
            {/each}
          </select>
        </div>
      {/if}
      {#if stats}
        <div class="group-row">
          <div class="group-row-content">
            <span class="group-row-title">{$t("telegram.dedupe.global")}</span>
            <span class="group-row-sub">
              {#if stats.avoided > 0}
                {stats.avoided_bytes > 0
                  ? $t("telegram.dedupe.stats_bytes", { count: stats.avoided, size: formatBytes(stats.avoided_bytes) })
                  : $t("telegram.dedupe.stats", { count: stats.avoided })}
              {:else}
                {$t("telegram.dedupe.global_hint")}
              {/if}
            </span>
          </div>
          <button class="toggle" role="switch" aria-checked={stats.enabled} aria-label={$t("telegram.dedupe.global")} onclick={() => stats && setDedupe(!stats.enabled)}><span class="toggle-knob"></span></button>
        </div>
      {/if}
    </div>
    {#if !realtime && interval === null}
      <p class="foot">{$t("telegram.watch.background_hint")}</p>
    {/if}
</section>

{#if creating}
  <WatchRuleEditor onclose={() => (creating = false)} />
{/if}
{#if editing}
  <WatchRuleEditor rule={editing} onclose={() => (editing = null)} />
{/if}

<style>
  .watch {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 820px;
  }

  .w-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .w-head h3 {
    margin: 0 0 2px;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .w-head p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    line-height: 1.5;
    max-width: 520px;
  }

  .empty-icon {
    width: 56px;
    height: 56px;
    border-radius: var(--radius-xl);
    display: grid;
    place-items: center;
    background: var(--accent-soft);
    color: var(--accent-hi);
  }

  .rules {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .rule {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    transition: opacity var(--duration-base) var(--ease-out);
  }

  .rule.off .r-body {
    opacity: 0.55;
  }

  .rule .toggle {
    margin-top: 2px;
  }

  .r-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .r-title {
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .r-tags {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
  }

  .r-filters,
  .r-meta {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .r-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    font-variant-numeric: tabular-nums;
  }

  .r-dir {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 100%;
    padding: 0;
    border: none;
    background: none;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--text-xs);
    cursor: pointer;
    align-self: flex-start;
  }

  .r-dir span {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    direction: rtl;
  }

  .r-dir:hover {
    color: var(--text);
  }

  .r-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }

  .dim,
  .foot {
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .foot {
    margin: 0;
    font-size: var(--text-xs);
  }

  .every {
    width: auto;
    flex-shrink: 0;
  }

  .live {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-left: 6px;
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--text-dim);
  }

  .live::before {
    content: "";
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--text-dim);
  }

  .live.on {
    color: var(--success);
  }

  .live.on::before {
    background: var(--success);
  }

  @media (max-width: 640px) {
    .rule {
      flex-wrap: wrap;
    }
  }
</style>
