<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Provider = "none" | "openai" | "anthropic" | "minimax" | "local";
  type MinimaxRegion = "global_en" | "cn_zh";
  type MinimaxApiFormat = "openai" | "anthropic";
  type ConfigView = {
    provider: Provider;
    model: string;
    local_base_url: string;
    minimax_region: MinimaxRegion;
    minimax_api_format: MinimaxApiFormat;
    has_openai_key: boolean;
    has_anthropic_key: boolean;
    has_minimax_key: boolean;
  };
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
  let modelOptions = $state<string[]>([]);
  let localBaseUrl = $state("");
  let minimaxRegion = $state<MinimaxRegion>("global_en");
  let minimaxApiFormat = $state<MinimaxApiFormat>("openai");
  let openaiKey = $state("");
  let anthropicKey = $state("");
  let minimaxKey = $state("");
  let hasOpenaiKey = $state(false);
  let hasAnthropicKey = $state(false);
  let hasMinimaxKey = $state(false);

  let testing = $state(false);
  let summarizeUrl = $state("");
  let summarizing = $state(false);
  let summaryStyle = $state<"short" | "balanced" | "detailed">("balanced");
  let summaryLang = $state("");
  let history = $state<HistoryEntry[]>([]);

  async function loadConfig() {
    try {
      const c = await invoke<ConfigView>("ai_get_config");
      provider = c.provider;
      model = c.model;
      localBaseUrl = c.local_base_url;
      minimaxRegion = c.minimax_region;
      minimaxApiFormat = c.minimax_api_format;
      hasOpenaiKey = c.has_openai_key;
      hasAnthropicKey = c.has_anthropic_key;
      hasMinimaxKey = c.has_minimax_key;
      openaiKey = "";
      anthropicKey = "";
      minimaxKey = "";
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : $t("common.error"));
    }
  }

  async function loadModelOptions(nextProvider: Provider) {
    try {
      const options = await invoke<string[]>("ai_get_models", { provider: nextProvider });
      if (provider !== nextProvider) return;
      modelOptions = options;
      if (options.length > 0 && !options.includes(model)) {
        model = options[0];
      }
    } catch {
      modelOptions = [];
    }
  }

  async function loadHistory() {
    try {
      history = (await invoke<HistoryEntry[]>("ai_history_list")).slice().reverse();
    } catch {
      history = [];
    }
  }

  onMount(() => {
    void (async () => {
      await loadConfig();
      await loadModelOptions(provider);
    })();
    loadHistory();
  });

  async function save() {
    try {
      await invoke("ai_set_config", {
        provider,
        model: model.trim(),
        localBaseUrl: localBaseUrl.trim(),
        minimaxRegion,
        minimaxApiFormat,
        openaiKey: openaiKey.trim() !== "" ? openaiKey.trim() : null,
        anthropicKey: anthropicKey.trim() !== "" ? anthropicKey.trim() : null,
        minimaxKey: minimaxKey.trim() !== "" ? minimaxKey.trim() : null,
      });
      await loadConfig();
      showToast("success", $t("settings.ai.saved") as string);
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : $t("common.error"));
    }
  }

  async function runTest() {
    if (testing) return;
    testing = true;
    try {
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
    if (!url || summarizing) return;
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
    if (!confirm($t("settings.ai.history_clear_confirm"))) return;
    try {
      await invoke("ai_history_clear");
      history = [];
    } catch {
      // best-effort
    }
  }
</script>

<section class="section">
  <h5 class="section-title">{$t('settings.ai.title')}</h5>

  <div class="card">
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t('settings.ai.provider')}</span>
        <span class="setting-path">{$t('settings.ai.provider_desc')}</span>
      </div>
      <select
        class="input-text select"
        bind:value={provider}
        onchange={(event) =>
          void loadModelOptions(event.currentTarget.value as Provider)}
      >
        <option value="none">{$t('settings.ai.provider_none')}</option>
        <option value="openai">OpenAI</option>
        <option value="anthropic">Anthropic</option>
        <option value="minimax">MiniMax</option>
        <option value="local">{$t('settings.ai.provider_local')}</option>
      </select>
    </div>

    {#if provider !== "none"}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.ai.model')}</span>
          <span class="setting-path">{$t('settings.ai.model_desc')}</span>
        </div>
        {#if modelOptions.length > 0}
          <select class="input-text select" bind:value={model}>
            {#each modelOptions as modelId}
              <option value={modelId}>{modelId}</option>
            {/each}
          </select>
        {:else}
          <input type="text" class="input-text" placeholder={$t('settings.ai.model_placeholder')} bind:value={model} />
        {/if}
      </div>
    {/if}

    {#if provider === "openai"}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.ai.openai_key')}</span>
          <span class="setting-path">{hasOpenaiKey ? $t('settings.ai.key_set') : $t('settings.ai.key_unset')}</span>
        </div>
        <input type="password" class="input-text" placeholder={hasOpenaiKey ? "••••••••" : "sk-…"} bind:value={openaiKey} />
      </div>
    {/if}

    {#if provider === "anthropic"}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.ai.anthropic_key')}</span>
          <span class="setting-path">{hasAnthropicKey ? $t('settings.ai.key_set') : $t('settings.ai.key_unset')}</span>
        </div>
        <input type="password" class="input-text" placeholder={hasAnthropicKey ? "••••••••" : "sk-ant-…"} bind:value={anthropicKey} />
      </div>
    {/if}

    {#if provider === "minimax"}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.ai.minimax_key')}</span>
          <span class="setting-path">{hasMinimaxKey ? $t('settings.ai.key_set') : $t('settings.ai.key_unset')}</span>
        </div>
        <input
          type="password"
          class="input-text"
          placeholder={hasMinimaxKey ? "••••••••" : ($t('settings.ai.key_unset') as string)}
          bind:value={minimaxKey}
        />
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.ai.minimax_region')}</span>
          <span class="setting-path">{minimaxRegion}</span>
        </div>
        <select class="input-text select" bind:value={minimaxRegion}>
          <option value="global_en">global_en - api.minimax.io</option>
          <option value="cn_zh">cn_zh - api.minimaxi.com</option>
        </select>
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.ai.minimax_api_format')}</span>
          <span class="setting-path">{minimaxApiFormat}</span>
        </div>
        <select class="input-text select" bind:value={minimaxApiFormat}>
          <option value="openai">openai</option>
          <option value="anthropic">anthropic</option>
        </select>
      </div>
    {/if}

    {#if provider === "local"}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.ai.local_url')}</span>
          <span class="setting-path">{$t('settings.ai.local_url_desc')}</span>
        </div>
        <input type="text" class="input-text" placeholder="http://localhost:11434/v1" bind:value={localBaseUrl} />
      </div>
    {/if}

    <div class="divider"></div>
    <div class="actions-row">
      <button class="primary-btn" onclick={save}>{$t('settings.ai.save')}</button>
      {#if provider !== "none"}
        <button class="ghost-btn" disabled={testing} onclick={runTest}>{$t('settings.ai.test')}</button>
      {/if}
    </div>
  </div>

  {#if provider !== "none"}
    <h5 class="section-title">{$t('settings.ai.summarize_title')}</h5>
    <div class="card">
      <div class="add-row">
        <input
          type="text"
          class="input-text"
          placeholder={$t('settings.ai.summarize_placeholder')}
          bind:value={summarizeUrl}
          onkeydown={(e) => { if (e.key === "Enter") summarize(); }}
        />
        <button class="primary-btn" disabled={!summarizeUrl.trim() || summarizing} onclick={summarize}>
          {summarizing ? $t('settings.ai.summarizing') : $t('settings.ai.summarize')}
        </button>
      </div>
      <div class="add-row sum-opts">
        <select class="input-text select" bind:value={summaryStyle} aria-label={$t('settings.ai.style') as string}>
          <option value="short">{$t('settings.ai.style_short')}</option>
          <option value="balanced">{$t('settings.ai.style_balanced')}</option>
          <option value="detailed">{$t('settings.ai.style_detailed')}</option>
        </select>
        <input
          type="text"
          class="input-text"
          placeholder={$t('settings.ai.lang_placeholder')}
          bind:value={summaryLang}
        />
      </div>
      <span class="setting-path add-hint">{$t('settings.ai.summarize_hint')}</span>
    </div>
  {/if}

  <div class="hist-head">
    <h5 class="section-title">{$t('settings.ai.history')}</h5>
    {#if history.length > 0}
      <button class="link-btn danger" onclick={clearHistory}>{$t('settings.ai.history_clear')}</button>
    {/if}
  </div>
  {#if history.length === 0}
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
    border: 1px solid var(--border);
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
