import type { CanvasEngine } from '../engine.svelte';
import { VelocityTracker } from '../camera/inertia';
import { recognize } from '../draw/recognize';
import { seedFromId } from '../model/wire';
import { drawingBounds } from '../model/store.svelte';
import { GRID, newId, type CanvasNode, type Drawing, type Op, type Rect } from '../model/types';
import { minSizeFor } from '../nodes/registry';
import { boundsOf, rectFromPoints } from '../spatial/geometry';
import { drawingHit, hitTest, type ResizeHandle } from '../spatial/hit';
import { marqueeHits, marqueeRect, type MarqueeState } from './marquee';
import { magnetSnap, snap } from './snap';
import { openTerminal } from '../spawn';

export type ToolName =
  | 'select'
  | 'hand'
  | 'pen'
  | 'highlighter'
  | 'smart'
  | 'eraser'
  | 'rect'
  | 'ellipse'
  | 'diamond'
  | 'arrow'
  | 'line'
  | 'connect'
  | 'create';

export const SHAPE_TOOLS: ToolName[] = ['rect', 'ellipse', 'diamond', 'arrow', 'line'];
export const DRAW_TOOLS: ToolName[] = ['pen', 'highlighter', 'smart'];

type Pt = { x: number; y: number };

type Gesture =
  | { type: 'pan'; last: Pt }
  | {
      type: 'move';
      start: Pt;
      nodes: Map<string, Rect>;
      drawings: Map<string, Drawing>;
      primary: string | null;
      moved: boolean;
      clickedId: string | null;
      shift: boolean;
      alt: boolean;
    }
  | { type: 'resize'; handle: ResizeHandle; start: Pt; bounds: Rect; nodes: Map<string, Rect>; drawings: Map<string, Drawing> }
  | { type: 'marquee'; m: MarqueeState; base: string[] }
  | { type: 'stroke'; pts: number[]; pressures: number[]; kind: 'pen' | 'highlighter' | 'smart' }
  | { type: 'shape'; kind: string; start: Pt }
  | { type: 'cord'; from: string }
  | { type: 'erase'; forward: Op[]; inverse: Op[] }
  | { type: 'create'; start: Pt };

export class PointerTools {
  private g: Gesture | null = null;
  private velocity = new VelocityTracker();
  private root: HTMLElement | null = null;
  spaceHeld = false;
  panStarts = 0;
  private edgeEv: PointerEvent | null = null;
  private edgeRaf = 0;

  constructor(private readonly e: CanvasEngine) {}

  bind(root: HTMLElement): void {
    this.root = root;
  }

  get active(): boolean {
    return false;
  }

  get busy(): boolean {
    return this.g !== null;
  }

  local(ev: { clientX: number; clientY: number }): Pt {
    const r = this.root?.getBoundingClientRect();
    return { x: ev.clientX - (r?.left ?? 0), y: ev.clientY - (r?.top ?? 0) };
  }

  world(ev: { clientX: number; clientY: number }): Pt {
    const l = this.local(ev);
    return this.e.camera.screenToWorld(l.x, l.y);
  }

  private capture(ev: PointerEvent): void {
    try {
      this.root?.setPointerCapture(ev.pointerId);
    } catch {
    }
  }

  pointerDown(ev: PointerEvent): void {
    const e = this.e;
    e.inertia.stop();
    e.spawnMenu = null;
    const l = this.local(ev);
    const w = e.camera.screenToWorld(l.x, l.y);

    if (ev.button === 1 || (ev.button === 0 && (this.spaceHeld || e.tool === 'hand'))) {
      this.panStarts++;
      this.g = { type: 'pan', last: l };
      this.velocity.reset();
      this.velocity.push(l.x, l.y, ev.timeStamp);
      this.capture(ev);
      this.setCursor('grabbing');
      return;
    }
    if (ev.button !== 0) return;
    this.capture(ev);

    switch (e.tool) {
      case 'pen':
      case 'highlighter':
      case 'smart':
        this.g = { type: 'stroke', pts: [w.x, w.y], pressures: [pressureOf(ev)], kind: e.tool };
        this.updateStrokeDraft();
        break;
      case 'eraser':
        this.g = { type: 'erase', forward: [], inverse: [] };
        this.eraseAt(w);
        break;
      case 'rect':
      case 'ellipse':
      case 'diamond':
      case 'arrow':
      case 'line':
        this.g = { type: 'shape', kind: e.tool, start: w };
        this.updateShapeDraft(w, ev.shiftKey);
        break;
      case 'create':
        this.g = { type: 'create', start: w };
        break;
      case 'connect': {
        const hit = hitTest(e.store, w.x, w.y, { zoom: e.camera.zoom, skipDrawings: true });
        if (hit.type === 'node' || hit.type === 'port') {
          const src = e.selection.list().filter((id) => e.store.node(id));
          if (src.length === 1 && src[0] !== hit.id) {
            e.connect(src[0], hit.id);
            e.setTool('select');
            break;
          }
          this.g = { type: 'cord', from: hit.id };
          e.draft.cord = { from: hit.id, x: w.x, y: w.y, target: null };
        }
        break;
      }
      default:
        this.selectDown(ev, w);
    }
    e.invalidate();
  }

