export function num(value: number, digits = 3): string {
  if (!Number.isFinite(value)) return "0";
  const factor = 10 ** digits;
  const rounded = Math.round(value * factor) / factor;
  const text = String(Object.is(rounded, -0) ? 0 : rounded);
  return text;
}

export function pct(value: number, digits = 2): string {
  return `${num(value * 100, digits)}%`;
}

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function ms(value: number): string {
  return `${Math.round(value)}ms`;
}
