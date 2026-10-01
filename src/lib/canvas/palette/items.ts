// Batuta index (study 95 §J, 03 §10): every node of every workspace and floor,
// note bodies, roles, workspaces, floors, and the global and contextual
// actions. The current floor is read live from the engine; the rest is loaded
// when the palette opens and cached briefly.

import { invoke } from '@tauri-apps/api/core';
import type { CanvasEngine } from '../engine.svelte';
import { isTerminalKind, terminalIdOf, type AgentKind, type CanvasNode, type Snapshot } from '../model/types';
import { agentStates, type AgentState } from '../state/agents.svelte';
import { noteList, noteRead, stackIdOf } from '../notes/api';
import { fuzzy, splitQuery, type Field } from './fuzzy';
import { workspaces, type WorkspaceSummary } from './workspaces.svelte';

export type Section = 'context' | 'actions' | 'attention' | 'nodes' | 'notes' | 'chats' | 'workspaces' | 'floors' | 'roles' | 'partituras' | 'files';

export type Tr = (key: string, fallback: string, vars?: Record<string, unknown>) => string;

export interface PaletteItem {
  id: string;
  section: Section;
  label: string;
  detail?: string;
  /** Icon key: a node kind, 'workspace', 'floor', 'role', 'action'... */
  icon: string;
  /** Searchable, not shown (note body, text, aliases). */
  body?: string;
  aliases?: string;
  keys?: string;
  workspaceId?: string;
  floorId?: string;
  nodeId?: string;
  state?: AgentState;
  /** Extra score (current workspace, attention). */
  boost?: number;
  /** Keep the palette open and go to another mode. */
  mode?:
    | 'ask'
    | 'check'
    | 'connect'
    | 'disconnect'
    | 'files'
    | 'attention'
    | 'rename-note'
    | 'edit-terminal'
    | 'workspaces'
    | 'partitura-new'
    | 'partitura-update';
  run?: () => void | Promise<void>;
  /** Mode-specific data (a file hit, a chat hit, a partitura id). */
  payload?: unknown;
}

export interface Ranked {
  item: PaletteItem;
  score: number;
  nameIdx: number[];
}

interface RemoteNode {
  node: CanvasNode;
  workspaceId: string;
  floorId: string;
}

export interface Role {
  id: string;
  name: string;
  prompt?: string;
  icon?: string;
  color?: string;
  agent?: AgentKind;
}

const CACHE_MS = 10_000;
const MAX_FLOORS = 60;
const MAX_NOTE_READS = 200;

export class PaletteIndex {
  remote: RemoteNode[] = [];
  noteText = new Map<string, string>();
  roles: Role[] = [];
  private loadedAt = 0;
  private loading: Promise<void> | null = null;

  /** Loads other floors, note bodies and roles (cached for a few seconds). */
  load(engine: CanvasEngine, force = false): Promise<void> {
    if (!force && performance.now() - this.loadedAt < CACHE_MS) return Promise.resolve();
    if (this.loading) return this.loading;
    this.loading = this.doLoad(engine).finally(() => {
      this.loading = null;
      this.loadedAt = performance.now();
    });
    return this.loading;
  }

  private async doLoad(engine: CanvasEngine): Promise<void> {
    if (!workspaces.loaded) await workspaces.refresh();
    const curWs = engine.store.workspaceId;
    const curFloor = engine.store.floorId;
    const jobs: Array<Promise<void>> = [];
    const remote: RemoteNode[] = [];
    let floors = 0;
    for (const ws of workspaces.list) {
      for (const f of ws.floors) {
        if (ws.id === curWs && f.id === curFloor) continue;
        if (++floors > MAX_FLOORS) break;
        jobs.push(
          invoke<Snapshot>('canvas_load', { workspaceId: ws.id, floorId: f.id })
            .then((s) => {
              for (const node of s?.nodes ?? []) remote.push({ node: { ...node, data: node.data ?? {} }, workspaceId: ws.id, floorId: f.id });
            })
            .catch(() => {}),
        );
      }
    }
    // Note bodies: full text for the current workspace, previews elsewhere.
    const texts = new Map<string, string>();
    for (const ws of workspaces.list) {
      jobs.push(
        noteList(ws.id).then(async (list) => {
          let reads = 0;
          for (const n of list) {
            if (!n.nodeId) continue;
            texts.set(n.nodeId, n.preview);
            if (ws.id === curWs && n.floorId && reads < MAX_NOTE_READS) {
              reads++;
              try {
                const c = await noteRead(ws.id, n.floorId, n.nodeId);
                texts.set(n.nodeId, c.text);
              } catch {
                // Keep the preview.
              }
            }
          }
        }),
      );
    }
    jobs.push(
      invoke<Role[]>('canvas_roles_list', { workspaceId: curWs })
        .then((r) => {
          this.roles = Array.isArray(r) ? r : [];
        })
        .catch(() => {
          this.roles = [];
        }),
    );
    await Promise.all(jobs);
    this.remote = remote;
    this.noteText = texts;
  }
}

