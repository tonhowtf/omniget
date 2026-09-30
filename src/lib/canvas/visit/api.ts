// Read-only canvas visits and phone control: typed wrappers over the
// `canvas_visit_*` / `canvas_remote_*` commands (`commands::canvas::remote`)
// and the events `crate::canvas::visit` emits.

import { invoke } from '@tauri-apps/api/core';

export const EVENT_VISIT_STATE = 'canvas://visit-state';
export const EVENT_VISIT_SCENE = 'canvas://visit-scene';
export const EVENT_VISIT_CHAT = 'canvas://visit-chat';
export const EVENT_VISIT_CLOSED = 'canvas://visit-closed';
export const EVENT_VISIT_REACTION = 'canvas://visit-reaction';
/** Emitted by the Central's remote hub when devices or the listener change. */
export const EVENT_REMOTE_CHANGED = 'remote://changed';

export interface VisitState {
  role: 'none' | 'host' | 'visitor';
  code?: string;
  server?: string;
  hostName?: string | null;
  workspaceId?: string | null;
  floorId?: string | null;
  shared: string[];
  visitors: Array<{ visitorId: number; name: string }>;
}

export interface SceneNode {
  id: string;
  kind: string;
  x: number;
  y: number;
  w: number;
  h: number;
  z?: number;
  name: string;
  color?: string | null;
  shared?: boolean;
  agent?: string;
  roleId?: string | null;
  state?: string | null;
  screen?: string;
}

export interface SceneCord {
  id: string;
  from: string;
  to: string;
  kind: string;
  label?: string | null;
}

export interface Reaction {
  visitorId: number;
  name?: string;
  type: 'wave' | 'point' | 'react';
  x?: number | null;
  y?: number | null;
  emoji?: string | null;
  at: number;
}

export interface Scene {
  v: number;
  house: string;
  host?: string;
  workspace: { name: string; icon?: string | null };
  floor: { id: string; name: string };
  nodes: SceneNode[];
  cords: SceneCord[];
  reactions?: Reaction[];
}

export interface ChatLine {
  from: number;
  name: string;
  text: string;
  ts: number;
}

export function visitStatus(): Promise<VisitState> {
  return invoke('canvas_visit_status');
}

export function visitOpen(workspaceId: string, floorId: string, server?: string): Promise<VisitState> {
  return invoke('canvas_visit_open', { workspaceId, floorId, server: server || null });
}

export function visitClose(): Promise<VisitState> {
  return invoke('canvas_visit_close');
}

export function visitShare(nodeId: string, shared: boolean): Promise<VisitState> {
  return invoke('canvas_visit_share', { nodeId, shared });
}

export function visitJoin(code: string, server?: string): Promise<{ state: VisitState; scene: Scene }> {
  return invoke('canvas_visit_join', { code, server: server || null });
}

export function visitLeave(): Promise<VisitState> {
  return invoke('canvas_visit_leave');
}

export function visitChat(text: string): Promise<void> {
  return invoke('canvas_visit_chat', { text });
}

export function visitReact(kind: Reaction['type'], x?: number, y?: number, emoji?: string): Promise<void> {
  return invoke('canvas_visit_react', { kind, x: x ?? null, y: y ?? null, emoji: emoji ?? null });
}

export function visitRefresh(): Promise<void> {
  return invoke('canvas_visit_refresh');
}

// ---- phone control ------------------------------------------------------

export interface RemoteDeviceView {
  id: string;
  name: string;
  scope: 'read' | 'drive';
  createdAt: string;
  lastUsedAt?: string | null;
  lastIp?: string | null;
  connected: number;
}

export interface RemoteStatusView {
  status: {
    enabled: boolean;
    running: boolean;
    bindIp: string;
    port: number;
    interfaces: Array<{ ip: string; kind: string }>;
    endpoints: Array<{ kind: string; url: string }>;
    devices: number;
    lastError?: string | null;
  };
  canvasUrls: string[];
}

export interface PairCodeView {
  pair: { code: string; scope: 'read' | 'drive'; label?: string | null; expiresAt: string };
  urls: string[];
  qrSvg?: string | null;
}

export function remoteStatus(): Promise<RemoteStatusView> {
  return invoke('canvas_remote_status');
}

/** Reuses the Central's listener command (same devices, same scopes). */
export function remoteStart(bindIp: string): Promise<unknown> {
  return invoke('remote_start', { bindIp, port: null });
}

export function remoteStop(): Promise<unknown> {
  return invoke('remote_stop');
}

export function remotePairCode(scope: 'read' | 'drive', label?: string): Promise<PairCodeView> {
  return invoke('canvas_remote_pair_code', { scope, label: label || null });
}

export function remoteDevices(): Promise<RemoteDeviceView[]> {
  return invoke('remote_devices');
}

export function remoteRevoke(id: string): Promise<boolean> {
  return invoke('remote_device_revoke', { id });
}

export function errorText(e: unknown): string {
  const s = e instanceof Error ? e.message : String(e);
  const i = s.indexOf(':');
  return i > 0 && /^[A-Z_]+$/.test(s.slice(0, i)) ? s.slice(i + 1).trim() : s;
}
