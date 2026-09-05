<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import {
    studyYoutubeMetricsGet,
    studyYoutubeMetricsReset,
    studyYoutubePotokenStatus,
    studyMusicLoopCookieStatus,
    type YoutubeEndpointMetrics,
    type LoopCookieStatus,
  } from "$lib/study-bridge";

  let metrics = $state<YoutubeEndpointMetrics[]>([]);
  let metricsLoading = $state(false);
  let metricsError = $state<string | null>(null);
  let resetBusy = $state(false);

  type PotokenStatus = {
    engine?: string;
    snapshot_version?: string;
    minting_available?: boolean;
    minting_status?: string;
    visitor?: {
      has_token: boolean;
      expires_in_seconds: number | null;
      expired: boolean | null;
    };
    content_cached_count?: number;
    last_generated_at?: number | null;
  };

  let potoken = $state<PotokenStatus | null>(null);
  let potokenLoading = $state(false);
  let potokenError = $state<string | null>(null);

  let loop = $state<LoopCookieStatus | null>(null);
  let loopLoading = $state(false);
  let loopError = $state<string | null>(null);

  async function loadMetrics() {
    metricsLoading = true;
    metricsError = null;
    try {
      const res = await studyYoutubeMetricsGet();
      metrics = res.metrics ?? [];
    } catch (e) {
      metricsError = e instanceof Error ? e.message : String(e);
    } finally {
      metricsLoading = false;
    }
  }

  async function resetMetrics() {
    if (resetBusy) return;
    resetBusy = true;
    try {
      await studyYoutubeMetricsReset();
      await loadMetrics();
    } catch (e) {
      metricsError = e instanceof Error ? e.message : String(e);
    } finally {
      resetBusy = false;
    }
  }

  async function loadPotoken() {
    potokenLoading = true;
    potokenError = null;
    try {
      potoken = (await studyYoutubePotokenStatus()) as PotokenStatus;
    } catch (e) {
      potokenError = e instanceof Error ? e.message : String(e);
    } finally {
      potokenLoading = false;
    }
  }

  async function loadLoop() {
    loopLoading = true;
    loopError = null;
    try {
      loop = await studyMusicLoopCookieStatus();
    } catch (e) {
      loopError = e instanceof Error ? e.message : String(e);
    } finally {
      loopLoading = false;
    }
  }

  function formatDateTs(ts: number | null | undefined): string {
    if (!ts) return "—";
    try {
      const d = new Date(ts * 1000);
      return d.toLocaleString();
    } catch {
      return String(ts);
    }
  }

  onMount(() => {
    void loadMetrics();
    void loadPotoken();
    void loadLoop();
  });
</script>

