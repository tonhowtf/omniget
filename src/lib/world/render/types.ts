// Shared types of the OmniGet world renderer.
// Contract: docs/agents/f6-renderer.md §4. No runtime code here except constants,
// so that every other module can import it without pulling GL in.

/** Measured machine tier (docs/llm-world-execution-plan.md §1.2/§1.3). */
export type Tier = 0 | 1 | 2 | 3;

export type BackendName = 'gl2' | 'gl1' | 'canvas2d';

/** Isometric tile footprint in pixels at zoom 1 (2:1 diamond). */
export const TILE_W = 64;
export const TILE_H = 32;
/** Pixels of screen height for one world z unit (one tile of elevation). */
export const TILE_Z = 32;
/** Tiles per chunk side. */
export const CHUNK_TILES = 16;

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

/** Camera looks at (x, y) in tile units; zoom is screen px per projected px. */
export interface Camera {
  x: number;
  y: number;
  zoom: number;
  /** Viewport size in device pixels. */
  width: number;
  height: number;
}

export interface SpriteInst {
  /** Atlas frame id. */
  frame: string;
  /** World position in tile units; z is elevation in tile units. */
  x: number;
  y: number;
  z: number;
  /** Horizontal mirror — the three mirrored directions live here, never in the art. */
  flip?: boolean;
  /** 0xRRGGBB multiplied per vertex. Default 0xffffff. */
  tint?: number;
  /** 0..1, default 1. */
  alpha?: number;
}

export interface TextInst {
  handle: TextHandle;
  x: number;
  y: number;
  z: number;
  alpha?: number;
  tint?: number;
}

export interface Light {
  x: number;
  y: number;
  radius: number;
  color: number;
  intensity: number;
}

export type ChunkId = string;

export interface Scene {
  camera: Camera;
  chunksVisible: ChunkId[];
  sprites: SpriteInst[];
  texts: TextInst[];
  lights?: Light[];
}

export interface FrameStats {
  cpuMs: number;
  drawCalls: number;
  sprites: number;
  chunksBaked: number;
}

export interface Caps {
  backend: BackendName;
  tier: Tier;
  maxTextureSize: number;
  /** True when the backend can bake chunks into a render texture. */
  renderTexture: boolean;
  /** EXT_disjoint_timer_query_webgl2 present. */
  timerQuery: boolean;
  /** Device pixel ratio actually used. */
  dpr: number;
}

export interface AtlasHandle {
  id: number;
  /** Frame ids known to this atlas. */
  frames: ReadonlySet<string>;
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

/** One tile placed inside a chunk when baking. */
export interface ChunkTile {
  frame: string;
  /** Tile coordinates relative to the chunk origin, 0..CHUNK_TILES-1. */
  lx: number;
  ly: number;
  /** Elevation in tile units. */
  z?: number;
  flip?: boolean;
  tint?: number;
}

/**
 * Fired on the old canvas when a GL context refused to come back and the
 * renderer put a fresh `<canvas>` in its place (`detail` is the new one).
 * Whoever listens on the old element (input, probes) moves to the new one.
 */
export const CANVAS_REPLACED_EVENT = 'omniget:world-canvas-replaced';

/** A frame of the dynamic page, in its pixels (see `Renderer.setDynamicPage`). */
export interface DynamicFrame {
  x: number;
  y: number;
  w: number;
  h: number;
  pivotX: number;
  pivotY: number;
}

/** What a renderer holds right now, for the "rebuilt, not duplicated" proof. */
export interface RenderMemory {
  /** Live textures (GL) or cached canvases/bitmaps (Canvas 2D). */
  textures: number;
  /** Their size as RGBA8, in bytes. */
  bytes: number;
  /** Size of the dynamic page, 0 when there is none. */
  dynamicPage: number;
}

export interface InitOpts {
  backend?: BackendName;
  tier: Tier;
  dpr?: number;
}

export interface Renderer {
  init(canvas: HTMLCanvasElement, opts: InitOpts): Promise<Caps>;
  loadAtlas(json: unknown, pages: ImageBitmap[]): AtlasHandle;
  frame(scene: Scene, dt: number): FrameStats;
  bakeChunk(id: ChunkId, tiles: ChunkTile[]): void;
  /**
   * The page drawn at runtime (pictures users published) next to the shipped
   * atlas pages. Replaces the previous page and all its frames; `null` drops
   * both. The renderer keeps the bitmap, so sleep/wake and a lost context
   * rebuild it; GL reuses one texture for it, never a second copy.
   */
  setDynamicPage(page: ImageBitmap | null, frames: Record<string, DynamicFrame>): void;
  memory(): RenderMemory;
  invalidateChunk(id: ChunkId): void;
  text(str: string, style?: TextStyle): TextHandle;
  resize(w: number, h: number, dpr: number): void;
  /** Cancels every rAF, drops the GL context and frees textures. */
  sleep(): void;
  /** Rebuilds everything from the ImageBitmaps kept in memory. */
  wake(): Promise<void>;
  destroy(): void;
  /** Current tier, which the watchdog may lower at runtime. */
  readonly tier: Tier;
  setTier(tier: Tier): void;
}

export interface CalibrationResult {
  tier: Tier;
  medianMs: number;
  backend: BackendName | 'none';
  contextLostDuring: boolean;
}
