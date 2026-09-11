<script lang="ts">
  /**
   * Legenda: converter formato, ressincronizar e queimar no vídeo. `mode`
   * decide qual formulário abre — o de converter também tem a queima.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, pct, pickDir, pickFile, pickFiles, reveal, FILTERS, type ToolProgress } from "$lib/tools/rt";

  let { mode = "convert" }: { mode?: "convert" | "resync" } = $props();

  type Item = { input: string; output: string | null; cues: number; first_ms: number; last_ms: number; applied_factor: number; ok: boolean; error: string | null };
  type Result = { items: Item[] };

  const SUBS = [{ name: "Subtitles", extensions: ["srt", "vtt", "ass", "ssa", "sub"] }];
  const FPS_PAIRS: [string, number][] = [
    ["23.976 → 25", 23.976 / 25], ["25 → 23.976", 25 / 23.976],
    ["24 → 25", 24 / 25], ["25 → 24", 25 / 24],
    ["29.97 → 30", 29.97 / 30], ["30 → 29.97", 30 / 29.97],
  ];

  let files = $state<string[]>([]);
  let format = $state("srt");
  let shiftMs = $state(0);
  let fpsIndex = $state(-1);
  let useAnchors = $state(false);
  let anchorFirstFrom = $state("");
  let anchorFirstTo = $state("");
  let anchorLastFrom = $state("");
  let anchorLastTo = $state("");
  let outDir = $state("");
  let busy = $state<string | null>(null);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  // queima
  let video = $state("");
  let burnSub = $state("");
  let fontSize = $state(24);
  let burned = $state("");
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "subtitle") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  /** "1:02.500", "62.5" ou "62500" → milissegundos. */
  function toMs(v: string): number | null {
    const s = v.trim();
    if (!s) return null;
    if (/^\d+$/.test(s) && !s.includes(":")) return Number(s);
    const parts = s.split(":");
    let total = 0;
    for (const p of parts) {
      const n = Number(p.replace(",", "."));
      if (Number.isNaN(n)) return null;
      total = total * 60 + n;
    }
    return Math.round(total * 1000);
  }

  let anchors = $derived.by(() => {
    if (!useAnchors) return [];
    const v = [anchorFirstFrom, anchorFirstTo, anchorLastFrom, anchorLastTo].map(toMs);
    return v.every((n) => n !== null) ? (v as number[]) : [];
  });

  async function run() {
    if (!files.length || busy) return;
    busy = "convert";
    result = null;
    try {
      result = await invoke<Result>("tool_subtitle_convert", {
        opts: {
          inputs: files,
          format,
          shift_ms: shiftMs,
          scale: fpsIndex >= 0 ? FPS_PAIRS[fpsIndex][1] : 0,
          anchors,
          output_dir: outDir,
          suffix: "",
        },
      });
      const failed = result.items.filter((i) => !i.ok).length;
      showToast(failed ? "info" : "success", `${result.items.length - failed} ${$t("tools.common.done")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
    }
  }

  async function burn() {
    if (!video || !burnSub || busy) return;
    busy = "burn";
    burned = "";
    try {
      burned = await invoke<string>("tool_subtitle_burn", {
        opts: { video, subtitle: burnSub, font_size: fontSize, output_dir: outDir, suffix: "" },
      });
      showToast("success", $t("tools.common.done") as string);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{files.length} {$t("tools.common.files")}</div><div class="group-row-sub">{files.length ? files.map(baseName).slice(0, 4).join(", ") : $t("tools.sub.note")}</div></div>
        <div class="group-row-trailing btn-row">{#if files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (files = [])}>×</button>{/if}<button class="btn btn-secondary btn-sm" type="button" onclick={async () => { files = [...files, ...(await pickFiles(SUBS))]; }}>{$t("tools.common.add")}</button></div>
      </div>

      {#if mode === "convert"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sub.format")}</div></div>
          <div class="group-row-trailing"><select class="input" bind:value={format}><option value="srt">SRT</option><option value="vtt">WebVTT</option><option value="ass">ASS</option></select></div>
        </div>
      {:else}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sub.shift")}</div><div class="group-row-sub mono">{(shiftMs / 1000).toFixed(2)} s</div></div>
          <div class="group-row-trailing btn-row">
            <input class="input" type="number" step="100" bind:value={shiftMs} style:width="7em" disabled={useAnchors} />
            <span class="dim">ms</span>
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sub.fps")}</div></div>
          <div class="group-row-trailing">
            <select class="input" bind:value={fpsIndex} disabled={useAnchors}>
              <option value={-1}>{$t("tools.sub.fps_none")}</option>
              {#each FPS_PAIRS as [label], i (label)}<option value={i}>{label}</option>{/each}
            </select>
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sub.anchors")}</div><div class="group-row-sub">{$t("tools.sub.anchor_hint")}</div></div>
          <div class="group-row-trailing"><button class="toggle" class:on={useAnchors} type="button" role="switch" aria-checked={useAnchors} aria-label={$t("tools.sub.anchors")} onclick={() => (useAnchors = !useAnchors)}><span class="toggle-knob"></span></button></div>
        </div>
        {#if useAnchors}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub">{$t("tools.sub.first_is")} → {$t("tools.sub.should_be")}</div></div>
            <div class="group-row-trailing btn-row">
              <input class="input mono" type="text" placeholder="0:01.000" bind:value={anchorFirstFrom} style:width="7em" />
              <input class="input mono" type="text" placeholder="0:02.000" bind:value={anchorFirstTo} style:width="7em" />
            </div>
          </div>
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub">{$t("tools.sub.last_is")} → {$t("tools.sub.should_be")}</div></div>
            <div class="group-row-trailing btn-row">
              <input class="input mono" type="text" placeholder="1:40.000" bind:value={anchorLastFrom} style:width="7em" />
              <input class="input mono" type="text" placeholder="1:45.000" bind:value={anchorLastTo} style:width="7em" />
            </div>
          </div>
        {/if}
      {/if}

      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{outDir || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy !== null || !files.length || (useAnchors && !anchors.length)} onclick={run}>{busy === "convert" ? $t("tools.common.working") : $t("tools.sub.run")}</button></div>
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
                {item.cues} {$t("tools.sub.cues")}
                <span class="dim"> · {(item.first_ms / 1000).toFixed(2)}s → {(item.last_ms / 1000).toFixed(2)}s</span>
                {#if Math.abs(item.applied_factor - 1) > 0.0001}<span class="dim"> · {$t("tools.sub.stretched", { factor: item.applied_factor.toFixed(4) })}</span>{/if}
              </div>
            {:else}
              <div class="group-row-sub"><span class="tag tag-danger">{$t("tools.common.failed")}</span> {item.error}</div>
            {/if}
          </div>
          {#if item.ok && item.output}<div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(item.output!)}>{$t("tools.common.reveal")}</button></div>{/if}
        </div>
      {/each}
    </div></section>
  {/if}

  {#if mode === "convert"}
    <section>
      <span class="group-label">{$t("tools.sub.burn")}</span>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sub.video")}</div><div class="group-row-sub mono">{video ? baseName(video) : "—"}</div></div>
          <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(FILTERS.video); if (f) video = f; }}>{$t("tools.common.choose")}</button></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sub.subtitle")}</div><div class="group-row-sub mono">{burnSub ? baseName(burnSub) : "—"}</div></div>
          <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(SUBS); if (f) burnSub = f; }}>{$t("tools.common.choose")}</button></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sub.font_size")} {fontSize}</div>{#if busy === "burn"}<div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
          <div class="group-row-trailing btn-row">
            <input type="range" min="12" max="48" step="2" bind:value={fontSize} />
            <button class="btn btn-primary" type="button" disabled={busy !== null || !video || !burnSub} onclick={burn}>{busy === "burn" ? $t("tools.common.working") : $t("tools.sub.burn_run")}</button>
          </div>
        </div>
        {#if burned}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub mono">{burned}</div></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(burned)}>{$t("tools.common.reveal")}</button></div>
          </div>
        {/if}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
