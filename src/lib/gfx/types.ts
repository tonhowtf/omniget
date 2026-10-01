// Shared types of the generic GL toolkit (world, canvas, pet). Constants only,
// so any module can import it without pulling GL in. The world keeps its own
// isometric types in `$lib/world/render/types`; the ones below are structurally
// identical to their world counterparts.

/** Measured machine tier (docs/llm-world-execution-plan.md §1.2/§1.3). */
export type Tier = 0 | 1 | 2 | 3;

/** Error codes the UI can map, in the mould of StreamError::code(). */
export const ERR = {
  NO_WEBGL: 'ERR_WORLD_NO_WEBGL',
  NO_CANVAS2D: 'ERR_WORLD_NO_CANVAS2D',
  CONTEXT_LOST: 'ERR_WORLD_CONTEXT_LOST',
  ATLAS_INVALID: 'ERR_WORLD_ATLAS_INVALID',
  FRAME_UNKNOWN: 'ERR_WORLD_FRAME_UNKNOWN',
  NOT_INITIALISED: 'ERR_WORLD_NOT_INITIALISED',
  SHADER: 'ERR_WORLD_SHADER',
} as const;

export class RenderError extends Error {
  readonly code: string;
  constructor(code: string, message?: string) {
    super(message ?? code);
    this.code = code;
    this.name = 'RenderError';
  }
}

export interface TextHandle {
  id: number;
  /** Width/height in CSS pixels of the rasterised line. */
  w: number;
  h: number;
}

export interface TextStyle {
  font?: string;
  size?: number;
  color?: string;
  outline?: string;
  maxWidth?: number;
}

