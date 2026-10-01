// Our own input layer for GL-drawn terminals (estudo 98/01 §5). A hidden
// <textarea> sits over the cursor cell so the IME candidate window opens in
// the right place. Text arrives through beforeinput/composition (never
// keypress), keydown only carries non-text keys, and Rust turns keys into
// bytes using the live terminal modes.
//
// Dead keys (ABNT2 ´ ~ ^ `): the browser reports them as a composition; we
// send only the committed text at compositionend, and drop an insertText
// that repeats it right after (WebKit sends both on some layouts).

import { invoke } from '@tauri-apps/api/core';
import { open as openUrl } from '@tauri-apps/plugin-shell';
import {
  MOD_ALT,
  MOD_CTRL,
  MOD_META,
  MOD_SHIFT,
  errorText,
  termClear,
  termCopy,
  termFocus,
  termKey,
  termLink,
  termMouse,
  termPaste,
  termScroll,
  termSelect,
  termText,
  type SelectInput,
} from './client';
import { termHub } from './hub';
import { setLodFocus } from './lod';
import { termLayout } from './pass';
import { cellMetrics } from './metrics';
import { termLive } from './state.svelte';
import { MOUSE_MODES, SF_LINK } from './wire';
import { resolvePath, smartClean, tokenAt } from './text';

export { resolvePath, smartClean, tokenAt };

const IS_MAC = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

/** Keys that never produce text and always go to Rust through keydown. */
const NAMED_KEYS = new Set([
  'Enter',
  'Backspace',
  'Tab',
  'Escape',
  'ArrowUp',
  'ArrowDown',
  'ArrowLeft',
  'ArrowRight',
  'Home',
  'End',
  'PageUp',
  'PageDown',
  'Insert',
  'Delete',
  'Clear',
]);

function modsOf(e: { shiftKey: boolean; altKey: boolean; ctrlKey: boolean; metaKey: boolean }): number {
  return (e.shiftKey ? MOD_SHIFT : 0) | (e.altKey ? MOD_ALT : 0) | (e.ctrlKey ? MOD_CTRL : 0) | (e.metaKey ? MOD_META : 0);
}

function isFunctionKey(key: string): boolean {
  return /^F([1-9]|1\d|2[0-4])$/.test(key);
}

type KeyRoute =
  | 'terminal'
  | 'text'
  | 'app'
  | 'kill-line'
  | 'copy'
  | 'paste'
  | 'select-all'
  | 'unfocus'
  | 'scroll-up'
  | 'scroll-down'
  | 'ignore'
  | TermCommand;

/** Terminal commands handled by the node (find bar, font) or by this layer (clear, smart copy). */
export type TermCommand = 'find' | 'clear' | 'scroll-lock' | 'font-up' | 'font-down' | 'font-reset' | 'smart-copy';

/** ⌘⇧+ / ⌘⇧− / ⌘⇧0 whatever the layout (US '+'/'_', ABNT2 '+'/'_', or the physical keys). */
function fontKey(e: { key: string; code?: string }): 'font-up' | 'font-down' | 'font-reset' | null {
  if (e.key === '+' || e.key === '=' || e.code === 'Equal' || e.code === 'NumpadAdd') return 'font-up';
  if (e.key === '-' || e.key === '_' || e.code === 'Minus' || e.code === 'NumpadSubtract') return 'font-down';
  if (e.key === '0' || e.key === ')' || e.code === 'Digit0') return 'font-reset';
  return null;
}

/**
 * Where a keydown goes. Pure, so the reserved-shortcut rules are testable.
 * `optionAsMeta` = Option sends ESC-prefixed keys instead of composing characters.
 */
