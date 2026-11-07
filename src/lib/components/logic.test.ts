import { describe, expect, it } from "vitest";
import { customSource, docsUrl, itemUrl, parseItem, parseRegistry, shortType } from "./registry";
import { scoreItem, search, typeCounts } from "./search";
import { buildCommand, detectPackageManager, parseWrittenFiles, stripAnsi } from "./command";
import { adaptValue, applyVars, parseColor, toHslChannels, buildVars, cssSnippet, cssValue, flattenDtcg, hunks, lineDiff, readVar, suggestMapping, tokenKey, tokenMode } from "./tokens";
import type { RegistrySource } from "./types";

const SHADCN: RegistrySource = {
  id: "shadcn",
  name: "shadcn/ui",
  homepage: "https://ui.shadcn.com",
  index_url: "https://ui.shadcn.com/r/styles/new-york-v4/registry.json",
  item_url: "https://ui.shadcn.com/r/styles/new-york-v4/{name}.json",
  docs_url: "https://ui.shadcn.com/docs/components/{name}",
};

describe("registry parsing", () => {
  it("reads object registries and skips bad items", () => {
    const parsed = parseRegistry(
      {
        name: "shadcn/ui",
        homepage: "https://ui.shadcn.com",
        items: [
          { name: "button", type: "registry:ui", description: "A button", dependencies: ["radix-ui"], files: [{}] },
          { name: "../evil", type: "registry:ui" },
          { name: "button", type: "registry:ui" },
          "nope",
          { name: "login-01", type: "registry:block", title: "Login", categories: ["auth"] },
        ],
      },
      "shadcn",
    );
    expect(parsed.items.map((i) => i.name)).toEqual(["button", "login-01"]);
    expect(parsed.skipped).toBe(3);
    expect(parsed.items[0]).toMatchObject({ type: "ui", fileCount: 1, dependencies: ["radix-ui"], key: "shadcn::button" });
    expect(parsed.items[1]).toMatchObject({ type: "block", title: "Login", categories: ["auth"] });
  });

  it("reads array indexes and rejects other shapes", () => {
    expect(parseRegistry([{ name: "accordion", type: "registry:ui" }], "x").items).toHaveLength(1);
    expect(() => parseRegistry({ hello: 1 }, "x")).toThrow();
    expect(() => parseRegistry("x", "x")).toThrow();
    expect(parseRegistry({ items: [], homepage: "http://insecure" }, "x").homepage).toBeNull();
  });

  it("normalizes types", () => {
    expect(shortType("registry:hook")).toBe("hook");
    expect(shortType(undefined)).toBe("ui");
    expect(shortType("registry:<script>")).toBe("other");
  });

  it("parses item detail defensively", () => {
    const item = parseItem({
      name: "button",
      type: "registry:ui",
      dependencies: ["radix-ui", 4],
      registryDependencies: ["utils"],
      files: [{ path: "ui/button.tsx", type: "registry:ui", content: "export {}" }, { content: "no path" }],
      cssVars: { light: { a: "1", b: "2" }, dark: { a: "3" } },
    });
    expect(item.dependencies).toEqual(["radix-ui"]);
    expect(item.files).toEqual([{ path: "ui/button.tsx", type: "ui", target: null, content: "export {}" }]);
    expect(item.cssVarCount).toBe(3);
    expect(() => parseItem([])).toThrow();
    const dup = parseItem({ name: "x", dependencies: ["clsx", "clsx", " clsx "], devDependencies: ["a", "a"], registryDependencies: ["b", "b"], files: [{ path: "a.tsx" }, { path: "a.tsx", content: "2" }] });
    expect(dup.dependencies).toEqual(["clsx"]);
    expect(dup.devDependencies).toEqual(["a"]);
    expect(dup.registryDependencies).toEqual(["b"]);
    expect(dup.files.map((f) => f.path)).toEqual(["a.tsx"]);
    expect(() => parseItem({ name: "a/b" })).toThrow();
  });

  it("builds item, docs and custom registry urls", () => {
    expect(itemUrl(SHADCN, "button")).toBe("https://ui.shadcn.com/r/styles/new-york-v4/button.json");
    expect(itemUrl(SHADCN, "../x")).toBeNull();
    expect(docsUrl(SHADCN, { name: "button", type: "ui" })).toBe("https://ui.shadcn.com/docs/components/button");
    expect(docsUrl(SHADCN, { name: "use-mobile", type: "hook" })).toBeNull();
    expect(docsUrl({ ...SHADCN, docs_url: null }, { name: "button", type: "ui" })).toBeNull();
    const custom = customSource("https://example.com/r/registry.json")!;
    expect(custom.item_url).toBe("https://example.com/r/{name}.json");
    expect(custom.custom).toBe(true);
    expect(customSource("http://example.com/r/registry.json")).toBeNull();
    expect(customSource("https://example.com/r/")).toBeNull();
    expect(customSource("https://127.0.0.1/r.json")).toBeNull();
    expect(customSource("https://localhost/r.json")).toBeNull();
  });
});

