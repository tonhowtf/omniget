import { invoke } from '@tauri-apps/api/core';
import type { CanvasEngine } from '../engine.svelte';
import { isTerminalKind, type AgentKind } from '../model/types';
import type { AlignMode } from '../interaction/snap';
import { canvasUi } from '../palette/ui.svelte';
import { workspaces } from '../palette/workspaces.svelte';
import { updateSettings } from '$lib/stores/settings-store.svelte';
import { copySelection, pasteClipboard } from '../clipboard';
import { canvasPrefs } from '../prefs.svelte';
import { agentFiles, openInIde, sidebarUi } from '../sidebar.svelte';
import { placeInStack, stackTargets } from '../notes/stack';
import { aiActivity } from '../activity/store.svelte';

export interface MenuEntry {
  id: string;
  key: string;
  label: string;
  children?: MenuEntry[];
  run?: () => unknown;
  danger?: boolean;
  sepBefore?: boolean;
}

interface MenuContext {
  nodeId?: string;
  wx?: number;
  wy?: number;
  connectFrom?: string;
  free?: boolean;
}

const SPAWN_AGENTS: Array<{ id: string; label: string; kind: string; agent: AgentKind }> = [
  { id: 'shell', label: 'Terminal', kind: 'terminal', agent: 'shell' },
  { id: 'claude', label: 'Claude Code', kind: 'agent', agent: 'claude' },
  { id: 'codex', label: 'Codex', kind: 'agent', agent: 'codex' },
  { id: 'gemini', label: 'Gemini', kind: 'agent', agent: 'gemini' },
  { id: 'opencode', label: 'OpenCode', kind: 'agent', agent: 'opencode' },
  { id: 'ollama', label: 'Ollama', kind: 'agent', agent: 'ollama' },
];

const ALIGN: Array<{ id: AlignMode; label: string }> = [
  { id: 'left', label: 'Align Left' },
  { id: 'hcenter', label: 'Center Horizontally' },
  { id: 'right', label: 'Align Right' },
  { id: 'top', label: 'Align Top' },
  { id: 'vcenter', label: 'Center Vertically' },
  { id: 'bottom', label: 'Align Bottom' },
  { id: 'hdistribute', label: 'Distribute Horizontally' },
  { id: 'vdistribute', label: 'Distribute Vertically' },
];

function e(id: string, key: string, label: string, run?: () => unknown, extra: Partial<MenuEntry> = {}): MenuEntry {
  return { id, key, label, run, ...extra };
}

export function addMenu(engine: CanvasEngine, ctx: MenuContext): MenuEntry[] {
  const at = ctx.free || ctx.wx === undefined || ctx.wy === undefined ? null : { x: ctx.wx, y: ctx.wy };
  const spawn = (kind: string, agent?: AgentKind) => () => {
    const node = engine.spawn(kind, at, { agent, connectFrom: ctx.connectFrom });
    if (kind === 'text' || kind === 'note' || kind === 'terminal' || kind === 'agent') engine.selection.focus(node.id);
    return { nodeId: node.id };
  };
  return [
    e('terminal', 'canvas.menu.add.terminal', 'Terminal', undefined, {
      children: [
        ...SPAWN_AGENTS.map((a) => e(a.id, `canvas.engine.spawn.${a.id === 'shell' ? 'terminal' : a.id}`, a.label, spawn(a.kind, a.agent))),
        ...Object.keys(canvasPrefs().agent_presets).map((name, i) =>
          e(`preset:${name}`, name, name, () => {
            const node = engine.spawn('terminal', at, { preset: name, connectFrom: ctx.connectFrom });
            engine.selection.focus(node.id);
            return { nodeId: node.id };
          }, { sepBefore: i === 0 }),
        ),
      ],
    }),
    e('note', 'canvas.engine.spawn.note', 'Note', spawn('note')),
    e('text', 'canvas.engine.spawn.text', 'Text', spawn('text')),
    e('portal', 'canvas.engine.spawn.portal', 'Portal', spawn('portal')),
    e('filetree', 'canvas.menu.add.filetree', 'File Tree', spawn('filetree')),
    e('paste', 'canvas.clipboard.paste', 'Paste', async () => ({ nodeIds: await pasteClipboard(engine, at ?? undefined) }), { sepBefore: true }),
  ];
}

const SIZE_SETTING: Record<string, string> = { note: 'note', terminal: 'terminal', agent: 'terminal', text: 'text', portal: 'portal' };

export function setBlur(engine: CanvasEngine, nodeId: string, on: boolean): void {
  const sel = engine.selection.list().filter((id) => engine.store.node(id));
  const ids = sel.includes(nodeId) ? sel : [nodeId];
  const ops = ids.map((id) => ({ op: 'update_node' as const, id, patch: { data: { blur: on ? true : null } } as never }));
  if (ops.length) engine.commit(ops);
}