function wsName(id: string): string {
  return workspaces.byId(id)?.name ?? '';
}

function floorName(wsId: string, floorId: string): string {
  const ws = workspaces.byId(wsId);
  return ws?.floors.find((f) => f.id === floorId)?.name ?? floorId;
}

function nodeLabel(n: CanvasNode): string {
  if (n.title) return n.title;
  if (n.kind === 'text') return String(n.data.text ?? '').split('\n')[0].slice(0, 60) || 'Text';
  if (n.kind === 'file') return String(n.data.displayName ?? n.data.path ?? 'File').split(/[\\/]/).pop() ?? 'File';
  if (n.kind === 'link') return String(n.data.title ?? n.data.url ?? 'Link');
  if (n.kind === 'portal') return String(n.data.name ?? n.data.url ?? 'Portal');
  return n.kind;
}

function nodeBody(n: CanvasNode, index: PaletteIndex): string {
  if (n.kind === 'note') return index.noteText.get(n.id) ?? String(n.data.text ?? '');
  if (n.kind === 'text') return String(n.data.text ?? '');
  if (n.kind === 'link' || n.kind === 'portal') return String(n.data.url ?? '');
  if (n.kind === 'file') return String(n.data.path ?? '');
  if (n.kind === 'filetree') return String(n.data.rootPath ?? '');
  if (isTerminalKind(n.kind)) return `${String(n.data.agent ?? '')} ${String(n.data.cwd ?? '')}`;
  return '';
}

/** Nodes of the current floor + everything loaded from elsewhere. */
export function nodeItems(engine: CanvasEngine, index: PaletteIndex, tr: Tr): PaletteItem[] {
  const out: PaletteItem[] = [];
  const ws = engine.store.workspaceId;
  const floor = engine.store.floorId;
  const add = (n: CanvasNode, w: string, f: string, local: boolean) => {
    if (n.kind === 'stack' && !n.title) return;
    const where = local ? tr('canvas.palette.here', 'This floor') : `${wsName(w)} · ${floorName(w, f)}`;
    const kindLabel = tr(`canvas.palette.kind.${n.kind}`, n.kind);
    const state = isTerminalKind(n.kind) ? agentStates.stateOf(n) : undefined;
    out.push({
      id: `node:${w}:${f}:${n.id}`,
      section: n.kind === 'note' ? 'notes' : 'nodes',
      label: nodeLabel(n),
      detail: stackIdOf(n) ? `${kindLabel} · ${tr('canvas.palette.inStack', 'in a fichário')} · ${where}` : `${kindLabel} · ${where}`,
      icon: n.kind,
      body: nodeBody(n, index),
      aliases: `${kindLabel} ${n.kind} ${String(n.data.agent ?? '')} ${wsName(w)}`,
      workspaceId: w,
      floorId: f,
      nodeId: n.id,
      state,
      boost: (w === ws ? 6 : 0) + (local ? 4 : 0) + (state === 'blocked' ? 3 : 0),
    });
  };
  for (const n of engine.store.nodes) add(n, ws, floor, true);
  // The cache may predate a workspace switch: the open floor is read live.
  for (const r of index.remote) if (!(r.workspaceId === ws && r.floorId === floor)) add(r.node, r.workspaceId, r.floorId, false);
  return out;
}

/** Terminals of every workspace in the Blocked state (⌥⌘A, Notifications). */
export function attentionItems(engine: CanvasEngine, index: PaletteIndex, tr: Tr): PaletteItem[] {
  return nodeItems(engine, index, tr)
    .filter((i) => i.state === 'blocked')
    .map((i) => ({ ...i, section: 'attention' as const, id: `attention:${i.id}` }));
}

