<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { t } from "$lib/i18n";
  import ConfirmDialog from "$lib/study-components/ConfirmDialog.svelte";

  let confirmOpen = $state(false);
  let confirmMessage = $state("");
  let confirmAction: (() => void) | null = null;

  function askConfirm(message: string, action: () => void) {
    confirmMessage = message;
    confirmAction = action;
    confirmOpen = true;
  }

  function runConfirm() {
    if (confirmAction) confirmAction();
    confirmAction = null;
  }

  type Settings = {
    enabled: boolean;
    acknowledged: boolean;
    acknowledged_at: number | null;
    domains: string[];
    log_mirrors: boolean;
  };

  let settings = $state<Settings | null>(null);
  let loading = $state(true);
  let errorMsg = $state("");
  let okMsg = $state("");

  let showWarning = $state(false);
  let warningChecked = $state(false);

  let domainsText = $state("");
  let domainsDirty = $state(false);

  async function load() {
    loading = true;
    try {
      settings = await pluginInvoke<Settings>("study", "study:read:annas:get_settings");
      domainsText = settings?.domains?.join("\n") ?? "";
      domainsDirty = false;
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  function requestEnable() {
    if (!settings) return;
    if (settings.acknowledged) {
      void doEnable(true);
    } else {
      warningChecked = false;
      showWarning = true;
    }
  }

  async function confirmWarning() {
    if (!warningChecked) return;
    try {
      await pluginInvoke("study", "study:read:annas:acknowledge");
      showWarning = false;
      okMsg = $t("study.read.annas_enabled");
      setTimeout(() => (okMsg = ""), 3000);
      await load();
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    }
  }

  async function doEnable(on: boolean) {
    try {
      await pluginInvoke("study", "study:read:annas:set_enabled", { enabled: on });
      await load();
      okMsg = on
        ? $t("study.read.annas_enabled")
        : $t("study.read.annas_disabled");
      setTimeout(() => (okMsg = ""), 3000);
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    }
  }

  async function toggleLog(on: boolean) {
    try {
      await pluginInvoke("study", "study:read:annas:set_log_mirrors", { on });
      await load();
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    }
  }

  async function saveDomains() {
    const list = domainsText
      .split(/\n|\r|,|\s+/)
      .map((s) => s.trim())
      .filter((s) => !!s);
    try {
      await pluginInvoke("study", "study:read:annas:set_domains", { domains: list });
      okMsg = $t("study.read.annas_domains_saved");
      setTimeout(() => (okMsg = ""), 3000);
      await load();
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    }
  }

  async function resetAll() {
    askConfirm($t("study.read.annas_reset_confirm"), () => doResetAll());
  }
  async function doResetAll() {
    try {
      await pluginInvoke("study", "study:read:annas:reset");
      await load();
      okMsg = $t("study.read.annas_reset_done");
      setTimeout(() => (okMsg = ""), 3000);
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    }
  }

  let clearingCache = $state(false);
  async function clearCache() {
    clearingCache = true;
    try {
      const r = await pluginInvoke<{ removed: number }>(
        "study",
        "study:read:annas:clear_cache",
      );
      okMsg =
        r.removed === 0
          ? $t("study.read.annas.cache_was_empty")
          : r.removed === 1
            ? "1 entrada de cache removida"
            : `${r.removed} entradas de cache removidas`;
      setTimeout(() => (okMsg = ""), 3000);
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    } finally {
      clearingCache = false;
    }
  }

  type BrowserStatus = {
    available: boolean;
    enabled: boolean;
    detected_at?: string | null;
  };
  let browserStatus = $state<BrowserStatus | null>(null);
  let browserBusy = $state(false);
  async function loadBrowserStatus() {
    try {
      browserStatus = await pluginInvoke<BrowserStatus>(
        "study",
        "study:read:browser:status",
      );
    } catch (e) {
      browserStatus = null;
    }
  }
  async function toggleBrowser(next: boolean) {
    browserBusy = true;
    try {
      await pluginInvoke("study", "study:read:browser:set_enabled", {
        enabled: next,
      });
      await loadBrowserStatus();
      okMsg = next
        ? "Browser auxiliar habilitado"
        : "Browser auxiliar desabilitado";
      setTimeout(() => (okMsg = ""), 3000);
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    } finally {
      browserBusy = false;
    }
  }

  function fmtDate(ts: number | null): string {
    if (!ts) return "—";
    return new Date(ts * 1000).toLocaleString();
  }

  onMount(async () => {
    await load();
    await loadBrowserStatus();
  });
</script>

<section class="page">
  <header class="head">
    <button type="button" class="back-btn" onclick={() => goto("/study/read/books/discover")}>
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
        <path d="M15 18l-6-6 6-6"></path>
      </svg>
      <span>{$t("study.read.back_to_library")}</span>
    </button>
    <h1>{$t("study.read.annas_title")}</h1>
  </header>

  <p class="subtitle">{$t("study.read.annas_subtitle")}</p>

  {#if loading}
    <p class="muted">{$t("study.common.loading")}</p>
  {:else if settings}
    <div class="warning">
      <strong>{$t("study.read.annas_warning_title")}</strong>
      <p>{$t("study.read.annas_warning_body1")}</p>
      <p>{$t("study.read.annas_warning_body2")}</p>
      <p>{$t("study.read.annas_warning_body3")}</p>
    </div>

    {#if okMsg}
      <p class="ok small">{okMsg}</p>
    {/if}
    {#if errorMsg}
      <p class="error small">{errorMsg}</p>
    {/if}

    <section class="block">
      <div class="row">
        <div>
          <strong>{$t("study.read.annas_status")}</strong>
          <p class="muted small">
            {#if settings.acknowledged}
              {$t("study.read.annas_acked_at", { date: fmtDate(settings.acknowledged_at) })}
            {:else}
              {$t("study.read.annas_not_acked")}
            {/if}
          </p>
        </div>
        <div class="actions">
          {#if !settings.enabled}
            <button type="button" class="cta" onclick={requestEnable}>
              {$t("study.read.annas_enable_btn")}
            </button>
          {:else}
            <span class="badge on">{$t("study.read.annas_on")}</span>
            <button type="button" class="ghost-btn" onclick={() => doEnable(false)}>
              {$t("study.read.annas_disable_btn")}
            </button>
          {/if}
        </div>
      </div>
    </section>

    <section class="block">
      <div class="row">
        <div>
          <strong>{$t("study.read.annas_log_title")}</strong>
          <p class="muted small">{$t("study.read.annas_log_hint")}</p>
        </div>
        <label class="toggle">
          <input
            type="checkbox"
            checked={settings.log_mirrors}
            onchange={(e) => toggleLog((e.currentTarget as HTMLInputElement).checked)}
          />
        </label>
      </div>
    </section>

    <section class="block">
      <h3>{$t("study.read.annas_domains_title")}</h3>
      <p class="muted small">{$t("study.read.annas_domains_hint")}</p>
      <textarea
        class="domains"
        bind:value={domainsText}
        oninput={() => (domainsDirty = true)}
        rows="6"
        spellcheck="false"
      ></textarea>
      <div class="actions">
        <button
          type="button"
          class="cta outline"
          onclick={saveDomains}
          disabled={!domainsDirty}
        >
          {$t("study.read.annas_domains_save")}
        </button>
        <button type="button" class="ghost-btn" onclick={() => { domainsText = settings?.domains?.join("\n") ?? ""; domainsDirty = false; }}>
          {$t("study.read.annas_domains_revert")}
        </button>
      </div>
    </section>

    <section class="block">
      <h3>Cache de busca</h3>
      <p class="muted small">
        Resultados são guardados em memória pra não bombardear os mirrors.
        Limpe se você está vendo dados desatualizados.
      </p>
      <button
        type="button"
        class="ghost-btn"
        onclick={clearCache}
        disabled={clearingCache}
      >
        {clearingCache ? "Limpando…" : "Limpar cache"}
      </button>
    </section>

    <section class="block">
      <h3>Browser auxiliar</h3>
      <p class="muted small">
        Alguns mirrors exigem JavaScript pra retornar HTML. Habilite o browser
        embutido pra esses casos. Pode ficar mais lento.
      </p>
      {#if browserStatus}
        <div class="browser-status">
          <span
            class="dot"
            class:on={browserStatus.enabled}
            class:off={!browserStatus.enabled}
            aria-hidden="true"
          ></span>
          <span>
            {browserStatus.enabled ? "Habilitado" : "Desabilitado"}
            {#if browserStatus.available === false}
              · não detectado no sistema
            {/if}
          </span>
        </div>
      {/if}
      <button
        type="button"
        class="ghost-btn"
        onclick={() => toggleBrowser(!(browserStatus?.enabled ?? false))}
        disabled={browserBusy}
      >
        {browserBusy
          ? "Aplicando…"
          : browserStatus?.enabled
            ? "Desabilitar"
            : "Habilitar"}
      </button>
    </section>

    <section class="block danger">
      <h3>{$t("study.read.annas_reset_title")}</h3>
      <p class="muted small">{$t("study.read.annas_reset_hint")}</p>
      <button type="button" class="ghost-btn danger-btn" onclick={resetAll}>
        {$t("study.read.annas_reset_btn")}
      </button>
    </section>
  {/if}
</section>

<ConfirmDialog
  bind:open={confirmOpen}
  message={confirmMessage}
  variant="danger"
  onConfirm={runConfirm}
/>

{#if showWarning}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="modal-overlay" onclick={() => (showWarning = false)}></div>
  <div class="modal" role="dialog" tabindex="-1" aria-label="warning">
    <header class="modal-head">
      <h2>{$t("study.read.annas_warning_modal_title")}</h2>
    </header>
    <div class="modal-body">
      <p><strong>{$t("study.read.annas_warning_modal_line1")}</strong></p>
      <p>{$t("study.read.annas_warning_modal_line2")}</p>
      <p>{$t("study.read.annas_warning_modal_line3")}</p>
      <p>{$t("study.read.annas_warning_modal_line4")}</p>
      <label class="ack">
        <input type="checkbox" bind:checked={warningChecked} />
        <span>{$t("study.read.annas_warning_ack_label")}</span>
      </label>
    </div>
    <footer class="modal-foot">
      <button type="button" class="ghost-btn" onclick={() => (showWarning = false)}>
        {$t("study.read.annas_warning_cancel")}
      </button>
      <button type="button" class="cta" disabled={!warningChecked} onclick={confirmWarning}>
        {$t("study.read.annas_warning_confirm")}
      </button>
    </footer>
  </div>
{/if}

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    width: 100%;
    max-width: 780px;
    margin-inline: auto;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 500;
    letter-spacing: -0.5px;
    color: var(--secondary);
    flex: 1;
  }
  .back-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    background: transparent;
    color: var(--tertiary);
    border: none;
    border-radius: var(--border-radius);
    font-size: 12px;
    cursor: pointer;
    font-family: inherit;
  }
  .back-btn:hover {
    color: var(--secondary);
    background: var(--sidebar-highlight);
  }
  .subtitle {
    margin: 0;
    color: var(--tertiary);
    font-size: 14px;
  }
  .warning {
    padding: var(--padding);
    background: color-mix(in oklab, var(--warning) 15%, transparent);
    border: 1px solid color-mix(in oklab, var(--warning) 50%, transparent);
    border-radius: var(--border-radius);
    font-size: 13px;
    color: var(--secondary);
  }
  .warning strong {
    display: block;
    margin-bottom: 6px;
  }
  .warning p {
    margin: 4px 0;
    line-height: 1.5;
    color: var(--secondary);
  }
  .ok {
    color: var(--success);
  }
  .error {
    color: var(--error);
  }
  .muted { color: var(--tertiary); }
  .small { font-size: 11px; }
  .block {
    padding: var(--padding);
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .block.danger {
    border-color: color-mix(in oklab, var(--error) 40%, var(--content-border));
  }
  .block h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 500;
    color: var(--secondary);
  }
  .row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  .row strong {
    display: block;
    font-size: 14px;
    color: var(--secondary);
  }
  .row p {
    margin: 4px 0 0;
  }
  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }
  .toggle {
    align-self: center;
  }
  .toggle input {
    accent-color: var(--accent);
    width: 18px;
    height: 18px;
  }
  .domains {
    width: 100%;
    box-sizing: border-box;
    padding: 8px 10px;
    background: var(--input-bg);
    color: var(--secondary);
    border: none;
    border-radius: var(--border-radius);
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-size: 12px;
    line-height: 1.5;
    resize: vertical;
  }
  .badge {
    display: inline-block;
    padding: 3px 10px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.4px;
  }
  .badge.on {
    background: color-mix(in oklab, var(--success) 20%, transparent);
    color: var(--success);
    border: 1px solid color-mix(in oklab, var(--success) 50%, transparent);
  }
  .cta {
    padding: 8px 16px;
    background: var(--cta, var(--accent));
    color: var(--on-cta, white);
    border: 1px solid transparent;
    border-radius: var(--border-radius);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    font-family: inherit;
  }
  .cta.outline {
    background: transparent;
    color: var(--secondary);
    border-color: var(--input-border);
  }
  .cta:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .ghost-btn {
    padding: 8px 14px;
    background: transparent;
    color: var(--tertiary);
    border: none;
    border-radius: var(--border-radius);
    font-size: 12px;
    cursor: pointer;
    font-family: inherit;
  }
  .ghost-btn:hover:not(:disabled) {
    color: var(--secondary);
    background: var(--sidebar-highlight);
  }
  .ghost-btn.danger-btn {
    color: var(--error);
    border-color: color-mix(in oklab, var(--error) 40%, var(--input-border));
  }
  .ghost-btn.danger-btn:hover {
    background: color-mix(in oklab, var(--error) 15%, transparent);
  }

  .browser-status {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 0;
    color: var(--secondary);
    font-size: 12px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .dot.on { background: var(--success, var(--accent)); }
  .dot.off { background: var(--tertiary); }

  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    backdrop-filter: blur(4px);
    z-index: 40;
  }
  .modal {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: min(560px, 90vw);
    max-height: 85vh;
    overflow-y: auto;
    background: var(--primary);
    border: none;
    border-radius: var(--border-radius);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.4);
    z-index: 41;
    display: flex;
    flex-direction: column;
  }
  .modal-head {
    padding: 16px 20px;
    border-bottom: none;
  }
  .modal-head h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 500;
    color: var(--secondary);
  }
  .modal-body {
    padding: 16px 20px;
    font-size: 13px;
    color: var(--secondary);
  }
  .modal-body p {
    margin: 0 0 10px;
    line-height: 1.5;
  }
  .ack {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 12px;
    padding: 10px;
    background: color-mix(in oklab, var(--warning) 12%, transparent);
    border: 1px solid color-mix(in oklab, var(--warning) 40%, transparent);
    border-radius: var(--border-radius);
    font-size: 13px;
    cursor: pointer;
  }
  .ack input {
    accent-color: var(--accent);
    width: 18px;
    height: 18px;
    flex-shrink: 0;
  }
  .modal-foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 20px;
    border-top: none;
  }
</style>