export function openLinkInPortal(engine: CanvasEngine, nodeId: string): { nodeId: string } | null {
  const n = engine.store.node(nodeId);
  const url = n && typeof n.data.url === 'string' ? n.data.url : '';
  if (!n || !url) return null;
  const portal = engine.spawn('portal', { x: n.x + n.w + 40, y: n.y }, { data: { url } });
  engine.selection.set([portal.id]);
  return { nodeId: portal.id };
}

export function nodeMenu(engine: CanvasEngine, nodeId: string): MenuEntry[] {
  const node = engine.store.node(nodeId);
  if (!node) return [];
  const terminal = isTerminalKind(node.kind);
  const p = engine.panels;
  const out: MenuEntry[] = [
    e('focus', 'canvas.engine.menu.focus', 'Focus', () => engine.enterSelected()),
    p.lifted === nodeId
      ? e('putBack', 'canvas.panels.putBack', 'Put back on the canvas', () => p.lift(null))
      : e('lift', 'canvas.panels.liftElement', 'Lift element', () => p.lift(nodeId)),
    p.dockOf(nodeId)
      ? e('undock', 'canvas.panels.undock', 'Undock', () => p.dock(nodeId, 'none'))
      : e('dock', 'canvas.panels.dock', 'Dock', undefined, {
          children: [
            e('left', 'canvas.panels.dockLeft', 'Dock left', () => p.dock(nodeId, 'left')),
            e('right', 'canvas.panels.dockRight', 'Dock right', () => p.dock(nodeId, 'right')),
          ],
        }),
    e('duplicate', 'canvas.engine.duplicate', 'Duplicate', () => engine.duplicateSelection(), { sepBefore: true }),
    e('copy', 'canvas.clipboard.copy', 'Copy', () => copySelection(engine)),
    e('cut', 'canvas.clipboard.cut', 'Cut', () => copySelection(engine, true)),
    e('front', 'canvas.engine.front', 'Bring to front', () => engine.zOrder('front')),
    e('back', 'canvas.engine.back', 'Send to back', () => engine.zOrder('back')),
    e('lock', 'canvas.engine.lock', node.locked ? 'Unlock' : 'Lock', () => engine.toggleLock()),
    node.data.blur
      ? e('unblur', 'canvas.menu.removeBlur', 'Remove Blur', () => setBlur(engine, nodeId, false))
      : e('blur', 'canvas.menu.blur', 'Blur', () => setBlur(engine, nodeId, true)),
  ];
  if (node.kind === 'note') {
    const stackName = (n: { title?: string; data: Record<string, unknown> }) =>
      String(n.title || (n.data.name as string) || 'Fichário');
    out.push(
      e('placeInStack', 'canvas.menu.placeInStack', 'Place in Fichário', undefined, {
        children: [
          ...stackTargets(engine.store, nodeId).map((st) =>
            e(st.id, stackName(st), stackName(st), () => placeInStack(engine, nodeId, st.id)),
          ),
          e('newStack', 'canvas.menu.placeInNewStack', 'New Fichário', () => placeInStack(engine, nodeId, null), { sepBefore: true }),
        ],
      }),
    );
  }
  if (node.kind === 'link' && typeof node.data.url === 'string' && node.data.url) {
    out.push(e('openInPortal', 'canvas.menu.openInPortal', 'Open in Portal', () => openLinkInPortal(engine, nodeId)));
  }
  const sizeKey = SIZE_SETTING[node.kind];
  if (sizeKey) {
    out.push(
      e('useSizeAsDefault', 'canvas.menu.useSizeAsDefault', 'Use Size as Default', () =>
        updateSettings({ canvas: { defaults: { [sizeKey]: { size: `${Math.round(node.w)}x${Math.round(node.h)}` } } } }),
      ),
    );
  }
  if (terminal) {
    out.push(
      e('role', 'canvas.engine.menu.role', 'Role…', () => (engine.panel = { kind: 'role-picker', nodeId }), { sepBefore: true }),
      e('routine', 'canvas.engine.menu.routine', 'New routine…', () => (engine.panel = { kind: 'routine', targetNodeId: nodeId })),
      e('mcp', 'canvas.mcpServers.menu', 'MCP servers…', () => aiActivity.show('mcp', nodeId)),
    );
  }
  out.push(e('delete', 'canvas.engine.delete', 'Delete', () => engine.deleteSelection(), { danger: true, sepBefore: true }));
  return out;
}