  private portNodes(): string[] {
    const s = this.e.selection;
    const out = s.list().filter((id) => this.e.store.node(id));
    if (s.hovered && !out.includes(s.hovered)) out.push(s.hovered);
    return out;
  }

  private selectDown(ev: PointerEvent, w: Pt): void {
    const e = this.e;
    const hit = hitTest(e.store, w.x, w.y, { zoom: e.camera.zoom, portsFor: this.portNodes() });
    if (hit.type === 'port') {
      this.g = { type: 'cord', from: hit.id };
      e.draft.cord = { from: hit.id, x: w.x, y: w.y, target: null };
      return;
    }
    if (hit.type === 'node' || hit.type === 'drawing') {
      const id = hit.id;
      if (e.selection.focused && e.selection.focused !== id) e.exitFocus();
      const gid = hit.type === 'drawing' ? e.store.drawing(id)?.groupId : undefined;
      const members = gid && ev.detail < 2 ? e.store.drawingGroupMembers(gid).map((d) => d.id) : [];
      if (members.length > 1) {
        if (ev.shiftKey) for (const m of members) e.selection.toggle(m);
        else if (!members.every((m) => e.selection.has(m))) e.selection.set(members);
        this.startMove(w, null, ev.shiftKey, ev.altKey);
        return;
      }
      if (gid && ev.detail >= 2 && !ev.shiftKey) {
        e.selection.set([id]);
        this.startMove(w, id, false, ev.altKey);
        return;
      }
      if (ev.shiftKey) e.selection.toggle(id);
      else if (!e.selection.has(id)) e.selection.set([id]);
      this.startMove(w, id, ev.shiftKey, ev.altKey);
      return;
    }
    if (hit.type === 'connection') {
      if (ev.shiftKey) e.selection.toggle(hit.id);
      else e.selection.set([hit.id]);
      return;
    }
    if (e.selection.focused) e.exitFocus();
    const base = ev.shiftKey ? e.selection.list() : [];
    if (!ev.shiftKey) e.selection.clear();
    this.g = { type: 'marquee', m: { start: w, end: w, additive: ev.shiftKey, invert: ev.altKey }, base };
  }

  private startMove(w: Pt, clickedId: string | null, shift: boolean, alt: boolean): void {
    const e = this.e;
    const nodes = new Map<string, Rect>();
    const drawings = new Map<string, Drawing>();
    for (const id of e.selection.ids) {
      const n = e.store.node(id);
      if (n && !n.locked) nodes.set(id, { x: n.x, y: n.y, w: n.w, h: n.h });
      const d = e.store.drawing(id);
      if (d) drawings.set(id, d);
    }
    const primary = clickedId && nodes.has(clickedId) ? clickedId : (nodes.keys().next().value ?? null);
    this.g = { type: 'move', start: w, nodes, drawings, primary, moved: false, clickedId, shift, alt };
  }

