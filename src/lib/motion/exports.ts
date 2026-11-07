import type { Bezier } from "./bezier";
import { num } from "./format";
import type { SpringParams } from "./spring";
import { toSvelteSpring, toSwiftUI } from "./spring";

const list = (b: Bezier) => b.map((v) => num(v, 3)).join(", ");

export function cssBezier(b: Bezier): string {
  return `cubic-bezier(${list(b)})`;
}

export function cssTransition(b: Bezier, durationMs: number, property = "transform"): string {
  return `transition: ${property} ${Math.round(durationMs)}ms ${cssBezier(b)};`;
}

export function motionEase(b: Bezier, durationMs: number): string {
  return `{ duration: ${num(durationMs / 1000, 3)}, ease: [${list(b)}] }`;
}

export function gsapEase(b: Bezier, name = "custom"): string {
  const [x1, y1, x2, y2] = b.map((v) => num(v, 3));
  return `CustomEase.create("${name}", "M0,0 C${x1},${y1} ${x2},${y2} 1,1");`;
}

export function svelteEase(b: Bezier): string {
  const [x1, y1, x2, y2] = b.map((v) => num(v, 3));
  return [
    "export function ease(t) {",
    `  const x1 = ${x1}, y1 = ${y1}, x2 = ${x2}, y2 = ${y2};`,
    "  const cx = 3 * x1, bx = 3 * (x2 - x1) - cx, ax = 1 - cx - bx;",
    "  const cy = 3 * y1, by = 3 * (y2 - y1) - cy, ay = 1 - cy - by;",
    "  const x = (s) => ((ax * s + bx) * s + cx) * s;",
    "  const dx = (s) => (3 * ax * s + 2 * bx) * s + cx;",
    "  if (t <= 0) return 0;",
    "  if (t >= 1) return 1;",
    "  let s = t;",
    "  for (let i = 0; i < 8; i++) {",
    "    const e = x(s) - t;",
    "    if (Math.abs(e) < 1e-6) break;",
    "    const d = dx(s);",
    "    if (Math.abs(d) < 1e-6) {",
    "      let lo = 0, hi = 1;",
    "      s = t;",
    "      for (let j = 0; j < 40; j++) { if (x(s) < t) lo = s; else hi = s; s = (lo + hi) / 2; }",
    "      break;",
    "    }",
    "    s -= e / d;",
    "  }",
    "  return ((ay * s + by) * s + cy) * s;",
    "}",
  ].join("\n");
}

export function motionSpring(p: SpringParams): string {
  return `{ type: "spring", stiffness: ${num(p.stiffness, 2)}, damping: ${num(p.damping, 2)}, mass: ${num(p.mass, 2)} }`;
}

export function svelteSpring(p: SpringParams): string {
  const s = toSvelteSpring(p);
  return `new Spring(0, { stiffness: ${num(s.stiffness, 4)}, damping: ${num(s.damping, 4)} })`;
}

export function swiftSpring(p: SpringParams): string {
  const s = toSwiftUI(p);
  return `.spring(response: ${num(s.response, 3)}, dampingFraction: ${num(s.dampingFraction, 3)})`;
}

export function cssSpring(linear: string, durationMs: number, property = "transform"): string {
  return `transition: ${property} ${Math.round(durationMs)}ms ${linear};`;
}