<div class="diag">
  <section class="card">
    <header class="card-head">
      <h2>{$t("study.music.diag_metrics_title")}</h2>
      <div class="actions">
        <button type="button" class="ghost" onclick={loadMetrics} disabled={metricsLoading}>
          {$t("study.music.diag_refresh")}
        </button>
        <button type="button" class="danger" onclick={resetMetrics} disabled={resetBusy || metrics.length === 0}>
          {$t("study.music.diag_metrics_reset")}
        </button>
      </div>
    </header>
    {#if metricsLoading && metrics.length === 0}
      <p class="muted">{$t("study.music.diag_loading")}</p>
    {:else if metricsError}
      <p class="error">{metricsError}</p>
    {:else if metrics.length === 0}
      <p class="muted">{$t("study.music.diag_metrics_empty")}</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>{$t("study.music.diag_metrics_endpoint")}</th>
            <th class="num">{$t("study.music.diag_metrics_success")}</th>
            <th class="num">{$t("study.music.diag_metrics_fail")}</th>
            <th>{$t("study.music.diag_metrics_last_error")}</th>
          </tr>
        </thead>
        <tbody>
          {#each metrics as m (m.endpoint)}
            <tr>
              <td>{m.endpoint}</td>
              <td class="num">{m.success_count}</td>
              <td class="num" class:has-fails={m.fail_count > 0}>{m.fail_count}</td>
              <td class="error-cell">{m.last_error ?? "—"}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>

  <section class="card">
    <header class="card-head">
      <h2>{$t("study.music.diag_potoken_title")}</h2>
      <div class="actions">
        <button type="button" class="ghost" onclick={loadPotoken} disabled={potokenLoading}>
          {$t("study.music.diag_refresh")}
        </button>
      </div>
    </header>
    {#if potokenLoading && !potoken}
      <p class="muted">{$t("study.music.diag_loading")}</p>
    {:else if potokenError}
      <p class="error">{potokenError}</p>
    {:else if potoken}
      <dl>
        <dt>{$t("study.music.diag_potoken_engine")}</dt>
        <dd>{potoken.engine ?? "—"}</dd>
        <dt>{$t("study.music.diag_potoken_snapshot")}</dt>
        <dd>{potoken.snapshot_version ?? "—"}</dd>
        <dt>{$t("study.music.diag_potoken_minting")}</dt>
        <dd>
          {#if potoken.minting_available}
            <span class="ok">{$t("study.music.diag_potoken_minting_active")}</span>
          {:else}
            <span class="pending">{$t("study.music.diag_potoken_minting_pending")}</span>
          {/if}
        </dd>
        <dt>{$t("study.music.diag_potoken_visitor")}</dt>
        <dd>
          {#if potoken.visitor?.has_token}
            {#if potoken.visitor.expired}
              <span class="warn">{$t("study.music.diag_potoken_visitor_expired")}</span>
            {:else if potoken.visitor.expires_in_seconds != null}
              <span class="ok">
                {$t("study.music.diag_potoken_visitor_active", { seconds: potoken.visitor.expires_in_seconds })}
              </span>
            {:else}
              <span class="ok">{$t("study.music.diag_potoken_visitor_active_unknown")}</span>
            {/if}
          {:else}
            <span class="muted">{$t("study.music.diag_potoken_visitor_none")}</span>
          {/if}
        </dd>
        <dt>{$t("study.music.diag_potoken_cache_count")}</dt>
        <dd>{potoken.content_cached_count ?? 0}</dd>
        <dt>{$t("study.music.diag_potoken_last_generated")}</dt>
        <dd>{formatDateTs(potoken.last_generated_at)}</dd>
      </dl>
    {/if}
  </section>

  <section class="card">
    <header class="card-head">
      <h2>{$t("study.music.diag_loop_title")}</h2>
      <div class="actions">
        <button type="button" class="ghost" onclick={loadLoop} disabled={loopLoading}>
          {$t("study.music.diag_refresh")}
        </button>
      </div>
    </header>
    {#if loopLoading && !loop}
      <p class="muted">{$t("study.music.diag_loading")}</p>
    {:else if loopError}
      <p class="error">{loopError}</p>
    {:else if loop}
      {#if !loop.available}
        <p class="muted">{$t("study.music.diag_loop_missing")}</p>
        <p class="hint">{$t("study.music.diag_loop_missing_hint")}</p>
        {#if loop.file_path}
          <code class="path">{loop.file_path}</code>
        {/if}
      {:else if !loop.has_youtube}
        <p class="warn">{$t("study.music.diag_loop_no_youtube")}</p>
        <p class="hint">{$t("study.music.diag_loop_no_youtube_hint")}</p>
        {#if loop.file_path}
          <code class="path">{loop.file_path}</code>
        {/if}
      {:else}
        <p class="ok">{$t("study.music.diag_loop_ready")}</p>
        {#if loop.file_path}
          <code class="path">{loop.file_path}</code>
        {/if}
      {/if}
    {/if}
  </section>
</div>

<style>
  .diag {
    display: grid;
    gap: 16px;
  }
  .card {
    background: color-mix(in oklab, var(--button) 30%, transparent);
    border: none;
    border-radius: 12px;
    padding: 16px 20px;
    display: grid;
    gap: 10px;
  }
  .card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  .card-head h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 700;
    color: var(--secondary);
  }
  .actions {
    display: flex;
    gap: 8px;
  }
  .ghost,
  .danger {
    padding: 6px 12px;
    border-radius: 999px;
    border: none;
    background: transparent;
    color: var(--secondary);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .ghost:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }
  .danger {
    color: var(--error);
    border-color: color-mix(in oklab, var(--error) 50%, transparent);
  }
  .danger:hover:not(:disabled) {
    background: color-mix(in oklab, var(--error) 10%, transparent);
  }
  .ghost:disabled,
  .danger:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }
  th,
  td {
    padding: 8px 10px;
    border-bottom: none;
    text-align: left;
    vertical-align: top;
  }
  th {
    font-weight: 600;
    color: var(--tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-size: 11px;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .has-fails {
    color: var(--error);
  }
  .error-cell {
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--tertiary);
    font-size: 11px;
  }
  dl {
    margin: 0;
    display: grid;
    grid-template-columns: minmax(140px, max-content) 1fr;
    gap: 6px 12px;
    font-size: 13px;
  }
  dt {
    color: var(--tertiary);
    font-size: 12px;
  }
  dd {
    margin: 0;
    color: var(--secondary);
  }
  .ok {
    color: var(--success);
  }
  .warn,
  .pending {
    color: var(--warning);
  }
  .muted {
    color: var(--tertiary);
    font-size: 13px;
    margin: 0;
  }
  .error {
    color: var(--error);
    font-size: 13px;
    margin: 0;
  }
  .hint {
    color: var(--tertiary);
    font-size: 12px;
    margin: 0;
    line-height: 1.5;
  }
  .path {
    display: block;
    padding: 6px 10px;
    background: color-mix(in oklab, var(--button) 50%, transparent);
    border-radius: 6px;
    font-size: 11px;
    word-break: break-all;
    color: var(--tertiary);
  }
</style>
