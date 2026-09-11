<script lang="ts">
  /** Corta o tempo morto da gravação, imagem e som na mesma marca. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtSecs, onToolProgress, pct, pickDir, pickFiles, reveal, FILTERS, type ToolProgress } from "$lib/tools/rt";

  type Item = {
    input: string; output: string | null; duration_before: number; duration_after: number;
    seconds_removed: number; cuts: number; skipped_short: number; ok: boolean; error: string | null;
  };
  type Result = { items: Item[] };

  let files = $state<string[]>([]);
  let threshold = $state(-30);
  let minSilence = $state(0.6);
  let padding = $state(0.1);
  let dryRun = $state(false);
  let outDir = $state("");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "video-silence") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!files.length || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_video_silence", {
        opts: { inputs: files, threshold_db: threshold, min_silence: minSilence, padding, dry_run: dryRun, output_dir: outDir, suffix: "" },
      });
      const failed = result.items.filter((i) => !i.ok).length;
      showToast(failed ? "info" : "success", `${result.items.length - failed} ${$t("tools.common.done")}`);
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
        <div class="group-row-content"><div class="group-row-title">{files.length} {$t("tools.common.files")}</div><div class="group-row-sub mono">{files.map(baseName).slice(0, 5).join(", ")}{files.length > 5 ? "…" : ""}</div></div>
        <div class="group-row-trailing btn-row">{#if files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (files = [])}>×</button>{/if}<button class="btn btn-secondary btn-sm" type="button" onclick={async () => { files = [...files, ...(await pickFiles(FILTERS.media))]; }}>{$t("tools.common.add")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.silence.threshold")} {threshold} dB</div><div class="group-row-sub">{$t("tools.silence.note")}</div></div>
        <div class="group-row-trailing"><input type="range" min="-60" max="-10" step="1" bind:value={threshold} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.silence.min")} <span class="dim">· {$t("tools.silence.padding")}</span></div></div>
        <div class="group-row-trailing btn-row">
          <input class="input" type="number" min="0.2" max="10" step="0.1" bind:value={minSilence} style:width="5em" />
          <input class="input" type="number" min="0" max="1" step="0.05" bind:value={padding} style:width="5em" />
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.silence.dry_run")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={dryRun} type="button" role="switch" aria-checked={dryRun} aria-label={$t("tools.silence.dry_run")} onclick={() => (dryRun = !dryRun)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{outDir || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !files.length} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.silence.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section><div class="group">
      {#each result.items as item (item.input)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{baseName(item.input)}</div>
            {#if item.ok}
              <div class="group-row-sub">
                {#if item.cuts === 0}{$t("tools.silence.nothing")}
                {:else}
                  {item.cuts} {$t("tools.silence.cuts")} · {fmtSecs(item.seconds_removed)} {$t("tools.silence.removed")}
                  <span class="dim"> · {fmtSecs(item.duration_before)} → {fmtSecs(item.duration_after)}</span>
                {/if}
              </div>
              {#if item.skipped_short}<div class="group-row-sub"><span class="tag tag-warning">{$t("tools.silence.skipped", { count: item.skipped_short })}</span></div>{/if}
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
