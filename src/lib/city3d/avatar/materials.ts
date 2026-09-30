// Toon look of the residents: a 3-step MeshToonMaterial with vertex colors
// (palette colors baked per vertex) and an inverted-hull outline.
import * as THREE from 'three';

let gradient: THREE.DataTexture | null = null;

export function toonGradient(): THREE.DataTexture {
  if (gradient) return gradient;
  const data = new Uint8Array([90, 90, 90, 255, 180, 180, 180, 255, 255, 255, 255, 255]);
  gradient = new THREE.DataTexture(data, 3, 1, THREE.RGBAFormat);
  gradient.minFilter = THREE.NearestFilter;
  gradient.magFilter = THREE.NearestFilter;
  gradient.generateMipmaps = false;
  gradient.needsUpdate = true;
  return gradient;
}

export function toonMaterial(opts: { doubleSide?: boolean } = {}): THREE.MeshToonMaterial {
  const m = new THREE.MeshToonMaterial({
    vertexColors: true,
    gradientMap: toonGradient(),
    side: opts.doubleSide ? THREE.DoubleSide : THREE.FrontSide,
  });
  m.name = opts.doubleSide ? 'resident-cloth' : 'resident-skin';
  return m;
}

/** Back-face hull pushed along the normal, in bind space before skinning. */
export function outlineMaterial(width = 0.0035, color = 0x2a1d18): THREE.MeshBasicMaterial {
  const m = new THREE.MeshBasicMaterial({ color, side: THREE.BackSide });
  m.name = 'resident-outline';
  m.onBeforeCompile = (shader) => {
    shader.uniforms.outlineWidth = { value: width };
    shader.vertexShader = shader.vertexShader
      .replace('#include <common>', '#include <common>\nuniform float outlineWidth;')
      .replace(
        '#include <begin_vertex>',
        '#include <begin_vertex>\n#ifdef USE_SKINNING\ntransformed += normalize( objectNormal ) * outlineWidth;\n#endif',
      );
  };
  m.customProgramCacheKey = () => 'resident-outline';
  return m;
}

export function srgbToLinear(c: [number, number, number]): [number, number, number] {
  const f = (v: number) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
  return [f(c[0]), f(c[1]), f(c[2])];
}
