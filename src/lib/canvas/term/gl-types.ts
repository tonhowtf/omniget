export type { CameraInfo, ViewportInfo, NodeView, FrameInfo, GlPass } from '$lib/canvas/render/gl-layer';
import type { CameraInfo } from '$lib/canvas/render/gl-layer';

export function worldToScreen(cam: CameraInfo, x: number, y: number): { x: number; y: number } {
  return { x: (x - cam.x) * cam.zoom, y: (y - cam.y) * cam.zoom };
}

export function terminalIdOf(node: { id: string; data?: unknown }): string {
  const data = node.data as { terminalId?: unknown } | null | undefined;
  return typeof data?.terminalId === 'string' && data.terminalId ? data.terminalId : node.id;
}

export function isTerminalKind(kind: string): boolean {
  return kind === 'terminal' || kind === 'agent';
}
