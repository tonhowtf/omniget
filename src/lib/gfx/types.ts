export type Tier = 0 | 1 | 2 | 3;

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