function routeKey(
  e: { key: string; code?: string; shiftKey: boolean; altKey: boolean; ctrlKey: boolean; metaKey: boolean; isComposing?: boolean; keyCode?: number },
  isMac: boolean,
  hasSelection: boolean,
  optionAsMeta = false,
): KeyRoute {
  if (e.isComposing || e.keyCode === 229) return 'ignore';
  if (e.key === 'Dead' || e.key === 'Process' || e.key === 'Unidentified') return 'ignore';
  if (['Shift', 'Control', 'Alt', 'Meta', 'CapsLock', 'AltGraph', 'OS', 'Fn'].includes(e.key)) return 'ignore';
  const k = e.key.length === 1 ? e.key.toLowerCase() : e.key;

  if (isMac) {
    if (e.metaKey && !e.ctrlKey && !e.altKey) {
      if (e.shiftKey) {
        if (k === 'c') return 'smart-copy';
        if (k === 'b') return 'scroll-lock';
        const f = fontKey(e);
        if (f) return f;
      } else {
        if (k === 'f') return 'find';
        if (k === 'k') return 'clear';
      }
    }
    if (e.metaKey) {
      if (e.key === 'Escape') return 'unfocus';
      if (k === 'c') return hasSelection ? 'copy' : 'app';
      if (k === 'v') return 'paste';
      if (k === 'a') return 'select-all';
      // ⌘⌫ erases to the start of the line (Maestri; readline's ^U).
      if (e.key === 'Backspace' && !e.shiftKey && !e.altKey && !e.ctrlKey) return 'kill-line';
      return 'app';
    }
  } else {
    if (e.ctrlKey && e.shiftKey && e.altKey && k === 'c') return 'smart-copy';
    if (e.ctrlKey && e.shiftKey && !e.altKey) {
      if (k === 'f') return 'find';
      if (k === 'k') return 'clear';
      if (k === 'b') return 'scroll-lock';
      const f = fontKey(e);
      if (f) return f;
    }
    if (e.ctrlKey && e.shiftKey) {
      if (k === 'c') return 'copy';
      if (k === 'v') return 'paste';
      if (k === 'a') return 'select-all';
      return 'app';
    }
    if (e.metaKey) return 'app';
  }

  if (e.shiftKey && !e.ctrlKey && !e.altKey && (e.key === 'PageUp' || e.key === 'PageDown')) {
    return e.key === 'PageUp' ? 'scroll-up' : 'scroll-down';
  }
  if (NAMED_KEYS.has(e.key) || isFunctionKey(e.key)) return 'terminal';

  if (e.key.length >= 1 && [...e.key].length === 1) {
    // AltGr on Windows/Linux arrives as Ctrl+Alt and types a character (ABNT2 AltGr+Q = '/').
    if (!isMac && e.ctrlKey && e.altKey) return 'text';
    if (e.ctrlKey) return 'terminal';
    if (e.altKey) {
      if (isMac && !optionAsMeta) return 'text';
      return 'terminal';
    }
    return 'text';
  }
  return 'ignore';
}

export interface TermInputHandle {
  focus(): void;
  destroy(): void;
}

interface TermInputOptions {
  id: string;
  /** Called when the user leaves the terminal with the keyboard (Cmd+Esc). */
  onUnfocus?: () => void;
  optionAsMeta?: () => boolean;
  /** Find bar, font size and scroll lock live in the node. */
  onCommand?: (cmd: TermCommand) => void;
}

let focused: string | null = null;

async function copySelection(id: string): Promise<void> {
  const text = await termCopy(id);
  if (!text) return;
  try {
    const { writeText } = await import('@tauri-apps/plugin-clipboard-manager');
    await writeText(text);
  } catch {
    await navigator.clipboard?.writeText(text).catch(() => {});
  }
}

async function writeClipboard(text: string): Promise<void> {
  try {
    const { writeText } = await import('@tauri-apps/plugin-clipboard-manager');
    await writeText(text);
  } catch {
    await navigator.clipboard?.writeText(text).catch(() => {});
  }
}

async function smartCopy(id: string): Promise<void> {
  const text = await termCopy(id);
  if (!text) return;
  await writeClipboard(smartClean(text, termHub.grid(id)?.cols ?? 80));
}

