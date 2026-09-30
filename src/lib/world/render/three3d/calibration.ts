// The 3D calibration scene (M0 item 3): identical on every platform. A
// synthetic region with CAL3D_INSTANCES toon instances, one 1024² shadow and
// CAL3D_CHARACTERS figures, 960×540 at DPR 1, fixed camera. `calibrate.ts`
// drives it with the same passes and sync methods as the 2D scene.

import { CapsuleGeometry, Mesh, Vector3 } from 'three';
import { fallbackKit } from './kit';
import { PaletteBook } from './palette';
import { buildRegion } from './region';
import { createThreeRenderer } from './renderer';
import { SYNTH_PALETTES, SYNTH_STYLE, synthRegion } from './synth';

export const CAL3D_INSTANCES = 2000;
export const CAL3D_CHARACTERS = 8;
export const CAL3D_SHADOW = 1024;

export interface Calibration3DScene {
  gl: WebGLRenderingContext | WebGL2RenderingContext;
  webgl2: boolean;
  draw(): void;
  dispose(): void;
}

export async function buildCalibration3D(canvas: HTMLCanvasElement, width: number, height: number): Promise<Calibration3DScene> {
  canvas.width = width;
  canvas.height = height;
  const r = createThreeRenderer();
  const caps = await r.init(canvas, { tier: 1, dpr: 1 });
  r.resize(width, height, 1);
  const gl3 = r.gl!;
  gl3.shadowMap.enabled = true;
  r.sun.castShadow = true;
  r.sun.shadow.mapSize.set(CAL3D_SHADOW, CAL3D_SHADOW);

  const { mesh, blueprint } = synthRegion([0, 0], CAL3D_INSTANCES);
  const kit = fallbackKit(SYNTH_STYLE);
  const built = buildRegion(mesh, kit, new PaletteBook(SYNTH_PALETTES), r.materials, { outline: false, shadows: true });
  r.scene.add(built.group);

  // Characters: capsules on the street tiles (the rig itself is agent D's and
  // loads assets; a calibration must not depend on a download).
  const capsule = new CapsuleGeometry(0.28, 1.0, 4, 8);
  capsule.translate(0, 0.78, 0);
  const figures: Mesh[] = [];
  let placed = 0;
  for (let i = 0; i < blueprint.surface.length && placed < CAL3D_CHARACTERS; i += 97) {
    if (blueprint.surface[i] !== 'street') continue;
    const m = new Mesh(capsule, r.materials.plain);
    m.position.set((i % 64) * 2 + 1, 0, Math.floor(i / 64) * 2 + 1);
    m.castShadow = true;
    r.scene.add(m);
    figures.push(m);
    placed++;
  }

  const c = built.center;
  r.camera.position.set(c.x + 70, 80, c.z + 70);
  r.camera.lookAt(c);
  r.followShadow(new Vector3(c.x, 0, c.z));
  await r.compile().catch(() => 0);

  return {
    gl: gl3.getContext(),
    webgl2: caps.webgl2,
    draw() {
      r.frame(16, true);
    },
    dispose() {
      for (const f of figures) f.removeFromParent();
      capsule.dispose();
      built.dispose();
      kit.dispose();
      r.destroy();
    },
  };
}
