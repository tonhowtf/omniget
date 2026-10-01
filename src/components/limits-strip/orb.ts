/**
 * Thinking orbs (libraries.dev `thinking-orbs`) without React: a Svelte
 * action over the package's framework-free `./engine` export.
 *
 * Only the documented knobs are used (`state`, `size` 20 | 64, `theme`,
 * `paused`); the draw call is the same one the package's own component makes
 * (`resolvePreset` → `MODE_FRAMES[mode]` → `paintFrame`).
 *
 * Every orb on the page shares one requestAnimationFrame loop, which only
 * runs while at least one orb is visible, unpaused, the document is not
 * hidden and the user has not asked for reduced motion. With no orb mounted
 * the page owns no frame callback at all.
 */
import { MODE_FRAMES, paintFrame, resolvePreset, type OrbSize, type OrbState } from "thinking-orbs/engine";

type OrbOptions = {
  state: OrbState;
  size?: OrbSize;
  theme: "dark" | "light";
  paused?: boolean;
};

type Live = {
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  opts: Required<OrbOptions>;
  dpr: number;
  visible: boolean;
};

const live = new Set<Live>();
let raf = 0;
let last = 0;
/** 20 px orbs read as smooth at 30 fps; half the frames, half the cost. */
const FRAME_MS = 1000 / 30;

const reducedQuery = typeof matchMedia === "function" ? matchMedia("(prefers-reduced-motion: reduce)") : null;
const reduced = () => reducedQuery?.matches ?? false;

function draw(o: Live, tSec: number) {
  const { state, size, theme } = o.opts;
  const { mode, speed, opts } = resolvePreset(state, size);
  o.ctx.setTransform(o.dpr, 0, 0, o.dpr, 0, 0);
  o.ctx.clearRect(0, 0, size, size);
  paintFrame(o.ctx, MODE_FRAMES[mode](size, tSec * speed, opts), theme === "dark");
}

function runnable(o: Live) {
  return o.visible && !o.opts.paused;
}

function tick(stamp: number) {
  raf = 0;
  const due = stamp - last >= FRAME_MS - 2;
  if (due) last = stamp;
  const t = performance.now() / 1000;
  let any = false;
  for (const o of live) {
    if (!runnable(o)) continue;
    if (due) draw(o, t);
    any = true;
  }
  if (any) raf = requestAnimationFrame(tick);
}

function kick() {
  if (raf || reduced() || document.visibilityState === "hidden") return;
  for (const o of live) {
    if (runnable(o)) {
      raf = requestAnimationFrame(tick);
      return;
    }
  }
}

function halt() {
  if (raf) cancelAnimationFrame(raf);
  raf = 0;
}

if (typeof document !== "undefined") {
  document.addEventListener("visibilitychange", () => (document.visibilityState === "hidden" ? halt() : kick()));
  reducedQuery?.addEventListener("change", () => {
    if (reduced()) {
      halt();
      for (const o of live) draw(o, 0.6);
    } else kick();
  });
}

function setup(o: Live) {
  const size = o.opts.size;
  o.dpr = Math.min(2, devicePixelRatio || 1);
  o.canvas.width = Math.round(size * o.dpr);
  o.canvas.height = Math.round(size * o.dpr);
  o.canvas.style.width = `${size}px`;
  o.canvas.style.height = `${size}px`;
  // One still frame right away: the reduced-motion pose, or the first frame.
  draw(o, reduced() ? 0.6 : performance.now() / 1000);
}

export function orb(canvas: HTMLCanvasElement, options: OrbOptions) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return {};
  const o: Live = {
    canvas,
    ctx,
    opts: { size: 20, paused: false, ...options },
    dpr: 1,
    visible: true,
  };
  setup(o);
  live.add(o);

  const io =
    typeof IntersectionObserver === "function"
      ? new IntersectionObserver(([entry]) => {
          o.visible = entry.isIntersecting;
          kick();
        })
      : null;
  io?.observe(canvas);
  kick();

  return {
    update(next: OrbOptions) {
      const sizeChanged = (next.size ?? 20) !== o.opts.size;
      o.opts = { size: 20, paused: false, ...next };
      if (sizeChanged) setup(o);
      else draw(o, reduced() ? 0.6 : performance.now() / 1000);
      kick();
    },
    destroy() {
      io?.disconnect();
      live.delete(o);
      if (![...live].some(runnable)) halt();
    },
  };
}
