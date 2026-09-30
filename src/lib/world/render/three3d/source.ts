// Where the 3D city's data comes from. The default source is the app's own
// Tauri commands (agent G2's `city3d_*`, agent E's kit); the bench and the
// calibration use a synthetic one with the same shape. Calls go straight to
// `invoke` so the renderer does not depend on the order agents land in.

import type { Blueprint } from '$lib/city3d/types';
import { fallbackKit, loadKit, type Kit } from './kit';
import type { RegionMeshData } from './region';
import { parseStatus, type CityInfo } from './status';

export { parseStatus, type CityInfo };

export interface RegionSource {
  info(): Promise<CityInfo | null>;
  mesh(region: [number, number]): Promise<RegionMeshData>;
  blueprint(region: [number, number]): Promise<Blueprint | null>;
  kit(style: string): Promise<Kit>;
  /** `city3d:changed` → the region to re-mesh. */
  onChanged(cb: (region: [number, number], revision: string) => void): Promise<() => void>;
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const core = await import('@tauri-apps/api/core');
  return (await core.invoke(cmd, args)) as T;
}

export function tauriSource(): RegionSource {
  return {
    async info() {
      try {
        return parseStatus(await invoke('city3d_status'));
      } catch {
        return null;
      }
    },
    async mesh(region) {
      return await invoke<RegionMeshData>('city3d_region_mesh', { region });
    },
    async blueprint(region) {
      try {
        const r = await invoke<{ blueprint?: Blueprint | null }>('city3d_region', { region });
        return r?.blueprint ?? null;
      } catch {
        return null;
      }
    },
    async kit(style) {
      return await loadKit(style);
    },
    async onChanged(cb) {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        return await listen<{ region: [number, number]; revision: string }>('city3d:changed', (e) => {
          const r = e.payload?.region;
          if (Array.isArray(r)) cb([Number(r[0]), Number(r[1])], String(e.payload.revision ?? ''));
        });
      } catch {
        return () => {};
      }
    },
  };
}

/** A fixed set of regions in memory (bench, calibration, previews). */
export function memorySource(data: Map<string, { mesh: RegionMeshData; blueprint?: Blueprint | null }>, info?: Partial<CityInfo>): RegionSource {
  const regions = [...data.values()].map((d) => d.mesh.instances.region as [number, number]);
  return {
    async info() {
      return {
        mode: 'memory',
        exists: true,
        style: 'bench',
        palettes: [],
        defaultPalette: 'default',
        regions,
        seed: 'bench',
        raw: null,
        ...info,
      };
    },
    async mesh(region) {
      const d = data.get(`${region[0]},${region[1]}`);
      if (!d) throw new Error('no region');
      return d.mesh;
    },
    async blueprint(region) {
      return data.get(`${region[0]},${region[1]}`)?.blueprint ?? null;
    },
    async kit(style) {
      return fallbackKit(style);
    },
    async onChanged() {
      return () => {};
    },
  };
}
