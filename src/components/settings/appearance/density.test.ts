import { describe, expect, it } from "vitest";
import { DENSITIES, DENSITY_VALUES, densityFor, densityPatch } from "./density";

describe("densityFor", () => {
  it("maps each grid value back to its density", () => {
    for (const d of DENSITIES) expect(densityFor(DENSITY_VALUES[d])).toBe(d);
  });

  it("treats a missing typography object as the standard density", () => {
    expect(densityFor(undefined)).toBe("default");
    expect(densityFor(null)).toBe("default");
    expect(densityFor({})).toBe("default");
  });

  it("tolerates float noise from the backend", () => {
    expect(densityFor({ spacing_scale: 0.9000000001, line_height_base: 1.4499999 })).toBe("compact");
  });

  it("returns null for values off the grid", () => {
    expect(densityFor({ spacing_scale: 1.2, line_height_base: 1.55 })).toBeNull();
    expect(densityFor({ spacing_scale: 1.0, line_height_base: 1.7 })).toBeNull();
    expect(densityFor({ spacing_scale: "1", line_height_base: 1.55 })).toBeNull();
    expect(densityFor({ spacing_scale: Number.NaN, line_height_base: 1.55 })).toBeNull();
  });
});

describe("densityPatch", () => {
  it("writes only spacing and line height", () => {
    expect(densityPatch("comfortable")).toEqual({ typography: { spacing_scale: 1.1, line_height_base: 1.7 } });
  });

  it("returns a fresh object each time", () => {
    const patch = densityPatch("default");
    patch.typography.spacing_scale = 2;
    expect(DENSITY_VALUES.default.spacing_scale).toBe(1.0);
  });
});
