import { describe, expect, it } from "vitest";
import { RATIOS, buildScale, cssString, fluidClamp, lineHeightFor, rem, sizeAt, stepName, toCssVariables, toTailwindTheme } from "./scale";

const base = { base: 16, ratio: 1.25, stepsDown: 2, stepsUp: 5, fluid: false, maxBase: 18, maxRatio: 1.333, minViewport: 320, maxViewport: 1280 };

describe("scale math", () => {
  it("has the eight ratio presets in order", () => {
    expect(RATIOS.map((r) => r.value)).toEqual([1.067, 1.125, 1.2, 1.25, 1.333, 1.414, 1.5, 1.618]);
  });

  it("computes modular steps", () => {
    expect(sizeAt(16, 1.25, 0)).toBe(16);
    expect(sizeAt(16, 1.25, 2)).toBeCloseTo(25);
    expect(sizeAt(16, 1.25, -1)).toBeCloseTo(12.8);
  });

  it("names steps like Tailwind", () => {
    expect([-3, -2, -1, 0, 1, 2, 3, 6].map(stepName)).toEqual(["2xs", "xs", "sm", "base", "lg", "xl", "2xl", "5xl"]);
  });

  it("converts px to rem without float noise", () => {
    expect(rem(16)).toBe("1rem");
    expect(rem(12.8)).toBe("0.8rem");
    expect(rem(25.000000001)).toBe("1.5625rem");
    expect(rem(20, 10)).toBe("2rem");
  });

  it("builds a static scale", () => {
    const steps = buildScale(base);
    expect(steps.map((s) => s.name)).toEqual(["xs", "sm", "base", "lg", "xl", "2xl", "3xl", "4xl"]);
    expect(steps[2]).toMatchObject({ min: 16, max: 16, value: "1rem", lineHeight: 1.5 });
    expect(steps[4].value).toBe("1.5625rem");
  });

  it("suggests tighter line height for bigger text", () => {
    expect(lineHeightFor(16)).toBe(1.5);
    expect(lineHeightFor(32)).toBe(1.25);
    expect(lineHeightFor(80)).toBe(1.05);
  });
});

describe("fluid clamp", () => {
  it("interpolates between two viewports", () => {
    expect(fluidClamp(16, 20, 320, 1280)).toBe("clamp(1rem, 0.4167vw + 0.9167rem, 1.25rem)");
  });

  it("collapses equal sizes to a single value", () => {
    expect(fluidClamp(16, 16, 320, 1280)).toBe("1rem");
    expect(fluidClamp(16, 20, 1280, 320)).toBe("1rem");
  });

  it("keeps the smaller bound first when the size shrinks", () => {
    const v = fluidClamp(20, 16, 320, 1280);
    expect(v.startsWith("clamp(1rem,")).toBe(true);
    expect(v.endsWith("1.25rem)")).toBe(true);
    expect(v).toContain("-0.4167vw");
  });

  it("writes a negative intercept with a minus sign", () => {
    expect(fluidClamp(16, 64, 320, 1280)).toBe("clamp(1rem, 5vw, 4rem)");
    expect(fluidClamp(10, 64, 320, 1280)).toBe("clamp(0.625rem, 5.625vw - 0.5rem, 4rem)");
  });

  it("builds a fluid scale with clamp values", () => {
    const steps = buildScale({ ...base, fluid: true });
    const body = steps.find((s) => s.name === "base")!;
    expect(body.min).toBe(16);
    expect(body.max).toBe(18);
    expect(body.value).toMatch(/^clamp\(1rem, .*vw \+ .*rem, 1\.125rem\)$/);
  });
});

describe("export", () => {
  const steps = buildScale({ ...base, stepsDown: 0, stepsUp: 1 });

  it("exports CSS custom properties", () => {
    expect(toCssVariables(steps, 'Geist "Mono"')).toBe(
      [":root {", '  --font-body: "Geist \\"Mono\\"", system-ui, sans-serif;', "  --text-base: 1rem;", "  --leading-base: 1.5;", "  --text-lg: 1.25rem;", "  --leading-lg: 1.4;", "}"].join("\n"),
    );
  });

  it("exports a Tailwind v4 theme block", () => {
    const out = toTailwindTheme(steps);
    expect(out.startsWith("@theme {")).toBe(true);
    expect(out).toContain("  --text-lg: 1.25rem;");
    expect(out).toContain("  --text-lg--line-height: 1.4;");
    expect(out).not.toContain("--font-sans");
  });

  it("picks the Tailwind font token from the category", () => {
    expect(toTailwindTheme(steps, "JetBrains Mono", "monospace")).toContain('  --font-mono: "JetBrains Mono", ui-monospace, monospace;');
    expect(toTailwindTheme(steps, "Lora", "serif")).toContain('  --font-serif: "Lora", ui-serif, Georgia, serif;');
    expect(toTailwindTheme(steps, "Inter")).toContain('  --font-sans: "Inter", ui-sans-serif, system-ui, sans-serif;');
  });

  it("quotes CSS strings safely", () => {
    expect(cssString('a"b\\c')).toBe('"a\\"b\\\\c"');
  });
});
