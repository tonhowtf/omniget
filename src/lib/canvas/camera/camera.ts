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

  center(): { x: number; y: number } {
    return { x: this.x + this.vw / (2 * this.zoom), y: this.y + this.vh / (2 * this.zoom) };
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

  fitRect(r: Rect, marginPx = 64, maxZoom = 1.5): CameraState {
    const w = Math.max(1, r.w);
    const h = Math.max(1, r.h);
    const inset = Math.min(this.insetLeft, this.vw * 0.5);
    const right = Math.min(this.insetRight, Math.max(0, this.vw * 0.75 - inset));
    const usable = Math.max(1, this.vw - inset - right);
    const zx = (usable - marginPx * 2) / w;
    const zy = (this.vh - marginPx * 2) / h;
    const zoom = clampZoom(Math.min(zx, zy, maxZoom));
    return {
      zoom,
      x: r.x + r.w / 2 - (inset + usable / 2) / zoom,
      y: r.y + r.h / 2 - this.vh / (2 * zoom),
    };
  }

  get overstretched(): boolean {
    return this.zoom < MIN_ZOOM - 1e-9 || this.zoom > MAX_ZOOM + 1e-9;
  }
}
