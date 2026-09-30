<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import SettingGroup from "./rebrand/SettingGroup.svelte";

  import {
    aiConnectionStatus, clearSavedAiSettingsDraft, isAiSettingsDirty, isLoopbackAiEndpoint,
    loadAiConnection, readAiSettingsDraft, retainAiSettingsDraft, saveAiConnection, switchAiProvider,
    testAiConnection,
    type AiProvider as Provider, type AiConfigView as ConfigView, type AiSettingsDraft,
  } from "./settings-ai-form";

  // Settings only configures the summary connection and reports its status.
  // Running a summary and reading its history live in /llm/summaries.

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
  // Bumped after each test so the status re-reads the module-level result.
  let testRevision = $state(0);
  function draft(): AiSettingsDraft {
    return { provider, model, localBaseUrl, openaiKey, anthropicKey, removeOpenaiKey, removeAnthropicKey };
  }
  function applyDraft(d: AiSettingsDraft) {
    provider = d.provider;
    model = d.model;
    localBaseUrl = d.localBaseUrl;
    openaiKey = d.openaiKey;
    anthropicKey = d.anthropicKey;
    removeOpenaiKey = d.removeOpenaiKey;
    removeAnthropicKey = d.removeAnthropicKey;
  }
  let configDirty = $derived(isAiSettingsDirty(draft(), savedConfig));
  let effectiveKind = $derived(
    savedConfig && provider === savedConfig.provider && localBaseUrl.trim().replace(/\/+$/, "") === savedConfig.local_base_url
      ? savedConfig.kind : ""
  );
  let connectionStatus = $derived.by(() => { void testRevision; return aiConnectionStatus(savedConfig); });

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
      // Local read only: opening Settings never contacts the provider.
      applyConfig(await loadAiConnection(invoke));
      const retained = readAiSettingsDraft();
      if (retained) applyDraft(retained);
    } catch {
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

  onMount(() => {
    loadConfig();
  });

  function onProviderChange(next: Provider) {
    applyDraft(switchAiProvider(draft(), next));
  }

  async function save(): Promise<boolean> {
    if (!configReady || saving) return false;
    saving = true;
    saveFailed = false;
    const submitted = draft();
    try {
      const result = await saveAiConnection(invoke, submitted);
      applyConfig(result);
      testRevision++;
      showToast("success", $t("settings.ai.saved") as string);
      return true;
    } catch {
      saveFailed = true;
      showToast("error", $t("settings.ai.save_failed"));
      return false;
    } finally {
      saving = false;
    }
  }

  async function runTest() {
    if (!configReady || testing || saving) return;
    testing = true;
    const dirty = configDirty;
    const submitted = draft();
    if (dirty) saving = true;
    saveFailed = false;
    try {
      // A dirty form is saved first; the provider is contacted only after the
      // save is confirmed, so a result never belongs to the previous connection.
      const outcome = await testAiConnection(invoke, { dirty, draft: submitted, saved: savedConfig });
      if (outcome.kind === "save_failed") {
        saveFailed = true;
        showToast("error", $t("settings.ai.save_failed"));
        return;
      }
      if (dirty && outcome.saved) {
        clearSavedAiSettingsDraft(submitted);
        applyConfig(outcome.saved);
      }
      if (outcome.kind === "ok") showToast("success", $t("settings.ai.test_ok") as string);
      else {
        const msg = typeof outcome.error === "string" ? outcome.error : $t("common.error");
        showToast("error", $t("settings.ai.test_fail", { error: msg }) as string);
      }
    } finally {
      saving = false;
      testing = false;
      testRevision++;
    }
  }
</script>

<SettingGroup id="connections" title={$t('settings.ai.title') as string} description={$t('settings.ai.provider_desc') as string}>
  {#snippet actions()}
    <a class="link-btn" href="/llm/summaries" data-setting-key="settings.ai.summarize_title">{$t('settings.ai.summarize_title')}</a>
  {/snippet}

  {#if configLoading}
    <div class="ai-state" role="status">{$t('common.loading')}</div>
  {:else if configError}
    <div class="ai-state" role="alert">
      <p>{$t('settings.ai.load_failed')}</p>
      <button class="ghost-btn" onclick={loadConfig}>{$t('error.remedy.retry')}</button>
    </div>
  {/if}
  <fieldset class="config-fields" disabled={!configReady || saving || testing} aria-busy={saving || testing}>
    <div class="setting-row" data-setting-key="settings.ai.provider">
      <div class="setting-col">
        <label class="setting-label" for="ai-provider">{$t('settings.ai.provider')}</label>
      </div>
      <select id="ai-provider" class="input-text select" value={provider} disabled={saving || testing} onchange={(event) => onProviderChange(event.currentTarget.value as Provider)}>
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
          <span class="setting-path wrap">{$t('settings.ai.model_desc')}</span>
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

    {#if provider !== "none" && savedConfig?.provider !== "none" && !configDirty}
      <div class="divider"></div>
      <p class="connection-status" data-state={connectionStatus} role={connectionStatus === "error" ? "alert" : "status"} data-setting-key="settings.ai.test">
        {$t(connectionStatus === "verified" ? 'settings.rebrand.connection.verified' : connectionStatus === "error" ? 'settings.rebrand.connection.error' : 'settings.rebrand.connection.unknown')}
      </p>
    {/if}

    <div class="divider"></div>
    <div class="actions-row">
      <button class="primary-btn" disabled={!configReady || !configDirty || saving || testing} onclick={save}>
        {saving ? $t('settings.ai.saving') : $t('settings.ai.save')}
      </button>
      {#if configDirty}<button class="ghost-btn" onclick={discard}>{$t('settings.ai.discard')}</button>{/if}
      {#if provider !== "none"}
        <button class="ghost-btn" disabled={!configReady || testing || saving} onclick={runTest} aria-describedby="ai-test-cost">
          {configDirty ? $t('settings.ai.save_test') : $t('settings.rebrand.connection.test')}
        </button>
      {/if}
    </div>
    {#if provider !== "none"}<p class="setting-path wrap" id="ai-test-cost">{$t('settings.rebrand.connection.test_cost_notice')}</p>{/if}
    {#if configDirty}<p class="setting-path unsaved-state wrap" role="status">{$t('settings.ai.draft_retained')}</p>{/if}
    {#if saveFailed}<p class="feedback-error" role="alert">{$t('settings.ai.save_failed')}</p>{/if}
  </fieldset>
</SettingGroup>

<style>
  /* Rows sit directly in the group: no bordered card inside the group card. */
  .config-fields { min-width: 0; margin: 0; padding: 0; border: 0; display: flex; flex-direction: column; }
  .config-fields > .setting-row { flex-wrap: wrap; gap: var(--space-2) var(--space-4); }
  /* Rows are always separated by a .divider. */
  .config-fields > .divider + .setting-row { border-block-start: var(--hairline, 1px) solid var(--separator); }
  .config-fields .setting-col { flex: 1 1 14rem; }
  .ai-state { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2) var(--space-3); padding-block: var(--space-2); }
  .ai-state p { margin: 0; }
  .credential-input { display: flex; flex-wrap: wrap; gap: 8px; }
  .credential-input .input-text { min-width: 0; flex: 1; }
  .wrap { white-space: normal; overflow: visible; line-height: 1.5; }
  .feedback-error { color: var(--error); }
  /* The provider select grows to its longest option; the row wraps before it would clip. */
  .config-fields .select { flex: 0 1 auto; min-width: 160px; max-width: 100%; }
  .actions-row { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; width: 100%; margin-block-start: var(--space-3); }
  .primary-btn {
    min-block-size: var(--control-h, 32px);
    padding: 0 16px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 600;
    cursor: pointer;
  }
  .primary-btn:disabled,
  .ghost-btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .ghost-btn {
    min-block-size: var(--control-h, 32px);
    padding: 0 16px;
    border: none;
    border-radius: var(--border-radius);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  .unsaved-state { margin: 10px 0 0; }
  .link-btn { color: var(--accent); font-size: 13px; text-decoration: none; white-space: nowrap; }
  .link-btn:focus-visible { outline: var(--focus-ring); }
  .connection-status { margin: 0; font-size: 13px; color: var(--text-muted); }
  .connection-status[data-state="verified"] { color: var(--text); }
  .connection-status[data-state="error"] { color: var(--error); }
</style>
