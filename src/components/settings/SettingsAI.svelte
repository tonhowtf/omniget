<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

  import {
    aiSettingsPayload, clearSavedAiSettingsDraft, isAiSettingsDirty, isLoopbackAiEndpoint,
    readAiSettingsDraft, retainAiSettingsDraft,
    type AiProvider as Provider, type AiConfigView as ConfigView, type AiSettingsDraft,
  } from "./settings-ai-form";

  type HistoryEntry = {
    id: number;
    kind: string;
    url: string;
    title: string;
    content: string;
    created_at_ms: number;
  };

  let provider = $state<Provider>("none");
  let model = $state("");
  let localBaseUrl = $state("");
  let openaiKey = $state("");
  let anthropicKey = $state("");
  let hasOpenaiKey = $state(false);
  let hasAnthropicKey = $state(false);
  let removeOpenaiKey = $state(false);
  let removeAnthropicKey = $state(false);
  let configError = $state(false);
  let configLoading = $state(true);
  let saveFailed = $state(false);

  let savedConfig = $state<ConfigView | null>(null);
  let configReady = $state(false);
  let saving = $state(false);
  let testing = $state(false);
  function draft(): AiSettingsDraft {
    return { provider, model, localBaseUrl, openaiKey, anthropicKey, removeOpenaiKey, removeAnthropicKey };
  }
  let configDirty = $derived(isAiSettingsDirty(draft(), savedConfig));
  let effectiveKind = $derived(
    savedConfig && provider === savedConfig.provider && localBaseUrl.trim().replace(/\/+$/, "") === savedConfig.local_base_url
      ? savedConfig.kind : ""
  );
  let summarizeUrl = $state("");
  let summarizing = $state(false);
  let summaryStyle = $state<"short" | "balanced" | "detailed">("balanced");
  let summaryLang = $state("");
  let history = $state<HistoryEntry[]>([]);
  let historyLoading = $state(true);
  let historyError = $state(false);
  let clearingHistory = $state(false);

  function applyConfig(c: ConfigView) {
    provider = c.provider;
    model = c.model;
    localBaseUrl = c.local_base_url;
    hasOpenaiKey = c.has_openai_key;
    hasAnthropicKey = c.has_anthropic_key;
    openaiKey = "";
    anthropicKey = "";
    removeOpenaiKey = false;
    removeAnthropicKey = false;
    saveFailed = false;
    savedConfig = { ...c };
    configReady = true;
  }

  async function loadConfig() {
    configLoading = true;
    configError = false;
    try {
      const c = await invoke<ConfigView>("ai_get_config");
      applyConfig(c);
      const retained = readAiSettingsDraft();
      if (retained) {
        provider = retained.provider;
        model = retained.model;
        localBaseUrl = retained.localBaseUrl;
        openaiKey = retained.openaiKey;
        anthropicKey = retained.anthropicKey;
        removeOpenaiKey = retained.removeOpenaiKey;
        removeAnthropicKey = retained.removeAnthropicKey;
      }
    } catch (e: any) {
      configReady = false;
      configError = true;
    } finally {
      configLoading = false;
    }
  }

  function discard() {
    if (!savedConfig || saving || testing) return;
    retainAiSettingsDraft(null);
    applyConfig(savedConfig);
  }

  onDestroy(() => {
    if (configReady) retainAiSettingsDraft(configDirty ? draft() : null);
  });

  async function loadHistory() {
    historyLoading = true;
    historyError = false;
    try {
      history = (await invoke<HistoryEntry[]>("ai_history_list")).slice().reverse();
    } catch {
      historyError = true;
    } finally {
      historyLoading = false;
    }
  }

  onMount(() => {
    loadConfig();
    loadHistory();
  });

  async function save(): Promise<boolean> {
    if (!configReady || saving || summarizing) return false;
    saving = true;
    saveFailed = false;
    const submitted = draft();
    try {
      const result = await invoke<ConfigView>("ai_set_config", aiSettingsPayload(submitted));
      clearSavedAiSettingsDraft(submitted);
      applyConfig(result);
      showToast("success", $t("settings.ai.saved") as string);
      return true;
    } catch (e: any) {
      saveFailed = true;
      showToast("error", $t("settings.ai.save_failed"));
      return false;
    } finally {
      saving = false;
    }
  }

  async function runTest() {
    if (!configReady || testing || saving || summarizing) return;
    testing = true;
    try {
      // A dirty form must never report a result for the previously saved provider.
      if (configDirty && !(await save())) return;
      await invoke<string>("ai_test");
      showToast("success", $t("settings.ai.test_ok") as string);
    } catch (e: any) {
      const msg = typeof e === "string" ? e : $t("common.error");
      showToast("error", $t("settings.ai.test_fail", { error: msg }) as string);
    } finally {
      testing = false;
    }
  }

  async function summarize() {
    const url = summarizeUrl.trim();
    if (!url || summarizing || !configReady || configDirty || saving || testing) return;
    summarizing = true;
    try {
      await invoke("ai_summarize_url", {
        url,
        style: summaryStyle === "balanced" ? null : summaryStyle,
        lang: summaryLang.trim() || null,
      });
      summarizeUrl = "";
      await loadHistory();
      showToast("success", $t("settings.ai.summary_ready") as string);
    } catch (e: any) {
      const raw = typeof e === "string" ? e : $t("common.error");
      const msg =
        raw === "no_transcript"
          ? ($t("settings.ai.no_transcript") as string)
          : raw === "ai_not_configured"
            ? ($t("settings.ai.not_configured") as string)
            : raw;
      showToast("error", msg);
    } finally {
      summarizing = false;
    }
  }

  async function clearHistory() {
    if (clearingHistory || !confirm($t("settings.ai.history_clear_confirm"))) return;
    clearingHistory = true;
    try {
      await invoke("ai_history_clear");
      history = [];
    } catch {
      showToast("error", $t("settings.ai.history_clear_failed"));
    } finally {
      clearingHistory = false;
    }
  }
