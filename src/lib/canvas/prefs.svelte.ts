// Canvas settings (AppSettings.canvas, Rust `CanvasSettings`): one reactive
// read for every consumer (gestures, grid, selection, cords, spawn, notes,
// composer, terminals). Writes from the settings screen go through the
// settings store; writes from MCP/CLI arrive as `canvas://settings`, which
// reloads the store so everything reacts live.

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getSettings, loadSettings } from '$lib/stores/settings-store.svelte';
import type { AgentKind } from './model/types';

export const CANVAS_SETTINGS_EVENT = 'canvas://settings';
export const GRID = 20;

export interface AgentPreset {
  command: string;
  args: string[];
  env: Record<string, string>;
  icon?: string | null;
  color?: string | null;
  maestro: boolean;
  start_in_chat_mode: boolean;
  cwd?: string | null;
}

export interface CanvasPrefs {
  background: 'grid' | 'plain' | 'transparent';
  zoom_step: number;
  zoom_modifier: 'alt' | 'ctrl' | 'cmd';
  right_drag_pan: boolean;
  focus_on_hover: boolean;
  selection_style: 'solid' | 'dashed' | 'corners' | 'corner_dots' | 'elevation';
  node_material: 'opaque' | 'glass';
  cord_style: 'bezier' | 'rope' | 'circuit';
  rope_routing: 'behind_nodes' | 'avoid_nodes' | 'circuit';
  spell_check: boolean;
  default_agent: string;
  defaults: {
    note: { size: string; font: string; render: 'formatted' | 'raw' };
    terminal: { size: string };
    text: { size: string };
    portal: { size: string };
  };
  terminal: {
    font_family: string;
    font_size: number;
    theme: string;
    padding: { x: number; y: number };
    option_as_meta: boolean;
    memory_limit_mb: number;
    shell_fallback: boolean;
  };
  notifications: { attention: boolean; badges_for_non_maestro: boolean };
  power: { prevent_sleep: 'never' | 'always' | 'while_working' };
  agent_presets: Record<string, AgentPreset>;
  /** Settings → Canvas → Shortcuts: action → combination ('' = unbound). */
  shortcuts?: Record<string, string>;
  /** Settings → Canvas → Skills: agent → folder the canvas skill is installed in ('' = session only). */
  skills?: { roots: Record<string, string> };
}

export const CANVAS_PREFS_DEFAULTS: CanvasPrefs = {
  background: 'grid',
  zoom_step: 0.1,
  zoom_modifier: 'alt',
  right_drag_pan: false,
  focus_on_hover: false,
  selection_style: 'solid',
  node_material: 'opaque',
  cord_style: 'bezier',
  rope_routing: 'behind_nodes',
  spell_check: false,
  default_agent: 'shell',
  defaults: {
    note: { size: '', font: '', render: 'formatted' },
    terminal: { size: '' },
    text: { size: '' },
    portal: { size: '' },
  },
  terminal: {
    font_family: '',
    font_size: 0,
    theme: 'system',
    padding: { x: 0, y: 0 },
    option_as_meta: false,
    memory_limit_mb: 0,
    shell_fallback: true,
  },
  notifications: { attention: true, badges_for_non_maestro: true },
  power: { prevent_sleep: 'while_working' },
  agent_presets: {},
};

const BUILTIN_AGENTS = new Set(['shell', 'claude', 'codex', 'gemini', 'opencode', 'ollama', 'native']);

/** Current canvas settings (reactive: reads the settings store). */
export function canvasPrefs(): CanvasPrefs {
  const c = (getSettings() as { canvas?: Partial<CanvasPrefs> } | null)?.canvas;
  if (!c) return CANVAS_PREFS_DEFAULTS;
  const d = CANVAS_PREFS_DEFAULTS;
  return {
    ...d,
    ...c,
    defaults: {
      note: { ...d.defaults.note, ...(c.defaults?.note ?? {}) },
      terminal: { ...d.defaults.terminal, ...(c.defaults?.terminal ?? {}) },
      text: { ...d.defaults.text, ...(c.defaults?.text ?? {}) },
      portal: { ...d.defaults.portal, ...(c.defaults?.portal ?? {}) },
    },
    terminal: { ...d.terminal, ...(c.terminal ?? {}), padding: { ...d.terminal.padding, ...(c.terminal?.padding ?? {}) } },
    notifications: { ...d.notifications, ...(c.notifications ?? {}) },
    power: { ...d.power, ...(c.power ?? {}) },
    agent_presets: c.agent_presets ?? {},
  };
}