  rotateSelection(deg: number): boolean {
    const e = this.e;
    const list = e.selection
      .list()
      .map((id) => e.store.drawing(id))
      .filter((d): d is Drawing => !!d);
    if (!list.length) return false;
    let x0 = Infinity;
    let y0 = Infinity;
    let x1 = -Infinity;
    let y1 = -Infinity;
    for (const d of list) {
      x0 = Math.min(x0, d.x);
      y0 = Math.min(y0, d.y);
      x1 = Math.max(x1, d.x + d.w);
      y1 = Math.max(y1, d.y + d.h);
    }
    const px = (x0 + x1) / 2;
    const py = (y0 + y1) / 2;
    const rad = (deg * Math.PI) / 180;
    const sin = Math.sin(rad);
    const cos = Math.cos(rad);
    const ops: Op[] = [];
    for (const d of list) {
      const cx = d.x + d.w / 2 - px;
      const cy = d.y + d.h / 2 - py;
      const nx = px + cx * cos - cy * sin;
      const ny = py + cx * sin + cy * cos;
      const rotation = ((((d.rotation ?? 0) + deg) % 360) + 360) % 360;
      ops.push({
        op: 'update_drawing',
        id: d.id,
        patch: { x: d.x + nx - (d.x + d.w / 2), y: d.y + ny - (d.y + d.h / 2), rotation },
      });
    }
    e.commit(ops);
    return true;
  }

  startGroupDrag(ev: PointerEvent, groupId: string): void {
    const e = this.e;
    const ids = [
      ...e.store.groupMembers(groupId).map((n) => n.id),
      ...e.store.drawingGroupMembers(groupId).map((d) => d.id),
    ];
    e.selection.set(ids);
    this.capture(ev);
    this.startMove(this.world(ev), null, false, false);
  }

  startResize(ev: PointerEvent, handle: ResizeHandle): void {
    const e = this.e;
    const bounds = e.selectionBounds();
    if (!bounds) return;
    const nodes = new Map<string, Rect>();
    const drawings = new Map<string, Drawing>();
    for (const id of e.selection.ids) {
      const n = e.store.node(id);
      if (n && !n.locked) nodes.set(id, { x: n.x, y: n.y, w: n.w, h: n.h });
      const d = e.store.drawing(id);
      if (d) drawings.set(id, d);
    }
    if (!nodes.size && !drawings.size) return;
    const frame = nodes.size && !drawings.size ? unionRects(Array.from(nodes.values())) ?? bounds : bounds;
    this.capture(ev);
    this.g = { type: 'resize', handle, start: this.world(ev), bounds: frame, nodes, drawings };
  }

  startCord(ev: PointerEvent, from: string): void {
    const w = this.world(ev);
    this.capture(ev);
    this.g = { type: 'cord', from };
    this.e.draft.cord = { from, x: w.x, y: w.y, target: null };
    this.e.invalidate();
  }

  pointerMove(ev: PointerEvent): void {
    const e = this.e;
    const g = this.g;
    if (!g) {
      this.hover(ev);
      return;
    }
    const l = this.local(ev);
    const w = e.camera.screenToWorld(l.x, l.y);
    switch (g.type) {
      case 'pan': {
        e.panBy(l.x - g.last.x, l.y - g.last.y);
        g.last = l;
        this.velocity.push(l.x, l.y, ev.timeStamp);
        break;
      }
      case 'move':
        this.moveTo(g, w, ev);
        this.edgePan(ev);
        break;
      case 'resize':
        this.resizeTo(g, w, ev.shiftKey);
        break;
      case 'marquee': {
        g.m.end = w;
        g.m.invert = ev.altKey;
        e.draft.marquee = marqueeRect(g.m);
        const hits = marqueeHits(e.store, g.m);
        e.selection.set([...g.base, ...hits]);
        break;
      }
      case 'stroke': {
        const events = typeof ev.getCoalescedEvents === 'function' ? ev.getCoalescedEvents() : [];
        for (const ce of events.length ? events : [ev]) {
          const p = this.world(ce);
          const n = g.pts.length;
          if (Math.hypot(p.x - g.pts[n - 2], p.y - g.pts[n - 1]) < 0.75 / e.camera.zoom) continue;
          g.pts.push(p.x, p.y);
          g.pressures.push(pressureOf(ce));
        }
        this.updateStrokeDraft();
        break;
      }
      case 'shape':
        this.updateShapeDraft(w, ev.shiftKey);
        break;
      case 'cord': {
        const hit = hitTest(e.store, w.x, w.y, { zoom: e.camera.zoom, skipDrawings: true });
        const target = hit.type === 'node' || hit.type === 'port' ? hit.id : null;
        e.draft.cord = { from: g.from, x: w.x, y: w.y, target: target && target !== g.from ? target : null };
        break;
      }
      case 'erase':
        this.eraseAt(w);
        break;
      case 'create': {
        const r = rectFromPoints(g.start, w);
        e.draft.create = r.w * e.camera.zoom > 8 || r.h * e.camera.zoom > 8 ? r : undefined;
        break;
      }
    }
    e.invalidate();
  }

