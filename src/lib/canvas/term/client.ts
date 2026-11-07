import { Channel, invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type AgentKind = 'claude' | 'codex' | 'gemini' | 'opencode' | 'ollama' | 'shell' | 'native';

interface AgentSignal {
  state: string;
  source: string;
  since_ms: number;
  message: string | null;
  progress: number | null;
}

export interface TermInfo {
  id: string;
  title: string;
  cwd: string | null;
  command: string[];
  pid: number | null;
  cols: number;
  rows: number;
  alive: boolean;
  exit_code: number | null;
  exit_signal: string | null;
  created_at: number;
  foreground: { running: boolean; label: string; pid: number | null };
  agent: AgentSignal;
  osc_title: string | null;
  osc_cwd: string | null;
}

export type WireLod = 0 | 1 | 2;
export interface Visible {
  id: string;
  lod: WireLod;
}
export interface SlotInfo {
  id: string;
  slot: number;
}

export const MOD_SHIFT = 1;
export const MOD_ALT = 2;
export const MOD_CTRL = 4;
export const MOD_META = 8;

interface KeyInput {
  key: string;
  code?: string;
  mods?: number;
  repeat?: boolean;
}
interface ScrollInput {
  lines?: number;
  pages?: number;
  to?: 'top' | 'bottom';
}
interface MouseInput {
  kind: 'down' | 'up' | 'move' | 'wheel';
  button?: 0 | 1 | 2;
  col: number;
  row: number;
  mods?: number;
  wheel?: number;
}
export interface SelectInput {
  kind: 'start' | 'update' | 'clear' | 'all';
  ty?: 'simple' | 'block' | 'semantic' | 'lines';
  col?: number;
  row?: number;
  side?: 'left' | 'right';
}
export interface SearchHit {
  start_col: number;
  start_row: number;
  end_col: number;
  end_row: number;
}
interface ScreenSnapshot {
  text: string;
  cursor_row: number;
  changed_ms: number;
  alt_screen: boolean;
}

interface TermExitEvent {
  id: string;
  code: number | null;
  signal: string | null;
}
interface TermStateEvent {
  id: string;
  state: string;
  source: string;
  since_ms: number;
  message: string | null;
  progress: number | null;
}

export const termClose = (id: string, deleteHistory = false) => invoke<void>('term_close', { id, deleteHistory });
export const termList = () => invoke<TermInfo[]>('term_list');
export const termSubscribe = (channel: Channel<unknown>, visible: Visible[]) =>
  invoke<SlotInfo[]>('term_subscribe', { channel, visible });
export const termSetVisible = (visible: Visible[]) => invoke<SlotInfo[]>('term_set_visible', { visible });
export const termUnsubscribe = () => invoke<void>('term_unsubscribe');
export const termFrameDone = (seq: number) => invoke<void>('term_frame_done', { seq });
export const termKey = (id: string, key: KeyInput) => invoke<void>('term_key', { id, key });
export const termText = (id: string, text: string) => invoke<void>('term_text', { id, text });
export const termPaste = (id: string, text: string) => invoke<void>('term_paste', { id, text });
export const termResize = (id: string, cols: number, rows: number) => invoke<void>('term_resize', { id, cols, rows });
export const termScroll = (id: string, scroll: ScrollInput) => invoke<void>('term_scroll', { id, scroll });
export const termMouse = (id: string, mouse: MouseInput) => invoke<boolean>('term_mouse', { id, mouse });
export const termSelect = (id: string, sel: SelectInput) => invoke<void>('term_select', { id, sel });
export const termCopy = (id: string) => invoke<string | null>('term_copy', { id });
export const termSearch = (id: string, query: string, backwards = false) =>
  invoke<SearchHit | null>('term_search', { id, query, backwards });
export const termFocus = (id: string, focused: boolean) => invoke<void>('term_focus', { id, focused });
export const termLink = (id: string, linkId: number) => invoke<string | null>('term_link', { id, linkId });
export const termClear = (id: string) => invoke<void>('term_clear', { id });
export const termScrollLock = (id: string, on?: boolean) => invoke<boolean>('term_scroll_lock', { id, on: on ?? null });
export const termSearchCount = (id: string, query: string) => invoke<number>('term_search_count', { id, query });
export const termSearchClear = (id: string) => invoke<void>('term_search_clear', { id });
export const termScreen = (id: string, lastLines: number) => invoke<ScreenSnapshot>('term_screen', { id, lastLines });

export function onTermExit(cb: (e: TermExitEvent) => void): Promise<UnlistenFn> {
  return listen<TermExitEvent>('term://exit', (e) => cb(e.payload));
}

export function onTermState(cb: (e: TermStateEvent) => void): Promise<UnlistenFn> {
  return listen<TermStateEvent>('term://state', (e) => cb(e.payload));
}

export { Channel };

export function errorText(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === 'string') return e;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}
