import type { CanvasNode, Connection } from '../model/types';

type MentionKind = 'agent' | 'note' | 'portal' | 'file' | 'action' | 'floor' | 'workspace' | 'other';

export interface MentionTarget {
  id: string;
  name: string;
  kind: MentionKind;
  detail?: string;
  color?: string;
  tag?: string;
  special?: 'file';
}

interface MentionContext {
  workspaceId?: string;
  floorId?: string;
  workspaces?: Array<{ id: string; name: string; floors?: Array<{ id: string; name: string }> }>;
}

export interface Paste {
  n: number;
  text: string;
  lines: number;
}

export interface Draft {
  text: string;
  pastes: Paste[];
}

const PILL_MIN_CHARS = 800;
const PILL_MIN_LINES = 3;

const DRAFT_PREFIX = 'omniget.canvas.composer.draft:';

function draftKey(workspaceId: string, nodeId: string): string {
  return `${DRAFT_PREFIX}${workspaceId}:${nodeId}`;
}

export function loadDraft(workspaceId: string, nodeId: string): Draft {
  try {
    const raw = localStorage.getItem(draftKey(workspaceId, nodeId));
    if (raw) {
      const d = JSON.parse(raw) as Draft;
      if (typeof d.text === 'string' && Array.isArray(d.pastes)) return d;
    }
  } catch {
  }
  return { text: '', pastes: [] };
}

export function saveDraft(workspaceId: string, nodeId: string, d: Draft): void {
  try {
    if (!d.text && !d.pastes.length) localStorage.removeItem(draftKey(workspaceId, nodeId));
    else localStorage.setItem(draftKey(workspaceId, nodeId), JSON.stringify(d));
  } catch {
  }
}

function kindOfNode(n: CanvasNode): MentionKind {
  switch (n.kind) {
    case 'terminal':
    case 'agent':
      return 'agent';
    case 'note':
    case 'stack':
      return 'note';
    case 'portal':
      return 'portal';
    case 'file':
    case 'image':
      return 'file';
    case 'connector':
      return 'agent';
    default:
      return 'other';
  }
}

function nameOf(n: CanvasNode): string {
  const d = n.data ?? {};
  return (
    n.title ||
    (typeof d.name === 'string' && d.name) ||
    (typeof d.url === 'string' && d.url) ||
    (typeof d.path === 'string' && d.path.split(/[\\/]/).pop()) ||
    n.kind
  );
}

function detailOf(n: CanvasNode): string | undefined {
  const d = n.data ?? {};
  if (n.kind === 'terminal' || n.kind === 'agent') return typeof d.agent === 'string' ? d.agent : undefined;
  if (n.kind === 'portal') return typeof d.url === 'string' ? d.url : undefined;
  if (n.kind === 'file' || n.kind === 'image') return typeof d.path === 'string' ? d.path : undefined;
  return undefined;
}

function connectedTargets(nodeId: string, nodes: CanvasNode[], connections: Connection[]): MentionTarget[] {
  const ids = new Set<string>();
  for (const c of connections) {
    if (c.from === nodeId) ids.add(c.to);
    else if (c.to === nodeId) ids.add(c.from);
  }
  ids.delete(nodeId);
  const out: MentionTarget[] = [];
  for (const n of nodes) {
    if (!ids.has(n.id)) continue;
    const kind = kindOfNode(n);
    if (kind === 'other') continue;
    const t: MentionTarget = { id: n.id, name: nameOf(n), kind, detail: detailOf(n) };
    if (kind === 'agent' && n.color) t.color = n.color;
    if (n.kind === 'connector') {
      const remote = typeof n.data?.remoteNodeId === 'string' ? n.data.remoteNodeId : n.id;
      t.id = `remote:${remote}`;
      t.detail = n.data?.route === 'crossWorkspace' ? 'other workspace' : 'other floor';
      if (typeof n.data?.remoteColor === 'string') t.color = n.data.remoteColor;
    }
    out.push(t);
  }
  out.sort((a, b) => a.kind.localeCompare(b.kind) || a.name.localeCompare(b.name));
  return out;
}

function hintTag(kind: string, name: string, cli: string): string {
  return `<omniget-canvas-${kind} name="${escAttr(name)}" skill="omniget-canvas" cli="${escAttr(cli)}" />`;
}

