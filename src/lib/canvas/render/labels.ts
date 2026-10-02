export interface LabelBox {
  x: number;
  y: number;
  w: number;
  h: number;
  priority: number;
}

export function placeLabels<T extends LabelBox>(items: T[], pad = 4): T[] {
  const kept: T[] = [];
  const order = items.slice().sort((a, b) => b.priority - a.priority);
  for (const it of order) {
    const clash = kept.some(
      (k) => it.x < k.x + k.w + pad && k.x < it.x + it.w + pad && it.y < k.y + k.h + pad && k.y < it.y + it.h + pad,
    );
    if (!clash) kept.push(it);
  }
  return kept;
}

export function labelPriority(kind: string, w: number, h: number, terminal: boolean, selected: boolean): number {
  return (selected ? 4e12 : 0) + (terminal ? 2e12 : 0) + (kind === 'note' || kind === 'text' ? 1e12 : 0) + w * h;
}
