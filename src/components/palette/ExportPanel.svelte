<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/palette/api";
  import { EXPORT_FORMATS, exportAs, fileName, type ExportFormat } from "$lib/palette/export";
  import { importFile } from "$lib/palette/importer";
  import { workspace } from "$lib/palette/workspace.svelte";
  import CopyButton from "./CopyButton.svelte";
  import StatusIcon from "#components/converter/StatusIcon.svelte";

  let project = $derived(workspace.project!);
  let format = $state<ExportFormat>("css");
  let code = $derived(exportAs(project, format));
  let name = $derived(fileName(project, format));
  let saving = $state(false);
  let saved = $state<string | null>(null);
  let saveError = $state<string | null>(null);
  let importInput = $state<HTMLInputElement | null>(null);
  let importMsg = $state<{ ok: boolean; text: string } | null>(null);
  let lines = $derived(code.split("\n").length);

  function choose(f: ExportFormat) {
    format = f;
    saved = null;
    saveError = null;
  }

  async function save() {
    if (saving) return;
    saving = true;
    saved = null;
    saveError = null;
    try {
      saved = await api.saveFile(code, name);
    } catch (e) {
      saveError = api.errorText(e);
    } finally {
      saving = false;
    }
  }

  let formatEls: HTMLButtonElement[] = $state([]);

  function formatKey(e: KeyboardEvent, index: number) {
    const moves: Record<string, number> = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 };
    let next = -1;
    if (e.key in moves) next = (index + moves[e.key] + EXPORT_FORMATS.length) % EXPORT_FORMATS.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = EXPORT_FORMATS.length - 1;
    if (next < 0) return;
    e.preventDefault();
    choose(EXPORT_FORMATS[next]);
    formatEls[next]?.focus();
  }

  async function onImport(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    importMsg = null;
    const res = await importFile(file, $t("palette.untitled_project") as string, (n) => $t("palette.scales.untitled", { num: n }) as string);
    if (!res.ok) {
      importMsg = { ok: false, text: $t(`palette.export.import_${res.reason}`) as string };
      return;
    }
    workspace.addProject(res.project);
    importMsg = { ok: true, text: $t("palette.export.imported", { name: res.project.name, count: res.project.palettes.length }) as string };
  }
</script>

<div class="pl-export">
  <div class="pl-export-head">
    <div class="segmented" role="radiogroup" aria-label={$t("palette.export.format")}>
      {#each EXPORT_FORMATS as f, i (f)}
        <button
          bind:this={formatEls[i]}
          type="button"
          class="segmented-btn"
          role="radio"
          aria-checked={format === f}
          tabindex={format === f ? 0 : -1}
          onclick={() => choose(f)}
          onkeydown={(e) => formatKey(e, i)}
        >
          {$t(`palette.export.f_${f}`)}
        </button>
      {/each}
    </div>
  </div>
  <p class="pl-hint">{$t(`palette.export.d_${format}`)}</p>

  {#if project.palettes.length === 0}
    <p class="pl-hint">{$t("palette.export.nothing")}</p>
  {:else}
    <pre class="pl-code" aria-label={$t("palette.export.preview", { name })}><code>{code}</code></pre>
    <div class="pl-export-bar">
      <span class="pl-dim">{$t("palette.export.meta", { name, lines })}</span>
      <div class="pl-export-actions">
        <CopyButton text={code} label={$t("palette.export.copy")} />
        <button type="button" class="btn btn-primary" disabled={saving} onclick={save}>
          {#if saving}<span class="spinner pl-spin" aria-hidden="true"></span>{/if}
          {saving ? $t("palette.export.saving") : $t("palette.export.save", { name })}
        </button>
      </div>
    </div>
    <div aria-live="polite">
      {#if saved}
        <div class="pl-msg">
          <StatusIcon name="done" size={16} />
          <p>{$t("palette.export.saved", { path: saved })}</p>
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => saved && api.reveal(saved).catch(() => {})}>{$t("palette.export.reveal")}</button>
        </div>
      {:else if saveError}
        <div class="pl-msg" role="alert">
          <StatusIcon name="failed" size={16} />
          <p>{$t("palette.export.save_error", { error: saveError })}</p>
        </div>
      {/if}
    </div>
  {/if}

  <section class="pl-import" aria-labelledby="pl-import-title">
    <div>
      <h3 id="pl-import-title" class="pl-subtitle">{$t("palette.export.import_title")}</h3>
      <p class="pl-hint">{$t("palette.export.import_body")}</p>
    </div>
    <input bind:this={importInput} class="pl-file" type="file" accept=".json,application/json" onchange={onImport} tabindex="-1" aria-hidden="true" />
    <button type="button" class="btn btn-secondary" onclick={() => importInput?.click()}>{$t("palette.export.import")}</button>
  </section>
  <div aria-live="polite">
    {#if importMsg}
      <div class="pl-msg" role={importMsg.ok ? "status" : "alert"}>
        <StatusIcon name={importMsg.ok ? "done" : "failed"} size={16} />
        <p>{importMsg.text}</p>
      </div>
    {/if}
  </div>
</div>

<style>
  .pl-export {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .pl-export-head {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .pl-hint,
  .pl-dim {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .pl-dim {
    font-size: var(--text-xs);
  }

  .pl-code {
    margin: 0;
    max-height: 420px;
    overflow: auto;
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 1.55;
    color: var(--text);
    tab-size: 2;
  }

  .pl-export-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .pl-export-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .pl-spin {
    width: 12px;
    height: 12px;
  }

  .pl-msg {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
  }

  .pl-msg p {
    margin: 0;
    flex: 1;
    min-width: 0;
    font-size: var(--text-sm);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .pl-import {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    margin-top: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pl-subtitle {
    margin: 0 0 var(--space-1);
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: var(--track-caps);
    text-transform: uppercase;
    color: var(--text-dim);
  }

  .pl-file {
    display: none;
  }
</style>
