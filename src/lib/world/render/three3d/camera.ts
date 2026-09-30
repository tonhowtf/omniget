// Camera rig of the 3D city, on camera-controls (MIT, the library threlte's
// CameraControls wraps). Two modes behind one setting, so the owner can pick
// by looking (M0 item 5):
// - `orbit`: free orbit. Left drag moves over the ground, right drag orbits,
//   wheel zooms to the cursor.
// - `snap`: fixed tilt; Q/E (or right click) turn the view 90° at a time.
// WASD / arrows move in both; Q/E turn in both.

import CameraControls from 'camera-controls';
import {
  Box3,
  Matrix4,
  Quaternion,
  Raycaster,
  Sphere,
  Spherical,
  Vector2,
  Vector3,
  Vector4,
  type PerspectiveCamera,
} from 'three';
import type { CameraMode3D } from '../types';

let installed = false;
function install(): void {
  if (installed) return;
  CameraControls.install({ THREE: { Box3, Matrix4, Quaternion, Raycaster, Sphere, Spherical, Vector2, Vector3, Vector4 } });
  installed = true;
}

const SNAP_POLAR = (52 * Math.PI) / 180;
const QUARTER = Math.PI / 2;
const MOVE_SPEED = 0.9; // fraction of the camera distance per second

export interface CameraRig {
  readonly controls: CameraControls;
  readonly mode: CameraMode3D;
  setMode(mode: CameraMode3D): void;
  /** Advances transitions and keyboard motion; true when the view changed. */
  update(dtSec: number): boolean;
  lookAt(x: number, z: number, distance?: number, smooth?: boolean): void;
  target(out?: Vector3): Vector3;
  turn(quarters: number): void;
  /** Moves the DOM listeners to another canvas (context swap on macOS). */
  connect(el: HTMLElement): void;
  setBounds(min: Vector3, max: Vector3): void;
  dispose(): void;
}

export function createCameraRig(camera: PerspectiveCamera, el: HTMLElement, mode: CameraMode3D): CameraRig {
  install();
  const controls = new CameraControls(camera, el);
  controls.minDistance = 6;
  controls.maxDistance = 360;
  controls.smoothTime = 0.18;
  controls.draggingSmoothTime = 0.08;
  controls.dollyToCursor = true;
  controls.minPolarAngle = 0.12;
  controls.maxPolarAngle = 1.32;
  let current: CameraMode3D = mode;
  const keys = new Set<string>();
  let host: HTMLElement = el;

  const onKeyDown = (e: KeyboardEvent) => {
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
    const k = e.key.toLowerCase();
    if (k === 'q' || k === 'e') {
      turn(k === 'q' ? -1 : 1);
      e.preventDefault();
      return;
    }
    if (['w', 'a', 's', 'd', 'arrowup', 'arrowdown', 'arrowleft', 'arrowright'].includes(k)) {
      keys.add(k);
      e.preventDefault();
    }
  };
  const onKeyUp = (e: KeyboardEvent) => keys.delete(e.key.toLowerCase());
  const onBlur = () => keys.clear();
  const onContext = (e: MouseEvent) => {
    if (current !== 'snap') return;
    e.preventDefault();
    turn(e.shiftKey ? -1 : 1);
  };

  function listen(target: HTMLElement): void {
    target.addEventListener('keydown', onKeyDown);
    target.addEventListener('keyup', onKeyUp);
    target.addEventListener('blur', onBlur);
    target.addEventListener('contextmenu', onContext);
  }
  function unlisten(target: HTMLElement): void {
    target.removeEventListener('keydown', onKeyDown);
    target.removeEventListener('keyup', onKeyUp);
    target.removeEventListener('blur', onBlur);
    target.removeEventListener('contextmenu', onContext);
  }
  listen(host);

  function turn(quarters: number): void {
    if (current === 'snap') {
      const snapped = Math.round(controls.azimuthAngle / QUARTER) * QUARTER;
      void controls.rotateTo(snapped + quarters * QUARTER, SNAP_POLAR, true);
    } else {
      void controls.rotate(quarters * QUARTER, 0, true);
    }
  }

  function apply(m: CameraMode3D): void {
    current = m;
    const A = CameraControls.ACTION;
    if (m === 'orbit') {
      controls.mouseButtons.left = A.TRUCK;
      controls.mouseButtons.right = A.ROTATE;
      controls.mouseButtons.wheel = A.DOLLY;
      controls.touches.one = A.TOUCH_TRUCK;
      controls.touches.two = A.TOUCH_DOLLY_ROTATE;
      controls.minPolarAngle = 0.12;
      controls.maxPolarAngle = 1.32;
    } else {
      controls.mouseButtons.left = A.TRUCK;
      controls.mouseButtons.right = A.NONE;
      controls.mouseButtons.wheel = A.DOLLY;
      controls.touches.one = A.TOUCH_TRUCK;
      controls.touches.two = A.TOUCH_DOLLY;
      controls.minPolarAngle = SNAP_POLAR;
      controls.maxPolarAngle = SNAP_POLAR;
      const snapped = Math.round(controls.azimuthAngle / QUARTER) * QUARTER;
      void controls.rotateTo(snapped, SNAP_POLAR, true);
    }
  }
  apply(mode);

  const tmp = new Vector3();
  return {
    controls,
    get mode() {
      return current;
    },
    setMode(m: CameraMode3D) {
      if (m !== current) apply(m);
    },
    update(dtSec: number): boolean {
      if (keys.size) {
        const step = controls.distance * MOVE_SPEED * dtSec;
        let x = 0;
        let f = 0;
        if (keys.has('a') || keys.has('arrowleft')) x -= step;
        if (keys.has('d') || keys.has('arrowright')) x += step;
        if (keys.has('w') || keys.has('arrowup')) f += step;
        if (keys.has('s') || keys.has('arrowdown')) f -= step;
        if (x) void controls.truck(x, 0, false);
        if (f) void controls.forward(f, false);
      }
      return controls.update(dtSec) || keys.size > 0;
    },
    lookAt(x: number, z: number, distance = 90, smooth = false) {
      const az = current === 'snap' ? Math.round(controls.azimuthAngle / QUARTER) * QUARTER : controls.azimuthAngle;
      const polar = current === 'snap' ? SNAP_POLAR : Math.min(1.1, Math.max(0.6, controls.polarAngle));
      const s = new Spherical(distance, polar, az);
      tmp.setFromSpherical(s);
      void controls.setLookAt(x + tmp.x, tmp.y, z + tmp.z, x, 0, z, smooth);
    },
    target(out = new Vector3()) {
      return controls.getTarget(out);
    },
    turn,
    connect(next: HTMLElement) {
      unlisten(host);
      controls.disconnect();
      host = next;
      controls.connect(next);
      listen(next);
    },
    setBounds(min: Vector3, max: Vector3) {
      controls.setBoundary(new Box3(min, max));
    },
    dispose() {
      unlisten(host);
      controls.dispose();
    },
  };
}
