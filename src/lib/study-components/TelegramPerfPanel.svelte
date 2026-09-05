<script lang="ts">
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";
  import {
    telegramPerfGet,
    telegramPerfSet,
    telegramBandwidthStats,
    telegramBandwidthSetQuota,
    telegramBandwidthReset,
    telegramSyncState,
    telegramSyncSettingsSet,
    type TelegramPerfSettings,
    type TelegramBandwidthStats,
    type TelegramSyncState,
  } from "$lib/study-telegram-bridge";

  let { open = $bindable(false) } = $props<{ open: boolean }>();

  let perf = $state<TelegramPerfSettings | null>(null);
  let loading = $state(false);
  let saving = $state(false);
  let error = $state("");
  let draftMax = $state(8);

  let bw = $state<TelegramBandwidthStats | null>(null);
  let draftQuotaGb = $state(250);
  let bwSaving = $state(false);

  let sync = $state<TelegramSyncState | null>(null);
  let draftSyncEnabled = $state(true);
  let draftSyncIntervalMin = $state(30);
  let syncSaving = $state(false);

  async function loadBandwidth() {
    try {
      bw = await telegramBandwidthStats();
      draftQuotaGb = Math.max(1, Math.round(bw.quota_bytes / (1024 * 1024 * 1024)));
    } catch {
      bw = null;
    }
  }

  async function loadSync() {
    try {
      sync = await telegramSyncState();
      draftSyncEnabled = sync.enabled;
      draftSyncIntervalMin = sync.interval_min;
    } catch {
      sync = null;
    }
  }

  async function saveSync() {
    if (syncSaving) return;
    syncSaving = true;
    try {
      await telegramSyncSettingsSet({
        enabled: draftSyncEnabled,
        intervalMin: draftSyncIntervalMin,
      });
      await loadSync();
      showToast("info", draftSyncEnabled ? `Sync a cada ${draftSyncIntervalMin} min` : "Sync desativada");
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? $t("common.error")));
    } finally {
      syncSaving = false;
    }
  }

  async function load() {
    loading = true;
    error = "";
    try {
      perf = await telegramPerfGet();
      draftMax = perf.max_threads;
      await Promise.all([loadBandwidth(), loadSync()]);
    } catch (e: any) {
      error = typeof e === "string" ? e : (e?.message ?? $t("common.error"));
    } finally {
      loading = false;
    }
  }

  function fmtBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
    return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }

  async function saveQuota() {
    if (bwSaving) return;
    bwSaving = true;
    try {
      await telegramBandwidthSetQuota({ gb: draftQuotaGb });
      await loadBandwidth();
      showToast("info", `Quota: ${draftQuotaGb} GB/dia`);
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? $t("common.error")));
    } finally {
      bwSaving = false;
    }
  }

  async function resetUsed() {
    if (bwSaving) return;
    bwSaving = true;
    try {
      await telegramBandwidthReset();
      await loadBandwidth();
      showToast("info", "Uso de hoje zerado");
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? $t("common.error")));
    } finally {
      bwSaving = false;
    }
  }

  async function save() {
    saving = true;
    try {
      const r = await telegramPerfSet({ maxThreads: draftMax });
      if (perf) perf = { ...perf, max_threads: r.max_threads };
      showToast("info", $t("telegram.toolbox.max_threads_toast", { count: r.max_threads }));
      await load();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? $t("common.error")));
    } finally {
      saving = false;
    }
  }

  function close() {
    if (saving) return;
    open = false;
  }

  $effect(() => {
    if (open) load();
  });

  function bucketLabel(b: { min_mb: number; max_mb: number }): string {
    if (b.max_mb === 0) return `≥ ${b.min_mb} MB`;
    return `${b.min_mb}–${b.max_mb} MB`;
  }
</script>

