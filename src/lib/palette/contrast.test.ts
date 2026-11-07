import { describe, expect, it } from "vitest";
import { parseColor } from "./color";
import { apcaContrast, apcaContrastRgb, contrastReport, levelOf, ratioText, suggestFix, wcagRatio } from "./contrast";

const c = (s: string) => parseColor(s)!;

describe("WCAG 2.x", () => {
  it("black on white is 21", () => {
    expect(wcagRatio(c("#000"), c("#fff"))).toBeCloseTo(21, 6);
    expect(wcagRatio(c("#fff"), c("#000"))).toBeCloseTo(21, 6);
  });
  it("same color is 1", () => {
    expect(wcagRatio(c("#777"), c("#777"))).toBeCloseTo(1, 6);
  });
  it("matches known pairs", () => {
    expect(wcagRatio(c("#767676"), c("#fff"))).toBeCloseTo(4.54, 2);
    expect(wcagRatio(c("#777777"), c("#fff"))).toBeCloseTo(4.48, 2);
    expect(wcagRatio(c("#0000ff"), c("#fff"))).toBeCloseTo(8.59, 2);
  });
  it("classifies levels", () => {
    expect(levelOf(7.1)).toBe("aaa");
    expect(levelOf(4.5)).toBe("aa");
    expect(levelOf(3.2)).toBe("aa_large");
    expect(levelOf(2)).toBe("fail");
    const r = contrastReport(c("#777777"), c("#fff"));
    expect(r.normalAA).toBe(false);
    expect(r.largeAA).toBe(true);
  });
  it("never prints a ratio above the threshold it fails", () => {
    expect(ratioText(4.46, 1)).toBe("4.4");
    expect(ratioText(6.96, 1)).toBe("6.9");
    expect(ratioText(4.5, 1)).toBe("4.5");
    expect(ratioText(7, 2)).toBe("7.00");
    expect(ratioText(4.4999, 2)).toBe("4.49");
  });
  it("blends translucent text over the background", () => {
    expect(wcagRatio(c("rgb(0 0 0 / 0%)"), c("#fff"))).toBeCloseTo(1, 6);
  });
});

describe("APCA", () => {
  const pairs: [string, string, number][] = [
    ["#000000", "#ffffff", 106.04],
    ["#ffffff", "#000000", -107.88],
    ["#888888", "#ffffff", 63.06],
    ["#ffffff", "#888888", -68.54],
    ["#000000", "#aaaaaa", 58.15],
    ["#aaaaaa", "#000000", -56.24],
  ];
  it.each(pairs)("%s on %s", (text, bg, lc) => {
    expect(apcaContrast(c(text), c(bg))).toBeCloseTo(lc, 1);
  });
  it("returns 0 for near identical colors", () => {
    expect(apcaContrastRgb([120, 120, 120], [120, 120, 121])).toBe(0);
  });
});

describe("fix", () => {
  it("finds a nearby lightness that passes AA", () => {
    const text = c("#888888");
    const bg = c("#ffffff");
    const fix = suggestFix(text, bg, "text", 4.5)!;
    expect(fix.ratio).toBeGreaterThanOrEqual(4.5);
    expect(fix.color.l).toBeLessThan(text.l);
    expect(fix.deltaL).toBeLessThan(0.1);
    expect(wcagRatio({ ...fix.color, l: fix.color.l + 0.003 }, bg)).toBeLessThan(4.5);
  });
  it("keeps a passing color", () => {
    expect(suggestFix(c("#000"), c("#fff"))!.deltaL).toBe(0);
  });
  it("can move the background", () => {
    const fix = suggestFix(c("#ffffff"), c("#999999"), "background")!;
    expect(fix.ratio).toBeGreaterThanOrEqual(4.5);
    expect(fix.color.l).toBeLessThan(c("#999999").l);
  });
});
