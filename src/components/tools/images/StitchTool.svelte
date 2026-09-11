<script lang="ts">
  /** Costura prints numa imagem longa, com detecção de sobreposição. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    baseName,
    dirName,
    errText,
    FILTERS,
    fmtBytes,
    onToolProgress,
    openPath,
    pct,
    pickFiles,
    reveal,
    saveAs,
    type ToolProgress,
  } from "$lib/tools/rt";

  type Seam = { index: number; overlap: number; score: number; matched: boolean };
  type Result = {
    output: string;
    width: number;
    height: number;
    bytes: number;
    count: number;
    saved_px: number;
    seams: Seam[];
  };

  let inputs = $state<string[]>([]);
  let direction = $state("vertical");
  let align = $state("start");
  let spacing = $state(0);
  let background = $state("#FFFFFF");
  let smart = $state(true);
  let search = $state(400);
  let tolerance = $state(6);
  let cropTop = $state(0);
  let cropBottom = $state(0);
  let maxWidth = $state(0);
  let format = $state("png");
  let quality = $state(90);
  let output = $state("");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const matched = $derived(result ? result.seams.filter((s) => s.matched).length : 0);

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "img-stitch") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function add() {
    const picked = await pickFiles(FILTERS.images);
    if (picked.length) inputs = [...inputs, ...picked];
  }

  function move(i: number, delta: number) {
    const j = i + delta;
    if (j < 0 || j >= inputs.length) return;
    const next = [...inputs];
    [next[i], next[j]] = [next[j], next[i]];
    inputs = next;
  }

  async function chooseOutput() {
    const ext = format === "jpeg" ? "jpg" : "png";
    const suggested = inputs.length ? `${dirName(inputs[0])}/costura.${ext}` : `costura.${ext}`;
    const picked = await saveAs(suggested, FILTERS.images);
    if (picked) output = picked;
  }

  async function run() {
    if (inputs.length < 2 || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_img_stitch", {
        opts: {
          inputs,
          direction,
          align,
          spacing,
          background,
          smart,
          search,
          tolerance,
          crop_top: cropTop,
          crop_bottom: cropBottom,
          max_width: maxWidth,
          format,
          quality,
          output,
        },
      });
      showToast("success", `${result.width}×${result.height}`);
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
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.stitch.prints")}</div>
          <div class="group-row-sub">{$t("tools.stitch.order_note")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if inputs.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (inputs = [])}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={add}>{$t("tools.common.add")}</button>
        </div>
      </div>
      {#each inputs as file, i (file + i)}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-sub mono">{i + 1}. {baseName(file)}</div></div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-ghost btn-sm" type="button" disabled={i === 0} onclick={() => move(i, -1)}>↑</button>
            <button class="btn btn-ghost btn-sm" type="button" disabled={i === inputs.length - 1} onclick={() => move(i, 1)}>↓</button>
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => (inputs = inputs.filter((_, k) => k !== i))}>×</button>
          </div>
        </div>
      {/each}
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.stitch.direction")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={direction}>
            <option value="vertical">{$t("tools.stitch.vertical")}</option>
            <option value="horizontal">{$t("tools.stitch.horizontal")}</option>
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.stitch.align")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={align}>
            <option value="start">{$t("tools.stitch.align_start")}</option>
            <option value="center">{$t("tools.stitch.align_center")}</option>
            <option value="end">{$t("tools.stitch.align_end")}</option>
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.stitch.spacing")}</div></div>
        <div class="group-row-trailing btn-row">
          <input class="input" type="number" min="0" step="4" bind:value={spacing} style:width="6em" />
          <span class="dim">px</span>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.stitch.background")}</div></div>
        <div class="group-row-trailing"><input class="input" type="text" bind:value={background} style:width="7em" /></div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.stitch.smart")}</div>
          <div class="group-row-sub">{$t("tools.stitch.smart_note")}</div>
        </div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={smart} /></div>
      </div>
      {#if smart}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.stitch.search")}</div>
            <div class="group-row-sub">{search === 0 ? $t("tools.stitch.search_auto") : `${search} px`}</div>
          </div>
          <div class="group-row-trailing"><input class="input" type="number" min="0" step="50" bind:value={search} style:width="6em" /></div>
        </div>
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.stitch.tolerance")} {tolerance}</div>
            <div class="group-row-sub">{$t("tools.stitch.tolerance_note")}</div>
          </div>
          <div class="group-row-trailing"><input type="range" min="1" max="30" step="1" bind:value={tolerance} /></div>
        </div>
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.stitch.bars")}</div>
            <div class="group-row-sub">{$t("tools.stitch.bars_note")}</div>
          </div>
          <div class="group-row-trailing btn-row">
            <input class="input" type="number" min="0" step="8" bind:value={cropTop} style:width="5em" />
            <input class="input" type="number" min="0" step="8" bind:value={cropBottom} style:width="5em" />
          </div>
        </div>
      {/if}
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.stitch.max_width")}</div><div class="group-row-sub">{maxWidth === 0 ? $t("tools.stitch.max_width_off") : `${maxWidth} px`}</div></div>
        <div class="group-row-trailing"><input class="input" type="number" min="0" step="100" bind:value={maxWidth} style:width="7em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.stitch.format")}</div></div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={format}>
            <option value="png">PNG</option>
            <option value="jpeg">JPEG</option>
          </select>
          {#if format === "jpeg"}<input class="input" type="number" min="1" max="100" bind:value={quality} style:width="5em" />{/if}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.stitch.output")}</div><div class="group-row-sub mono">{output || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={chooseOutput}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-primary" type="button" disabled={busy || inputs.length < 2} onclick={run}>
            {busy ? $t("tools.common.working") : $t("tools.stitch.run")}
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
            <div class="group-row-title">{result.width}×{result.height} · {fmtBytes(result.bytes)}</div>
            <div class="group-row-sub">
              {result.count} {$t("tools.common.files")}
              {#if smart} · {matched}/{result.seams.length} {$t("tools.stitch.seams_matched")} · {result.saved_px} px {$t("tools.stitch.saved")}{/if}
            </div>
            <div class="group-row-sub mono">{result.output}</div>
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => openPath(result!.output)}>{$t("tools.common.open")}</button>
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result!.output)}>↗</button>
          </div>
        </div>
        {#each result.seams as seam (seam.index)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-sub">
                {$t("tools.stitch.seam")} {seam.index}
                {#if seam.matched}
                  <span class="tag tag-success">{seam.overlap} px</span>
                {:else}
                  <span class="tag">{$t("tools.stitch.stacked")}</span>
                {/if}
              </div>
            </div>
            <div class="group-row-trailing"><span class="dim mono">{seam.matched ? seam.score.toFixed(2) : "—"}</span></div>
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
