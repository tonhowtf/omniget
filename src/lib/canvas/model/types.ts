type NodeKind =
  | 'terminal'
  | 'agent'
  | 'note'
  | 'text'
  | 'file'
  | 'filetree'
  | 'link'
  | 'portal'
  | 'shape'
  | 'image'
  | 'stack'
  | 'connector'
  | 'unknown'
  | (string & {});

export type AgentKind = 'claude' | 'codex' | 'gemini' | 'opencode' | 'ollama' | 'shell' | 'native';

export interface CanvasNode {
  id: string;
  kind: NodeKind;
  x: number;
  y: number;
  w: number;
  h: number;
  z: number;
  title?: string;
  groupId?: string;
  locked?: boolean;
  stackId?: string;
  subtitle?: string;
  icon?: string;
  color?: string;
  data: Record<string, unknown>;
}

type ConnectionKind = 'ask' | 'message' | 'handoff' | 'spawn' | 'note' | 'portal' | (string & {});

export interface Connection {
  id: string;
  from: string;
  to: string;
  kind: ConnectionKind;
  perms: string[];
  condition?: 'always' | 'success' | 'failure';
  verify?: string;
  label?: string;
}

type DrawingKind =
  | 'pen'
  | 'highlighter'
  | 'rect'
  | 'ellipse'
  | 'diamond'
  | 'triangle'
  | 'arrow'
  | 'line'
  | 'text'
  | (string & {});

export type FillStyle = 'none' | 'solid' | 'hachure' | 'cross' | 'tint' | 'solidOpaque';

type StrokePattern = 'solid' | 'dashed' | 'dotted';

type DrawingEdge = 'top' | 'bottom' | 'left' | 'right' | (string & {});

interface DrawingAnchor {
  drawingId: string;
  edge: DrawingEdge;
}

export interface Drawing {
  id: string;
  kind: DrawingKind;
  x: number;
  y: number;
  w: number;
  h: number;
  z?: number;
  points?: number[];
  pressures?: number[];
  color: string;
  strokeWidth: number;
  opacity?: number;
  fill?: string;
  fillStyle?: FillStyle;
  sketch?: boolean;
  seed?: number;
  text?: string;
  groupId?: string;
  rotation?: number;
  strokePattern?: StrokePattern;
  cornerRadius?: number;
  textFontSize?: number;
  locked?: boolean;
  sourceConnection?: DrawingAnchor;
  targetConnection?: DrawingAnchor;
  controlPointOffset?: number;
}

export interface Group {
  id: string;
  name?: string;
  color?: string;
  target?: 'nodes' | 'drawings';
}

export interface CameraState {
  x: number;
  y: number;
  zoom: number;
}

export interface Snapshot {
  workspaceId: string;
  floorId: string;
  epoch: string | number;
  rev: number;
  seq?: number;
  camera?: CameraState;
  nodes: CanvasNode[];
  connections: Connection[];
  drawings: Drawing[];
  groups: Group[];
}

export interface Workspace {
  id: string;
  name: string;
  icon?: string;
  color?: string;
  workingDirectory?: string;
  activeFloorId?: string;
  floors?: Array<{ id: string; name?: string }>;
}

export type Origin = 'user' | `agent:${string}` | (string & {});

export type Op =
  | { op: 'add_node'; node: CanvasNode }
  | { op: 'update_node'; id: string; patch: Partial<CanvasNode> }
  | { op: 'remove_node'; id: string }
  | { op: 'add_connection'; connection: Connection }
  | { op: 'update_connection'; id: string; patch: Partial<Connection> }
  | { op: 'remove_connection'; id: string }
  | { op: 'add_drawing'; drawing: Drawing }
  | { op: 'update_drawing'; id: string; patch: Partial<Drawing> }
  | { op: 'remove_drawing'; id: string }
  | { op: 'set_group'; id: string; group: Group | null }
  | { op: 'set_camera'; camera: CameraState };

export interface ApplyResult {
  epoch?: string | number;
  rev: number;
  seq: number;
  ops?: unknown[];
  inverse?: unknown[];
  rebased?: boolean;
}

export interface PatchEvent {
  workspaceId: string;
  floorId: string;
  epoch: string | number;
  rev?: number;
  seq: number;
  ops: Op[];
  origin: Origin;
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export const GRID = 20;
export const MIN_ZOOM = 0.1;
export const MAX_ZOOM = 3;

export function isTerminalKind(kind: string): boolean {
  return kind === 'terminal' || kind === 'agent';
}

export function terminalIdOf(node: { id: string; data?: unknown }): string {
  const data = node.data as { terminalId?: unknown } | null | undefined;
  return typeof data?.terminalId === 'string' && data.terminalId ? data.terminalId : node.id;
}

export function newId(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) return crypto.randomUUID();
  const hex = () => Math.floor(Math.random() * 0x10000).toString(16).padStart(4, '0');
  return `${hex()}${hex()}-${hex()}-4${hex().slice(1)}-${hex()}-${hex()}${hex()}${hex()}`;
}