async function readClipboard(): Promise<string> {
  try {
    const { readText } = await import('@tauri-apps/plugin-clipboard-manager');
    return (await readText()) ?? '';
  } catch {
    try {
      return await navigator.clipboard.readText();
    } catch {
      return '';
    }
  }
}

function report(id: string, e: unknown): void {
  termLive(id).error = errorText(e);
}

/** Wires the hidden textarea of one terminal node. */
export function attachTermInput(ta: HTMLTextAreaElement, opts: TermInputOptions): TermInputHandle {
  const id = opts.id;
  let composing = false;
  let lastCommit = '';
  let lastCommitAt = 0;

  const send = (p: Promise<unknown>) => void p.catch((e) => report(id, e));
  const clear = () => {
    if (!composing) ta.value = '';
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const route = routeKey(e, IS_MAC, termLive(id).hasSelection, opts.optionAsMeta?.() ?? false);
    switch (route) {
      case 'ignore':
      case 'text':
        return;
      case 'app':
        return; // bubble to the app's shortcut handler
      case 'clear':
        e.preventDefault();
        e.stopPropagation();
        send(termClear(id));
        clear();
        return;
      case 'smart-copy':
        e.preventDefault();
        e.stopPropagation();
        void smartCopy(id).catch((err) => report(id, err));
        return;
      case 'find':
      case 'scroll-lock':
      case 'font-up':
      case 'font-down':
      case 'font-reset':
        e.preventDefault();
        e.stopPropagation();
        opts.onCommand?.(route);
        return;
      case 'unfocus':
        e.preventDefault();
        ta.blur();
        opts.onUnfocus?.();
        return;
      case 'copy':
        e.preventDefault();
        void copySelection(id).catch((err) => report(id, err));
        return;
      case 'paste':
        // Cmd+V: the native paste event follows. Ctrl+Shift+V has none: read ourselves.
        if (!IS_MAC) {
          e.preventDefault();
          void readClipboard().then((t) => t && send(termPaste(id, t)));
        }
        return;
      case 'select-all':
        e.preventDefault();
        send(termSelect(id, { kind: 'all' }));
        return;
      case 'kill-line':
        e.preventDefault();
        e.stopPropagation();
        send(termText(id, '\x15'));
        clear();
        return;
      case 'scroll-up':
      case 'scroll-down':
        e.preventDefault();
        e.stopPropagation();
        send(termScroll(id, { pages: route === 'scroll-up' ? 1 : -1 }));
        return;
      case 'terminal':
        e.preventDefault();
        e.stopPropagation();
        send(termKey(id, { key: e.key, code: e.code, mods: modsOf(e), repeat: e.repeat }));
        clear();
        return;
    }
  };

  const onBeforeInput = (e: InputEvent) => {
    const type = e.inputType;
    if (composing || type === 'insertCompositionText' || type === 'insertFromComposition' || type === 'deleteCompositionText') {
      return; // committed at compositionend
    }
    if (type === 'insertText' || type === 'insertReplacementText') {
      e.preventDefault();
      const data = e.data ?? e.dataTransfer?.getData('text/plain') ?? '';
      if (!data) return;
      if (data === lastCommit && performance.now() - lastCommitAt < 50) return;
      send(termText(id, data));
      return;
    }
    if (type === 'insertLineBreak' || type === 'insertParagraph') {
      e.preventDefault();
      send(termKey(id, { key: 'Enter', mods: 0 }));
      return;
    }
    if (type === 'deleteContentBackward') {
      e.preventDefault();
      send(termKey(id, { key: 'Backspace', mods: 0 }));
      return;
    }
    if (type === 'insertFromPaste' || type === 'insertFromDrop') {
      e.preventDefault(); // the paste event carries the text
      return;
    }
    e.preventDefault();
  };

  const onCompositionStart = () => {
    composing = true;
    ta.classList.add('composing');
  };
  const onCompositionEnd = (e: CompositionEvent) => {
    composing = false;
    ta.classList.remove('composing');
    const data = e.data ?? '';
    if (data) {
      lastCommit = data;
      lastCommitAt = performance.now();
      send(termText(id, data));
    }
    // WebKit may still write the committed text into the textarea after this.
    setTimeout(clear, 0);
  };
  const onInput = () => clear();

  const onPaste = (e: ClipboardEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const text = e.clipboardData?.getData('text/plain') ?? '';
    if (text) send(termPaste(id, text));
  };

  const onFocus = () => {
    focused = id;
    setLodFocus(id);
    termHub.redraw();
    send(termFocus(id, true));
  };
  const onBlur = () => {
    if (focused === id) {
      focused = null;
      setLodFocus(null);
    }
    composing = false;
    ta.classList.remove('composing');
    ta.value = '';
    termHub.redraw();
    send(termFocus(id, false));
  };

  ta.addEventListener('keydown', onKeyDown);
  ta.addEventListener('beforeinput', onBeforeInput);
  ta.addEventListener('input', onInput);
  ta.addEventListener('compositionstart', onCompositionStart);
  ta.addEventListener('compositionend', onCompositionEnd);
  ta.addEventListener('paste', onPaste);
  ta.addEventListener('focus', onFocus);
  ta.addEventListener('blur', onBlur);

  return {
    focus() {
      ta.focus({ preventScroll: true });
    },
    destroy() {
      ta.removeEventListener('keydown', onKeyDown);
      ta.removeEventListener('beforeinput', onBeforeInput);
      ta.removeEventListener('input', onInput);
      ta.removeEventListener('compositionstart', onCompositionStart);
      ta.removeEventListener('compositionend', onCompositionEnd);
      ta.removeEventListener('paste', onPaste);
      ta.removeEventListener('focus', onFocus);
      ta.removeEventListener('blur', onBlur);
      if (focused === id) {
        focused = null;
        setLodFocus(null);
      }
    },
  };
}

