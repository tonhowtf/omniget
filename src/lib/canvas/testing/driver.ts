import { envDrop } from '../floors/api';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { CanvasEngine } from '../engine.svelte';
import { chats } from '../chat/store.svelte';
import { loadDraft, saveDraft, mentionTargets, pathToken } from '../chat/composer';
import { workspaces } from '../palette/workspaces.svelte';
import { revealNode, switchTo } from '../palette/navigate';
import { describe, findPath, menus } from '../menu/model';
import { hitTest } from '../spatial/hit';
import { aiActivity } from '../activity/store.svelte';
import { clampZoom } from '../camera/camera';
import type { CameraState } from '../model/types';

type Args = Record<string, unknown>;

interface NodeHit {
  nodeId: string;
  title: string;
  kind: string;
  x: number;
  y: number;
}

interface CanvasDriver {
  run(op: string, args: Args): Promise<unknown>;
  nodeRect(id: string): { x: number; y: number; w: number; h: number } | null;
  nodeAt(clientX: number, clientY: number): NodeHit | null;
  where(): { workspaceId: string; floorId: string };
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
    const root =
      document.querySelector<HTMLElement>('[data-canvas-root]') ?? document.querySelector<HTMLElement>('canvas.l0');
    if (!n) return null;
    const off = root?.getBoundingClientRect() ?? { x: 0, y: 0 };
    const s = engine.camera.rectToScreen({ x: n.x, y: n.y, w: n.w, h: n.h });
    return { x: s.x + off.x, y: s.y + off.y, w: s.w, h: s.h };
  };

  const rootEl = () =>
    document.querySelector<HTMLElement>('.canvas-root') ?? document.querySelector<HTMLElement>('[data-canvas-root]');

  const nodeAt = (cx: number, cy: number): NodeHit | null => {
    const describeNode = (id: string | undefined): NodeHit | null => {
      const n = id ? engine.store.node(id) : undefined;
      return n ? { nodeId: n.id, title: n.title ?? '', kind: n.kind, x: n.x, y: n.y } : null;
    };
    const el = document.elementFromPoint(cx, cy);
    if (el?.closest('[data-canvas-chrome]')) return null;
    const host = el?.closest<HTMLElement>('[data-node-id]');
    if (host) return describeNode(host.dataset.nodeId);
    const root = rootEl();
    if (!root || !el || !root.contains(el)) return null;
    const off = root.getBoundingClientRect();
    const w = engine.camera.screenToWorld(cx - off.left, cy - off.top);
    const hit = hitTest(engine.store, w.x, w.y, { zoom: engine.camera.zoom });
    return hit.type === 'node' ? describeNode(hit.id) : null;
  };

  const round = (v: number) => Math.round(v * 100) / 100;
  const view = (target: CameraState = engine.cameraTarget()) => {
    const vis = engine.visibleWorld(target);
    const scr = engine.camera.safeRect();
    return {
      workspaceId: engine.store.workspaceId,
      floorId: engine.store.floorId,
      camera: { x: round(vis.x), y: round(vis.y), zoom: round(target.zoom * 1000) / 1000 },
      visibleRect: { x: round(vis.x), y: round(vis.y), w: round(vis.w), h: round(vis.h) },
      visibleScreen: { x: round(scr.x), y: round(scr.y), w: round(scr.w), h: round(scr.h) },
      viewport: { w: engine.camera.vw, h: engine.camera.vh },
    };
  };

  const elsewhere = (a: Args) => {
    const ws = str(a, 'workspaceId');
    const floor = str(a, 'floorId');
    if ((ws && ws !== engine.store.workspaceId) || (floor && floor !== engine.store.floorId)) {
      return { elsewhere: true, workspaceId: engine.store.workspaceId, floorId: engine.store.floorId };
    }
    return null;
  };

  const state = () => ({
    workspaceId: engine.store.workspaceId,
    floorId: engine.store.floorId,
    selection: engine.selection.list(),
    focus: engine.selection.focused,
    editing: engine.selection.editing,
    hovered: engine.selection.hovered,
    camera: engine.camera.state(),
    visibleRect: view().visibleRect,
    viewport: { w: engine.camera.vw, h: engine.camera.vh },
    activity: { open: aiActivity.open, follow: aiActivity.follow ? (aiActivity.followOrigin ?? 'any') : null, presence: aiActivity.presence },
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
        return ok
          ? { focus: engine.selection.focused, selection: engine.selection.list(), ...view() }
          : { error: `CANVAS_NOT_FOUND: ${id} is not on the open floor` };
      }
      case 'activate': {
        const ws = str(a, 'workspaceId');
        if (!ws) return { error: 'MISSING_ARGUMENT: workspaceId' };
        await switchTo(engine, ws, str(a, 'floorId'));
        return { workspaceId: engine.store.workspaceId, floorId: engine.store.floorId };
      }
      case 'camera': {
        const away = elsewhere(a);
        if (away) return away;
        const x = num(a, 'x');
        const y = num(a, 'y');
        const zoom = num(a, 'zoom');
        if (x !== undefined || y !== undefined || zoom !== undefined) {
          const now = engine.cameraTarget();
          const vis = engine.visibleWorld(now);
          const z = clampZoom(zoom ?? now.zoom);
          let target: CameraState;
          if (x === undefined && y === undefined) {
            const c = engine.camera.safeCenter();
            const wx = vis.x + vis.w / 2;
            const wy = vis.y + vis.h / 2;
            target = { x: wx - c.x / z, y: wy - c.y / z, zoom: z };
          } else {
            target = engine.camera.fromVisible(x ?? vis.x, y ?? vis.y, z);
          }
          if (a.animate) engine.flyToCamera(target);
          else {
            engine.camera.set(target);
            engine.invalidate();
          }
        }
        return view();
      }
      case 'fit': {
        const away = elsewhere(a);
        if (away) return away;
        engine.fitAll(false);
        return view();
      }
      case 'nodePos': {
        const n = engine.store.node(str(a, 'nodeId') ?? '');
        return n ? { x: n.x, y: n.y } : null;
      }
      case 'where':
        return { workspaceId: engine.store.workspaceId, floorId: engine.store.floorId };
      case 'view': {
        if (typeof a.activity === 'boolean') {
          if (a.activity) aiActivity.show('activity');
          else aiActivity.open = false;
        }
        if (typeof a.presence === 'boolean') aiActivity.presence = a.presence;
        if ('follow' in a) {
          const f = a.follow;
          if (f === null || f === false) {
            aiActivity.follow = false;
            aiActivity.followOrigin = null;
          } else if (typeof f === 'string' || f === true) {
            aiActivity.follow = true;
            const want = typeof f === 'string' ? f.trim() : '';
            const named = aiActivity.actors().find((x) => x.name.toLowerCase() === want.toLowerCase());
            aiActivity.followOrigin = !want || want === 'any' ? null : (named?.origin ?? want);
          }
        }
        return {
          workspaceId: engine.store.workspaceId,
          floorId: engine.store.floorId,
          activity: aiActivity.open,
          presence: aiActivity.presence,
          follow: aiActivity.follow ? (aiActivity.followOrigin ?? 'any') : null,
          actors: aiActivity.actors().map((x) => ({ origin: x.origin, name: x.name })),
        };
      }
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

  const driver: CanvasDriver = {
    run,
    nodeRect,
    nodeAt,
    where: () => ({ workspaceId: engine.store.workspaceId, floorId: engine.store.floorId }),
  };
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