export function workspaceItems(engine: CanvasEngine, tr: Tr, onSwitch: (ws: WorkspaceSummary, floorId?: string) => void): PaletteItem[] {
  const out: PaletteItem[] = [];
  workspaces.list.forEach((ws, i) => {
    const current = ws.id === engine.store.workspaceId;
    out.push({
      id: `ws:${ws.id}`,
      section: 'workspaces',
      label: ws.name,
      detail: current ? tr('canvas.palette.current', 'Current') : (ws.workingDirectory ?? ''),
      icon: 'workspace',
      aliases: `${tr('canvas.palette.workspace', 'Workspace')} ${ws.workingDirectory ?? ''}`,
      keys: i < 9 ? `⌃${i + 1}` : undefined,
      workspaceId: ws.id,
      run: () => onSwitch(ws),
    });
    for (const f of ws.floors) {
      out.push({
        id: `floor:${ws.id}:${f.id}`,
        section: 'floors',
        label: f.name || f.id,
        detail: `${tr('canvas.palette.floor', 'Floor')} · ${ws.name}`,
        icon: 'floor',
        aliases: `${tr('canvas.palette.floor', 'Floor')} ${f.branch ?? ''}`,
        workspaceId: ws.id,
        floorId: f.id,
        boost: ws.id === engine.store.workspaceId ? 3 : 0,
        run: () => onSwitch(ws, f.id),
      });
    }
  });
  return out;
}

export function roleItems(engine: CanvasEngine, index: PaletteIndex, tr: Tr): PaletteItem[] {
  return index.roles.map((r) => ({
    id: `role:${r.id}`,
    section: 'roles' as const,
    label: r.name,
    detail: tr('canvas.palette.spawnRole', 'Start an agent with this role'),
    icon: 'role',
    body: r.prompt,
    aliases: tr('canvas.palette.role', 'Role'),
    run: () => {
      const node = engine.spawn('agent', null, { agent: r.agent ?? 'claude', title: r.name, data: { roleId: r.id } });
      engine.reveal(node.id);
    },
  }));
}

const AGENTS: Array<{ agent: AgentKind; label: string }> = [
  { agent: 'claude', label: 'Claude Code' },
  { agent: 'codex', label: 'Codex' },
  { agent: 'gemini', label: 'Gemini CLI' },
  { agent: 'opencode', label: 'OpenCode' },
  { agent: 'ollama', label: 'Ollama' },
];

interface ActionHooks {
  toggleSidebar?: () => void;
  newWorkspace?: () => void;
  newFloor?: () => void;
}

function openPanel(engine: CanvasEngine, panel: NonNullable<CanvasEngine['panel']>): void {
  engine.panel = panel;
}

async function openWorkspaceFolder(engine: CanvasEngine): Promise<void> {
  const dir = workspaces.byId(engine.store.workspaceId)?.workingDirectory;
  if (!dir) {
    engine.say('canvas.palette.noFolder');
    return;
  }
  await invoke('open_path_default', { path: dir }).catch(() => engine.say('canvas.palette.noFolder'));
}

/** Workspace > Open in IDE from the palette (M-A8): the workspace's editor. */
async function openWorkspaceInIde(engine: CanvasEngine): Promise<void> {
  if (!workspaces.byId(engine.store.workspaceId)?.workingDirectory) {
    engine.say('canvas.palette.noFolder');
    return;
  }
  await invoke('canvas_workspace_open_ide', { workspaceId: engine.store.workspaceId, ide: null }).catch(() =>
    engine.say('canvas.palette.noFolder'),
  );
}

