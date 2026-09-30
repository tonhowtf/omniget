// Semantic zoom (02-motor-do-canvas §2.6): the level of each node is decided in
// projected pixels, with hysteresis so it never flickers at a threshold.
//   near = live DOM content (L1), mid = card (L2), far = sprite (L0).
// Also the small budget allocator used for mounted terminals and live portals.

import { isTerminalKind, type CanvasNode } from './model/types';

export type Lod = 'near' | 'mid' | 'far';

interface LodRule {
  /** Projected font size (px) at which "near" starts, and below which it ends. */
  baseFont: number;
  nearEnter: number;
  nearLeave: number;
  /** Projected width (px) below which "far" starts, and above which it ends. */
  farEnter: number;
  farLeave: number;
}

const TERMINAL_RULE: LodRule = { baseFont: 13, nearEnter: 7.5, nearLeave: 6.5, farEnter: 140, farLeave: 170 };
const NOTE_RULE: LodRule = { baseFont: 14, nearEnter: 7, nearLeave: 6.3, farEnter: 90, farLeave: 110 };
const SMALL_RULE: LodRule = { baseFont: 14, nearEnter: 4, nearLeave: 3.5, farEnter: 40, farLeave: 50 };

function ruleFor(kind: string): LodRule {
  if (isTerminalKind(kind) || kind === 'portal') return TERMINAL_RULE;
  if (kind === 'text' || kind === 'image' || kind === 'shape') return SMALL_RULE;
  return NOTE_RULE;
}

export function nodeLod(node: Pick<CanvasNode, 'kind' | 'w'>, zoom: number, prev?: Lod): Lod {
  const r = ruleFor(node.kind);
  const font = r.baseFont * zoom;
  const width = node.w * zoom;
  if (prev === 'near') {
    if (font >= r.nearLeave) return 'near';
  } else if (font >= r.nearEnter) {
    return 'near';
  }
  if (prev === 'far') {
    if (width <= r.farLeave) return 'far';
  } else if (width < r.farEnter) {
    return 'far';
  }
  return 'mid';
}

/**
 * True when switching between the two levels is expensive. Terminals stay
 * mounted in L1 at near and mid (their GL pass draws text or a minimap), so
 * only crossing the far boundary mounts or unmounts them; portals show a
 * native webview only at near.
 */
export function isCostlySwap(kind: string, from: Lod | undefined, to: Lod): boolean {
  if (from === to) return false;
  if (isTerminalKind(kind)) return (from === 'far' || from === undefined) !== (to === 'far');
  if (kind === 'portal') return to === 'near' || from === 'near';
  return false;
}

/** Terminals are DOM-mounted (L1) at both near and mid. */
export function mountsInWorld(kind: string, lod: Lod): boolean {
  return lod === 'near' || (lod === 'mid' && isTerminalKind(kind));
}

/** Keeps the `limit` best candidates (highest score first). */
export function allocate(cands: Array<{ id: string; score: number }>, limit: number): Set<string> {
  return new Set(
    cands
      .slice()
      .sort((a, b) => b.score - a.score)
      .slice(0, Math.max(0, limit))
      .map((c) => c.id),
  );
}

/** Mounted xterm ceiling (any renderer) and live portals by default (§8.2, §7.3). */
export const MOUNTED_TERMINALS_LIMIT = 12;
export const LIVE_PORTALS_LIMIT = 4;
