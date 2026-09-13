<script lang="ts">
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
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
      showToast("error", typeof e === "string" ? e : (e?.message ?? "Erro"));
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
      error = typeof e === "string" ? e : (e?.message ?? "Erro");
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
      showToast("error", typeof e === "string" ? e : (e?.message ?? "Erro"));
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
      showToast("info", $t("study.telegram.perf.today_reset"));
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? "Erro"));
    } finally {
      bwSaving = false;
    }
  }

  async function save() {
    saving = true;
    try {
      const r = await telegramPerfSet({ maxThreads: draftMax });
      if (perf) perf = { ...perf, max_threads: r.max_threads };
      showToast("info", `Máximo de threads: ${r.max_threads}`);
      await load();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? "Erro"));
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
    <div class="panel" role="dialog" aria-modal="true" aria-label="Performance de download">
      <header class="panel-header">
        <div>
          <h2>{$t("study.telegram.perf.title")}</h2>
          <p class="subtitle">{$t("study.telegram.perf.subtitle")}</p>
        </div>
        <button type="button" class="icon-btn" onclick={close} aria-label="Fechar">
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
          <button type="button" class="button" onclick={load}>{$t("study.anki.settings.try_again")}</button>
        </div>
      {:else if perf}
        <section class="setting-section">
          <label class="field">
            <div class="field-row">
              <span class="field-label">{$t("study.telegram.perf.max_threads")}</span>
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
              Telegram cobra 1 MiB por chunk. Mais threads = downloads mais rápidos em arquivos grandes,
              mas pode disparar FLOOD_WAIT em conexões lentas. Padrão: 8.
            </span>
          </label>
        </section>

        <section class="buckets-section">
          <span class="section-label">{$t("study.telegram.perf.threads_by_size")}</span>
          <table class="buckets-table">
            <thead>
              <tr>
                <th>{$t("study.telegram.perf.size")}</th>
                <th>{$t("study.telegram.perf.current_threads")}</th>
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
            Os valores acima refletem o cap atual ({perf.max_threads}). Reduza o cap pra economizar
            CPU/largura de banda; aumente pra acelerar arquivos &gt; 50 MB.
          </p>
        </section>

        {#if bw}
          <section class="bandwidth-section">
            <span class="section-label">{$t("study.telegram.perf.bandwidth")}</span>
            <div class="bw-bar-container">
              <div class="bw-bar-outer">
                <div
                  class="bw-bar-inner"
                  style:width="{Math.min(100, bw.percentage)}%"
                  class:warn={bw.percentage > 80}
                ></div>
              </div>
              <div class="bw-row">
                <span class="bw-used">{fmtBytes(bw.used_today_bytes)} usados hoje</span>
                <span class="bw-quota">de {fmtBytes(bw.quota_bytes)}</span>
              </div>
              <span class="bw-total">Total geral: {fmtBytes(bw.total_used_bytes)}</span>
            </div>
            <div class="quota-row">
              <label class="quota-field">
                <span class="field-label">{$t("study.telegram.perf.daily_quota")}</span>
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
                    Aplicar
                  </button>
                </div>
              </label>
              <button type="button" class="button ghost-btn" onclick={resetUsed} disabled={bwSaving}>
                Zerar uso de hoje
              </button>
            </div>
          </section>
        {/if}

        {#if sync}
          <section class="sync-section">
            <span class="section-label">{$t("study.telegram.perf.auto_sync")}</span>
            <p class="info-msg">
              A cada N minutos o plugin atualiza o cache de canais em background — evita erros CHANNEL_INVALID quando você abre chats antigos.
            </p>
            <label class="toggle-row">
              <input type="checkbox" bind:checked={draftSyncEnabled} />
              <span>{$t("study.telegram.perf.sync_background")}</span>
            </label>
            {#if draftSyncEnabled}
              <label class="field">
                <div class="field-row">
                  <span class="field-label">{$t("study.telegram.perf.interval")}</span>
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
                  Última: {new Date(sync.last_success_at * 1000).toLocaleTimeString()}
                  · {sync.last_updated_count} atualizados
                  · {sync.last_duration_ms}ms
                {:else}
                  Ainda não sincronizou.
                {/if}
              </span>
              <button
                type="button"
                class="button"
                onclick={saveSync}
                disabled={syncSaving || (draftSyncEnabled === sync.enabled && draftSyncIntervalMin === sync.interval_min)}
              >
                {syncSaving ? "Salvando..." : "Salvar"}
              </button>
            </div>
          </section>
        {/if}

        <footer class="panel-footer">
          <button type="button" class="button" onclick={close} disabled={saving}>{$t("study.common.cancel")}</button>
          <button
            type="button"
            class="button primary"
            onclick={save}
            disabled={saving || draftMax === perf.max_threads}
          >
            {saving ? "Salvando..." : "Salvar"}
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
    border-bottom: none;
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
    border-bottom: none;
  }

  .buckets-table td {
    padding: 8px;
    color: var(--secondary);
    border-bottom: none;
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
    border-top: none;
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
    border: none;
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
    border-top: none;
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
    border-top: none;
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