/** Global actions (empty query shows them first). */
export function actionItems(engine: CanvasEngine, tr: Tr, hooks: ActionHooks): PaletteItem[] {
  const spawn = (kind: string, agent?: AgentKind) => () => {
    // A file tree needs its root up front (Rust validates `rootPath`).
    const dir = workspaces.byId(engine.store.workspaceId)?.workingDirectory;
    const data = kind === 'filetree' && dir ? { rootPath: dir } : undefined;
    const node = engine.spawn(kind, null, { agent, data });
    engine.reveal(node.id);
    if (kind === 'note' || kind === 'text') engine.selection.focus(node.id);
  };
  const a = (id: string, label: string, run: () => void, keys?: string, aliases?: string, icon = 'action'): PaletteItem => ({
    id: `action:${id}`,
    section: 'actions',
    label,
    icon,
    keys,
    aliases,
    run,
  });
  const out: PaletteItem[] = [
    a('new-terminal', tr('canvas.palette.action.newTerminal', 'New Terminal'), spawn('terminal'), 'T', 'shell', 'terminal'),
    ...AGENTS.map(({ agent, label }) =>
      a(`new-agent-${agent}`, tr('canvas.palette.action.newAgent', 'New {{agent}}', { agent: label }), spawn('agent', agent), undefined, `agent spawn ${agent}`, 'agent'),
    ),
    a('new-note', tr('canvas.palette.action.newNote', 'New Note'), spawn('note'), 'N', 'sticky markdown', 'note'),
    a('new-text', tr('canvas.palette.action.newText', 'New Text'), spawn('text'), 'L', undefined, 'text'),
    a('new-filetree', tr('canvas.palette.action.newFileTree', 'New File Tree'), spawn('filetree'), 'F', 'files folder', 'filetree'),
    a('new-portal', tr('canvas.palette.action.newPortal', 'New Portal'), spawn('portal'), 'P', 'browser web', 'portal'),
    a('new-stack', tr('canvas.palette.action.newStack', 'New Fichário'), () => newStackFromSelection(engine), undefined, 'stack folder notes', 'stack'),
    a('zoom-fit', tr('canvas.palette.action.zoomFit', 'Zoom to Fit'), () => engine.fitAll(), '⇧1'),
    a('zoom-selection', tr('canvas.palette.action.zoomSelection', 'Zoom to Selection'), () => engine.zoomToggle(), '⌥⌘\\'),
    a('zoom-in', tr('canvas.palette.action.zoomIn', 'Zoom In'), () => engine.setZoom(engine.camera.zoom * 1.25), '⌘+'),
    a('zoom-out', tr('canvas.palette.action.zoomOut', 'Zoom Out'), () => engine.setZoom(engine.camera.zoom / 1.25), '⌘-'),
    a('next-attention', tr('canvas.palette.action.nextAttention', 'Next Needs You'), () => engine.nextAttention(), '⌘⇧A', 'attention blocked waiting'),
    a('minimap', tr('canvas.palette.action.minimap', 'Show or Hide Minimap'), () => (engine.minimapVisible = !engine.minimapVisible), '⌘⇧M'),
    a('undo', tr('canvas.palette.action.undo', 'Undo'), () => engine.undo(), '⌘Z'),
    a('redo', tr('canvas.palette.action.redo', 'Redo'), () => engine.redo(), '⌘⇧Z'),
    a('tidy', tr('canvas.palette.action.tidy', 'Tidy Up'), () => tidy(engine), '⌘⇧T', 'arrange grid layout'),
    a('save', tr('canvas.palette.action.save', 'Save Now'), () => {
      engine.sync.flush();
      void invoke('canvas_flush').catch(() => {});
    }),
    {
      ...a('ask', tr('canvas.palette.action.ask', 'Ask…'), () => {}, undefined, 'prompt send message terminal'),
      mode: 'ask',
    },
    {
      ...a('check', tr('canvas.palette.action.check', 'Check…'), () => {}, undefined, 'peek output terminal read'),
      mode: 'check',
    },
    {
      ...a('notifications', tr('canvas.palette.action.notifications', 'Notifications'), () => {}, '⌥⌘A', 'attention waiting'),
      mode: 'attention',
    },
    {
      ...a('switch-workspace', tr('canvas.palette.action.switchWorkspace', 'Switch Workspace…'), () => {}, '⌃1–9', 'workspace go change', 'workspace'),
      mode: 'workspaces',
    },
    a('routines', tr('canvas.palette.action.routines', 'Routines…'), () => openPanel(engine, { kind: 'routine' }), undefined, 'routine schedule cron timer', 'action'),
    a('ombro', tr('canvas.palette.action.ombro', 'Ombro'), () => openPanel(engine, { kind: 'companion' }), undefined, 'companion omni assistant shoulder', 'action'),
    a('partituras', tr('canvas.palette.action.partituras', 'Partituras…'), () => openPanel(engine, { kind: 'partituras' }), '⌘⇧L', 'partitura template library score', 'action'),
    a('roles', tr('canvas.palette.action.roles', 'Roles…'), () => openPanel(engine, { kind: 'roles' }), undefined, 'role library persona', 'role'),
    a('open-folder', tr('canvas.palette.action.openFolder', 'Open Workspace Folder'), () => void openWorkspaceFolder(engine), undefined, 'finder explorer project', 'filetree'),
    a('open-ide', tr('canvas.menu.openInIde', 'Open in IDE'), () => void openWorkspaceInIde(engine), undefined, 'ide editor vscode cursor zed xcode code', 'filetree'),
  ];
  if (engine.selection.size > 0) {
    out.push(
      { ...a('partitura-new', tr('canvas.palette.action.partituraNew', 'New Partitura from Selection'), () => {}, undefined, 'partitura save template', 'action'), mode: 'partitura-new' },
      { ...a('partitura-update', tr('canvas.palette.action.partituraUpdate', 'Update Partitura from Selection…'), () => {}, undefined, 'partitura save template replace', 'action'), mode: 'partitura-update' },
    );
  }
  if (hooks.newWorkspace) out.push(a('new-workspace', tr('canvas.palette.action.newWorkspace', 'New Workspace'), hooks.newWorkspace, '⌘⇧N', undefined, 'workspace'));
  if (hooks.newFloor) out.push(a('new-floor', tr('canvas.palette.action.newFloor', 'New Floor'), hooks.newFloor, undefined, undefined, 'floor'));
  if (hooks.toggleSidebar) out.push(a('sidebar', tr('canvas.palette.action.sidebar', 'Show or Hide Sidebar'), hooks.toggleSidebar, '⌘S'));
  return out;
}

