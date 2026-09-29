<script lang="ts">
  /**
   * Provider + model picker.
   *
   * Providers come from the AI-keys table (`tool_keys_kinds`) and the keys the
   * user already saved (`tool_keys_list`); the model list is only fetched when
   * the user clicks Load, never on mount — `llm_models_list` hits the provider.
   * The model id also stays editable when a provider is offline or returns
   * no models. Empty results and failures are reported separately.
   */
  import { onMount, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { isLocalModelProvider, loadProviderModels } from "$lib/llm/provider-models";
  import type { ModelRef } from "$lib/llm/types";

  let {
    value = null,
    explicit = false,
    busy = false,
    connectHref = "/llm/accounts?guide=api",
    onconnect,
    onchange,
  }: {
    value?: ModelRef | null;
    explicit?: boolean;
    busy?: boolean;
    connectHref?: string;
    onconnect?: () => void;
    onchange: (ref: ModelRef) => void;
  } = $props();

  type Kind = { id: string; name: string };
  type KeyView = { id: string; kind: string; name: string; has_key?: boolean };

  let kinds = $state<Kind[]>([]);
  let saved = $state<KeyView[]>([]);
  // Seeded once: after mount the two fields are the source of truth and every
  // change is pushed back through `onchange`.
  let provider = $state(untrack(() => value?.provider ?? ""));
  let model = $state(untrack(() => value?.model ?? ""));
  let models = $state<string[]>([]);
  let loading = $state(false);
  let providerError = $state(false);
  let providersLoading = $state(true);
  let loadError = $state<string | null>(null);
  let modelsLoaded = $state(false);
  let requestId = 0;
  const optionsId = $props.id();

  async function loadProviders() {
    providerError = false;
    providersLoading = true;
    try {
      kinds = (await invoke<Kind[] | null>("tool_keys_kinds")) ?? [];
    } catch {
      providerError = true;
    }
    try {
      saved = (await invoke<KeyView[] | null>("tool_keys_list")) ?? [];
    } catch {
      providerError = true;
    }
    if (!provider) provider = saved.find(key => key.has_key !== false)?.id ?? "ollama";
    providersLoading = false;
  }

  async function loadModels() {
    if (!provider) return;
    const request = ++requestId;
    const requestedProvider = provider;
    loading = true;
    modelsLoaded = false;
    loadError = null;
    models = [];
    try {
      const list = await loadProviderModels(requestedProvider);
      if (request !== requestId) return;
      models = list;
      modelsLoaded = true;
    } catch (error) {
      if (request !== requestId) return;
      loadError = error instanceof Error ? error.message : String(error);
    } finally {
      if (request === requestId) loading = false;
    }
  }

  function changeProvider() {
    requestId++;
    models = [];
    modelsLoaded = false;
    loadError = null;
    loading = false;
    model = "";
    if (!explicit) onchange({ provider, model: "" });
  }

  function commit(force = false) {
    if (explicit && !force) return;
    if (!provider) return;
    onchange({ provider, model: model.trim() });
  }

  let hasKey = $derived(saved.some((k) => (k.id === provider || k.kind === provider) && k.has_key !== false));

  onMount(() => {
    void loadProviders();
  });
</script>

<div class="picker">
  {#if providersLoading}<p class="picker-note" role="status">{$t("llm.models.loading")}</p>{/if}
  {#if providerError}<div class="provider-error" role="alert"><span>{$t("llm.models.providers_failed")}</span><button type="button" class="button" disabled={providersLoading} onclick={loadProviders}>{$t("llm.surface.retry")}</button></div>{/if}
  <label class="field">
    <span class="field-label">{$t("llm.roster.provider")}</span>
    <select class="input" disabled={busy || providersLoading} bind:value={provider} onchange={changeProvider}>
      {#if provider && !kinds.some(kind => kind.id === provider) && !saved.some(key => key.id === provider) && !isLocalModelProvider(provider)}<option value={provider}>{provider}</option>{/if}
      {#if saved.length}
        <optgroup label={$t("llm.models.saved_connections")}>
          {#each saved as key (key.id)}<option value={key.id}>{key.name || key.kind} · {kinds.find(kind => kind.id === key.kind)?.name ?? key.kind}</option>{/each}
        </optgroup>
      {/if}
      <optgroup label={$t("llm.accounts.wizard.local_model")}>
        <option value="ollama">Ollama</option><option value="lmstudio">LM Studio</option><option value="llama-server">llama.cpp</option>
      </optgroup>
      <optgroup label={$t("llm.models.other_providers")}>
        {#each kinds.filter(kind => !isLocalModelProvider(kind.id) && !saved.some(key => key.id === kind.id)) as kind (kind.id)}<option value={kind.id}>{kind.name}</option>{/each}
      </optgroup>
    </select>
    {#if isLocalModelProvider(provider)}
      <span class="field-hint">{$t("llm.models.local_hint")}</span>
    {:else if !hasKey && provider && !providersLoading && !providerError}
      <span class="field-hint">{$t("llm.models.add_key_hint")}</span>
      <a href={connectHref} onclick={() => onconnect?.()}>{$t("llm.surface.connect")}</a>
    {/if}
  </label>

  <label class="field">
    <span class="field-label">{$t("llm.roster.model")}</span>
    <input class="input" disabled={busy} bind:value={model} list={optionsId} onchange={() => commit()} aria-describedby={`${optionsId}-hint`} />
    <span id={`${optionsId}-hint`} class="field-hint">{$t("llm.models.choose_or_enter")}</span>
    <datalist id={optionsId}>
      {#each models as m (m)}
        <option value={m}></option>
      {/each}
    </datalist>
  </label>

  <div class="picker-actions">
    {#if explicit}<button type="button" class="button primary" disabled={busy || !provider || !model.trim()} onclick={() => commit(true)}>{$t("llm.conv.switch_model")}</button>{/if}
    <button type="button" class="button" onclick={loadModels} disabled={busy || providersLoading || loading || !provider}>
      {loading ? $t("llm.models.loading") : $t("llm.models.load")}
    </button>
    {#if models.length > 0}
      <span class="picker-note">{$t("llm.models.count", { count: models.length })}</span>
    {:else if loadError}
      <span class="picker-note" role="alert">{$t("llm.models.load_failed", { error: loadError })}</span>
    {:else if modelsLoaded}
      <span class="picker-note" role="status">{$t("llm.models.no_models")}</span>
    {/if}
  </div>
</div>

<style>
  .picker {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .picker-actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .provider-error { display:flex; align-items:center; gap:12px; flex-wrap:wrap; font-size:13px; }

  .picker-note {
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
</style>
