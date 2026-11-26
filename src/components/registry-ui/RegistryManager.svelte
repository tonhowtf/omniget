<script lang="ts" module>
  export type SourceStatus = {
    state: "loading" | "ready" | "error";
    count: number;
    fromCache: boolean;
    stale: boolean;
    offline: boolean;
    error: string | null;
    detail: string | null;
    fetchedAt: number | null;
  };
</script>

<script lang="ts">
  import { t } from "$lib/i18n";
  import { customSource } from "$lib/components/registry";
  import { errorKey } from "$lib/components/errors";
  import type { RegistrySource } from "$lib/components/types";
  import StatusIcon from "#components/converter/StatusIcon.svelte";

  let {
    sources,
    statuses,
    onadd,
    onremove,
    onreload,
  }: {
    sources: RegistrySource[];
    statuses: Record<string, SourceStatus>;
    onadd: (source: RegistrySource) => void;
    onremove: (id: string) => void;
    onreload: (id: string) => void;
  } = $props();

  let url = $state("");
  let urlError = $state<string | null>(null);

  function add(event: SubmitEvent) {
    event.preventDefault();
    const raw = url.trim();
    if (!raw) {
      urlError = errorKey("url_empty");
      return;
    }
    if (!/^https:\/\//i.test(raw)) {
      urlError = errorKey("url_not_https");
      return;
    }
    const source = customSource(raw);
    if (!source) {
      urlError = errorKey(/\.json(\?|#|$)/i.test(raw) ? "url_private" : "url_not_json");
      return;
    }
    if (sources.some((s) => s.index_url === source.index_url)) {
      urlError = "components.registry_duplicate";
      return;
    }
    urlError = null;
    url = "";
    onadd(source);
  }

  function age(at: number | null): string {
    if (!at) return "";
    const hours = Math.max(0, Math.round((Date.now() / 1000 - at) / 3600));
    return hours < 1 ? ($t("components.updated_now") as string) : ($t("components.updated_hours", { count: hours }) as string);
  }
</script>

<div class="cr-reg">
  <ul class="cr-reg-list">
    {#each sources as source (source.id)}
      {@const st = statuses[source.id]}
      <li class="cr-reg-row">
        <StatusIcon name={!st || st.state === "loading" ? "running" : st.state === "error" ? "failed" : st.offline ? "warning" : "done"} size={14} />
        <div class="cr-reg-copy">
          <span class="cr-reg-name">{source.name}</span>
          <span class="cr-reg-url" title={source.index_url}>{source.index_url}</span>
          {#if st?.state === "error" && st.error}
            <span class="cr-reg-error">{$t(errorKey(st.error), { detail: st.detail ?? "" })}</span>
          {/if}
        </div>
        <span class="cr-reg-state">
          {#if !st || st.state === "loading"}
            {$t("components.registry_loading")}
          {:else if st.state === "error"}
            {$t("components.registry_failed")}
          {:else if st.offline}
            {$t("components.registry_offline", { count: st.count })} · {age(st.fetchedAt)}
          {:else}
            {$t("components.registry_items", { count: st.count })} · {age(st.fetchedAt)}
          {/if}
        </span>
        <button type="button" class="btn btn-ghost btn-sm" disabled={st?.state === "loading"} onclick={() => onreload(source.id)}>
          {$t("components.registry_reload")}
        </button>
        {#if source.custom}
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => onremove(source.id)} aria-label={$t("components.registry_remove_named", { name: source.name })}>
            {$t("components.registry_remove")}
          </button>
        {/if}
      </li>
    {/each}
  </ul>

  <form class="cr-reg-add" onsubmit={add}>
    <label class="cr-reg-label" for="cr-reg-url">{$t("components.registry_add_label")}</label>
    <div class="cr-reg-add-row">
      <input
        id="cr-reg-url"
        class="input"
        type="url"
        inputmode="url"
        spellcheck="false"
        autocomplete="off"
        placeholder="https://example.com/r/registry.json"
        bind:value={url}
        aria-invalid={urlError ? "true" : undefined}
        aria-describedby="cr-reg-hint"
      />
      <button type="submit" class="btn btn-secondary">{$t("components.registry_add")}</button>
    </div>
    <p id="cr-reg-hint" class="cr-reg-hint" class:err={urlError} role={urlError ? "alert" : undefined}>
      {urlError ? $t(urlError, { detail: "" }) : $t("components.registry_add_hint")}
    </p>
  </form>
</div>

<style>
  .cr-reg {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .cr-reg-list {
    margin: 0;
    padding: 0;
    list-style: none;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .cr-reg-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-height: var(--row-base);
    padding: var(--space-2) var(--space-3);
  }

  .cr-reg-row + .cr-reg-row {
    border-top: var(--hairline) solid var(--content-border);
  }

  .cr-reg-copy {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  .cr-reg-name {
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text);
  }

  .cr-reg-url {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .cr-reg-error {
    font-size: var(--text-xs);
    color: var(--text);
  }

  .cr-reg-state {
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .cr-reg-add {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .cr-reg-label {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
  }

  .cr-reg-add-row {
    display: flex;
    gap: var(--space-2);
  }

  .cr-reg-add-row .input {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono);
  }

  .cr-reg-hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .cr-reg-hint.err {
    color: var(--text);
  }
</style>