</script>

<section class="section">
  <h5 class="section-title">{$t('settings.ai.title')}</h5>

  {#if configLoading}
    <div class="card" role="status">{$t('common.loading')}</div>
  {:else if configError}
    <div class="card" role="alert">
      <p>{$t('settings.ai.load_failed')}</p>
      <button class="ghost-btn" onclick={loadConfig}>{$t('error.remedy.retry')}</button>
    </div>
  {/if}
  <fieldset class="card config-fields" disabled={!configReady || saving || testing || summarizing} aria-busy={saving || testing}>
    <div class="setting-row" data-setting-key="settings.ai.provider">
      <div class="setting-col">
        <label class="setting-label" for="ai-provider">{$t('settings.ai.provider')}</label>
        <span class="setting-path">{$t('settings.ai.provider_desc')}</span>
      </div>
      <select id="ai-provider" class="input-text select" bind:value={provider} disabled={saving || testing} onchange={(event) => {
        const next = event.currentTarget.value as Provider;
        if (next !== provider && (next === "openai" || next === "anthropic")) localBaseUrl = "";
      }}>
        <option value="none">{$t('settings.ai.provider_none')}</option>
        <option value="openai">OpenAI</option>
        <option value="anthropic">Anthropic</option>
        <option value="local">{$t('settings.ai.provider_endpoint')}</option>
      </select>
    </div>
    {#if provider === "none" && (hasOpenaiKey || hasAnthropicKey)}
      <p class="setting-path wrap">{$t('settings.ai.off_keeps_keys')}</p>
    {/if}
    {#if effectiveKind && provider !== "none"}
      <p class="setting-path wrap">{$t('settings.ai.connection_kind', { kind: effectiveKind })}</p>
    {/if}

    {#if provider !== "none"}
      <div class="divider"></div>
      <div class="setting-row" data-setting-key="settings.ai.model">
        <div class="setting-col">
          <label class="setting-label" for="ai-model">{$t('settings.ai.model')}</label>
          <span class="setting-path">{$t('settings.ai.model_desc')}</span>
        </div>
        <input type="text" class="input-text" placeholder={$t('settings.ai.model_placeholder')} id="ai-model" bind:value={model} disabled={saving || testing} />
      </div>
    {/if}

    {#if provider === "openai" || provider === "local"}
      <div class="divider"></div>
      <div class="setting-row" data-setting-key="settings.ai.connection_key">
        <div class="setting-col">
          <label class="setting-label" for="ai-openai-key">{$t('settings.ai.connection_key')}</label>
          <span class="setting-path wrap" id="ai-openai-key-hint">{removeOpenaiKey ? $t('settings.ai.key_remove_pending') : hasOpenaiKey ? $t('settings.ai.key_set') : $t('settings.ai.key_unset')}</span>
        </div>
        <div class="credential-input">
          <input id="ai-openai-key" type="password" class="input-text" autocomplete="off" placeholder={hasOpenaiKey && !removeOpenaiKey ? "••••••••" : ""} bind:value={openaiKey} disabled={saving || testing || removeOpenaiKey} aria-describedby="ai-openai-key-hint" />
          {#if hasOpenaiKey}
            <button type="button" class="ghost-btn" onclick={() => { removeOpenaiKey = !removeOpenaiKey; openaiKey = ""; }}>
              {removeOpenaiKey ? $t('settings.ai.keep_key') : $t('settings.ai.remove_key')}
            </button>
          {/if}
        </div>
      </div>
    {/if}

    {#if provider === "anthropic"}
      <div class="divider"></div>
      <div class="setting-row" data-setting-key="settings.ai.anthropic_key">
        <div class="setting-col">
          <label class="setting-label" for="ai-anthropic-key">{$t('settings.ai.anthropic_key')}</label>
          <span class="setting-path wrap" id="ai-anthropic-key-hint">{removeAnthropicKey ? $t('settings.ai.key_remove_pending') : hasAnthropicKey ? $t('settings.ai.key_set') : $t('settings.ai.key_unset')}</span>
        </div>
        <div class="credential-input">
          <input id="ai-anthropic-key" type="password" class="input-text" autocomplete="off" placeholder={hasAnthropicKey && !removeAnthropicKey ? "••••••••" : ""} bind:value={anthropicKey} disabled={saving || testing || removeAnthropicKey} aria-describedby="ai-anthropic-key-hint" />
          {#if hasAnthropicKey}
            <button type="button" class="ghost-btn" onclick={() => { removeAnthropicKey = !removeAnthropicKey; anthropicKey = ""; }}>
              {removeAnthropicKey ? $t('settings.ai.keep_key') : $t('settings.ai.remove_key')}
            </button>
          {/if}
        </div>
      </div>
    {/if}

    {#if provider === "local"}
      <div class="divider"></div>
      <div class="setting-row" data-setting-key="settings.ai.endpoint_url">
        <div class="setting-col">
          <label class="setting-label" for="ai-endpoint">{$t('settings.ai.endpoint_url')}</label>
          <span class="setting-path wrap">{$t('settings.ai.endpoint_url_desc')}</span>
          {#if localBaseUrl.trim()}<span class="setting-path wrap">{$t(isLoopbackAiEndpoint(localBaseUrl) ? 'settings.ai.endpoint_loopback' : 'settings.ai.endpoint_remote')}</span>{/if}
        </div>
        <input type="text" class="input-text" placeholder="http://localhost:11434/v1" id="ai-endpoint" bind:value={localBaseUrl} disabled={saving || testing} />
      </div>
    {/if}

    <div class="divider"></div>
    <div class="actions-row">
      <button class="primary-btn" disabled={!configReady || !configDirty || saving || testing} onclick={save}>
        {saving ? $t('settings.ai.saving') : $t('settings.ai.save')}
      </button>
      {#if configDirty}<button class="ghost-btn" onclick={discard}>{$t('settings.ai.discard')}</button>{/if}
      {#if provider !== "none"}
        <button class="ghost-btn" disabled={!configReady || testing || saving} onclick={runTest}>
          {configDirty ? $t('settings.ai.save_test') : $t('settings.ai.test')}
        </button>
      {/if}
    </div>
    {#if configDirty}<p class="setting-path unsaved-state wrap" role="status">{$t('settings.ai.draft_retained')}</p>{/if}
    {#if saveFailed}<p class="feedback-error" role="alert">{$t('settings.ai.save_failed')}</p>{/if}
    {#if provider !== "none"}<p class="setting-path wrap">{$t('settings.ai.test_effect')}</p>{/if}
  </fieldset>

  {#if provider !== "none"}
    <h5 class="section-title">{$t('settings.ai.summarize_title')}</h5>
    <div class="card">
      <div class="add-row">
        <input
          type="text"
          class="input-text"
          placeholder={$t('settings.ai.summarize_placeholder')}
          aria-label={$t('settings.ai.summarize_placeholder')}
          disabled={summarizing}
          bind:value={summarizeUrl}
          onkeydown={(e) => { if (e.key === "Enter") summarize(); }}
        />
        <button class="primary-btn" disabled={!summarizeUrl.trim() || summarizing || !configReady || configDirty || saving || testing} onclick={summarize}>
          {summarizing ? $t('settings.ai.summarizing') : $t('settings.ai.summarize')}
        </button>
      </div>
      <div class="add-row sum-opts">
        <select class="input-text select" bind:value={summaryStyle} disabled={summarizing} aria-label={$t('settings.ai.style') as string}>
          <option value="short">{$t('settings.ai.style_short')}</option>
          <option value="balanced">{$t('settings.ai.style_balanced')}</option>
          <option value="detailed">{$t('settings.ai.style_detailed')}</option>
        </select>
        <input
          type="text"
          class="input-text"
          placeholder={$t('settings.ai.lang_placeholder')}
          aria-label={$t('settings.ai.lang_placeholder')}
          disabled={summarizing}
          bind:value={summaryLang}
        />
      </div>
      <span class="setting-path add-hint wrap">{$t('settings.ai.summarize_effect')}</span>
    </div>
  {/if}

  <div class="hist-head">
    <h5 class="section-title">{$t('settings.ai.history')}</h5>
    {#if history.length > 0 && !historyError}
      <button class="link-btn danger" disabled={clearingHistory || historyLoading} onclick={clearHistory}>{$t('settings.ai.history_clear')}</button>
    {/if}
  </div>
  {#if historyLoading}
    <div class="card empty" role="status">{$t('common.loading')}</div>
  {:else if historyError}
    <div class="card empty" role="alert">
      <p>{$t('settings.ai.history_load_failed')}</p>
      <button class="ghost-btn" onclick={loadHistory}>{$t('error.remedy.retry')}</button>
    </div>
  {:else if history.length === 0}
    <div class="card empty">
      <span class="setting-path">{$t('settings.ai.history_empty')}</span>
    </div>
  {:else}
    <div class="card">
      {#each history as h, i (h.id)}
        {#if i > 0}<div class="divider"></div>{/if}
        <details class="hist">
          <summary>{h.title || h.url}</summary>
          <p class="hist-body">{h.content}</p>
        </details>
      {/each}
    </div>
  {/if}
</section>

<style>
  .config-fields { min-width: 0; border: 0; }
  .credential-input { display: flex; flex-wrap: wrap; gap: 8px; }
  .credential-input .input-text { min-width: 0; flex: 1; }
  .wrap { white-space: normal; overflow: visible; line-height: 1.5; }
  .feedback-error { color: var(--error); }

  .select {
    min-width: 160px;
  }

  .actions-row,
  .add-row {
    display: flex;
    gap: 8px;
    width: 100%;
  }

  .add-row .input-text {
    flex: 1;
    min-width: 0;
  }

  .primary-btn {
    padding: 0 16px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 600;
    cursor: pointer;
  }

  .primary-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .ghost-btn {
    padding: 0 16px;
    border: none;
    border-radius: var(--border-radius);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }

  .ghost-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .add-hint {
    display: block;
    margin-top: 8px;
  }

  .unsaved-state { margin: 10px 0 0; }

  .sum-opts {
    margin-top: 8px;
  }

  .sum-opts .input-text {
    flex: 1;
    min-width: 0;
  }

  .hist-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .link-btn {
    background: none;
    border: none;
    color: var(--accent);
    cursor: pointer;
    font-size: 13px;
    padding: 0;
  }

  .link-btn.danger {
    color: var(--error);
  }

  .empty {
    text-align: center;
    padding: 20px 16px;
  }

  .hist summary {
    cursor: pointer;
    font-weight: 600;
    padding: 6px 0;
  }

  .hist-body {
    white-space: pre-wrap;
    margin: 4px 0 8px;
    color: var(--text);
    font-size: 13px;
    line-height: 1.5;
  }
</style>
