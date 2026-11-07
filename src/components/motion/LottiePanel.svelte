<script lang="ts">
  import { onMount, tick } from "svelte";
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import type { AnimationItem } from "lottie-web";
  import { t } from "$lib/i18n";
  import * as api from "$lib/motion/api";
  import { num } from "$lib/motion/format";
  import { pngName, readLottie, timeLabel, type LottieInfo } from "$lib/motion/lottie";
  import { reducedMotion } from "$lib/motion/reduced.svelte";

  let { active = true }: { active?: boolean } = $props();

  type Phase = { kind: "empty" } | { kind: "loading"; name: string } | { kind: "error"; message: string } | { kind: "ready" };
  type Swatch = "checker" | "light" | "dark" | "custom";
  type ExportState = { kind: "idle" } | { kind: "busy" } | { kind: "ok"; path: string; name: string } | { kind: "error"; message: string };

  const reduced = reducedMotion();
  const SPEEDS = [0.25, 0.5, 1, 1.5, 2];

  let phase = $state<Phase>({ kind: "empty" });
  let file = $state<api.LottieFile | null>(null);
  let info = $state<LottieInfo | null>(null);
  let host = $state<HTMLDivElement | null>(null);
  let anim: AnimationItem | null = null;
  let playing = $state(false);
  let looping = $state(true);
  let speed = $state(1);
  let frame = $state(0);
  let swatch = $state<Swatch>("checker");
  let custom = $state("#1c1c1e");
  let scale = $state(1);
  let withBackground = $state(false);
  let exportState = $state<ExportState>({ kind: "idle" });
  let dragging = $state(false);
  let token = 0;

  let lastFrame = $derived(info ? Math.max(0, info.frames - 1) : 0);
  let background = $derived(swatch === "light" ? "#ffffff" : swatch === "dark" ? "#000000" : swatch === "custom" ? custom : null);

  function destroy() {
    anim?.destroy();
    anim = null;
  }

  async function load(path: string) {
    const mine = ++token;
    destroy();
    exportState = { kind: "idle" };
    phase = { kind: "loading", name: api.fileName(path) };
    try {
      const loaded = await api.openLottie(path);
      if (mine !== token) return;
      const check = readLottie(loaded.animation);
      if (!check.ok) {
        phase = { kind: "error", message: $t(`motion.lottie.err_${check.error}`, { name: loaded.name }) as string };
        return;
      }
      file = loaded;
      info = check.info;
      phase = { kind: "ready" };
      await tick();
      if (mine !== token) return;
      await mount(check.data, mine);
    } catch (e) {
      if (mine !== token) return;
      phase = { kind: "error", message: $t("motion.lottie.open_error", { name: api.fileName(path), error: api.errorText(e) }) as string };
    }
  }

  async function mount(data: Record<string, unknown>, mine: number) {
    if (!host) return;
    const mod = await import("lottie-web/build/player/lottie_light");
    if (mine !== token || !host) return;
    const lottie = mod.default;
    destroy();
    host.replaceChildren();
    const autoplay = !reduced.current && active;
    anim = lottie.loadAnimation({
      container: host,
      renderer: "svg",
      loop: looping,
      autoplay,
      animationData: data,
      rendererSettings: { preserveAspectRatio: "xMidYMid meet" },
    });
    anim.setSpeed(speed);
    anim.addEventListener("enterFrame", (e) => (frame = e.currentTime));
    anim.addEventListener("complete", () => (playing = false));
    anim.addEventListener("data_failed", () => {
      phase = { kind: "error", message: $t("motion.lottie.render_error") as string };
    });
    playing = autoplay;
    frame = 0;
    if (!autoplay) anim.goToAndStop(0, true);
  }

  async function choose() {
    const picked = await openDialog({
      multiple: false,
      directory: false,
      filters: [{ name: "Lottie", extensions: ["json", "lottie"] }],
    }).catch(() => null);
    if (typeof picked === "string") await load(picked);
  }

  function togglePlay() {
    if (!anim) return;
    if (playing) {
      anim.pause();
      playing = false;
    } else {
      if (frame >= lastFrame - 0.5 && !looping) anim.goToAndPlay(0, true);
      else anim.play();
      playing = true;
    }
  }

  function scrub(value: number) {
    if (!anim) return;
    anim.goToAndStop(value, true);
    frame = value;
    playing = false;
  }

  function step(delta: number) {
    scrub(Math.min(lastFrame, Math.max(0, Math.round(frame) + delta)));
  }

  $effect(() => {
    anim?.setSpeed(speed);
  });

  $effect(() => {
    anim?.setLoop(looping);
  });

  $effect(() => {
    if (!active && anim && playing) {
      anim.pause();
      playing = false;
    }
  });

  function svgMarkup(width: number, height: number): string | null {
    const svg = host?.querySelector("svg");
    if (!svg || !info) return null;
    const clone = svg.cloneNode(true) as SVGSVGElement;
    clone.setAttribute("xmlns", "http://www.w3.org/2000/svg");
    clone.setAttribute("xmlns:xlink", "http://www.w3.org/1999/xlink");
    clone.setAttribute("width", String(width));
    clone.setAttribute("height", String(height));
    clone.setAttribute("viewBox", `0 0 ${info.width} ${info.height}`);
    clone.removeAttribute("style");
    return new XMLSerializer().serializeToString(clone);
  }

  function rasterize(markup: string, width: number, height: number, fill: string | null): Promise<string> {
    return new Promise((resolve, reject) => {
      const img = new Image();
      img.onload = () => {
        const canvas = document.createElement("canvas");
        canvas.width = width;
        canvas.height = height;
        const ctx = canvas.getContext("2d");
        if (!ctx) return reject(new Error($t("motion.lottie.canvas_error") as string));
        if (fill) {
          ctx.fillStyle = fill;
          ctx.fillRect(0, 0, width, height);
        }
        ctx.drawImage(img, 0, 0, width, height);
        try {
          resolve(canvas.toDataURL("image/png"));
        } catch (e) {
          reject(e);
        }
      };
      img.onerror = () => reject(new Error($t("motion.lottie.raster_error") as string));
      img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(markup)}`;
    });
  }

  async function exportPng() {
    if (!info || !file) return;
    const width = Math.max(1, Math.round(info.width * scale));
    const height = Math.max(1, Math.round(info.height * scale));
    const markup = svgMarkup(width, height);
    if (!markup) return;
    const path = await saveDialog({ defaultPath: pngName(file.name, frame), filters: [{ name: "PNG", extensions: ["png"] }] }).catch(() => null);
    if (!path) return;
    exportState = { kind: "busy" };
    try {
      const data = await rasterize(markup, width, height, withBackground ? background : null);
      const saved = await api.savePng(path, data);
      exportState = { kind: "ok", path: saved.path, name: api.fileName(saved.path) };
    } catch (e) {
      exportState = { kind: "error", message: $t("motion.lottie.export_error", { error: api.errorText(e) }) as string };
    }
  }

  function reset() {
    token++;
    destroy();
    file = null;
    info = null;
    phase = { kind: "empty" };
    exportState = { kind: "idle" };
  }

  onMount(() => {
    let disposed = false;
    const cleanups: (() => void)[] = [];
    try {
      void getCurrentWebview()
        .onDragDropEvent((event) => {
          if (!active) return;
          const p = event.payload as { type: string; paths?: string[] };
          if (p.type === "enter" || p.type === "over") dragging = true;
          else if (p.type === "leave") dragging = false;
          else if (p.type === "drop") {
            dragging = false;
            const hit = p.paths?.find((x) => /\.(json|lottie)$/i.test(x));
            if (hit) void load(hit);
            else if (p.paths?.length) phase = { kind: "error", message: $t("motion.lottie.drop_wrong") as string };
          }
        })
        .then((fn) => (disposed ? fn() : cleanups.push(fn)))
        .catch(() => {});
    } catch {
      dragging = false;
    }
    return () => {
      disposed = true;
      cleanups.forEach((fn) => fn());
      destroy();
    };
  });
</script>

<div class="lp" class:dragging>
  {#if phase.kind === "empty" || phase.kind === "error"}
    <div class="mo-card lp-empty">
      {#if phase.kind === "error"}
        <div class="lp-error" role="alert">
          <svg viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><circle cx="8" cy="8" r="6.5" /><path d="M8 4.8v3.8M8 11.1v.1" /></svg>
          <p>{phase.message}</p>
        </div>
      {/if}
      <svg class="lp-art" viewBox="0 0 48 48" width="48" height="48" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <rect x="6" y="8" width="36" height="32" rx="4" />
        <path d="M20 18v12l10-6z" />
      </svg>
      <h2 class="lp-title">{$t("motion.lottie.empty_title")}</h2>
      <p class="lp-body">{$t("motion.lottie.empty_body")}</p>
      <button type="button" class="btn btn-primary" onclick={choose}>{$t("motion.lottie.open")}</button>
    </div>
  {:else if phase.kind === "loading"}
    <div class="mo-card lp-empty" aria-busy="true">
      <span class="spinner" aria-hidden="true"></span>
      <p class="lp-body" role="status">{$t("motion.lottie.loading", { name: phase.name })}</p>
    </div>
  {:else if info && file}
    <section class="mo-card lp-player" aria-labelledby="lp-title">
      <div class="mo-card-head">
        <h2 id="lp-title" class="mo-card-title lp-name">{info.name || file.name}</h2>
        <button type="button" class="btn btn-secondary btn-sm" onclick={choose}>{$t("motion.lottie.open_other")}</button>
        <button type="button" class="btn btn-ghost btn-sm" onclick={reset}>{$t("motion.lottie.close")}</button>
      </div>
      <div class="lp-stage swatch-{swatch}" style:background-color={background ?? undefined} style:aspect-ratio="{info.width} / {info.height}" style:max-width="calc(420px * {info.width / Math.max(1, info.height)})">
        <div class="lp-host" bind:this={host}></div>
      </div>
      <div class="lp-scrub">
        <input
          type="range"
          min="0"
          max={lastFrame}
          step="1"
          value={Math.round(frame)}
          oninput={(e) => scrub(Number((e.currentTarget as HTMLInputElement).value))}
          aria-label={$t("motion.lottie.scrub")}
          aria-valuetext={$t("motion.lottie.frame_of", { frame: Math.round(frame), total: lastFrame })}
        />
        <span class="lp-clock">{Math.round(frame)} / {lastFrame} · {$t("motion.unit_s", { value: timeLabel((frame / info.fps) * 1000) })}</span>
      </div>
      <div class="mo-transport">
        <button type="button" class="btn btn-primary btn-sm" onclick={togglePlay} aria-pressed={playing}>{playing ? $t("motion.pause") : $t("motion.play")}</button>
        <button type="button" class="btn btn-ghost btn-sm" onclick={() => step(-1)} aria-label={$t("motion.lottie.prev_frame")}>‹</button>
        <button type="button" class="btn btn-ghost btn-sm" onclick={() => step(1)} aria-label={$t("motion.lottie.next_frame")}>›</button>
        <label class="mo-check"><input type="checkbox" bind:checked={looping} />{$t("motion.timeline.loop")}</label>
        <label class="lp-inline">
          {$t("motion.timeline.speed")}
          <select class="select" bind:value={speed}>
            {#each SPEEDS as s (s)}<option value={s}>{s}×</option>{/each}
          </select>
        </label>
        <div class="segmented" role="radiogroup" aria-label={$t("motion.lottie.background")}>
          {#each ["checker", "light", "dark", "custom"] as const as s (s)}
            <button type="button" class="segmented-btn" role="radio" aria-checked={swatch === s} onclick={() => (swatch = s)}>{$t(`motion.lottie.bg_${s}`)}</button>
          {/each}
        </div>
        {#if swatch === "custom"}
          <input class="lp-color" type="color" bind:value={custom} aria-label={$t("motion.lottie.bg_custom")} />
        {/if}
      </div>
      {#if reduced.current}
        <p class="mo-note" role="note">{$t("motion.reduced_note")}</p>
      {/if}
    </section>

    <section class="mo-card" aria-labelledby="lp-info-title">
      <h2 id="lp-info-title" class="mo-card-title">{$t("motion.lottie.info")}</h2>
      <dl class="lp-info">
        <div><dt>{$t("motion.lottie.size")}</dt><dd>{info.width} × {info.height}</dd></div>
        <div><dt>{$t("motion.lottie.fps")}</dt><dd>{num(info.fps, 2)}</dd></div>
        <div><dt>{$t("motion.lottie.frames")}</dt><dd>{num(info.frames, 0)}</dd></div>
        <div><dt>{$t("motion.duration")}</dt><dd>{$t("motion.unit_s", { value: timeLabel(info.durationMs) })}</dd></div>
        <div><dt>{$t("motion.lottie.layers")}</dt><dd>{info.layers}</dd></div>
        <div><dt>{$t("motion.lottie.assets")}</dt><dd>{info.assets}</dd></div>
        <div><dt>{$t("motion.lottie.format")}</dt><dd>{file.kind === "dotlottie" ? "dotLottie" : "Lottie JSON"}{info.version ? ` · v${info.version}` : ""}</dd></div>
        <div><dt>{$t("motion.lottie.file_size")}</dt><dd>{$t("motion.unit_kb", { value: num(file.size_bytes / 1024, 1) })}</dd></div>
      </dl>
      {#if file.kind === "dotlottie" && file.animation_count > 1}
        <p class="mo-meta">{$t("motion.lottie.first_of", { count: file.animation_count, id: file.animation_id ?? "" })}</p>
      {/if}
      {#if file.embedded_images > 0}
        <p class="mo-meta">{$t("motion.lottie.images_embedded", { count: file.embedded_images })}</p>
      {/if}
      {#if info.is3d}
        <p class="mo-note">{$t("motion.lottie.is3d")}</p>
      {/if}

      <h3 class="mo-sub">{$t("motion.lottie.export_title")}</h3>
      <div class="mo-transport">
        <label class="lp-inline">
          {$t("motion.lottie.scale")}
          <select class="select" bind:value={scale}>
            {#each [0.5, 1, 2, 3] as s (s)}<option value={s}>{s}× · {Math.round(info.width * s)}×{Math.round(info.height * s)}</option>{/each}
          </select>
        </label>
        <label class="mo-check" title={background ? undefined : ($t("motion.lottie.bg_transparent_hint") as string)}>
          <input type="checkbox" bind:checked={withBackground} disabled={!background} />
          {$t("motion.lottie.include_bg")}
        </label>
      </div>
      <button type="button" class="btn btn-secondary" onclick={exportPng} disabled={exportState.kind === "busy"}>
        {exportState.kind === "busy" ? $t("motion.lottie.exporting") : $t("motion.lottie.export_png", { frame: Math.round(frame) })}
      </button>
      <div class="mo-status" role="status" aria-live="polite">
        {#if exportState.kind === "ok"}
          {@const p = exportState.path}
          <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" class="mo-ok" aria-hidden="true"><circle cx="8" cy="8" r="6.5" /><path d="m5.2 8.2 1.9 1.9 3.7-4" /></svg>
          <span>{$t("motion.lottie.exported", { name: exportState.name })}</span>
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => void api.reveal(p).catch(() => {})}>{$t("motion.show_in_folder")}</button>
        {:else if exportState.kind === "error"}
          <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" class="mo-bad" aria-hidden="true"><circle cx="8" cy="8" r="6.5" /><path d="M8 4.8v3.8M8 11.1v.1" /></svg>
          <span>{exportState.message}</span>
        {/if}
      </div>
    </section>
  {/if}

  {#if dragging}
    <div class="lp-dropping" aria-hidden="true"><p>{$t("motion.lottie.drop_here")}</p></div>
  {/if}
</div>

<style>
  .lp {
    position: relative;
    display: grid;
    grid-template-columns: 1fr minmax(260px, 320px);
    gap: var(--space-4);
    align-items: start;
  }

  .lp-empty {
    grid-column: 1 / -1;
    align-items: center;
    justify-content: center;
    min-height: 320px;
    text-align: center;
  }

  .lp-art {
    color: var(--text-dim);
  }

  .lp-title {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .lp-body {
    margin: 0;
    max-width: 440px;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .lp-error {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    max-width: 520px;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-1);
    color: var(--error);
    text-align: left;
  }

  .lp-error p {
    margin: 0;
    color: var(--text);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  .lp-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .lp-stage {
    width: 100%;
    margin-inline: auto;
    border-radius: var(--radius-md);
    overflow: hidden;
    display: flex;
    justify-content: center;
  }

  .lp-host {
    width: 100%;
    height: 100%;
  }

  .lp-host :global(svg) {
    display: block;
  }

  .swatch-checker {
    background: repeating-conic-gradient(var(--fill-2) 0 25%, var(--fill-1) 0 50%) 0 0 / 16px 16px;
  }

  .lp-scrub {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .lp-scrub input {
    flex: 1;
    accent-color: var(--accent);
  }

  .lp-clock {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .lp-inline {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .lp-color {
    width: 32px;
    height: var(--control-h);
    padding: 0;
    border: none;
    background: none;
  }

  .lp-info {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2);
    margin: 0;
  }

  .lp-info div {
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }

  .lp-info dt {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .lp-info dd {
    margin: 2px 0 0;
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }

  .lp-dropping {
    position: absolute;
    inset: 0;
    z-index: 2;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 2px dashed var(--accent);
    border-radius: var(--radius-xl);
    background: color-mix(in srgb, var(--pane-bg) 82%, transparent);
    pointer-events: none;
  }

  .lp-dropping p {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  @media (max-width: 760px) {
    .lp {
      grid-template-columns: 1fr;
    }
  }
</style>
