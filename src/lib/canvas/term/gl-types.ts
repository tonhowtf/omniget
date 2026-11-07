export type { CameraInfo, ViewportInfo, NodeView, FrameInfo, GlPass } from '$lib/canvas/render/gl-layer';

export function terminalIdOf(node: { id: string; data?: unknown }): string {
  const data = node.data as { terminalId?: unknown } | null | undefined;
  return typeof data?.terminalId === 'string' && data.terminalId ? data.terminalId : node.id;
}

export function isTerminalKind(kind: string): boolean {
  return kind === 'terminal' || kind === 'agent';
}
