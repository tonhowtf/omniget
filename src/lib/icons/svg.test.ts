import { describe, expect, it } from "vitest";
import { applyTransform, buildSprite, clampSize, buildSvg, combine, cssBackground, dataUri, dimensions, hasStroke, recolor, rewriteStroke, safeColor } from "./svg";
import type { IconData } from "./types";

const lucide: IconData = {
  name: "house",
  body: '<g fill="none" stroke="currentColor" stroke-linecap="round" stroke-width="2"><path d="M3 10v9h18"/></g>',
  width: 24,
  height: 24,
  left: 0,
  top: 0,
  rotate: 0,
  h_flip: false,
  v_flip: false,
};

describe("buildSvg", () => {
  it("builds a plain svg from body and box", () => {
    expect(buildSvg(lucide)).toBe(`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24">${lucide.body}</svg>`);
  });

  it("scales non-square icons keeping the ratio", () => {
    const wide = { ...lucide, width: 32, height: 16 };
    expect(buildSvg(wide, { size: 48 })).toContain('width="48" height="24" viewBox="0 0 32 16"');
    expect(dimensions({ width: 10, height: 20 }, 40)).toEqual({ width: 20, height: 40 });
  });

  it("rewrites stroke width and color", () => {
    const svg = buildSvg(lucide, { strokeWidth: 1.5, color: "#FF9500" });
    expect(svg).toContain('stroke-width="1.5"');
    expect(svg).toContain('stroke="#ff9500"');
    expect(svg).not.toContain("currentColor");
  });

  it("ignores unsafe colors", () => {
    expect(safeColor('red" onload="x')).toBeNull();
    expect(recolor("currentColor", "url(#a)")).toBe("currentColor");
    expect(safeColor("currentColor")).toBeNull();
    expect(safeColor("#abc")).toBe("#abc");
  });
});

describe("stroke", () => {
  it("detects and rewrites every stroke-width", () => {
    const body = `<path stroke-width="2"/><path stroke-width='1.25'/>`;
    expect(hasStroke(body)).toBe(true);
    expect(rewriteStroke(body, 3)).toBe(`<path stroke-width="3"/><path stroke-width="3"/>`);
    expect(rewriteStroke(body, 0)).toBe(body);
    expect(hasStroke('<path fill="currentColor"/>')).toBe(false);
  });
});

describe("transforms", () => {
  it("flips horizontally with translate and scale", () => {
    const p = applyTransform(lucide, { rotate: 0, hFlip: true, vFlip: false });
    expect(p.body.startsWith('<g transform="translate(24 0) scale(-1 1)">')).toBe(true);
  });

  it("flips icons with an offset box back into the origin", () => {
    const icon = { body: "", left: 2, top: 3, width: 20, height: 18 };
    const h = applyTransform(icon, { rotate: 0, hFlip: true, vFlip: false });
    expect(h.body).toBe('<g transform="translate(22 -3) scale(-1 1)"></g>');
    expect(h.viewBox).toBe("0 0 20 18");
    const v = applyTransform(icon, { rotate: 0, hFlip: false, vFlip: true });
    expect(v.body).toBe('<g transform="translate(-2 21) scale(1 -1)"></g>');
    expect(v.viewBox).toBe("0 0 20 18");
    const mapX = (x: number) => -x + 22;
    expect([mapX(2), mapX(22)]).toEqual([20, 0]);
  });

  it("keeps the offset box when only rotating", () => {
    const p = applyTransform({ body: "", left: 2, top: 3, width: 20, height: 18 }, { rotate: 2, hFlip: false, vFlip: false });
    expect(p.body).toContain("rotate(180 12 12)");
    expect(p.viewBox).toBe("2 3 20 18");
  });

  it("turns double flip into a half turn", () => {
    const p = applyTransform(lucide, { rotate: 0, hFlip: true, vFlip: true });
    expect(p.body).toContain("rotate(180 12 12)");
  });

  it("swaps the box on quarter turns", () => {
    const p = applyTransform({ ...lucide, width: 32, height: 16 }, { rotate: 1, hFlip: false, vFlip: false });
    expect(p.viewBox).toBe("0 0 16 32");
    expect(p.body).toContain("rotate(90 8 8)");
  });

  it("combines icon and user transforms", () => {
    expect(combine({ rotate: 3, h_flip: true, v_flip: false }, { rotate: 2, hFlip: true })).toEqual({ rotate: 1, hFlip: false, vFlip: false });
  });
});

describe("css and sprite", () => {
  it("encodes a data uri", () => {
    const uri = dataUri('<svg a="#1"><g/></svg>');
    expect(uri).toBe("data:image/svg+xml,%3Csvg a='%231'%3E%3Cg/%3E%3C/svg%3E");
  });

  it("writes a css background with a concrete color", () => {
    const css = cssBackground(lucide, { size: 32 });
    expect(css).toContain("width: 32px;");
    expect(css).toContain("stroke='%23000'");
  });

  it("builds a sprite with one symbol per unique icon", () => {
    const sprite = buildSprite([
      { prefix: "lucide", icon: lucide },
      { prefix: "lucide", icon: lucide },
      { prefix: "ph", icon: { ...lucide, name: "star", body: "<path/>" } },
    ]);
    expect(sprite.match(/<symbol /g)?.length).toBe(2);
    expect(sprite).toContain('<symbol id="lucide-house" viewBox="0 0 24 24">');
    expect(sprite).toContain('id="ph-star"');
    expect(sprite.startsWith('<svg xmlns="http://www.w3.org/2000/svg" style="display: none">')).toBe(true);
  });
});

describe("clampSize", () => {
  it("keeps sizes inside the allowed range", () => {
    expect(clampSize(0)).toBe(8);
    expect(clampSize(-5)).toBe(8);
    expect(clampSize(99999)).toBe(1024);
    expect(clampSize(31.6)).toBe(32);
    expect(clampSize(null)).toBe(24);
    expect(clampSize(Number.NaN)).toBe(24);
    expect(clampSize("")).toBe(24);
  });
});