export function mentionTargets(
  nodeId: string,
  nodes: CanvasNode[],
  connections: Connection[],
  ctx: MentionContext = {},
): MentionTarget[] {
  const out = connectedTargets(nodeId, nodes, connections);
  out.push({ id: 'special:file', name: 'File…', kind: 'action', detail: '#', special: 'file' });
  out.push({
    id: 'special:lead',
    name: 'Lead',
    kind: 'action',
    detail: 'recruit, connect, notify',
    tag: hintTag('lead', 'Lead', 'omniget canvas recruit|dismiss|connect|disconnect|notify'),
  });
  out.push({
    id: 'special:new-note',
    name: 'New Note',
    kind: 'action',
    tag: hintTag('action', 'New Note', 'omniget canvas note create "text" --name "Name"'),
  });
  out.push({
    id: 'special:new-portal',
    name: 'New Portal',
    kind: 'action',
    tag: hintTag('action', 'New Portal', 'omniget canvas portal create URL [Name]'),
  });
  out.push({
    id: 'special:notifications',
    name: 'Notifications',
    kind: 'action',
    tag: hintTag('action', 'Notifications', 'omniget canvas notify "message"'),
  });
  out.push({
    id: 'special:routines',
    name: 'Routines',
    kind: 'action',
    tag: hintTag('action', 'Routines', 'omniget canvas routine list'),
  });
  const here = ctx.workspaces?.find((w) => w.id === ctx.workspaceId);
  for (const f of here?.floors ?? []) {
    if (f.id === ctx.floorId) continue;
    const name = f.name || f.id;
    out.push({ id: `floor:${f.id}`, name, kind: 'floor', tag: hintTag('floor', name, `omniget canvas floor run "${name}"`) });
  }
  for (const w of ctx.workspaces ?? []) {
    if (w.id === ctx.workspaceId) continue;
    const name = w.name || w.id;
    out.push({ id: `workspace:${w.id}`, name, kind: 'workspace', tag: hintTag('workspace', name, 'omniget canvas workspace list') });
  }
  return out;
}

export function filterTargets(targets: MentionTarget[], query: string): MentionTarget[] {
  const q = query.trim().toLowerCase();
  if (!q) return targets;
  const starts = targets.filter((t) => t.name.toLowerCase().startsWith(q));
  const has = targets.filter((t) => !t.name.toLowerCase().startsWith(q) && t.name.toLowerCase().includes(q));
  return [...starts, ...has];
}

export function mentionQuery(text: string, caret: number): { start: number; query: string } | null {
  const before = text.slice(0, caret);
  const m = /(^|\s)@([^\s@]{0,40})$/.exec(before);
  if (!m) return null;
  return { start: caret - m[2].length - 1, query: m[2] };
}

export function mentionToken(t: MentionTarget): string {
  return /\s/.test(t.name) ? `@"${t.name}"` : `@${t.name}`;
}

function shouldPill(text: string): boolean {
  return text.length > PILL_MIN_CHARS || text.split('\n').length >= PILL_MIN_LINES;
}

export function pillLabel(p: Paste): string {
  return `[Pasted text #${p.n} · ${p.lines} ${p.lines === 1 ? 'line' : 'lines'}]`;
}

function escAttr(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;');
}

function tagFor(t: MentionTarget): string {
  if (t.tag) return t.tag;
  const name = escAttr(t.name);
  switch (t.kind) {
    case 'agent':
      return `<omniget-canvas-agent name="${name}" skill="omniget-canvas" />`;
    case 'note':
      return `<omniget-canvas-note name="${name}" skill="omniget-canvas" />`;
    case 'portal':
      return `<omniget-canvas-portal name="${name}" skill="omniget-canvas" />`;
    case 'file':
      return t.detail ? t.detail : name;
    default:
      return name;
  }
}

export function expand(draft: Draft, targets: MentionTarget[]): string {
  let text = draft.text;
  for (const p of draft.pastes) text = text.split(pillLabel(p)).join(p.text);
  const byName = [...targets].sort((a, b) => b.name.length - a.name.length);
  for (const t of byName) {
    if (t.special) continue;
    const tokens = [`@"${t.name}"`, `@${t.name}`];
    for (const tok of tokens) {
      if (!text.includes(tok)) continue;
      text = text.split(tok).join(tagFor(t));
    }
  }
  return text;
}

export function prunePastes(d: Draft): Draft {
  return { text: d.text, pastes: d.pastes.filter((p) => d.text.includes(pillLabel(p))) };
}

function nextPasteNumber(d: Draft): number {
  return d.pastes.reduce((m, p) => Math.max(m, p.n), 0) + 1;
}

export function pathToken(path: string): string {
  return /\s/.test(path) ? `"${path}"` : path;
}

export function fileQuery(text: string, caret: number): { start: number; query: string } | null {
  const before = text.slice(0, caret);
  const m = /(^|\s)#([^\s#]{0,120})$/.exec(before);
  if (!m) return null;
  if (/^\d/.test(m[2])) return null;
  return { start: caret - m[2].length - 1, query: m[2] };
}

export function pastePill(d: Draft, text: string): { draft: Draft; label: string } | null {
  if (!text || !shouldPill(text)) return null;
  const p: Paste = { n: nextPasteNumber(d), text, lines: text.split('\n').length };
  return { draft: { ...d, pastes: [...d.pastes, p] }, label: pillLabel(p) };
}
