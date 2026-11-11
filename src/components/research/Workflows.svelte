<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/research/api";
  import { WORKFLOWS, buildArgs, evidenceOnly, initialValues, tablesFor, toMarkdown, visibleFields } from "$lib/research/workflows";
  import type { Result, Table, Workflow, WorkflowId } from "$lib/research/types";
  import type { DevtoolsError } from "$lib/devtools/types";
  import ErrorNotice from "$components/devtools/ErrorNotice.svelte";
  import CopyButton from "$components/palette/CopyButton.svelte";
  import EvidenceTable from "./EvidenceTable.svelte";

  let { ready, onran }: { ready: boolean; onran: () => void } = $props();

  let active = $state<WorkflowId>("creators");
  let values = $state<Record<WorkflowId, Record<string, string>>>({
    creators: initialValues(WORKFLOWS[0].fields),
    brief: initialValues(WORKFLOWS[1].fields),
    ads: initialValues(WORKFLOWS[2].fields),
    reels: initialValues(WORKFLOWS[3].fields),
  });
  let results = $state<Partial<Record<WorkflowId, Result>>>({});
  let errors = $state<Partial<Record<WorkflowId, DevtoolsError>>>({});
  let busy = $state<WorkflowId | null>(null);
  let notice = $state("");

  let workflow = $derived(WORKFLOWS.find((w) => w.id === active) as Workflow);
  let form = $derived(values[active]);
  let fields = $derived(visibleFields(workflow.fields, form.op ?? ""));
  let result = $derived(results[active] ?? null);
  let error = $derived(errors[active] ?? null);
  let tables = $derived<Table[]>(result ? tablesFor(active, result) : []);
  let caveats = $derived(Array.isArray(result?.caveats) ? (result.caveats as string[]) : []);
  let guide = $derived(typeof result?.guide === "string" ? result.guide : "");
  let markdown = $derived(
    result
      ? toMarkdown({
          title: $t(`research.wf.${active}.title`) as string,
          guide,
          caveats,
          tables: tables.map((table) => ({
            title: $t(`research.table.${table.id}`) as string,
            columns: table.columns.map((column) => $t(`research.col.${column.key}`) as string),
            rows: table.rows.map((row) => table.columns.map((column) => row[column.key] ?? null)),
          })),
          data: evidenceOnly(result),
          labels: {
            guide: $t("research.md_guide") as string,
            caveats: $t("research.md_caveats") as string,
            data: $t("research.md_data") as string,
            cut: $t("research.md_cut") as string,
            empty: $t("research.empty_table") as string,
          },
        })
      : "",
  );

  async function run() {
    if (busy || !ready) return;
    const id = active;
    busy = id;
    notice = "";
    try {
      const out = await api.tool(workflow.tool, buildArgs(workflow.fields, values[id]));
      const suggested = Array.isArray(out.suggested_queries) ? (out.suggested_queries as string[]) : [];
      if (id === "creators" && out.reason === "no_queries" && suggested.length) {
        values.creators.queries = suggested.join("\n");
        notice = $t("research.suggested_filled") as string;
        delete results[id];
      } else {
        results[id] = out;
      }
      delete errors[id];
    } catch (e) {
      errors[id] = api.parseError(e);
      delete results[id];
    } finally {
      busy = null;
      onran();
    }
  }
</script>

