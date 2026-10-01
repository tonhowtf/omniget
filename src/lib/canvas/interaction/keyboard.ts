import type { Rect } from '../model/types';
import type { ToolName } from './tools';
import type { AlignMode } from './snap';

export type Dir = 'left' | 'down' | 'up' | 'right';

export type Command =
  | { cmd: 'escape' }
  | { cmd: 'tool'; tool: ToolName }
  | { cmd: 'create'; kind: string }
  | { cmd: 'focus-dir'; dir: Dir }
  | { cmd: 'nudge'; dx: number; dy: number }
  | { cmd: 'zoom-toggle' }
  | { cmd: 'zoom-fit' }
  | { cmd: 'zoom-in' }
  | { cmd: 'zoom-out' }
  | { cmd: 'zoom-reset' }
  | { cmd: 'split'; dir: 'right' | 'down' }
  | { cmd: 'close' }
  | { cmd: 'delete' }
  | { cmd: 'jump'; index: number }
  | { cmd: 'enter' }
  | { cmd: 'undo' }
  | { cmd: 'redo' }
  | { cmd: 'group' }
  | { cmd: 'ungroup' }
  | { cmd: 'raise' }
  | { cmd: 'lower' }
  | { cmd: 'front' }
  | { cmd: 'back' }
  | { cmd: 'select-all' }
  | { cmd: 'duplicate' }
  | { cmd: 'next-attention' }
  | { cmd: 'toggle-minimap' }
  | { cmd: 'floors-overview' }
  | { cmd: 'omni-window' }
  | { cmd: 'stroke-width'; delta: number }
  | { cmd: 'lock' }
  | { cmd: 'connect' }
  | { cmd: 'copy' }
  | { cmd: 'cut' }
  | { cmd: 'paste' }
  | { cmd: 'cycle'; delta: number }
  | { cmd: 'view'; id: string }
  | { cmd: 'new-terminal' }
  | { cmd: 'cord-walk'; delta: number }
  | { cmd: 'new-note' }
  | { cmd: 'lift' }
  | { cmd: 'align'; mode: AlignMode }
  | { cmd: 'rotate'; deg: number };

