import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import type { CanvasEngine } from './engine.svelte';
import type { CanvasNode } from './model/types';
import { canvasPrefs, imageBudgetBytes } from './prefs.svelte';
import { clearSnapshot, setSnapshot } from './render/snapshot-textures';

export interface ImageResolved {
  asset?: string | null;
  path?: string | null;
  original: string;
  thumb: string;
  w: number;
  h: number;
  bytes?: number | null;
  imported: boolean;
  sourceUrl?: string | null;
}

type Status = 'loading' | 'ready' | 'error';

interface Entry {
  key: string;
  status: Status;
  resolved?: ImageResolved;
  bitmap?: ImageBitmap;
  bitmapLoading?: boolean;
  error?: string;
  bytes: number;
  used: number;
  version: number;
}

type NodeLike = Pick<CanvasNode, 'id' | 'kind' | 'data'>;

const RASTER = /\.(png|jpe?g|gif|webp)$/i;
const NEIGHBOUR_MARGIN = 0.5;
const FIT_MAX = 480;
const GRID = 20;

export const imageState = $state({ rev: 0 });

const entries = new Map<string, Entry>();
const nodeKeys = new Map<string, string>();
let version = 0;

function str(v: unknown): string {
  return typeof v === 'string' ? v.trim() : '';
}

export function isImageLike(node: NodeLike): boolean {
  if (node.kind === 'image') return true;
  if (node.kind !== 'file' || node.data?.view === 'editor') return false;
  const path = str(node.data?.path);
  return !!path && RASTER.test(path);
}

function sourceKey(node: NodeLike): string | null {
  const d = node.data ?? {};
  if (node.kind === 'file') {
    const p = str(d.path);
    return p ? `f:${p}` : null;
  }
  const asset = str(d.asset);
  if (asset) return `a:${asset}`;
  const path = str(d.path);
  if (path) return `p:${path}`;
  const url = str(d.url);
  if (url) return `u:${url}`;
  const src = str(d.src);
  if (src) return `s:${src.length > 256 ? `${src.slice(0, 128)}#${src.length}` : src}`;
  return null;
}

function cacheKey(ws: string, key: string): string {
  return `${ws}|${key}`;
}

function bump(): void {
  imageState.rev++;
}

