// Pass contract of the canvas engine, re-exported from gl-layer plus the
// small helpers the terminal modules share.

export type { CameraInfo, ViewportInfo, NodeView, FrameInfo, GlPass } from '$lib/canvas/render/gl-layer';
import type { CameraInfo } from '$lib/canvas/render/gl-layer';

/** World point → CSS pixel inside the viewport. */
export function worldToScreen(cam: CameraInfo, x: number, y: number): { x: number; y: number } {
  return { x: (x - cam.x) * cam.zoom, y: (y - cam.y) * cam.zoom };
}

/** Terminal id carried by a terminal/agent node (falls back to the node id). */
export function terminalIdOf(node: { id: string; data?: unknown }): string {
  const data = node.data as { terminalId?: unknown } | null | undefined;
  return typeof data?.terminalId === 'string' && data.terminalId ? data.terminalId : node.id;
}

export function isTerminalKind(kind: string): boolean {
  return kind === 'terminal' || kind === 'agent';
}
