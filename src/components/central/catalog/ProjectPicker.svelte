<script lang="ts">
  /** Project folder: native folder dialog + recent folders + the LLM workspace folder. */
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { baseName, recentProjects, rememberProject, workspaceFolder } from "$lib/central/catalog";

  let {
    value = $bindable<string | null>(null),
    onchange,
    allowNone = false,
  }: { value?: string | null; onchange?: (v: string | null) => void; allowNone?: boolean } = $props();

  let recent = $state<string[]>([]);
  let ws = $state<string | null>(null);

  onMount(async () => {
    recent = recentProjects();
    ws = await workspaceFolder();
  });

  let options = $derived([...new Set([...(value ? [value] : []), ...(ws ? [ws] : []), ...recent])]);

  function set(v: string | null) {
    value = v;
    if (v) {
      rememberProject(v);
      recent = recentProjects();
    }
    onchange?.(v);
  }

  async function pick() {
    try {
      const r = await openDialog({ directory: true, multiple: false, defaultPath: value ?? undefined });
      if (typeof r === "string" && r) set(r);
    } catch {
      /* dialog closed */
    }
  }

  function onselect(e: Event) {
    const v = (e.currentTarget as HTMLSelectElement).value;
    if (v === "__pick") void pick();
    else set(v || null);
  }
</script>

<div class="picker">
  <select value={value ?? ""} onchange={onselect} aria-label={$t("llm.central.catalog.project.label")}>
    {#if allowNone || !value}<option value="">{$t("llm.central.catalog.project.none")}</option>{/if}
    {#each options as o (o)}
      <option value={o} title={o}>{baseName(o)}{o === ws ? ` · ${$t("llm.central.catalog.project.workspace")}` : ""}</option>
    {/each}
    <option value="__pick">{$t("llm.central.catalog.project.choose")}</option>
  </select>
  <button type="button" class="btn" onclick={pick}>{$t("llm.central.catalog.project.browse")}</button>
</div>
{#if value}<p class="path" title={value}>{value}</p>{/if}

<style>
  .picker {
    display: flex;
    gap: var(--space-1);
  }
  select {
    flex: 1;
    min-width: 0;
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .btn {
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    color: var(--text);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .path {
    margin: 2px 0 0;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