/** Actions on the selected node (Maestri contextual actions). */
export function contextItems(engine: CanvasEngine, tr: Tr): PaletteItem[] {
  const ids = engine.selection.list().filter((id) => engine.store.node(id));
  if (!ids.length) return [];
  const node = engine.store.node(ids[ids.length - 1])!;
  const c = (id: string, label: string, extra: Partial<PaletteItem>): PaletteItem => ({
    id: `context:${id}`,
    section: 'context',
    label,
    detail: node.title ?? node.kind,
    icon: 'action',
    nodeId: node.id,
    ...extra,
  });
  const out: PaletteItem[] = [];
  if (node.kind === 'note') {
    out.push(
      c('copy', tr('canvas.palette.ctx.copyContents', 'Copy Contents'), {
        run: async () => {
          const text = await noteRead(engine.store.workspaceId, engine.store.floorId, node.id)
            .then((n) => n.text)
            .catch(() => String(node.data.text ?? ''));
          await navigator.clipboard?.writeText(text).catch(() => {});
        },
      }),
      c('lock', node.data.isContentLocked ? tr('canvas.palette.ctx.unlockContents', 'Unlock Contents') : tr('canvas.palette.ctx.lockContents', 'Lock Contents'), {
        run: () => engine.updateNode(node.id, { data: { ...node.data, isContentLocked: node.data.isContentLocked ? null : true } }),
      }),
      c('rename', tr('canvas.palette.ctx.rename', 'Rename…'), { mode: 'rename-note' }),
    );
    // "Place in Fichário": one entry per fichário on the floor, plus a new one.
    const inStack = stackIdOf(node);
    const place = (stackId: string | null) =>
      void import('../notes/stack').then(({ placeInStack }) => placeInStack(engine, node.id, stackId));
    for (const st of engine.store.nodes.filter((n) => n.kind === 'stack' && n.id !== inStack)) {
      const name = String(st.title || (st.data.name as string) || 'Fichário');
      out.push(
        c(`stack-${st.id}`, tr('canvas.palette.ctx.placeInStack', 'Place in Fichário: {{name}}', { name }), {
          aliases: 'stack fichario file into folder',
          run: () => place(st.id),
        }),
      );
    }
    out.push(
      c('stack-new', tr('canvas.palette.ctx.placeInNewStack', 'Place in New Fichário'), {
        aliases: 'stack fichario file into folder new',
        run: () => place(null),
      }),
    );
  }
  if (node.kind === 'filetree') out.push(c('files', tr('canvas.palette.ctx.searchFiles', 'Search Files'), { mode: 'files' }));
  if (isTerminalKind(node.kind)) {
    // Maestri puts Edit, Reload, Unload and Delete first for a terminal.
    out.push(
      c('edit', tr('canvas.palette.ctx.edit', 'Edit…'), { mode: 'edit-terminal' }),
      c('reload', tr('canvas.palette.ctx.reload', 'Reload'), { run: () => engine.requestRestart(node.id) }),
      c('unload', tr('canvas.palette.ctx.unload', 'Unload'), {
        run: async () => {
          await invoke('term_close', { id: terminalIdOf(node), deleteHistory: false }).catch(() => {});
        },
      }),
      c('delete-terminal', tr('canvas.palette.ctx.delete', 'Delete'), { run: () => engine.deleteSelection() }),
    );
    out.push(
      c('ask', tr('canvas.palette.ctx.ask', 'Ask {{name}}…', { name: node.title ?? '' }), { mode: 'ask' }),
      c('check', tr('canvas.palette.ctx.check', 'Check {{name}}', { name: node.title ?? '' }), { mode: 'check' }),
    );
  }
  out.push(
    c('connect', tr('canvas.palette.ctx.connectTo', 'Connect to…'), { mode: 'connect' }),
    c('disconnect', tr('canvas.palette.ctx.disconnect', 'Disconnect…'), { mode: 'disconnect' }),
    c('remove-connections', tr('canvas.palette.ctx.removeConnections', 'Remove All Connections'), {
      run: () => {
        const ops = engine.store.connections
          .filter((k) => ids.includes(k.from) || ids.includes(k.to))
          .map((k) => ({ op: 'remove_connection' as const, id: k.id }));
        engine.commit(ops);
      },
    }),
    c('lock-position', node.locked ? tr('canvas.palette.ctx.unlock', 'Unlock Position') : tr('canvas.palette.ctx.lock', 'Lock Position'), {
      run: () => engine.toggleLock(),
    }),
  );
  if (!isTerminalKind(node.kind)) out.push(c('delete', tr('canvas.palette.ctx.delete', 'Delete'), { run: () => engine.deleteSelection() }));
  return out;
}

