// Dev harness for the C01 acceptance (session 06): the real renderer draws a
// V2 actor and the object it uses as separate sprites on one tile, and hands
// back the pixels of the cell so they can be compared with the workshop's
// reference composites. Loaded only by the test driver in a dev build
// (`await import('/src/lib/world/c01-harness.ts')`); nothing imports it.

import { mergeAtlases } from './assets';
import { actorFrame, objectLayers, parseAnimationSet, screenDir, type V2State } from './c01';
import { createRenderer } from './render';
import { parseAtlas } from './render/atlas';
import { worldToScreen } from './render/camera';
import type { BackendName, SpriteInst } from './render/types';

async function json(url: string): Promise<unknown> {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${url}: ${r.status}`);
  return r.json();
}
async function bitmap(url: string): Promise<ImageBitmap> {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${url}: ${r.status}`);
  return createImageBitmap(await r.blob());
}

export interface ProbeResult {
  backend: string;
  /** Screen direction → base64 RGBA of the 128x160 cell (unpremultiplied). */
  cells: Record<string, string>;
  /** Sprite draw order the renderer used, per direction. */
  order: Record<string, string[]>;
}

export async function runC01Probe(opts: { pack: string; objects: string; backend: BackendName; state: V2State; object: string; recipe?: string }): Promise<ProbeResult> {
  const recipe = opts.recipe ?? 'resident-01';
  const set = parseAnimationSet(await json(`${opts.pack}/animation-set.json`));
  const parts = [await json(`${opts.pack}/atlas.json`), await json(`${opts.objects}/objects.atlas.json`)] as Array<{ pages: string[] }>;
  const merged = mergeAtlases(parts as never);
  const pages = await Promise.all([...parts[0].pages.map((p) => bitmap(`${opts.pack}/${p}`)), ...parts[1].pages.map((p) => bitmap(`${opts.objects}/${p}`))]);
  const atlas = parseAtlas(merged.json);

  const canvas = document.createElement('canvas');
  canvas.width = 256;
  canvas.height = 256;
  canvas.style.cssText = 'position:fixed;left:-400px;top:0;width:256px;height:256px';
  document.body.appendChild(canvas);
  const r = createRenderer();
  try {
    await r.init(canvas, { tier: 3, backend: opts.backend, dpr: 1 });
    r.resize(256, 256, 1);
    r.loadAtlas(merged.json, pages);
    const camera = { x: 0.5, y: 0.5, zoom: 1, width: 256, height: 256 };
    const cells: Record<string, string> = {};
    const order: Record<string, string[]> = {};
    // Every screen direction, through the world direction that maps to it.
    for (let world = 0; world < 8; world++) {
      const dir = screenDir(set, world);
      const actor = actorFrame(atlas, set, recipe, opts.state, world, 0);
      const layers = objectLayers(atlas, opts.object, opts.state, dir);
      // Deliberately in the "wrong" order: the depth sort must fix it.
      const sprites: SpriteInst[] = [];
      if (layers.over) sprites.push({ frame: layers.over, x: 0.5, y: 0.5, z: 0 });
      sprites.push({ frame: actor, x: 0.5, y: 0.5, z: 0 });
      if (layers.under) sprites.push({ frame: layers.under, x: 0.5, y: 0.5, z: 0 });
      r.frame({ camera, chunksVisible: [], sprites, texts: [] }, 16);
      const p = worldToScreen(camera, 0.5, 0.5);
      const x0 = Math.round(p.sx) - set.pivot[0];
      const y0 = Math.round(p.sy) - set.pivot[1];
      const [w, h] = set.cell;
      let px: Uint8Array;
      const gl = (canvas.getContext('webgl2') ?? canvas.getContext('webgl')) as WebGLRenderingContext | null;
      if (gl && opts.backend !== 'canvas2d') {
        const buf = new Uint8Array(w * h * 4);
        gl.readPixels(x0, canvas.height - y0 - h, w, h, gl.RGBA, gl.UNSIGNED_BYTE, buf);
        px = new Uint8Array(w * h * 4);
        for (let y = 0; y < h; y++) px.set(buf.subarray((h - 1 - y) * w * 4, (h - y) * w * 4), y * w * 4);
      } else {
        px = new Uint8Array(canvas.getContext('2d')!.getImageData(x0, y0, w, h).data.buffer);
      }
      let s = '';
      for (let i = 0; i < px.length; i += 0x8000) s += String.fromCharCode(...px.subarray(i, i + 0x8000));
      cells[dir] = btoa(s);
      const { sortByDepth } = await import('./render/batcher');
      order[dir] = sortByDepth(sprites, atlas).map((i) => sprites[i].frame.split('/').slice(-2).join('/'));
    }
    return { backend: opts.backend, cells, order };
  } finally {
    r.destroy();
    canvas.remove();
  }
}