  private moveTo(g: Extract<Gesture, { type: 'move' }>, w: Pt, ev: PointerEvent): void {
    const e = this.e;
    let dx = w.x - g.start.x;
    let dy = w.y - g.start.y;
    if (!g.moved) {
      if (Math.hypot(dx, dy) * e.camera.zoom < 3) return;
      g.moved = true;
      if (g.alt) this.duplicateForDrag(g);
    }
    if (ev.metaKey && g.primary && g.nodes.has(g.primary)) {
      const s = g.nodes.get(g.primary)!;
      const proposed = { x: s.x + dx, y: s.y + dy, w: s.w, h: s.h };
      const exclude = new Set(g.nodes.keys());
      const near = e.store.nodeIndex
        .queryRect({ x: proposed.x - 400, y: proposed.y - 400, w: proposed.w + 800, h: proposed.h + 800 })
        .filter((id) => !exclude.has(id))
        .map((id) => e.store.node(id))
        .filter((n): n is CanvasNode => !!n && !n.stackId);
      const m = magnetSnap(proposed, near, 24 / e.camera.zoom);
      dx = m.snappedX ? dx + m.dx : snap(proposed.x) - s.x;
      dy = m.snappedY ? dy + m.dy : snap(proposed.y) - s.y;
      e.draft.guides = undefined;
    } else if (g.primary && g.nodes.has(g.primary)) {
      const s = g.nodes.get(g.primary)!;
      const proposed = { x: s.x + dx, y: s.y + dy, w: s.w, h: s.h };
      const guide = e.guidesFor(proposed, new Set(g.nodes.keys()));
      dx = guide.guides.some((x) => x.axis === 'x') ? proposed.x + guide.dx - s.x : snap(proposed.x) - s.x;
      dy = guide.guides.some((x) => x.axis === 'y') ? proposed.y + guide.dy - s.y : snap(proposed.y) - s.y;
      e.draft.guides = guide.guides;
    } else if (!ev.metaKey) {
      dx = snap(dx);
      dy = snap(dy);
    }
    const ops: Op[] = [];
    for (const [id, s] of g.nodes) ops.push({ op: 'update_node', id, patch: { x: s.x + dx, y: s.y + dy } });
    for (const [id, d] of g.drawings) {
      ops.push({ op: 'update_drawing', id, patch: { x: d.x + dx, y: d.y + dy } });
    }
    e.store.apply(ops, 'user');
  }

  private duplicateForDrag(g: Extract<Gesture, { type: 'move' }>): void {
    const e = this.e;
    const ops: Op[] = [];
    let z = e.store.maxZ();
    const terminals: CanvasNode[] = [];
    for (const [id, start] of [...g.nodes]) {
      const n = e.store.node(id);
      if (!n) continue;
      const copy: CanvasNode = { ...n, id: newId(), z: ++z, data: { ...n.data } };
      delete copy.groupId;
      if (n.kind === 'terminal' || n.kind === 'agent') {
        copy.data.terminalId = newId();
        delete copy.data.isManager;
        copy.title = e.uniqueTitle(n.title ?? 'Terminal');
        g.nodes.delete(id);
        g.nodes.set(copy.id, start);
        if (g.primary === id) g.primary = copy.id;
        terminals.push(copy);
      } else if (copy.title) {
        copy.title = e.uniqueTitle(copy.title);
      }
      ops.push({ op: 'add_node', node: copy });
    }
    for (const [, d] of g.drawings) ops.push({ op: 'add_drawing', drawing: { ...d, id: newId() } });
    if (!ops.length) return;
    e.commit(ops);
    if (terminals.length) {
      e.selection.set([...g.nodes.keys(), ...g.drawings.keys()]);
      for (const n of terminals) {
        void openTerminal(n, e.store.workspaceId, { floorId: e.store.floorId }).catch(() => e.say('canvas.engine.terminalFailed'));
      }
    }
  }

