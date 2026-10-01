// Files dropped on a terminal body are typed into it as paths (Maestri: a
// bracketed paste, so an agent CLI attaches them). Dropped anywhere else on
// the canvas they become nodes (files/drop.ts).

import type { CanvasEngine } from '../engine.svelte';
import { isTerminalKind } from '../model/types';
import { envDrop } from '../floors/api';
import { termPaste } from './client';
import { terminalIdOf } from './gl-types';
import { TERM_HEADER_H } from './metrics';

/** Shell-quotes a path only when it needs it. */
function quotePath(p: string): string {
  return /^[\w@%+=:,./-]+$/.test(p) ? p : `'${p.replace(/'/g, `'\\''`)}'`;
}

/**
 * When the world point lands on the body of a terminal node, pastes the
 * paths there (space separated, quoted, with a trailing space) and returns
 * true. The topmost terminal wins.
 */
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
  // A terminal in SSH/Docker/runtime gets copies in /tmp/omniget-drops there
  // (Maestri: /tmp/maestri-drops); a local one gets the paths as they are.
  void envDrop(hit.id, paths)
    .catch(() => paths)
    .then((typed) => termPaste(terminalIdOf(hit), typed.map(quotePath).join(' ') + ' '))
    .catch(() => {});
  return true;
}
