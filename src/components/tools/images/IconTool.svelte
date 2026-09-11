<script lang="ts">
  /** Uma imagem → favicon.ico, PNG de web, apple-touch-icon e .icns. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, onToolProgress, pct, pickDir, pickFile, reveal, FILTERS, type ToolProgress } from "$lib/tools/rt";

  type Out = { path: string; label: string; bytes: number };
  type Result = { folder: string; outputs: Out[]; source_size: [number, number]; upscaled_from: number | null };

  let input = $state("");
  let favicon = $state(true);
  let apple = $state(true);
  let android = $state(true);
  let icns = $state(false);
  let winIco = $state(false);
  let background = $state("#FFFFFF");
  let padding = $state(0);
  let outDir = $state("");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  let nothing = $derived(!favicon && !apple && !android && !icns && !winIco);

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "img-icon") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!input || busy || nothing) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_icon_pack", {
        opts: {
          input,
          output_dir: outDir,
          favicon,
          apple,
          android,
          icns,
          win_ico: winIco,
          background,
          padding_pct: padding,
        },
      });
      showToast("success", `${result.outputs.length} ${$t("tools.common.done")}`);
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
        <div class="group-row-content"><div class="group-row-title">{$t("tools.iconpack.source")}</div><div class="group-row-sub mono">{input ? baseName(input) : $t("tools.iconpack.square_note")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(FILTERS.images); if (f) { input = f; result = null; } }}>{$t("tools.common.choose")}</button></div>
      </div>

      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.iconpack.favicon")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={favicon} type="button" role="switch" aria-checked={favicon} aria-label={$t("tools.iconpack.favicon")} onclick={() => (favicon = !favicon)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.iconpack.apple")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={apple} type="button" role="switch" aria-checked={apple} aria-label={$t("tools.iconpack.apple")} onclick={() => (apple = !apple)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.iconpack.android")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={android} type="button" role="switch" aria-checked={android} aria-label={$t("tools.iconpack.android")} onclick={() => (android = !android)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.iconpack.icns")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={icns} type="button" role="switch" aria-checked={icns} aria-label={$t("tools.iconpack.icns")} onclick={() => (icns = !icns)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.iconpack.win_ico")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={winIco} type="button" role="switch" aria-checked={winIco} aria-label={$t("tools.iconpack.win_ico")} onclick={() => (winIco = !winIco)}><span class="toggle-knob"></span></button></div>
      </div>

      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.iconpack.background")} <span class="dim">· {$t("tools.iconpack.padding")} {padding}%</span></div></div>
        <div class="group-row-trailing btn-row">
          <input class="input" type="color" bind:value={background} style:width="4em" />
          <input type="range" min="0" max="30" step="5" bind:value={padding} />
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{outDir || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub mono">{progress?.message ?? "…"}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !input || nothing} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.iconpack.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.iconpack.folder")}</div>
            <div class="group-row-sub mono">{result.folder}</div>
            {#if result.upscaled_from}<div class="group-row-sub"><span class="tag tag-warning">{$t("tools.iconpack.small_source", { size: result.upscaled_from })}</span></div>{/if}
          </div>
          <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.outputs[0]?.path ?? result!.folder)}>{$t("tools.common.reveal")}</button></div>
        </div>
        {#each result.outputs as o (o.path)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub mono">{o.label}</div></div>
            <div class="group-row-trailing"><span class="dim">{fmtBytes(o.bytes)}</span></div>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