describe("search", () => {
  const items = parseRegistry(
    {
      items: [
        { name: "button", type: "registry:ui", description: "Displays a button" },
        { name: "button-group", type: "registry:ui", description: "Groups buttons" },
        { name: "AnimatedContent-JS-CSS", title: "AnimatedContent", type: "registry:component", description: "Reveals content on scroll" },
        { name: "login-01", type: "registry:block", description: "A simple login form", categories: ["authentication"] },
        { name: "button-demo", type: "registry:example" },
        { name: "use-mobile", type: "registry:hook", description: "Detects mobile" },
      ],
    },
    "a",
  ).items;

  it("ranks exact name above prefix and description hits", () => {
    const out = search(items, { query: "button", registryId: null, type: null });
    expect(out.map((i) => i.name)).toEqual(["button", "button-group"]);
  });

  it("matches camelCase words, categories and all tokens", () => {
    expect(search(items, { query: "content", registryId: null, type: null })[0].name).toBe("AnimatedContent-JS-CSS");
    expect(search(items, { query: "auth", registryId: null, type: null })[0].name).toBe("login-01");
    expect(search(items, { query: "login form", registryId: null, type: null })).toHaveLength(1);
    expect(search(items, { query: "login zebra", registryId: null, type: null })).toHaveLength(0);
    expect(scoreItem(items[0], "")).toBe(1);
  });

  it("hides examples unless asked and filters by registry", () => {
    expect(search(items, { query: "", registryId: null, type: null }).some((i) => i.type === "example")).toBe(false);
    expect(search(items, { query: "", registryId: null, type: "example" })).toHaveLength(1);
    expect(search(items, { query: "", registryId: "b", type: null })).toHaveLength(0);
    expect(typeCounts(items, null)[0]).toEqual(["ui", 2]);
  });
});

describe("commands", () => {
  it("detects package managers", () => {
    expect(detectPackageManager([])).toEqual({ manager: "npm", runner: "npm", lockfile: null, ambiguous: false });
    expect(detectPackageManager(["yarn.lock"]).manager).toBe("yarn");
    expect(detectPackageManager(["bun.lock"]).manager).toBe("bun");
    expect(detectPackageManager(["package-lock.json", "pnpm-lock.yaml"])).toEqual({ manager: "pnpm", runner: "pnpm", lockfile: "pnpm-lock.yaml", ambiguous: true });
  });

  it("runs Yarn 1 projects through npx because Yarn 1 has no dlx", () => {
    expect(detectPackageManager(["yarn.lock"], true)).toMatchObject({ manager: "yarn", runner: "npm" });
    expect(detectPackageManager(["yarn.lock"], false)).toMatchObject({ manager: "yarn", runner: "yarn" });
    expect(detectPackageManager(["pnpm-lock.yaml"], true).runner).toBe("pnpm");
  });

  it("never lists the same written file twice", () => {
    const out = parseWrittenFiles(["✔ Created 2 files:", "  - a.tsx", "  - a.tsx", "ℹ Updated 1 file:", "  - a.tsx", "  - b.tsx"]);
    expect(out.created).toEqual(["a.tsx"]);
    expect(out.updated).toEqual(["b.tsx"]);
  });

  it("builds argument arrays per manager", () => {
    const url = "https://ui.shadcn.com/r/styles/new-york-v4/button.json";
    expect(buildCommand("pnpm", { kind: "add", targets: [url] })).toEqual({
      program: "pnpm",
      args: ["dlx", "shadcn@latest", "add", url, "--yes"],
      display: `pnpm dlx shadcn@latest add ${url} --yes`,
    });
    expect(buildCommand("npm", { kind: "add", targets: [url], overwrite: true }).args).toEqual(["shadcn@latest", "add", url, "--yes", "--overwrite"]);
    expect(buildCommand("bun", { kind: "init" }).display).toBe("bunx --bun shadcn@latest init --yes --defaults");
    expect(buildCommand("yarn", { kind: "init" }).args.slice(0, 2)).toEqual(["dlx", "shadcn@latest"]);
    expect(() => buildCommand("npm", { kind: "add", targets: ["button; rm -rf /"] })).toThrow();
  });

  it("reads written files from shadcn output", () => {
    const lines = [
      "\u001b[32m✔\u001b[39m Checking registry.",
      "✔ Created 2 files:",
      "  - components/ui/button.tsx",
      "  - lib/utils.ts",
      "ℹ Skipped 1 file: (files might be identical, use --overwrite to overwrite)",
      "  - components/ui/card.tsx",
      "Done.",
    ];
    expect(parseWrittenFiles(lines)).toEqual({ created: ["components/ui/button.tsx", "lib/utils.ts"], updated: [], skipped: ["components/ui/card.tsx"] });
    expect(stripAnsi("\u001b[1mbold\u001b[0m\r")).toBe("bold");
  });
});