// ---------------------------------------------------------------- pointer

function rowCells(id: string, row: number): string[] {
  const grid = termHub.grid(id);
  if (!grid || row >= grid.rows) return [];
  const out: string[] = [];
  for (let x = 0; x < grid.cols; x++) {
    const cp = grid.cp[row * grid.cols + x];
    out.push(cp === 0 ? ' ' : grid.cluster(cp));
  }
  return out;
}

interface CellPos {
  col: number;
  row: number;
  /** Which half of the cell was hit (selection anchoring). */
  side: 'left' | 'right';
}

/**
 * Cell under a client point. `body` is the DOM element laid over the GL body;
 * the cell size comes from what the pass actually drew (bucket-snapped), so
 * hits match the pixels even when the grid is not an exact fit.
 */
function cellAt(id: string, body: HTMLElement, clientX: number, clientY: number): CellPos {
  const r = body.getBoundingClientRect();
  const layout = termLayout(id);
  let cw: number;
  let ch: number;
  // Cell (0,0) where the pass drew it: the body box and the GL body can
  // differ by the padding (Settings → Canvas → Terminal padding, M-D18).
  const origin = drawnOrigin(layout) ?? { x: r.left, y: r.top };
  if (layout) {
    cw = layout.cellW;
    ch = layout.cellH;
  } else {
    const m = cellMetrics();
    const scale = body.offsetWidth ? r.width / body.offsetWidth : 1;
    cw = m.w * scale;
    ch = m.h * scale;
  }
  const grid = termHub.grid(id);
  const maxCol = Math.max(0, (grid?.cols ?? 1) - 1);
  const maxRow = Math.max(0, (grid?.rows ?? 1) - 1);
  const fx = (clientX - origin.x) / cw;
  const fy = (clientY - origin.y) / ch;
  const col = Math.max(0, Math.min(maxCol, Math.floor(fx)));
  const row = Math.max(0, Math.min(maxRow, Math.floor(fy)));
  return { col, row, side: fx - Math.floor(fx) < 0.5 ? 'left' : 'right' };
}

