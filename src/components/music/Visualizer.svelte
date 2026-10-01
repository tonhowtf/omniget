<script lang="ts">
  import { onMount } from "svelte";
  import { registerSemantic } from "$lib/native-agent/semantic";
  import { getAnalyser, player } from "$lib/music/player-store.svelte";
  import type { VisualizerMode } from "$lib/music/player-store.svelte";

  let { mode = "bars", bars = 64 }: { mode?: VisualizerMode; bars?: number } = $props();

  let canvas: HTMLCanvasElement | undefined = $state();
  const rid = `music-visualizer-${Math.random().toString(36).slice(2, 8)}`;
  onMount(() => registerSemantic(rid, () => ({ ready: true, kind: "audio visualizer", mode, playing: player.playing })));
  let box: HTMLDivElement | undefined = $state();
  let w = $state(0);
  let h = $state(0);
  let visible = $state(true);

  $effect(() => {
    if (!box) return;
    const ro = new ResizeObserver((e) => {
      w = Math.floor(e[0].contentRect.width);
      h = Math.floor(e[0].contentRect.height);
    });
    ro.observe(box);
    const io = new IntersectionObserver((e) => (visible = e[0].isIntersecting));
    io.observe(box);
    return () => {
      ro.disconnect();
      io.disconnect();
    };
  });

  $effect(() => {
    const c = canvas;
    if (!c || !w || !h || !visible || mode === "off") return;
    const reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    const dpr = window.devicePixelRatio || 1;
    c.width = Math.round(w * dpr);
    c.height = Math.round(h * dpr);
    const g = c.getContext("2d");
    if (!g) return;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    const styles = getComputedStyle(c);
    const accent = styles.getPropertyValue("--accent").trim() || "#ff9f0a";
    const dim = styles.getPropertyValue("--viz-dim").trim() || "rgba(255,255,255,.18)";
    let freq = new Uint8Array(1024);
    let time = new Uint8Array(2048);
    const smooth = new Float32Array(bars);
    let raf = 0;
    let last = 0;

    const draw = (now: number) => {
      raf = requestAnimationFrame(draw);
      if (reduce && now - last < 120) return;
      last = now;
      const a = getAnalyser();
      g.clearRect(0, 0, w, h);
      if (!a || !player.playing) {
        drawIdle();
        return;
      }
      if (freq.length !== a.frequencyBinCount) freq = new Uint8Array(a.frequencyBinCount);
      if (time.length !== a.fftSize) time = new Uint8Array(a.fftSize);
      if (mode === "wave") {
        a.getByteTimeDomainData(time);
        g.lineWidth = 2;
        g.strokeStyle = accent;
        g.beginPath();
        for (let i = 0; i < time.length; i++) {
          const x = (i / (time.length - 1)) * w;
          const y = (time[i] / 255) * h;
          if (i) g.lineTo(x, y);
          else g.moveTo(x, y);
        }
        g.stroke();
        return;
      }
      a.getByteFrequencyData(freq);
      const nyq = a.context.sampleRate / 2;
      const lo = Math.log10(30);
      const hi = Math.log10(Math.min(16000, nyq));
      for (let b = 0; b < bars; b++) {
        const f0 = Math.pow(10, lo + ((hi - lo) * b) / bars);
        const f1 = Math.pow(10, lo + ((hi - lo) * (b + 1)) / bars);
        const i0 = Math.floor((f0 / nyq) * freq.length);
        const i1 = Math.max(i0 + 1, Math.floor((f1 / nyq) * freq.length));
        let v = 0;
        for (let i = i0; i < i1 && i < freq.length; i++) v = Math.max(v, freq[i]);
        const target = v / 255;
        smooth[b] = target > smooth[b] ? target : smooth[b] * 0.9 + target * 0.1;
      }
      if (mode === "circle") drawCircle();
      else drawBars();
    };

    const drawBars = () => {
      const gap = 2;
      const bw = Math.max(1, w / bars - gap);
      g.fillStyle = accent;
      for (let b = 0; b < bars; b++) {
        const bh = Math.max(2, smooth[b] * h);
        g.globalAlpha = 0.35 + 0.65 * smooth[b];
        g.beginPath();
        g.roundRect(b * (bw + gap), h - bh, bw, bh, Math.min(3, bw / 2));
        g.fill();
      }
      g.globalAlpha = 1;
    };

    const drawCircle = () => {
      const cx = w / 2;
      const cy = h / 2;
      const r = Math.min(w, h) * 0.32;
      g.strokeStyle = accent;
      g.lineCap = "round";
      g.lineWidth = Math.max(2, (2 * Math.PI * r) / bars / 2.2);
      for (let b = 0; b < bars; b++) {
        const ang = (b / bars) * Math.PI * 2 - Math.PI / 2;
        const len = 4 + smooth[b] * Math.min(w, h) * 0.16;
        g.globalAlpha = 0.4 + 0.6 * smooth[b];
        g.beginPath();
        g.moveTo(cx + Math.cos(ang) * r, cy + Math.sin(ang) * r);
        g.lineTo(cx + Math.cos(ang) * (r + len), cy + Math.sin(ang) * (r + len));
        g.stroke();
      }
      g.globalAlpha = 1;
    };

    const drawIdle = () => {
      g.fillStyle = dim;
      if (mode === "circle") return;
      const gap = 2;
      const bw = Math.max(1, w / bars - gap);
      for (let b = 0; b < bars; b++) {
        g.beginPath();
        g.roundRect(b * (bw + gap), h - 2, bw, 2, 1);
        g.fill();
      }
    };

    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  });
</script>

<div class="viz" bind:this={box} aria-hidden="true">
  {#if mode !== "off"}
    <canvas bind:this={canvas} data-agent-renderer={rid} style:width="{w}px" style:height="{h}px"></canvas>
  {/if}
</div>

<style>
  .viz {
    --viz-dim: color-mix(in srgb, var(--secondary) 18%, transparent);
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 24px;
  }
  canvas {
    position: absolute;
    inset: 0;
    display: block;
  }
</style>
