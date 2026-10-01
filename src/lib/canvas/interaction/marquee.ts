// Marquee selection (02-motor-do-canvas §4.3): dragging left-to-right selects
// what is fully inside, right-to-left what intersects (CAD convention); ⌥
// inverts. Group frames never block the marquee.

import type { CanvasStore } from '../model/store.svelte';
import { drawingBounds } from '../model/store.svelte';
import type { Rect } from '../model/types';
import { containsRect, intersects, rectFromPoints, type Pt } from '../spatial/geometry';

export interface MarqueeState {
  start: Pt;
  end: Pt;
  additive: boolean;
  invert: boolean;
}

export function marqueeRect(m: MarqueeState): Rect {
  return rectFromPoints(m.start, m.end);
}

function marqueeContains(m: MarqueeState): boolean {
  const ltr = m.end.x >= m.start.x;
  return m.invert ? !ltr : ltr;
}

/** Node and drawing ids the marquee picks. */
export function marqueeHits(store: CanvasStore, m: MarqueeState): string[] {
  const r = marqueeRect(m);
  const contain = marqueeContains(m);
  const out: string[] = [];
  for (const id of store.nodeIndex.queryRect(r)) {
    const n = store.node(id);
    if (!n) continue;
    if (contain ? containsRect(r, n) : intersects(r, n)) out.push(id);
  }
  for (const id of store.drawingIndex.queryRect(r)) {
    const d = store.drawing(id);
    if (!d) continue;
    const b = drawingBounds(d);
    if (contain ? containsRect(r, b) : intersects(r, b)) out.push(id);
  }
  return out;
}
