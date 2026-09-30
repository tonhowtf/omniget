// City bench for CI (`OMNIGET_WORLD_BENCH_ROUTE=/city`): the real example
// city (founded without LLM), the real kit, both backends, in the real Tauri
// webview. The Rust hook opens `/city?bench=<scenario>`, the route calls
// `runCityCiBench(host, scenario)`, and the JSON goes to `world_bench_report`
// like the `/world` bench's.
//
// What it measures, each over the same stretch of frames at 1280×720, DPR 1:
// - `city.three3d`: the 3D city pinned to tier 0 (no shadow, the tier's LOD
//   and instance budget), `renderer.frame(force)` + a 1 px readback;
// - `city.lite`: the lite mode (2D renderer + baked sprites), `drawNow()`
//   (frame + readback) while the camera circles and people walk;
// - `calibration`: the 3D calibration scene and the backend it would pick.
// `scene` repeats the lite numbers, so `scripts/world-bench.mjs`'s sanity
// gate (frames > 0, fps > 0) applies to the backend a weak machine gets.
//
// Same rule as `runner.ts`: it reports, it does not decide. A backend that
// fails becomes an `error` field inside its block.

import { invoke, Channel } from "@tauri-apps/api/core";
import { percentile, round } from "./ipc-bench";

export const CITY_BENCH_W = 1280;
export const CITY_BENCH_H = 720;
export const CITY_BENCH_WARMUP = 30;
export const CITY_BENCH_FRAMES = 240;
/** Inside the process deadline the workflow gives the city (300 s). */
export const CITY_REPORT_DEADLINE_MS = 240_000;

export interface CityBackendReport {
  p50_ms: number;
  p95_ms: number;
  cpu_ms: number;
  frames: number;
  first_frame_ms: number;
  ready_ms: number;
  draw_calls: number;
  /** Instances drawn (3D) or sprites drawn (lite). */
  drawn: number;
  error?: string;
}

const empty = (): CityBackendReport => ({
  p50_ms: 0,
  p95_ms: 0,
  cpu_ms: 0,
  frames: 0,
  first_frame_ms: 0,
  ready_ms: 0,
  draw_calls: 0,
  drawn: 0,
});

function stage(host: HTMLElement): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.width = CITY_BENCH_W;
  c.height = CITY_BENCH_H;
  c.style.width = `${CITY_BENCH_W}px`;
  c.style.height = `${CITY_BENCH_H}px`;
  host.replaceChildren(c);
  return c;
}

const tick = () => new Promise((ok) => setTimeout(ok, 0));

function summarise(out: CityBackendReport, times: number[], cpu: number[]): void {
  out.frames = times.length;
  out.p50_ms = round(percentile(times, 50), 2);
  out.p95_ms = round(percentile(times, 95), 2);
  out.cpu_ms = round(percentile(cpu, 50), 2);
}