{#if open}
  <div
    class="overlay"
    role="presentation"
    onclick={(e) => { if (e.target === e.currentTarget) close(); }}
    onkeydown={(e) => { if (e.key === "Escape") close(); }}
  >
    <div class="panel" role="dialog" aria-modal="true" aria-label={$t("telegram.download_performance")} tabindex="-1">
      <header class="panel-header">
        <div>
          <h2>{$t("telegram.download_performance")}</h2>
          <p class="subtitle">{$t("telegram.toolbox.performance_subtitle")}</p>
        </div>
        <button type="button" class="icon-btn" onclick={close} aria-label={$t("common.close")}>
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18 6L6 18" />
            <path d="M6 6l12 12" />
          </svg>
        </button>
      </header>

      {#if loading}
        <div class="loading-section"><span class="spinner"></span></div>
      {:else if error}
        <div class="error-section">
          <p class="error-msg">{error}</p>
          <button type="button" class="button" onclick={load}>{$t("common.retry")}</button>
        </div>
      {:else if perf}
        <section class="setting-section">
          <label class="field">
            <div class="field-row">
              <span class="field-label">{$t("telegram.toolbox.max_threads")}</span>
              <span class="field-value">{draftMax}</span>
            </div>
            <input
              type="range"
              min="1"
              max="16"
              step="1"
              bind:value={draftMax}
              class="slider"
            />
            <span class="field-hint">
              {$t("telegram.toolbox.max_threads_hint")}
            </span>
          </label>
        </section>

        <section class="buckets-section">
          <span class="section-label">{$t("telegram.toolbox.threads_by_file_size")}</span>
          <table class="buckets-table">
            <thead>
              <tr>
                <th>{$t("telegram.toolbox.size")}</th>
                <th>{$t("telegram.toolbox.current_threads")}</th>
              </tr>
            </thead>
            <tbody>
              {#each perf.buckets as b}
                <tr>
                  <td>{bucketLabel(b)}</td>
                  <td><span class="thread-badge">{b.threads}</span></td>
                </tr>
              {/each}
            </tbody>
          </table>
          <p class="info-msg">
            {$t("telegram.toolbox.threads_cap_hint", { count: perf.max_threads })}
          </p>
        </section>

        {#if bw}
          <section class="bandwidth-section">
            <span class="section-label">{$t("telegram.toolbox.bandwidth")}</span>
            <div class="bw-bar-container">
              <div class="bw-bar-outer">
                <div
                  class="bw-bar-inner"
                  style:width="{Math.min(100, bw.percentage)}%"
                  class:warn={bw.percentage > 80}
                ></div>
              </div>
              <div class="bw-row">
                <span class="bw-used">{$t("telegram.toolbox.used_today", { amount: fmtBytes(bw.used_today_bytes) })}</span>
                <span class="bw-quota">{$t("telegram.toolbox.of_quota", { amount: fmtBytes(bw.quota_bytes) })}</span>
              </div>
              <span class="bw-total">{$t("telegram.toolbox.total_usage", { amount: fmtBytes(bw.total_used_bytes) })}</span>
            </div>
            <div class="quota-row">
              <label class="quota-field">
                <span class="field-label">{$t("telegram.toolbox.daily_quota")}</span>
                <div class="quota-input-row">
                  <input
                    type="number"
                    min="1"
                    max="10000"
                    step="1"
                    bind:value={draftQuotaGb}
                    class="input quota-input"
                    disabled={bwSaving}
                  />
                  <span class="quota-suffix">GB</span>
                  <button
                    type="button"
                    class="button"
                    onclick={saveQuota}
                    disabled={bwSaving || draftQuotaGb < 1}
                  >
                    {$t("telegram.toolbox.apply")}
                  </button>
                </div>
              </label>
              <button type="button" class="button ghost-btn" onclick={resetUsed} disabled={bwSaving}>
                {$t("telegram.toolbox.reset_today")}
              </button>
            </div>
          </section>
        {/if}

        {#if sync}
          <section class="sync-section">
            <span class="section-label">{$t("telegram.toolbox.automatic_sync")}</span>
            <p class="info-msg">
              {$t("telegram.toolbox.automatic_sync_hint")}
            </p>
            <label class="toggle-row">
              <input type="checkbox" bind:checked={draftSyncEnabled} />
              <span>{$t("telegram.toolbox.sync_in_background")}</span>
            </label>
            {#if draftSyncEnabled}
              <label class="field">
                <div class="field-row">
                  <span class="field-label">{$t("telegram.toolbox.interval")}</span>
                  <span class="field-value">{draftSyncIntervalMin} min</span>
                </div>
                <input
                  type="range"
                  min="5"
                  max="180"
                  step="5"
                  bind:value={draftSyncIntervalMin}
                  class="slider"
                />
              </label>
            {/if}
            <div class="sync-status-row">
              <span class="sync-meta">
                {#if sync.last_success_at > 0}
                  {$t("telegram.toolbox.last_sync", { time: new Date(sync.last_success_at * 1000).toLocaleTimeString() })}
                  · {$t("telegram.toolbox.updated_count", { count: sync.last_updated_count })}
                  · {sync.last_duration_ms}ms
                {:else}
                  {$t("telegram.toolbox.never_synced")}
                {/if}
              </span>
              <button
                type="button"
                class="button"
                onclick={saveSync}
                disabled={syncSaving || (draftSyncEnabled === sync.enabled && draftSyncIntervalMin === sync.interval_min)}
              >
                {syncSaving ? $t("telegram.accounts_saving") : $t("telegram.toolbox.save")}
              </button>
            </div>
          </section>
        {/if}

        <footer class="panel-footer">
          <button type="button" class="button" onclick={close} disabled={saving}>{$t("common.cancel")}</button>
          <button
            type="button"
            class="button primary"
            onclick={save}
            disabled={saving || draftMax === perf.max_threads}
          >
            {saving ? $t("telegram.accounts_saving") : $t("telegram.toolbox.save")}
          </button>
        </footer>
      {/if}
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    z-index: 150;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--padding);
    animation: fade-in 150ms ease;
  }

  @keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .panel {
    width: min(520px, 100vw);
    max-height: 90vh;
    overflow-y: auto;
    background: var(--surface, var(--button));
    border-radius: var(--border-radius);
    padding: calc(var(--padding) * 1.5);
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.3);
  }

  .panel-header {
    display: flex;
    align-items: flex-start;
    gap: var(--padding);
    border-bottom: 1px solid var(--input-border);
    padding-bottom: var(--padding);
  }

  .panel-header > div {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .panel-header h2 {
    margin: 0;
    font-size: 16px;
    color: var(--secondary);
  }

  .subtitle {
    margin: 0;
    font-size: 12px;
    color: var(--gray);
  }

  .icon-btn {
    background: transparent;
    border: none;
    color: var(--gray);
    cursor: pointer;
    padding: 6px;
    border-radius: var(--border-radius);
  }

  .icon-btn:hover {
    background: var(--button-elevated);
    color: var(--secondary);
  }

  .setting-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .field-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
  }

  .field-label {
    font-size: 13px;
    font-weight: 500;
    color: var(--secondary);
  }

  .field-value {
    font-size: 18px;
    font-weight: 600;
    color: var(--blue);
  }

  .field-hint {
    font-size: 11.5px;
    color: var(--gray);
    line-height: 1.5;
  }

  .slider {
    width: 100%;
    accent-color: var(--blue);
  }

  .section-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--gray);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .buckets-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .buckets-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }

  .buckets-table th {
    text-align: left;
    padding: 6px 8px;
    color: var(--gray);
    font-weight: 500;
    font-size: 11.5px;
    text-transform: uppercase;
    letter-spacing: 0.3px;
    border-bottom: 1px solid var(--input-border);
  }

  .buckets-table td {
    padding: 8px;
    color: var(--secondary);
    border-bottom: 1px solid var(--input-border);
  }

  .buckets-table tr:last-child td {
    border-bottom: none;
  }

  .thread-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 28px;
    padding: 2px 8px;
    background: var(--blue);
    color: #fff;
    border-radius: 4px;
    font-size: 12px;
    font-weight: 600;
  }

  .info-msg {
    margin: 0;
    font-size: 11.5px;
    color: var(--gray);
    line-height: 1.5;
  }

  .bandwidth-section {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding-top: var(--padding);
    border-top: 1px solid var(--input-border);
  }

  .bw-bar-container {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .bw-bar-outer {
    width: 100%;
    height: 8px;
    background: var(--button-elevated);
    border-radius: 4px;
    overflow: hidden;
  }

  .bw-bar-inner {
    height: 100%;
    background: var(--blue);
    border-radius: 4px;
    transition: width 200ms;
  }

  .bw-bar-inner.warn {
    background: var(--red);
  }

  .bw-row {
    display: flex;
    justify-content: space-between;
    font-size: 12px;
  }

  .bw-used {
    color: var(--secondary);
    font-weight: 500;
  }

  .bw-quota {
    color: var(--gray);
  }

  .bw-total {
    font-size: 11px;
    color: var(--gray);
  }

  .quota-row {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .quota-field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .quota-input-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .quota-input {
    width: 100px;
  }

  .quota-suffix {
    font-size: 12px;
    color: var(--gray);
  }

  .ghost-btn {
    background: transparent;
    color: var(--gray);
    border: 1px solid var(--input-border);
    align-self: flex-start;
    font-size: 11.5px;
  }

  .ghost-btn:hover:not(:disabled) {
    color: var(--secondary);
    background: var(--button-elevated);
  }

  .sync-section {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding-top: var(--padding);
    border-top: 1px solid var(--input-border);
  }

  .toggle-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--secondary);
    cursor: pointer;
  }

  .sync-status-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    margin-top: 4px;
  }

  .sync-meta {
    font-size: 11.5px;
    color: var(--gray);
  }

  .panel-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding-top: var(--padding);
    border-top: 1px solid var(--input-border);
  }

  .button {
    padding: 8px 16px;
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    color: var(--secondary);
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
  }

  .button:hover:not(:disabled) {
    background: color-mix(in oklab, var(--secondary) 8%, var(--button-elevated));
  }

  .button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .button.primary {
    background: var(--blue);
    color: #fff;
  }

  .button.primary:hover:not(:disabled) {
    background: color-mix(in oklab, var(--blue) 90%, #000);
  }

  .loading-section,
  .error-section {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--padding);
    padding: calc(var(--padding) * 3);
  }

  .spinner {
    width: 24px;
    height: 24px;
    border: 2px solid var(--input-border);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .error-msg {
    color: var(--red);
    font-size: 12px;
    margin: 0;
  }
</style>
