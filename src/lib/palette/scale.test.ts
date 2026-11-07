import { describe, expect, it } from "vitest";
import { inP3, inSrgb, parseColor } from "./color";
import { STEPS, baseIndex, evenLightness, generateScale, harmony, harmonyHues } from "./scale";

describe("scale", () => {
  const bases = ["#3b82f6", "#ef4444", "#22c55e", "#eab308", "#71717a", "#0f172a", "#fefce8"].map((s) => parseColor(s)!);

  it("has 11 Tailwind steps", () => {
    const scale = generateScale(bases[0]);
    expect(scale.map((s) => s.step)).toEqual([...STEPS]);
    expect(scale.filter((s) => s.isBase)).toHaveLength(1);
  });

  it.each(bases.map((b, i) => [i, b] as const))("lightness decreases strictly (base %i)", (_, base) => {
    const scale = generateScale(base, { hueShift: 20 });
    for (let i = 1; i < scale.length; i++) expect(scale[i].color.l).toBeLessThan(scale[i - 1].color.l);
  });

  it("keeps the base color exactly at its step", () => {
    const base = parseColor("#3b82f6")!;
    const scale = generateScale(base);
    const b = scale.find((s) => s.isBase)!;
    expect(b.color.l).toBeCloseTo(base.l, 6);
    expect(b.color.c).toBeCloseTo(base.c, 6);
    expect(STEPS.indexOf(b.step)).toBe(baseIndex(base.l));
  });

  it("is perceptually even on each side of the base", () => {
    const scale = generateScale({ l: 0.615, c: 0.1, h: 250 });
    const diffs = scale.slice(1).map((s, i) => scale[i].color.l - s.color.l);
    const k = scale.findIndex((s) => s.isBase);
    const above = diffs.slice(0, k);
    const below = diffs.slice(k);
    for (const d of above) expect(d).toBeCloseTo(above[0], 6);
    for (const d of below) expect(d).toBeCloseTo(below[0], 6);
  });

  it("eases chroma toward the ends", () => {
    const scale = generateScale({ l: 0.62, c: 0.15, h: 250 });
    const k = scale.findIndex((s) => s.isBase);
    expect(scale[0].color.c).toBeLessThan(scale[k].color.c);
    expect(scale[10].color.c).toBeLessThan(scale[k].color.c);
  });

  it("stays in the chosen gamut", () => {
    const vivid = { l: 0.65, c: 0.3, h: 145 };
    for (const s of generateScale(vivid, { gamut: "srgb" })) expect(inSrgb(s.color)).toBe(true);
    for (const s of generateScale(vivid, { gamut: "p3" })) expect(inP3(s.color)).toBe(true);
  });

  it("drifts hue across the scale", () => {
    const scale = generateScale({ l: 0.62, c: 0.1, h: 200 }, { hueShift: 30 });
    expect(scale[10].color.h).toBeGreaterThan(scale[0].color.h);
  });

  it("spaces even lightness from top to bottom", () => {
    const even = evenLightness();
    expect(even[0]).toBeCloseTo(0.97, 6);
    expect(even[10]).toBeCloseTo(0.26, 6);
  });
});

describe("harmonies", () => {
  it("rotates hues", () => {
    expect(harmonyHues("complementary", 30)).toEqual([210]);
    expect(harmonyHues("analogous", 10)).toEqual([340, 40]);
    expect(harmonyHues("triadic", 300)).toEqual([60, 180]);
  });
  it("keeps lightness and stays in gamut", () => {
    for (const c of harmony("triadic", { l: 0.6, c: 0.25, h: 20 })) {
      expect(c.l).toBe(0.6);
      expect(inSrgb(c)).toBe(true);
    }
  });
});
