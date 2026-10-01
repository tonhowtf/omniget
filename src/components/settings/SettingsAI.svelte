<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import SettingGroup from "./rebrand/SettingGroup.svelte";

  import {
    aiConnectionStatus, clearSavedAiSettingsDraft, isAiSettingsDirty, isLoopbackAiEndpoint,
    loadAiConnection, providerForKind, readAiSettingsDraft, retainAiSettingsDraft, saveAiConnection,
    testAiConnection,
    type AiConfigView as ConfigView, type AiKind, type AiSettingsDraft,
  } from "./settings-ai-form";

  const MODEL_HINTS: Record<string, string> = {
    openai: "gpt-4o-mini",
    anthropic: "claude-sonnet-4-5",
    deepseek: "deepseek-chat",
    openrouter: "openai/gpt-4o-mini",
    gemini: "gemini-2.5-flash",
    groq: "llama-3.3-70b-versatile",
    xai: "grok-3",
    mistral: "mistral-large-latest",
    siliconflow: "deepseek-ai/DeepSeek-V3",
    newapi: "gpt-4o-mini",
    ollama: "llama3.2",
  };

  let savedConfig = $state<ConfigView | null>(null);
  let kinds = $state<AiKind[]>([]);
  let kind = $state("none");
  let provider = $derived(providerForKind(kind, kinds));
  let selectedKind = $derived(kinds.find(k => k.id === kind));
  let savedKind = $derived(savedConfig ? kindOf(savedConfig) : "none");
  let acceptsKey = $derived(selectedKind ? selectedKind.accepts_key : kind !== "ollama");
  let urlEditable = $derived(selectedKind ? selectedKind.base_url_editable : true);
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

  let configReady = $state(false);
  let saving = $state(false);
  let testing = $state(false);
  let testRevision = $state(0);
  function draft(): AiSettingsDraft {
    return { provider, kind: kind === "none" ? "" : kind, model, localBaseUrl, openaiKey, anthropicKey, removeOpenaiKey, removeAnthropicKey };
  }
  function applyDraft(d: AiSettingsDraft) {
    kind = d.provider === "none" ? "none" : d.kind || (savedConfig ? kindOf({ ...savedConfig, provider: d.provider, kind: "" }) : d.provider);
    model = d.model;
    localBaseUrl = d.localBaseUrl;
    openaiKey = d.openaiKey;
    anthropicKey = d.anthropicKey;
    removeOpenaiKey = d.removeOpenaiKey;
    removeAnthropicKey = d.removeAnthropicKey;
  }
  let configDirty = $derived(isAiSettingsDirty(draft(), savedConfig));
  let keyStored = $derived(kind === savedKind);
  let storedOpenaiKey = $derived(hasOpenaiKey && keyStored);
  let storedAnthropicKey = $derived(hasAnthropicKey && keyStored);

  function kindOf(c: ConfigView): string {
    if (c.provider === "none") return "none";
    return c.kind || (c.provider === "local" ? "custom" : c.provider);
  }

  async function loadKinds() {
    try {
      const list = await invoke<AiKind[]>("tool_keys_kinds");
      kinds = Array.isArray(list) ? list : [];
    } catch {
      kinds = [];
    }
  }
  let connectionStatus = $derived.by(() => { void testRevision; return aiConnectionStatus(savedConfig); });

  function applyConfig(c: ConfigView) {
    kind = kindOf(c);
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
    loadKinds();
    loadConfig();
  });

  function changeKind(next: string) {
    if (next === kind) return;
    kind = next;
    openaiKey = "";
    anthropicKey = "";
    removeOpenaiKey = false;
    removeAnthropicKey = false;
    if (savedConfig && next === savedKind) {
      localBaseUrl = savedConfig.local_base_url;
      return;
    }
    const row = kinds.find(k => k.id === next);
    localBaseUrl = row?.base_url_editable && next === "ollama" ? row.base_url : "";
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
      <select id="ai-provider" class="input-text select" value={kind} disabled={saving || testing} onchange={(event) => changeKind(event.currentTarget.value)}>
        <option value="none">{$t('settings.ai.provider_none')}</option>
        {#if kinds.length}
          {#each kinds as k (k.id)}
            <option value={k.id}>{k.id === "custom" ? $t('settings.ai.provider_endpoint') : k.name}</option>
          {/each}
          {#if kind !== "none" && !kinds.some(k => k.id === kind)}<option value={kind}>{kind}</option>{/if}
        {:else}
          <option value="openai">OpenAI</option>
          <option value="anthropic">Anthropic</option>
          <option value="custom">{$t('settings.ai.provider_endpoint')}</option>
          {#if !["none", "openai", "anthropic", "custom"].includes(kind)}<option value={kind}>{kind}</option>{/if}
        {/if}
      </select>
    </div>
    {#if provider === "none" && (hasOpenaiKey || hasAnthropicKey)}
      <p class="setting-path wrap">{$t('settings.ai.off_keeps_keys')}</p>
    {/if}

    {#if provider !== "none"}
      <div class="divider"></div>
      <div class="setting-row" data-setting-key="settings.ai.model">
        <div class="setting-col">
          <label class="setting-label" for="ai-model">{$t('settings.ai.model')}</label>
          <span class="setting-path wrap">{$t('settings.ai.model_desc')}</span>
        </div>
        <input type="text" class="input-text" placeholder={MODEL_HINTS[kind] ?? $t('settings.ai.model_placeholder')} id="ai-model" bind:value={model} disabled={saving || testing} />
      </div>
    {/if}

    {#if (provider === "openai" || provider === "local") && acceptsKey}
      <div class="divider"></div>
      <div class="setting-row" data-setting-key="settings.ai.connection_key">
        <div class="setting-col">
          <label class="setting-label" for="ai-openai-key">{$t('settings.ai.connection_key')}</label>
          <span class="setting-path wrap" id="ai-openai-key-hint">{removeOpenaiKey ? $t('settings.ai.key_remove_pending') : storedOpenaiKey ? $t('settings.ai.key_set') : $t('settings.ai.key_unset')}{#if selectedKind?.env && selectedKind.needs_key} · {selectedKind.env}{/if}</span>
        </div>
        <div class="credential-input">
          <input id="ai-openai-key" type="password" class="input-text" autocomplete="off" placeholder={storedOpenaiKey && !removeOpenaiKey ? "••••••••" : ""} bind:value={openaiKey} disabled={saving || testing || removeOpenaiKey} aria-describedby="ai-openai-key-hint" />
          {#if storedOpenaiKey}
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
          <span class="setting-path wrap" id="ai-anthropic-key-hint">{removeAnthropicKey ? $t('settings.ai.key_remove_pending') : storedAnthropicKey ? $t('settings.ai.key_set') : $t('settings.ai.key_unset')}</span>
        </div>
        <div class="credential-input">
          <input id="ai-anthropic-key" type="password" class="input-text" autocomplete="off" placeholder={storedAnthropicKey && !removeAnthropicKey ? "••••••••" : ""} bind:value={anthropicKey} disabled={saving || testing || removeAnthropicKey} aria-describedby="ai-anthropic-key-hint" />
          {#if storedAnthropicKey}
            <button type="button" class="ghost-btn" onclick={() => { removeAnthropicKey = !removeAnthropicKey; anthropicKey = ""; }}>
              {removeAnthropicKey ? $t('settings.ai.keep_key') : $t('settings.ai.remove_key')}
            </button>
          {/if}
        </div>
      </div>
    {/if}

    {#if provider === "local" && urlEditable}
      <div class="divider"></div>
      <div class="setting-row" data-setting-key="settings.ai.endpoint_url">
        <div class="setting-col">
          <label class="setting-label" for="ai-endpoint">{$t('settings.ai.endpoint_url')}</label>
          <span class="setting-path wrap">{$t('settings.ai.endpoint_url_desc')}</span>
          {#if localBaseUrl.trim()}<span class="setting-path wrap">{$t(isLoopbackAiEndpoint(localBaseUrl) ? 'settings.ai.endpoint_loopback' : 'settings.ai.endpoint_remote')}</span>{/if}
        </div>
        <input type="text" class="input-text" placeholder={selectedKind && selectedKind.id !== "custom" ? selectedKind.base_url : "http://localhost:11434/v1"} id="ai-endpoint" bind:value={localBaseUrl} disabled={saving || testing} />
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
  .config-fields { min-width: 0; margin: 0; padding: 0; border: 0; display: flex; flex-direction: column; }
  .config-fields > .setting-row { flex-wrap: wrap; gap: var(--space-2) var(--space-4); }
  .config-fields > .divider + .setting-row { border-block-start: var(--hairline, 1px) solid var(--separator); }
  .config-fields .setting-col { flex: 1 1 14rem; }
  .ai-state { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2) var(--space-3); padding-block: var(--space-2); }
  .ai-state p { margin: 0; }
  .credential-input { display: flex; flex-wrap: wrap; gap: 8px; }
  .credential-input .input-text { min-width: 0; flex: 1; }
  .wrap { white-space: normal; overflow: visible; line-height: 1.5; }
  .feedback-error { color: var(--error); }
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
