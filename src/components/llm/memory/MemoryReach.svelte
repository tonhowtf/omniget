<script lang="ts">
  /**
   * What "forget" reaches and what it does not, plus export, import and the
   * app's own backups. `compact` shows only export and the explanation (the
   * bot panel); the full version lives on /llm/memory.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import {
    deleteBackups,
    exportMemory,
    getBackups,
    importMemory,
    loadBackups,
    reindexMemory,
    type ImportReport,
  } from "$lib/stores/assist-memory-store.svelte";

  let { compact = false }: { compact?: boolean } = $props();

  let busy = $state(false);
  let message = $state<string | null>(null);
  let errorKey = $state<string | null>(null);
  let allowResurrect = $state(false);
  let report = $state<ImportReport | null>(null);
  let confirmBackups = $state(false);
  let backups = $derived(getBackups());

  onMount(() => {
    void loadBackups();
  });

  async function doExport(format: "json" | "markdown") {
    busy = true;
    errorKey = null;
    message = null;
    try {
      const out = await exportMemory(format);
      if (out?.ok) message = $t("assist.memory.exported", { path: out.value }) as string;
      else if (out) errorKey = out.errorKey;
    } catch {
      errorKey = "assist.memory.err_generic";
    } finally {
      busy = false;
    }
  }

  async function doImport() {
    busy = true;
    errorKey = null;
    message = null;
    report = null;
    try {
      const out = await importMemory(allowResurrect);
      if (out?.ok) report = out.value;
      else if (out) errorKey = out.errorKey;
    } catch {
      errorKey = "assist.memory.err_generic";
    } finally {
      busy = false;
    }
  }

  async function doDeleteBackups() {
    busy = true;
    const out = await deleteBackups();
    busy = false;
    confirmBackups = false;
    if (out.ok) message = $t("assist.memory.backups_deleted", { count: out.value }) as string;
    else errorKey = out.errorKey;
  }

  async function doReindex() {
    busy = true;
    const out = await reindexMemory();
    busy = false;
    if (out.ok) message = $t("assist.memory.reindexed", { count: out.value }) as string;
    else errorKey = out.errorKey;
  }
</script>

<section class="block reach" aria-label={$t("assist.memory.reach_title")}>
  <h3 class="block-title">{$t("assist.memory.reach_title")}</h3>
  <ul class="reach-list">
    <li>{$t("assist.memory.reach_memory")}</li>
    <li>{$t("assist.memory.reach_conversation")}</li>
    <li>{$t("assist.memory.reach_exports")}</li>
    <li>
      {$t("assist.memory.reach_backups", { count: backups?.count ?? 0 })}
      {#if !compact && backups && backups.count > 0}
        {#if confirmBackups}
          <span class="confirm">
            <button type="button" class="button small danger" disabled={busy} onclick={doDeleteBackups}>{$t("assist.memory.backups_delete_yes")}</button>
            <button type="button" class="button small" onclick={() => (confirmBackups = false)}>{$t("assist.memory.cancel")}</button>
          </span>
        {:else}
          <button type="button" class="button small" disabled={busy} onclick={() => (confirmBackups = true)}>{$t("assist.memory.backups_delete")}</button>
        {/if}
      {/if}
    </li>
    <li>{$t("assist.memory.reach_providers")}</li>
  </ul>

  <div class="actions">
    <button type="button" class="button" disabled={busy} onclick={() => doExport("markdown")}>{$t("assist.memory.export_md")}</button>
    <button type="button" class="button" disabled={busy} onclick={() => doExport("json")}>{$t("assist.memory.export_json")}</button>
    {#if compact}
      <a class="link" href="/llm/memory">{$t("assist.memory.open_page")}</a>
    {/if}
  </div>

  {#if !compact}
    <div class="import">
      <button type="button" class="button" disabled={busy} onclick={doImport}>{$t("assist.memory.import")}</button>
      <label class="check">
        <input type="checkbox" bind:checked={allowResurrect} />
        <span>{$t("assist.memory.import_resurrect")}</span>
      </label>
    </div>
    <details class="advanced">
      <summary>{$t("assist.memory.advanced")}</summary>
      <p class="dim">{$t("assist.memory.reindex_hint")}</p>
      <button type="button" class="button small" disabled={busy} onclick={doReindex}>{$t("assist.memory.reindex")}</button>
    </details>
  {/if}

  {#if report}
    <p class="notice" role="status">
      {$t("assist.memory.import_report", {
        imported: report.imported,
        duplicates: report.duplicates,
        forgotten: report.skipped_forgotten,
      })}
      {#if report.resurrected}{$t("assist.memory.import_resurrected", { count: report.resurrected })}{/if}
    </p>
    {#if report.rejected.length}
      <details><summary>{$t("assist.memory.import_rejected", { count: report.rejected.length })}</summary>
        <ul>{#each report.rejected as r}<li>{r}</li>{/each}</ul>
      </details>
    {/if}
  {/if}
  {#if message}<p class="notice" role="status">{message}</p>{/if}
  {#if errorKey}<p class="notice error" role="alert">{$t(errorKey)}</p>{/if}
</section>

<style>
  .reach { display: grid; gap: 10px; }
  .reach-list { margin: 0; padding-left: 20px; display: grid; gap: 6px; font-size: 13px; line-height: 1.5; color: var(--text); max-width: 80ch; }
  .actions, .import { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .check { display: inline-flex; gap: 6px; align-items: center; font-size: 13px; }
  .confirm { display: inline-flex; gap: 6px; margin-left: 6px; }
  .small { min-height: 32px; padding: 4px 10px; font-size: 13px; }
  .danger { color: var(--danger, #c0392b); }
  .link { color: var(--accent-text); font-size: 13px; border-radius: var(--radius-sm); }
  .dim { margin: 0 0 8px; color: var(--text-muted); font-size: 13px; }
  .advanced summary { cursor: pointer; font-size: 13px; color: var(--text-muted); }
  .button:focus-visible, .link:focus-visible, summary:focus-visible, input:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
</style>
