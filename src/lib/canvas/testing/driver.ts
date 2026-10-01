// UI half of the MCP canvas tools (`src-tauri/src/mcp/canvas_ui.rs`): the
// operations only the mounted canvas can do (live selection, camera, undo,
// chat faces, the palette and the composer). Rust evaluates a fixed runner in
// the main window; the runner hands canvas operations to this driver and emits
// the result back as `mcp-ui://reply`. Nothing here takes code from outside:
// each operation is a named case with plain arguments.
//
// It also follows `canvas://navigate {workspaceId, floorId, nodeId?}` (an MCP
// client switched floors or opened a portal) by flying there.

import { envDrop } from '../floors/api';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { CanvasEngine } from '../engine.svelte';
import { chats } from '../chat/store.svelte';
import { loadDraft, saveDraft, mentionTargets, pathToken } from '../chat/composer';
import { workspaces } from '../palette/workspaces.svelte';
import { revealNode, switchTo } from '../palette/navigate';
import { describe, findPath, menus } from '../menu/model';

type Args = Record<string, unknown>;

interface CanvasDriver {
  run(op: string, args: Args): Promise<unknown>;
  nodeRect(id: string): { x: number; y: number; w: number; h: number } | null;
}

declare global {
  interface Window {
    __omnigetCanvasDriver?: CanvasDriver;
  }
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const mac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform);

function str(a: Args, k: string): string | undefined {
  const v = a[k];
  return typeof v === 'string' && v.trim() ? v : undefined;
}

function num(a: Args, k: string): number | undefined {
  const v = a[k];
  return typeof v === 'number' && Number.isFinite(v) ? v : undefined;
}

function ids(a: Args, k: string): string[] {
  const v = a[k];
  if (Array.isArray(v)) return v.filter((x): x is string => typeof x === 'string');
  return typeof v === 'string' ? [v] : [];
}