function newStackFromSelection(engine: CanvasEngine): void {
  // Lazy import keeps the palette free of the stack module until used.
  void import('../notes/stack').then(({ createStackOps }) => {
    const notes = engine.selection
      .list()
      .map((id) => engine.store.node(id))
      .filter((n): n is CanvasNode => !!n && n.kind === 'note' && !stackIdOf(n));
    const c = engine.camera.center();
    const first = notes[0];
    const rect = first ? { x: first.x, y: first.y, w: first.w, h: first.h + 36 } : { x: Math.round((c.x - 140) / 20) * 20, y: Math.round((c.y - 120) / 20) * 20, w: 280, h: 240 };
    const { ops, stackId } = createStackOps(engine.store, notes, rect, notes.length ? undefined : 'Fichário');
    engine.commit(ops, { selectionAfter: [stackId] });
  });
}

function tidy(engine: CanvasEngine): void {
  const ids = engine.selection.list().filter((id) => engine.store.node(id));
  const target = ids.length > 1 ? ids : engine.store.nodes.filter((n) => !stackIdOf(n)).map((n) => n.id);
  void invoke('canvas_layout_arrange', {
    workspaceId: engine.store.workspaceId,
    floorId: engine.store.floorId,
    ids: target,
    mode: 'tidy',
    origin: 'user',
  }).catch(() => engine.say('canvas.palette.tidyFailed'));
}

/** Ranks items for a query; empty query keeps the given order. */
export function rank(items: PaletteItem[], query: string, limit = 80): Ranked[] {
  const words = splitQuery(query);
  if (!words.length) return items.slice(0, limit).map((item) => ({ item, score: 0, nameIdx: [] }));
  const out: Ranked[] = [];
  for (const item of items) {
    const fields: Field[] = [
      { text: item.label, weight: 3 },
      { text: item.aliases ?? '', weight: 1.5, loose: false },
      { text: item.detail ?? '', weight: 1.2, loose: false },
      { text: item.body ?? '', weight: 1, loose: false },
    ];
    const r = fuzzy(words, fields);
    if (!r) continue;
    out.push({ item, score: r.score + (item.boost ?? 0), nameIdx: r.nameIdx });
  }
  out.sort((a, b) => b.score - a.score);
  // One row per element: a terminal waiting for you is also a plain node.
  const seen = new Set<string>();
  const unique = out.filter((r) => {
    const s = r.item.section;
    if (!r.item.nodeId || (s !== 'nodes' && s !== 'notes' && s !== 'attention')) return true;
    const key = `${r.item.workspaceId}:${r.item.nodeId}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
  return unique.slice(0, limit);
}