interface KeyLike {
  key: string;
  code?: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

interface KeyContext {
  hasSelection: boolean;
  drawingsOnly?: boolean;
  mac: boolean;
}

export function resolveKey(e: KeyLike, ctx: KeyContext): Command | null {
  const primary = ctx.mac ? e.metaKey : e.ctrlKey;
  const k = e.key;
  const lower = k.length === 1 ? k.toLowerCase() : k;

  if (k === 'Escape') return { cmd: 'escape' };
  if (k === 'Tab' && e.ctrlKey && !e.metaKey && !e.altKey) return { cmd: 'cycle', delta: e.shiftKey ? -1 : 1 };

  const custom = customShortcut(e, ctx.mac);
  if (custom !== undefined) return custom;

  if (primary) {
    if (e.altKey && !e.shiftKey && (k === 'ArrowRight' || k === 'ArrowLeft')) {
      return { cmd: 'cord-walk', delta: k === 'ArrowRight' ? 1 : -1 };
    }
    if (lower === 'z') return e.shiftKey ? { cmd: 'redo' } : { cmd: 'undo' };
    if (lower === 'y' && !ctx.mac) return { cmd: 'redo' };
    if (k === '=' || k === '+') return { cmd: 'zoom-in' };
    if (k === '-' || k === '_') return { cmd: 'zoom-out' };
    if (k === '0') return { cmd: 'zoom-reset' };
    if (/^[1-9]$/.test(k)) return { cmd: 'jump', index: Number(k) };
    if (lower === 'a' && e.shiftKey) return { cmd: 'next-attention' };
    if (lower === 'a') return { cmd: 'select-all' };
    if (lower === 'm' && e.shiftKey) return { cmd: 'toggle-minimap' };
    if (lower === 'g') return e.shiftKey ? { cmd: 'ungroup' } : { cmd: 'group' };
    if (lower === 'd' && !e.shiftKey) return { cmd: 'duplicate' };
    if (lower === 'c' && !e.shiftKey && !e.altKey) return ctx.hasSelection ? { cmd: 'copy' } : null;
    if (lower === 'x' && !e.shiftKey && !e.altKey) return ctx.hasSelection ? { cmd: 'cut' } : null;
    if (lower === 'v' && !e.shiftKey && !e.altKey) return { cmd: 'paste' };
    if (lower === 'l') return { cmd: 'connect' };
    if (lower === 'o' && e.shiftKey && !e.altKey) return { cmd: 'omni-window' };
    if ((k === '\\' || k === '|' || e.code === 'Backslash') && e.shiftKey && !e.altKey) return { cmd: 'floors-overview' };
    if (k === '\\' || k === '«' || e.code === 'Backslash') return e.altKey ? { cmd: 'view', id: 'zoomToSelection' } : { cmd: 'zoom-toggle' };
    if (lower === 'w' && !e.shiftKey && !e.altKey) return ctx.hasSelection ? { cmd: 'delete' } : null;
    if (lower === 't' && e.shiftKey && !e.altKey) return { cmd: 'view', id: 'tidy' };
    if (lower === 't' && !e.altKey) return { cmd: 'new-terminal' };
    if (lower === 'd' && e.shiftKey && !e.altKey) return { cmd: 'new-note' };
    if (k === ']' || k === '}' || e.code === 'BracketRight') {
      if (e.shiftKey) return { cmd: 'front' };
      return e.altKey ? { cmd: 'raise' } : null;
    }
    if (k === '[' || k === '{' || e.code === 'BracketLeft') {
      if (e.shiftKey) return { cmd: 'back' };
      return e.altKey ? { cmd: 'lower' } : null;
    }
    return null;
  }
  if (e.ctrlKey || e.metaKey) return null;

  if (k === 'Delete' || k === 'Backspace') return ctx.hasSelection ? { cmd: 'delete' } : null;
  if (k === '!') return { cmd: 'next-attention' };

  if (ctx.hasSelection) {
    if (k === 'Enter') return { cmd: 'enter' };
    const step = e.shiftKey ? 100 : 20;
    if (e.altKey) {
      if (k === 'ArrowLeft') return { cmd: 'nudge', dx: -step, dy: 0 };
      if (k === 'ArrowRight') return { cmd: 'nudge', dx: step, dy: 0 };
      if (k === 'ArrowUp') return { cmd: 'nudge', dx: 0, dy: -step };
      if (k === 'ArrowDown') return { cmd: 'nudge', dx: 0, dy: step };
    }
    if (ctx.drawingsOnly) {
      if (lower === 'r' && !e.altKey) return { cmd: 'rotate', deg: e.shiftKey ? -15 : 15 };
      const fine = e.shiftKey ? 10 : 1;
      if (k === 'ArrowLeft') return { cmd: 'nudge', dx: -fine, dy: 0 };
      if (k === 'ArrowRight') return { cmd: 'nudge', dx: fine, dy: 0 };
      if (k === 'ArrowUp') return { cmd: 'nudge', dx: 0, dy: -fine };
      if (k === 'ArrowDown') return { cmd: 'nudge', dx: 0, dy: fine };
    }
    if (lower === 'h' || k === 'ArrowLeft') return { cmd: 'focus-dir', dir: 'left' };
    if (lower === 'j' || k === 'ArrowDown') return { cmd: 'focus-dir', dir: 'down' };
    if (lower === 'k' || k === 'ArrowUp') return { cmd: 'focus-dir', dir: 'up' };
    if (lower === 'l' || k === 'ArrowRight') return { cmd: 'focus-dir', dir: 'right' };
    if (lower === 'z') return { cmd: 'zoom-toggle' };
    if (k === '%') return { cmd: 'split', dir: 'right' };
    if (k === '"') return { cmd: 'split', dir: 'down' };
    if (lower === 'x') return { cmd: 'close' };
    if (/^[1-9]$/.test(k)) return { cmd: 'jump', index: Number(k) };
    if (k === '[') return { cmd: 'stroke-width', delta: -1 };
    if (k === ']') return { cmd: 'stroke-width', delta: 1 };
    return null;
  }

  switch (lower) {
    case 'v':
      return { cmd: 'tool', tool: 'select' };
    case 'h':
      return { cmd: 'tool', tool: 'hand' };
    case 'd':
      return { cmd: 'tool', tool: 'pen' };
    case 'e':
      return { cmd: 'tool', tool: 'eraser' };
    case 'b':
      return { cmd: 'tool', tool: 'rect' };
    case 'o':
      return { cmd: 'tool', tool: 'ellipse' };
    case 'a':
      return { cmd: 'tool', tool: 'arrow' };
    case 'c':
      return { cmd: 'tool', tool: 'connect' };
    case 't':
      return { cmd: 'create', kind: 'terminal' };
    case 'n':
      return { cmd: 'create', kind: 'note' };
    case 'l':
      return { cmd: 'create', kind: 'text' };
    case 'f':
      return { cmd: 'create', kind: 'filetree' };
    case 'p':
      return { cmd: 'create', kind: 'portal' };
    case 'z':
      return { cmd: 'zoom-fit' };
  }
  if (k === '[') return { cmd: 'stroke-width', delta: -1 };
  if (k === ']') return { cmd: 'stroke-width', delta: 1 };
  if (k === '%') return { cmd: 'split', dir: 'right' };
  if (k === '"') return { cmd: 'split', dir: 'down' };
  if (/^[1-9]$/.test(k)) return { cmd: 'jump', index: Number(k) };
  return null;
}

export const SHORTCUT_ACTIONS: Record<string, { combo: string; command: Command }> = {
  new_note: { combo: 'mod+shift+d', command: { cmd: 'new-note' } },
  new_terminal: { combo: 'mod+t', command: { cmd: 'new-terminal' } },
  lift: { combo: '', command: { cmd: 'lift' } },
  next_attention: { combo: 'mod+shift+a', command: { cmd: 'next-attention' } },
  cord_next: { combo: 'mod+alt+arrowright', command: { cmd: 'cord-walk', delta: 1 } },
  cord_previous: { combo: 'mod+alt+arrowleft', command: { cmd: 'cord-walk', delta: -1 } },
  toggle_minimap: { combo: 'mod+shift+m', command: { cmd: 'toggle-minimap' } },
  focus_selection: { combo: 'mod+\\', command: { cmd: 'zoom-toggle' } },
  zoom_to_selection: { combo: 'mod+alt+\\', command: { cmd: 'view', id: 'zoomToSelection' } },
  tidy: { combo: 'mod+shift+t', command: { cmd: 'view', id: 'tidy' } },
  duplicate: { combo: 'mod+d', command: { cmd: 'duplicate' } },
  group: { combo: 'mod+g', command: { cmd: 'group' } },
  ungroup: { combo: 'mod+shift+g', command: { cmd: 'ungroup' } },
  connect: { combo: 'mod+l', command: { cmd: 'connect' } },
  align_left: { combo: '', command: { cmd: 'align', mode: 'left' } },
  center_horizontally: { combo: '', command: { cmd: 'align', mode: 'hcenter' } },
  align_right: { combo: '', command: { cmd: 'align', mode: 'right' } },
  align_top: { combo: '', command: { cmd: 'align', mode: 'top' } },
  center_vertically: { combo: '', command: { cmd: 'align', mode: 'vcenter' } },
  align_bottom: { combo: '', command: { cmd: 'align', mode: 'bottom' } },
  distribute_horizontally: { combo: '', command: { cmd: 'align', mode: 'hdistribute' } },
  distribute_vertically: { combo: '', command: { cmd: 'align', mode: 'vdistribute' } },
};

let overrides: Record<string, string> = {};

export function setShortcutOverrides(map: Record<string, string> | null | undefined): void {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(map ?? {})) if (k in SHORTCUT_ACTIONS && typeof v === 'string') out[k] = normCombo(v);
  overrides = out;
}