describe("tokens", () => {
  const doc = {
    color: {
      $type: "color",
      brand: { 500: { $value: "#3366FF" } },
      light: {
        background: { $value: "#ffffff" },
        foreground: { $value: "#0a0a0a" },
        primary: { $value: "{color.brand.500}" },
        "primary-foreground": { $value: { colorSpace: "srgb", components: [1, 1, 1], hex: "#FFFFFF" } },
        danger: { $value: { colorSpace: "oklch", components: [0.6, 0.2, 25] } },
      },
      dark: {
        background: { $value: "#0a0a0a" },
        primary: { $value: "{color.brand.missing}" },
      },
    },
    radius: { md: { $type: "dimension", $value: { value: 0.5, unit: "rem" } } },
    legacy: { ring: { value: "#999999", type: "color" } },
    loop: { a: { $value: "{loop.b}" }, b: { $value: "{loop.a}" } },
  };

  it("flattens DTCG with alias resolution and inherited types", () => {
    const { tokens, unresolved } = flattenDtcg(doc);
    const by = Object.fromEntries(tokens.map((t) => [t.path, t]));
    expect(by["color.light.primary"].value).toBe("#3366FF");
    expect(by["color.light.primary"].type).toBe("color");
    expect(by["color.light.primary-foreground"].value).toBe("#ffffff");
    expect(by["color.light.danger"].value).toBe("oklch(0.6 0.2 25)");
    expect(by["radius.md"].value).toBe("0.5rem");
    expect(by["legacy.ring"].value).toBe("#999999");
    expect(unresolved.sort()).toEqual(["color.dark.primary", "loop.a", "loop.b"]);
  });

  it("refuses css injection in values", () => {
    expect(cssValue("red; } body { display:none", "color")).toBeNull();
    expect(cssValue(["Inter", "system ui"], "fontFamily")).toBe('Inter, "system ui"');
  });

  it("suggests a mapping by name and mode", () => {
    const { tokens } = flattenDtcg(doc);
    const rows = Object.fromEntries(suggestMapping(tokens).map((r) => [r.name, r]));
    expect(rows.background).toEqual({ name: "background", light: "color.light.background", dark: "color.dark.background" });
    expect(rows.primary.light).toBe("color.light.primary");
    expect(rows["primary-foreground"].light).toBe("color.light.primary-foreground");
    expect(rows.destructive.light).toBe("color.light.danger");
    expect(rows.radius.light).toBe("radius.md");
    expect(rows.ring.light).toBe("legacy.ring");
    expect(rows.foreground.light).toBe("color.light.foreground");
    expect(tokenKey("Colors.Light.primaryForeground")).toBe("primary-foreground");
    expect(tokenMode("theme.dark.bg")).toBe("dark");
  });

  it("updates existing vars, appends missing ones and keeps hsl channel format", () => {
    const css = "@layer base {\n  :root {\n    --background: 0 0% 100%;\n    --radius: 0.5rem;\n  }\n}\n";
    const { css: out, changes } = applyVars(css, ":root", { background: "#000000", primary: "#ffffff", radius: "1rem" });
    expect(out).toBe("@layer base {\n  :root {\n    --background: 0 0% 0%;\n    --radius: 1rem;\n    --primary: 0 0% 100%;\n  }\n}\n");
    expect(changes.map((c) => c.name)).toEqual(["background", "primary", "radius"]);
    expect(readVar(out, ":root", "primary")).toBe("0 0% 100%");
    expect(adaptValue("#ff0000", "oklch(1 0 0)")).toBe("#ff0000");
  });

  it("converts every supported color form into hsl channels and leaves out the rest", () => {
    const css = ":root {\n  --background: 0 0% 100%;\n  --primary: 222.2 47.4% 11.2%;\n}\n";
    const { css: out, skipped } = applyVars(css, ":root", { primary: "oklch(0.5 0.2 250)", background: "#ffffff80", accent: "lab(50 20 30)" });
    expect(out).not.toMatch(/oklch|#ffffff80|lab\(/);
    expect(readVar(out, ":root", "background")).toBe("0 0% 100% / 0.502");
    expect(readVar(out, ":root", "primary")).toMatch(/^\d+(\.\d)? \d+(\.\d)?% \d+(\.\d)?%$/);
    expect(skipped).toEqual(["accent"]);
    expect(toHslChannels("rgb(255 0 0)")).toBe("0 100% 50%");
    expect(toHslChannels("rgba(0, 0, 255, 0.5)")).toBe("240 100% 50% / 0.5");
    expect(toHslChannels("hsl(120 50% 25%)")).toBe("120 50% 25%");
    expect(toHslChannels("oklch(1 0 0)")).toBe("0 0% 100%");
    expect(toHslChannels("oklch(0 0 0)")).toBe("0 0% 0%");
    expect(toHslChannels("color(srgb 1 0 0)")).toBe("0 100% 50%");
    expect(toHslChannels("transparent")).toBeNull();
    expect(parseColor("oklch(0.628 0.2577 29.23)")).toMatchObject({ a: 1 });
    const red = parseColor("oklch(0.628 0.2577 29.23)")!;
    expect(Math.round(red.r * 255)).toBeGreaterThan(250);
    expect(Math.round(red.g * 255)).toBeLessThan(5);
  });

  it("keeps dtcg color objects valid", () => {
    expect(cssValue({ colorSpace: "srgb", components: [1, 0, 0], hex: "#ff000080", alpha: 0.5 }, "color")).toBe("#ff000080");
    expect(cssValue({ colorSpace: "srgb", components: [1, 0, 0], hex: "#f00", alpha: 0.5 }, "color")).toBe("#ff000080");
    expect(cssValue({ colorSpace: "srgb", components: [1, "none", 0] }, "color")).toBe("color(srgb 1 none 0)");
    expect(cssValue({ colorSpace: "hsl", components: ["none", 50, 40] }, "color")).toBe("hsl(none 50% 40%)");
  });

  it("keeps the first of two tokens with the same path", () => {
    const res = flattenDtcg({ "a.b": { $type: "color", $value: "#111" }, a: { b: { $type: "color", $value: "#222" } } });
    expect(res.tokens).toEqual([{ path: "a.b", type: "color", value: "#111" }]);
    expect(res.duplicates).toEqual(["a.b"]);
  });

  it("resolves Tokens Studio aliases across sets", () => {
    const res = flattenDtcg({
      global: { colors: { blue: { value: "#3366ff", type: "color" }, white: { value: "#ffffff", type: "color" } } },
      light: { primary: { value: "{colors.blue}", type: "color" }, ring: { value: "{primary}", type: "color" } },
      dark: { primary: { value: "{colors.white}", type: "color" } },
      $themes: [],
      $metadata: { tokenSetOrder: ["global", "light", "dark"] },
    });
    expect(res.unresolved).toEqual([]);
    const by = Object.fromEntries(res.tokens.map((t) => [t.path, t.value]));
    expect(by["light.primary"]).toBe("#3366ff");
    expect(by["light.ring"]).toBe("#3366ff");
    expect(by["dark.primary"]).toBe("#ffffff");
  });

  it("adds a block when the selector is missing", () => {
    const { css } = applyVars(":root {\n  --a: 1;\n}\n", ".dark", { background: "#000" });
    expect(css).toBe(":root {\n  --a: 1;\n}\n\n.dark {\n  --background: #000;\n}\n");
    expect(applyVars("x", ".dark", {}).css).toBe("x");
  });

  it("builds vars and a snippet from a mapping", () => {
    const { tokens } = flattenDtcg(doc);
    const rows = suggestMapping(tokens);
    const light = buildVars(rows, tokens, "light");
    expect(light.background).toBe("#ffffff");
    const snippet = cssSnippet({ background: "#fff" }, { background: "#000" });
    expect(snippet).toBe(":root {\n  --background: #fff;\n}\n\n.dark {\n  --background: #000;\n}\n");
  });

  it("diffs lines and groups hunks", () => {
    const diff = lineDiff("a\nb\nc\nd\ne\nf\ng", "a\nb\nC\nd\ne\nf\ng\nh");
    expect(diff.filter((d) => d.kind !== "same").map((d) => `${d.kind}:${d.text}`)).toEqual(["del:c", "add:C", "add:h"]);
    const groups = hunks(diff, 1);
    expect(groups).toHaveLength(2);
    expect(groups[0].lines.map((l) => l.text)).toEqual(["b", "c", "C", "d"]);
  });
});
