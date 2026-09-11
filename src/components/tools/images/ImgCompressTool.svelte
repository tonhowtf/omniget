<script lang="ts">
  /** Comprimir imagem até caber num tamanho, por busca binária de qualidade. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, onToolProgress, pct, pickDir, pickFiles, reveal, FILTERS, type ToolProgress } from "$lib/tools/rt";

  type Item = { input: string; output: string | null; bytes_before: number; bytes_after: number; quality: number; width: number; height: number; scaled: boolean; hit_target: boolean; ok: boolean; error: string | null };
  type Result = { items: Item[]; bytes_before: number; bytes_after: number };

  let files = $state<string[]>([]);
  let byTarget = $state(true);
  let targetKb = $state(500);
  let quality = $state(82);
  let maxSide = $state(0);
  let background = $state("#FFFFFF");
  let dryRun = $state(false);
  let outDir = $state("");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "img-compress") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!files.length || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_img_compress", {
        opts: { inputs: files, target_kb: byTarget ? targetKb : 0, quality, max_side: maxSide, background, dry_run: dryRun, output_dir: outDir, suffix: "" },
      });
      const missed = result.items.filter((i) => i.ok && !i.hit_target).length;
      showToast(missed ? "info" : "success", `${result.items.filter((i) => i.ok).length} ${$t("tools.common.done")}`);
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
        <div class="group-row-content"><div class="group-row-title">{files.length} {$t("tools.common.files")}</div><div class="group-row-sub">{files.length ? files.map(baseName).slice(0, 4).join(", ") : $t("tools.imgcompress.note")}</div></div>
        <div class="group-row-trailing btn-row">{#if files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (files = [])}>×</button>{/if}<button class="btn btn-secondary btn-sm" type="button" onclick={async () => { files = [...files, ...(await pickFiles(FILTERS.images))]; }}>{$t("tools.common.add")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{byTarget ? $t("tools.imgcompress.target") : $t("tools.imgcompress.fixed_quality")}</div></div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={byTarget}><option value={true}>{$t("tools.imgcompress.by_size")}</option><option value={false}>{$t("tools.imgcompress.by_quality")}</option></select>
          {#if byTarget}
            <input class="input" type="number" min="10" step="50" bind:value={targetKb} style:width="7em" /><span class="dim">KB</span>
          {:else}
            <input type="range" min="30" max="98" step="2" bind:value={quality} /><span class="dim">{quality}</span>
          {/if}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.imgcompress.max_side")} <span class="dim">· {$t("tools.imgcompress.background")}</span></div></div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={maxSide}><option value={0}>{$t("tools.imgcompress.keep")}</option><option value={4000}>4000</option><option value={2560}>2560</option><option value={1920}>1920</option><option value={1280}>1280</option></select>
          <input class="input" type="color" bind:value={background} style:width="4em" />
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.imgcompress.dry_run")}</div><div class="group-row-sub mono">{outDir || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing btn-row">
          <button class="toggle" class:on={dryRun} type="button" role="switch" aria-checked={dryRun} aria-label={$t("tools.imgcompress.dry_run")} onclick={() => (dryRun = !dryRun)}><span class="toggle-knob"></span></button>
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !files.length} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.imgcompress.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section><div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{fmtBytes(result.bytes_before)} → {fmtBytes(result.bytes_after)}</div><div class="group-row-sub">{fmtBytes(Math.max(0, result.bytes_before - result.bytes_after))} {$t("tools.imgcompress.saved")}</div></div>
      </div>
      {#each result.items as item (item.input)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{baseName(item.input)}</div>
            {#if item.ok}
              <div class="group-row-sub">
                {fmtBytes(item.bytes_before)} → {fmtBytes(item.bytes_after)}
                <span class="dim"> · {$t("tools.imgcompress.quality")} {item.quality} · {item.width}×{item.height}</span>
                {#if item.scaled}<span class="tag">{$t("tools.imgcompress.scaled")}</span>{/if}
                {#if !item.hit_target}<span class="tag tag-warning">{$t("tools.imgcompress.missed")}</span>{/if}
              </div>
            {:else}
              <div class="group-row-sub"><span class="tag tag-danger">{$t("tools.common.failed")}</span> {item.error}</div>
            {/if}
          </div>
          {#if item.output}<div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(item.output!)}>{$t("tools.common.reveal")}</button></div>{/if}
        </div>
      {/each}
    </div></section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
