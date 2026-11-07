function area(p: ArrayLike<number>): number {
  let a = 0;
  const n = p.length / 2;
  for (let i = 0, j = n - 1; i < n; j = i++) a += (p[j * 2] - p[i * 2]) * (p[j * 2 + 1] + p[i * 2 + 1]);
  return a / 2;
}

function pointInTri(
  px: number,
  py: number,
  ax: number,
  ay: number,
  bx: number,
  by: number,
  cx: number,
  cy: number,
): boolean {
  const d1 = (px - bx) * (ay - by) - (ax - bx) * (py - by);
  const d2 = (px - cx) * (by - cy) - (bx - cx) * (py - cy);
  const d3 = (px - ax) * (cy - ay) - (cx - ax) * (py - ay);
  const neg = d1 < 0 || d2 < 0 || d3 < 0;
  const pos = d1 > 0 || d2 > 0 || d3 > 0;
  return !(neg && pos);
}

function triangulate(poly: ArrayLike<number>): number[] {
  const n = Math.floor(poly.length / 2);
  if (n < 3) return [];
  const idx: number[] = [];
  if (area(poly) > 0) for (let i = 0; i < n; i++) idx.push(i);
  else for (let i = n - 1; i >= 0; i--) idx.push(i);
  const out: number[] = [];
  const x = (i: number) => poly[i * 2];
  const y = (i: number) => poly[i * 2 + 1];
  let guard = 0;
  while (idx.length > 3 && guard++ < n * n) {
    let clipped = false;
    for (let k = 0; k < idx.length; k++) {
      const a = idx[(k + idx.length - 1) % idx.length];
      const b = idx[k];
      const c = idx[(k + 1) % idx.length];
      const cross = (x(b) - x(a)) * (y(c) - y(a)) - (y(b) - y(a)) * (x(c) - x(a));
      if (cross <= 0) continue;
      let inside = false;
      for (const p of idx) {
        if (p === a || p === b || p === c) continue;
        if (pointInTri(x(p), y(p), x(a), y(a), x(b), y(b), x(c), y(c))) {
          inside = true;
          break;
        }
      }
      if (inside) continue;
      out.push(a, b, c);
      idx.splice(k, 1);
      clipped = true;
      break;
    }
    if (!clipped) {
      for (let k = 1; k + 1 < idx.length; k++) out.push(idx[0], idx[k], idx[k + 1]);
      return out;
    }
  }
  if (idx.length === 3) out.push(idx[0], idx[1], idx[2]);
  return out;
}

export function fillTriangles(poly: ArrayLike<number>): number[] {
  const tris = triangulate(poly);
  const out: number[] = [];
  for (const i of tris) out.push(poly[i * 2], poly[i * 2 + 1]);
  return out;
}
