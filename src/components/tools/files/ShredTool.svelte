<script lang="ts">
  /** Apagar sobrescrevendo. Sem lixeira, sem desfazer — por isso o dry-run. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, onToolProgress, pct, pickDir, pickFiles, type ToolProgress } from "$lib/tools/rt";

  type Item = { path: string; bytes: number; passes: number; ok: boolean; error: string | null };
  type Result = { items: Item[]; bytes_total: number; copy_on_write_warning: boolean };

  let paths = $state<string[]>([]);
  let passes = $state(1);
  let rename = $state(true);
  let recursive = $state(false);
  let dryRun = $state(true);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "sys-shred") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!paths.length || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_shred", {
        opts: { paths, passes, rename, recursive, dry_run: dryRun },
      });
      const ok = result.items.filter((i) => i.ok).length;
      showToast("success", `${ok} ${dryRun ? $t("tools.shred.listed") : $t("tools.shred.erased")}`);
      if (!dryRun) paths = [];
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{paths.length} {$t("tools.shred.files")}</div><div class="group-row-sub mono">{paths.map(baseName).slice(0, 5).join(", ")}{paths.length > 5 ? "…" : ""}</div></div>
        <div class="group-row-trailing btn-row">
          {#if paths.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (paths = [])}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { paths = [...paths, ...(await pickFiles())]; }}>{$t("tools.common.add")}</button>
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) { paths = [...paths, d]; recursive = true; } }}>+ /</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.shred.passes")} {passes}</div><div class="group-row-sub">{$t("tools.shred.ssd_warning")}</div></div>
        <div class="group-row-trailing"><input type="range" min="1" max="7" step="1" bind:value={passes} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.shred.rename")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={rename} type="button" role="switch" aria-checked={rename} aria-label={$t("tools.shred.rename")} onclick={() => (rename = !rename)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.shred.recursive")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={recursive} type="button" role="switch" aria-checked={recursive} aria-label={$t("tools.shred.recursive")} onclick={() => (recursive = !recursive)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.shred.dry_run")}</div>{#if !dryRun}<div class="group-row-sub"><span class="tag tag-danger">{$t("tools.shred.danger")}</span></div>{/if}</div>
        <div class="group-row-trailing"><button class="toggle" class:on={dryRun} type="button" role="switch" aria-checked={dryRun} aria-label={$t("tools.shred.dry_run")} onclick={() => (dryRun = !dryRun)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn" class:btn-destructive={!dryRun} class:btn-primary={dryRun} type="button" disabled={busy || !paths.length} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.shred.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section><div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{result.items.filter((i) => i.ok).length} {dryRun ? $t("tools.shred.listed") : $t("tools.shred.erased")}</div><div class="group-row-sub">{fmtBytes(result.bytes_total)}</div></div>
      </div>
      {#each result.items.filter((i) => !i.ok) as item (item.path)}
        <div class="group-row"><div class="group-row-sub"><span class="tag tag-danger">{$t("tools.common.failed")}</span> <span class="mono">{item.path}</span> {item.error}</div></div>
      {/each}
    </div></section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
