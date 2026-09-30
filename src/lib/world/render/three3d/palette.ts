// Palette lookup: an instance names a palette key, a module part names a
// slot, the colour is `palettes[key].slots[slot]`. The charter carries the
// city's palettes; the built-in one (`palette-data.ts`) covers anything it leaves out, so a
// missing key or slot draws in a sane colour instead of black.

import { Color } from 'three';
import { PALETTE_SLOTS, type Palette } from '$lib/city3d/types';
import { DEFAULT_PALETTE } from './palette-data';

export { DEFAULT_PALETTE };

export class PaletteBook {
  private readonly map = new Map<string, Palette>();
  private readonly cache = new Map<string, Color>();
  private fallbackKey = 'default';

  constructor(palettes: Palette[] = [], defaultKey?: string) {
    this.set(palettes, defaultKey);
  }

  set(palettes: Palette[], defaultKey?: string): void {
    this.map.clear();
    this.cache.clear();
    this.map.set('default', DEFAULT_PALETTE);
    for (const p of palettes) if (p && p.key) this.map.set(p.key, p);
    this.fallbackKey = defaultKey && this.map.has(defaultKey) ? defaultKey : 'default';
  }

  /** Colour of (palette, slot), linear, shared: never mutate the result. */
  color(palette: string | null | undefined, slot: string): Color {
    const id = `${palette ?? ''}|${slot}`;
    let c = this.cache.get(id);
    if (c) return c;
    const hex =
      this.map.get(palette ?? '')?.slots[slot] ??
      this.map.get(this.fallbackKey)?.slots[slot] ??
      DEFAULT_PALETTE.slots[slot] ??
      '#cccccc';
    c = new Color();
    try {
      c.set(hex);
    } catch {
      c.set('#cccccc');
    }
    this.cache.set(id, c);
    return c;
  }

  /** Colour of a terrain vertex slot index (into PALETTE_SLOTS). */
  slotColor(index: number): Color {
    return this.color(this.fallbackKey, PALETTE_SLOTS[index] ?? 'ground');
  }
}
