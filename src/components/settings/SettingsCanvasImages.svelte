<script lang="ts">
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { canvasPrefs, replaceCanvasPrefs, type CanvasPrefs } from "$lib/canvas/prefs.svelte";
  import { clearThumbnails } from "$lib/canvas/images.svelte";

  const tr = (key: string, fallback: string, vars?: Record<string, unknown>) =>
    $t(key, { default: fallback, ...(vars ?? {}) }) as string;

  const prefs = $derived(canvasPrefs());
  let busy = $state(false);
  let clearing = $state(false);

  async function save(mutate: (c: CanvasPrefs) => void) {
    if (busy) return;
    busy = true;
    try {
      const next = structuredClone($state.snapshot(prefs)) as CanvasPrefs;
      mutate(next);
      await replaceCanvasPrefs(next);
    } catch (error) {
      showToast("error", String(error));
    } finally {
      busy = false;
    }
  }

  async function clear() {
    if (clearing) return;
    clearing = true;
    try {
      const freed = await clearThumbnails();
      showToast("success", tr("canvas.settings.thumbsCleared", "Thumbnails cleared ({mb} MB)", { mb: (freed / 1048576).toFixed(1) }));
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

<section class="settings-section" id="settings-section-canvas-images">
  <h5 class="section-title">{tr("canvas.settings.imagesTitle", "Images")}</h5>
  <div class="card">
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{tr("canvas.settings.imagesAnyZoom", "Show images at any zoom")}</span>
        <span class="setting-path">{tr("canvas.settings.imagesAnyZoomHint", "Zoomed out, images stay visible as a lighter copy instead of a colored plate")}</span>
      </div>
      <button class="toggle" class:on={prefs.images.any_zoom} disabled={busy} role="switch" aria-checked={prefs.images.any_zoom}
        aria-label={tr("canvas.settings.imagesAnyZoom", "Show images at any zoom")}
        onclick={() => save((c) => (c.images.any_zoom = !c.images.any_zoom))}><span class="toggle-knob"></span></button>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <label class="setting-label" for="cv-img-q">{tr("canvas.settings.imagesQuality", "Quality when zoomed out")}</label>
        <span class="setting-path">{tr("canvas.settings.imagesQualityHint", "Sharper keeps more images in memory")}</span>
      </div>
      <select id="cv-img-q" class="input-text select" disabled={busy} value={prefs.images.quality}
        onchange={(e) => save((c) => (c.images.quality = (e.target as HTMLSelectElement).value as CanvasPrefs["images"]["quality"]))}>
        <option value="light">{tr("canvas.settings.imagesLight", "Light")}</option>
        <option value="balanced">{tr("canvas.settings.imagesBalanced", "Balanced")}</option>
        <option value="sharp">{tr("canvas.settings.imagesSharp", "Sharp")}</option>
      </select>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{tr("canvas.settings.imagesDownload", "Download images from the web")}</span>
        <span class="setting-path">{tr("canvas.settings.imagesDownloadHint", "Image links are saved into the workspace, up to 25 MB each")}</span>
      </div>
      <button class="toggle" class:on={prefs.images.download_web} disabled={busy} role="switch" aria-checked={prefs.images.download_web}
        aria-label={tr("canvas.settings.imagesDownload", "Download images from the web")}
        onclick={() => save((c) => (c.images.download_web = !c.images.download_web))}><span class="toggle-knob"></span></button>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{tr("canvas.settings.imagesThumbs", "Thumbnails")}</span>
        <span class="setting-path">{tr("canvas.settings.imagesThumbsHint", "They are made again when needed")}</span>
      </div>
      <button class="button" disabled={clearing} onclick={clear}>{tr("canvas.settings.imagesClearThumbs", "Clear thumbnails")}</button>
    </div>
  </div>
</section>

<section class="settings-section" id="settings-section-canvas-performance">
  <h5 class="section-title">{tr("canvas.settings.performanceTitle", "Performance")}</h5>
  <div class="card">
    <div class="setting-row">
      <div class="setting-col">
        <label class="setting-label" for="cv-perf-render">{tr("canvas.settings.rendering", "Rendering")}</label>
        <span class="setting-path">{tr("canvas.settings.renderingHint", "Faithful draws every node as it looks up close, at any zoom; Light shows summaries when zoomed out")}</span>
      </div>
      <select id="cv-perf-render" class="input-text select" disabled={busy} value={prefs.performance.rendering}
        onchange={(e) => save((c) => (c.performance.rendering = (e.target as HTMLSelectElement).value as CanvasPrefs["performance"]["rendering"]))}>
        <option value="faithful">{tr("canvas.settings.renderingFaithful", "Faithful (like up close)")}</option>
        <option value="light">{tr("canvas.settings.renderingLight", "Light (summaries when zoomed out)")}</option>
      </select>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <label class="setting-label" for="cv-perf-term">{tr("canvas.settings.mountedTerminals", "Terminals kept open")}</label>
        <span class="setting-path">{tr("canvas.settings.mountedTerminalsHint", "Terminals off screen beyond this are closed in the view; they keep running")}</span>
      </div>
      <input id="cv-perf-term" class="input-text num" type="number" min="1" max="64" disabled={busy} value={prefs.performance.mounted_terminals}
        onchange={(e) => save((c) => (c.performance.mounted_terminals = limit((e.target as HTMLInputElement).value, 1, 64, 12)))} />
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <label class="setting-label" for="cv-perf-portal">{tr("canvas.settings.livePortals", "Live portals")}</label>
        <span class="setting-path">{tr("canvas.settings.livePortalsHint", "Portals beyond this show their last picture until you get close")}</span>
      </div>
      <input id="cv-perf-portal" class="input-text num" type="number" min="0" max="16" disabled={busy} value={prefs.performance.live_portals}
        onchange={(e) => save((c) => (c.performance.live_portals = limit((e.target as HTMLInputElement).value, 0, 16, 4)))} />
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{tr("canvas.settings.forceCanvas2d", "Draw the canvas without the graphics card")}</span>
        <span class="setting-path">{tr("canvas.settings.forceCanvas2dHint", "Slower; try it if the canvas flickers or stays blank")}</span>
      </div>
      <button class="toggle" class:on={prefs.performance.force_canvas2d} disabled={busy} role="switch" aria-checked={prefs.performance.force_canvas2d}
        aria-label={tr("canvas.settings.forceCanvas2d", "Draw the canvas without the graphics card")}
        onclick={() => save((c) => (c.performance.force_canvas2d = !c.performance.force_canvas2d))}><span class="toggle-knob"></span></button>
    </div>
  </div>
</section>

<style>
  .num {
    width: 88px;
  }
</style>
