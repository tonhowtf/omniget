import { describe, expect, it } from "vitest";
import { toHex } from "./color";
import { exportCss, exportDtcg, exportJson, exportScss, exportTailwind, fileName, importText } from "./export";
import { makeProject, readProjectsFile, slugify, uniqueToken } from "./project";

const P = (n: number) => `Untitled ${n}`;
const project = makeProject("My Brand", (k) => k[0].toUpperCase() + k.slice(1));

describe("export", () => {
  it("writes CSS with hex fallback and oklch in @supports", () => {
    const css = exportCss(project);
    expect(css).toMatch(/--brand-500: #[0-9a-f]{6};/);
    expect(css).toContain("@supports (color: oklch(0% 0 0))");
    expect(css).toMatch(/--danger-950: oklch\(/);
    expect(css.match(/--neutral-/g)).toHaveLength(22);
  });
  it("writes a Tailwind v4 theme", () => {
    const tw = exportTailwind(project);
    expect(tw).toContain("@theme {");
    expect(tw).toMatch(/--color-success-50: oklch\(/);
  });
  it("writes SCSS variables and maps", () => {
    const scss = exportScss(project);
    expect(scss).toMatch(/\$accent-300: #[0-9a-f]{6};/);
    expect(scss).toContain("$accent: (");
  });
  it("writes DTCG tokens", () => {
    const tokens = JSON.parse(exportDtcg(project));
    expect(tokens.color.brand["500"].$type).toBe("color");
    expect(tokens.color.brand["500"].$value).toMatch(/^#[0-9a-f]{6}$/);
  });
  it("names files", () => {
    expect(fileName(project, "tailwind")).toBe("my-brand.theme.css");
    expect(fileName(project, "dtcg")).toBe("my-brand.tokens.json");
  });
});

describe("import", () => {
  it("round trips the native JSON", () => {
    const res = importText(exportJson(project), "x", P);
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    expect(res.project.name).toBe("My Brand");
    expect(res.project.palettes.map((p) => p.token)).toEqual(project.palettes.map((p) => p.token));
    expect(toHex(res.project.palettes[0].base)).toBe(toHex(project.palettes[0].base));
  });
  it("reads DTCG tokens back, keeping the marked base", () => {
    const res = importText(exportDtcg(project), "Imported", P);
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    expect(res.kind).toBe("dtcg");
    expect(res.project.palettes).toHaveLength(6);
    expect(toHex(res.project.palettes[0].base)).toBe(toHex(project.palettes[0].base));
  });
  it("reads foreign DTCG without our extension", () => {
    const res = importText(JSON.stringify({ blue: { "100": { $value: "#dbeafe" }, "500": { $value: "#3b82f6", $type: "color" } } }), "T", P);
    expect(res.ok && toHex(res.project.palettes[0].base)).toBe("#3b82f6");
  });
  it("reads DTCG colour objects in every colour space", () => {
    const tok = (colorSpace: string, components: unknown[]) => ({ $type: "color", $value: { colorSpace, components } });
    const cases: [string, unknown[]][] = [
      ["srgb", [1, 0, 0]],
      ["srgb-linear", [1, 0, 0]],
      ["display-p3", [0.9175, 0.2003, 0.1386]],
      ["hsl", [0, 100, 50]],
      ["oklab", [0.62796, 0.22486, 0.12585]],
      ["oklch", [0.62796, 0.25768, 29.2339]],
    ];
    for (const [space, comps] of cases) {
      const res = importText(JSON.stringify({ color: { brand: { "500": tok(space, comps) } } }), "x", P);
      expect(res.ok, space).toBe(true);
      if (!res.ok) continue;
      expect(toHex(res.project.palettes[0].base), space).toBe("#ff0000");
    }
    const none = importText(JSON.stringify({ g: { "500": tok("srgb", ["none", "none", "none"]) } }), "x", P);
    expect(none.ok && toHex(none.project.palettes[0].base)).toBe("#000000");
    expect(importText(JSON.stringify({ g: { "500": tok("xyz-d65", [0.4, 0.2, 0.1]) } }), "x", P)).toEqual({ ok: false, reason: "empty" });
  });
  it("names palettes without a name through the callback", () => {
    const res = importText(JSON.stringify({ palettes: [{ base: "#ff0000" }, { name: "Kept", base: "#00ff00" }] }), "x", (n) => `Paleta ${n}`);
    expect(res.ok && res.project.palettes.map((p) => p.name)).toEqual(["Paleta 1", "Kept"]);
  });
  it("explains failures", () => {
    expect(importText("{", "x", P)).toEqual({ ok: false, reason: "json" });
    expect(importText("{}", "x", P)).toEqual({ ok: false, reason: "empty" });
    expect(importText("12", "x", P)).toEqual({ ok: false, reason: "shape" });
  });
});

describe("project helpers", () => {
  it("slugifies names", () => {
    expect(slugify("Marca Ação", "x")).toBe("marca-acao");
    expect(slugify("ブランド", "palette-1")).toBe("palette-1");
    expect(slugify("2nd", "x")).toBe("c-2nd");
    expect(uniqueToken("brand", ["brand", "brand-2"])).toBe("brand-3");
  });
  it("drops invalid entries when reading the store", () => {
    const file = readProjectsFile({ activeId: "nope", projects: [project, { bad: true }, null] }, "P", P);
    expect(file.projects).toHaveLength(1);
    expect(file.activeId).toBe(project.id);
    expect(readProjectsFile(null, "P", P).projects).toEqual([]);
  });
});

describe("file import", () => {
  it("reads a file and names the project after it", async () => {
    const { importFile } = await import("./importer");
    const file = new File([exportDtcg(project)], "acme.tokens.json", { type: "application/json" });
    const res = await importFile(file, "x", P);
    expect(res.ok && res.project.name).toBe("acme");
    const big = new File([new Uint8Array(8 * 1024 * 1024 + 1)], "big.json");
    expect(await importFile(big, "x", P)).toEqual({ ok: false, reason: "big" });
  });
});
