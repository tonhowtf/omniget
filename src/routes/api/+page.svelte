<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/devtools/api";
  import { headersToRecord, pushHistory, statusTone } from "$lib/devtools/helpers";
  import type { DevtoolsError, HttpMethod, HttpResponse } from "$lib/devtools/types";
  import DevShell from "$components/devtools/DevShell.svelte";
  import ErrorNotice from "$components/devtools/ErrorNotice.svelte";
  import CopyButton from "$components/palette/CopyButton.svelte";

  type Row = { name: string; value: string };
  type Entry = { id: number; method: HttpMethod; url: string; rows: Row[]; body: string; response: HttpResponse | null; error: DevtoolsError | null };

  const METHODS: HttpMethod[] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

  let method = $state<HttpMethod>("GET");
  let url = $state("");
  let rows = $state<Row[]>([{ name: "", value: "" }]);
  let body = $state("");
  let busy = $state(false);
  let history = $state<Entry[]>([]);
  let current = $state<Entry | null>(null);
  let serial = 0;

  let response = $derived(current?.response ?? null);
  let hasBody = $derived(method !== "GET" && method !== "HEAD");

  function count(n: number): string {
    return n.toLocaleString();
  }

  async function send() {
    if (!url.trim() || busy) return;
    busy = true;
    const entry: Entry = { id: ++serial, method, url: url.trim(), rows: rows.map((row) => ({ ...row })), body, response: null, error: null };
    try {
      entry.response = await api.httpSend({ method, url: entry.url, headers: headersToRecord(rows), body: hasBody && body ? body : undefined });
    } catch (e) {
      entry.error = api.parseError(e);
    } finally {
      busy = false;
    }
    current = entry;
    history = pushHistory(history, entry);
  }

  function restore(entry: Entry) {
    method = entry.method;
    url = entry.url;
    rows = entry.rows.map((row) => ({ ...row }));
    body = entry.body;
    current = entry;
  }

  function removeRow(index: number) {
    rows = rows.length > 1 ? rows.filter((_, i) => i !== index) : [{ name: "", value: "" }];
  }
</script>

