import { parseAtlas, type AtlasData } from '$lib/gfx/atlas';
import { TextAtlas, textUV } from '$lib/gfx/text';
import type { CanvasEngine } from '../engine.svelte';
import { isTerminalKind } from '../model/types';
import { agentStates, STATE_COLORS, type AgentState } from '../state/agents.svelte';
import { hexToRgba, withAlpha, type Rgba } from './color';
import { labelPriority, placeLabels } from './labels';
import type { FrameInfo, GlPass, NodeView } from './gl-layer';
import { QuadBatch, uploadTexture } from './quads';
import { SdfBatch } from './sdf-shapes';

const ATLAS_URL = '/world/omni/atlas.json';

interface OmniSheet {
  atlas: AtlasData;
  image: ImageBitmap;
  size: { w: number; h: number };
}

let sheetPromise: Promise<OmniSheet | null> | null = null;

function loadSheet(): Promise<OmniSheet | null> {
  if (sheetPromise) return sheetPromise;
  sheetPromise = (async () => {
    try {
      const res = await fetch(ATLAS_URL);
      const json = await res.json();
      const atlas = parseAtlas(json);
      const page = atlas.pages[0];
      const url = page.startsWith('/') ? page : `/world/omni/${page}`;
      const blob = await (await fetch(url)).blob();
      const image = await createImageBitmap(blob);
      return { atlas, image, size: { w: image.width, h: image.height } };
    } catch {
      return null;
    }
  })();
  return sheetPromise;
}

function animFor(state: AgentState): { anim: string; fps: number } {
  switch (state) {
    case 'working':
    case 'thinking':
      return { anim: 'omni/work', fps: 10 };
    case 'blocked':
      return { anim: 'omni/wave', fps: 10 };
    case 'waitingPeer':
    case 'compacting':
    case 'looping':
      return { anim: 'omni/sit', fps: 6 };
    case 'failed':
    case 'unknown':
      return { anim: 'omni/sleep', fps: 4 };
    default:
      return { anim: 'omni/idle', fps: 6 };
  }
}

function animates(state: AgentState): boolean {
  return state === 'working' || state === 'thinking' || state === 'blocked' || state === 'compacting' || state === 'looping';
}

