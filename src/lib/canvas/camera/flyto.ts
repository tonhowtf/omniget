// Camera flights (⌘1–⌘9, zoom to node, frame selection, "next needs you"):
// van Wijk & Nuij "Smooth and efficient zooming and panning" (2003) — zoom out,
// glide, zoom in — with a duration proportional to the path, 250–700 ms.

import type { CameraState } from '../model/types';

const RHO = 1.4;
const MIN_FLIGHT_MS = 250;
const MAX_FLIGHT_MS = 700;

export interface Flight {
  /** Camera at progress t (0..1, already eased by the caller if wanted). */
  at(t: number): CameraState;
  durationMs: number;
}

/**
 * Builds the flight between two cameras for a viewport of vw × vh CSS px.
 * Works on the centre and the visible width in world units, as in the paper.
 */
export function flyTo(from: CameraState, to: CameraState, vw: number, vh: number): Flight {
  const c0x = from.x + vw / (2 * from.zoom);
  const c0y = from.y + vh / (2 * from.zoom);
  const c1x = to.x + vw / (2 * to.zoom);
  const c1y = to.y + vh / (2 * to.zoom);
  const w0 = vw / from.zoom;
  const w1 = vw / to.zoom;
  const dx = c1x - c0x;
  const dy = c1y - c0y;
  const u1 = Math.hypot(dx, dy);
  const rho2 = RHO * RHO;

  const toCamera = (cx: number, cy: number, w: number): CameraState => {
    const zoom = vw / w;
    return { x: cx - vw / (2 * zoom), y: cy - vh / (2 * zoom), zoom };
  };

  let S: number;
  let at: (t: number) => CameraState;
  if (u1 < 1e-6) {
    // Pure zoom.
    const k = w1 < w0 ? -1 : 1;
    S = Math.abs(Math.log(w1 / w0)) / RHO;
    at = (t) => {
      const w = w0 * Math.exp(k * RHO * t * S);
      return toCamera(c0x, c0y, w);
    };
  } else {
    const b = (w: number, sign: number) =>
      (w1 * w1 - w0 * w0 + sign * rho2 * rho2 * u1 * u1) / (2 * (sign > 0 ? w0 : w1) * rho2 * u1);
    const b0 = b(w0, 1);
    const b1 = b(w1, -1);
    const r0 = Math.log(Math.sqrt(b0 * b0 + 1) - b0);
    const r1 = Math.log(Math.sqrt(b1 * b1 + 1) - b1);
    S = (r1 - r0) / RHO;
    const cosh = Math.cosh;
    const sinh = Math.sinh;
    const tanh = Math.tanh;
    at = (t) => {
      const s = t * S;
      const coshr0 = cosh(r0);
      const u = (w0 / rho2) * (coshr0 * tanh(RHO * s + r0) - sinh(r0));
      const w = (w0 * coshr0) / cosh(RHO * s + r0);
      const f = u1 > 0 ? u / u1 : 0;
      return toCamera(c0x + dx * f, c0y + dy * f, w);
    };
  }
  if (!Number.isFinite(S) || S <= 0) {
    return { at: (t) => (t >= 1 ? to : from), durationMs: MIN_FLIGHT_MS };
  }
  const durationMs = Math.min(MAX_FLIGHT_MS, Math.max(MIN_FLIGHT_MS, S * 260));
  return {
    durationMs,
    at: (t) => (t <= 0 ? from : t >= 1 ? to : at(t)),
  };
}

export function easeInOutCubic(t: number): number {
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}
