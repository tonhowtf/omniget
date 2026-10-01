import type { CanvasEngine } from '../engine.svelte';
import { isTerminalKind } from '../model/types';
import { envDrop } from '../floors/api';
import { termPaste } from './client';
import { terminalIdOf } from './gl-types';
import { TERM_HEADER_H } from './metrics';

function quotePath(p: string): string {
  return /^[\w@%+=:,./-]+$/.test(p) ? p : `'${p.replace(/'/g, `'\\''`)}'`;
}

export function dropOnTerminal(engine: CanvasEngine, paths: string[], at: { x: number; y: number }): boolean {
  const hit = engine.store.nodes
    .filter(
      (n) =>
        isTerminalKind(n.kind) &&
        at.x >= n.x &&
        at.x <= n.x + n.w &&
        at.y >= n.y + TERM_HEADER_H &&
        at.y <= n.y + n.h,
    )
    .sort((a, b) => (b.z ?? 0) - (a.z ?? 0))[0];
  if (!hit || !paths.length) return false;
  void envDrop(hit.id, paths)
    .catch(() => paths)
    .then((typed) => termPaste(terminalIdOf(hit), typed.map(quotePath).join(' ') + ' '))
    .catch(() => {});
  return true;
}
