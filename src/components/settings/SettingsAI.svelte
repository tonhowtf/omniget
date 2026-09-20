<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

  // the provider table the LLM layer routes on (ai_keys::KINDS). Kept in sync by
  // reading it instead of repeating it here: the list was hardcoded to three entries
  // while twelve exist, which is what left deepseek and the other eight unreachable
  // from this panel even though every other surface could already use them.
  type Kind = { id: string; name: string; base_url: string; balance: boolean; env: string };
  // a model name is not available from the registry, so these are placeholders only -
  // the field stays free text and whatever the provider accepts is what works
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
    ollama: "llama3.2"
  };
  // only the two that have to be typed point at a URL of their own; the rest have an
  // endpoint in the table or a fixed one in provider_from_config
  const NEEDS_BASE_URL = new Set(["ollama", "custom"]);
  // ollama and custom take no key; an empty key is a valid state for every other kind
  const NEEDS_KEY = new Set(["openai", "anthropic", "openrouter", "deepseek", "gemini", "groq", "xai", "mistral", "siliconflow", "newapi"]);

  type Provider = "none" | "openai" | "anthropic" | "local";
  type ConfigView = {
    provider: Provider;
    kind: string;
    model: string;
    local_base_url: string;
    has_openai_key: boolean;
    has_anthropic_key: boolean;
  };
  type HistoryEntry = {
    id: number;
    kind: string;
    url: string;
    title: string;
    content: string;
    created_at_ms: number;
  };

  let kinds = $state<Kind[]>([]);
  // "none" | a kind id. The kind is the selection; the legacy `provider` string is
  // derived from it server-side so a request still has a wire to speak.
  let kind = $state<string>("none");
  let model = $state("");
  let localBaseUrl = $state("");
  let keyInput = $state("");
  let hasKey = $state(false);

  let testing = $state(false);
  let summarizeUrl = $state("");
  let summarizing = $state(false);
  let summaryStyle = $state<"short" | "balanced" | "detailed">("balanced");
  let summaryLang = $state("");
  let history = $state<HistoryEntry[]>([]);

  const selectedKind = $derived(kinds.find((k) => k.id === kind));
  const needsBaseUrl = $derived(NEEDS_BASE_URL.has(kind));
  const needsKey = $derived(kind !== "none" && NEEDS_KEY.has(kind));

  async function loadKinds() {
    try {
      kinds = await invoke<Kind[]>("tool_keys_kinds");
    } catch {
      kinds = [];
    }
  }

  async function loadConfig() {
    try {
      const c = await invoke<ConfigView>("ai_get_config");
      kind = c.kind || c.provider;
      model = c.model;
      localBaseUrl = c.local_base_url;
      hasKey = c.provider === "anthropic" ? c.has_anthropic_key : c.has_openai_key;
      keyInput = "";
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : $t("common.error"));
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
    loadKinds();
    loadConfig();
    loadHistory();
  });

  // the endpoint belongs to the provider, so it follows the choice instead of being
  // retyped. `custom` and `ollama` are the two that keep whatever the user typed.
  function onKindChange() {
    const k = selectedKind;
    if (!k) return;
    if (!needsBaseUrl) {
      localBaseUrl = "";
    } else if (!localBaseUrl.trim()) {
      localBaseUrl = k.base_url;
    }
  }

  async function save() {
    try {
      await invoke("ai_set_config", {
        provider: kind === "none" ? "none" : "local",
        kind: kind === "none" ? null : kind,
        model: model.trim(),
        localBaseUrl: localBaseUrl.trim(),
        openaiKey: keyInput.trim() !== "" ? keyInput.trim() : null,
        anthropicKey: keyInput.trim() !== "" ? keyInput.trim() : null,
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
      <select class="input-text select" bind:value={kind} onchange={onKindChange}>
        <option value="none">{$t('settings.ai.provider_none')}</option>
        {#each kinds as k (k.id)}
          <option value={k.id}>{k.name}</option>
        {/each}
      </select>
    </div>

    {#if kind !== "none"}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.ai.model')}</span>
          <span class="setting-path">{$t('settings.ai.model_desc')}</span>
        </div>
        <input
          type="text"
          class="input-text"
          placeholder={MODEL_HINTS[kind] ?? ($t('settings.ai.model_placeholder') as string)}
          bind:value={model}
        />
      </div>
    {/if}

    {#if needsKey}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.ai.api_key')}</span>
          <span class="setting-path">
            {hasKey ? $t('settings.ai.key_set') : $t('settings.ai.key_unset')}
            {#if selectedKind}· {selectedKind.env}{/if}
          </span>
        </div>
        <input type="password" class="input-text" placeholder={hasKey ? "••••••••" : "sk-…"} bind:value={keyInput} />
      </div>
    {/if}

    {#if needsBaseUrl}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.ai.local_url')}</span>
          <span class="setting-path">{$t('settings.ai.local_url_desc')}</span>
        </div>
        <input
          type="text"
          class="input-text"
          placeholder={selectedKind?.base_url ?? "http://localhost:11434/v1"}
          bind:value={localBaseUrl}
        />
      </div>
    {/if}

    <div class="divider"></div>
    <div class="actions-row">
      <button class="primary-btn" onclick={save}>{$t('settings.ai.save')}</button>
      {#if kind !== "none"}
        <button class="ghost-btn" disabled={testing} onclick={runTest}>{$t('settings.ai.test')}</button>
      {/if}
    </div>
  </div>

  {#if kind !== "none"}
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