/** Local (unscaled) position of the cursor cell inside `body`, for placing the textarea. */
export function cursorBox(id: string, body: HTMLElement): { x: number; y: number; w: number; h: number; font: number } {
  const m = cellMetrics();
  const grid = termHub.grid(id);
  const layout = termLayout(id);
  const r = body.getBoundingClientRect();
  const scale = body.offsetWidth ? r.width / body.offsetWidth : 1;
  // Drawn cell size back in the body's local units.
  const cw = layout ? layout.cellW / scale : m.w;
  const ch = layout ? layout.cellH / scale : m.h;
  const cx = grid?.cursor.x ?? 0;
  const cy = grid?.cursor.y ?? 0;
  const o = drawnOrigin(layout);
  const ox = o ? (o.x - r.left) / scale : 0;
  const oy = o ? (o.y - r.top) / scale : 0;
  return { x: ox + cx * cw, y: oy + cy * ch, w: cw, h: ch, font: (ch / m.h) * 13 };
}

/** Client position of the drawn cell (0,0): the pass layout is in canvas CSS px. */
function drawnOrigin(layout: ReturnType<typeof termLayout>): { x: number; y: number } | null {
  if (!layout || typeof document === 'undefined') return null;
  const gl = document.querySelector<HTMLCanvasElement>('canvas.l0');
  if (!gl) return null;
  const c = gl.getBoundingClientRect();
  return { x: c.left + layout.x, y: c.top + layout.y };
}

function buttonOf(e: PointerEvent | MouseEvent): 0 | 1 | 2 {
  return e.button === 1 ? 1 : e.button === 2 ? 2 : 0;
}

/**
 * Pointer handling over a terminal body: mouse reporting when the program
 * asked for it (Shift forces local selection), otherwise selection in Rust
 * (double = semantic, triple = line, Alt = block), Cmd/Ctrl+click opens links.
 */
