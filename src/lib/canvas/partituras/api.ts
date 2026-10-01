// Partitura library: typed wrappers over the `canvas_partitura_*` commands.

import { invoke } from '@tauri-apps/api/core';
import type { CanvasNode } from '../model/types';

interface EmbeddedRole {
  id: string;
  name: string;
  prompt: string;
  icon?: string;
  color?: string;
}

export interface Budget {
  maxUsd?: number;
  maxTokens?: number;
  maxMinutes?: number;
}

export interface Permissions {
  mode?: 'supervised' | 'acceptEdits' | 'auto' | 'full';
  allowRaw?: boolean;
  allowEvaluate?: boolean;
}

interface SetupSpec {
  setup?: string;
  run?: string;
  teardown?: string;
}

export interface LibraryPartitura {
  format: 'omniget.partitura';
  version: number;
  id: string;
  name: string;
  summary?: string;
  icon?: string;
  color?: string;
  createdAt: string;
  sourceWorkspaceId?: string;
  bounds: { w: number; h: number };
  nodes: CanvasNode[];
  connections: unknown[];
  drawings: unknown[];
  groups: unknown[];
  roles: EmbeddedRole[];
  noteTexts: Record<string, string>;
  scopeWorkspaceId?: string;
  updatedAt?: string;
  budget?: Budget;
  permissions?: Permissions;
  setup?: SetupSpec;
  privacy?: { noStartupCommands?: boolean; blankNotes?: boolean };
}

export interface Shape {
  id: string;
  kind: string;
  x: number;
  y: number;
  w: number;
  h: number;
  title?: string;
  color?: string;
}

export interface PartituraSummary {
  id: string;
  name: string;
  summary?: string;
  icon?: string;
  color?: string;
  scopeWorkspaceId?: string;
  workspaceName?: string;
  roleNames: string[];
  nodeCount: number;
  terminalCount: number;
  createdAt: string;
  updatedAt?: string;
  bounds: { w: number; h: number };
  shapes: Shape[];
  cords: [string, string][];
  hasSetup: boolean;
  hasBudget: boolean;
  /** Options it was saved with; Update Existing keeps them. */
  privacy?: { noStartupCommands?: boolean; blankNotes?: boolean };
  /** Earlier versions kept (the last update can be undone while > 0). */
  versions?: number;
}

export type RoleAction = 'keepMine' | 'useTheirs' | 'rename';

interface RolePlan {
  role: EmbeddedRole;
  status: 'new' | 'same' | 'conflict';
  matchedBy?: 'id' | 'name';
  local?: EmbeddedRole;
  defaultAction: RoleAction;
}

interface CommandLine {
  nodeId: string;
  title?: string;
  agent: string;
  command?: string;
  cwd?: string;
  roleName?: string;
}

export interface ImportPreview {
  partitura: PartituraSummary;
  commands: CommandLine[];
  roles: RolePlan[];
  portals: string[];
  noteCount: number;
  setup?: SetupSpec;
  budget?: Budget;
  permissions?: Permissions;
  replacesExisting: boolean;
  warnings: string[];
  /** Terminals whose chat history travels with it (format 2). */
  chatTranscripts?: number;
}

interface ExportOptions {
  name: string;
  summary?: string;
  icon?: string;
  color?: string;
  includeStartupCommands?: boolean;
  blankNotes?: boolean;
}

interface SaveRequest {
  selection: { nodeIds: string[]; drawingIds?: string[] };
  options: ExportOptions;
  scope?: string;
  updateExisting?: string;
  commands?: Record<string, string | null>;
  budget?: Budget;
  permissions?: Permissions;
  setup?: SetupSpec;
  /** Format 2: carry the terminals' chat history. */
  includeChat?: boolean;
}

interface StampOptions {
  commands?: Record<string, string | null>;
  roles?: Record<string, RoleAction>;
  applyHooks?: boolean;
  origin?: string;
  acceptChat?: boolean;
}

export interface ImportResult {
  apply: { rev: number; seq: number };
  nodeIds: string[];
  drawingIds: string[];
  roles: EmbeddedRole[];
}

interface LibraryImport {
  added: string[];
  skipped: string[];
}

export const listPartituras = (scope?: string, query?: string) =>
  invoke<PartituraSummary[]>('canvas_partitura_list', { scope: scope ?? null, query: query || null });

export const savePartitura = (workspaceId: string, floorId: string, request: SaveRequest) =>
  invoke<LibraryPartitura>('canvas_partitura_save', { workspaceId, floorId, request });

export const editPartitura = (id: string, patch: Record<string, unknown>) =>
  invoke<LibraryPartitura>('canvas_partitura_edit', { id, patch });

export const deletePartitura = (id: string) => invoke<void>('canvas_partitura_delete', { id });

export const previewImport = (workspaceId: string, source: { id?: string; partitura?: LibraryPartitura }) =>
  invoke<ImportPreview>('canvas_partitura_preview', {
    workspaceId,
    id: source.id ?? null,
    partitura: source.partitura ?? null,
  });

export const importPartitura = (
  workspaceId: string,
  floorId: string,
  source: { id?: string; partitura?: LibraryPartitura },
  at: { x: number; y: number },
  options: StampOptions,
) =>
  invoke<ImportResult>('canvas_partitura_import', {
    workspaceId,
    floorId,
    id: source.id ?? null,
    partitura: source.partitura ?? null,
    at,
    options,
  });

export const readPartituraFile = (path: string) => invoke<LibraryPartitura[]>('canvas_partitura_read_file', { path });

export const addToLibrary = (items: LibraryPartitura[]) =>
  invoke<LibraryImport>('canvas_partitura_add_to_library', { items });

export const exportPartituraFile = (ids: string[], path: string) =>
  invoke<void>('canvas_partitura_export_file', { ids, path });

/** Drag payload of a library card (drop on the canvas to stamp there). */
export const PARTITURA_DRAG_TYPE = 'application/x-omniget-partitura';
