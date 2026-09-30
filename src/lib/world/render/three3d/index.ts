// Entry of the `three3d` backend (estudo 99, M0). Imported dynamically
// through `loadThree3d()` in `../index.ts`, so nothing of three.js reaches a
// route that does not draw the 3D city.

export { createThreeRenderer, type ThreeRenderer, type ThreeRendererOpts } from './renderer';
export { createCityScene, type CityScene, type CitySceneOpts, type CitySelection, type CityStats } from './city';
export { createCameraRig, type CameraRig } from './camera';
export { loadKit, parseKit, fallbackKit, fallbackParts, parseKey, type Kit, type KitPart } from './kit';
export { buildRegion, instanceMatrix, regionKey, REGION_M, type BuiltRegion, type RegionMeshData, type InstancePick } from './region';
export { PaletteBook, DEFAULT_PALETTE } from './palette';
export { createMaterials, buildBrushNoise, type MaterialSet } from './materials';
export { createPeople, type People, type ServerAgent } from './people';
export { tauriSource, memorySource, parseStatus, type CityInfo, type RegionSource } from './source';
export { synthRegion, SYNTH_PALETTES } from './synth';
export { runCityBench, type BenchReport, type BenchRow } from './bench';
