<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    ENTRANCE_ATTACHMENTS,
    capabilityFor,
    catalogStatus,
    optionsForEntrance,
    reconcileForEntrance,
    type CatalogEntrance,
    type CatalogView,
    type ModelOption,
  } from "$lib/orcarouter";

  type Provider = "none" | "openai" | "anthropic" | "local" | "orcarouter" | "orcarouter-auth";
  type ConfigView = {
    provider: Provider;
    model: string;
    local_base_url: string;
    has_openai_key: boolean;
    has_anthropic_key: boolean;
    has_orcarouter_key: boolean;
    orcarouter_key_masked: string;
    orcarouter_method: string;
    orcarouter_account: string;
    orcarouter_scope: string;
    orcarouter_generation: number;
    orcarouter_needs_reauth: boolean;
    orcarouter_auth_base: string;
    orcarouter_api_base: string;
  };
  type HistoryEntry = {
    id: number;
    kind: string;
    url: string;
    title: string;
    content: string;
    created_at_ms: number;
  };
  type LoginPending = {
    authorize_url: string;
    callback_url: string;
    auth_base: string;
    api_base: string;
    attempt: number;
  };
  type LoginOutcome = {
    ok: boolean;
    message: string;
    attempt: number;
    denied: boolean;
    connected: boolean;
  };

  let provider = $state<Provider>("none");
  let model = $state("");
  let localBaseUrl = $state("");
  let openaiKey = $state("");
  let anthropicKey = $state("");
  let orcaKey = $state("");
  let hasOpenaiKey = $state(false);
  let hasAnthropicKey = $state(false);
  let config = $state<ConfigView | null>(null);

  let testing = $state(false);
  let summarizeUrl = $state("");
  let summarizing = $state(false);
  let summaryStyle = $state<"short" | "balanced" | "detailed">("balanced");
  let summaryLang = $state("");
  let history = $state<HistoryEntry[]>([]);

  // -- OrcaRouter model catalog (live, capability-filtered) -----------------
  let catalog = $state<CatalogView | null>(null);
  let catalogLoading = $state(false);
  let catalogError = $state<string | null>(null);

  // -- OrcaRouter sign-in ---------------------------------------------------
  let loginPending = $state<LoginPending | null>(null);
  let loginBusy = $state(false);
  let loginHint = $state<string | null>(null);
  /**
   * Monotonic attempt id. Every async response and poll iteration confirms it
   * still belongs to the current generation before touching state, so a late
   * URL or outcome from a superseded attempt cannot appear under a newer one.
   */
  let loginGeneration = 0;

  const isOrca = $derived(provider === "orcarouter" || provider === "orcarouter-auth");

  /**
   * The entrance the selector is currently serving. This repository has no
   * media-attaching entrance today, so it is plain chat; `capabilityFor` is
   * still what decides, so an entrance that starts attaching media follows the
   * same path and the options change with it.
   */
  const orcaEntrance = $derived.by((): CatalogEntrance => {
    const req = capabilityFor("chat", ENTRANCE_ATTACHMENTS.chat ?? 0);
    return req.capability === "multimodal" ? "multimodal" : "chat";
  });

  /**
   * The options actually rendered. They are re-derived from the directory
   * response for the current entrance, so the list cannot drift from the
   * capability rule even if a stale catalog is on screen.
   */
  const orcaOptions = $derived<ModelOption[]>(optionsForEntrance(catalog, orcaEntrance));

  async function loadConfig() {
    try {
      const c = await invoke<ConfigView>("ai_get_config");
      config = c;
      provider = c.provider;
      model = c.model;
      localBaseUrl = c.local_base_url;
      hasOpenaiKey = c.has_openai_key;
      hasAnthropicKey = c.has_anthropic_key;
      openaiKey = "";
      anthropicKey = "";
      orcaKey = "";
      if (c.provider === "orcarouter" || c.provider === "orcarouter-auth") {
        await loadCatalog();
      }
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : $t("common.error"));
    }
  }

  /**
   * Resolve the catalog for the entrance currently selected.
   *
   * The `capability` argument is what makes the dropdown match the entrance: a
   * text entrance asks for chat, and an entrance that has media attached asks
   * for chat restricted to that modality, so a model that never declared the
   * modality fails closed and never appears.
   */
  async function loadCatalog(force = false) {
    const gen = ++loginGeneration;
    catalogLoading = true;
    catalogError = null;
    try {
      const attachments = ENTRANCE_ATTACHMENTS.chat ?? 0;
      const req = capabilityFor("chat", attachments);
      const view = await invoke<CatalogView>("orcarouter_models", {
        capability: req.capability,
        modality: req.modality ?? null,
        force,
      });
      if (gen !== loginGeneration) return;
      catalog = view;
      // A selection that is no longer compatible is cleared rather than kept.
      const kept = reconcileForEntrance(view, model, orcaEntrance);
      if (kept === null && model.trim() !== "") {
        model = "";
        showToast("info", $t("settings.ai.orca_model_cleared") as string);
      } else if (kept !== null) {
        model = kept;
      }
      // A freshly connected workspace has nothing stored yet, and a cleared
      // model leaves the selector blank: in both cases the first compatible
      // model is preselected so the trigger always shows a real choice.
      if (model.trim() === "" && orcaOptions.length > 0) {
        model = orcaOptions[0].id;
      }
    } catch (e: any) {
      if (gen !== loginGeneration) return;
      catalogError = typeof e === "string" ? e : $t("common.error");
    } finally {
      if (gen === loginGeneration) catalogLoading = false;
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
    loadConfig();
    loadHistory();
    // A pagehide means the webview may be put into the back-forward cache.
    // Clear the busy flag and the hint synchronously here: the invalidated
    // request's own cleanup correctly refuses to mutate state, which would
    // otherwise leave a restored page permanently busy.
    const onPageHide = () => {
      loginGeneration++;
      loginBusy = false;
      loginPending = null;
      loginHint = null;
      invoke("orcarouter_login_pagehide").catch(() => {});
    };
    window.addEventListener("pagehide", onPageHide);
    return () => {
      window.removeEventListener("pagehide", onPageHide);
      // A real unmount cancels the server work without writing UI state.
      invoke("orcarouter_login_cancel").catch(() => {});
    };
  });

  async function save() {
    try {
      await invoke("ai_set_config", {
        provider,
        model: model.trim(),
        localBaseUrl: localBaseUrl.trim(),
        openaiKey: openaiKey.trim() !== "" ? openaiKey.trim() : null,
        anthropicKey: anthropicKey.trim() !== "" ? anthropicKey.trim() : null,
        orcarouterKey: orcaKey.trim() !== "" ? orcaKey.trim() : null,
      });
      await loadConfig();
      showToast("success", $t("settings.ai.saved") as string);
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : $t("common.error"));
    }
  }

  /** Save a pasted key through the same credential seam the PKCE flow uses. */
  async function saveOrcaKey() {
    try {
      config = await invoke<ConfigView>("orcarouter_set_api_key", {
        provider: "orcarouter",
        model: model.trim(),
        key: orcaKey.trim(),
      });
      orcaKey = "";
      await loadCatalog(true);
      showToast("success", $t("settings.ai.saved") as string);
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : $t("common.error"));
    }
  }

  async function signOut() {
    try {
      config = await invoke<ConfigView>("orcarouter_sign_out");
      catalog = null;
      showToast("success", $t("settings.ai.orca_signed_out") as string);
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : $t("common.error"));
    }
  }

  /**
   * Start the OAuth 2.0 + PKCE sign-in and await its outcome.
   *
   * The authorize URL is shown as well as opened, so a machine whose browser
   * does not launch automatically is not a dead end.
   */
  async function connect() {
    if (loginBusy) return;
    const gen = ++loginGeneration;
    loginBusy = true;
    loginHint = null;
    loginPending = null;
    try {
      const pending = await invoke<LoginPending>("orcarouter_login_begin");
      if (gen !== loginGeneration) return;
      loginPending = pending;
      loginHint = $t("settings.ai.orca_waiting") as string;
      const outcome = await invoke<LoginOutcome>("orcarouter_login_finish", {
        attempt: pending.attempt,
      });
      // A response from a superseded generation must not write anything.
      if (gen !== loginGeneration) return;
      await loadConfig();
      if (outcome.connected) {
        showToast("success", $t("settings.ai.orca_connected") as string);
      } else if (outcome.denied) {
        showToast("info", $t("settings.ai.orca_denied") as string);
      } else {
        showToast("error", outcome.message);
      }
    } catch (e: any) {
      if (gen !== loginGeneration) return;
      showToast("error", typeof e === "string" ? e : $t("common.error"));
    } finally {
      // Another pagehide may have superseded this attempt; if so, leave the
      // state it already cleared alone.
      if (gen === loginGeneration) {
        loginBusy = false;
        loginPending = null;
        loginHint = null;
      }
    }
  }

  async function cancelLogin() {
    loginGeneration++;
    loginBusy = false;
    loginPending = null;
    loginHint = null;
    try {
      await invoke("orcarouter_login_cancel");
    } catch {
      // The lock is already released locally; a failed round trip is not fatal.
    }
    showToast("info", $t("settings.ai.orca_cancelled") as string);
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
      <select class="input-text select" bind:value={provider} onchange={() => isOrca && loadCatalog()}>
        <option value="none">{$t('settings.ai.provider_none')}</option>
        <option value="openai">OpenAI</option>
        <option value="anthropic">Anthropic</option>
        <option value="local">{$t('settings.ai.provider_local')}</option>
        <option value="orcarouter">{$t('settings.ai.provider_orcarouter_api')}</option>
        <option value="orcarouter-auth">{$t('settings.ai.provider_orcarouter_auth')}</option>
      </select>
    </div>

    {#if isOrca}
      <div class="divider"></div>
      <!--
        Both entrances are shown together, side by side, inside the real
        OrcaRouter configuration: a user with an existing key never has to
        start a browser login, and a user without one never has to go and mint
        a key by hand. Whichever is used, the same sk-orca-… key reaches the
        relay.
      -->
      <div class="orca-auth" data-testid="orca-auth-methods">
        <div class="orca-method" data-testid="orca-method-api-key">
          <span class="setting-label">{$t('settings.ai.orca_api_key_title')}</span>
          <span class="setting-path">{$t('settings.ai.orca_api_key_desc')}</span>
          <div class="orca-row">
            <input
              type="password"
              class="input-text"
              data-testid="orca-api-key-input"
              placeholder={config?.has_orcarouter_key ? "••••••••" : "sk-orca-…"}
              bind:value={orcaKey}
            />
            <button
              class="primary-btn"
              data-testid="orca-api-key-save"
              disabled={orcaKey.trim() === ""}
              onclick={saveOrcaKey}
            >
              {$t('settings.ai.orca_api_key_save')}
            </button>
          </div>
          {#if config?.has_orcarouter_key && config?.orcarouter_method === "api_key"}
            <span class="setting-path orca-state" data-testid="orca-api-key-state">
              {$t('settings.ai.orca_key_set', { masked: config.orcarouter_key_masked })}
            </span>
            <button class="link-btn danger" data-testid="orca-sign-out" onclick={signOut}>
              {$t('settings.ai.orca_sign_out')}
            </button>
          {/if}
        </div>

        <div class="orca-method" data-testid="orca-method-pkce">
          <span class="setting-label">{$t('settings.ai.orca_pkce_title')}</span>
          <span class="setting-path">{$t('settings.ai.orca_pkce_desc')}</span>
          <div class="orca-row">
            {#if loginBusy}
              <button class="ghost-btn" data-testid="orca-login-cancel" onclick={cancelLogin}>
                {$t('settings.ai.orca_cancel')}
              </button>
            {:else}
              <button class="primary-btn" data-testid="orca-login-start" onclick={connect}>
                {$t('settings.ai.orca_connect')}
              </button>
            {/if}
          </div>
          {#if loginHint}
            <span class="setting-path orca-state" data-testid="orca-login-hint">{loginHint}</span>
          {/if}
          {#if loginPending}
            <!-- Shown as well as opened, so a machine with no browser handler
                 is not a dead end. -->
            <code class="orca-url" data-testid="orca-login-url">{loginPending.authorize_url}</code>
          {/if}
          {#if config?.has_orcarouter_key && config?.orcarouter_method === "pkce"}
            <span class="setting-path orca-state" data-testid="orca-pkce-state">
              {$t('settings.ai.orca_account_connected', {
                account: config.orcarouter_account || "—",
                masked: config.orcarouter_key_masked,
              })}
            </span>
            <button class="link-btn danger" data-testid="orca-pkce-sign-out" onclick={signOut}>
              {$t('settings.ai.orca_sign_out')}
            </button>
          {/if}
        </div>
      </div>

      {#if config?.orcarouter_needs_reauth}
        <div class="orca-banner" data-testid="orca-reauth-banner">
          {$t('settings.ai.orca_reauth')}
        </div>
      {/if}

      <div class="divider"></div>
    {/if}

    {#if provider !== "none" && !isOrca}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.ai.model')}</span>
          <span class="setting-path">{$t('settings.ai.model_desc')}</span>
        </div>
        <input type="text" class="input-text" placeholder={$t('settings.ai.model_placeholder')} bind:value={model} />
      </div>
    {/if}

    {#if isOrca}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.ai.model')}</span>
          <span class="setting-path">
            {$t('settings.ai.orca_model_desc')}
            {#if catalog}
              {' · '}
              <span class:degraded={catalog.degraded} data-testid="orca-catalog-status">
                {$t(catalogStatus(catalog).labelKey, { count: orcaOptions.length })}
              </span>
            {/if}
          </span>
        </div>
        <!-- A dropdown built from the live directory, never a free-text field:
             a typed model id cannot be checked against the catalog. -->
        <select
          class="input-text select"
          data-testid="orca-model-select"
          disabled={catalogLoading || orcaOptions.length === 0}
          bind:value={model}
        >
          {#if orcaOptions.length === 0}
            <option value="">
              {catalogLoading ? $t('settings.ai.orca_models_loading') : $t('settings.ai.orca_models_empty')}
            </option>
          {:else}
            {#each orcaOptions as m (m.id)}
              <option value={m.id}>{m.name}</option>
            {/each}
          {/if}
        </select>
      </div>
      {#if catalogError}
        <span class="setting-path orca-state" data-testid="orca-catalog-error">{catalogError}</span>
      {/if}
      <div class="actions-row">
        <button
          class="ghost-btn"
          data-testid="orca-catalog-refresh"
          disabled={catalogLoading}
          onclick={() => loadCatalog(true)}
        >
          {$t('settings.ai.orca_models_refresh')}
        </button>
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

  /* The two OrcaRouter entrances sit side by side in a bordered pair so they
     read as one provider with two ways in, rather than as two providers. */
  .orca-auth {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    width: 100%;
    padding: 4px 0;
  }

  .orca-method {
    flex: 1 1 260px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--border-radius);
  }

  .orca-row {
    display: flex;
    gap: 8px;
    width: 100%;
  }

  .orca-row .input-text {
    flex: 1;
    min-width: 0;
  }

  .orca-state {
    display: block;
    margin-top: 4px;
  }

  .orca-url {
    display: block;
    margin-top: 6px;
    padding: 6px 8px;
    border-radius: var(--border-radius);
    background: var(--surface-2, rgba(127, 127, 127, 0.12));
    font-size: 11px;
    line-height: 1.4;
    word-break: break-all;
    user-select: all;
  }

  .orca-banner {
    width: 100%;
    margin-top: 8px;
    padding: 8px 10px;
    border-radius: var(--border-radius);
    border: 1px solid var(--error);
    color: var(--error);
    font-size: 12px;
  }

  .degraded {
    color: var(--error);
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
