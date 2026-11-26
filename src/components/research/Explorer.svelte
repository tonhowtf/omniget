<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/research/api";
  import { coerceParams, missingRequired } from "$lib/research/workflows";
  import type { Operation, PlatformRow, Result } from "$lib/research/types";
  import type { DevtoolsError } from "$lib/devtools/types";
  import ErrorNotice from "#components/devtools/ErrorNotice.svelte";
  import CopyButton from "#components/palette/CopyButton.svelte";

  let { ready, onran }: { ready: boolean; onran: () => void } = $props();

  let platforms = $state<PlatformRow[]>([]);
  let operations = $state<Operation[]>([]);
  let platform = $state("");
  let endpoint = $state("");
  let values = $state<Record<string, string>>({});
  let loading = $state(true);
  let busy = $state(false);
  let error = $state<DevtoolsError | null>(null);
  let result = $state<Result | null>(null);

  let operation = $derived(operations.find((op) => op.id === endpoint) ?? null);
  let missing = $derived(operation ? missingRequired(operation.params, values) : []);
  let json = $derived(result ? (typeof result.preview === "string" ? result.preview : JSON.stringify(result.data, null, 2)) : "");
  let nextPage = $derived(result?.next_page && typeof result.next_page === "object" ? (result.next_page as Record<string, string | number>) : null);

  async function loadPlatforms() {
    loading = true;
    try {
      platforms = (await api.tool<{ platforms: PlatformRow[] }>("research_endpoints")).platforms ?? [];
      error = null;
    } catch (e) {
      error = api.parseError(e);
    } finally {
      loading = false;
    }
  }

  async function pickPlatform() {
    endpoint = "";
    operations = [];
    values = {};
    result = null;
    if (!platform) return;
    loading = true;
    try {
      operations = (await api.tool<{ operations: Operation[] }>("research_endpoints", { platform, limit: 100 })).operations ?? [];
      error = null;
    } catch (e) {
      error = api.parseError(e);
    } finally {
      loading = false;
    }
  }

  function pickEndpoint() {
    values = {};
    result = null;
    error = null;
  }

  async function run() {
    if (!operation || busy || !ready || missing.length) return;
    busy = true;
    try {
      result = await api.tool("research_call", { endpoint: operation.id, params: coerceParams(operation.params, values), maxChars: 20_000 });
      error = null;
    } catch (e) {
      error = api.parseError(e);
      result = null;
    } finally {
      busy = false;
      onran();
    }
  }

  function useNextPage() {
    if (!nextPage) return;
    for (const [name, token] of Object.entries(nextPage)) values[name] = String(token);
    void run();
  }

  $effect(() => {
    void loadPlatforms();
  });
</script>

<form
  class="dv-panel"
  onsubmit={(e) => {
    e.preventDefault();
    void run();
  }}
>
  <div class="dv-row">
    <label class="dv-field">
      {$t("research.ex_platform")}
      <select class="input" bind:value={platform} onchange={() => void pickPlatform()} disabled={loading && platforms.length === 0}>
        <option value="">{$t("research.ex_pick_platform")}</option>
        {#each platforms as p (p.platform)}<option value={p.platform}>{p.name} ({p.operations})</option>{/each}
      </select>
    </label>
    <label class="dv-field wide">
      {$t("research.ex_endpoint")}
      <select class="input" bind:value={endpoint} onchange={pickEndpoint} disabled={operations.length === 0}>
        <option value="">{$t("research.ex_pick_endpoint")}</option>
        {#each operations as op (op.id)}<option value={op.id}>{op.method} {op.path}</option>{/each}
      </select>
    </label>
  </div>

  {#if loading}
    <p class="dv-meta" role="status"><span class="spinner" aria-hidden="true"></span> {$t("research.ex_loading")}</p>
  {/if}

  {#if operation}
    <p class="dv-meta">{operation.summary}</p>
    {#if operation.credits}<p class="dv-meta"><b>{$t("research.ex_credits_note")}</b> {operation.credits}</p>{/if}
    {#if operation.params.length === 0}
      <p class="dv-meta">{$t("research.ex_no_params")}</p>
    {:else}
      <div class="rs-params">
        {#each operation.params as param (param.name)}
          <label class="dv-field" title={param.description ?? ""}>
            <span class="dv-mono">{param.name}{#if param.required}<span class="rs-req" title={$t("research.ex_required") as string}> *</span>{/if}</span>
            {#if param.enum}
              <select class="input" bind:value={values[param.name]}>
                <option value="">{$t("research.any")}</option>
                {#each param.enum as option (option)}<option value={option}>{option}</option>{/each}
              </select>
            {:else if param.type === "boolean"}
              <select class="input" bind:value={values[param.name]}>
                <option value="">{$t("research.any")}</option>
                <option value="true">true</option>
                <option value="false">false</option>
              </select>
            {:else}
              <input class="input dv-mono" type="text" inputmode={param.type === "number" || param.type === "integer" ? "numeric" : "text"} bind:value={values[param.name]} placeholder={param.example ?? ""} spellcheck="false" autocomplete="off" />
            {/if}
            {#if param.description}<span class="rs-help">{param.description}</span>{/if}
          </label>
        {/each}
      </div>
    {/if}
    <div class="dv-bar">
      <p class="dv-meta">
        {#if !ready}{$t("research.key_needed")}{:else if missing.length}{$t("research.ex_missing", { names: missing.join(", ") })}{:else}{$t("research.spends")}{/if}
      </p>
      <button type="submit" class="btn btn-primary" disabled={!ready || busy || missing.length > 0}>
        {#if busy}<span class="spinner" aria-hidden="true"></span>{/if}
        {busy ? $t("research.running") : $t("research.ex_run")}
      </button>
    </div>
  {/if}
</form>

{#if error}
  <ErrorNotice {error} />
{:else if result}
  <div class="dv-panel">
    <div class="dv-bar">
      <div class="dv-bar-group">
        <span class="tag tag-info">{$t("research.ex_result_credits", { used: result.credits_used ?? "?", left: typeof result.credits_remaining === "number" ? result.credits_remaining.toLocaleString() : "?" })}</span>
        {#if nextPage}
          <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={useNextPage}>{$t("research.ex_next_page")}</button>
        {/if}
      </div>
      <CopyButton text={json} label={$t("research.ex_copy_json") as string} named />
    </div>
    <pre class="dv-code">{json}</pre>
    {#if result.truncated === true}
      <p class="dv-warn">{$t("research.ex_truncated", { shown: json.length.toLocaleString(), total: Number(result.total_chars ?? 0).toLocaleString() })}</p>
    {/if}
    {#if typeof result.saved_to === "string"}
      {@const path = result.saved_to}
      <div class="dv-bar">
        <p class="dv-meta">{$t("research.ex_saved")} <span class="dv-mono">{path}</span></p>
        <div class="dv-bar-group">
          <button type="button" class="btn btn-secondary btn-sm" onclick={() => void api.reveal(path).catch(() => {})}>{$t("pack.reveal")}</button>
          <CopyButton text={path} label={$t("pack.copy_path") as string} compact />
        </div>
      </div>
    {/if}
    <p class="dv-meta">{$t("research.untrusted_note")}</p>
  </div>
{:else if !loading && !operation}
  <p class="dv-empty">{$t("research.ex_empty")}</p>
{/if}

<style>
  .rs-params {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    align-items: start;
    gap: var(--space-3);
  }

  .rs-req {
    color: var(--danger);
  }

  .rs-help {
    display: -webkit-box;
    overflow: hidden;
    color: var(--text-dim);
    font-size: var(--text-xs);
    line-height: 1.35;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
</style>