  private resizeTo(g: Extract<Gesture, { type: 'resize' }>, w: Pt, keepRatio: boolean): void {
    const e = this.e;
    const b = g.bounds;
    const dx = w.x - g.start.x;
    const dy = w.y - g.start.y;
    let x0 = b.x;
    let y0 = b.y;
    let x1 = b.x + b.w;
    let y1 = b.y + b.h;
    if (g.handle.includes('w')) x0 = snap(b.x + dx);
    if (g.handle.includes('e')) x1 = snap(b.x + b.w + dx);
    if (g.handle.includes('n')) y0 = snap(b.y + dy);
    if (g.handle.includes('s')) y1 = snap(b.y + b.h + dy);
    let min = { w: GRID * 2, h: GRID * 2 };
    if (g.nodes.size === 1 && !g.drawings.size) {
      const n = e.store.node(g.nodes.keys().next().value as string);
      if (n) min = minSizeFor(n.kind);
    }
    if (x1 - x0 < min.w) {
      if (g.handle.includes('w')) x0 = x1 - min.w;
      else x1 = x0 + min.w;
    }
    if (y1 - y0 < min.h) {
      if (g.handle.includes('n')) y0 = y1 - min.h;
      else y1 = y0 + min.h;
    }
    if (keepRatio && b.w > 0 && b.h > 0) {
      const k = Math.max((x1 - x0) / b.w, (y1 - y0) / b.h);
      if (g.handle.includes('w')) x0 = x1 - b.w * k;
      else x1 = x0 + b.w * k;
      if (g.handle.includes('n')) y0 = y1 - b.h * k;
      else y1 = y0 + b.h * k;
    }
    const sx = (x1 - x0) / (b.w || 1);
    const sy = (y1 - y0) / (b.h || 1);
    const ops: Op[] = [];
    for (const [id, r] of g.nodes) {
      const n = e.store.node(id);
      const mn = n ? minSizeFor(n.kind) : { w: GRID, h: GRID };
      const nx = snap(x0 + (r.x - b.x) * sx);
      const ny = snap(y0 + (r.y - b.y) * sy);
      const nw = Math.max(mn.w, snap(r.w * sx));
      const nh = Math.max(mn.h, snap(r.h * sy));
      ops.push({ op: 'update_node', id, patch: { x: nx, y: ny, w: nw, h: nh } });
    }
    for (const [id, d] of g.drawings) {
      const patch: Partial<Drawing> = {
        x: x0 + (d.x - b.x) * sx,
        y: y0 + (d.y - b.y) * sy,
        w: d.w * sx,
        h: d.h * sy,
      };
      if (d.points) patch.points = d.points.map((v, i) => Math.round(v * (i % 2 === 0 ? sx : sy) * 10) / 10);
      ops.push({ op: 'update_drawing', id, patch });
    }
    e.store.apply(ops, 'user');
  }

  private eraseAt(w: Pt): void {
    const e = this.e;
    const g = this.g;
    if (!g || g.type !== 'erase') return;
    const tol = 8 / e.camera.zoom;
    e.draft.eraser = { x: w.x, y: w.y, r: tol };
    for (const id of e.store.drawingIndex.queryPoint(w.x, w.y, tol)) {
      const d = e.store.drawing(id);
      if (!d || !drawingHit(d, w.x, w.y, tol)) continue;
      const fwd: Op[] = [{ op: 'remove_drawing', id }];
      const inv = e.store.apply(fwd, 'user');
      g.forward.push(...fwd);
      g.inverse = [...inv, ...g.inverse];
    }
  }

  private updateStrokeDraft(): void {
    const g = this.g;
    if (!g || g.type !== 'stroke') return;
    const e = this.e;
    e.draft.stroke = {
      pts: g.pts,
      pressures: g.pressures,
      color: e.penColor,
      width: g.kind === 'highlighter' ? e.strokeWidth * 5 : e.strokeWidth,
      highlighter: g.kind === 'highlighter',
    };
  }

