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
