import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

const EVENT_FRAME = 'canvas://device-frame';
const EVENT_STATE = 'canvas://device-state';

export type DevicePlatform = 'ios' | 'android';

export interface DeviceEntry {
  platform: DevicePlatform;
  id: string;
  name: string;
  os?: string;
  state: string;
  kind: string;
}

export interface DeviceList {
  devices: DeviceEntry[];
  touchIos: boolean;
  notes: string[];
}

export interface DeviceFrame {
  nodeId: string;
  seq: number;
  file: string;
  w: number;
  h: number;
  scale: number;
}

export interface DeviceInfo {
  nodeId: string;
  platform: DevicePlatform;
  device: { id: string; name: string };
  bootedByOmniget: boolean;
  scale: number;
  capabilities: Record<string, boolean>;
  frame?: { path: string; w: number; h: number; seq: number } | null;
}

export interface DeviceStateEvent extends Partial<DeviceInfo> {
  nodeId: string;
  closed?: boolean;
  error?: string;
}

export function listDevices(): Promise<DeviceList> {
  return invoke<DeviceList>('canvas_device_list');
}

export function openDevice(nodeId: string, platform: DevicePlatform, device?: string): Promise<DeviceInfo> {
  return invoke<DeviceInfo>('canvas_device_open', { nodeId, platform, device });
}

export function deviceState(nodeId: string): Promise<DeviceInfo | null> {
  return invoke<DeviceInfo | null>('canvas_device_state', { nodeId });
}

export function watchDevice(nodeId: string, on: boolean): Promise<void> {
  return invoke('canvas_device_watch', { nodeId, on });
}

export function actDevice(nodeId: string, action: string, args: Record<string, unknown> = {}): Promise<void> {
  return invoke('canvas_device_act', { nodeId, action, args });
}

export function closeDevice(nodeId: string): Promise<void> {
  return invoke('canvas_device_close', { nodeId });
}

export function frameSrc(file: string, seq: number): string {
  return `${convertFileSrc(file)}?v=${seq}`;
}

export function onDeviceFrame(cb: (f: DeviceFrame) => void): Promise<UnlistenFn> {
  return listen<DeviceFrame>(EVENT_FRAME, (e) => cb(e.payload));
}

export function onDeviceState(cb: (s: DeviceStateEvent) => void): Promise<UnlistenFn> {
  return listen<DeviceStateEvent>(EVENT_STATE, (e) => cb(e.payload));
}

export function toDevicePoint(
  rect: { left: number; top: number; width: number; height: number },
  frame: { w: number; h: number; scale: number },
  clientX: number,
  clientY: number,
): { x: number; y: number } | null {
  if (rect.width <= 0 || rect.height <= 0 || frame.w <= 0 || frame.h <= 0) return null;
  const fit = Math.min(rect.width / frame.w, rect.height / frame.h);
  const shownW = frame.w * fit;
  const shownH = frame.h * fit;
  const ox = rect.left + (rect.width - shownW) / 2;
  const oy = rect.top + (rect.height - shownH) / 2;
  const fx = (clientX - ox) / shownW;
  const fy = (clientY - oy) / shownH;
  if (fx < 0 || fy < 0 || fx > 1 || fy > 1) return null;
  const s = frame.scale > 0 ? frame.scale : 1;
  return { x: Math.round((fx * frame.w) / s), y: Math.round((fy * frame.h) / s) };
}