export function normCombo(s: string): string {
  const parts = s
    .toLowerCase()
    .split('+')
    .map((p) => p.trim())
    .filter(Boolean);
  if (!parts.length) return '';
  const key = parts.pop()!;
  const mods = new Set(
    parts.map((p) => (p === 'cmd' || p === 'command' || p === 'meta' || p === 'primary' || p === 'ctrl' || p === 'control' ? 'mod' : p === 'option' || p === 'opt' ? 'alt' : p)),
  );
  const order = ['mod', 'alt', 'shift'].filter((m) => mods.has(m));
  const k = key === 'right' ? 'arrowright' : key === 'left' ? 'arrowleft' : key === 'up' ? 'arrowup' : key === 'down' ? 'arrowdown' : key;
  return [...order, k].join('+');
}

function comboOf(e: KeyLike, mac: boolean): string {
  const primary = mac ? e.metaKey : e.ctrlKey;
  const k = e.key.length === 1 ? e.key.toLowerCase() : e.key.toLowerCase();
  const parts: string[] = [];
  if (primary) parts.push('mod');
  if (e.altKey) parts.push('alt');
  if (e.shiftKey) parts.push('shift');
  parts.push(k);
  return parts.join('+');
}

function customShortcut(e: KeyLike, mac: boolean): Command | null | undefined {
  const ids = Object.keys(overrides);
  if (!ids.length) return undefined;
  const combo = comboOf(e, mac);
  for (const id of ids) if (overrides[id] && overrides[id] === combo) return SHORTCUT_ACTIONS[id].command;
  for (const id of ids) if (normCombo(SHORTCUT_ACTIONS[id].combo) === combo) return null;
  return undefined;
}

export function neighborInDirection(from: Rect, candidates: Array<Rect & { id: string }>, dir: Dir): string | null {
  const fx = from.x + from.w / 2;
  const fy = from.y + from.h / 2;
  let best: { id: string; score: number } | null = null;
  for (const c of candidates) {
    const cx = c.x + c.w / 2;
    const cy = c.y + c.h / 2;
    const dx = cx - fx;
    const dy = cy - fy;
    let along: number;
    let ortho: number;
    switch (dir) {
      case 'left':
        along = -dx;
        ortho = Math.abs(dy);
        break;
      case 'right':
        along = dx;
        ortho = Math.abs(dy);
        break;
      case 'up':
        along = -dy;
        ortho = Math.abs(dx);
        break;
      case 'down':
        along = dy;
        ortho = Math.abs(dx);
        break;
    }
    if (along <= 1) continue;
    const score = along + 2 * ortho;
    if (!best || score < best.score) best = { id: c.id, score };
  }
  return best?.id ?? null;
}

export function isMac(): boolean {
  return typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
}

export function isEditableTarget(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  if (!el || typeof el.closest !== 'function') return false;
  if (el.isContentEditable) return true;
  const tag = el.tagName;
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true;
  return !!el.closest('.xterm, [data-canvas-keys="own"]');
}
