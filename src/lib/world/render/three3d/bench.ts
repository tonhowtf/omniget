// M0 bench of the 3D city: one region with ~2k and ~10k instances, with and
// without shadow, measured like estudo 01's perf page (render + readPixels
// per frame, rAF paced, camera orbiting so culling varies) plus the first
// frame and compileAsync. A fresh WebGLRenderer per case, so every case pays
// its own shader compile. Ends with a sleep/wake round trip checked by pixel.
// Runs at `/city?bench=1`.

import { Vector3 } from 'three';
import { fallbackKit } from './kit';
import { PaletteBook } from './palette';
import { buildRegion, REGION_M, type RegionMeshData } from './region';
import { createThreeRenderer } from './renderer';
import { tauriSource } from './source';
import { SYNTH_PALETTES, SYNTH_STYLE, synthRegion } from './synth';

export interface BenchCase {
  label: string;
  instances: number;
  shadows: boolean;
  source: 'synthetic' | 'region';
}

export interface BenchRow extends BenchCase {
  buildMs: number;
  compileMs: number;
  firstMs: number;
  p50: number;
  p95: number;
  cpuP50: number;
  drawCalls: number;
  triangles: number;
  gpuBytes: number;
  gpu: string;
  error?: string;
}

export interface BenchReport {
  rows: BenchRow[];
  restore: { ok: boolean; beforePixel: number[] | null; afterPixel: number[] | null; canvasSwapped: boolean; ms: number };
  width: number;
  height: number;
  userAgent: string;
  startedAt: string;
}

const W = 1280;
const H = 720;

function pct(values: number[], p: number): number {
  if (!values.length) return 0;
  const s = [...values].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.floor(s.length * p))];
}

const round = (n: number) => Math.round(n * 100) / 100;

function nextFrame(): Promise<number> {
  return new Promise((r) => requestAnimationFrame(r));
}

async function runCase(host: HTMLElement, c: BenchCase, data: RegionMeshData, frames: number, warm: number): Promise<BenchRow> {
  const canvas = document.createElement('canvas');
  canvas.width = W;
  canvas.height = H;
  canvas.style.width = '100%';
  canvas.style.aspectRatio = `${W} / ${H}`;
  host.replaceChildren(canvas);
  const r = createThreeRenderer();
  const row: BenchRow = { ...c, buildMs: 0, compileMs: 0, firstMs: 0, p50: 0, p95: 0, cpuP50: 0, drawCalls: 0, triangles: 0, gpuBytes: 0, gpu: '' };
  try {
    const caps = await r.init(canvas, { tier: 2, dpr: 1, preserveDrawingBuffer: false });
    row.gpu = caps.gpu;
    r.resize(W, H, 1);
    const gl = r.gl!;
    gl.shadowMap.enabled = c.shadows;
    r.sun.castShadow = c.shadows;
    const palettes = new PaletteBook(SYNTH_PALETTES);
    const kit = fallbackKit(SYNTH_STYLE);
    const t0 = performance.now();
    const built = buildRegion(data, kit, palettes, r.materials, { outline: false, shadows: c.shadows });
    r.scene.add(built.group);
    row.buildMs = round(performance.now() - t0);
    row.instances = built.instanceCount;

    const center = built.center.clone();
    const cam = r.camera;
    const orbit = (i: number) => {
      const a = i * 0.02;
      const rad = REGION_M * 0.55;
      cam.position.set(center.x + Math.cos(a) * rad, rad * 0.8, center.z + Math.sin(a) * rad);
      cam.lookAt(center);
      r.followShadow(new Vector3(center.x, 0, center.z));
    };
    orbit(0);
    const g = gl.getContext();
    const px = new Uint8Array(4);

    row.compileMs = round(await r.compile());
    const f0 = performance.now();
    r.frame(16, true);
    g.readPixels(0, 0, 1, 1, g.RGBA, g.UNSIGNED_BYTE, px);
    row.firstMs = round(performance.now() - f0);

    const times: number[] = [];
    const cpu: number[] = [];
    for (let i = 0; i < warm + frames; i++) {
      orbit(i);
      await nextFrame();
      const s = performance.now();
      const st = r.frame(16, true);
      g.readPixels(0, 0, 1, 1, g.RGBA, g.UNSIGNED_BYTE, px);
      const e = performance.now();
      if (i >= warm) {
        times.push(e - s);
        cpu.push(st.cpuMs);
      }
      row.drawCalls = st.drawCalls;
      row.triangles = st.triangles;
    }
    row.p50 = round(pct(times, 0.5));
    row.p95 = round(pct(times, 0.95));
    row.cpuP50 = round(pct(cpu, 0.5));
    row.gpuBytes = r.memory().bytes;
    built.dispose();
    kit.dispose();
  } catch (e) {
    row.error = e instanceof Error ? e.message : String(e);
  } finally {
    r.destroy();
  }
  return row;
}

