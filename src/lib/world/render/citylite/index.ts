// The city's lite mode: the 3D city drawn by the 2D renderer with the kit
// baked into sprites (estudo 99, arquivo 09 §5.5; M0 kept the 2D for this).
// Imported dynamically by `CityView.svelte`; no three.js inside.

export { createLiteCityScene, tauriLiteSource, type LiteCityScene, type LiteCitySceneOpts, type LiteSource, type LiteFrameReport } from './scene';
export {
  mergeAtlases,
  layersFor,
  regionSprites,
  regionChunkTiles,
  terrainTiles,
  toDisplay,
  fromDisplay,
  LitePalettes,
  Z_PER_M,
  type KitAtlasPayload,
  type MergedAtlas,
  type LiteSprite,
} from './model';

import type { CityScene } from '../three3d/city';
import type { LiteCityScene } from './scene';

/** Either city backend, as `CityView.svelte` hands it out. */
export type AnyCityScene = CityScene | LiteCityScene;
/** Why a backend was chosen: the setting, a calibration, a remembered calibration or a failed 3D start. */
export type BackendReason = 'setting' | 'calibration' | 'remembered' | 'fallback';
