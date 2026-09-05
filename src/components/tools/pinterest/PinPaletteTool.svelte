<script lang="ts">
  /**
   * Paleta de cores (estudo 67): o que PaletteParrot e PinSuite cobram.
   * k-means nos thumbnails + as `dominant_color` que o Pinterest calcula.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, pct, type ToolProgress } from "$lib/tools/rt";
  import { darkText, loadCookies, paletteCss, type PaletteOut, type Swatch } from "$lib/tools/pinterest";
  import PinCookies from "./PinCookies.svelte";

  let url = $state("");
  let cookies = $state(loadCookies());
  let colors = $state(8);
  let limit = $state(60);
  let skipExtremes = $state(true);
  let busy = $state(false);
  let out = $state<PaletteOut | null>(null);
  let progress = $state<ToolProgress | null>(null);
  let showCookies = $state(false);
  let unlisten: (() => void) | null = null;

  onMount(async () => { unlisten = await onToolProgress((p) => { if (p.id === "pinterest:palette") progress = p; }); });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!url.trim() || busy) return;
    busy = true; out = null; progress = null;
    try {
      out = await invoke<PaletteOut>("tool_pin_palette", { url, cookies: cookies || null, limit, colors, skipExtremes });
    } catch (e) { showToast("error", errText(e)); } finally { busy = false; }
  }
  async function copy(text: string) { await navigator.clipboard.writeText(text); showToast("success", $t("tools.common.copied") as string); }
  const hexList = (s: Swatch[]) => s.map((x) => x.hex).join("\n");
  const json = (s: Swatch[]) => JSON.stringify(s.map((x) => ({ hex: x.hex, rgb: x.rgb, share: Number(x.share.toFixed(4)) })), null, 2);
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="text" bind:value={url} placeholder={$t("tools.pinterest.url_placeholder")} onkeydown={(e) => e.key === "Enter" && run()} /></div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => (showCookies = !showCookies)} title={$t("tools.pinterest.cookies_hint")}>🍪</button>
          <button class="btn btn-primary" type="button" disabled={busy || !url.trim()} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.pinterest.load")}</button>
        </div>
      </div>
      {#if showCookies}<PinCookies bind:value={cookies} />{/if}
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.colors")}</div></div>
        <div class="group-row-trailing btn-row"><input type="range" min="2" max="16" step="1" bind:value={colors} /> <span class="mono">{colors}</span></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.limit")}</div></div>
        <div class="group-row-trailing"><input class="input" type="number" min="1" max="500" step="10" bind:value={limit} style:width="7em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.skip_extremes")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={skipExtremes} /></div>
      </div>
      {#if busy && progress}<div class="group-row"><div class="group-row-content"><div class="group-row-sub">{$t(progress.stage === "sample" ? "tools.pinterest.stage_sample" : "tools.pinterest.stage_list")} {progress.done}{#if progress.total}/{progress.total}{/if}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div></div></div>{/if}
    </div>
  </section>

  {#if out}
    {#each [["swatches", out.swatches], ["dominant", out.dominant]] as [key, sw] (key)}
      {#if (sw as Swatch[]).length}
        <section>
          <span class="group-label">{$t(`tools.pinterest.${key}`)} · {out.title} · {out.pins_used} {$t("tools.pinterest.pins_used")}</span>
          <div class="group">
            <div class="group-row">
              <div class="group-row-content">
                <div class="bar">{#each sw as Swatch[] as s (s.hex)}<div style:background={s.hex} style:flex={s.share} title={`${s.hex} · ${(s.share * 100).toFixed(1)}%`}></div>{/each}</div>
                <div class="swatches">
                  {#each sw as Swatch[] as s (s.hex)}
                    <button class="sw" type="button" style:background={s.hex} style:color={darkText(s.hex) ? "#111" : "#fff"} onclick={() => copy(s.hex)} title={$t("tools.common.copy")}>
                      <span class="hex">{s.hex}</span><span class="share">{(s.share * 100).toFixed(0)}%</span>
                    </button>
                  {/each}
                </div>
              </div>
              <div class="group-row-trailing btn-row col">
                <button class="btn btn-secondary btn-sm" type="button" onclick={() => copy(hexList(sw as Swatch[]))}>{$t("tools.pinterest.copy_hex")}</button>
                <button class="btn btn-secondary btn-sm" type="button" onclick={() => copy(paletteCss(sw as Swatch[]))}>{$t("tools.pinterest.copy_css")}</button>
                <button class="btn btn-secondary btn-sm" type="button" onclick={() => copy(json(sw as Swatch[]))}>{$t("tools.pinterest.copy_json")}</button>
              </div>
            </div>
          </div>
        </section>
      {/if}
    {/each}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); }
  .bar { display: flex; height: 28px; border-radius: var(--radius-md); overflow: hidden; margin-bottom: var(--space-2); }
  .swatches { display: flex; flex-wrap: wrap; gap: var(--space-2); }
  .sw { display: flex; flex-direction: column; align-items: flex-start; justify-content: flex-end; width: 92px; height: 72px; padding: 6px 8px; border: 0; border-radius: var(--radius-md); cursor: pointer; box-shadow: inset 0 0 0 var(--hairline) rgba(0, 0, 0, 0.15); }
  .hex { font-family: var(--font-mono); font-size: var(--text-xs); font-weight: 600; }
  .share { font-size: 10px; opacity: 0.8; }
  .col { flex-direction: column; align-items: stretch; }
</style>
