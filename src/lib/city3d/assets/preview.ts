// Small turntable preview for the import panel: one WebGL renderer on a
// canvas, the asset on a 2 m tile grid, slow auto-rotation.

import {
  AmbientLight,
  Box3,
  DirectionalLight,
  GridHelper,
  PerspectiveCamera,
  Scene,
  Vector3,
  WebGLRenderer,
  type Object3D,
} from "three";
import { disposeObject } from "./loader";

export interface Preview {
  show(object: Object3D | null): void;
  resize(): void;
  dispose(): void;
}

export function createPreview(canvas: HTMLCanvasElement): Preview {
  const renderer = new WebGLRenderer({ canvas, antialias: true, alpha: true });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  const scene = new Scene();
  const camera = new PerspectiveCamera(35, 1, 0.05, 500);
  scene.add(new AmbientLight(0xffffff, 1.4));
  const sun = new DirectionalLight(0xffffff, 2.2);
  sun.position.set(4, 8, 5);
  scene.add(sun);
  let grid = new GridHelper(8, 4, 0x888888, 0x555555);
  scene.add(grid);
  let current: Object3D | null = null;
  let angle = 0.7;
  let radius = 4;
  let target = new Vector3(0, 0.8, 0);
  let raf = 0;
  let last = performance.now();

  function frame(now: number) {
    const dt = Math.min(0.05, (now - last) / 1000);
    last = now;
    angle += dt * 0.35;
    camera.position.set(target.x + Math.sin(angle) * radius, target.y + radius * 0.45, target.z + Math.cos(angle) * radius);
    camera.lookAt(target);
    renderer.render(scene, camera);
    raf = requestAnimationFrame(frame);
  }

  function resize() {
    const w = canvas.clientWidth || 320;
    const h = canvas.clientHeight || 240;
    renderer.setSize(w, h, false);
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
  }

  function show(object: Object3D | null) {
    if (current) {
      scene.remove(current);
      disposeObject(current);
    }
    current = object;
    if (!object) return;
    scene.add(object);
    const box = new Box3().setFromObject(object);
    const size = box.getSize(new Vector3());
    const center = box.getCenter(new Vector3());
    const extent = Math.max(size.x, size.y, size.z, 0.5);
    target = new Vector3(center.x, center.y, center.z);
    radius = extent * 1.9;
    // Grid of 2 m tiles sized to the asset.
    scene.remove(grid);
    grid.geometry.dispose();
    const tiles = Math.max(2, Math.ceil(Math.max(size.x, size.z) / 2) + 2);
    grid = new GridHelper(tiles * 2, tiles, 0x888888, 0x555555);
    scene.add(grid);
  }

  resize();
  raf = requestAnimationFrame(frame);
  return {
    show,
    resize,
    dispose() {
      cancelAnimationFrame(raf);
      show(null);
      grid.geometry.dispose();
      renderer.dispose();
    },
  };
}
