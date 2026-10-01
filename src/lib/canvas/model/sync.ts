import { markWorkspaceOpen } from '../session';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { recordKey } from './ops';
import type { CanvasStore } from './store.svelte';
import type { ApplyResult, CameraState, Op, Origin, PatchEvent, Snapshot, Workspace } from './types';
import { applyWireOps, drawingFromWire, opsToWire, type WireDrawing } from './wire';

export type SyncMode = 'connecting' | 'rust' | 'memory';

const PATCH_EVENT = 'canvas://patch';
const LOCAL_WORKSPACE = 'local';
const LOCAL_FLOOR = 'ground';
const CAMERA_DEBOUNCE_MS = 2000;

function emptySnapshot(workspaceId: string, floorId: string): Snapshot {
  return { workspaceId, floorId, epoch: 0, rev: 0, nodes: [], connections: [], drawings: [], groups: [] };
}

function normaliseSnapshot(raw: unknown, workspaceId: string, floorId: string): Snapshot {
  const s = (raw ?? {}) as Record<string, unknown>;
  const arr = <T>(v: unknown): T[] => (Array.isArray(v) ? (v as T[]) : []);
  return {
    workspaceId: (s.workspaceId as string) ?? workspaceId,
    floorId: (s.floorId as string) ?? floorId,
    epoch: (s.epoch as string | number) ?? 0,
    rev: typeof s.rev === 'number' ? s.rev : 0,
    seq: typeof s.seq === 'number' ? s.seq : undefined,
    camera: s.camera as Snapshot['camera'],
    nodes: arr<Snapshot['nodes'][number]>(s.nodes).map((n) => ({ ...n, z: n.z ?? 0, data: n.data ?? {} })),
    connections: arr<Snapshot['connections'][number]>(s.connections).map((c) => ({ ...c, perms: c.perms ?? [] })),
    drawings: arr<WireDrawing>(s.drawings).map(drawingFromWire),
    groups: arr<Snapshot['groups'][number]>(s.groups),
  };
}

export class CanvasSync {
  mode: SyncMode = 'connecting';
  workspaces: Workspace[] = [];
  lastError: string | null = null;
  onModeChange?: (mode: SyncMode) => void;
  onRemote?: (ops: Op[], origin: Origin) => void;

  private unlisten: UnlistenFn | null = null;
  private lastSeq = 0;
  private mySeqs = new Set<number>();
  private inFlight = 0;
  private deferred: PatchEvent[] = [];
  private queue: Promise<unknown> = Promise.resolve();
  private cameraTimer: ReturnType<typeof setTimeout> | null = null;
  private pendingCamera: CameraState | null = null;
  private disposed = false;

  constructor(private readonly store: CanvasStore) {}

  private setMode(mode: SyncMode, err?: unknown): void {
    if (err !== undefined) this.lastError = err instanceof Error ? err.message : String(err);
    if (this.mode === mode) return;
    this.mode = mode;
    this.onModeChange?.(mode);
  }

  async open(workspaceId?: string, floorId?: string): Promise<void> {
    try {
      const list = await invoke<Workspace[]>('canvas_workspaces');
      this.workspaces = Array.isArray(list) ? list : [];
      let ws = this.workspaces.find((w) => w.id === workspaceId) ?? this.workspaces[0];
      if (!ws) {
        ws = await invoke<Workspace>('canvas_workspace_create', { input: { name: 'Home' } });
        this.workspaces = [ws];
      }
      const floor = floorId ?? ws.activeFloorId ?? ws.floors?.[0]?.id ?? LOCAL_FLOOR;
      await this.listenPatches();
      const snap = await invoke<unknown>('canvas_load', { workspaceId: ws.id, floorId: floor });
      if (this.disposed) return;
      const norm = normaliseSnapshot(snap, ws.id, floor);
      this.lastSeq = norm.seq ?? 0;
      this.store.load(norm);
      this.setMode('rust');
      markWorkspaceOpen(ws.id);
    } catch (err) {
      if (this.disposed) return;
      this.store.load(emptySnapshot(workspaceId ?? LOCAL_WORKSPACE, floorId ?? LOCAL_FLOOR));
      this.setMode('memory', err);
    }
  }

  private async listenPatches(): Promise<void> {
    if (this.unlisten) return;
    this.unlisten = await listen<PatchEvent>(PATCH_EVENT, (e) => this.onPatch(e.payload));
    if (this.disposed) this.unlisten();
  }

  private onPatch(p: PatchEvent): void {
    if (this.mode !== 'rust') return;
    if (!p || p.workspaceId !== this.store.workspaceId || p.floorId !== this.store.floorId) return;
    if (this.inFlight > 0) {
      this.deferred.push(p);
      return;
    }
    this.consume(p);
  }