  private updateShapeDraft(w: Pt, shift: boolean): void {
    const g = this.g;
    if (!g || g.type !== 'shape') return;
    const e = this.e;
    let x1 = w.x;
    let y1 = w.y;
    if (shift) {
      if (g.kind === 'arrow' || g.kind === 'line') {
        const a = Math.round(Math.atan2(y1 - g.start.y, x1 - g.start.x) / (Math.PI / 4)) * (Math.PI / 4);
        const len = Math.hypot(x1 - g.start.x, y1 - g.start.y);
        x1 = g.start.x + Math.cos(a) * len;
        y1 = g.start.y + Math.sin(a) * len;
      } else {
        const s = Math.max(Math.abs(x1 - g.start.x), Math.abs(y1 - g.start.y));
        x1 = g.start.x + Math.sign(x1 - g.start.x || 1) * s;
        y1 = g.start.y + Math.sign(y1 - g.start.y || 1) * s;
      }
    }
    e.draft.shape = {
      kind: g.kind,
      x0: g.start.x,
      y0: g.start.y,
      x1,
      y1,
      color: e.penColor,
      width: e.strokeWidth,
      sketch: e.sketch,
      fillStyle: e.fillStyle,
    };
  }

  private edgePan(ev: PointerEvent): void {
    this.edgeEv = ev;
    if (this.edgeRaf) return;
    const EDGE = 28;
    const step = () => {
      this.edgeRaf = 0;
      const e = this.e;
      const g = this.g;
      const last = this.edgeEv;
      if (!g || g.type !== 'move' || !g.moved || !last) return;
      const l = this.local(last);
      const left = e.camera.insetLeft ?? 0;
      const right = e.camera.vw - (e.camera.insetRight ?? 0);
      const speed = (d: number) => Math.min(1.5, Math.max(0, d) / EDGE) * 16;
      let vx = 0;
      let vy = 0;
      if (l.x < left + EDGE) vx = -speed(left + EDGE - l.x);
      else if (l.x > right - EDGE) vx = speed(l.x - (right - EDGE));
      if (l.y < EDGE) vy = -speed(EDGE - l.y);
      else if (l.y > e.camera.vh - EDGE) vy = speed(l.y - (e.camera.vh - EDGE));
      if (!vx && !vy) return;
      e.panBy(-vx, -vy);
      this.moveTo(g, this.world(last), last);
      e.invalidate();
      this.edgeRaf = requestAnimationFrame(step);
    };
    this.edgeRaf = requestAnimationFrame(step);
  }

  private stopEdgePan(): void {
    if (this.edgeRaf) cancelAnimationFrame(this.edgeRaf);
    this.edgeRaf = 0;
    this.edgeEv = null;
  }

  pointerUp(ev: PointerEvent): void {
    const e = this.e;
    const g = this.g;
    this.g = null;
    this.stopEdgePan();
    try {
      this.root?.releasePointerCapture(ev.pointerId);
    } catch {
    }
    if (!g) return;
    switch (g.type) {
      case 'pan': {
        const v = this.velocity.velocity(ev.timeStamp);
        if (e.inertia.start(v.vx, v.vy)) e.invalidate();
        this.setCursor(this.spaceHeld || e.tool === 'hand' ? 'grab' : '');
        break;
      }
      case 'move':
        this.endMove(g);
        break;
      case 'resize':
        this.endTransform(g.nodes, g.drawings);
        break;
      case 'marquee':
        e.draft.marquee = undefined;
        break;
      case 'stroke':
        this.endStroke(g);
        break;
      case 'shape':
        this.endShape(g);
        break;
      case 'cord': {
        const target = e.draft.cord?.target ?? null;
        const at = e.draft.cord;
        e.draft.cord = undefined;
        const back = at ? hitTest(e.store, at.x, at.y, { zoom: e.camera.zoom, skipDrawings: true }) : null;
        if (target) {
          e.connect(g.from, target);
          if (e.tool === 'connect') e.setTool('select');
        } else if (e.tool === 'connect' && back && back.type !== 'none' && back.id === g.from) {
          e.selection.set([g.from]);
        } else if (at) {
          const s = e.camera.worldToScreen(at.x, at.y);
          e.spawnMenu = { sx: s.x, sy: s.y, wx: at.x, wy: at.y, connectFrom: g.from };
        }
        break;
      }
      case 'erase':
        e.draft.eraser = undefined;
        e.recordApplied(g.forward, g.inverse);
        break;
      case 'create': {
        const kind = e.createKind ?? 'note';
        const w = this.world(ev);
        const r = rectFromPoints(g.start, w);
        e.draft.create = undefined;
        const dragged = r.w * e.camera.zoom > 8 && r.h * e.camera.zoom > 8;
        const node = e.spawn(kind, dragged ? null : g.start, {
          agent: e.createAgent ?? undefined,
          preset: e.createPreset ?? undefined,
          rect: dragged ? { x: snap(r.x), y: snap(r.y), w: Math.max(snap(r.w), GRID * 4), h: Math.max(snap(r.h), GRID * 2) } : undefined,
        });
        e.setTool('select');
        if (kind === 'text' || kind === 'note') e.selection.focus(node.id);
        break;
      }
    }
    e.draft.guides = undefined;
    e.invalidate();
  }