function argsFor(node: NodeLike): Record<string, unknown> | string {
  const d = node.data ?? {};
  if (node.kind === 'file') return { path: str(d.path), import: false };
  const asset = str(d.asset);
  if (asset) return { asset };
  const path = str(d.path);
  if (path) return { path };
  const url = str(d.url);
  if (url) return { url };
  const src = str(d.src);
  if (/^data:image\//i.test(src)) return { base64: src };
  if (/^https?:\/\//i.test(src)) return { url: src };
  return 'ASSET_UNSUPPORTED';
}

export function imageEntry(engine: CanvasEngine, node: NodeLike): Entry | undefined {
  void imageState.rev;
  const key = sourceKey(node);
  if (!key) return undefined;
  return entries.get(cacheKey(engine.store.workspaceId, key));
}

export function imageError(entry: Entry | undefined): string {
  return entry?.status === 'error' ? (entry.error ?? '') : '';
}

function fitted(w: number, h: number): { w: number; h: number } {
  const k = Math.min(1, FIT_MAX / Math.max(w, h));
  const snap = (v: number) => Math.max(GRID * 2, Math.round((v * k) / GRID) * GRID);
  return { w: snap(w), h: snap(h) };
}

function adopt(engine: CanvasEngine, nodeId: string, r: ImageResolved): void {
  const node = engine.store.node(nodeId);
  if (!node || node.kind !== 'image' || !r.asset) return;
  if (node.data.asset === r.asset && node.data.path === r.path) return;
  const data: Record<string, unknown> = {
    asset: r.asset,
    path: r.path,
    naturalW: r.w,
    naturalH: r.h,
    url: null,
    src: null,
  };
  const from = str(node.data.url) || (/^https?:/i.test(str(node.data.src)) ? str(node.data.src) : '');
  if (r.sourceUrl || from) data.sourceUrl = r.sourceUrl || from;
  const patch: Partial<CanvasNode> = { data };
  if (node.data.fit === true) {
    data.fit = null;
    if (r.w > 0 && r.h > 0) Object.assign(patch, fitted(r.w, r.h));
  }
  engine.updateNode(nodeId, patch);
}

async function loadBitmap(engine: CanvasEngine, e: Entry): Promise<void> {
  if (!e.resolved || e.bitmap || e.bitmapLoading) return;
  e.bitmapLoading = true;
  try {
    const res = await fetch(convertFileSrc(e.resolved.thumb));
    if (!res.ok) throw new Error(`ASSET_THUMB: ${res.status}`);
    const bmp = await createImageBitmap(await res.blob());
    if (entries.get(e.key) !== e) {
      bmp.close();
      return;
    }
    e.bitmap = bmp;
    e.bytes = bmp.width * bmp.height * 4;
    e.version = ++version;
    publish(e);
    engine.invalidate();
  } catch (err) {
    e.status = 'error';
    e.error = String(err);
    bump();
  } finally {
    e.bitmapLoading = false;
  }
}

function publish(e: Entry): void {
  for (const [nodeId, k] of nodeKeys) {
    if (entries.get(k) !== e) continue;
    if (e.bitmap && e.resolved) {
      setSnapshot(nodeId, e.bitmap, e.version, { key: e.key, w: e.resolved.w, h: e.resolved.h });
    } else {
      clearSnapshot(nodeId);
    }
  }
}

async function load(engine: CanvasEngine, node: NodeLike, e: Entry, ws: string): Promise<void> {
  const args = argsFor(node);
  try {
    if (typeof args === 'string') throw new Error(args);
    const r = await invoke<ImageResolved>('canvas_image_thumb', { workspaceId: ws, ...args });
    e.resolved = r;
    e.status = 'ready';
    if (r.asset && node.kind === 'image') {
      const alias = cacheKey(ws, `a:${r.asset}`);
      if (alias !== e.key && !entries.has(alias)) entries.set(alias, e);
      if (engine.store.workspaceId === ws) adopt(engine, node.id, r);
    }
    bump();
    if (canvasPrefs().images.any_zoom) await loadBitmap(engine, e);
  } catch (err) {
    e.status = 'error';
    e.error = String(err);
    bump();
  }
}

export function requestImage(engine: CanvasEngine, node: NodeLike): Entry | undefined {
  if (engine.syncMode !== 'rust') return undefined;
  const key = sourceKey(node);
  if (!key) return undefined;
  const ws = engine.store.workspaceId;
  const ck = cacheKey(ws, key);
  const prevKey = nodeKeys.get(node.id);
  if (prevKey !== ck) {
    nodeKeys.set(node.id, ck);
    if (prevKey) clearSnapshot(node.id);
  }
  let e = entries.get(ck);
  if (!e) {
    e = { key: ck, status: 'loading', bytes: 0, used: performance.now(), version: 0 };
    entries.set(ck, e);
    bump();
    void load(engine, node, e, ws);
    return e;
  }
  e.used = performance.now();
  if (e.status === 'ready' && canvasPrefs().images.any_zoom) {
    if (e.bitmap) {
      if (prevKey !== ck) publish(e);
    } else {
      void loadBitmap(engine, e);
    }
  }
  return e;
}

export function retryImage(engine: CanvasEngine, node: NodeLike): void {
  const key = sourceKey(node);
  if (!key) return;
  const ck = cacheKey(engine.store.workspaceId, key);
  const e = entries.get(ck);
  if (e) {
    e.bitmap?.close();
    for (const [k, v] of entries) if (v === e) entries.delete(k);
  }
  nodeKeys.delete(node.id);
  clearSnapshot(node.id);
  requestImage(engine, node);
}

export function hidesCard(engine: CanvasEngine, node: NodeLike): boolean {
  if (!canvasPrefs().images.any_zoom || !isImageLike(node)) return false;
  return imageEntry(engine, node)?.status !== 'error';
}

function evict(keep: Set<Entry>): void {
  const budget = imageBudgetBytes();
  const live = new Set<Entry>();
  for (const e of entries.values()) if (e.bitmap) live.add(e);
  let total = 0;
  for (const e of live) total += e.bytes;
  if (total <= budget) return;
  const old = Array.from(live)
    .filter((e) => !keep.has(e))
    .sort((a, b) => a.used - b.used);
  for (const e of old) {
    if (total <= budget) break;
    total -= e.bytes;
    e.bitmap?.close();
    e.bitmap = undefined;
    e.bytes = 0;
    publish(e);
  }
}

function dropAll(): void {
  for (const id of nodeKeys.keys()) clearSnapshot(id);
  nodeKeys.clear();
  for (const e of new Set(entries.values())) e.bitmap?.close();
  entries.clear();
  bump();
}

export function watchImages(engine: CanvasEngine): () => void {
  let lastCam = -1;
  let lastVersion = -1;
  let lastWs = '';
  let lastOn = canvasPrefs().images.any_zoom;
  const sync = () => {
    const on = canvasPrefs().images.any_zoom;
    if (on !== lastOn) {
      lastOn = on;
      dropAll();
      lastCam = -1;
    }
    const ws = engine.store.workspaceId;
    if (ws !== lastWs) {
      lastWs = ws;
      for (const id of nodeKeys.keys()) clearSnapshot(id);
      nodeKeys.clear();
      lastCam = -1;
    }
    const cam = engine.camera.rev;
    const ver = engine.store.version;
    if (cam === lastCam && ver === lastVersion) return;
    const storeChanged = ver !== lastVersion;
    lastCam = cam;
    lastVersion = ver;
    if (storeChanged) {
      for (const id of Array.from(nodeKeys.keys())) {
        const n = engine.store.node(id);
        if (!n || !isImageLike(n)) {
          nodeKeys.delete(id);
          clearSnapshot(id);
        }
      }
    }
    const keep = new Set<Entry>();
    for (const id of engine.store.nodeIndex.queryRect(engine.camera.viewRect(NEIGHBOUR_MARGIN))) {
      const n = engine.store.node(id);
      if (!n || !isImageLike(n)) continue;
      if (n.kind === 'file' && !on) continue;
      const e = requestImage(engine, n);
      if (e) keep.add(e);
    }
    if (on) evict(keep);
  };
  const off = engine.onFrame(sync);
  engine.invalidate();
  return () => {
    off();
    for (const id of nodeKeys.keys()) clearSnapshot(id);
    nodeKeys.clear();
  };
}

export async function clearThumbnails(): Promise<number> {
  const freed = await invoke<number>('canvas_image_thumbs_clear');
  dropAll();
  return freed;
}
