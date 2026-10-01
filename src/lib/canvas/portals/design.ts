// Portal design-select (Maestri Design mode): typed wrappers over
// commands::canvas::portal::canvas_portal_design_* and the
// `canvas://design-select` event (Rust `crate::canvas::design`).

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

const DESIGN_EVENT = 'canvas://design-select';

interface DesignRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

interface DesignPick {
  selector: string;
  tag: string;
  html: string;
  text: string;
  box: DesignRect | null;
  elementBox?: DesignRect | null;
  viewport?: { w: number; h: number; dpr: number } | null;
  url: string;
  title: string;
}

interface DesignTarget {
  nodeId: string;
  name: string;
  agent: string;
}

export interface DesignEvent {
  workspaceId: string;
  nodeId: string;
  targets: DesignTarget[];
  cancelled?: boolean;
  error?: string | null;
  pick?: DesignPick;
  screenshotPath?: string;
  screenshotError?: string;
}

export const designArm = (nodeId: string, hint?: string) =>
  invoke<{ armed: boolean; targets: DesignTarget[] }>('canvas_portal_design_arm', { nodeId, hint: hint ?? null });

export const designDisarm = (nodeId: string) => invoke('canvas_portal_design_disarm', { nodeId });

export const designSend = (a: {
  nodeId: string;
  target: string;
  comment: string;
  pick: DesignPick;
  screenshotPath?: string | null;
  attachments?: string[];
}) =>
  invoke<{ deliveredTo: DesignTarget }>('canvas_portal_design_send', {
    nodeId: a.nodeId,
    target: a.target,
    comment: a.comment,
    pick: a.pick,
    screenshotPath: a.screenshotPath ?? null,
    attachments: a.attachments ?? [],
  });

export function onDesign(cb: (e: DesignEvent) => void): Promise<UnlistenFn> {
  return listen<DesignEvent>(DESIGN_EVENT, (e) => cb(e.payload));
}