async function restoreCheck(host: HTMLElement, data: RegionMeshData): Promise<BenchReport['restore']> {
  const canvas = document.createElement('canvas');
  canvas.width = 640;
  canvas.height = 360;
  canvas.style.width = '100%';
  host.replaceChildren(canvas);
  let swapped = false;
  const r = createThreeRenderer({ onCanvasReplaced: () => (swapped = true) });
  const t0 = performance.now();
  try {
    await r.init(canvas, { tier: 1, dpr: 1 });
    r.resize(640, 360, 1);
    const kit = fallbackKit(SYNTH_STYLE);
    const built = buildRegion(data, kit, new PaletteBook(SYNTH_PALETTES), r.materials, { outline: false, shadows: true });
    r.scene.add(built.group);
    r.camera.position.set(built.center.x + 60, 70, built.center.z + 60);
    r.camera.lookAt(built.center);
    r.frame(16, true);
    const before = r.probe(320, 180);
    r.sleep();
    await new Promise((res) => setTimeout(res, 150));
    await r.wake();
    r.frame(16, true);
    const after = r.probe(320, 180);
    const ok = !!before && !!after && after[3] === 255 && before.every((v, i) => Math.abs(v - after[i]) <= 2);
    built.dispose();
    kit.dispose();
    return { ok, beforePixel: before, afterPixel: after, canvasSwapped: swapped, ms: round(performance.now() - t0) };
  } catch {
    return { ok: false, beforePixel: null, afterPixel: null, canvasSwapped: swapped, ms: round(performance.now() - t0) };
  } finally {
    r.destroy();
  }
}

export async function runCityBench(
  host: HTMLElement,
  opts: { frames?: number; warmup?: number; onRow?: (row: BenchRow) => void } = {},
): Promise<BenchReport> {
  const frames = opts.frames ?? 120;
  const warm = opts.warmup ?? 30;
  const startedAt = new Date().toISOString();
  const cases: Array<{ c: BenchCase; data: RegionMeshData }> = [];
  for (const n of [2000, 10000]) {
    const data = synthRegion([0, 0], n).mesh;
    for (const shadows of [false, true]) {
      cases.push({ c: { label: `${n / 1000}k${shadows ? ' + shadow' : ''}`, instances: n, shadows, source: 'synthetic' }, data });
    }
  }
  // The real region 0,0 when the app has a city (agent C's mesher through G2).
  try {
    const real = await tauriSource().mesh([0, 0]);
    if (real?.instances?.instances?.length) {
      for (const shadows of [false, true]) {
        cases.push({
          c: { label: `region 0,0${shadows ? ' + shadow' : ''}`, instances: real.instances.instances.length, shadows, source: 'region' },
          data: real,
        });
      }
    }
  } catch {
    // No city yet: synthetic rows only.
  }
  const rows: BenchRow[] = [];
  for (const { c, data } of cases) {
    const row = await runCase(host, c, data, frames, warm);
    rows.push(row);
    opts.onRow?.(row);
  }
  const restore = await restoreCheck(host, cases[0].data);
  return { rows, restore, width: W, height: H, userAgent: typeof navigator !== 'undefined' ? navigator.userAgent : '', startedAt };
}
