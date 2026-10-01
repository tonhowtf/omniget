import type { Component } from 'svelte';

interface NodeKindDef {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  component: Component<any>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  card?: Component<any>;
  minSize: { w: number; h: number };
  defaultSize: { w: number; h: number };
  icon: string;
  focusOnClick?: boolean;
  focusOnSecondClick?: boolean;
  labelKey?: string;
}

const kinds = new Map<string, NodeKindDef>();
const listeners = new Set<() => void>();
let version = 0;

export function registerNodeKind(kind: string, def: NodeKindDef, opts: { ifAbsent?: boolean } = {}): void {
  if (opts.ifAbsent && kinds.has(kind)) return;
  kinds.set(kind, def);
  version++;
  for (const l of listeners) l();
}

export function nodeKind(kind: string): NodeKindDef | undefined {
  return kinds.get(kind) ?? kinds.get('unknown');
}

export function onRegistryChange(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export const DEFAULT_SIZES: Record<string, { w: number; h: number }> = {
  terminal: { w: 640, h: 420 },
  agent: { w: 640, h: 420 },
  note: { w: 320, h: 260 },
  text: { w: 240, h: 60 },
  file: { w: 260, h: 80 },
  filetree: { w: 300, h: 420 },
  link: { w: 300, h: 90 },
  portal: { w: 800, h: 560 },
  image: { w: 320, h: 240 },
  stack: { w: 320, h: 260 },
  connector: { w: 160, h: 48 },
  generation: { w: 380, h: 600 },
  unknown: { w: 200, h: 120 },
};

export const MIN_SIZES: Record<string, { w: number; h: number }> = {
  terminal: { w: 200, h: 140 },
  agent: { w: 200, h: 140 },
  note: { w: 120, h: 80 },
  text: { w: 40, h: 24 },
  portal: { w: 240, h: 180 },
  generation: { w: 300, h: 420 },
};

export function sizeFor(kind: string): { w: number; h: number } {
  return kinds.get(kind)?.defaultSize ?? DEFAULT_SIZES[kind] ?? DEFAULT_SIZES.unknown;
}

export function minSizeFor(kind: string): { w: number; h: number } {
  return kinds.get(kind)?.minSize ?? MIN_SIZES[kind] ?? { w: 40, h: 24 };
}

export const ICONS = {
  terminal: 'M4 5h16v14H4z M7 9l3 3-3 3 M12 15h5',
  agent: 'M12 3a4 4 0 1 1 0 8 4 4 0 0 1 0-8z M4 21c1-4 4.5-6 8-6s7 2 8 6',
  note: 'M6 3h9l4 4v14H6z M14 3v5h5 M9 12h7 M9 16h7',
  text: 'M5 6h14 M12 6v13 M9 19h6',
  file: 'M6 3h8l5 5v13H6z M14 3v5h5',
  link: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1 M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1',
  image: 'M4 5h16v14H4z M4 16l5-5 4 4 3-3 4 4 M15 9h.01',
  portal: 'M3 5h18v14H3z M3 9h18 M6 7h.01 M8.5 7h.01',
  generation: 'M12 3l1.8 4.6L18.5 9.4l-4.7 1.8L12 16l-1.8-4.8L5.5 9.4l4.7-1.8z M18 15l.8 2.2L21 18l-2.2.8L18 21l-.8-2.2L15 18l2.2-.8z',
  unknown: 'M4 4h16v16H4z M9.5 9a2.5 2.5 0 1 1 3.5 2.3c-.6.3-1 .8-1 1.5V14 M12 17h.01',
} as const;
