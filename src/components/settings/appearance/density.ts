export type Density = "compact" | "default" | "comfortable";

export const DENSITIES: readonly Density[] = ["compact", "default", "comfortable"];

export const DENSITY_VALUES: Readonly<Record<Density, { spacing_scale: number; line_height_base: number }>> = {
  compact: { spacing_scale: 0.9, line_height_base: 1.45 },
  default: { spacing_scale: 1.0, line_height_base: 1.55 },
  comfortable: { spacing_scale: 1.1, line_height_base: 1.7 },
};

const TOLERANCE = 0.001;

function near(a: unknown, b: number): boolean {
  return typeof a === "number" && Number.isFinite(a) && Math.abs(a - b) < TOLERANCE;
}

export function densityFor(typography: { spacing_scale?: unknown; line_height_base?: unknown } | null | undefined): Density | null {
  const spacing = typography?.spacing_scale ?? DENSITY_VALUES.default.spacing_scale;
  const lineHeight = typography?.line_height_base ?? DENSITY_VALUES.default.line_height_base;
  return DENSITIES.find((d) => near(spacing, DENSITY_VALUES[d].spacing_scale) && near(lineHeight, DENSITY_VALUES[d].line_height_base)) ?? null;
}

export function densityPatch(density: Density): { typography: { spacing_scale: number; line_height_base: number } } {
  return { typography: { ...DENSITY_VALUES[density] } };
}
