// City status parsing, without three.js: shared by the 3D source and the
// lite (2D) city, so the lite chunk never pulls the 3D backend in.

import type { Charter, Palette } from '$lib/city3d/types';

export interface CityInfo {
  mode: string;
  /** False when there is no city yet (the page shows founding). */
  exists: boolean;
  style: string;
  palettes: Palette[];
  defaultPalette: string;
  /** Regions the charter's districts name; empty = unknown, probe around. */
  regions: [number, number][];
  seed: string;
  /** Whatever the status carried, for the page (server id, name…). */
  raw: unknown;
}

function asCharter(city: unknown): Partial<Charter> | null {
  if (!city || typeof city !== 'object') return null;
  const c = city as Record<string, unknown>;
  if (c.charter && typeof c.charter === 'object') return c.charter as Partial<Charter>;
  if ('palettes' in c || 'districts' in c || 'style' in c) return c as Partial<Charter>;
  return null;
}

export function parseStatus(status: unknown): CityInfo {
  const s = (status ?? {}) as { mode?: string; city?: unknown };
  const charter = asCharter(s.city);
  const regions: [number, number][] = [];
  const seen = new Set<string>();
  const extra = (s.city as { regions?: unknown } | null)?.regions;
  const lists: unknown[] = [...(charter?.districts ?? []).map((d) => d.regions), Array.isArray(extra) ? extra : []];
  for (const list of lists) {
    if (!Array.isArray(list)) continue;
    for (const r of list) {
      if (!Array.isArray(r) || r.length < 2) continue;
      const k = `${r[0]},${r[1]}`;
      if (seen.has(k)) continue;
      seen.add(k);
      regions.push([Number(r[0]), Number(r[1])]);
    }
  }
  return {
    mode: s.mode ?? 'local',
    exists: !!s.city,
    style: charter?.style ?? 'default',
    palettes: charter?.palettes ?? [],
    defaultPalette: charter?.palette ?? 'default',
    regions,
    seed: charter?.seed ?? charter?.city_id ?? 'city',
    raw: status,
  };
}
