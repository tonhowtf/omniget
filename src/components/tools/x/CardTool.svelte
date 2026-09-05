<script lang="ts">
  /** Post → imagem (estudo 67): render em canvas, como TwitterShots/BrandBird, sem servidor. */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { reveal, saveAs } from "$lib/tools/rt";
  import { fmtN, xErr, type XPost } from "$lib/tools/x";

  type Theme = "light" | "dark" | "dim";
  const BACKGROUNDS: Record<string, [string, string]> = {
    none: ["", ""],
    sky: ["#5AA9FF", "#1E6FE8"],
    sunset: ["#FF8A5B", "#E0303A"],
    grape: ["#C77DFF", "#5B3FD8"],
    mint: ["#4CD964", "#1A9EB5"],
    ink: ["#3A3A44", "#101014"],
  };

  let url = $state("");
  let busy = $state<string | null>(null);
  let post = $state<XPost | null>(null);
  let avatar = $state<HTMLImageElement | null>(null);
  let photo = $state<HTMLImageElement | null>(null);
  let theme = $state<Theme>("light");
  let bg = $state("sky");
  let scale = $state(2);
  let showMetrics = $state(true);
  let showMedia = $state(true);
  let canvas = $state<HTMLCanvasElement | null>(null);
  let previewUrl = $state("");

  async function loadImage(src: string): Promise<HTMLImageElement | null> {
    try {
      const data = await invoke<string>("tool_x_data_url", { url: src });
      return await new Promise((resolve) => {
        const img = new Image();
        img.onload = () => resolve(img);
        img.onerror = () => resolve(null);
        img.src = data;
      });
    } catch {
      return null;
    }
  }

  async function fetchPost() {
    if (!url.trim() || busy) return;
    busy = "fetch";
    post = null;
    previewUrl = "";
    try {
      const p = await invoke<XPost>("tool_x_post", { input: url });
      post = p;
      avatar = p.author.avatar ? await loadImage(p.author.avatar) : null;
      const first = p.media.find((m) => m.kind === "photo") ?? p.media[0];
      photo = first ? await loadImage(first.kind === "photo" ? first.url : first.thumb) : null;
      render();
    } catch (e) {
      showToast("error", xErr(e));
    } finally {
      busy = null;
    }
  }

  function wrap(ctx: CanvasRenderingContext2D, text: string, maxWidth: number): string[] {
    const lines: string[] = [];
    for (const para of text.split("\n")) {
      const words = para.split(" ");
      let line = "";
      for (const w of words) {
        const test = line ? `${line} ${w}` : w;
        if (ctx.measureText(test).width > maxWidth && line) {
          lines.push(line);
          line = w;
        } else {
          line = test;
        }
      }
      lines.push(line);
    }
    return lines;
  }

  function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
    ctx.beginPath();
    ctx.moveTo(x + r, y);
    ctx.arcTo(x + w, y, x + w, y + h, r);
    ctx.arcTo(x + w, y + h, x, y + h, r);
    ctx.arcTo(x, y + h, x, y, r);
    ctx.arcTo(x, y, x + w, y, r);
    ctx.closePath();
  }

  export function render() {
    if (!post || !canvas) return;
    const p = post;
    const W = 640;
    const pad = 28;
    const cardW = W - (bg === "none" ? 0 : 56);
    const inner = cardW - pad * 2;
    const c = document.createElement("canvas");
    const ctx = c.getContext("2d")!;
    const font = "-apple-system, 'SF Pro Text', 'Segoe UI', Roboto, Helvetica, Arial, sans-serif";
    ctx.font = `400 21px ${font}`;
    const lines = wrap(ctx, p.text, inner);
    const lineH = 30;
    let mediaH = 0;
    if (showMedia && photo) {
      mediaH = Math.min(420, Math.round((inner * photo.naturalHeight) / photo.naturalWidth)) + 16;
    }
    const metricsH = showMetrics ? 44 : 0;
    const cardH = pad + 56 + 18 + lines.length * lineH + mediaH + 34 + metricsH + pad;
    const H = cardH + (bg === "none" ? 0 : 56);
    c.width = W * scale;
    c.height = H * scale;
    ctx.scale(scale, scale);
    const colors = theme === "light" ? { card: "#ffffff", text: "#0f1419", muted: "#536471", line: "#eff3f4" } : theme === "dim" ? { card: "#15202b", text: "#f7f9f9", muted: "#8b98a5", line: "#38444d" } : { card: "#000000", text: "#e7e9ea", muted: "#71767b", line: "#2f3336" };
    if (bg !== "none") {
      const [a, b] = BACKGROUNDS[bg];
      const g = ctx.createLinearGradient(0, 0, W, H);
      g.addColorStop(0, a);
      g.addColorStop(1, b);
      ctx.fillStyle = g;
      ctx.fillRect(0, 0, W, H);
    }
    const cx = bg === "none" ? 0 : 28;
    const cy = bg === "none" ? 0 : 28;
    if (bg !== "none") {
      ctx.shadowColor = "rgba(0,0,0,0.25)";
      ctx.shadowBlur = 30;
      ctx.shadowOffsetY = 12;
    }
    roundRect(ctx, cx, cy, cardW, cardH, bg === "none" ? 0 : 24);
    ctx.fillStyle = colors.card;
    ctx.fill();
    ctx.shadowColor = "transparent";
    let y = cy + pad;
    const x = cx + pad;
    // avatar
    if (avatar) {
      ctx.save();
      ctx.beginPath();
      ctx.arc(x + 26, y + 26, 26, 0, Math.PI * 2);
      ctx.clip();
      ctx.drawImage(avatar, x, y, 52, 52);
      ctx.restore();
    } else {
      ctx.fillStyle = colors.line;
      ctx.beginPath();
      ctx.arc(x + 26, y + 26, 26, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.fillStyle = colors.text;
    ctx.font = `700 19px ${font}`;
    ctx.textBaseline = "alphabetic";
    ctx.fillText(p.author.name, x + 64, y + 22);
    if (p.author.verified) {
      const nw = ctx.measureText(p.author.name).width;
      ctx.fillStyle = "#1d9bf0";
      ctx.beginPath();
      ctx.arc(x + 64 + nw + 12, y + 16, 8, 0, Math.PI * 2);
      ctx.fill();
      ctx.strokeStyle = "#fff";
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.moveTo(x + 64 + nw + 8, y + 16);
      ctx.lineTo(x + 64 + nw + 11, y + 19);
      ctx.lineTo(x + 64 + nw + 16, y + 12);
      ctx.stroke();
    }
    ctx.fillStyle = colors.muted;
    ctx.font = `400 17px ${font}`;
    ctx.fillText(`@${p.author.handle}`, x + 64, y + 45);
    // X mark
    ctx.fillStyle = colors.text;
    ctx.font = `800 26px ${font}`;
    ctx.fillText("𝕏", x + inner - 26, y + 30);
    y += 56 + 18;
    ctx.fillStyle = colors.text;
    ctx.font = `400 21px ${font}`;
    for (const l of lines) {
      y += lineH;
      ctx.fillText(l, x, y - 8);
    }
    if (showMedia && photo) {
      y += 8;
      const h = mediaH - 16;
      ctx.save();
      roundRect(ctx, x, y, inner, h, 16);
      ctx.clip();
      const r = Math.max(inner / photo.naturalWidth, h / photo.naturalHeight);
      const dw = photo.naturalWidth * r;
      const dh = photo.naturalHeight * r;
      ctx.drawImage(photo, x + (inner - dw) / 2, y + (h - dh) / 2, dw, dh);
      ctx.restore();
      y += h + 8;
    }
    y += 26;
    ctx.fillStyle = colors.muted;
    ctx.font = `400 15px ${font}`;
    const d = new Date(p.created_at);
    ctx.fillText(d.toLocaleString(undefined, { hour: "2-digit", minute: "2-digit", day: "numeric", month: "short", year: "numeric" }), x, y);
    if (showMetrics) {
      y += 14;
      ctx.strokeStyle = colors.line;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(x, y);
      ctx.lineTo(x + inner, y);
      ctx.stroke();
      y += 30;
      ctx.font = `600 16px ${font}`;
      let mx = x;
      const items: [string, number][] = [["♥", p.likes], ["↻", p.reposts], ["💬", p.replies]];
      if (p.views) items.push(["👁", p.views]);
      for (const [icon, n] of items) {
        ctx.fillStyle = colors.muted;
        ctx.fillText(icon, mx, y);
        ctx.fillStyle = colors.text;
        ctx.fillText(fmtN(n), mx + 26, y);
        mx += 26 + ctx.measureText(fmtN(n)).width + 30;
      }
    }
    canvas.width = c.width;
    canvas.height = c.height;
    canvas.getContext("2d")!.drawImage(c, 0, 0);
    previewUrl = c.toDataURL("image/png");
  }

  async function save() {
    if (!post || !previewUrl) return;
    const dest = await saveAs(`x-${post.author.handle}-${post.id}.png`, [{ name: "PNG", extensions: ["png"] }]);
    if (!dest) return;
    busy = "save";
    try {
      const path = await invoke<string>("tool_x_save_data_url", { dataUrl: previewUrl, dest });
      showToast("success", $t("tools.common.done") as string);
      await reveal(path);
    } catch (e) {
      showToast("error", xErr(e));
    } finally {
      busy = null;
    }
  }

  async function copy() {
    if (!previewUrl) return;
    try {
      const blob = await (await fetch(previewUrl)).blob();
      await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
      showToast("success", $t("tools.common.copied") as string);
    } catch {
      showToast("error", $t("tools.x.copy_image_fail") as string);
    }
  }

  $effect(() => {
    void theme;
    void bg;
    void scale;
    void showMetrics;
    void showMedia;
    render();
  });
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="url" bind:value={url} placeholder={$t("tools.x.post_placeholder")} onkeydown={(e) => e.key === "Enter" && fetchPost()} /></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy !== null || !url.trim()} onclick={fetchPost}>{busy === "fetch" ? $t("tools.common.working") : $t("tools.x.load_post")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.x.card_theme")}</div></div>
        <div class="group-row-trailing"><div class="segmented">{#each ["light", "dim", "dark"] as th (th)}<button class="segmented-btn" class:active={theme === th} type="button" onclick={() => (theme = th as Theme)}>{$t(`tools.x.theme_${th}`)}</button>{/each}</div></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.x.card_bg")}</div></div>
        <div class="group-row-trailing swatches">
          {#each Object.entries(BACKGROUNDS) as [k, [a, b]] (k)}
            <button class="swatch" class:active={bg === k} type="button" title={k} style:background={k === "none" ? "transparent" : `linear-gradient(135deg, ${a}, ${b})`} onclick={() => (bg = k)}>{k === "none" ? "∅" : ""}</button>
          {/each}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.x.card_scale")}</div></div>
        <div class="group-row-trailing btn-row">
          <div class="segmented">{#each [1, 2, 3] as s (s)}<button class="segmented-btn" class:active={scale === s} type="button" onclick={() => (scale = s)}>{s}×</button>{/each}</div>
          <label class="opt"><input class="checkbox" type="checkbox" bind:checked={showMetrics} /> {$t("tools.x.card_metrics")}</label>
          <label class="opt"><input class="checkbox" type="checkbox" bind:checked={showMedia} /> {$t("tools.x.card_media")}</label>
        </div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row preview-row">
        <div class="preview" class:empty={!post}>
          {#if !post}<div class="hint">{$t("tools.x.card_hint")}</div>{/if}
          <canvas bind:this={canvas} class:hidden={!post}></canvas>
        </div>
      </div>
      {#if post}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-sub">{Math.round((canvas?.width ?? 0))} × {Math.round((canvas?.height ?? 0))} px</div></div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-secondary btn-sm" type="button" onclick={copy}>{$t("tools.x.copy_image")}</button>
            <button class="btn btn-primary btn-sm" type="button" disabled={busy !== null} onclick={save}>{busy === "save" ? $t("tools.common.working") : $t("tools.x.save_png")}</button>
          </div>
        </div>
      {/if}
    </div>
  </section>
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .opt { display: inline-flex; align-items: center; gap: var(--space-1); font-size: var(--text-sm); }
  .swatches { display: flex; gap: var(--space-1); }
  .swatch { width: 28px; height: 28px; border-radius: 50%; border: 2px solid transparent; cursor: pointer; color: var(--text-muted); font-size: 14px; box-shadow: inset 0 0 0 1px var(--content-border); }
  .swatch.active { border-color: var(--accent); }
  .preview-row { justify-content: center; }
  .preview { width: 100%; display: flex; justify-content: center; padding: var(--space-3) 0; }
  .preview canvas { max-width: 100%; height: auto; border-radius: var(--radius-lg); box-shadow: 0 10px 30px -12px rgba(0, 0, 0, 0.4); }
  .hidden { display: none; }
  .hint { color: var(--text-muted); font-size: var(--text-sm); padding: var(--space-6); text-align: center; }
</style>