<div class="rs-cards" role="tablist" aria-label={$t("research.tab_workflows") as string}>
  {#each WORKFLOWS as w (w.id)}
    <button type="button" role="tab" id="rs-wf-{w.id}" class="rs-card" class:active={active === w.id} aria-selected={active === w.id} aria-controls="rs-wf-panel" onclick={() => { active = w.id; notice = ""; }}>
      <b>{$t(`research.wf.${w.id}.title`)}</b>
      <span>{$t(`research.wf.${w.id}.desc`)}</span>
    </button>
  {/each}
</div>

<div class="dv-stack" id="rs-wf-panel" role="tabpanel" aria-labelledby="rs-wf-{active}">
  <form
    class="dv-panel"
    onsubmit={(e) => {
      e.preventDefault();
      void run();
    }}
  >
    <div class="rs-fields">
      {#each fields as field (field.key)}
        {#if field.kind === "toggle"}
          <label class="rs-toggle">
            <input type="checkbox" checked={form[field.key] === "true"} onchange={(e) => (form[field.key] = e.currentTarget.checked ? "true" : "false")} />
            {$t(`research.field.${field.key}`)}
          </label>
        {:else}
          <label class="dv-field" class:rs-wide={field.kind === "area" || field.kind === "lines"}>
            {$t(`research.field.${field.key}`)}
            {#if field.kind === "area" || field.kind === "lines"}
              <textarea class="input" rows="3" bind:value={form[field.key]} placeholder={field.placeholder ?? ""} spellcheck="false"></textarea>
            {:else if field.kind === "select"}
              <select class="input" bind:value={form[field.key]}>
                {#each field.options ?? [] as option (option)}
                  <option value={option}>{option === "" ? $t("research.any") : field.key === "op" ? $t(`research.op.${option}`) : option}</option>
                {/each}
              </select>
            {:else if field.kind === "number"}
              <input class="input" type="number" min={field.min} max={field.max} bind:value={form[field.key]} />
            {:else}
              <input class="input" type="text" bind:value={form[field.key]} placeholder={field.placeholder ?? ""} spellcheck="false" autocomplete="off" />
            {/if}
          </label>
        {/if}
      {/each}
    </div>
    <div class="dv-bar">
      <p class="dv-meta">{ready ? $t("research.spends") : $t("research.key_needed")}</p>
      <button type="submit" class="btn btn-primary" disabled={!ready || busy !== null}>
        {#if busy === active}<span class="spinner" aria-hidden="true"></span>{/if}
        {busy === active ? $t("research.running") : $t("research.run")}
      </button>
    </div>
    {#if notice}<p class="dv-meta" role="status">{notice}</p>{/if}
  </form>

  {#if error}
    <ErrorNotice {error} />
  {:else if busy === active}
    <p class="dv-empty" role="status"><span class="spinner" aria-hidden="true"></span> {$t("research.running")}</p>
  {:else if result}
    <div class="dv-panel">
      <div class="dv-bar">
        <div class="dv-bar-group">
          {#if typeof result.credits_used === "number"}<span class="tag tag-info">{$t("research.credits_used", { credits: result.credits_used.toLocaleString() })}</span>{/if}
          {#if typeof result.reason === "string"}<span class="tag tag-warning">{result.reason}</span>{/if}
          {#if result.complete === false}<span class="tag tag-warning">complete=false</span>{/if}
        </div>
        <CopyButton text={markdown} label={$t("research.copy_brief") as string} named />
      </div>
      {#if typeof result.next_step === "string"}<p class="dv-meta">{result.next_step}</p>{/if}
      {#if guide}
        <details class="dv-details">
          <summary>{$t("research.md_guide")}</summary>
          <p class="rs-guide">{guide}</p>
        </details>
      {/if}
      {#if caveats.length}
        <ul class="rs-caveats" aria-label={$t("research.md_caveats") as string}>
          {#each caveats as caveat (caveat)}<li>{caveat}</li>{/each}
        </ul>
      {/if}
      <p class="dv-meta">{$t("research.untrusted_note")}</p>
    </div>
    {#each tables as table (table.id)}
      <EvidenceTable {table} />
    {/each}
  {:else}
    <p class="dv-empty">{$t("research.results_empty")}</p>
  {/if}
</div>

<style>
  .rs-cards {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(210px, 1fr));
    gap: var(--space-3);
  }

  .rs-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-3) var(--space-4);
    border: 0;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out);
  }

  .rs-card b {
    font-size: var(--text-md);
    font-weight: 650;
  }

  .rs-card span {
    color: var(--text-dim);
    font-size: var(--text-sm);
    line-height: 1.4;
  }

  @media (hover: hover) {
    .rs-card:hover:not(.active) {
      background: var(--fill-1);
    }
  }

  .rs-card.active {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }

  .rs-card:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .rs-fields {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
    align-items: end;
    gap: var(--space-3);
  }

  .rs-wide {
    grid-column: 1 / -1;
  }

  .rs-toggle {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--control-h);
    color: var(--text);
    font-size: var(--text-sm);
  }

  .rs-guide {
    margin: var(--space-2) 0 0;
    color: var(--text);
    font-size: var(--text-sm);
    line-height: 1.55;
    user-select: text;
  }

  .rs-caveats {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding-left: var(--space-5);
    color: var(--text-dim);
    font-size: var(--text-sm);
    line-height: 1.45;
  }
</style>
