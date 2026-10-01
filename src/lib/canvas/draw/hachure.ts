const HACHURE_ANGLE = -41;

export function hachureLines(poly: ArrayLike<number>, gap: number, angleDeg = HACHURE_ANGLE): number[] {
  const n = Math.floor(poly.length / 2);
  if (n < 3 || gap <= 0) return [];
  const a = (angleDeg * Math.PI) / 180;
  const cos = Math.cos(a);
  const sin = Math.sin(a);
  const rx: number[] = [];
  const ry: number[] = [];
  let minY = Infinity;
  let maxY = -Infinity;
  for (let i = 0; i < n; i++) {
    const x = poly[i * 2];
    const y = poly[i * 2 + 1];
    const X = x * cos + y * sin;
    const Y = -x * sin + y * cos;
    rx.push(X);
    ry.push(Y);
    if (Y < minY) minY = Y;
    if (Y > maxY) maxY = Y;
  }
  const out: number[] = [];
  const back = (X: number, Y: number) => [X * cos - Y * sin, X * sin + Y * cos];
  let guard = 0;
  for (let y = minY + gap / 2; y < maxY && guard++ < 2000; y += gap) {
    const xs: number[] = [];
    for (let i = 0, j = n - 1; i < n; j = i++) {
      const yi = ry[i];
      const yj = ry[j];
      if (yi > y !== yj > y) xs.push(rx[i] + ((y - yi) * (rx[j] - rx[i])) / (yj - yi));
    }
    xs.sort((p, q) => p - q);
    for (let k = 0; k + 1 < xs.length; k += 2) {
      const [x0, y0] = back(xs[k], y);
      const [x1, y1] = back(xs[k + 1], y);
      out.push(x0, y0, x1, y1);
    }
  }
  return out;
}

export function crossHatchLines(poly: ArrayLike<number>, gap: number): number[] {
  return [...hachureLines(poly, gap, HACHURE_ANGLE), ...hachureLines(poly, gap, HACHURE_ANGLE + 90)];
}
