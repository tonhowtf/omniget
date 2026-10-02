import { MAX_ZOOM, MIN_ZOOM, type CameraState, type Rect } from '../model/types';

const RUBBER = 0.1;

export function clampZoom(z: number, rubber = false): number {
  const lo = rubber ? MIN_ZOOM * (1 - RUBBER) : MIN_ZOOM;
  const hi = rubber ? MAX_ZOOM * (1 + RUBBER) : MAX_ZOOM;
  return Math.min(hi, Math.max(lo, z));
}

export class Camera implements CameraState {
  x = 0;
  y = 0;
  zoom = 1;
  vw = 1;
  vh = 1;
  rev = 0;

  set(c: Partial<CameraState>): void {
    if (c.x !== undefined && Number.isFinite(c.x)) this.x = c.x;
    if (c.y !== undefined && Number.isFinite(c.y)) this.y = c.y;
    if (c.zoom !== undefined && Number.isFinite(c.zoom)) this.zoom = clampZoom(c.zoom, true);
    this.rev++;
  }

  state(): CameraState {
    return { x: this.x, y: this.y, zoom: this.zoom };
  }

  resize(w: number, h: number): void {
    if (w === this.vw && h === this.vh) return;
    const cx = this.x + this.vw / (2 * this.zoom);
    const cy = this.y + this.vh / (2 * this.zoom);
    this.vw = Math.max(1, w);
    this.vh = Math.max(1, h);
    this.x = cx - this.vw / (2 * this.zoom);
    this.y = cy - this.vh / (2 * this.zoom);
    this.rev++;
  }

  worldToScreen(wx: number, wy: number): { x: number; y: number } {
    return { x: (wx - this.x) * this.zoom, y: (wy - this.y) * this.zoom };
  }

  screenToWorld(sx: number, sy: number): { x: number; y: number } {
    return { x: sx / this.zoom + this.x, y: sy / this.zoom + this.y };
  }

  rectToScreen(r: Rect): Rect {
    return { x: (r.x - this.x) * this.zoom, y: (r.y - this.y) * this.zoom, w: r.w * this.zoom, h: r.h * this.zoom };
  }

  viewRect(margin = 0): Rect {
    const w = this.vw / this.zoom;
    const h = this.vh / this.zoom;
    return { x: this.x - w * margin, y: this.y - h * margin, w: w * (1 + margin * 2), h: h * (1 + margin * 2) };
  }

  get focusX(): number {
    const inset = Math.min(this.insetLeft, this.vw * 0.5);
    const right = Math.min(this.insetRight, Math.max(0, this.vw * 0.75 - inset));
    return inset + Math.max(1, this.vw - inset - right) / 2;
  }

  center(): { x: number; y: number } {
    return { x: this.x + this.focusX / this.zoom, y: this.y + this.vh / (2 * this.zoom) };
  }

  panBy(dx: number, dy: number): void {
    this.x -= dx / this.zoom;
    this.y -= dy / this.zoom;
    this.rev++;
  }

  zoomTo(zoom: number, sx: number, sy: number, rubber = false): void {
    const z = clampZoom(zoom, rubber);
    const wx = sx / this.zoom + this.x;
    const wy = sy / this.zoom + this.y;
    this.zoom = z;
    this.x = wx - sx / z;
    this.y = wy - sy / z;
    this.rev++;
  }

  zoomBy(factor: number, sx: number, sy: number, rubber = false): void {
    this.zoomTo(this.zoom * factor, sx, sy, rubber);
  }

  insetLeft = 0;
  insetRight = 0;
  occluders: Rect[] = [];

  safeRect(): Rect {
    const inset = Math.min(this.insetLeft, this.vw * 0.5);
    const right = Math.min(this.insetRight, Math.max(0, this.vw * 0.75 - inset));
    let r: Rect = { x: inset, y: 0, w: Math.max(1, this.vw - inset - right), h: this.vh };
    const minW = Math.min(240, r.w * 0.5);
    const minH = Math.min(160, r.h * 0.5);
    for (const o of this.occluders) {
      const x0 = Math.max(r.x, o.x);
      const y0 = Math.max(r.y, o.y);
      const x1 = Math.min(r.x + r.w, o.x + o.w);
      const y1 = Math.min(r.y + r.h, o.y + o.h);
      if (x1 <= x0 || y1 <= y0) continue;
      const cuts: Rect[] = [
        { x: r.x, y: y1, w: r.w, h: r.y + r.h - y1 },
        { x: r.x, y: r.y, w: r.w, h: y0 - r.y },
        { x: x1, y: r.y, w: r.x + r.w - x1, h: r.h },
        { x: r.x, y: r.y, w: x0 - r.x, h: r.h },
      ].filter((c) => c.w >= minW && c.h >= minH);
      if (!cuts.length) continue;
      r = cuts.reduce((a, b) => (b.w * b.h > a.w * a.h ? b : a));
    }
    return r;
  }

  visibleWorld(c: CameraState = this): Rect {
    const s = this.safeRect();
    return { x: c.x + s.x / c.zoom, y: c.y + s.y / c.zoom, w: s.w / c.zoom, h: s.h / c.zoom };
  }

  fromVisible(x: number, y: number, zoom: number): CameraState {
    const s = this.safeRect();
    return { x: x - s.x / zoom, y: y - s.y / zoom, zoom };
  }

  safeCenter(): { x: number; y: number } {
    const s = this.safeRect();
    return { x: s.x + s.w / 2, y: s.y + s.h / 2 };
  }

  fitRect(r: Rect, marginPx = 64, maxZoom = 1.5): CameraState {
    const w = Math.max(1, r.w);
    const h = Math.max(1, r.h);
    const s = this.safeRect();
    const m = Math.min(marginPx, s.w / 10, s.h / 10);
    const zx = (s.w - m * 2) / w;
    const zy = (s.h - m * 2) / h;
    const zoom = clampZoom(Math.min(zx, zy, maxZoom));
    return {
      zoom,
      x: r.x + r.w / 2 - (s.x + s.w / 2) / zoom,
      y: r.y + r.h / 2 - (s.y + s.h / 2) / zoom,
    };
  }

  get overstretched(): boolean {
    return this.zoom < MIN_ZOOM - 1e-9 || this.zoom > MAX_ZOOM + 1e-9;
  }
}
