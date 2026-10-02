import { bootWorkspace } from './session';
import { getContext, setContext } from 'svelte';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { Camera, clampZoom } from './camera/camera';
import { flyTo, easeInOutCubic, type Flight } from './camera/flyto';
import { Inertia, springStep } from './camera/inertia';
import { cordPath, reverseCord, sampleCord } from './cords/route';
import { kindColor, Traffic } from './cords/traffic';
import { cordRefusal } from './cords/validate';
import { allocate, isCostlySwap, mountsInWorld, mountedTerminalsLimit, nodeLod, type Lod } from './lod';
import { History } from './model/history';
import { removeNodesOps } from './model/ops';
import { CanvasStore, drawingBounds } from './model/store.svelte';
import { CanvasSync, type SyncMode } from './model/sync';
import {
  GRID,
  isTerminalKind,
  MAX_ZOOM,
  MIN_ZOOM,
  newId,
  terminalIdOf,
  type AgentKind,
  type CameraState,
  type CanvasNode,
  type Connection,
  type Drawing,
  type FillStyle,
  type Op,
  type Origin,
  type Rect,
} from './model/types';
import { onRegistryChange, sizeFor } from './nodes/registry';
import { canvasPrefs, faithfulRendering, prefSize, presetAgent, presetData, userPreset } from './prefs.svelte';
import { GlLayer, onPassesChange, type FrameInfo, type NodeView } from './render/gl-layer';
import { Selection } from './interaction/selection.svelte';
import { alignGuides, arrange, nearestFreeSpot, snap, type AlignMode, type Guide } from './interaction/snap';
import { neighborInDirection, type Dir } from './interaction/keyboard';
import { PointerTools, type ToolName } from './interaction/tools';
import { agentStates } from './state/agents.svelte';
import { union, intersects } from './spatial/geometry';
import { openTerminal, closeTerminal } from './spawn';
import { workspaces } from './palette/workspaces.svelte';
import { Panels } from './panels/panels.svelte';

export const ENGINE_KEY = Symbol('canvas-engine');
export const SETTLE_MS = 150;
const SOFT_DELETE_MS = 60_000;
const MESSAGE_EVENT = 'canvas://message';
const ASK_EVENT = 'canvas://ask';

type CanvasPanel =
  | { kind: 'partituras' }
  | { kind: 'import'; id: string; at: { x: number; y: number } }
  | { kind: 'routine'; targetNodeId?: string }
  | { kind: 'roles' }
  | { kind: 'role-picker'; nodeId: string }
  | { kind: 'companion' }
  | { kind: 'visit' }
  | { kind: 'floors' }
  | { kind: 'new-portal' }
  | { kind: 'new-filetree' };

interface MessagePayload {
  id?: string;
  from: string | null;
  to: string;
  kind?: string;
  state?: string;
  askId?: string | null;
}

export interface Draft {
  stroke?: { pts: number[]; pressures: number[]; color: string; width: number; highlighter: boolean };
  shape?: { kind: string; x0: number; y0: number; x1: number; y1: number; color: string; width: number; sketch: boolean; fillStyle: FillStyle };
  cord?: { from: string; x: number; y: number; target: string | null };
  marquee?: Rect;
  guides?: Guide[];
  eraser?: { x: number; y: number; r: number };
  create?: Rect;
}

interface SpawnMenuState {
  sx: number;
  sy: number;
  wx: number;
  wy: number;
  nodeId?: string;
  connectFrom?: string;
  free?: boolean;
}

interface SpawnOptions {
  agent?: AgentKind;
  rect?: Rect;
  title?: string;
  data?: Record<string, unknown>;
  connectFrom?: string;
  select?: boolean;
  preset?: string;
}

const AGENT_TITLES: Record<string, string> = {
  shell: 'Terminal',
  claude: 'Claude Code',
  codex: 'Codex',
  gemini: 'Gemini',
  opencode: 'OpenCode',
  ollama: 'Ollama',
  native: 'Agent',
};

export class CanvasEngine {
  readonly store = new CanvasStore();
  private histories = new Map<string, History>();
  get history(): History {
    const key = `${this.store.workspaceId}\u0000${this.store.floorId}`;
    let h = this.histories.get(key);
    if (!h) {
      h = new History();
      if (this.histories.size >= 24) this.histories.delete(this.histories.keys().next().value!);
      this.histories.set(key, h);
    }
    return h;
  }
  readonly sync: CanvasSync;
  readonly selection = new Selection();
  readonly camera = new Camera();
  readonly traffic = new Traffic();
  readonly layer = new GlLayer();
  readonly tools: PointerTools;
  readonly inertia = new Inertia();
  readonly panels: Panels = new Panels(this);

  tool = $state<ToolName>('select');
  createKind = $state<string | null>(null);
  createAgent = $state<AgentKind | null>(null);
  createPreset = $state<string | null>(null);
  syncMode = $state<SyncMode>('connecting');
  #minimap = $state(readFlag('omniget.canvas.minimap.v2', false));
  get minimapVisible(): boolean {
    return this.#minimap;
  }
  set minimapVisible(v: boolean) {
    this.#minimap = v;
    try {
      localStorage.setItem('omniget.canvas.minimap.v2', v ? '1' : '0');
    } catch {
    }
  }
  lod = $state.raw<Record<string, Lod>>({});
  mounted = $state.raw<Array<{ id: string; hidden: boolean }>>([]);
  cards = $state.raw<string[]>([]);
  moving = $state(false);
  floorsView = $state(false);
  zoomUi = $state(1);
  showBadges = $state(false);
  spawnMenu = $state<SpawnMenuState | null>(null);
  notice = $state<string | null>(null);
  registryRev = $state(0);
  glFallback = $state(false);
  penColor = $state('#e8e6e3');
  strokeWidth = $state(3);
  sketch = $state(true);
  fillStyle = $state<FillStyle>('none');
  splitConnects = $state(true);
  cordStyle: 'bezier' | 'rope' = 'bezier';
  panel = $state<CanvasPanel | null>(null);
  restarts = $state.raw<Record<string, number>>({});
  draft: Draft = {};