async function bench3d(host: HTMLElement): Promise<CityBackendReport> {
  const out = empty();
  let scene: import("$lib/world/render/three3d").CityScene | null = null;
  try {
    const mod = await import("$lib/world/render/three3d");
    const canvas = stage(host);
    const t0 = performance.now();
    scene = await mod.createCityScene({ canvas, tier: 0, camera: "orbit", watchdog: false });
    scene.resize(CITY_BENCH_W, CITY_BENCH_H, 1);
    await scene.ready;
    // The city's own loop streams regions and applies the LOD; let it settle.
    await new Promise((ok) => setTimeout(ok, 1500));
    out.ready_ms = round(performance.now() - t0, 1);
    const r = scene.renderer;
    const gl = r.gl?.getContext();
    const px = new Uint8Array(4);
    const f0 = performance.now();
    r.frame(16, true);
    gl?.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
    out.first_frame_ms = round(performance.now() - f0, 2);
    const times: number[] = [];
    const cpu: number[] = [];
    for (let i = 0; i < CITY_BENCH_WARMUP + CITY_BENCH_FRAMES; i++) {
      await tick();
      const s = performance.now();
      const st = r.frame(16, true);
      gl?.readPixels(CITY_BENCH_W >> 1, CITY_BENCH_H >> 1, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
      if (i >= CITY_BENCH_WARMUP) {
        times.push(performance.now() - s);
        cpu.push(st.cpuMs);
      }
      out.draw_calls = st.drawCalls;
      out.drawn = st.instances;
    }
    summarise(out, times, cpu);
  } catch (e) {
    out.error = e instanceof Error ? e.message : String(e);
  } finally {
    scene?.destroy();
  }
  return out;
}

async function benchLite(host: HTMLElement): Promise<CityBackendReport & { backend?: string }> {
  const out: CityBackendReport & { backend?: string } = empty();
  let scene: import("$lib/world/render/citylite").LiteCityScene | null = null;
  try {
    const mod = await import("$lib/world/render/citylite");
    const canvas = stage(host);
    const t0 = performance.now();
    scene = await mod.createLiteCityScene({ canvas });
    scene.resize(CITY_BENCH_W, CITY_BENCH_H, 1);
    await scene.ready;
    // Remaining regions load in the background: wait until nothing changes.
    let last = -1;
    for (let i = 0; i < 100; i++) {
      await new Promise((ok) => setTimeout(ok, 50));
      const n = scene.drawNow().sprites;
      if (n === last && i > 4) break;
      last = n;
    }
    out.ready_ms = round(performance.now() - t0, 1);
    out.backend = scene.backend;
    const c = { x: scene.camera.x, y: scene.camera.y, zoom: scene.camera.zoom };
    for (let i = 0; i < 40; i++) scene.drawNow(); // bake every visible chunk
    out.first_frame_ms = round(scene.drawNow().wallMs, 2);
    const times: number[] = [];
    const cpu: number[] = [];
    for (let i = 0; i < CITY_BENCH_WARMUP + CITY_BENCH_FRAMES; i++) {
      const a = i * 0.03;
      scene.setView(c.x + Math.cos(a) * 1.5, c.y + Math.sin(a) * 1.5, c.zoom);
      scene.step(33);
      await tick();
      const st = scene.drawNow();
      if (i >= CITY_BENCH_WARMUP) {
        times.push(st.wallMs);
        cpu.push(st.cpuMs);
      }
      out.draw_calls = st.drawCalls;
      out.drawn = st.sprites;
    }
    summarise(out, times, cpu);
  } catch (e) {
    out.error = e instanceof Error ? e.message : String(e);
  } finally {
    scene?.destroy();
  }
  return out;
}

export async function runCityCiBench(host: HTMLElement, scenario: string): Promise<void> {
  const report: Record<string, unknown> = { scenario, route: "city", backend: "none" };
  const deadline = new Promise<"deadline">((ok) => setTimeout(() => ok("deadline"), CITY_REPORT_DEADLINE_MS));
  try {
    const work = (async () => {
      // A fresh profile has no city: found the example one (no LLM), fixed seed.
      const status = (await invoke("city3d_status")) as { city?: unknown } | null;
      if (!status?.city) await invoke("city3d_found_example", { cityId: "omni", seed: "omni" });

      const { calibrate3dDetailed } = await import("$lib/world/render/calibrate");
      const { cityBackendFromCalibration } = await import("$lib/world/render/tier");
      const cal = await calibrate3dDetailed(document.createElement("canvas"));
      report.calibration = {
        median_ms: round(cal.medianMs, 2),
        tier: cal.tier,
        backend: cal.backend,
        city_backend: cityBackendFromCalibration(cal),
      };
      report.calibrate_median_ms = Number.isFinite(cal.medianMs) ? round(cal.medianMs, 2) : 0;

      const three3d = await bench3d(host);
      const lite = await benchLite(host);
      report.city = { three3d, lite };
      report.backend = lite.backend ?? "none";
      report.first_frame_ms = lite.first_frame_ms;
      report.scene = {
        median_frame_ms: lite.p50_ms,
        p95_frame_ms: lite.p95_ms,
        cpu_ms: lite.cpu_ms,
        fps: lite.p50_ms > 0 ? round(1000 / lite.p50_ms, 1) : 0,
        frames: lite.frames,
        draw_calls: lite.draw_calls,
        sprites: lite.drawn,
      };
      if (three3d.error && lite.error) report.error = `3d: ${three3d.error}; lite: ${lite.error}`;
      else if (lite.error) report.error = `lite: ${lite.error}`;
    })();
    if ((await Promise.race([work, deadline])) === "deadline") report.error = "ERR_CITY_BENCH_DEADLINE";
  } catch (e) {
    report.error = e instanceof Error ? e.message : String(e);
  }
  await invoke("world_bench_report", { report, channel: new Channel<unknown>() });
}
