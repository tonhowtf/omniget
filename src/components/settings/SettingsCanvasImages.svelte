<script lang="ts">
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { canvasPrefs, type CanvasPrefs } from "$lib/canvas/prefs.svelte";
  import { clearThumbnails } from "$lib/canvas/images.svelte";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import GeneralChoice from "./rebrand/GeneralChoice.svelte";
  import Subgroup from "./ai/Subgroup.svelte";
  import { canvasSaving, saveCanvas } from "./ai/canvas-save.svelte";

  const prefs = $derived(canvasPrefs());
  const busy = $derived(canvasSaving.busy);
  let clearing = $state(false);

  async function clear() {
    if (clearing) return;
    clearing = true;
    try {
      const freed = await clearThumbnails();
      showToast("success", $t("canvas.settings.thumbsCleared", { mb: (freed / 1048576).toFixed(1) }) as string);
    } catch (error) {
      showToast("error", String(error));
    } finally {
      clearing = false;
    }
  }

  function limit(v: string, min: number, max: number, fallback: number): number {
    const n = Math.round(Number(v));
    return Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : fallback;
  }
</script>

{#snippet toggle(id: string, describedBy: string | undefined, on: boolean, onclick: () => void)}
  <button {id} type="button" class="toggle" class:on role="switch" aria-checked={on} aria-labelledby={`${id}-label`} aria-describedby={describedBy} disabled={busy} {onclick}><span class="toggle-knob"></span></button>
{/snippet}

{#snippet faithful()}
  <rect class="node" x="10" y="9" width="27" height="18" rx="3" />
  <path class="line" d="M15 15h16M15 20h10" />
  <rect class="node" x="44" y="22" width="27" height="18" rx="3" />
  <path class="line" d="M49 28h16M49 33h10" />
{/snippet}

{#snippet light()}
  <rect class="plate" x="10" y="9" width="27" height="18" rx="3" />
  <rect class="plate" x="44" y="22" width="27" height="18" rx="3" />
{/snippet}

<Subgroup id="canvas-performance" title={$t("canvas.settings.performanceTitle") as string}>
  <div class="block" data-setting-key="canvas.settings.rendering">
    <span class="block-label">{$t("canvas.settings.rendering")}</span>
    <GeneralChoice
      name="canvas-rendering"
      label={$t("canvas.settings.rendering") as string}
      value={prefs.performance.rendering}
      options={[
        { value: "faithful", draw: faithful, label: $t("settings.ai.render_faithful") as string, help: $t("settings.ai.render_faithful_help") as string },
        { value: "light", draw: light, label: $t("settings.ai.render_light") as string, help: $t("settings.ai.render_light_help") as string },
      ]}
      onchange={(v) => saveCanvas((c) => (c.performance.rendering = v as CanvasPrefs["performance"]["rendering"]))}
    />
  </div>
  <SettingRow id="canvas-mounted-terminals" label={$t("canvas.settings.mountedTerminals")} help={$t("canvas.settings.mountedTerminalsHint")} settingKey="canvas.settings.mountedTerminals">
    {#snippet control({ id, describedBy })}
      <input {id} class="input-text num" type="number" min="1" max="64" aria-describedby={describedBy} disabled={busy} value={prefs.performance.mounted_terminals}
        onchange={(e) => saveCanvas((c) => (c.performance.mounted_terminals = limit(e.currentTarget.value, 1, 64, 12)))} />
    {/snippet}
  </SettingRow>
  <SettingRow id="canvas-live-portals" label={$t("canvas.settings.livePortals")} help={$t("canvas.settings.livePortalsHint")} settingKey="canvas.settings.livePortals">
    {#snippet control({ id, describedBy })}
      <input {id} class="input-text num" type="number" min="0" max="16" aria-describedby={describedBy} disabled={busy} value={prefs.performance.live_portals}
        onchange={(e) => saveCanvas((c) => (c.performance.live_portals = limit(e.currentTarget.value, 0, 16, 4)))} />
    {/snippet}
  </SettingRow>
  <SettingRow id="canvas-force-2d" label={$t("canvas.settings.forceCanvas2d")} help={$t("canvas.settings.forceCanvas2dHint")} settingKey="canvas.settings.forceCanvas2d">
    {#snippet control({ id, describedBy })}
      {@render toggle(id, describedBy, prefs.performance.force_canvas2d, () => saveCanvas((c) => (c.performance.force_canvas2d = !c.performance.force_canvas2d)))}
    {/snippet}
  </SettingRow>
</Subgroup>

<Subgroup id="canvas-images" title={$t("canvas.settings.imagesTitle") as string}>
  <SettingRow id="canvas-images-any-zoom" label={$t("canvas.settings.imagesAnyZoom")} help={$t("canvas.settings.imagesAnyZoomHint")} settingKey="canvas.settings.imagesAnyZoom">
    {#snippet control({ id, describedBy })}
      {@render toggle(id, describedBy, prefs.images.any_zoom, () => saveCanvas((c) => (c.images.any_zoom = !c.images.any_zoom)))}
    {/snippet}
  </SettingRow>
  <SettingRow id="canvas-images-quality" label={$t("canvas.settings.imagesQuality")} help={$t("canvas.settings.imagesQualityHint")} settingKey="canvas.settings.imagesQuality">
    {#snippet control({ id, describedBy })}
      <select {id} class="input-text select" aria-describedby={describedBy} disabled={busy} value={prefs.images.quality}
        onchange={(e) => saveCanvas((c) => (c.images.quality = e.currentTarget.value as CanvasPrefs["images"]["quality"]))}>
        <option value="light">{$t("canvas.settings.imagesLight")}</option>
        <option value="balanced">{$t("canvas.settings.imagesBalanced")}</option>
        <option value="sharp">{$t("canvas.settings.imagesSharp")}</option>
      </select>
    {/snippet}
  </SettingRow>
  <SettingRow id="canvas-images-download" label={$t("canvas.settings.imagesDownload")} help={$t("canvas.settings.imagesDownloadHint")} settingKey="canvas.settings.imagesDownload">
    {#snippet control({ id, describedBy })}
      {@render toggle(id, describedBy, prefs.images.download_web, () => saveCanvas((c) => (c.images.download_web = !c.images.download_web)))}
    {/snippet}
  </SettingRow>
  <SettingRow id="canvas-images-thumbs" label={$t("canvas.settings.imagesThumbs")} help={$t("canvas.settings.imagesThumbsHint")} settingKey="canvas.settings.imagesThumbs">
    {#snippet control({ id, describedBy })}
      <button {id} type="button" class="btn btn-secondary btn-sm" aria-describedby={describedBy} disabled={clearing} onclick={clear}>{$t("canvas.settings.imagesClearThumbs")}</button>
    {/snippet}
  </SettingRow>
</Subgroup>

<style>
  .block { padding-block: var(--space-2); }
  .block-label { display: block; font-size: var(--text-base); line-height: 20px; color: var(--text); }
  .num { inline-size: 88px; }
  .node { fill: var(--surface-hi-src, var(--surface-src)); stroke: var(--tertiary); }
  .line { fill: none; stroke: var(--tertiary); stroke-width: 2; stroke-linecap: round; opacity: 0.6; }
  .plate { fill: var(--accent-soft); stroke: var(--accent); }
</style>