  private raf = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private lastTick = 0;
  private lastCameraRev = -1;
  private lastCameraMove = 0;
  private settled = true;
  private flight: { f: Flight; t0: number; to: CameraState } | null = null;
  chromeProbe: (() => Rect[]) | null = null;
  private zoomVelocity = 0;
  pinching = false;
  private frameHooks = new Set<(now: number) => void>();
  private wantFps = 0;
  private prevCamera: CameraState | null = null;
  private pendingKills = new Map<string, ReturnType<typeof setTimeout>>();
  private unlisten: Array<() => void> = [];
  private rootEl: HTMLElement | null = null;
  private worldEl: HTMLElement | null = null;
  private mountedLru = new Map<string, number>();
  private noticeTimer: ReturnType<typeof setTimeout> | null = null;
  private destroyed = false;

  constructor() {
    this.sync = new CanvasSync(this.store);
    this.sync.onModeChange = (m) => (this.syncMode = m);
    this.sync.onRemote = (ops, origin) => {
      if (ops.length && origin !== 'user' && origin !== 'load' && origin !== 'normalise') {
        this.history.record(origin, ops, []);
      }
      this.selection.prune((id) => !!this.store.node(id) || !!this.store.drawing(id) || !!this.store.connection(id));
      this.invalidate();
    };
    this.tools = new PointerTools(this);
    this.unlisten.push(
      this.store.subscribe((ops) => {
        for (const op of ops) {
          if (op.op === 'add_node' && isTerminalKind(op.node.kind)) this.cancelKill(terminalIdOf(op.node));
          if (op.op === 'remove_node') this.panels.forget(op.id);
        }
      }),
    );
    this.unlisten.push(onRegistryChange(() => this.registryRev++));
    this.unlisten.push(onPassesChange(() => this.invalidate()));
    this.unlisten.push(
      $effect.root(() => {
        $effect(() => {
          void this.store.version;
          void this.selection.ids;
          void this.selection.hovered;
          void this.selection.focused;
          void agentStates.byTerminal;
          void agentStates.byNode;
          this.invalidate();
        });
      }),
    );
  }

  homeDir = '';

  async start(workspaceId?: string): Promise<void> {
    void agentStates.start();
    void import('@tauri-apps/api/path')
      .then((m) => m.homeDir())
      .then((h) => (this.homeDir = h))
      .catch(() => {});
    await this.sync.open(await bootWorkspace(workspaceId));
    if (this.destroyed) return;
    this.selection.setScope(this.store.workspaceId, this.store.floorId);
    const cam = this.store.camera;
    if (cam) this.camera.set(cam);
    else this.fitAll(false);
    await this.listenTraffic();
    this.invalidate();
  }

  private async listenTraffic(): Promise<void> {
    try {
      const u1: UnlistenFn = await listen<MessagePayload>(
        MESSAGE_EVENT,
        (e) => this.onMessage(e.payload),
      );
      const u2: UnlistenFn = await listen<{ id: string; from: string; to: string; state: string }>(ASK_EVENT, (e) => {
        if (this.traffic.trackAsk(e.payload)) this.invalidate();
        if (e.payload?.state === 'answered') {
          this.onMessage({ id: e.payload.id, from: e.payload.from, to: e.payload.to, state: 'answered' });
        }
      });
      if (this.destroyed) {
        u1();
        u2();
        return;
      }
      this.unlisten.push(u1, u2);
    } catch {
    }
  }

  attachElements(root: HTMLElement, world: HTMLElement): void {
    this.rootEl = root;
    this.worldEl = world;
    this.writeTransforms();
  }

  destroy(): void {
    this.destroyed = true;
    this.sync.dispose();
    agentStates.stop();
    if (this.raf) cancelAnimationFrame(this.raf);
    if (this.timer) clearTimeout(this.timer);
    for (const u of this.unlisten) u();
    this.unlisten = [];
    this.layer.destroy();
    for (const [tid, t] of this.pendingKills) {
      clearTimeout(t);
      void closeTerminal(tid);
    }
    this.pendingKills.clear();
  }

  onFrame(fn: (now: number) => void): () => void {
    this.frameHooks.add(fn);
    return () => this.frameHooks.delete(fn);
  }

  invalidate(): void {
    if (this.destroyed || this.raf) return;
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    this.raf = requestAnimationFrame((t) => this.tick(t));
  }

  wantFrames(fps: number): void {
    if (fps > this.wantFps) this.wantFps = fps;
  }

