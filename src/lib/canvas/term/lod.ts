import { TERM_FONT_SIZE } from './metrics';
import { terminalIdOf, isTerminalKind, type FrameInfo, type NodeView } from './gl-types';
import { termHub } from './hub';
import { termLive, type TermLod } from './state.svelte';
import type { Visible, WireLod } from './client';

const TEXT_ENTER_PX = 7.5;
const TEXT_LEAVE_PX = 6.5;
const CARD_ENTER_W = 140;
const CARD_LEAVE_W = 170;

const WIRE: Record<TermLod, WireLod> = { text: 0, mini: 1, card: 2 };

export function nextLod(prev: TermLod | null, fontPx: number, widthPx: number, focused: boolean): TermLod {
  if (focused) return 'text';
  const textIn = prev === 'text' ? fontPx >= TEXT_LEAVE_PX : fontPx >= TEXT_ENTER_PX;
  if (textIn) return 'text';
  const cardIn = prev === 'card' ? widthPx <= CARD_LEAVE_W : widthPx < CARD_ENTER_W;
  return cardIn ? 'card' : 'mini';
}

const lods = new Map<string, TermLod>();
let focusedId: string | null = null;

export function setLodFocus(id: string | null): void {
  focusedId = id;
}

export function focusedTerm(): string | null {
  return focusedId;
}

interface TermNodeLod {
  node: NodeView;
  termId: string;
  lod: TermLod;
}

export function updateLods(frame: FrameInfo): TermNodeLod[] {
  const zoom = frame.camera.zoom;
  const fontPx = TERM_FONT_SIZE * zoom;
  const out: TermNodeLod[] = [];
  const screenVisible: Visible[] = [];
  const seen = new Set<string>();
  for (const node of frame.visibleNodes) {
    if (!isTerminalKind(node.kind)) continue;
    const termId = terminalIdOf(node);
    if (seen.has(termId)) continue;
    seen.add(termId);
    const lod = nextLod(lods.get(termId) ?? null, fontPx, node.w * zoom, termId === focusedId);
    if (lods.get(termId) !== lod) {
      lods.set(termId, lod);
      const live = termLive(termId);
      if (live.lod !== lod) live.lod = lod;
    }
    out.push({ node, termId, lod });
  }
  for (const node of frame.screenNodes ?? []) {
    if (!isTerminalKind(node.kind)) continue;
    const termId = terminalIdOf(node);
    if (seen.has(termId)) continue;
    seen.add(termId);
    if (lods.get(termId) !== 'text') {
      lods.set(termId, 'text');
      const live = termLive(termId);
      if (live.lod !== 'text') live.lod = 'text';
    }
    screenVisible.push({ id: termId, lod: WIRE.text });
  }
  for (const id of lods.keys()) {
    if (!seen.has(id)) lods.delete(id);
  }
  const visible: Visible[] = out.map((t) => ({ id: t.termId, lod: WIRE[t.lod] })).concat(screenVisible);
  visible.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  termHub.setVisible(visible);
  return out;
}