export function createSpritePass(engine: CanvasEngine, colors: () => { ink: Rgba; surface: Rgba }): GlPass {
  const sdf = new SdfBatch();
  const sprites = new QuadBatch();
  const texts = new QuadBatch();
  const textAtlas = new TextAtlas(1024, 256);
  let gl: WebGL2RenderingContext | null = null;
  let sheet: OmniSheet | null = null;
  let sheetTex: WebGLTexture | null = null;
  let textTex: WebGLTexture | null = null;
  let textVersion = -1;
  let requested = false;

  const ensureSheet = (g: WebGL2RenderingContext) => {
    if (!requested) {
      requested = true;
      void loadSheet().then((s) => {
        sheet = s;
        engine.invalidate();
      });
    }
    if (sheet && !sheetTex) sheetTex = uploadTexture(g, sheet.image, null, true);
  };

  const label = (str: string, color: string) =>
    textAtlas.get(str, { size: 12, color, font: 'system-ui, sans-serif', outline: 'rgba(0,0,0,0.55)', maxWidth: 220 });

  const drawAgent = (v: NodeView, t: number, sx: number, sy: number, sw: number, sh: number, ink: Rgba) => {
    const node = engine.store.node(v.id);
    const state = node ? agentStates.stateOf(node) : 'unknown';
    const color = STATE_COLORS[state];
    const busy = state === 'working' || state === 'thinking';
    const unit = Math.max(0.35, Math.min(2.2, sw / 140));
    const cx = sx + sw / 2;
    const floor = sy + sh * 0.78;

    const deskW = 84 * unit;
    const deskH = 16 * unit;
    sdf.rect(cx - deskW / 2, floor - deskH, deskW, deskH, { radius: 3 * unit, fill: [0.55, 0.39, 0.25, 1], strokeWidth: 1, stroke: [0.25, 0.17, 0.1, 1] });
    sdf.rect(cx - deskW / 2 + 4 * unit, floor, 5 * unit, 14 * unit, { fill: [0.4, 0.28, 0.18, 1] });
    sdf.rect(cx + deskW / 2 - 9 * unit, floor, 5 * unit, 14 * unit, { fill: [0.4, 0.28, 0.18, 1] });
    const monW = 34 * unit;
    const monH = 22 * unit;
    const monX = cx + 10 * unit;
    const monY = floor - deskH - monH - 2 * unit;
    sdf.rect(monX + monW / 2 - 2 * unit, monY + monH, 4 * unit, 3 * unit, { fill: [0.2, 0.2, 0.22, 1] });
    sdf.rect(monX, monY, monW, monH, {
      radius: 2 * unit,
      fill: busy ? [0.62, 0.86, 0.98, 1] : [0.12, 0.13, 0.16, 1],
      strokeWidth: 1.5 * unit,
      stroke: [0.18, 0.18, 0.2, 1],
    });
    if (busy) {
      const scroll = (t / 600) % 1;
      for (let i = 0; i < 3; i++) {
        const ly = monY + monH - 5 * unit - ((i + scroll) * 6 * unit);
        if (ly < monY + 3 * unit) continue;
        const lw = (10 + ((i * 7 + Math.floor(t / 600)) % 3) * 6) * unit;
        sdf.rect(monX + 3 * unit, ly, Math.min(lw, monW - 6 * unit), 1.6 * unit, { fill: [0.13, 0.35, 0.5, 0.9] });
      }
      if (Math.floor(t / 530) % 2 === 0) {
        sdf.rect(monX + monW - 8 * unit, monY + monH - 6 * unit, 3 * unit, 3.5 * unit, { fill: [0.13, 0.35, 0.5, 1] });
      }
    }

    const pulse = busy ? 0.55 + 0.45 * Math.sin((t / 1200) * Math.PI * 2) : 1;
    const charX = busy ? cx - 14 * unit : cx - deskW / 2 - 10 * unit;
    sdf.push({
      cx: charX,
      cy: floor + 2 * unit,
      hw: 20 * unit,
      hh: 7 * unit,
      radius: 7 * unit,
      fill: hexToRgba(color, 0.25 * pulse),
      strokeWidth: 2,
      stroke: hexToRgba(color, 0.9 * pulse),
    });

    let headY = floor - 48 * unit;
    if (sheet && sheetTex) {
      const { anim, fps } = animFor(state);
      const clip = sheet.atlas.anims.get(anim) ?? sheet.atlas.anims.get('omni/idle');
      const frames = clip?.dirs.S ?? [];
      const fid = frames.length ? frames[Math.floor((t / 1000) * fps) % frames.length] : undefined;
      const f = fid ? sheet.atlas.frames.get(fid) : undefined;
      if (f) {
        const scale = (46 * unit) / f.h;
        const w = f.w * scale;
        const h = f.h * scale;
        const px = charX - f.pivotX * scale;
        const py = floor + 2 * unit - f.pivotY * scale;
        headY = py;
        sprites.quad({
          page: 0,
          sx: px,
          sy: py,
          w,
          h,
          u0: f.x / sheet.size.w,
          v0: f.y / sheet.size.h,
          u1: (f.x + f.w) / sheet.size.w,
          v1: (f.y + f.h) / sheet.size.h,
          alpha: state === 'failed' || state === 'unknown' ? 0.5 : 1,
        });
      }
    } else {
      sdf.circle(charX, floor - 20 * unit, 14 * unit, { fill: [0.96, 0.83, 0.37, 1], strokeWidth: 1.5, stroke: withAlpha(ink, 0.5) });
    }

    const gy = headY - 10 * unit;
    if (state === 'blocked') {
      if (Math.floor(t / 200) % 2 === 0) {
        sdf.rect(charX - 3 * unit, gy - 14 * unit, 6 * unit, 10 * unit, { radius: 2 * unit, fill: [0.85, 0.42, 0.38, 1] });
        sdf.circle(charX, gy + 1 * unit, 3 * unit, { fill: [0.85, 0.42, 0.38, 1] });
      }
    } else if (state === 'compacting') {
      const k = 0.7 + 0.3 * Math.sin((t / 500) * Math.PI);
      sdf.rect(charX - 6 * unit * k, gy - 10 * unit * k, 12 * unit * k, 10 * unit * k, { radius: 1.5 * unit, fill: hexToRgba(0xb48ee0, 1) });
    } else if (state === 'looping') {
      const step = Math.floor(t / 125) % 4;
      for (let i = 0; i < 4; i++) {
        const a = (i / 4) * Math.PI * 2;
        sdf.circle(charX + Math.cos(a) * 7 * unit, gy - 6 * unit + Math.sin(a) * 7 * unit, 2 * unit, {
          fill: hexToRgba(0xe8a33c, i === step ? 1 : 0.35),
        });
      }
    } else if (state === 'success') {
      sdf.push({ cx: charX, cy: gy - 6 * unit, hw: 5 * unit, hh: 5 * unit, radius: 1, rotation: Math.PI / 4, fill: hexToRgba(0xf4d35e, 1) });
    }

    if (animates(state)) engine.wantFrames(state === 'blocked' ? 10 : 8);
  };

  return {
    id: 'canvas.sprites',
    order: 50,
    draw(g: WebGL2RenderingContext, frame: FrameInfo) {
      if (gl !== g) {
        gl = g;
        sheetTex = null;
        textTex = null;
        textVersion = -1;
      }
      const cam = frame.camera;
      const z = cam.zoom;
      const t = frame.time;
      const { ink } = colors();
      const far = frame.visibleNodes.filter((v) => v.lod === 'far');
      if (!far.length) return;
      ensureSheet(g);
      sdf.begin();
      sprites.begin();
      texts.begin();
      const wanted: Array<{ t: ReturnType<typeof label>; x: number; y: number; w: number; h: number; priority: number }> = [];
      for (const v of far) {
        const sx = (v.x - cam.x) * z;
        const sy = (v.y - cam.y) * z;
        const sw = v.w * z;
        const sh = v.h * z;
        if (isTerminalKind(v.kind)) drawAgent(v, t, sx, sy, sw, sh, ink);
        const title = v.title ?? '';
        if (title) {
          const t = label(title, '#ffffff');
          wanted.push({
            t,
            x: sx + sw / 2 - t.w / 2,
            y: isTerminalKind(v.kind) ? sy + sh * 0.78 + 18 : sy + sh / 2 - t.h / 2,
            w: t.w,
            h: t.h,
            priority: labelPriority(v.kind, v.w, v.h, isTerminalKind(v.kind), engine.selection.has(v.id)),
          });
        }
      }
      const labels = placeLabels(wanted, 3);
      sdf.flush(g, frame.viewport.w, frame.viewport.h);
      if (sheetTex) sprites.flush(g, sheetTex, frame.viewport.w, frame.viewport.h);
      if (labels.length && textAtlas.page) {
        if (textVersion !== textAtlas.version || !textTex) {
          textTex = uploadTexture(g, textAtlas.page as HTMLCanvasElement | OffscreenCanvas, textTex);
          textVersion = textAtlas.version;
        }
        for (const l of labels) {
          const r = textAtlas.rect(l.t.id);
          if (!r) continue;
          const uv = textUV(r, textAtlas.size);
          texts.quad({ page: 0, sx: Math.round(l.x), sy: Math.round(l.y), w: r.w, h: r.h, ...uv });
        }
        if (textTex) texts.flush(g, textTex, frame.viewport.w, frame.viewport.h);
      }
    },
    dispose() {
      sdf.dispose();
      sprites.dispose();
      texts.dispose();
      if (gl && !gl.isContextLost()) {
        if (sheetTex) gl.deleteTexture(sheetTex);
        if (textTex) gl.deleteTexture(textTex);
      }
      sheetTex = null;
      textTex = null;
      gl = null;
      textAtlas.release();
    },
  };
}