  private endMove(g: Extract<Gesture, { type: 'move' }>): void {
    const e = this.e;
    if (!g.moved) {
      if (!g.shift && g.clickedId && e.selection.size > 1) e.selection.set([g.clickedId]);
      return;
    }
    this.endTransform(g.nodes, g.drawings);
  }

  private endTransform(nodes: Map<string, Rect>, drawings: Map<string, Drawing>): void {
    const e = this.e;
    const forward: Op[] = [];
    const inverse: Op[] = [];
    for (const [id, s] of nodes) {
      const n = e.store.node(id);
      if (!n) continue;
      if (n.x === s.x && n.y === s.y && n.w === s.w && n.h === s.h) continue;
      forward.push({ op: 'update_node', id, patch: { x: n.x, y: n.y, w: n.w, h: n.h } });
      inverse.push({ op: 'update_node', id, patch: { x: s.x, y: s.y, w: s.w, h: s.h } });
    }
    for (const [id, before] of drawings) {
      const d = e.store.drawing(id);
      if (!d) continue;
      const keys: Array<keyof Drawing> = ['x', 'y', 'w', 'h', 'points'];
      const fwd: Partial<Drawing> = {};
      const inv: Partial<Drawing> = {};
      for (const k of keys) {
        (fwd as Record<string, unknown>)[k] = d[k];
        (inv as Record<string, unknown>)[k] = before[k];
      }
      forward.push({ op: 'update_drawing', id, patch: fwd });
      inverse.push({ op: 'update_drawing', id, patch: inv });
    }
    e.recordApplied(forward, inverse);
  }

  private endStroke(g: Extract<Gesture, { type: 'stroke' }>): void {
    const e = this.e;
    e.draft.stroke = undefined;
    if (g.pts.length < 2) return;
    if (g.kind === 'smart') {
      const r = recognize(g.pts);
      if (r) {
        if (r.kind === 'line') {
          this.commitDrawing(this.lineDrawing('line', r.x0, r.y0, r.x1, r.y1));
        } else {
          this.commitDrawing(this.shapeDrawing(r.kind, r.rect));
        }
        return;
      }
    }
    const b = boundsOf(g.pts);
    const pts = g.pts.map((v, i) => Math.round((v - (i % 2 === 0 ? b.x : b.y)) * 10) / 10);
    const hasPressure = g.pressures.some((p) => p !== 0.5 && p > 0);
    const d: Drawing = {
      id: newId(),
      kind: g.kind === 'highlighter' ? 'highlighter' : 'pen',
      x: b.x,
      y: b.y,
      w: b.w,
      h: b.h,
      z: e.store.maxDrawingZ() + 1,
      points: pts,
      color: e.penColor,
      strokeWidth: g.kind === 'highlighter' ? e.strokeWidth * 5 : e.strokeWidth,
    };
    if (hasPressure) d.pressures = g.pressures.map((p) => Math.round(p * 100) / 100);
    if (g.kind === 'highlighter') d.opacity = 0.4;
    this.commitDrawing(d);
  }

  private shapeDrawing(kind: string, r: Rect): Drawing {
    const e = this.e;
    const id = newId();
    const d: Drawing = {
      id,
      kind,
      x: r.x,
      y: r.y,
      w: r.w,
      h: r.h,
      z: e.store.maxDrawingZ() + 1,
      color: e.penColor,
      strokeWidth: e.strokeWidth,
      seed: seedFromId(id),
    };
    if (e.sketch) d.sketch = true;
    if (e.fillStyle !== 'none') {
      d.fillStyle = e.fillStyle;
      d.fill = e.penColor;
    }
    return d;
  }

