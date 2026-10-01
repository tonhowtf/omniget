// Turning paths into canvas nodes (study 95 §K, 03 §2): opening a file from a
// tree puts an editor node beside it; dropping things on the canvas makes a
// note (.md/.markdown/.txt, external), a file tree (folder) or a file node.

import type { CanvasEngine } from '../engine.svelte';
import { nearestFreeSpot } from '../interaction/snap';
import { stackIdOf, isNoteFile } from '../notes/api';
import type { CanvasNode } from '../model/types';
import { baseName, IMAGE_RE, joinPath, VIDEO_RE, type FileEditorData } from './api';

const EDITOR_SIZE = { w: 600, h: 480 };

function onFloor(engine: CanvasEngine): CanvasNode[] {
  return engine.store.nodes.filter((n) => !stackIdOf(n));
}

function tabsOf(n: CanvasNode): string[] {
  const t = n.data.tabs;
  const list = Array.isArray(t) ? t.filter((x): x is string => typeof x === 'string') : [];
  const cur = typeof n.data.path === 'string' ? n.data.path : '';
  return cur && !list.includes(cur) ? [...list, cur] : list;
}

/** Editor node already showing `abs` (active or in a tab), if any. */
export function findEditor(engine: CanvasEngine, abs: string): CanvasNode | undefined {
  const eds = engine.store.nodes.filter((n) => n.kind === 'file' && n.data.view === 'editor');
  return eds.find((n) => n.data.path === abs) ?? eds.find((n) => tabsOf(n).includes(abs));
}

/** Makes `abs` the active tab of editor `ed` (adding the tab when new). */
export function showInEditor(engine: CanvasEngine, ed: CanvasNode, abs: string, line?: number): void {
  const tabs = tabsOf(ed);
  const data: Record<string, unknown> = {
    ...ed.data,
    path: abs,
    displayName: baseName(abs),
    tabs: tabs.includes(abs) ? tabs : [...tabs, abs],
    isImage: IMAGE_RE.test(abs) || null,
    isVideo: VIDEO_RE.test(abs) || null,
  };
  if (line) data.line = line;
  else delete data.line;
  engine.updateNode(ed.id, { title: baseName(abs), data });
}

/**
 * Opens (or reveals) `rel` inside `root`. Opened from a file tree, the file
 * becomes a tab of that tree's editor (Maestri's editor tabs); the first file
 * puts an editor node beside `anchor`.
 */
export function openEditorBeside(engine: CanvasEngine, anchor: CanvasNode | null, root: string, rel: string, line?: number): CanvasNode {
  const abs = joinPath(root, rel);
  const existing = findEditor(engine, abs);
  if (existing) {
    if (existing.data.path !== abs) showInEditor(engine, existing, abs, line);
    else if (line) engine.updateNode(existing.id, { data: { ...existing.data, line } });
    engine.selection.set([existing.id]);
    engine.reveal(existing.id);
    return existing;
  }
  const treeEditor =
    anchor?.kind === 'filetree'
      ? engine.store.nodes.find((n) => n.kind === 'file' && n.data.view === 'editor' && n.data.treeId === anchor.id)
      : undefined;
  if (treeEditor) {
    showInEditor(engine, treeEditor, abs, line);
    engine.selection.set([treeEditor.id]);
    engine.reveal(treeEditor.id);
    return treeEditor;
  }
  const { w, h } = EDITOR_SIZE;
  const want = anchor ? { x: anchor.x + anchor.w + 40, y: anchor.y } : (() => {
    const c = engine.camera.center();
    return { x: c.x - w / 2, y: c.y - h / 2 };
  })();
  const spot = nearestFreeSpot(want.x, want.y, w, h, onFloor(engine));
  const data: FileEditorData & { treeId?: string } = { path: abs, displayName: baseName(rel), view: 'editor', rootPath: root, tabs: [abs] };
  if (anchor?.kind === 'filetree') data.treeId = anchor.id;
  // A remote tree's editor reads and writes over there too.
  if (anchor && anchor.data.environment != null) data.environment = anchor.data.environment;
  if (line) data.line = line;
  if (IMAGE_RE.test(rel)) data.isImage = true;
  if (VIDEO_RE.test(rel)) data.isVideo = true;
  const node = engine.spawn('file', null, {
    rect: { x: spot.x, y: spot.y, w, h },
    title: baseName(rel),
    data: data as unknown as Record<string, unknown>,
  });
  engine.reveal(node.id);
  return node;
}

/**
 * Node for a dropped path at world point `at`: folder → file tree, note file
 * → external note, anything else → file node (image/video preview).
 */
export function nodeForPath(engine: CanvasEngine, abs: string, isDir: boolean, at: { x: number; y: number }, root?: string): CanvasNode {
  const name = baseName(abs);
  if (isDir) {
    return engine.spawn('filetree', at, { title: name, data: { rootPath: abs } });
  }
  if (isNoteFile(abs)) {
    return engine.spawn('note', at, { title: name.replace(/\.(md|markdown|txt)$/i, ''), data: { path: abs, isExternal: true } });
  }
  const data: Record<string, unknown> = { path: abs, displayName: name };
  if (IMAGE_RE.test(abs)) data.isImage = true;
  if (VIDEO_RE.test(abs)) data.isVideo = true;
  if (root) data.rootPath = root;
  return engine.spawn('file', at, { title: name, data });
}