<DevShell icon="api" title={$t("api.title") as string} subtitle={$t("api.subtitle") as string} {busy}>
  <div class="ap-grid">
    <div class="dv-stack">
      <form
        class="dv-panel"
        onsubmit={(e) => {
          e.preventDefault();
          void send();
        }}
      >
        <div class="ap-line">
          <select class="input input-lg dv-mono ap-method" aria-label={$t("api.method") as string} bind:value={method}>
            {#each METHODS as m (m)}<option value={m}>{m}</option>{/each}
          </select>
          <!-- svelte-ignore a11y_autofocus -->
          <input class="input input-lg dv-mono ap-url" type="text" inputmode="url" aria-label={$t("api.url") as string} bind:value={url} placeholder="https://api.example.com/v1/items" spellcheck="false" autocomplete="off" autofocus />
          <button type="submit" class="btn btn-primary btn-lg" disabled={!url.trim() || busy}>
            {#if busy}<span class="spinner" aria-hidden="true"></span>{/if}
            {$t("api.send")}
          </button>
        </div>

        <fieldset class="ap-headers">
          <legend>{$t("api.headers")}</legend>
          {#each rows as row, i (i)}
            <div class="ap-header">
              <input class="input dv-mono" aria-label={$t("api.header_name") as string} placeholder="Authorization" bind:value={row.name} spellcheck="false" autocomplete="off" />
              <input class="input dv-mono" aria-label={$t("api.header_value") as string} placeholder="Bearer …" bind:value={row.value} spellcheck="false" autocomplete="off" />
              <button type="button" class="btn btn-ghost btn-sm ap-x" aria-label={$t("api.remove_header") as string} title={$t("api.remove_header") as string} onclick={() => removeRow(i)}>
                <svg viewBox="0 0 12 12" width="10" height="10" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><path d="m3 3 6 6M9 3 3 9" /></svg>
              </button>
            </div>
          {/each}
          <button type="button" class="btn btn-ghost btn-sm ap-add" onclick={() => (rows = [...rows, { name: "", value: "" }])}>{$t("api.add_header")}</button>
        </fieldset>

        {#if hasBody}
          <label class="dv-field">
            {$t("api.body")}
            <textarea class="input" bind:value={body} rows="5" placeholder={'{"name": "value"}'} spellcheck="false"></textarea>
          </label>
        {/if}
      </form>

      {#if current?.error}
        <ErrorNotice error={current.error} />
      {:else if response}
        <div class="dv-panel">
          <div class="dv-bar">
            <div class="dv-bar-group">
              <span class="tag tag-{statusTone(response.status)} ap-status">{response.status}</span>
              <span class="dv-meta">{$t("api.time", { ms: count(response.timing.totalMs) })}</span>
              <span class="dv-meta">{$t("api.size", { bytes: count(response.bodyBytes) })}</span>
            </div>
            <CopyButton text={response.curl} label={$t("api.copy_curl") as string} named />
          </div>

          {#if response.redirects.length > 0}
            <div class="ap-chain">
              <p class="dv-meta">{$t("api.redirects")}</p>
              <ol>
                {#each response.redirects as hop, i (i)}
                  <li class="dv-mono"><span class="tag tag-info">{hop.status}</span> {hop.url}</li>
                {/each}
                <li class="dv-mono"><span class="tag tag-{statusTone(response.status)}">{response.status}</span> {response.finalUrl}</li>
              </ol>
            </div>
          {/if}

          <details class="dv-details">
            <summary>{$t("api.response_headers", { count: Object.keys(response.headers).length })}</summary>
            <div class="dv-scroll">
              <table class="dv-table">
                <tbody>
                  {#each Object.entries(response.headers) as [name, value] (name)}
                    <tr>
                      <td class="dim dv-mono">{name}</td>
                      <td class="dv-mono">{value}</td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          </details>

          {#if response.body.binary}
            <p class="dv-meta">{$t("api.binary")}</p>
          {:else if response.body.preview}
            <div class="ap-body">
              <pre class="dv-code">{response.body.preview}</pre>
              <div class="ap-copy"><CopyButton text={response.body.preview} label={$t("api.copy_body") as string} compact /></div>
            </div>
          {:else}
            <p class="dv-meta">{$t("api.empty_body")}</p>
          {/if}
          {#if response.body.truncated}
            <p class="dv-warn">{$t("api.truncated", { shown: count(response.body.preview.length), total: count(response.body.total) })}</p>
          {/if}
          {#if response.bodyPath}
            <div class="dv-bar">
              <p class="dv-meta">{$t("api.saved")} <span class="dv-mono">{response.bodyPath}</span></p>
              <div class="dv-bar-group">
                <button type="button" class="btn btn-secondary btn-sm" onclick={() => response?.bodyPath && void api.reveal(response.bodyPath).catch(() => {})}>{$t("pack.reveal")}</button>
                <CopyButton text={response.bodyPath} label={$t("pack.copy_path") as string} compact />
              </div>
            </div>
          {/if}
        </div>
      {:else if !busy}
        <p class="dv-empty">{$t("api.empty")}</p>
      {/if}
    </div>

    <aside class="dv-panel ap-history" aria-label={$t("api.history") as string}>
      <h2 class="dv-title">{$t("api.history")}</h2>
      {#if history.length === 0}
        <p class="dv-meta">{$t("api.history_empty")}</p>
      {:else}
        <ul>
          {#each history as entry (entry.id)}
            <li>
              <button type="button" class="ap-entry" class:active={current?.id === entry.id} aria-current={current?.id === entry.id ? "true" : undefined} onclick={() => restore(entry)}>
                <span class="ap-entry-top">
                  <b class="dv-mono">{entry.method}</b>
                  {#if entry.response}
                    <span class="tag tag-{statusTone(entry.response.status)}">{entry.response.status}</span>
                    <span class="dv-meta">{$t("api.time", { ms: count(entry.response.timing.totalMs) })}</span>
                  {:else}
                    <span class="tag tag-danger">{entry.error?.code}</span>
                  {/if}
                </span>
                <span class="ap-entry-url dv-mono">{entry.url}</span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </aside>
  </div>
</DevShell>

<style>
  .ap-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 260px;
    align-items: start;
    gap: var(--space-4);
  }

  .ap-line {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .ap-method {
    flex: 0 0 auto;
    width: 112px;
    font-weight: 600;
  }

  .ap-url {
    flex: 1 1 220px;
    min-width: 0;
  }

  .ap-headers {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }

  .ap-headers legend {
    margin-bottom: var(--space-1);
    padding: 0;
    color: var(--text-dim);
    font-size: var(--text-xs);
  }

  .ap-header {
    display: grid;
    grid-template-columns: minmax(0, 2fr) minmax(0, 3fr) var(--control-h);
    gap: var(--space-2);
  }

  .ap-header .input {
    min-width: 0;
  }

  .ap-x {
    width: var(--control-h);
    padding: 0;
    justify-content: center;
  }

  .ap-add {
    align-self: flex-start;
  }

  .ap-status {
    font-family: var(--font-mono);
    font-weight: 700;
  }

  .ap-chain {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .ap-chain ol,
  .ap-history ul {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .ap-chain li {
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .ap-body {
    position: relative;
  }

  .ap-copy {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
  }

  .ap-entry {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    padding: var(--space-2);
    border: 0;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out);
  }

  @media (hover: hover) {
    .ap-entry:hover:not(.active) {
      background: var(--fill-1);
    }
  }

  .ap-entry:active:not(.active) {
    background: var(--fill-2);
  }

  .ap-entry.active {
    background: var(--accent-soft);
  }

  .ap-entry:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .ap-entry-top {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .ap-entry-url {
    overflow: hidden;
    color: var(--text-dim);
    font-size: var(--text-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  @media (max-width: 860px) {
    .ap-grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
