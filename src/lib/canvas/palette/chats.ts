import type { CanvasEngine } from '../engine.svelte';
import { THREADS, chatSearch, type ChatSearchHit } from '../chat/client';
import { chats } from '../chat/store.svelte';
import { goTo } from './navigate';
import type { PaletteIndex, PaletteItem, Tr } from './items';
import { workspaces } from './workspaces.svelte';

export interface ChatHitItem extends PaletteItem {
  open: () => Promise<void>;
}

function whereOf(engine: CanvasEngine, index: PaletteIndex, hit: ChatSearchHit): { floorId: string; title: string } | null {
  if (hit.workspaceId === engine.store.workspaceId) {
    const here = engine.store.node(hit.nodeId);
    if (here) return { floorId: engine.store.floorId, title: here.title ?? '' };
  }
  const away = index.remote.find((r) => r.node.id === hit.nodeId && r.workspaceId === hit.workspaceId);
  if (away) return { floorId: away.floorId, title: away.node.title ?? '' };
  if (hit.floorId) return { floorId: hit.floorId, title: '' };
  const ws = workspaces.byId(hit.workspaceId);
  const floorId = ws?.activeFloorId ?? ws?.floors[0]?.id;
  return floorId ? { floorId, title: '' } : null;
}

export async function searchChats(engine: CanvasEngine, index: PaletteIndex, query: string, tr: Tr): Promise<ChatHitItem[]> {
  let hits: ChatSearchHit[];
  try {
    hits = await chatSearch(query, 30);
  } catch {
    return [];
  }
  const out: ChatHitItem[] = [];
  for (const hit of hits) {
    const where = whereOf(engine, index, hit);
    if (!where) continue;
    const thread = THREADS[hit.thread]?.key ?? String(hit.thread);
    const wsName = workspaces.byId(hit.workspaceId)?.name ?? '';
    const name = where.title || tr('canvas.palette.kind.terminal', 'terminal');
    out.push({
      id: `chat:${hit.workspaceId}:${hit.nodeId}:${hit.entryId}`,
      section: 'chats',
      label: hit.snippet.replace(/\s+/g, ' ').trim(),
      detail: tr('canvas.palette.chatHit', '{{name}} · {{thread}} thread · {{ws}}', { name, thread, ws: wsName }),
      icon: 'agent',
      workspaceId: hit.workspaceId,
      floorId: where.floorId,
      nodeId: hit.nodeId,
      open: async () => {
        await goTo(engine, { workspaceId: hit.workspaceId, floorId: where.floorId, nodeId: hit.nodeId }, { focus: false });
        await chats.openAt(hit.nodeId, hit.thread, hit.entryId, query, hit.workspaceId);
      },
    });
  }
  return out;
}
