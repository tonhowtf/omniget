// Own ear clipping for simple polygons (02-motor-do-canvas §6.2). O(n²), fine
// for shape outlines of a few hundred points; earcut stays as plan B if the
// bench says so. Input flat [x,y,...]; output vertex indices, 3 per triangle.

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

export function triangulate(poly: ArrayLike<number>): number[] {
  const n = Math.floor(poly.length / 2);
  if (n < 3) return [];
  // Normalise to positive signed area (counter-clockwise in y-up terms): a
  // convex corner then has a positive cross product.
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
      if (cross <= 0) continue; // reflex (or degenerate) for this winding
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
      // Degenerate input: fan the rest instead of looping forever.
      for (let k = 1; k + 1 < idx.length; k++) out.push(idx[0], idx[k], idx[k + 1]);
      return out;
    }
  }
  if (idx.length === 3) out.push(idx[0], idx[1], idx[2]);
  return out;
}

/** Flat triangle list (x,y per vertex) of a polygon's fill. */
export function fillTriangles(poly: ArrayLike<number>): number[] {
  const tris = triangulate(poly);
  const out: number[] = [];
  for (const i of tris) out.push(poly[i * 2], poly[i * 2 + 1]);
  return out;
}