/** Sets the value of a Svelte-bound input/textarea the way typing does. */
function typeInto(el: HTMLInputElement | HTMLTextAreaElement, text: string, append = true): void {
  el.focus();
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;
  const next = append ? el.value + text : text;
  setter?.call(el, next);
  el.setSelectionRange(next.length, next.length);
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

function press(el: Element | Window, key: string, mods: { meta?: boolean; ctrl?: boolean; shift?: boolean; alt?: boolean } = {}): void {
  const code = key.length === 1 && /[a-z]/i.test(key) ? `Key${key.toUpperCase()}` : key;
  const init: KeyboardEventInit = {
    key,
    code,
    bubbles: true,
    cancelable: true,
    metaKey: !!mods.meta,
    ctrlKey: !!mods.ctrl,
    shiftKey: !!mods.shift,
    altKey: !!mods.alt,
  };
  el.dispatchEvent(new KeyboardEvent('keydown', init));
  el.dispatchEvent(new KeyboardEvent('keyup', init));
}

const primary = () => (mac ? { meta: true } : { ctrl: true });

function paletteEl(): HTMLElement | null {
  return document.querySelector<HTMLElement>('.palette[role="dialog"]');
}

function paletteRows(): { index: number; label: string; detail: string; active: boolean }[] {
  return Array.from(document.querySelectorAll<HTMLElement>('#batuta-list .row')).map((r, index) => ({
    index,
    label: r.querySelector('.label')?.textContent?.trim() ?? r.textContent?.trim() ?? '',
    detail: r.querySelector('.detail')?.textContent?.trim() ?? '',
    active: r.getAttribute('aria-selected') === 'true',
  }));
}

function composerEl(): HTMLElement | null {
  return document.querySelector<HTMLElement>('.composer');
}

function composerArea(): HTMLTextAreaElement | null {
  return document.querySelector<HTMLTextAreaElement>('.composer textarea');
}

export function installCanvasDriver(engine: CanvasEngine): () => void {
  const nodeRect = (id: string) => {
    const el = document.querySelector<HTMLElement>(`[data-node-id="${CSS.escape(id)}"]`);
    if (el) {
      const r = el.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) return { x: r.x, y: r.y, w: r.width, h: r.height };
    }
    const n = engine.store.node(id);
    // The GL canvas spans the camera viewport: its origin is the screen origin.
    const root =
      document.querySelector<HTMLElement>('[data-canvas-root]') ?? document.querySelector<HTMLElement>('canvas.l0');
    if (!n) return null;
    const off = root?.getBoundingClientRect() ?? { x: 0, y: 0 };
    const s = engine.camera.rectToScreen({ x: n.x, y: n.y, w: n.w, h: n.h });
    return { x: s.x + off.x, y: s.y + off.y, w: s.w, h: s.h };
  };

  const state = () => ({
    workspaceId: engine.store.workspaceId,
    floorId: engine.store.floorId,
    selection: engine.selection.list(),
    focus: engine.selection.focused,
    editing: engine.selection.editing,
    hovered: engine.selection.hovered,
    camera: engine.camera.state(),
    viewport: { w: engine.camera.vw, h: engine.camera.vh },
    tool: engine.tool,
    panel: engine.panel,
    chatFaces: engine.store.nodes.filter((n) => chats.isChat(n.id)).map((n) => n.id),
    palette: paletteEl() ? { open: true, query: paletteEl()?.querySelector<HTMLInputElement>('.search input')?.value ?? '' } : { open: false },
    composer: composerEl() ? { open: true, text: composerArea()?.value ?? '' } : { open: false },
    ...engine.panels.snapshot(),
  });

  const run = async (op: string, a: Args): Promise<unknown> => {
    switch (op) {
      case 'state':
        return state();
      case 'nodeRect': {
        const id = str(a, 'nodeId');
        return id ? nodeRect(id) : null;
      }
      case 'select': {
        const list = ids(a, 'nodeIds');
        engine.selection.set(list);
        engine.invalidate();
        return { selection: engine.selection.list() };
      }
      case 'focus': {
        const id = str(a, 'nodeId');
        if (!id) return { error: 'MISSING_ARGUMENT: nodeId' };
        const ws = str(a, 'workspaceId');
        const floor = str(a, 'floorId');
        if (ws && (ws !== engine.store.workspaceId || (floor && floor !== engine.store.floorId))) {
          await switchTo(engine, ws, floor);
        }
        const ok = revealNode(engine, id, { focus: a.focus !== false });
        return ok ? { focus: engine.selection.focused, selection: engine.selection.list() } : { error: `CANVAS_NOT_FOUND: ${id} is not on the open floor` };
      }
      case 'activate': {
        const ws = str(a, 'workspaceId');
        if (!ws) return { error: 'MISSING_ARGUMENT: workspaceId' };
        await switchTo(engine, ws, str(a, 'floorId'));
        return { workspaceId: engine.store.workspaceId, floorId: engine.store.floorId };
      }
      case 'camera': {
        const x = num(a, 'x');
        const y = num(a, 'y');
        const zoom = num(a, 'zoom');
        if (x !== undefined || y !== undefined || zoom !== undefined) {
          const target = { ...engine.camera.state() };
          if (x !== undefined) target.x = x;
          if (y !== undefined) target.y = y;
          if (zoom !== undefined) target.zoom = zoom;
          if (a.animate) engine.flyToCamera(target);
          else {
            engine.camera.set(target);
            engine.invalidate();
          }
          if (a.animate) await sleep(600);
        }
        return { camera: engine.camera.state(), viewport: { w: engine.camera.vw, h: engine.camera.vh } };
      }
      case 'fit':
        engine.fitAll(false);
        return { camera: engine.camera.state() };
      case 'undo':
        engine.undo();
        return { ok: true };
      case 'redo':
        engine.redo();
        return { ok: true };
      case 'chatFace': {
        const id = str(a, 'nodeId');
        if (!id) return { error: 'MISSING_ARGUMENT: nodeId' };
        chats.setFace(id, a.on !== false);
        return { nodeId: id, chat: chats.isChat(id) };
      }
      case 'paletteOpen': {
        if (!paletteEl()) press(window, 'k', primary());
        await sleep(120);
        return { open: !!paletteEl(), rows: paletteRows() };
      }
      case 'paletteQuery': {
        if (!paletteEl()) press(window, 'k', primary());
        await sleep(80);
        const input = paletteEl()?.querySelector<HTMLInputElement>('.search input');
        if (!input) return { error: 'UI_PALETTE_NOT_OPEN' };
        typeInto(input, String(a.text ?? ''), false);
        await sleep(num(a, 'waitMs') ?? 250);
        return { query: input.value, rows: paletteRows() };
      }
      case 'paletteRun': {
        if (typeof a.text === 'string') {
          const r = (await run('paletteQuery', { text: a.text })) as { error?: string };
          if (r?.error) return r;
        }
        const rows = Array.from(document.querySelectorAll<HTMLElement>('#batuta-list .row'));
        const i = num(a, 'index') ?? 0;
        const row = rows[i];
        if (!row) return { error: `UI_PALETTE_NO_ROW: ${i} (${rows.length} rows)` };
        const label = row.querySelector('.label')?.textContent?.trim() ?? '';
        row.click();
        await sleep(200);
        return { ran: label, open: !!paletteEl() };
      }
      case 'paletteClose': {
        if (paletteEl()) press(window, 'Escape');
        await sleep(80);
        return { open: !!paletteEl() };
      }
      case 'composerOpen': {
        const id = str(a, 'nodeId') ?? engine.selection.focused ?? undefined;
        if (!id) return { error: 'MISSING_ARGUMENT: nodeId (or a focused terminal)' };
        revealNode(engine, id, { focus: true });
        await sleep(60);
        if (!composerEl()) press(window, 'p', { ...primary(), shift: true });
        await sleep(150);
        return { open: !!composerEl(), target: engine.selection.focused, text: composerArea()?.value ?? '' };
      }
      case 'composerType': {
        const ta = composerArea();
        if (!ta) return { error: 'UI_COMPOSER_NOT_OPEN: canvas_composer_open first' };
        typeInto(ta, String(a.text ?? ''), a.replace !== true);
        // The # picker lists files over IPC after a short debounce.
        await sleep(/(^|\s)#[^\s#]*$/.test(ta.value) ? 400 : 120);
        return { text: ta.value, mentions: Array.from(document.querySelectorAll('.composer .mentions .opt .oname')).map((e) => e.textContent?.trim() ?? '') };
      }
      case 'composerMentions': {
        const id = str(a, 'nodeId') ?? engine.selection.focused;
        if (!id) return { error: 'MISSING_ARGUMENT: nodeId' };
        return {
          nodeId: id,
          targets: mentionTargets(id, engine.store.nodes, engine.store.connections, {
            workspaceId: engine.store.workspaceId,
            floorId: engine.store.floorId,
            workspaces: workspaces.list,
          }),
        };
      }
      case 'composerPaste': {
        // Same path as a real paste (pills over 800 chars or 3+ lines).
        const ta = composerArea();
        if (!ta) return { error: 'UI_COMPOSER_NOT_OPEN: canvas_composer_open first' };
        ta.focus();
        ta.dispatchEvent(new CustomEvent('omniget:paste', { detail: { text: String(a.text ?? '') } }));
        await sleep(120);
        const id = engine.selection.focused;
        const draft = id ? loadDraft(engine.store.workspaceId, id) : null;
        return { text: ta.value, pastes: draft?.pastes.map((p) => ({ n: p.n, lines: p.lines, chars: p.text.length })) ?? [] };
      }
      case 'composerAttach': {
        const ta = composerArea();
        const path = str(a, 'path');
        if (!path) return { error: 'MISSING_ARGUMENT: path' };
        if (!ta) return { error: 'UI_COMPOSER_NOT_OPEN: canvas_composer_open first' };
        // Like a pasted file: a remote terminal gets a copy in /tmp/omniget-drops there.
        const target = engine.selection.focused;
        let typed = path;
        if (target) {
          try {
            typed = (await envDrop(target, [path]))[0] ?? path;
          } catch (e) {
            return { error: String(e) };
          }
        }
        typeInto(ta, (ta.value && !ta.value.endsWith(' ') ? ' ' : '') + pathToken(typed) + ' ');
        return { text: ta.value, path: typed };
      }
      case 'composerSend': {
        const ta = composerArea();
        if (!ta) return { error: 'UI_COMPOSER_NOT_OPEN' };
        const text = ta.value;
        press(ta, 'Enter');
        await sleep(300);
        return { sent: text, open: !!composerEl() };
      }
      case 'composerClose': {
        const ta = composerArea();
        if (ta) press(ta, 'Escape');
        await sleep(80);
        return { open: !!composerEl() };
      }
      case 'composerDraft': {
        const id = str(a, 'nodeId') ?? engine.selection.focused;
        if (!id) return { error: 'MISSING_ARGUMENT: nodeId' };
        const ws = engine.store.workspaceId;
        if (typeof a.text === 'string') {
          saveDraft(ws, id, { ...loadDraft(ws, id), text: a.text });
          // An open composer on this node holds the draft in memory and would
          // save it back: set its field too (an empty text clears it).
          const ta = composerArea();
          if (ta && engine.selection.focused === id) {
            ta.value = a.text;
            ta.dispatchEvent(new Event('input', { bubbles: true }));
          }
        }
        return { nodeId: id, draft: loadDraft(ws, id) };
      }
      case 'menu': {
        const at = a.at as { x?: number; y?: number } | undefined;
        const c = engine.camera.center();
        const ctx = {
          nodeId: str(a, 'nodeId'),
          wx: typeof at?.x === 'number' ? at.x : Math.round(c.x),
          wy: typeof at?.y === 'number' ? at.y : Math.round(c.y),
        };
        if (ctx.nodeId && !engine.store.node(ctx.nodeId)) return { error: `CANVAS_NOT_FOUND: node ${ctx.nodeId} is not on the open floor` };
        if (ctx.nodeId && !engine.selection.has(ctx.nodeId)) engine.selection.set([ctx.nodeId]);
        const top = menus(engine, ctx);
        const path = str(a, 'path');
        if (!path || a.list === true) {
          const entry = path ? findPath(top, path) : null;
          if (path && !entry) return { error: `MENU_NOT_FOUND: ${path}` };
          return { path: path ?? null, menus: describe(entry ? (entry.children ?? [entry]) : top) };
        }
        const entry = findPath(top, path);
        if (!entry) return { error: `MENU_NOT_FOUND: ${path}`, menus: describe(top) };
        if (entry.children) return { path, menus: describe(entry.children) };
        if (!entry.run) return { error: `MENU_DISABLED: ${path}` };
        let result = await entry.run();
        if (result && typeof result === 'object' && 'schema_version' in result) result = { settingsUpdated: true };
        await sleep(120);
        engine.invalidate();
        return { invoked: path, id: entry.id, label: entry.label, result: result ?? null, selection: engine.selection.list(), ...engine.panels.snapshot() };
      }
      default:
        return { error: `UI_UNKNOWN_OP: ${op}` };
    }
  };

  const driver: CanvasDriver = { run, nodeRect };
  window.__omnigetCanvasDriver = driver;

  let unlisten: UnlistenFn | null = null;
  let stopped = false;
  void listen<{ workspaceId?: string; floorId?: string; nodeId?: string | null }>('canvas://navigate', async (e) => {
    const p = e.payload;
    if (!p?.workspaceId) return;
    await switchTo(engine, p.workspaceId, p.floorId ?? undefined).catch(() => {});
    if (p.nodeId) revealNode(engine, p.nodeId);
  }).then((u) => {
    if (stopped) u();
    else unlisten = u;
  });

  return () => {
    stopped = true;
    unlisten?.();
    if (window.__omnigetCanvasDriver === driver) delete window.__omnigetCanvasDriver;
  };
}
