<script lang="ts">
  import { inP3, inSrgb, oklchToSrgb, type Oklch } from "$lib/palette/color";

  let { color, label, onpick }: { color: Oklch; label: string; onpick: (l: number, c: number) => void } = $props();

  const W = 240;
  const H = 150;
  const C_MAX = 0.4;
  let canvas = $state<HTMLCanvasElement | null>(null);
  let dragging = false;

  $effect(() => {
    const ctx = canvas?.getContext("2d");
    if (!ctx) return;
    const h = color.h;
    const img = ctx.createImageData(W, H);
    for (let y = 0; y < H; y++) {
      const l = 1 - y / (H - 1);
      for (let x = 0; x < W; x++) {
        const c = (x / (W - 1)) * C_MAX;
        const col = { l, c, h };
        const i = (y * W + x) * 4;
        if (inSrgb(col)) {
          const rgb = oklchToSrgb(col);
          img.data[i] = rgb.r * 255;
          img.data[i + 1] = rgb.g * 255;
          img.data[i + 2] = rgb.b * 255;
          img.data[i + 3] = 255;
        } else if (inP3(col)) {
          const rgb = oklchToSrgb({ l, c: 0, h });
          img.data[i] = rgb.r * 255;
          img.data[i + 1] = rgb.g * 255;
          img.data[i + 2] = rgb.b * 255;
          img.data[i + 3] = 90;
        } else {
          img.data[i + 3] = 0;
        }
      }
    }
    ctx.putImageData(img, 0, 0);
  });

  function pick(e: PointerEvent) {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const x = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
    const y = Math.min(1, Math.max(0, (e.clientY - rect.top) / rect.height));
    onpick(1 - y, x * C_MAX);
  }

  function down(e: PointerEvent) {
    dragging = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    pick(e);
  }

  function key(e: KeyboardEvent) {
    const stepL = e.shiftKey ? 0.05 : 0.01;
    const stepC = e.shiftKey ? 0.02 : 0.004;
    const moves: Record<string, [number, number]> = { ArrowUp: [stepL, 0], ArrowDown: [-stepL, 0], ArrowRight: [0, stepC], ArrowLeft: [0, -stepC] };
    const m = moves[e.key];
    if (!m) return;
    e.preventDefault();
    onpick(Math.min(1, Math.max(0, color.l + m[0])), Math.min(C_MAX, Math.max(0, color.c + m[1])));
  }
</script>

<div
  class="pl-plane"
  role="slider"
  tabindex="0"
  aria-label={label}
  aria-valuetext={`L ${(color.l * 100).toFixed(1)}%, C ${color.c.toFixed(3)}`}
  aria-valuenow={Math.round(color.l * 100)}
  aria-valuemin={0}
  aria-valuemax={100}
  onpointerdown={down}
  onpointermove={(e) => dragging && pick(e)}
  onpointerup={() => (dragging = false)}
  onkeydown={key}
>
  <canvas bind:this={canvas} width={W} height={H}></canvas>
  <span class="pl-plane-dot" style:left="{(Math.min(color.c, C_MAX) / C_MAX) * 100}%" style:top="{(1 - color.l) * 100}%" aria-hidden="true"></span>
</div>

<style>
  .pl-plane {
    position: relative;
    width: 100%;
    aspect-ratio: 240 / 150;
    border-radius: var(--radius-md);
    background: repeating-conic-gradient(var(--surface-hi) 0% 25%, var(--surface) 0% 50%) 0 0 / 12px 12px;
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    overflow: hidden;
    cursor: crosshair;
    touch-action: none;
  }

  .pl-plane:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }

  .pl-plane canvas {
    display: block;
    width: 100%;
    height: 100%;
  }

  .pl-plane-dot {
    position: absolute;
    width: 12px;
    height: 12px;
    margin: -6px 0 0 -6px;
    border-radius: var(--radius-full);
    box-shadow: 0 0 0 2px var(--secondary), 0 0 0 3px rgba(var(--shadow-ink), var(--elev-alpha-3));
    pointer-events: none;
  }
</style>