/** `"340x220"` → `{w, h}` snapped to the grid; empty/invalid → null. */
export function parseSize(s: string | undefined | null): { w: number; h: number } | null {
  const m = /^\s*(\d+(?:\.\d+)?)\s*[x×X]\s*(\d+(?:\.\d+)?)\s*$/.exec(s ?? '');
  if (!m) return null;
  const snap = (v: number) => Math.max(1, Math.round(v / GRID)) * GRID;
  return { w: snap(Number(m[1])), h: snap(Number(m[2])) };
}

/** The user's default size for a node kind, when set (New Item Defaults). */
export function prefSize(kind: string): { w: number; h: number } | null {
  const d = canvasPrefs().defaults;
  const s =
    kind === 'terminal' || kind === 'agent'
      ? d.terminal.size
      : kind === 'note'
        ? d.note.size
        : kind === 'text'
          ? d.text.size
          : kind === 'portal'
            ? d.portal.size
            : '';
  return parseSize(s);
}

/** Built-in agent a preset runs (for state detection), `shell` otherwise. */
export function presetAgent(p: AgentPreset): AgentKind {
  const prog = (p.command.trim().split(/\s+/)[0] ?? '').split(/[\\/]/).pop()?.replace(/\.exe$/i, '').toLowerCase() ?? '';
  return (['claude', 'codex', 'gemini', 'opencode', 'ollama'].includes(prog) ? prog : 'shell') as AgentKind;
}

/** Node data of a terminal started from a user preset (mirror of Rust `apply_preset`). */
export function presetData(name: string, p: AgentPreset): Record<string, unknown> {
  const command = [...p.command.trim().split(/\s+/).filter(Boolean), ...(p.args ?? [])];
  const data: Record<string, unknown> = { command, agent: presetAgent(p), presetId: name };
  if (p.env && Object.keys(p.env).length) data.env = { ...p.env };
  if (p.cwd?.trim()) data.cwd = p.cwd;
  if (p.maestro) data.isManager = true;
  if (p.start_in_chat_mode) data.prefersChatMode = true;
  return data;
}

export function userPreset(name: string | null | undefined): AgentPreset | null {
  if (!name) return null;
  const all = canvasPrefs().agent_presets;
  if (all[name]) return all[name];
  const k = Object.keys(all).find((x) => x.toLowerCase() === name.toLowerCase());
  return k ? all[k] : null;
}

/** ⌘T / T tool: the configured default (a built-in agent or a preset name). */
export function defaultAgent(): { agent: AgentKind; preset?: string } {
  const d = canvasPrefs().default_agent || 'shell';
  if (BUILTIN_AGENTS.has(d)) return { agent: d as AgentKind };
  const p = userPreset(d);
  return p ? { agent: presetAgent(p), preset: d } : { agent: 'shell' };
}

/** Wheel zoom factor for one notch in `dir` (-1 out, +1 in). */
export function zoomNotch(dir: number): number {
  const step = Math.min(1, Math.max(0.05, canvasPrefs().zoom_step || 0.1));
  return dir > 0 ? 1 + step : 1 / (1 + step);
}

let listening = false;
/** Keeps the store fresh when MCP/CLI change the canvas section. Idempotent. */
export function watchCanvasPrefs(): () => void {
  if (listening) return () => {};
  listening = true;
  if (!getSettings()) void loadSettings().catch(() => {});
  const un = listen(CANVAS_SETTINGS_EVENT, () => void loadSettings().catch(() => {})).catch(() => () => {});
  return () => {
    listening = false;
    void un.then((f) => f());
  };
}

/** Whole-section write (the settings screen; removing a preset needs it). */
export async function replaceCanvasPrefs(next: CanvasPrefs): Promise<CanvasPrefs> {
  const out = await invoke<CanvasPrefs>('canvas_settings_replace', { canvas: next });
  await loadSettings().catch(() => {});
  return out;
}