export function viewMenu(engine: CanvasEngine): MenuEntry[] {
  const selected = () => engine.selection.list().filter((id) => engine.store.node(id));
  return [
    e('tidy', 'canvas.palette.action.tidy', 'Tidy', () => {
      const ids = selected();
      const target = ids.length > 1 ? ids : engine.store.nodes.filter((n) => !n.stackId).map((n) => n.id);
      return invoke('canvas_layout_arrange', {
        workspaceId: engine.store.workspaceId,
        floorId: engine.store.floorId,
        ids: target,
        mode: 'tidy',
        origin: 'user',
      });
    }),
    e('align', 'canvas.menu.align', 'Align', undefined, {
      children: ALIGN.map((a) => e(a.id, `canvas.menu.alignTo.${a.id}`, a.label, () => engine.align(a.id))),
    }),
    e('zoomToSelection', 'canvas.menu.zoomToSelection', 'Zoom to Selection', () => {
      const b = engine.selectionBounds();
      if (b) engine.flyToRect(b, 1.5);
      else engine.fitAll();
    }, { sepBefore: true }),
    e('zoomToFit', 'canvas.menu.zoomToFit', 'Zoom to Fit', () => engine.fitAll()),
    e('toggleMinimap', 'canvas.menu.toggleMinimap', 'Toggle Minimap', () => (engine.minimapVisible = !engine.minimapVisible)),
    e('toggleSidebar', 'canvas.menu.toggleSidebar', 'Toggle Sidebar', () => canvasUi.toggleSidebar()),
    e('toggleMiniSidebar', 'canvas.menu.toggleMiniSidebar', 'Toggle Mini Sidebar', () => {
      sidebarUi.toggleMini();
      return { mini: sidebarUi.mini, sidebarOpen: canvasUi.sidebarOpen };
    }),
    e('liftElement', 'canvas.panels.liftElement', 'Lift element', () => {
      const id = engine.selection.focused ?? selected()[0];
      if (id) engine.panels.toggleLift(id);
    }, { sepBefore: true }),
    e('putBackAll', 'canvas.menu.putBackAll', 'Put Back All Panels', () => engine.panels.clear()),
    e('resetFocus', 'canvas.menu.resetFocus', 'Reset Focus', () => resetFocus(engine), { sepBefore: true }),
  ];
}

function workspaceMenu(engine: CanvasEngine): MenuEntry[] {
  const ws = workspaces.byId(engine.store.workspaceId);
  return [
    e('copyPath', 'canvas.menu.copyPath', 'Copy Path', async () => {
      const dir = ws?.workingDirectory ?? '';
      if (!dir) return { copied: null };
      try {
        const { writeText } = await import('@tauri-apps/plugin-clipboard-manager');
        await writeText(dir);
      } catch {
        await navigator.clipboard?.writeText(dir).catch(() => {});
      }
      return { copied: dir };
    }),
    e('openInIde', 'canvas.menu.openInIde', 'Open in IDE', async () => {
      if (!ws?.workingDirectory) return { error: 'the workspace has no working folder' };
      return openInIde(ws.id);
    }),
    e('syncAgentFiles', 'canvas.menu.syncAgentFiles', 'Sync Agent Files', async () => {
      if (!ws) return { error: 'no workspace' };
      return agentFiles(ws.id, 'CLAUDE.md', true);
    }),
  ];
}

function resetFocus(engine: CanvasEngine): void {
  if (engine.tools.busy) engine.tools.cancel();
  engine.tools.spaceHeld = false;
  engine.tools.setCursor('');
  engine.setTool('select');
  engine.spawnMenu = null;
  engine.panel = null;
  engine.draft = {};
  engine.pinching = false;
  engine.exitFocus();
  engine.invalidate();
}

export function menus(engine: CanvasEngine, ctx: MenuContext = {}): MenuEntry[] {
  const top: MenuEntry[] = [];
  const nodeId = ctx.nodeId ?? (engine.selection.size === 1 ? engine.selection.list()[0] : undefined);
  if (nodeId && engine.store.node(nodeId)) top.push(e('node', 'canvas.menu.node', 'Node', undefined, { children: nodeMenu(engine, nodeId) }));
  top.push(e('add', 'canvas.engine.menu.add', 'Add', undefined, { children: addMenu(engine, ctx) }));
  top.push(e('view', 'canvas.menu.view', 'View', undefined, { children: viewMenu(engine) }));
  top.push(e('workspace', 'canvas.menu.workspace', 'Workspace', undefined, { children: workspaceMenu(engine) }));
  return top;
}

const clean = (s: string) => s.toLowerCase().replace(/[….]+$/g, '').replace(/\s+/g, ' ').trim();

function match(list: MenuEntry[], seg: string): MenuEntry | undefined {
  const s = clean(seg);
  return list.find((m) => clean(m.label) === s || m.id.toLowerCase() === s) ?? list.find((m) => clean(m.label).startsWith(s));
}

export function findPath(list: MenuEntry[], path: string): MenuEntry | null {
  const segs = path
    .split(/>|›|\//)
    .map((x) => x.trim())
    .filter(Boolean);
  let level = list;
  let found: MenuEntry | undefined;
  for (const seg of segs) {
    found = match(level, seg);
    if (!found) {
      for (const m of level) {
        found = m.children ? match(m.children, seg) : undefined;
        if (found) break;
      }
    }
    if (!found) return null;
    level = found.children ?? [];
  }
  return found ?? null;
}

export function describe(list: MenuEntry[]): unknown[] {
  return list.map((m) => ({
    id: m.id,
    label: m.label,
    ...(m.danger ? { danger: true } : {}),
    ...(m.children ? { children: describe(m.children) } : {}),
  }));
}