export function attachTermPointer(
  body: HTMLElement,
  opts: {
    id: string;
    focus: () => void;
    /** ⌘⇧-click on a path: open it on the canvas (file node / tree). */
    openInCanvas?: (path: string) => void;
  },
): () => void {
  const id = opts.id;
  let mode: 'none' | 'mouse' | 'select' = 'none';
  let moved = false;
  let wheelAcc = 0;
  const send = (p: Promise<unknown>) => void p.catch((e) => report(id, e));

  const onDown = (e: PointerEvent) => {
    if (e.button !== 0 && e.button !== 1 && e.button !== 2) return;
    e.stopPropagation();
    opts.focus();
    const pos = cellAt(id, body, e.clientX, e.clientY);
    const modes = termLive(id).modes;
    if (e.button === 0 && (e.metaKey || (!IS_MAC && e.ctrlKey))) {
      const grid = termHub.grid(id);
      if (grid) {
        const st = grid.style(grid.styleId[pos.row * grid.cols + pos.col] ?? 0);
        if (st.flags & SF_LINK && st.link) {
          e.preventDefault();
          void termLink(id, st.link)
            .then((uri) => {
              if (uri && /^(https?|mailto|file):/i.test(uri)) return openUrl(uri);
            })
            .catch((err) => report(id, err));
          return;
        }
        // Plain text: a URL, or a path relative to the shell's directory.
        const tok = tokenAt(rowCells(id, pos.row), pos.col);
        if (/^(https?:\/\/|mailto:)\S+$/i.test(tok)) {
          e.preventDefault();
          void openUrl(tok).catch((err) => report(id, err));
          return;
        }
        const abs = resolvePath(tok, termLive(id).cwd);
        if (abs && /[/.]/.test(tok)) {
          e.preventDefault();
          if (e.shiftKey && opts.openInCanvas) opts.openInCanvas(abs);
          else void invoke('open_path_default', { path: abs }).catch((err) => report(id, err));
          return;
        }
      }
    }
    if (modes & MOUSE_MODES && !e.shiftKey) {
      mode = 'mouse';
      body.setPointerCapture(e.pointerId);
      e.preventDefault();
      send(termMouse(id, { kind: 'down', button: buttonOf(e), col: pos.col, row: pos.row, mods: modsOf(e) }));
      return;
    }
    if (e.button !== 0) return;
    e.preventDefault();
    mode = 'select';
    moved = false;
    body.setPointerCapture(e.pointerId);
    const ty: SelectInput['ty'] = e.detail >= 3 ? 'lines' : e.detail === 2 ? 'semantic' : e.altKey ? 'block' : 'simple';
    send(termSelect(id, { kind: 'start', ty, col: pos.col, row: pos.row, side: pos.side }));
    if (e.detail >= 2) moved = true;
  };

  const onMove = (e: PointerEvent) => {
    if (mode === 'none') return;
    const pos = cellAt(id, body, e.clientX, e.clientY);
    if (mode === 'mouse') {
      send(termMouse(id, { kind: 'move', button: buttonOf(e), col: pos.col, row: pos.row, mods: modsOf(e) }));
    } else {
      moved = true;
      send(termSelect(id, { kind: 'update', col: pos.col, row: pos.row, side: pos.side }));
    }
  };

  const onUp = (e: PointerEvent) => {
    if (mode === 'none') return;
    const pos = cellAt(id, body, e.clientX, e.clientY);
    if (mode === 'mouse') {
      send(termMouse(id, { kind: 'up', button: buttonOf(e), col: pos.col, row: pos.row, mods: modsOf(e) }));
    } else if (!moved) {
      send(termSelect(id, { kind: 'clear' }));
    }
    mode = 'none';
    if (body.hasPointerCapture(e.pointerId)) body.releasePointerCapture(e.pointerId);
  };

  const onWheel = (e: WheelEvent) => {
    // Cmd/Ctrl+wheel zooms the canvas; an unfocused terminal lets the canvas pan.
    if (e.ctrlKey || e.metaKey) return;
    if (focused !== id) return;
    e.preventDefault();
    e.stopPropagation();
    const layout = termLayout(id);
    const ch = layout?.cellH ?? cellMetrics().h;
    const px = e.deltaMode === 1 ? e.deltaY * ch : e.deltaMode === 2 ? e.deltaY * ch * 20 : e.deltaY;
    wheelAcc += -px / ch;
    const lines = wheelAcc > 0 ? Math.floor(wheelAcc) : Math.ceil(wheelAcc);
    if (lines === 0) return;
    wheelAcc -= lines;
    const modes = termLive(id).modes;
    if (modes & MOUSE_MODES && !e.shiftKey) {
      const pos = cellAt(id, body, e.clientX, e.clientY);
      send(termMouse(id, { kind: 'wheel', col: pos.col, row: pos.row, wheel: lines, mods: modsOf(e) }));
    } else {
      send(termScroll(id, { lines }));
    }
  };

  const onContext = (e: MouseEvent) => {
    if (termLive(id).modes & MOUSE_MODES && !e.shiftKey) e.preventDefault();
  };

  body.addEventListener('pointerdown', onDown);
  body.addEventListener('pointermove', onMove);
  body.addEventListener('pointerup', onUp);
  body.addEventListener('pointercancel', onUp);
  body.addEventListener('wheel', onWheel, { passive: false });
  body.addEventListener('contextmenu', onContext);
  return () => {
    body.removeEventListener('pointerdown', onDown);
    body.removeEventListener('pointermove', onMove);
    body.removeEventListener('pointerup', onUp);
    body.removeEventListener('pointercancel', onUp);
    body.removeEventListener('wheel', onWheel);
    body.removeEventListener('contextmenu', onContext);
  };
}