  private consume(p: PatchEvent): void {
    if (String(p.epoch) !== String(this.store.epoch)) {
      void this.reload();
      return;
    }
    if (this.mySeqs.delete(p.seq)) {
      this.lastSeq = Math.max(this.lastSeq, p.seq);
      if (typeof p.rev === 'number') this.store.rev = Math.max(this.store.rev, p.rev);
      return;
    }
    if (p.seq <= this.lastSeq) return;
    if (p.seq > this.lastSeq + 1) {
      void this.reload();
      return;
    }
    this.lastSeq = p.seq;
    if (typeof p.rev === 'number') this.store.rev = p.rev;
    const applied = applyWireOps(p.ops as unknown[], this.store, p.origin);
    this.onRemote?.(applied, p.origin);
  }

  private flushDeferred(): void {
    if (this.inFlight > 0) return;
    const list = this.deferred.sort((a, b) => a.seq - b.seq);
    this.deferred = [];
    for (const p of list) this.consume(p);
  }

  async reload(): Promise<void> {
    if (this.mode !== 'rust') return;
    try {
      const snap = await invoke<unknown>('canvas_load', {
        workspaceId: this.store.workspaceId,
        floorId: this.store.floorId,
      });
      const norm = normaliseSnapshot(snap, this.store.workspaceId, this.store.floorId);
      this.lastSeq = norm.seq ?? 0;
      this.mySeqs.clear();
      this.deferred = [];
      this.store.load(norm);
      this.onRemote?.([], 'load');
    } catch (err) {
      this.lastError = err instanceof Error ? err.message : String(err);
    }
  }

  push(ops: Op[], origin: Origin = 'user'): Promise<void> {
    if (ops.length === 0 || this.mode !== 'rust') return Promise.resolve();
    const wire = opsToWire(ops, this.store);
    if (wire.length === 0) return Promise.resolve();
    const mine = new Set(ops.map(recordKey));
    this.inFlight++;
    const run = async () => {
      try {
        const res = await invoke<ApplyResult>('canvas_apply', {
          workspaceId: this.store.workspaceId,
          floorId: this.store.floorId,
          baseRev: this.store.rev,
          ops: wire,
          origin,
        });
        if (res && typeof res.rev === 'number') this.store.rev = res.rev;
        if (res && typeof res.seq === 'number' && res.seq > this.lastSeq) this.mySeqs.add(res.seq);
        if (res?.ops?.length) {
          const extra = (res.ops as Array<Record<string, unknown>>).filter((w) => {
            const key = wireKey(w);
            return !key || !mine.has(key) || w.op === 'add_node';
          });
          if (extra.length) {
            const applied = applyWireOps(extra, this.store, origin);
            if (applied.length) this.onRemote?.(applied, 'normalise');
          }
        }
      } catch (err) {
        this.lastError = err instanceof Error ? err.message : String(err);
        if (/CANVAS_CONFLICT|CANVAS_NOT_FOUND/.test(this.lastError)) void this.reload();
      } finally {
        this.inFlight--;
        this.flushDeferred();
      }
    };
    const p = this.queue.then(run);
    this.queue = p;
    return p;
  }

  saveCamera(camera: CameraState): void {
    this.pendingCamera = { x: camera.x, y: camera.y, zoom: camera.zoom };
    if (this.cameraTimer) clearTimeout(this.cameraTimer);
    this.cameraTimer = setTimeout(() => {
      this.cameraTimer = null;
      this.sendCamera();
    }, CAMERA_DEBOUNCE_MS);
  }

  private sendCamera(): void {
    const cam = this.pendingCamera;
    this.pendingCamera = null;
    if (!cam) return;
    const ops: Op[] = [{ op: 'set_camera', camera: cam }];
    this.store.apply(ops, 'user');
    void this.push(ops, 'user');
  }

  flush(): void {
    if (this.cameraTimer) {
      clearTimeout(this.cameraTimer);
      this.cameraTimer = null;
      this.sendCamera();
    }
    if (this.mode === 'rust') {
      void this.queue.then(() => invoke('canvas_flush').catch(() => {}));
    }
  }

  dispose(): void {
    this.flush();
    this.disposed = true;
    this.unlisten?.();
    this.unlisten = null;
  }
}

function wireKey(w: Record<string, unknown>): string | null {
  const id = (w.id as string | undefined) ?? undefined;
  switch (w.op) {
    case 'add_node':
      return `n:${(w.node as { id: string }).id}`;
    case 'update_node':
    case 'remove_node':
      return `n:${id}`;
    case 'add_connection':
      return `c:${(w.connection as { id: string }).id}`;
    case 'update_connection':
    case 'remove_connection':
      return `c:${id}`;
    case 'add_drawing':
      return `d:${(w.drawing as { id: string }).id}`;
    case 'update_drawing':
    case 'remove_drawing':
      return `d:${id}`;
    case 'set_group':
      return `g:${id}`;
    case 'set_camera':
      return 'camera';
    default:
      return null;
  }
}
