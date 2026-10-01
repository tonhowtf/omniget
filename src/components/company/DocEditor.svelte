<script lang="ts">
  import { t } from "$lib/i18n";
  import { rpc, errorText, type Document, type DocumentRev } from "$lib/company/rpc";
  import IssueMarkdown from "./IssueMarkdown.svelte";
  import { relTime } from "./company-ui";
  import { dateTime } from "./company-format";

  let { company, issue, documents }: { company: string; issue: string; documents: Document[] } = $props();

  let active = $state<string | null>(null);
  let mode = $state<"view" | "edit" | "history">("view");
  let draft = $state("");
  let summary = $state("");
  let baseRev = $state<number | null>(null);
  let newKey = $state("");
  let revisions = $state<DocumentRev[]>([]);
  let revOpen = $state<number | null>(null);
  let busy = $state(false);
  let error = $state("");

  let current = $derived(documents.find((d) => d.key === active) ?? null);

  $effect(() => {
    if (active === null && documents.length > 0) active = documents[0].key;
    if (active !== null && active !== "" && !documents.some((d) => d.key === active) && mode !== "edit") {
      active = documents[0]?.key ?? null;
    }
  });

  function open(key: string) {
    active = key;
    mode = "view";
    error = "";
  }

  function startEdit() {
    if (!current) return;
    draft = current.body;
    baseRev = current.rev;
    summary = "";
    mode = "edit";
    error = "";
  }

  function startNew() {
    active = "";
    newKey = "";
    draft = "";
    baseRev = null;
    summary = "";
    mode = "edit";
    error = "";
  }

  async function save() {
    const key = active === "" ? newKey.trim() : active;
    if (!key || busy) return;
    busy = true;
    error = "";
    try {
      await rpc("issue.doc_put", {
        company,
        issue,
        key,
        body: draft,
        base_rev: baseRev,
        summary: summary.trim() || null,
      });
      active = key;
      mode = "view";
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function history() {
    if (!current) return;
    mode = "history";
    revOpen = null;
    try {
      revisions = await rpc<DocumentRev[]>("issue.revisions", { company, issue, key: current.key });
    } catch (e) {
      error = errorText(e);
    }
  }

  async function toggleLock() {
    if (!current) return;
    try {
      await rpc("issue.doc_lock", { company, issue, key: current.key, locked: !current.locked });
    } catch (e) {
      error = errorText(e);
    }
  }
</script>

<div class="docs">
  <div class="tabs" role="tablist">
    {#each documents as d (d.id)}
      <button type="button" class="tab" role="tab" aria-selected={active === d.key} onclick={() => open(d.key)}>
        {#if d.locked}
          <svg width="10" height="10" viewBox="0 0 10 10" role="img" aria-label={$t("company.issue.doc_locked")}>
            <rect x="1.5" y="4.5" width="7" height="5" rx="1" fill="currentColor" />
            <path d="M3 4.5V3a2 2 0 0 1 4 0v1.5" stroke="currentColor" fill="none" stroke-width="1.2" />
          </svg>
        {/if}
        {d.key}
      </button>
    {/each}
    <button type="button" class="tab add" role="tab" aria-selected={active === ""} onclick={startNew}>
      + {$t("company.issue.doc_new")}
    </button>
  </div>

  {#if mode === "edit"}
    <div class="edit">
      {#if active === ""}
        <input class="input mono" type="text" bind:value={newKey} placeholder={$t("company.issue.doc_key_placeholder")} aria-label={$t("company.issue.doc_key")} />
      {/if}
      <textarea class="input mono" rows="14" bind:value={draft} aria-label={$t("company.issue.doc_body")}></textarea>
      <div class="row">
        <input class="input grow" type="text" bind:value={summary} placeholder={$t("company.issue.doc_summary_placeholder")} aria-label={$t("company.issue.doc_summary")} />
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => (mode = "view")}>{$t("company.common.cancel")}</button>
        <button type="button" class="btn btn-primary btn-sm" disabled={busy || (active === "" && !newKey.trim())} onclick={save}>
          {$t("company.common.save")}
        </button>
      </div>
      {#if baseRev !== null}<p class="hint">{$t("company.issue.doc_base_rev", { rev: baseRev })}</p>{/if}
    </div>
  {:else if current}
    <div class="toolbar-row">
      <span class="hint">{$t("company.issue.doc_rev", { rev: current.rev })} · {relTime(current.updated_ms, $t)}</span>
      <div class="spacer"></div>
      <button type="button" class="btn btn-ghost btn-sm" class:active={mode === "view"} onclick={() => (mode = "view")}>{$t("company.issue.doc_view")}</button>
      <button type="button" class="btn btn-ghost btn-sm" onclick={startEdit}>{$t("company.common.edit")}</button>
      <button type="button" class="btn btn-ghost btn-sm" class:active={mode === "history"} onclick={history}>{$t("company.issue.doc_history")}</button>
      <button type="button" class="btn btn-ghost btn-sm" onclick={toggleLock}>
        {current.locked ? $t("company.issue.doc_unlock") : $t("company.issue.doc_lock")}
      </button>
    </div>
    {#if mode === "history"}
      <div class="group">
        {#each revisions as r (r.rev)}
          <div class="rev">
            <button type="button" class="list-row interactive compact" onclick={() => (revOpen = revOpen === r.rev ? null : r.rev)}>
              <span class="mono">r{r.rev}</span>
              <span class="grow ellipsis">{r.summary || $t("company.issue.doc_no_summary")}</span>
              <span class="hint">{r.author}</span>
              <time class="hint" title={dateTime(r.created_ms)}>{relTime(r.created_ms, $t)}</time>
            </button>
            {#if revOpen === r.rev}<pre class="pre">{r.body}</pre>{/if}
          </div>
        {:else}
          <p class="hint pad">{$t("company.common.loading")}</p>
        {/each}
      </div>
    {:else}
      <div class="doc-body">
        {#if current.body.trim()}
          <IssueMarkdown text={current.body} />
        {:else}
          <p class="hint">{$t("company.issue.doc_empty")}</p>
        {/if}
      </div>
    {/if}
  {:else}
    <p class="hint">{$t("company.issue.no_docs")}</p>
  {/if}
  {#if error}<p class="field-error" role="alert">{error}</p>{/if}
</div>

<style>
  .docs {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 2px;
    border-bottom: var(--hairline) solid var(--separator);
  }
  .tab {
    padding: 4px var(--space-3);
    border: 0;
    border-bottom: 2px solid transparent;
    background: none;
    color: var(--text-muted);
    font-size: var(--text-sm);
    font-family: var(--font-mono);
    cursor: pointer;
  }
  .tab[aria-selected="true"] {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .tab.add {
    font-family: inherit;
    color: var(--text-dim);
  }
  .toolbar-row,
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .spacer,
  .grow {
    flex: 1;
    min-width: 0;
  }
  .edit {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .edit textarea {
    height: auto;
    resize: vertical;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }
  .doc-body {
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .pad {
    padding: var(--space-3);
  }
  .pre {
    margin: 0 var(--space-3) var(--space-2);
    padding: var(--space-2);
    max-height: 280px;
    overflow: auto;
    border-radius: var(--radius-sm);
    background: var(--fill-2);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    white-space: pre-wrap;
    user-select: text;
  }
</style>
