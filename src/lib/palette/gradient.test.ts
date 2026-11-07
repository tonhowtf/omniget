import { describe, expect, it } from "vitest";
import { parseColor, toHex } from "./color";
import { cubicBezier, expandStops, gradientCss, mix, nativeGradientCss, sampleAt, type GradientDef } from "./gradient";

const def = (over: Partial<GradientDef> = {}): GradientDef => ({
  stops: [
    { color: parseColor("#0000ff")!, pos: 0 },
    { color: parseColor("#ffff00")!, pos: 1 },
  ],
  space: "oklch",
  easing: "linear",
  bezier: [0.42, 0, 0.58, 1],
  angle: 90,
  samples: 8,
  ...over,
});

describe("easing", () => {
  it("linear is identity and ends are fixed", () => {
    const f = cubicBezier([0, 0, 1, 1]);
    for (const x of [0, 0.25, 0.5, 0.9, 1]) expect(f(x)).toBeCloseTo(x, 5);
    const g = cubicBezier([0.42, 0, 0.58, 1]);
    expect(g(0)).toBe(0);
    expect(g(1)).toBe(1);
    expect(g(0.5)).toBeCloseTo(0.5, 4);
    expect(g(0.25)).toBeLessThan(0.25);
  });
  it("is monotonic for ease-in-out", () => {
    const g = cubicBezier([0.42, 0, 0.58, 1]);
    let prev = -1;
    for (let i = 0; i <= 100; i++) {
      const v = g(i / 100);
      expect(v).toBeGreaterThanOrEqual(prev);
      prev = v;
    }
  });
});

describe("mixing", () => {
  it("sRGB midpoint of blue and yellow is gray", () => {
    const m = mix(parseColor("#0000ff")!, parseColor("#ffff00")!, 0.5, "srgb");
    expect(m.c).toBeLessThan(0.01);
  });
  it("OKLCH midpoint keeps chroma", () => {
    const m = mix(parseColor("#0000ff")!, parseColor("#ffff00")!, 0.5, "oklch");
    expect(m.c).toBeGreaterThan(0.2);
  });
  it("takes the shorter hue path", () => {
    const m = mix({ l: 0.6, c: 0.1, h: 350 }, { l: 0.6, c: 0.1, h: 30 }, 0.5, "oklch");
    expect(m.h).toBeCloseTo(10, 6);
  });
  it("uses the other hue when one end is gray", () => {
    const m = mix({ l: 1, c: 0, h: 0 }, { l: 0.5, c: 0.2, h: 250 }, 0.5, "oklch");
    expect(m.h).toBeCloseTo(250, 6);
  });
});

describe("stops", () => {
  it("adds eased stops between each pair", () => {
    const out = expandStops(def({ easing: "ease-in-out", samples: 6 }));
    expect(out).toHaveLength(7);
    expect(out[0].pos).toBe(0);
    expect(out[6].pos).toBe(1);
  });
  it("keeps plain sRGB linear gradients minimal", () => {
    expect(expandStops(def({ space: "srgb" }))).toHaveLength(2);
  });
  it("samples ends exactly", () => {
    expect(toHex(sampleAt(def(), 0))).toBe("#0000ff");
    expect(toHex(sampleAt(def(), 1))).toBe("#ffff00");
  });
  it("writes CSS", () => {
    const css = gradientCss(def({ samples: 2 }));
    expect(css.startsWith("linear-gradient(90deg, #0000ff 0%")).toBe(true);
    expect(css.endsWith("#ffff00 100%)")).toBe(true);
    expect(nativeGradientCss(def())).toContain("in oklch shorter hue 90deg");
  });
});
