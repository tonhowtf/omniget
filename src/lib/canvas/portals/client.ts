import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export const EVENT_STATE = 'canvas://portal-state';
const EVENT_DRIVE = 'canvas://portal-drive';
const EVENT_CLOSED = 'canvas://portal-closed';

export type StorageScope = 'isolated' | 'workspace' | 'global' | 'named';

export interface PortalState {
  nodeId: string;
  label: string;
  url: string;
  title: string;
  loading: boolean;
  visible: boolean;
  scope: StorageScope;
  userAgent?: string;
  viewport: [number, number];
  certError?: string;
  muted?: boolean;
  openerId?: string;
  sharesSessionWith?: string;
}

export interface PortalDrive {
  nodeId: string;
  by: string;
  method: string;
  at: number;
}

export interface PortalBounds {
  x: number;
  y: number;
  width: number;
  height: number;
  scale?: number;
  zoom?: number;
}

interface OpenArgs {
  nodeId: string;
  workspaceId: string;
  url?: string;
  storageScope?: StorageScope;
  userAgent?: string;
  width?: number;
  height?: number;
}

export function openPortal(a: OpenArgs): Promise<PortalState> {
  return invoke<PortalState>('canvas_portal_open', { ...a });
}

export function closePortal(nodeId: string): Promise<void> {
  snapshots.delete(nodeId);
  return invoke('canvas_portal_close', { nodeId });
}

export function setPortalBounds(nodeId: string, bounds: PortalBounds): Promise<void> {
  return invoke('canvas_portal_set_bounds', { nodeId, bounds });
}

export function hidePortal(nodeId: string): Promise<void> {
  return invoke('canvas_portal_set_visible', { nodeId, visible: false });
}

export function navigatePortal(
  nodeId: string,
  to: { url: string } | { action: 'back' | 'forward' | 'reload' },
): Promise<PortalState> {
  return invoke<PortalState>('canvas_portal_navigate', { nodeId, ...to });
}

export function setPortalUserAgent(nodeId: string, userAgent: string | null): Promise<PortalState> {
  return invoke<PortalState>('canvas_portal_set_user_agent', { nodeId, userAgent });
}

export function trustPortalCert(nodeId: string): Promise<PortalState> {
  return invoke<PortalState>('canvas_portal_trust_cert', { nodeId });
}

export function inspectPortal(nodeId: string): Promise<void> {
  return invoke('canvas_portal_inspect', { nodeId });
}

export function copyPortalScreenshot(nodeId: string): Promise<{ copied: boolean; width: number; height: number }> {
  return invoke('canvas_portal_copy_screenshot', { nodeId });
}

export function portalStates(): Promise<PortalState[]> {
  return invoke<PortalState[]>('canvas_portal_states');
}

const snapshots = new Map<string, string>();
const inflight = new Map<string, Promise<string | null>>();
const snapListeners = new Set<(nodeId: string, url: string) => void>();

export function lastSnapshot(nodeId: string): string | undefined {
  return snapshots.get(nodeId);
}

export function refreshSnapshot(nodeId: string): Promise<string | null> {
  const running = inflight.get(nodeId);
  if (running) return running;
  const p = invoke<string>('canvas_portal_snapshot_image', { nodeId })
    .then((url) => {
      snapshots.set(nodeId, url);
      for (const l of snapListeners) l(nodeId, url);
      return url;
    })
    .catch(() => null)
    .finally(() => inflight.delete(nodeId));
  inflight.set(nodeId, p);
  return p;
}

export function onSnapshot(fn: (nodeId: string, url: string) => void): () => void {
  snapListeners.add(fn);
  return () => snapListeners.delete(fn);
}

type Handlers = {
  state: Set<(s: PortalState) => void>;
  drive: Set<(d: PortalDrive) => void>;
  closed: Set<(nodeId: string) => void>;
};

const handlers: Handlers = { state: new Set(), drive: new Set(), closed: new Set() };
let started: Promise<UnlistenFn[]> | null = null;
const states = new Map<string, PortalState>();
const drives = new Map<string, PortalDrive>();

function start(): void {
  if (started) return;
  started = Promise.all([
    listen<PortalState>(EVENT_STATE, (e) => {
      states.set(e.payload.nodeId, e.payload);
      for (const h of handlers.state) h(e.payload);
    }),
    listen<PortalDrive>(EVENT_DRIVE, (e) => {
      drives.set(e.payload.nodeId, e.payload);
      for (const h of handlers.drive) h(e.payload);
      if (e.payload.method.endsWith(':done') && !states.get(e.payload.nodeId)?.visible) {
        void refreshSnapshot(e.payload.nodeId);
      }
    }),
    listen<{ nodeId: string }>(EVENT_CLOSED, (e) => {
      states.delete(e.payload.nodeId);
      for (const h of handlers.closed) h(e.payload.nodeId);
    }),
  ]).catch(() => []);
}

export function portalState(nodeId: string): PortalState | undefined {
  return states.get(nodeId);
}

export function onPortalState(fn: (s: PortalState) => void): () => void {
  start();
  handlers.state.add(fn);
  return () => handlers.state.delete(fn);
}

export function onPortalDrive(fn: (d: PortalDrive) => void): () => void {
  start();
  handlers.drive.add(fn);
  return () => handlers.drive.delete(fn);
}

export function onPortalClosed(fn: (nodeId: string) => void): () => void {
  start();
  handlers.closed.add(fn);
  return () => handlers.closed.delete(fn);
}

interface DevicePreset {
  id: string;
  key: string;
  label: string;
  width: number;
  height: number;
  ua: string | null;
}

export const DEVICE_PRESETS: DevicePreset[] = [
  { id: 'desktop', key: 'canvas.portal.device.desktop', label: 'Desktop', width: 1280, height: 800, ua: null },
  { id: 'laptop', key: 'canvas.portal.device.laptop', label: 'Laptop', width: 1024, height: 640, ua: null },
  { id: 'ipad', key: 'canvas.portal.device.tablet', label: 'Tablet', width: 820, height: 1180, ua: 'ios' },
  { id: 'iphone', key: 'canvas.portal.device.iphone', label: 'iPhone', width: 390, height: 844, ua: 'ios' },
  { id: 'pixel', key: 'canvas.portal.device.android', label: 'Android phone', width: 412, height: 915, ua: 'android' },
];

export function normalizeUrl(raw: string): string {
  const s = raw.trim();
  if (!s) return '';
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(s) || s.startsWith('about:')) return s;
  if (s.startsWith(':')) return `http://localhost${s}`;
  if (/^(localhost|127\.|\[|\d)/.test(s)) return `http://${s}`;
  if (!s.includes('.') || /\s/.test(s)) return `https://duckduckgo.com/?q=${encodeURIComponent(s)}`;
  return `https://${s}`;
}
