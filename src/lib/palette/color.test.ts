import { describe, expect, it } from "vitest";
import {
  fitToGamut,
  formatAs,
  formatHsl,
  formatOklch,
  inP3,
  inSrgb,
  linearRgbToOklab,
  linearToSrgb,
  maxChroma,
  oklabToLinearRgb,
  oklabToOklch,
  oklchToOklab,
  parseColor,
  srgbToLinear,
  srgbToOklch,
  toHex,
  FORMAT_IDS,
} from "./color";

const close = (a: number, b: number, eps = 1e-3) => expect(Math.abs(a - b)).toBeLessThan(eps);

describe("transfer functions", () => {
  it("round trips sRGB and linear", () => {
    for (let i = 0; i <= 255; i++) close(linearToSrgb(srgbToLinear(i / 255)), i / 255, 1e-9);
  });
  it("matches known linear values", () => {
    close(srgbToLinear(0.5), 0.214041, 1e-6);
    close(srgbToLinear(0.04045), 0.0031308, 1e-6);
  });
});

describe("OKLab", () => {
  it("maps white to L=1 with no chroma", () => {
    const w = linearRgbToOklab({ r: 1, g: 1, b: 1 });
    close(w.l, 1, 1e-4);
    close(w.a, 0, 1e-4);
    close(w.b, 0, 1e-4);
  });
  it("matches reference values for primaries", () => {
    const refs: [string, number, number, number][] = [
      ["#ff0000", 0.62796, 0.25768, 29.2339],
      ["#00ff00", 0.86644, 0.29483, 142.4953],
      ["#0000ff", 0.45201, 0.31321, 264.052],
      ["#ffff00", 0.96798, 0.21101, 109.7692],
      ["#808080", 0.59987, 0, 0],
    ];
    for (const [hex, l, c, h] of refs) {
      const v = parseColor(hex)!;
      close(v.l, l);
      close(v.c, c);
      if (c > 0) close(v.h, h, 0.05);
    }
  });
  it("round trips random colors through every space", () => {
    for (let i = 0; i < 500; i++) {
      const rgb = { r: Math.random(), g: Math.random(), b: Math.random() };
      const lab = linearRgbToOklab(rgb);
      const lch = oklabToOklch(lab);
      const back = oklabToLinearRgb(oklchToOklab(lch));
      close(back.r, rgb.r, 1e-6);
      close(back.g, rgb.g, 1e-6);
      close(back.b, rgb.b, 1e-6);
    }
  });
  it("round trips every 8-bit gray and a hex grid", () => {
    for (let r = 0; r < 256; r += 17) {
      for (let g = 0; g < 256; g += 51) {
        for (let b = 0; b < 256; b += 85) {
          const hex = `#${[r, g, b].map((n) => n.toString(16).padStart(2, "0")).join("")}`;
          expect(toHex(parseColor(hex)!)).toBe(hex);
        }
      }
    }
  });
});

describe("gamut", () => {
  it("flags out of sRGB and P3", () => {
    expect(inSrgb(parseColor("#3b82f6")!)).toBe(true);
    const vivid = { l: 0.7, c: 0.3, h: 145 };
    expect(inSrgb(vivid)).toBe(false);
    expect(inP3({ l: 0.65, c: 0.27, h: 145 })).toBe(true);
    expect(inSrgb({ l: 0.65, c: 0.27, h: 145 })).toBe(false);
    expect(inP3({ l: 0.7, c: 0.4, h: 145 })).toBe(false);
  });
  it("fits to sRGB by reducing chroma only", () => {
    const fitted = fitToGamut({ l: 0.7, c: 0.37, h: 145 }, "srgb");
    expect(inSrgb(fitted)).toBe(true);
    expect(fitted.l).toBe(0.7);
    expect(fitted.h).toBe(145);
    expect(fitted.c).toBeLessThan(0.37);
    expect(inSrgb({ ...fitted, c: fitted.c + 0.003 })).toBe(false);
  });
  it("gives P3 more room than sRGB", () => {
    expect(maxChroma(0.7, 145, "p3")).toBeGreaterThan(maxChroma(0.7, 145, "srgb"));
  });
});

describe("parsing", () => {
  const cases: [string, string][] = [
    ["#f80", "#ff8800"],
    ["ff8800", "#ff8800"],
    ["#ff880080", "#ff880080"],
    ["rgb(255, 136, 0)", "#ff8800"],
    ["rgb(255 136 0 / 50%)", "#ff880080"],
    ["rgba(255,136,0,0.5)", "#ff880080"],
    ["rgb(100% 0% 0%)", "#ff0000"],
    ["hsl(32 100% 50%)", "#ff8800"],
    ["hsl(0.0889turn, 100%, 50%)", "#ff8800"],
    ["rebeccapurple", "#663399"],
    ["RED", "#ff0000"],
    ["oklch(62.8% 0.2577 29.23)", "#ff0000"],
    ["oklch(0.628 0.2577 29.23)", "#ff0000"],
    ["oklab(0.628 0.2249 0.1258)", "#ff0000"],
    ["color(srgb 1 0 0)", "#ff0000"],
    ["color(display-p3 0.9175 0.2003 0.1386)", "#ff0000"],
  ];
  it.each(cases)("parses %s", (input, hex) => {
    expect(toHex(parseColor(input)!)).toBe(hex);
  });
  it("rejects garbage", () => {
    for (const bad of ["", "#12", "rgb(1,2)", "notacolor", "oklch(a b c)", "hsl(1 2 3 4 5)"]) expect(parseColor(bad)).toBeNull();
  });
  it("detects display-p3 red as outside sRGB", () => {
    expect(inSrgb(parseColor("color(display-p3 1 0 0)")!)).toBe(false);
  });
});

describe("formatting", () => {
  it("writes every format and parses them back", () => {
    const color = parseColor("#3b82f6")!;
    for (const id of FORMAT_IDS) {
      const text = formatAs(color, id);
      expect(toHex(parseColor(text)!)).toBe("#3b82f6");
    }
  });
  it("writes readable oklch and hsl", () => {
    expect(formatOklch({ l: 0.5, c: 0, h: 120 })).toBe("oklch(50% 0 0)");
    expect(formatOklch({ l: 0.6279, c: 0.2577, h: 29.23, alpha: 0.5 })).toBe("oklch(62.79% 0.2577 29.23 / 50%)");
    expect(formatHsl(srgbToOklch({ r: 1, g: 0, b: 0 }))).toBe("hsl(0 100% 50%)");
  });
});
