<script lang="ts">
  /** Spritesheet: fatiar em quadros, montar a folha com atlas, ou passar a régua numa pasta. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    baseName,
    errText,
    FILTERS,
    onToolProgress,
    openPath,
    pct,
    pickDir,
    pickFile,
    pickFiles,
    reveal,
    type ToolProgress,
  } from "$lib/tools/rt";

  type Frame = { name: string; x: number; y: number; w: number; h: number };
  type Result = {
    mode: string;
    outputs: string[];
    frames: Frame[];
    sheet: string | null;
    atlas: string | null;
    width: number;
    height: number;
    count: number;
    skipped: number;
  };

  let mode = $state("slice");
  let inputs = $state<string[]>([]);
  let inputDir = $state("");
  let outputDir = $state("");
  let name = $state("");

  let by = $state("grid");
  let cols = $state(4);
  let rows = $state(4);
  let cellW = $state(32);
  let cellH = $state(32);
  let margin = $state(0);
  let spacing = $state(0);
  let trim = $state(false);

  let packCols = $state(0);
  let padding = $state(0);
  let atlas = $state(true);

  let rule = $state("width");
  let value = $state(512);
  let boxH = $state(512);
  let suffix = $state("");
  let format = $state("png");
  let quality = $state(90);

  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const ready = $derived(mode === "slice" ? inputs.length > 0 : inputs.length > 0 || inputDir !== "");

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "img-sprite") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  function reset() {
    inputs = [];
    inputDir = "";
    result = null;
  }

  async function pickSheet() {
    const f = await pickFile(FILTERS.images);
    if (f) inputs = [f];
  }

  async function addFrames() {
    const picked = await pickFiles(FILTERS.images);
    if (picked.length) inputs = [...inputs, ...picked];
  }

  async function run() {
    if (!ready || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_img_sprite", {
        opts: {
          mode,
          inputs,
          input_dir: inputDir,
          output_dir: outputDir,
          name,
          cols: by === "grid" ? cols : 0,
          rows: by === "grid" ? rows : 0,
          cell_w: by === "cell" ? cellW : 0,
          cell_h: by === "cell" ? cellH : 0,
          margin,
          spacing,
          trim,
          pack_cols: packCols,
          padding,
          atlas,
          rule,
          value,
          box_h: boxH,
          suffix,
          format,
          quality,
        },
      });
      showToast("success", `${result.count} ${$t("tools.sprite.frames")}`);
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
        <div class="group-row-content"><div class="group-row-title">{$t("tools.sprite.mode")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={mode} onchange={reset}>
            <option value="slice">{$t("tools.sprite.mode_slice")}</option>
            <option value="pack">{$t("tools.sprite.mode_pack")}</option>
            <option value="batch">{$t("tools.sprite.mode_batch")}</option>
          </select>
        </div>
      </div>

      {#if mode === "slice"}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.sprite.sheet")}</div>
            <div class="group-row-sub mono">{inputs[0] ? baseName(inputs[0]) : "—"}</div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={pickSheet}>{$t("tools.common.choose")}</button></div>
        </div>
      {:else}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{mode === "pack" ? $t("tools.sprite.frames_in") : $t("tools.sprite.images_in")}</div>
            <div class="group-row-sub mono">{inputDir || (inputs.length ? `${inputs.length} ${$t("tools.common.files")}` : "—")}</div>
          </div>
          <div class="group-row-trailing btn-row">
            {#if inputs.length || inputDir}<button class="btn btn-ghost btn-sm" type="button" onclick={reset}>×</button>{/if}
            <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) { inputDir = d; inputs = []; } }}>{$t("tools.sprite.pick_folder")}</button>
            <button class="btn btn-secondary btn-sm" type="button" onclick={addFrames}>{$t("tools.common.add")}</button>
          </div>
        </div>
      {/if}
    </div>
  </section>

  {#if mode === "slice"}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sprite.cut_by")}</div></div>
          <div class="group-row-trailing">
            <select class="input" bind:value={by}>
              <option value="grid">{$t("tools.sprite.by_grid")}</option>
              <option value="cell">{$t("tools.sprite.by_cell")}</option>
            </select>
          </div>
        </div>
        {#if by === "grid"}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.sprite.grid")}</div></div>
            <div class="group-row-trailing btn-row">
              <input class="input" type="number" min="1" bind:value={cols} style:width="5em" />
              <span class="dim">×</span>
              <input class="input" type="number" min="1" bind:value={rows} style:width="5em" />
            </div>
          </div>
        {:else}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.sprite.cell")}</div></div>
            <div class="group-row-trailing btn-row">
              <input class="input" type="number" min="1" bind:value={cellW} style:width="5em" />
              <span class="dim">×</span>
              <input class="input" type="number" min="1" bind:value={cellH} style:width="5em" />
            </div>
          </div>
        {/if}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sprite.margin_spacing")}</div><div class="group-row-sub">{$t("tools.sprite.margin_note")}</div></div>
          <div class="group-row-trailing btn-row">
            <input class="input" type="number" min="0" bind:value={margin} style:width="5em" />
            <input class="input" type="number" min="0" bind:value={spacing} style:width="5em" />
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sprite.trim")}</div><div class="group-row-sub">{$t("tools.sprite.trim_note")}</div></div>
          <div class="group-row-trailing"><input type="checkbox" bind:checked={trim} /></div>
        </div>
      </div>
    </section>
  {:else if mode === "pack"}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sprite.pack_cols")}</div><div class="group-row-sub">{packCols === 0 ? $t("tools.sprite.pack_auto") : `${packCols}`}</div></div>
          <div class="group-row-trailing"><input class="input" type="number" min="0" bind:value={packCols} style:width="5em" /></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sprite.padding")}</div></div>
          <div class="group-row-trailing btn-row"><input class="input" type="number" min="0" bind:value={padding} style:width="5em" /><span class="dim">px</span></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sprite.atlas")}</div><div class="group-row-sub">{$t("tools.sprite.atlas_note")}</div></div>
          <div class="group-row-trailing"><input type="checkbox" bind:checked={atlas} /></div>
        </div>
      </div>
    </section>
  {:else}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sprite.rule")}</div></div>
          <div class="group-row-trailing">
            <select class="input" bind:value={rule}>
              <option value="width">{$t("tools.sprite.rule_width")}</option>
              <option value="height">{$t("tools.sprite.rule_height")}</option>
              <option value="fit">{$t("tools.sprite.rule_fit")}</option>
              <option value="scale">{$t("tools.sprite.rule_scale")}</option>
            </select>
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{rule === "scale" ? $t("tools.sprite.percent") : $t("tools.sprite.size")}</div></div>
          <div class="group-row-trailing btn-row">
            <input class="input" type="number" min="1" bind:value style:width="6em" />
            {#if rule === "fit"}<span class="dim">×</span><input class="input" type="number" min="1" bind:value={boxH} style:width="6em" />{/if}
            <span class="dim">{rule === "scale" ? "%" : "px"}</span>
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sprite.suffix")}</div></div>
          <div class="group-row-trailing"><input class="input" type="text" bind:value={suffix} placeholder="-lote" style:width="8em" /></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sprite.format")}</div></div>
          <div class="group-row-trailing btn-row">
            <select class="input" bind:value={format}>
              <option value="png">PNG</option>
              <option value="jpeg">JPEG</option>
            </select>
            {#if format === "jpeg"}<input class="input" type="number" min="1" max="100" bind:value={quality} style:width="5em" />{/if}
          </div>
        </div>
      </div>
    </section>
  {/if}

  <section>
    <div class="group">
      {#if mode !== "batch"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sprite.name")}</div><div class="group-row-sub">{$t("tools.sprite.name_note")}</div></div>
          <div class="group-row-trailing"><input class="input" type="text" bind:value={name} style:width="10em" /></div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{outputDir || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing btn-row">
          {#if outputDir}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (outputDir = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outputDir = d; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-primary" type="button" disabled={busy || !ready} onclick={run}>
            {busy ? $t("tools.common.working") : $t("tools.sprite.run")}
          </button>
        </div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.count} {$t("tools.sprite.frames")}{#if result.width}<span class="dim"> · {result.width}×{result.height}</span>{/if}</div>
            {#if result.skipped}<div class="group-row-sub">{result.skipped} {$t("tools.sprite.skipped")}</div>{/if}
            {#if result.atlas}<div class="group-row-sub mono">{baseName(result.atlas)}</div>{/if}
          </div>
          {#if result.sheet && result.mode === "pack"}
            <div class="group-row-trailing btn-row">
              <button class="btn btn-secondary btn-sm" type="button" onclick={() => openPath(result!.sheet!)}>{$t("tools.common.open")}</button>
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result!.sheet!)}>↗</button>
            </div>
          {:else if result.outputs.length}
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result!.outputs[0])}>↗</button></div>
          {/if}
        </div>
        {#each result.outputs.slice(0, 40) as out (out)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub mono">{baseName(out)}</div></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(out)}>↗</button></div>
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