  private schedule(delayMs: number): void {
    if (this.destroyed || this.raf || this.timer) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      this.invalidate();
    }, delayMs);
  }

  private tick(now: number): void {
    this.raf = 0;
    const dt = this.lastTick ? Math.min(100, now - this.lastTick) : 16;
    this.lastTick = now;
    let animating = false;

    if (this.flight) {
      const t = Math.min(1, (now - this.flight.t0) / this.flight.f.durationMs);
      this.camera.set(this.flight.f.at(easeInOutCubic(t)));
      if (t >= 1) this.flight = null;
      else animating = true;
    }
    if (this.inertia.running) {
      const d = this.inertia.step(dt);
      this.camera.panBy(d.dx, d.dy);
      animating = this.inertia.running;
    }
    if (!this.pinching && this.camera.overstretched) {
      const target = this.camera.zoom < MIN_ZOOM ? MIN_ZOOM : MAX_ZOOM;
      const s = springStep(this.camera.zoom, this.zoomVelocity, target, dt);
      this.zoomVelocity = s.velocity;
      const nz = Math.abs(s.value - target) < 1e-4 ? target : s.value;
      this.camera.zoomTo(nz, this.camera.focusX, this.camera.vh / 2, true);
      if (nz !== target) animating = true;
      else this.zoomVelocity = 0;
    }

    if (this.camera.rev !== this.lastCameraRev) {
      this.lastCameraRev = this.camera.rev;
      this.lastCameraMove = now;
      if (this.settled) {
        this.settled = false;
        this.moving = true;
      }
      this.writeTransforms();
    }
    const cameraIdle = now - this.lastCameraMove >= SETTLE_MS;
    if (!this.settled && cameraIdle && !animating && !this.pinching) {
      this.settled = true;
      this.moving = false;
      this.zoomUi = this.camera.zoom;
      this.sync.saveCamera(this.camera.state());
    } else if (!this.settled && Math.abs(this.zoomUi - this.camera.zoom) > 0.01) {
      this.zoomUi = this.camera.zoom;
    }

    this.updateView(now);

    this.wantFps = 0;
    const frame = this.frameInfo(now);
    if (this.glFallback) this.fallbackDraw?.(frame);
    else this.layer.render(frame);
    for (const fn of this.frameHooks) fn(now);

    if (animating || this.tools.active) this.invalidate();
    else if (this.traffic.active) this.schedule(33);
    else if (this.wantFps > 0) this.schedule(1000 / this.wantFps);
    else if (!this.settled) this.schedule(SETTLE_MS);
  }

  fallbackDraw: ((frame: FrameInfo) => void) | null = null;

  private writeTransforms(): void {
    const c = this.camera;
    if (this.worldEl) {
      this.worldEl.style.transform = `scale(${c.zoom}) translate(${-c.x}px, ${-c.y}px)`;
    }
    if (this.rootEl) {
      this.rootEl.style.setProperty('--cx', String(c.x));
      this.rootEl.style.setProperty('--cy', String(c.y));
      this.rootEl.style.setProperty('--z', String(c.zoom));
    }
  }

  private visibleViews: NodeView[] = [];
  private screenViews: NodeView[] = [];

  isRaised(id: string): boolean {
    return this.panels.isRaised(id);
  }

  private updateView(now: number): void {
    const z = this.camera.zoom;
    const moving = now - this.lastCameraMove < SETTLE_MS || this.pinching || !!this.flight;
    const view = this.camera.viewRect(0.25);
    const ids = this.store.nodeIndex.queryRect(view);
    const tight = this.camera.viewRect(0.02);
    const prevLod = this.lod;
    const nextLod: Record<string, Lod> = {};
    const views: NodeView[] = [];
    const near: string[] = [];
    const cards: string[] = [];
    const focused = this.selection.focused;
    const raised = new Set(this.panels.raisedIds());
    const faithful = faithfulRendering();
    for (const id of ids) {
      const n = this.store.node(id);
      if (!n || n.stackId || raised.has(id)) continue;
      let lod: Lod = faithful || id === focused ? 'near' : nodeLod(n, z, prevLod[id]);
      const prev = prevLod[id];
      if (moving && prev && isCostlySwap(n.kind, prev, lod)) lod = prev;
      else if (moving && !prev && isCostlySwap(n.kind, undefined, lod)) lod = isTerminalKind(n.kind) ? 'far' : 'mid';
      nextLod[id] = lod;
      const inTight = intersects(tight, n);
      if (inTight) views.push({ id, kind: n.kind, x: n.x, y: n.y, w: n.w, h: n.h, z: n.z, title: n.title, data: n.data, lod });
      if (mountsInWorld(n.kind, lod)) near.push(id);
      else if (lod === 'mid' && inTight) cards.push(id);
    }
    this.visibleViews = views;
    const screen: NodeView[] = [];
    for (const id of raised) {
      const n = this.store.node(id);
      const r = this.panels.rect(id);
      if (!n || !r) continue;
      screen.push({ id, kind: n.kind, x: n.x, y: n.y, w: n.w, h: n.h, z: n.z, title: n.title, data: n.data, lod: 'near', screen: r });
    }
    this.screenViews = screen;

    const nearSet = new Set(near);
    for (const id of near) this.mountedLru.set(id, now);
    for (const id of Array.from(this.mountedLru.keys())) {
      const n = this.store.node(id);
      if (!n || raised.has(id)) this.mountedLru.delete(id);
      else if (!isTerminalKind(n.kind) && !nearSet.has(id)) this.mountedLru.delete(id);
    }
    let nearTerminals = 0;
    for (const id of near) if (isTerminalKind(this.store.node(id)?.kind ?? '')) nearTerminals++;
    const room = Math.max(0, mountedTerminalsLimit() - nearTerminals);
    if (!moving && this.mountedLru.size - near.length > room) {
      const keep = allocate(
        Array.from(this.mountedLru.entries())
          .filter(([id]) => !nearSet.has(id))
          .map(([id, t]) => ({ id, score: t })),
        room,
      );
      for (const id of Array.from(this.mountedLru.keys())) if (!nearSet.has(id) && !keep.has(id)) this.mountedLru.delete(id);
    }
    const mounted = Array.from(this.mountedLru.keys())
      .map((id) => ({ id, hidden: !nearSet.has(id) }))
      .sort((a, b) => (this.store.node(a.id)?.z ?? 0) - (this.store.node(b.id)?.z ?? 0));

    if (!sameRecord(prevLod, nextLod)) this.lod = nextLod;
    if (!sameMounted(this.mounted, mounted)) this.mounted = mounted;
    if (!sameList(this.cards, cards)) this.cards = cards;
  }

  frameInfo(now: number): FrameInfo {
    return {
      camera: { x: this.camera.x, y: this.camera.y, zoom: this.camera.zoom },
      viewport: { w: this.camera.vw, h: this.camera.vh, dpr: this.layer.dpr || 1 },
      visibleNodes: this.visibleViews,
      screenNodes: this.screenViews,
      time: now,
    };
  }

  resize(w: number, h: number, dpr: number): void {
    this.camera.resize(w, h);
    this.layer.resize(w, h, Math.min(dpr, 2));
    this.panels.rev++;
    this.invalidate();
  }

  panBy(dx: number, dy: number): void {
    this.flight = null;
    this.camera.panBy(dx, dy);
    this.invalidate();
  }

  zoomAt(factor: number, sx: number, sy: number): void {
    this.flight = null;
    this.camera.zoomBy(factor, sx, sy, true);
    this.invalidate();
  }

  setZoom(zoom: number, sx?: number, sy?: number): void {
    this.flight = null;
    if (sx === undefined || sy === undefined) {
      const c = this.measureChrome().safeCenter();
      sx = c.x;
      sy = c.y;
    }
    this.camera.zoomTo(clampZoom(zoom), sx, sy);
    this.invalidate();
  }

  flyToCamera(target: CameraState): void {
    this.inertia.stop();
    const f = flyTo(this.camera.state(), target, this.camera.vw, this.camera.vh);
    this.flight = { f, t0: performance.now(), to: { x: target.x, y: target.y, zoom: target.zoom } };
    this.invalidate();
  }

  flyToRect(r: Rect, maxZoom = 1.2): void {
    this.flyToCamera(this.measureChrome().fitRect(r, 64, maxZoom));
  }

  measureChrome(): Camera {
    try {
      this.camera.occluders = (this.chromeProbe?.() ?? []).filter((o) => o.w > 0 && o.h > 0);
    } catch {
      this.camera.occluders = [];
    }
    return this.camera;
  }

  cameraTarget(): CameraState {
    return this.flight ? { ...this.flight.to } : this.camera.state();
  }

  visibleWorld(c: CameraState = this.cameraTarget()): Rect {
    return this.measureChrome().visibleWorld(c);
  }

  contentBounds(): Rect | null {
    const rects: Rect[] = this.store.nodes.filter((n) => !n.stackId).map((n) => ({ x: n.x, y: n.y, w: n.w, h: n.h }));
    for (const d of this.store.drawings) rects.push(drawingBounds(d));
    return union(rects);
  }

  fitAll(animate = true): void {
    const b = this.contentBounds();
    this.measureChrome();
    if (!b) {
      const c = this.camera.safeCenter();
      const target = { x: -c.x, y: -c.y, zoom: 1 };
      if (animate) this.flyToCamera(target);
      else this.camera.set(target);
      this.invalidate();
      return;
    }
    const cam = this.camera.fitRect(b, 64, 1);
    if (animate) this.flyToCamera(cam);
    else {
      this.camera.set(cam);
      this.invalidate();
    }
  }

  selectionBounds(): Rect | null {
    const rects: Rect[] = [];
    for (const id of this.selection.ids) {
      const n = this.store.node(id);
      if (n) rects.push(n);
      const d = this.store.drawing(id);
      if (d) rects.push(drawingBounds(d));
    }
    return union(rects);
  }

  zoomToggle(): void {
    if (this.prevCamera) {
      const back = this.prevCamera;
      this.prevCamera = null;
      this.flyToCamera(back);
      return;
    }
    const b = this.selectionBounds();
    if (!b) return this.fitAll();
    this.prevCamera = this.camera.state();
    this.flyToRect(b, 1.5);
  }

  reveal(id: string): void {
    const n = this.store.node(id);
    if (!n) return;
    const v = this.visibleWorld(this.camera.state());
    const view = { x: v.x + v.w * 0.05, y: v.y + v.h * 0.05, w: v.w * 0.9, h: v.h * 0.9 };
    const inside = n.x >= view.x && n.y >= view.y && n.x + n.w <= view.x + view.w && n.y + n.h <= view.y + view.h;
    if (!inside) this.flyToRect(n, Math.max(this.camera.zoom, 0.6));
  }

  commit(ops: Op[], opts: { origin?: Origin; history?: boolean; sync?: boolean; selectionAfter?: string[] } = {}): void {
    if (ops.length === 0) return;
    const origin = opts.origin ?? 'user';
    const before = this.selection.list();
    const inverse = this.store.apply(ops, origin);
    if (opts.selectionAfter) this.selection.set(opts.selectionAfter);
    if (opts.history !== false) this.history.record(origin, ops, inverse, before, opts.selectionAfter ?? this.selection.list());
    if (opts.sync !== false) void this.sync.push(ops, origin);
    this.invalidate();
  }

  recordApplied(forward: Op[], inverse: Op[]): void {
    if (forward.length === 0) return;
    const sel = this.selection.list();
    this.history.record('user', forward, inverse, sel, sel);
    void this.sync.push(forward, 'user');
    this.invalidate();
  }

  undo(): void {
    const step = this.history.undo();
    if (!step) return;
    this.applyStep(step);
  }

  redo(): void {
    const step = this.history.redo();
    if (!step) return;
    this.applyStep(step);
  }

  private applyStep(step: { ops: Op[]; skipped: string[]; selection?: string[] }): void {
    this.store.apply(step.ops, 'user');
    void this.sync.push(step.ops, 'user');
    if (step.selection) this.selection.set(step.selection.filter((id) => this.exists(id)));
    else this.selection.prune((id) => this.exists(id));
    if (step.skipped.length) this.say('canvas.engine.undoSkipped');
    this.invalidate();
  }

  exists(id: string): boolean {
    return !!this.store.node(id) || !!this.store.drawing(id) || !!this.store.connection(id);
  }

  say(key: string): void {
    this.notice = key;
    if (this.noticeTimer) clearTimeout(this.noticeTimer);
    this.noticeTimer = setTimeout(() => (this.notice = null), 3200);
  }

  uniqueTitle(base: string): string {
    const taken = new Set(this.store.nodes.map((n) => n.title));
    if (!taken.has(base)) return base;
    for (let i = 2; i < 1000; i++) if (!taken.has(`${base} ${i}`)) return `${base} ${i}`;
    return `${base} ${Date.now()}`;
  }

  spawn(kind: string, at: { x: number; y: number } | null, opts: SpawnOptions = {}): CanvasNode {
    const preset = kind === 'terminal' || kind === 'agent' ? userPreset(opts.preset) : null;
    const agent = preset ? presetAgent(preset) : (opts.agent ?? (kind === 'terminal' ? 'shell' : undefined));
    const nodeKindName = preset || (agent && agent !== 'shell') ? 'agent' : kind;
    const size = opts.rect ? { w: opts.rect.w, h: opts.rect.h } : (prefSize(nodeKindName) ?? sizeFor(nodeKindName));
    let x: number;
    let y: number;
    if (opts.rect) {
      x = opts.rect.x;
      y = opts.rect.y;
    } else if (at) {
      x = snap(at.x);
      y = snap(at.y);
    } else {
      const c = this.camera.center();
      const spot = nearestFreeSpot(c.x - size.w / 2, c.y - size.h / 2, size.w, size.h, this.store.nodes);
      x = spot.x;
      y = spot.y;
    }
    const data: Record<string, unknown> = {
      ...(preset && opts.preset ? presetData(opts.preset, preset) : {}),
      ...(opts.data ?? {}),
    };
    let title = opts.title ?? (preset && opts.preset ? this.uniqueTitle(opts.preset) : undefined);
    if (isTerminalKind(nodeKindName)) {
      data.terminalId = (data.terminalId as string) ?? newId();
      data.agent = agent ?? 'shell';
      title = title ?? this.uniqueTitle(AGENT_TITLES[agent ?? 'shell'] ?? 'Terminal');
    } else if (kind === 'note') {
      data.text = data.text ?? '';
      const nd = canvasPrefs().defaults.note;
      if (nd.font && data.font === undefined) data.font = nd.font;
      if (nd.render === 'raw' && data.mode === undefined) data.mode = 'raw';
      title = title ?? this.uniqueTitle('Note');
    } else if (kind === 'text') {
      data.text = data.text ?? '';
    } else if (kind === 'portal') {
      data.portalKind = data.portalKind ?? 'web';
      title = title ?? this.uniqueTitle('Portal');
    } else if (kind === 'filetree') {
      data.rootPath =
        data.rootPath ||
        workspaces.byId(this.store.workspaceId)?.workingDirectory ||
        this.sync.workspaces.find((w) => w.id === this.store.workspaceId)?.workingDirectory ||
        this.homeDir ||
        '/';
      title = title ?? this.uniqueTitle('Files');
    } else if (kind === 'link') {
      data.url = data.url ?? '';
    } else if (kind === 'file') {
      data.path = data.path ?? '';
    }
    const node: CanvasNode = {
      id: newId(),
      kind: nodeKindName,
      x: snap(x),
      y: snap(y),
      w: snap(size.w),
      h: snap(size.h),
      z: this.store.maxZ() + 1,
      data,
    };
    if (title) node.title = title;
    if (preset?.color) node.color = preset.color;
    if (preset?.icon) node.icon = preset.icon;
    const ops: Op[] = [{ op: 'add_node', node }];
    if (opts.connectFrom && this.store.node(opts.connectFrom) && !cordRefusal(this.store.node(opts.connectFrom), node)) {
      ops.push({ op: 'add_connection', connection: this.newConnection(opts.connectFrom, node.id) });
    }
    this.commit(ops, { selectionAfter: opts.select === false ? undefined : [node.id] });
    if (isTerminalKind(node.kind)) {
      void openTerminal(node, this.store.workspaceId, { floorId: this.store.floorId }).catch(() =>
        this.say('canvas.engine.terminalFailed'),
      );
    }
    return node;
  }

  newConnection(from: string, to: string): Connection {
    const a = this.store.node(from);
    const b = this.store.node(to);
    let kind: Connection['kind'] = 'ask';
    if (a?.kind === 'note' || b?.kind === 'note') kind = 'note';
    else if (a?.kind === 'portal' || b?.kind === 'portal') kind = 'portal';
    return { id: newId(), from, to, kind, perms: kind === 'ask' ? ['ask', 'check'] : [] };
  }

  connect(from: string, to: string): void {
    if (from === to) return;
    const refusal = cordRefusal(this.store.node(from), this.store.node(to));
    if (refusal) return this.say(refusal);
    const exists = this.store.connections.some(
      (c) => (c.from === from && c.to === to) || (c.from === to && c.to === from),
    );
    if (exists) return this.say('canvas.engine.alreadyConnected');
    this.commit([{ op: 'add_connection', connection: this.newConnection(from, to) }]);
  }

  updateNode(id: string, patch: Partial<CanvasNode>): void {
    if (!this.store.node(id)) return;
    this.commit([{ op: 'update_node', id, patch }]);
  }

  deleteSelection(): void {
    const ids = this.selection.list();
    const nodeIds = ids.filter((id) => this.store.node(id));
    const ops: Op[] = [];
    for (const id of ids) if (this.store.connection(id)) ops.push({ op: 'remove_connection', id });
    ops.push(...removeNodesOps(this.store.doc, nodeIds).filter((o) => !(o.op === 'remove_connection' && ids.includes(o.id))));
    for (const id of ids) if (this.store.drawing(id)) ops.push({ op: 'remove_drawing', id });
    if (ops.length === 0) return;
    for (const id of nodeIds) {
      const n = this.store.node(id);
      if (n && isTerminalKind(n.kind)) this.scheduleKill(terminalIdOf(n));
    }
    if (this.selection.focused && nodeIds.includes(this.selection.focused)) this.selection.focus(null);
    this.commit(ops, { selectionAfter: [] });
    this.dissolveSmallGroups();
  }

  private scheduleKill(terminalId: string): void {
    this.cancelKill(terminalId);
    void closeTerminal(terminalId, false);
    this.pendingKills.set(
      terminalId,
      setTimeout(() => {
        this.pendingKills.delete(terminalId);
        void closeTerminal(terminalId);
        agentStates.forget(terminalId);
      }, SOFT_DELETE_MS),
    );
  }

  private cancelKill(terminalId: string): void {
    const t = this.pendingKills.get(terminalId);
    if (t) {
      clearTimeout(t);
      this.pendingKills.delete(terminalId);
    }
  }

  duplicateSelection(): void {
    const ids = this.selection.list();
    const ops: Op[] = [];
    const created: string[] = [];
    const openAfter: CanvasNode[] = [];
    let z = this.store.maxZ();
    for (const id of ids) {
      const n = this.store.node(id);
      if (n) {
        const copy: CanvasNode = { ...n, id: newId(), x: n.x + GRID * 2, y: n.y + GRID * 2, z: ++z, data: { ...n.data } };
        delete copy.groupId;
        if (isTerminalKind(n.kind)) {
          copy.data.terminalId = newId();
          copy.title = this.uniqueTitle(n.title ?? 'Terminal');
          openAfter.push(copy);
        } else if (n.title) {
          copy.title = this.uniqueTitle(n.title);
        }
        ops.push({ op: 'add_node', node: copy });
        created.push(copy.id);
      }
      const d = this.store.drawing(id);
      if (d) {
        const copy: Drawing = { ...d, id: newId(), x: d.x + GRID * 2, y: d.y + GRID * 2, z: this.store.maxDrawingZ() + 1 };
        delete copy.groupId;
        ops.push({ op: 'add_drawing', drawing: copy });
        created.push(copy.id);
      }
    }
    this.commit(ops, { selectionAfter: created });
    for (const n of openAfter) {
      void openTerminal(n, this.store.workspaceId, { floorId: this.store.floorId }).catch(() =>
        this.say('canvas.engine.terminalFailed'),
      );
    }
  }

  toggleLock(): void {
    const nodes = this.selection.list().map((id) => this.store.node(id)).filter((n): n is CanvasNode => !!n);
    if (!nodes.length) return;
    const lock = !nodes.every((n) => n.locked);
    this.commit(nodes.map((n) => ({ op: 'update_node' as const, id: n.id, patch: { locked: lock ? true : undefined } })));
  }

  zOrder(mode: 'raise' | 'lower' | 'front' | 'back'): void {
    const sel = this.selection.list().map((id) => this.store.node(id)).filter((n): n is CanvasNode => !!n);
    if (!sel.length) return;
    const ops: Op[] = [];
    if (mode === 'front') {
      let z = this.store.maxZ();
      for (const n of sel.sort((a, b) => a.z - b.z)) ops.push({ op: 'update_node', id: n.id, patch: { z: ++z } });
    } else if (mode === 'back') {
      let z = this.store.minZ();
      for (const n of sel.sort((a, b) => b.z - a.z)) ops.push({ op: 'update_node', id: n.id, patch: { z: --z } });
    } else {
      const ids = new Set(sel.map((n) => n.id));
      for (const n of sel) {
        const others = this.store.nodes.filter((o) => !ids.has(o.id) && intersects(o, n));
        const cand =
          mode === 'raise'
            ? others.filter((o) => o.z >= n.z).sort((a, b) => a.z - b.z)[0]
            : others.filter((o) => o.z <= n.z).sort((a, b) => b.z - a.z)[0];
        if (!cand) continue;
        ops.push({ op: 'update_node', id: n.id, patch: { z: cand.z } }, { op: 'update_node', id: cand.id, patch: { z: n.z } });
      }
    }
    this.commit(ops);
  }

  group(): void {
    const nodes = this.selection.list().map((id) => this.store.node(id)).filter((n): n is CanvasNode => !!n);
    const drawings = this.selection.list().map((id) => this.store.drawing(id)).filter((d): d is Drawing => !!d);
    const members: Array<{ id: string; groupId?: string; isNode: boolean }> = nodes.length
      ? nodes.map((n) => ({ id: n.id, groupId: n.groupId, isNode: true }))
      : drawings.map((d) => ({ id: d.id, groupId: d.groupId, isNode: false }));
    if (members.length < 2) return;
    const existing = members.find((m) => m.groupId && this.store.group(m.groupId))?.groupId;
    const groupId = existing ?? newId();
    const ops: Op[] = [];
    if (!existing) {
      ops.push({
        op: 'set_group',
        id: groupId,
        group: { id: groupId, name: 'Group', color: '#7c8cff', target: nodes.length ? 'nodes' : 'drawings' },
      });
    }
    for (const m of members) {
      if (m.groupId === groupId) continue;
      if (m.isNode) ops.push({ op: 'update_node', id: m.id, patch: { groupId } });
      else {
        ops.push({ op: 'update_drawing', id: m.id, patch: { groupId } });
      }
    }
    this.commit(ops);
  }

  ungroup(): void {
    const ops: Op[] = [];
    const touched = new Set<string>();
    for (const id of this.selection.list()) {
      const n = this.store.node(id);
      if (n?.groupId) {
        touched.add(n.groupId);
        ops.push({ op: 'update_node', id, patch: { groupId: undefined } });
      }
      const d = this.store.drawing(id);
      if (d?.groupId) {
        touched.add(d.groupId);
        ops.push({ op: 'update_drawing', id, patch: { groupId: undefined } });
      }
    }
    for (const id of this.selection.list()) {
      if (this.store.group(id)) {
        for (const m of this.store.groupMembers(id)) ops.push({ op: 'update_node', id: m.id, patch: { groupId: undefined } });
        ops.push({ op: 'set_group', id, group: null });
      }
    }
    this.commit(ops);
    this.dissolveSmallGroups();
  }

  dissolveSmallGroups(): void {
    const ops: Op[] = [];
    for (const g of this.store.groups) {
      const members = [...this.store.groupMembers(g.id), ...this.store.drawingGroupMembers(g.id)];
      if (members.length >= 2) continue;
      for (const m of this.store.groupMembers(g.id)) ops.push({ op: 'update_node', id: m.id, patch: { groupId: undefined } });
      for (const d of this.store.drawingGroupMembers(g.id)) {
        ops.push({ op: 'update_drawing', id: d.id, patch: { groupId: undefined } });
      }
      ops.push({ op: 'set_group', id: g.id, group: null });
    }
    if (ops.length) this.commit(ops);
  }

  renameGroup(id: string, name: string): void {
    const g = this.store.group(id);
    if (!g) return;
    this.commit([{ op: 'set_group', id, group: { ...g, name } }]);
  }

  groupBounds(id: string): Rect | null {
    const rects: Rect[] = this.store.groupMembers(id).map((n) => ({ x: n.x, y: n.y, w: n.w, h: n.h }));
    for (const d of this.store.drawingGroupMembers(id)) rects.push(drawingBounds(d));
    const u = union(rects);
    if (!u) return null;
    const pad = GRID;
    return { x: u.x - pad, y: u.y - pad - GRID, w: u.w + pad * 2, h: u.h + pad * 2 + GRID };
  }

  align(mode: AlignMode): void {
    const items = this.selection
      .list()
      .map((id) => this.store.node(id))
      .filter((n): n is CanvasNode => !!n && !n.locked)
      .map((n) => ({ id: n.id, x: n.x, y: n.y, w: n.w, h: n.h }));
    const pos = arrange(items, mode);
    const ops: Op[] = [];
    for (const [id, p] of pos) ops.push({ op: 'update_node', id, patch: { x: p.x, y: p.y } });
    this.commit(ops);
  }

  nudge(dx: number, dy: number): void {
    const ops: Op[] = [];
    for (const id of this.selection.list()) {
      const n = this.store.node(id);
      if (n && !n.locked) ops.push({ op: 'update_node', id, patch: { x: n.x + dx, y: n.y + dy } });
      const d = this.store.drawing(id);
      if (d) ops.push({ op: 'update_drawing', id, patch: { x: d.x + dx, y: d.y + dy } });
    }
    this.commit(ops);
  }

  guidesFor(moving: Rect, exclude: Set<string>): { dx: number; dy: number; guides: Guide[] } {
    const tol = 6 / this.camera.zoom;
    const near = this.store.nodeIndex
      .queryRect({ x: moving.x - 400, y: moving.y - 400, w: moving.w + 800, h: moving.h + 800 })
      .filter((id) => !exclude.has(id))
      .map((id) => this.store.node(id)!)
      .filter(Boolean);
    return alignGuides(moving, near, tol);
  }

  numbered(): CanvasNode[] {
    return this.store.nodes
      .filter((n) => isTerminalKind(n.kind))
      .sort((a, b) => Math.round(a.y / 200) - Math.round(b.y / 200) || a.x - b.x);
  }

  jump(index: number): void {
    if (this.panels.liftIndex(index)) return;
    const n = this.numbered()[index - 1];
    if (!n) return;
    this.selection.set([n.id]);
    this.flyToRect(n, Math.max(1, this.camera.zoom));
    this.selection.focus(n.id);
  }

  private attentionLast: string | null = null;

  async nextAttention(): Promise<void> {
    type Item = { nodeId: string; workspaceId: string; floorId: string; reason: string; sinceMs?: number };
    const inbox = (await tryInvoke<Item[]>('canvas_inbox', { workspaceId: null })) ?? [];
    const rank = (r: string) => (r === 'permission' || r === 'blocked' || r === 'stuck' ? 0 : 1);
    const here = this.store.workspaceId;
    const items: Item[] = inbox
      .filter((i) => i && i.nodeId && i.workspaceId)
      .map((i) => ({ ...i, floorId: i.floorId || 'ground' }));
    for (const n of this.store.nodes) {
      if (!isTerminalKind(n.kind) || agentStates.stateOf(n) !== 'blocked') continue;
      if (!items.some((i) => i.nodeId === n.id)) {
        items.push({ nodeId: n.id, workspaceId: here, floorId: this.store.floorId, reason: 'blocked' });
      }
    }
    if (!items.length) return this.say('canvas.engine.nobodyBlocked');
    items.sort(
      (a, b) =>
        rank(a.reason) - rank(b.reason) ||
        Number(b.workspaceId === here) - Number(a.workspaceId === here) ||
        (a.sinceMs ?? 0) - (b.sinceMs ?? 0) ||
        a.nodeId.localeCompare(b.nodeId),
    );
    const current = this.attentionLast ?? this.selection.focused ?? this.selection.list()[0] ?? null;
    const idx = current ? items.findIndex((i) => i.nodeId === current) : -1;
    const next = items[(idx + 1) % items.length];
    this.attentionLast = next.nodeId;
    if (next.workspaceId !== this.store.workspaceId || next.floorId !== this.store.floorId) {
      const { goTo } = await import('./palette/navigate');
      await goTo(this, { workspaceId: next.workspaceId, floorId: next.floorId, nodeId: next.nodeId }, { focus: true });
      return;
    }
    const n = this.store.node(next.nodeId);
    if (!n) return;
    this.selection.set([n.id]);
    this.flyToRect(n, Math.max(1, this.camera.zoom));
    this.selection.focus(n.id);
  }

  private cordWalkState: {
    anchor: string;
    targets: Array<{ nodeId: string; workspaceId: string; floorId: string }>;
    index: number;
  } | null = null;

  async walkCord(delta: number): Promise<void> {
    const sel = this.selection.focused ?? this.selection.list().find((id) => this.store.node(id)) ?? null;
    let st = this.cordWalkState;
    const onWalk = st && sel && st.targets[st.index]?.nodeId === sel;
    if (!st || !onWalk) {
      if (!sel || !this.store.node(sel)) return;
      const ws = this.store.workspaceId;
      const floor = this.store.floorId;
      const seen = new Set<string>();
      const local: CanvasNode[] = [];
      for (const c of this.store.connections) {
        const other = c.from === sel ? c.to : c.to === sel ? c.from : null;
        if (!other || seen.has(other)) continue;
        const n = this.store.node(other);
        if (!n) continue;
        seen.add(other);
        local.push(n);
      }
      local.sort((a, b) => Math.round(a.y / 200) - Math.round(b.y / 200) || a.x - b.x || a.id.localeCompare(b.id));
      const targets = local.map((n) => {
        const d = n.data as Record<string, unknown>;
        if (n.kind === 'connector' && typeof d.remoteWorkspaceId === 'string' && typeof d.remoteNodeId === 'string') {
          return {
            nodeId: d.remoteNodeId,
            workspaceId: d.remoteWorkspaceId,
            floorId: typeof d.remoteFloorId === 'string' && d.remoteFloorId ? d.remoteFloorId : 'ground',
          };
        }
        return { nodeId: n.id, workspaceId: ws, floorId: floor };
      });
      if (!targets.length) return this.say('canvas.engine.noCords');
      st = { anchor: sel, targets, index: delta > 0 ? -1 : 0 };
      this.cordWalkState = st;
    }
    const n = st.targets.length;
    st.index = (((st.index + delta) % n) + n) % n;
    const t = st.targets[st.index];
    const edit = (id: string) => {
      const node = this.store.node(id);
      return !!node && (node.kind === 'note' || node.kind === 'text' || isTerminalKind(node.kind));
    };
    if (t.workspaceId !== this.store.workspaceId || t.floorId !== this.store.floorId) {
      const { goTo } = await import('./palette/navigate');
      await goTo(this, t, { focus: true });
      return;
    }
    const node = this.store.node(t.nodeId);
    if (!node) return;
    this.selection.set([node.id]);
    this.reveal(node.id);
    this.selection.focus(edit(node.id) ? node.id : null);
    this.invalidate();
  }

  focusDir(dir: Dir): void {
    const cur = this.store.node(this.selection.focused ?? this.selection.list().at(-1) ?? '');
    if (!cur) return;
    const cands = this.store.nodes.filter((n) => n.id !== cur.id).map((n) => ({ id: n.id, x: n.x, y: n.y, w: n.w, h: n.h }));
    const next = neighborInDirection(cur, cands, dir);
    if (!next) return;
    const wasFocused = this.selection.focused !== null;
    this.selection.set([next]);
    this.selection.focus(wasFocused ? next : null);
    this.reveal(next);
  }

  split(dir: 'right' | 'down'): void {
    const cur = this.store.node(this.selection.focused ?? this.selection.list().at(-1) ?? '');
    const agent = (cur?.data.agent as AgentKind | undefined) ?? 'shell';
    const size = sizeFor(agent === 'shell' ? 'terminal' : 'agent');
    let at: { x: number; y: number } | null = null;
    if (cur) {
      const want = dir === 'right' ? { x: cur.x + cur.w + GRID * 2, y: cur.y } : { x: cur.x, y: cur.y + cur.h + GRID * 2 };
      at = nearestFreeSpot(want.x, want.y, size.w, size.h, this.store.nodes);
    }
    const node = this.spawn(agent === 'shell' ? 'terminal' : 'agent', at, {
      agent,
      connectFrom: cur && this.splitConnects && isTerminalKind(cur.kind) ? cur.id : undefined,
      data: cur?.data.cwd ? { cwd: cur.data.cwd } : undefined,
    });
    this.reveal(node.id);
    this.selection.focus(node.id);
  }

  requestRestart(nodeId: string): void {
    this.restarts = { ...this.restarts, [nodeId]: (this.restarts[nodeId] ?? 0) + 1 };
  }

  enterSelected(): void {
    const id = this.selection.list().at(-1);
    if (!id || !this.store.node(id)) return;
    this.reveal(id);
    this.selection.focus(id);
    this.invalidate();
  }

  exitFocus(): void {
    this.selection.focus(null);
    const a = typeof document !== 'undefined' ? (document.activeElement as HTMLElement | null) : null;
    if (a && this.rootEl?.contains(a)) a.blur();
    this.rootEl?.focus({ preventScroll: true });
    this.invalidate();
  }

  setTool(tool: ToolName): void {
    this.tool = tool;
    if (tool !== 'create') {
      this.createKind = null;
      this.createAgent = null;
    }
    this.invalidate();
  }

  armCreate(kind: string, agent?: AgentKind, preset?: string): void {
    this.tool = 'create';
    this.createKind = kind;
    this.createAgent = agent ?? null;
    this.createPreset = preset ?? null;
  }

  resolveNode(ref: string): CanvasNode | undefined {
    const direct = this.store.node(ref);
    if (direct) return direct;
    return this.store.nodes.find((n) => n.data.terminalId === ref) ?? this.store.nodes.find((n) => n.title === ref);
  }

  private onMessage(p: MessagePayload): void {
    if (!p || !p.from) return;
    const state = (p.state ?? '').toLowerCase();
    const answered = state === 'answered' || state === 'replied';
    if (state && !answered && !['queued', 'sent', 'delivering', 'delivered', 'pending'].includes(state)) return;
    if (p.kind === 'reply' && p.askId) p = { ...p, id: `${p.askId}:answer` };
    const src = p.from;
    if (!src) return;
    const from = this.resolveNode(answered ? p.to : src);
    const to = this.resolveNode(answered ? src : p.to);
    if (!from || !to || from.id === to.id) return;
    const conn = this.store.connections.find(
      (c) => (c.from === from.id && c.to === to.id) || (c.from === to.id && c.to === from.id),
    );
    let cord = conn
      ? sampleCord(cordPath(this.store.node(conn.from)!, this.store.node(conn.to)!))
      : sampleCord(cordPath(from, to));
    if (conn && conn.from !== from.id) cord = reverseCord(cord);
    const kind = answered ? 'reply' : (p.kind ?? 'message');
    this.traffic.launch(
      {
        id: `${p.id ?? `${from.id}>${to.id}:${performance.now()}`}${answered && !p.id?.endsWith(':answer') ? ':answer' : ''}`,
        kind,
        color: kindColor(kind),
        pts: cord.pts,
        lens: cord.lens,
        connectionId: conn?.id,
        toNode: to.id,
      },
      performance.now(),
      this.camera.zoom,
    );
    this.invalidate();
  }
}

function sameRecord(a: Record<string, Lod>, b: Record<string, Lod>): boolean {
  const ka = Object.keys(a);
  if (ka.length !== Object.keys(b).length) return false;
  for (const k of ka) if (a[k] !== b[k]) return false;
  return true;
}

function sameList(a: string[], b: string[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

function sameMounted(a: Array<{ id: string; hidden: boolean }>, b: Array<{ id: string; hidden: boolean }>): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i].id !== b[i].id || a[i].hidden !== b[i].hidden) return false;
  return true;
}

export function provideEngine(engine: CanvasEngine): void {
  setContext(ENGINE_KEY, engine);
}

export function useEngine(): CanvasEngine {
  const e = getContext<CanvasEngine | undefined>(ENGINE_KEY);
  if (!e) throw new Error('canvas engine missing from context');
  return e;
}

async function tryInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T | null> {
  try {
    return await invoke<T>(cmd, args);
  } catch {
    return null;
  }
}

function readFlag(key: string, fallback: boolean): boolean {
  try {
    const v = localStorage.getItem(key);
    return v === null ? fallback : v === '1';
  } catch {
    return fallback;
  }
}
