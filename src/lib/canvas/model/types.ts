// Mirror of the serde types of core::canvas (02-motor-do-canvas §9, briefing
// "Documento do canvas"). Enums are open on read: an unknown kind renders as a
// plain box and is written back untouched. Optionals are omitted, never null.

export type NodeKind =
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
  /** World units (CSS px at zoom 1), top-left. */
  x: number;
  y: number;
  w: number;
  h: number;
  /** Stacking order; higher draws on top. */
  z: number;
  title?: string;
  groupId?: string;
  locked?: boolean;
  /** Page of a fichário (stack node): hidden on the canvas, still addressable. */
  stackId?: string;
  subtitle?: string;
  icon?: string;
  color?: string;
  data: Record<string, unknown>;
}

export interface TerminalData {
  terminalId: string;
  agent: AgentKind;
  roleId?: string;
  cwd?: string;
  command?: string[] | string;
  botId?: string;
  [k: string]: unknown;
}

export interface PortalData {
  url?: string;
  portalKind: 'web' | 'ios' | 'android';
  [k: string]: unknown;
}

export type ConnectionKind = 'ask' | 'message' | 'handoff' | 'spawn' | 'note' | 'portal' | (string & {});

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

export type DrawingKind =
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

/** `cross` is the front's name of the wire's `crossHatch`; `solid` is a light tint kept for old drawings. */
export type FillStyle = 'none' | 'solid' | 'hachure' | 'cross' | 'tint' | 'solidOpaque';

export type StrokePattern = 'solid' | 'dashed' | 'dotted';

export type DrawingEdge = 'top' | 'bottom' | 'left' | 'right' | (string & {});

/** Arrow end attached to the edge of another drawing (Maestri `sourceConnection`). */
export interface DrawingAnchor {
  drawingId: string;
  edge: DrawingEdge;
}

export interface Drawing {
  id: string;
  kind: DrawingKind;
  /** Origin (top-left of the bounding box) in world units. */
  x: number;
  y: number;
  w: number;
  h: number;
  z?: number;
  /**
   * Flat [x0, y0, x1, y1, ...] relative to the origin, rounded to 0.1.
   * Pen/highlighter: the stroke samples. Arrow/line: the two end points.
   */
  points?: number[];
  /** Per-sample pressure 0..1 for pen strokes, when a pen gave it. */
  pressures?: number[];
  /** CSS colour of the stroke. */
  color: string;
  strokeWidth: number;
  opacity?: number;
  fill?: string;
  fillStyle?: FillStyle;
  /** Hand-drawn look (rough.js-like). */
  sketch?: boolean;
  /** PRNG seed of the sketch jitter; kept so the shape never "boils". */
  seed?: number;
  text?: string;
  groupId?: string;
  /** Degrees, clockwise, around the centre of the box. */
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
  /** Node groups and drawing groups are separate (Maestri). */
  target?: 'nodes' | 'drawings';
}

export interface CameraState {
  /** World coordinate at the viewport's top-left corner. */
  x: number;
  y: number;
  zoom: number;
}

export interface Snapshot {
  workspaceId: string;
  floorId: string;
  /** Opaque; a new epoch means "drop local state and reload". */
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

/** Who made a change: the user or an agent (`agent:<terminalId>`). */
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
  /** Effective ops after Rust normalisation (cascades, filled ids), wire format. */
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

/** Message between agents travelling along a cord (`canvas://message`). */
export interface MessageEvent {
  id: string;
  from: string;
  to: string;
  kind: string;
  state?: string;
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

/** Plain UUID (the Rust side echoes ids literally, as the Maestri does). */
export function newId(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) return crypto.randomUUID();
  const hex = () => Math.floor(Math.random() * 0x10000).toString(16).padStart(4, '0');
  return `${hex()}${hex()}-${hex()}-4${hex().slice(1)}-${hex()}-${hex()}${hex()}${hex()}`;
}