  private lineDrawing(kind: 'arrow' | 'line', x0: number, y0: number, x1: number, y1: number): Drawing {
    const e = this.e;
    const x = Math.min(x0, x1);
    const y = Math.min(y0, y1);
    const id = newId();
    const d: Drawing = {
      id,
      kind,
      x,
      y,
      w: Math.abs(x1 - x0),
      h: Math.abs(y1 - y0),
      z: e.store.maxDrawingZ() + 1,
      points: [x0 - x, y0 - y, x1 - x, y1 - y].map((v) => Math.round(v * 10) / 10),
      color: e.penColor,
      strokeWidth: e.strokeWidth,
      seed: seedFromId(id),
    };
    if (e.sketch) d.sketch = true;
    return d;
  }

  private endShape(g: Extract<Gesture, { type: 'shape' }>): void {
    const e = this.e;
    const s = e.draft.shape;
    e.draft.shape = undefined;
    if (!s) return;
    const small = Math.hypot(s.x1 - s.x0, s.y1 - s.y0) * e.camera.zoom < 6;
    if (g.kind === 'arrow' || g.kind === 'line') {
      if (small) return;
      this.commitDrawing(this.lineDrawing(g.kind, s.x0, s.y0, s.x1, s.y1));
    } else {
      const r = small ? { x: s.x0 - 80, y: s.y0 - 50, w: 160, h: 100 } : rectFromPoints({ x: s.x0, y: s.y0 }, { x: s.x1, y: s.y1 });
      this.commitDrawing(this.shapeDrawing(g.kind, r));
    }
  }

  private commitDrawing(d: Drawing): void {
    this.e.commit([{ op: 'add_drawing', drawing: d }]);
  }

  hover(ev: PointerEvent): void {
    const e = this.e;
    if (e.tool !== 'select' && e.tool !== 'connect') {
      this.setCursor(e.tool === 'hand' ? 'grab' : 'crosshair');
      return;
    }
    if (this.spaceHeld) return this.setCursor('grab');
    const w = this.world(ev);
    const hit = hitTest(e.store, w.x, w.y, { zoom: e.camera.zoom, portsFor: this.portNodes(), skipDrawings: false });
    const hovered = hit.type === 'node' || hit.type === 'port' ? hit.id : null;
    if (e.selection.hovered !== hovered) {
      e.selection.hovered = hovered;
      e.invalidate();
    }
    if (hit.type === 'port') this.setCursor('crosshair');
    else if (hit.type === 'node' && hit.zone === 'header') this.setCursor('grab');
    else if (hit.type === 'drawing' || hit.type === 'connection') this.setCursor('pointer');
    else this.setCursor('');
  }

  setCursor(c: string): void {
    if (this.root && this.root.style.cursor !== c) this.root.style.cursor = c;
  }

  cancel(): void {
    const e = this.e;
    this.stopEdgePan();
    if (!this.g) return;
    const g = this.g;
    this.g = null;
    if (g.type === 'move' || g.type === 'resize') {
      const ops: Op[] = [];
      for (const [id, s] of g.nodes) ops.push({ op: 'update_node', id, patch: { x: s.x, y: s.y, w: s.w, h: s.h } });
      for (const [id, d] of g.drawings) ops.push({ op: 'update_drawing', id, patch: { x: d.x, y: d.y, w: d.w, h: d.h, points: d.points } });
      e.store.apply(ops, 'user');
    } else if (g.type === 'erase') {
      e.store.apply(g.inverse, 'user');
    }
    e.draft = {};
    e.invalidate();
  }
}

function pressureOf(ev: PointerEvent): number {
  return ev.pointerType === 'pen' ? ev.pressure || 0.5 : 0.5;
}

function unionRects(rs: Rect[]): Rect | null {
  if (!rs.length) return null;
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  for (const r of rs) {
    x0 = Math.min(x0, r.x);
    y0 = Math.min(y0, r.y);
    x1 = Math.max(x1, r.x + r.w);
    y1 = Math.max(y1, r.y + r.h);
  }
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

export { drawingBounds };
